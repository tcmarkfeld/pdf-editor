//! Rendering of laid-out pages. The document is drawn from the reconstructed
//! layout — never from the source PDF bitmap (which is only a debug overlay).

pub mod glyphs;
pub mod gpu;
pub mod raster;

pub use glyphs::GlyphCache;
