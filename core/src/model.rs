//! The Library's domain types. See GLOSSARY.md for the vocabulary.

use crate::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FormatId(pub i64);

/// The file types a Format can have. Adding a variant here (plus its
/// Metadata reader) is all a new file type should need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FormatKind {
    Epub,
    Pdf,
}

impl FormatKind {
    pub const ALL: [FormatKind; 2] = [FormatKind::Epub, FormatKind::Pdf];

    /// Lowercase file extension, also used as the database value.
    pub fn extension(self) -> &'static str {
        match self {
            FormatKind::Epub => "epub",
            FormatKind::Pdf => "pdf",
        }
    }

    pub fn from_extension(ext: &str) -> Option<FormatKind> {
        FormatKind::ALL
            .into_iter()
            .find(|kind| kind.extension().eq_ignore_ascii_case(ext))
    }

    pub fn label(self) -> &'static str {
        match self {
            FormatKind::Epub => "EPUB",
            FormatKind::Pdf => "PDF",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Author {
    pub name: String,
    /// E.g. "Herbert, Frank". Derived on Import, editable afterwards.
    pub sort_name: String,
}

impl Author {
    /// An author with an auto-derived sort name.
    pub fn new(name: impl Into<String>) -> Author {
        let name = name.into();
        let sort_name = text::author_sort_name(&name);
        Author { name, sort_name }
    }
}

/// The descriptive facts the Library records about a Book.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub title: String,
    /// In display order; the first author names the Book's folder.
    pub authors: Vec<Author>,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub publisher: Option<String>,
    /// As found or entered, e.g. "1965" or "1965-08-01".
    pub published: Option<String>,
    pub language: Option<String>,
    pub isbn: Option<String>,
    pub description: Option<String>,
}

impl Metadata {
    /// Key under which two Books count as the same work: normalized title
    /// plus normalized first author.
    pub fn match_key(&self) -> String {
        let author = self.authors.first().map_or("", |a| a.name.as_str());
        format!(
            "{}\u{1f}{}",
            text::match_key(&self.title),
            text::match_key(author)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Format {
    pub id: FormatId,
    pub kind: FormatKind,
    /// File name inside the Book's folder.
    pub file_name: String,
    pub size: u64,
}

/// Where a Book's Cover came from. An EPUB's Cover replaces one rendered from
/// a PDF; nothing replaces one the user chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverSource {
    Epub,
    Pdf,
    User,
}

impl CoverSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CoverSource::Epub => "epub",
            CoverSource::Pdf => "pdf",
            CoverSource::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<CoverSource> {
        [CoverSource::Epub, CoverSource::Pdf, CoverSource::User]
            .into_iter()
            .find(|c| c.as_str() == s)
    }

    pub fn from_format(kind: FormatKind) -> CoverSource {
        match kind {
            FormatKind::Epub => CoverSource::Epub,
            FormatKind::Pdf => CoverSource::Pdf,
        }
    }

    /// Whether a Cover extracted from `kind` should replace this one.
    pub fn yields_to(self, kind: FormatKind) -> bool {
        self == CoverSource::Pdf && kind == FormatKind::Epub
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Book {
    pub id: BookId,
    pub metadata: Metadata,
    /// Unix seconds.
    pub date_added: i64,
    /// The Book's folder, relative to the Library root.
    pub path: String,
    /// `None` when the Book has no `cover.jpg` and shows a placeholder.
    pub cover: Option<CoverSource>,
    pub formats: Vec<Format>,
}

impl Book {
    pub fn format(&self, kind: FormatKind) -> Option<&Format> {
        self.formats.iter().find(|f| f.kind == kind)
    }

    /// Text that live search looks in: title, authors and series, folded with
    /// [`text::fold`]. Compute once per Book and reuse while the user types.
    pub fn search_text(&self) -> String {
        let m = &self.metadata;
        let mut parts = vec![m.title.as_str()];
        parts.extend(m.authors.iter().map(|a| a.name.as_str()));
        parts.extend(m.series.as_deref());
        text::fold(&parts.join("\n"))
    }
}

/// Whether `search_text` (from [`Book::search_text`]) matches `query`: every
/// whitespace-separated word of the query must occur in it, ignoring case and
/// diacritics. An empty query matches everything.
pub fn search_matches(search_text: &str, query: &str) -> bool {
    text::fold(query)
        .split_whitespace()
        .all(|word| search_text.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_kind_round_trips_extension() {
        for kind in FormatKind::ALL {
            assert_eq!(FormatKind::from_extension(kind.extension()), Some(kind));
        }
        assert_eq!(FormatKind::from_extension("EPUB"), Some(FormatKind::Epub));
        assert_eq!(FormatKind::from_extension("mobi"), None);
    }

    #[test]
    fn search_matches_every_word_across_fields_ignoring_accents() {
        let book = Book {
            id: BookId(1),
            metadata: Metadata {
                title: "Jane Eyre".into(),
                authors: vec![Author::new("Charlotte Brontë")],
                series: Some("Brontë Classics".into()),
                publisher: Some("Smith, Elder".into()),
                ..Default::default()
            },
            date_added: 0,
            path: String::new(),
            cover: None,
            formats: Vec::new(),
        };
        let text = book.search_text();
        assert!(search_matches(&text, ""));
        assert!(search_matches(&text, "bronte"));
        assert!(search_matches(&text, "  EYRE  charl "));
        assert!(search_matches(&text, "classics"));
        assert!(!search_matches(&text, "smith"), "publisher is not searched");
        assert!(!search_matches(&text, "eyre emily"));
    }

    #[test]
    fn match_key_uses_title_and_first_author() {
        let a = Metadata {
            title: "Dune".into(),
            authors: vec![Author::new("Frank Herbert"), Author::new("Someone Else")],
            ..Default::default()
        };
        let b = Metadata {
            title: "DUNE.".into(),
            authors: vec![Author::new("frank herbert")],
            ..Default::default()
        };
        let c = Metadata {
            title: "Dune".into(),
            authors: vec![Author::new("Brian Herbert")],
            ..Default::default()
        };
        assert_eq!(a.match_key(), b.match_key());
        assert_ne!(a.match_key(), c.match_key());
    }
}
