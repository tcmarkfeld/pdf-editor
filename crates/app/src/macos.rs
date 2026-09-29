//! macOS "open documents" Apple Events: Finder's Open With, double-clicking
//! a document, or dropping one on the Dock icon. macOS delivers these as
//! Apple Events rather than command-line arguments, and winit ignores them,
//! so we install our own handler and queue the paths for the UI.
//!
//! Timing matters: when a document *launches* the app, AppKit delivers the
//! event right after `applicationWillFinishLaunching`. Our handler must be
//! installed by then, or AppKit's default (NSDocument-based) handling runs
//! and shows "cannot open files in the PDF Document format". So we observe
//! the will-finish-launching notification from `main`, before the event
//! loop starts, and install the Apple Event handler inside it.

use std::ffi::{CStr, c_char};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};

static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();

const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

extern "C-unwind" fn handle_open(_this: *mut AnyObject, _cmd: Sel, event: *mut AnyObject, _reply: *mut AnyObject) {
    // SAFETY: standard NSAppleEventDescriptor / NSURL / NSString messages on
    // objects AppKit hands us; every pointer is null-checked.
    unsafe {
        let list: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
        if list.is_null() {
            return;
        }
        let n: isize = msg_send![list, numberOfItems];
        let mut paths = Vec::new();
        for i in 1..=n {
            let item: *mut AnyObject = msg_send![list, descriptorAtIndex: i];
            let url: *mut AnyObject = if item.is_null() { item } else { msg_send![item, fileURLValue] };
            let path: *mut AnyObject = if url.is_null() { url } else { msg_send![url, path] };
            if path.is_null() {
                continue;
            }
            let utf8: *const c_char = msg_send![path, UTF8String];
            if !utf8.is_null() {
                paths.push(PathBuf::from(CStr::from_ptr(utf8).to_string_lossy().into_owned()));
            }
        }
        if std::env::var_os("REFLOW_TRACE").is_some() {
            eprintln!("open documents event: {paths:?}");
        }
        if let Ok(mut q) = OPENED.lock() {
            q.extend(paths);
        }
    }
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

extern "C-unwind" fn will_finish_launching(this: *mut AnyObject, _cmd: Sel, _note: *mut AnyObject) {
    // SAFETY: `this` is our handler instance; the selector matches
    // `handleOpen:withReply:` (v@:@@) registered on its class.
    if std::env::var_os("REFLOW_TRACE").is_some() {
        eprintln!("will finish launching: installing open-documents handler");
    }
    unsafe {
        let manager: *mut AnyObject = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
        let _: () = msg_send![
            manager,
            setEventHandler: this,
            andSelector: sel!(handleOpen:withReply:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: OPEN_DOCUMENTS
        ];
    }
}

fn handler_class() -> Option<&'static AnyClass> {
    let mut builder = ClassBuilder::new(c"ReflowOpenDocumentsHandler", class!(NSObject))?;
    // SAFETY: both functions match their selectors' type encodings
    // (v@:@@ and v@:@).
    unsafe {
        builder.add_method(
            sel!(handleOpen:withReply:),
            handle_open as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
        );
        builder.add_method(
            sel!(willFinishLaunching:),
            will_finish_launching as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
    }
    Some(builder.register())
}

/// Call from `main` before the event loop starts.
pub fn register_open_documents() {
    let Some(cls) = handler_class() else { return };
    // SAFETY: plain Foundation calls; the handler instance is intentionally
    // leaked because it must live for the whole process.
    unsafe {
        let handler: *mut AnyObject = msg_send![cls, new];
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let name: *mut AnyObject =
            msg_send![class!(NSString), stringWithUTF8String: c"NSApplicationWillFinishLaunchingNotification".as_ptr()];
        let _: () = msg_send![
            center,
            addObserver: handler,
            selector: sel!(willFinishLaunching:),
            name: name,
            object: std::ptr::null_mut::<AnyObject>()
        ];
    }
}

/// Lets queued and future open requests wake the UI.
pub fn set_context(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

/// Documents macOS asked us to open since the last call.
pub fn take_opened() -> Vec<PathBuf> {
    OPENED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}
