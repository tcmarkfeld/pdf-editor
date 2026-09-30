//! Spell checking via the system spell checker (macOS NSSpellChecker), so
//! it follows the user's languages and learned words. Other platforms get
//! no-op stubs.

use std::ops::Range;

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{CStr, CString, c_char};
    use std::ops::Range;

    use objc2::encode::{Encode, Encoding};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRange {
        location: usize,
        length: usize,
    }

    // SAFETY: matches Foundation's `struct _NSRange { NSUInteger, NSUInteger }`.
    unsafe impl Encode for NSRange {
        const ENCODING: Encoding = Encoding::Struct("_NSRange", &[usize::ENCODING, usize::ENCODING]);
    }

    const NOT_FOUND: usize = isize::MAX as usize;

    fn nsstring(s: &str) -> *mut AnyObject {
        let c = CString::new(s.replace('\0', " ")).unwrap_or_default();
        // SAFETY: returns an autoreleased NSString copy of a valid C string.
        unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
    }

    fn checker() -> *mut AnyObject {
        // SAFETY: shared singleton.
        unsafe { msg_send![class!(NSSpellChecker), sharedSpellChecker] }
    }

    /// UTF-16 offset -> byte offset table for `s` (one extra entry at the end).
    fn utf16_to_bytes(s: &str) -> Vec<usize> {
        let mut map = Vec::with_capacity(s.len() + 1);
        for (i, c) in s.char_indices() {
            for _ in 0..c.len_utf16() {
                map.push(i);
            }
        }
        map.push(s.len());
        map
    }

    pub fn misspellings(text: &str) -> Vec<Range<usize>> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        let map = utf16_to_bytes(text);
        let ns = nsstring(text);
        let mut out = Vec::new();
        let mut start = 0usize;
        while start < map.len() - 1 {
            // SAFETY: valid NSString; `start` is within its UTF-16 length.
            let r: NSRange = unsafe { msg_send![checker(), checkSpellingOfString: ns, startingAt: start as isize] };
            if r.location == NOT_FOUND || r.length == 0 || r.location < start {
                break;
            }
            let (a, b) = (r.location, (r.location + r.length).min(map.len() - 1));
            out.push(map[a]..map[b]);
            start = b;
        }
        out
    }

    pub fn suggestions(word: &str) -> Vec<String> {
        let ns = nsstring(word);
        let len = word.encode_utf16().count();
        // SAFETY: standard NSSpellChecker call; the result is an NSArray of
        // NSStrings (or nil).
        unsafe {
            let range = NSRange { location: 0, length: len };
            let nil: *mut AnyObject = std::ptr::null_mut();
            let arr: *mut AnyObject =
                msg_send![checker(), guessesForWordRange: range, inString: ns, language: nil, inSpellDocumentWithTag: 0isize];
            if arr.is_null() {
                return Vec::new();
            }
            let n: usize = msg_send![arr, count];
            (0..n.min(6))
                .filter_map(|i| {
                    let s: *mut AnyObject = msg_send![arr, objectAtIndex: i];
                    let p: *const c_char = msg_send![s, UTF8String];
                    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
                })
                .collect()
        }
    }

    pub fn learn(word: &str) {
        // SAFETY: adds the word to the user's dictionary.
        unsafe {
            let _: () = msg_send![checker(), learnWord: nsstring(word)];
        }
    }

    pub fn ignore(word: &str) {
        // SAFETY: ignores the word for this spell document (the app session).
        unsafe {
            let _: () = msg_send![checker(), ignoreWord: nsstring(word), inSpellDocumentWithTag: 0isize];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::ops::Range;
    pub fn misspellings(_: &str) -> Vec<Range<usize>> {
        Vec::new()
    }
    pub fn suggestions(_: &str) -> Vec<String> {
        Vec::new()
    }
    pub fn learn(_: &str) {}
    pub fn ignore(_: &str) {}
}

pub use imp::{ignore, learn, suggestions};

/// Misspelled byte ranges in `text`.
pub fn misspellings(text: &str) -> Vec<Range<usize>> {
    imp::misspellings(text)
}
