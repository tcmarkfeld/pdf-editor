//! Background PDF worker. Owns the PDFium document on a dedicated thread;
//! the UI talks to it through channels and never blocks on PDF work.
//! Render requests (what the user is looking at) are served before page
//! reconstruction, which proceeds in the background in page order.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;

use image::RgbaImage;
use pdf_source::PdfSource;
use reconstruction::PageAnalysis;

pub enum Request {
    Render { page: u32, scale: f32 },
}

pub enum Response {
    Opened { page_sizes: Vec<(f32, f32)> },
    Rendered { page: u32, scale: f32, image: RgbaImage },
    Reconstructed { index: u32, section: document::Section, analysis: Box<PageAnalysis> },
    /// The PDF carries its own editable document (saved by Reflow).
    Restored(document::Document),
    Failed(String),
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Response>,
}

impl Worker {
    pub fn open(path: PathBuf, wake: impl Fn() + Send + 'static) -> Worker {
        let (req_tx, req_rx) = channel::<Request>();
        let (resp_tx, resp_rx) = channel::<Response>();
        thread::Builder::new()
            .name("pdf-worker".into())
            .spawn(move || run(path, req_rx, resp_tx, wake))
            .expect("spawn pdf worker");
        Worker { tx: req_tx, rx: resp_rx }
    }

    pub fn request(&self, r: Request) {
        let _ = self.tx.send(r);
    }

    pub fn poll(&self) -> Vec<Response> {
        self.rx.try_iter().collect()
    }
}

fn run(path: PathBuf, rx: Receiver<Request>, tx: Sender<Response>, wake: impl Fn()) {
    let send = |r: Response| {
        let _ = tx.send(r);
        wake();
    };
    let source = match PdfSource::open(&path) {
        Ok(s) => s,
        Err(e) => return send(Response::Failed(e)),
    };
    let count = source.page_count();
    let page_sizes = (0..count).map(|i| source.page_size(i).unwrap_or((612.0, 792.0))).collect();
    send(Response::Opened { page_sizes });

    let mut renders: VecDeque<(u32, f32)> = VecDeque::new();
    let mut next_extract = 0;
    if let Some(doc) = crate::persist::embedded_document(&source) {
        send(Response::Restored(doc));
        next_extract = count;
    }
    loop {
        // Collect everything queued; block only when there is nothing to do.
        loop {
            let msg = if renders.is_empty() && next_extract >= count {
                rx.recv().map_err(|_| TryRecvError::Disconnected)
            } else {
                rx.try_recv()
            };
            match msg {
                Ok(Request::Render { page, scale }) => {
                    renders.retain(|(p, _)| *p != page);
                    renders.push_back((page, scale));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if let Some((page, scale)) = renders.pop_front() {
            match source.render(page, scale) {
                Ok(image) => send(Response::Rendered { page, scale, image }),
                Err(e) => send(Response::Failed(e)),
            }
        } else if next_extract < count {
            match reconstruction::import_page(&source, next_extract) {
                Ok((section, analysis)) => send(Response::Reconstructed { index: next_extract, section, analysis: Box::new(analysis) }),
                Err(e) => send(Response::Failed(e)),
            }
            next_extract += 1;
        }
    }
}
