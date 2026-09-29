//! Editable document model. After import this — not the PDF — is the source
//! of truth: layout, editing, rendering and export all read from it.

pub mod geom;
pub mod model;
mod outline;
mod text;
mod tree;

pub use geom::*;
pub use model::*;
pub use tree::ParaRef;

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// On-disk format version; bump when the model changes incompatibly.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct SavedDocument {
    format: String,
    version: u32,
    document: Document,
}

impl Document {
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let saved = SavedDocument { format: "reflow".into(), version: FORMAT_VERSION, document: self.clone() };
        let json = serde_json::to_vec(&saved).map_err(io::Error::other)?;
        let tmp = path.with_extension("reflow.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)
    }

    pub fn load(path: &Path) -> io::Result<Document> {
        let bytes = std::fs::read(path)?;
        let saved: SavedDocument = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if saved.format != "reflow" || saved.version > FORMAT_VERSION {
            return Err(io::Error::other(format!("unsupported document format {} v{}", saved.format, saved.version)));
        }
        Ok(saved.document)
    }
}

mod b64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD.decode(s).map_err(serde::de::Error::custom)
    }
}
