//! PDF source layer: turns PDF pages into plain [`SourcePage`] data and
//! renders reference bitmaps. Nothing downstream touches PDFium directly.

pub mod fontname;
mod model;
mod pdfium;

pub use model::*;
pub use pdfium::{PdfSource, pdfium};
