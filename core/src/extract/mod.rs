//! Reading embedded Metadata and Cover images out of Format files. Done once
//! at Import; afterwards the database is the source of truth (ADR 0003).

mod epub;
mod pdf;

use std::path::Path;

use crate::Result;
use crate::model::{FormatKind, Metadata};

/// What a Format file says about itself. `metadata.title` may be empty and
/// `metadata.authors` may be empty; Import fills in defaults.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Extracted {
    pub metadata: Metadata,
    /// The embedded cover image, still encoded (JPEG, PNG, …).
    pub cover: Option<Vec<u8>>,
}

pub fn extract(kind: FormatKind, path: &Path) -> Result<Extracted> {
    match kind {
        FormatKind::Epub => epub::extract(path),
        FormatKind::Pdf => pdf::extract(path),
    }
}

/// Returns the ISBN in `raw` as bare digits (plus a final "X" for ISBN-10),
/// or `None` if it is not a valid ISBN-10 or ISBN-13. Accepts "urn:isbn:"
/// prefixes, hyphens and spaces.
pub(crate) fn normalize_isbn(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let raw = raw
        .get(..9)
        .filter(|p| p.eq_ignore_ascii_case("urn:isbn:"))
        .map_or(raw, |_| &raw[9..]);
    let isbn: String = raw
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let digit = |c: char| c.to_digit(10);
    let valid = match isbn.len() {
        13 => {
            let digits: Option<Vec<u32>> = isbn.chars().map(digit).collect();
            digits.is_some_and(|d| {
                d.iter()
                    .enumerate()
                    .map(|(i, &n)| if i % 2 == 0 { n } else { 3 * n })
                    .sum::<u32>()
                    % 10
                    == 0
            })
        }
        10 => {
            let digits: Option<Vec<u32>> = isbn
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i == 9 && c == 'X' {
                        Some(10)
                    } else {
                        digit(c)
                    }
                })
                .collect();
            digits.is_some_and(|d| {
                d.iter()
                    .enumerate()
                    .map(|(i, &n)| (10 - i as u32) * n)
                    .sum::<u32>()
                    % 11
                    == 0
            })
        }
        _ => false,
    };
    valid.then_some(isbn)
}

/// Trims `s` and returns it, or `None` if nothing is left.
pub(crate) fn non_empty(s: impl AsRef<str>) -> Option<String> {
    let s = s.as_ref().trim();
    (!s.is_empty()).then(|| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_valid_isbns() {
        assert_eq!(
            normalize_isbn("978-0-441-01359-3").as_deref(),
            Some("9780441013593")
        );
        assert_eq!(
            normalize_isbn("urn:isbn:9780441013593").as_deref(),
            Some("9780441013593")
        );
        assert_eq!(
            normalize_isbn("0-8044-2957-x").as_deref(),
            Some("080442957X")
        );
        assert_eq!(normalize_isbn("9780441013594"), None);
        assert_eq!(normalize_isbn("urn:uuid:1234"), None);
        assert_eq!(normalize_isbn(""), None);
    }
}
