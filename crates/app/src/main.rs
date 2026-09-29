mod app;
mod capture;
mod cli;
mod view;
mod worker;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    let file = args.first().map(PathBuf::from);
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 900.0]).with_title("Reflow"),
        ..Default::default()
    };
    eframe::run_native("Reflow", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, file)))))
}
