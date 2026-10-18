//! Names of Book folders and Format files inside the Library:
//! `Author/Title (id)/Title - Author.ext`.

use crate::model::{BookId, FormatKind, Metadata};

/// Longest a title or author may be in a path component, in bytes, so
/// `Title - Author.ext` stays well under the 255-byte file name limit.
const MAX_PART_BYTES: usize = 100;

pub const UNKNOWN_AUTHOR: &str = "Unknown";

/// The Book's folder relative to the Library root, e.g. `Frank Herbert/Dune (12)`.
pub fn book_dir(metadata: &Metadata, id: BookId) -> String {
    format!(
        "{}/{} ({})",
        author_part(metadata),
        title_part(metadata),
        id.0
    )
}

/// The Format's file name, e.g. `Dune - Frank Herbert.epub`.
pub fn format_file_name(metadata: &Metadata, kind: FormatKind) -> String {
    format!(
        "{} - {}.{}",
        title_part(metadata),
        author_part(metadata),
        kind.extension()
    )
}

pub const COVER_FILE_NAME: &str = "cover.jpg";

fn title_part(metadata: &Metadata) -> String {
    sanitize(&metadata.title).unwrap_or_else(|| "Untitled".into())
}

fn author_part(metadata: &Metadata) -> String {
    metadata
        .authors
        .first()
        .and_then(|a| sanitize(&a.name))
        .unwrap_or_else(|| UNKNOWN_AUTHOR.into())
}

/// Makes `s` safe as one path component on Linux and on the FAT/exFAT
/// filesystems a Library might be copied to: drops control characters and
/// `/ \ : * ? " < > |`, collapses whitespace, trims leading/trailing dots and
/// spaces, and truncates. Returns `None` if nothing is left.
pub fn sanitize(s: &str) -> Option<String> {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut end = collapsed.len().min(MAX_PART_BYTES);
    while !collapsed.is_char_boundary(end) {
        end -= 1;
    }
    let trimmed = collapsed[..end].trim_matches(|c: char| c == '.' || c == ' ');
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    fn metadata(title: &str, authors: &[&str]) -> Metadata {
        Metadata {
            title: title.into(),
            authors: authors.iter().map(|&a| Author::new(a)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn uses_title_and_first_author() {
        let m = metadata("Good Omens", &["Terry Pratchett", "Neil Gaiman"]);
        assert_eq!(book_dir(&m, BookId(7)), "Terry Pratchett/Good Omens (7)");
        assert_eq!(
            format_file_name(&m, FormatKind::Pdf),
            "Good Omens - Terry Pratchett.pdf"
        );
    }

    #[test]
    fn falls_back_when_title_or_author_is_unusable() {
        let m = metadata("???", &[]);
        assert_eq!(book_dir(&m, BookId(1)), "Unknown/Untitled (1)");
        assert_eq!(
            format_file_name(&m, FormatKind::Epub),
            "Untitled - Unknown.epub"
        );
    }

    #[test]
    fn sanitizes_path_components() {
        assert_eq!(
            sanitize("AC/DC: The \"Story\"?").as_deref(),
            Some("ACDC The Story")
        );
        assert_eq!(
            sanitize("..hidden\n\tname..").as_deref(),
            Some("hidden name")
        );
        assert_eq!(sanitize(".."), None);
        // "é" is two bytes, so 100 bytes is 50 of them; never split one.
        assert_eq!(sanitize(&"é".repeat(200)).unwrap(), "é".repeat(50));
        assert_eq!(
            sanitize(&format!("a{}", "é".repeat(200))).unwrap().len(),
            99
        );
    }
}
