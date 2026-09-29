//! Font resolution.
//!
//! PDF fonts are the riskiest fidelity input: a resume set in Calibri may be
//! opened on a machine without Calibri. Resolution is a fixed, ordered list
//! of candidates per request — the source family, known metric-compatible
//! equivalents, then a generic class fallback — and the first installed
//! family wins. Every non-exact resolution is recorded for debug tooling.

use std::borrow::Cow;
use std::collections::HashMap;

use document::{FontSpec, GenericFamily};
use parley::{FontContext, FontFamily, FontFamilyName};

/// Metric-compatible or visually close equivalents, most faithful first.
const ALIASES: &[(&str, &[&str])] = &[
    ("calibri", &["Carlito", "Helvetica Neue", "Arial"]),
    ("cambria", &["Caladea", "Georgia", "Times New Roman"]),
    ("arial", &["Arial", "Liberation Sans", "Arimo", "Helvetica"]),
    ("helvetica", &["Helvetica", "Arial", "Liberation Sans", "Arimo"]),
    ("helvetica neue", &["Helvetica Neue", "Helvetica", "Arial"]),
    ("times new roman", &["Times New Roman", "Times", "Liberation Serif", "Tinos"]),
    ("times", &["Times", "Times New Roman", "Liberation Serif", "Tinos"]),
    ("courier new", &["Courier New", "Courier", "Liberation Mono", "Cousine"]),
    ("courier", &["Courier", "Courier New", "Liberation Mono", "Cousine"]),
    ("garamond", &["EB Garamond", "Garamond", "Georgia"]),
    ("segoe ui", &["Segoe UI", "Helvetica Neue", "Arial"]),
    ("verdana", &["Verdana", "DejaVu Sans"]),
    ("georgia", &["Georgia", "Gelasio", "Times New Roman"]),
    ("symbol", &["Symbol", "Apple Symbols"]),
];

fn generic_candidates(g: GenericFamily) -> &'static [&'static str] {
    match g {
        GenericFamily::Serif => &["Times New Roman", "Times", "Georgia", "Liberation Serif", "DejaVu Serif"],
        GenericFamily::SansSerif => &["Helvetica", "Arial", "Liberation Sans", "DejaVu Sans"],
        GenericFamily::Monospace => &["Menlo", "Courier New", "Courier", "Liberation Mono", "DejaVu Sans Mono"],
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    /// Installed family used for layout and export.
    pub family: String,
    pub exact: bool,
    pub reason: &'static str,
}

pub struct FontSystem {
    pub fcx: FontContext,
    cache: HashMap<(String, GenericFamily), Resolution>,
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FontSystem {
    pub fn new() -> Self {
        Self { fcx: FontContext::new(), cache: HashMap::new() }
    }

    fn installed(&mut self, name: &str) -> Option<String> {
        self.fcx.collection.family_by_name(name).map(|f| f.name().to_string())
    }

    pub fn resolve(&mut self, spec: &FontSpec) -> Resolution {
        let key = (spec.family.to_string(), spec.generic);
        if let Some(r) = self.cache.get(&key) {
            return r.clone();
        }
        let wanted = spec.family.trim().to_lowercase();
        let resolution = if let Some(family) = self.installed(&spec.family) {
            Resolution { family, exact: true, reason: "installed" }
        } else if let Some(family) = ALIASES
            .iter()
            .find(|(k, _)| *k == wanted)
            .and_then(|(_, alts)| alts.iter().find_map(|a| self.installed(a)))
        {
            Resolution { family, exact: false, reason: "metric-compatible alias" }
        } else if let Some(family) = generic_candidates(spec.generic).iter().find_map(|a| self.installed(a)) {
            Resolution { family, exact: false, reason: "generic fallback" }
        } else {
            Resolution { family: String::new(), exact: false, reason: "system fallback" }
        };
        self.cache.insert(key, resolution.clone());
        resolution
    }

    /// Family stack for Parley: the resolved family, then the generic class
    /// so per-cluster fallback covers characters the family lacks.
    pub fn family_stack(&mut self, spec: &FontSpec) -> FontFamily<'static> {
        let r = self.resolve(spec);
        let generic = match spec.generic {
            GenericFamily::Serif => parley::GenericFamily::Serif,
            GenericFamily::SansSerif => parley::GenericFamily::SansSerif,
            GenericFamily::Monospace => parley::GenericFamily::Monospace,
        };
        let mut list = Vec::with_capacity(2);
        if !r.family.is_empty() {
            list.push(FontFamilyName::Named(Cow::Owned(r.family)));
        }
        list.push(FontFamilyName::Generic(generic));
        FontFamily::List(Cow::Owned(list))
    }

    /// All non-exact resolutions made so far, for the debug panel.
    pub fn substitutions(&self) -> Vec<(String, Resolution)> {
        let mut v: Vec<_> = self.cache.iter().filter(|(_, r)| !r.exact).map(|((f, _), r)| (f.clone(), r.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn unknown_family_falls_back_deterministically() {
        let mut fs = FontSystem::new();
        let spec = FontSpec { family: Arc::from("Definitely Not A Font"), generic: GenericFamily::Serif, ..Default::default() };
        let a = fs.resolve(&spec);
        let b = FontSystem::new().resolve(&spec);
        assert_eq!(a, b);
        assert!(!a.exact);
        assert_ne!(a.reason, "installed");
    }
}
