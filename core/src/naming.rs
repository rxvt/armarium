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

const DEVICE_TAG_PREFIX: &str = "[armarium-";

/// The name of a Book's copy on a Device, tagged with its id so it can be
/// found again ([ADR 0005]), e.g. `Dune - Frank Herbert [armarium-42].azw3`.
///
/// [ADR 0005]: ../../docs/adr/0005-device-books-matched-by-filename-tag.md
#[cfg_attr(not(test), expect(dead_code, reason = "Device::send will use it"))]
pub fn device_file_name(metadata: &Metadata, id: BookId, extension: &str) -> String {
    format!(
        "{} - {} {DEVICE_TAG_PREFIX}{}].{extension}",
        title_part(metadata),
        author_part(metadata),
        id.0
    )
}

/// Whether `file_name` is a partial copy left by an interrupted Send:
/// `.armarium-<id>.part`. The leading dot keeps the Kindle from indexing it.
pub fn is_device_part_file(file_name: &str) -> bool {
    file_name
        .strip_prefix(".armarium-")
        .and_then(|rest| rest.strip_suffix(".part"))
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// The Book id tagged in a file name made by [`device_file_name`], if any.
pub fn book_id_in(file_name: &str) -> Option<BookId> {
    let (stem, _extension) = file_name.rsplit_once('.')?;
    let (_, tag) = stem.strip_suffix(']')?.rsplit_once(DEVICE_TAG_PREFIX)?;
    if tag.is_empty() || !tag.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    tag.parse().ok().map(BookId)
}

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
    fn device_file_names_carry_the_book_id() {
        let m = metadata("Dune", &["Frank Herbert"]);
        let name = device_file_name(&m, BookId(42), "azw3");
        assert_eq!(name, "Dune - Frank Herbert [armarium-42].azw3");
        assert_eq!(book_id_in(&name), Some(BookId(42)));
    }

    #[test]
    fn device_file_names_round_trip_awkward_titles() {
        let long = "é".repeat(200);
        for (title, author) in [
            ("Notes [armarium-7]", "Anon"),
            (long.as_str(), long.as_str()),
            ("Война и мир", "Лев Толстой"),
            ("Dr. Who?", "..."),
        ] {
            let name = device_file_name(&metadata(title, &[author]), BookId(i64::MAX), "pdf");
            assert!(name.len() <= 255, "{name}");
            assert_eq!(book_id_in(&name), Some(BookId(i64::MAX)), "{name}");
        }
    }

    #[test]
    fn untagged_and_look_alike_names_have_no_book_id() {
        for name in [
            "Dune - Frank Herbert.azw3",
            "Dune [armarium-].azw3",
            "Dune [armarium-x].azw3",
            "Dune [armarium-+42].azw3",
            "Dune [armarium--1].azw3",
            "Dune [armarium-42]",
            "Dune [armarium-42] copy.azw3",
            "Dune [Armarium-42].azw3",
            "Dune (armarium-42).azw3",
            "Dune [armarium-99999999999999999999].azw3",
        ] {
            assert_eq!(book_id_in(name), None, "{name}");
        }
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
