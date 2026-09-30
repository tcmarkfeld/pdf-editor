//! Development aid: `REVISE_SCREENSHOT=out.png` saves a window screenshot
//! once the UI has settled (and `REVISE_EXIT=1` quits afterwards), so GUI
//! changes can be verified without screen-recording permissions.

use std::path::PathBuf;

pub struct Capture {
    path: PathBuf,
    exit: bool,
    settled_frames: u32,
    requested: bool,
    script: std::collections::VecDeque<String>,
}

impl Capture {
    pub fn from_env() -> Option<Capture> {
        let path = std::env::var_os("REVISE_SCREENSHOT")?;
        Some(Capture {
            path: path.into(),
            exit: std::env::var_os("REVISE_EXIT").is_some(),
            settled_frames: 0,
            requested: false,
            script: std::env::var("REVISE_SCRIPT").unwrap_or_default().split(';').filter(|s| !s.is_empty()).map(str::to_string).collect(),
        })
    }

    /// Next `REVISE_SCRIPT` step (steps are `;`-separated, one per frame so
    /// every intermediate state is painted, as with real input).
    pub fn next_step(&mut self) -> Option<String> {
        self.script.pop_front()
    }

    pub fn script_done(&self) -> bool {
        self.script.is_empty()
    }

    /// `ready` should be true once content the screenshot must show is loaded.
    pub fn update(&mut self, ctx: &egui::Context, ready: bool) {
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let [w, h] = image.size;
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                let _ = img.save(&self.path);
            }
            if self.exit {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        self.settled_frames = if ready { self.settled_frames + 1 } else { 0 };
        if self.settled_frames > 5 && !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint();
    }
}

/// Script steps that are raw pointer input, in window points:
/// `mouse:x:y` clicks, `hover:x:y` moves the pointer.
pub fn pointer_events(step: &str) -> Option<Vec<egui::Event>> {
    let (cmd, arg) = step.split_once(':')?;
    let v: Vec<f32> = arg.split(':').filter_map(|s| s.parse().ok()).collect();
    let [x, y] = v[..] else { return None };
    let pos = egui::pos2(x, y);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    match cmd {
        "hover" => Some(vec![egui::Event::PointerMoved(pos)]),
        "mouse" => Some(vec![egui::Event::PointerMoved(pos), button(true), button(false)]),
        "down" => Some(vec![egui::Event::PointerMoved(pos), button(true)]),
        "up" => Some(vec![egui::Event::PointerMoved(pos), button(false)]),
        _ => None,
    }
}
