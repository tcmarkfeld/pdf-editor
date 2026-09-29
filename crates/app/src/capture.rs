//! Development aid: `REFLOW_SCREENSHOT=out.png` saves a window screenshot
//! once the UI has settled (and `REFLOW_EXIT=1` quits afterwards), so GUI
//! changes can be verified without screen-recording permissions.

use std::path::PathBuf;

pub struct Capture {
    path: PathBuf,
    exit: bool,
    settled_frames: u32,
    requested: bool,
    script: Option<String>,
}

impl Capture {
    pub fn from_env() -> Option<Capture> {
        let path = std::env::var_os("REFLOW_SCREENSHOT")?;
        Some(Capture {
            path: path.into(),
            exit: std::env::var_os("REFLOW_EXIT").is_some(),
            settled_frames: 0,
            requested: false,
            script: std::env::var("REFLOW_SCRIPT").ok(),
        })
    }

    /// `REFLOW_SCRIPT` steps (separated by `;`), handed out once.
    pub fn take_script(&mut self) -> Vec<String> {
        self.script.take().map(|s| s.split(';').map(str::to_string).collect()).unwrap_or_default()
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
