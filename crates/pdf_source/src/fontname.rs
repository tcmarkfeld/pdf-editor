//! Recovering family / weight / style from PDF BaseFont names such as
//! `ABCDEF+TimesNewRomanPS-BoldItalicMT` or `Arial,Bold`.

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedFontName {
    pub family: String,
    /// `None` when the name carries no weight information.
    pub weight: Option<u16>,
    pub italic: bool,
}

const WEIGHTS: &[(&str, u16)] = &[
    ("extrabold", 800),
    ("ultrabold", 800),
    ("semibold", 600),
    ("demibold", 600),
    ("extralight", 200),
    ("ultralight", 200),
    ("hairline", 100),
    ("black", 900),
    ("heavy", 900),
    ("bold", 700),
    ("medium", 500),
    ("light", 300),
    ("thin", 100),
    ("regular", 400),
    ("book", 400),
    ("normal", 400),
    ("roman", 400),
];

pub fn parse_font_name(raw: &str) -> ParsedFontName {
    let name = strip_subset_prefix(raw.trim());
    let (mut base, mut style) = match name.find(['-', ',']) {
        Some(i) => (name[..i].to_string(), name[i + 1..].to_string()),
        None => (name.to_string(), String::new()),
    };

    // Style words glued to the family without a separator ("ArialBold").
    if style.is_empty() {
        let lower = base.to_ascii_lowercase();
        for suffix in ["bolditalic", "boldoblique", "italic", "oblique", "bold"] {
            if lower.ends_with(suffix) && lower.len() > suffix.len() {
                style = base[base.len() - suffix.len()..].to_string();
                base.truncate(base.len() - suffix.len());
                break;
            }
        }
    }

    for suffix in ["PSMT", "MT", "PS"] {
        if base.len() > suffix.len() + 2 && base.ends_with(suffix) {
            base.truncate(base.len() - suffix.len());
            break;
        }
    }

    let lower_style = style.to_ascii_lowercase();
    let weight = WEIGHTS.iter().find(|(w, _)| lower_style.contains(w)).map(|&(_, v)| v);
    let italic = lower_style.contains("italic")
        || lower_style.contains("oblique")
        || lower_style.ends_with("it")
        || lower_style.ends_with("itmt");

    ParsedFontName { family: split_camel_case(&base), weight, italic }
}

fn strip_subset_prefix(name: &str) -> &str {
    let b = name.as_bytes();
    if b.len() > 7 && b[6] == b'+' && b[..6].iter().all(u8::is_ascii_uppercase) { &name[7..] } else { name }
}

/// "TimesNewRoman" -> "Times New Roman"; leaves existing spaces and
/// acronyms ("ITCAvantGarde" -> "ITC Avant Garde") intact.
fn split_camel_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || (prev.is_uppercase() && next_lower) || prev.is_ascii_digit() {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> (String, Option<u16>, bool) {
        let r = parse_font_name(s);
        (r.family, r.weight, r.italic)
    }

    #[test]
    fn common_producer_names() {
        assert_eq!(p("ABCDEF+TimesNewRomanPS-BoldItalicMT"), ("Times New Roman".into(), Some(700), true));
        assert_eq!(p("ArialMT"), ("Arial".into(), None, false));
        assert_eq!(p("Arial,Bold"), ("Arial".into(), Some(700), false));
        assert_eq!(p("Calibri-Light"), ("Calibri".into(), Some(300), false));
        assert_eq!(p("OpenSans-SemiBold"), ("Open Sans".into(), Some(600), false));
        assert_eq!(p("Helvetica-Oblique"), ("Helvetica".into(), None, true));
        assert_eq!(p("MinionPro-It"), ("Minion Pro".into(), None, true));
        assert_eq!(p("QWERTY+ArialBold"), ("Arial".into(), Some(700), false));
        assert_eq!(p("ITCAvantGarde-Book"), ("ITC Avant Garde".into(), Some(400), false));
    }
}
