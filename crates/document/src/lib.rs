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
    /// Fingerprint of the PDF text this document was saved alongside (see
    /// `pdf_source::PdfSource::text_fingerprint`); lets a reader tell whether
    /// the PDF was changed by another program since.
    #[serde(default)]
    fingerprint: Option<u64>,
    document: Document,
}

impl Document {
    /// Serialized editable document, optionally tied to a PDF fingerprint.
    pub fn to_bytes(&self, fingerprint: Option<u64>) -> io::Result<Vec<u8>> {
        let saved = SavedDocument { format: "revise".into(), version: FORMAT_VERSION, fingerprint, document: self.clone() };
        serde_json::to_vec(&saved).map_err(io::Error::other)
    }

    pub fn from_bytes(bytes: &[u8]) -> io::Result<(Document, Option<u64>)> {
        let saved: SavedDocument = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        // "reflow" is the format name from before the app was renamed.
        if !matches!(saved.format.as_str(), "revise" | "reflow") || saved.version > FORMAT_VERSION {
            return Err(io::Error::other(format!("unsupported document format {} v{}", saved.format, saved.version)));
        }
        Ok((saved.document, saved.fingerprint))
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("revise.tmp");
        std::fs::write(&tmp, self.to_bytes(None)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn load(path: &Path) -> io::Result<Document> {
        Ok(Self::from_bytes(&std::fs::read(path)?)?.0)
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
