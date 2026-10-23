//! The Library's domain types. See GLOSSARY.md for the vocabulary.

use std::cmp::Ordering;

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

    pub fn sort_keys(&self) -> SortKeys {
        let m = &self.metadata;
        let authors: Vec<&str> = m.authors.iter().map(|a| a.sort_name.as_str()).collect();
        let formats: Vec<&str> = self.formats.iter().map(|f| f.kind.label()).collect();
        SortKeys {
            id: self.id.0,
            title: text::title_sort_key(&m.title),
            authors: text::fold(&authors.join("\u{1f}")),
            series: m
                .series
                .as_deref()
                .map(|s| (text::title_sort_key(s), m.series_index.unwrap_or(0.0))),
            formats: formats.join(" "),
            date_added: self.date_added,
        }
    }
}

/// The orders the Book list can be sorted in. Each compares ascending; the
/// UI reverses it for descending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Authors,
    Series,
    Formats,
    DateAdded,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [
        SortKey::Title,
        SortKey::Authors,
        SortKey::Series,
        SortKey::Formats,
        SortKey::DateAdded,
    ];

    /// Stable name, used in settings and action states.
    pub fn as_str(self) -> &'static str {
        match self {
            SortKey::Title => "title",
            SortKey::Authors => "authors",
            SortKey::Series => "series",
            SortKey::Formats => "formats",
            SortKey::DateAdded => "date-added",
        }
    }

    pub fn parse(s: &str) -> Option<SortKey> {
        SortKey::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Whether choosing this key should start descending: newest first for
    /// Date added, A to Z for the rest.
    pub fn starts_descending(self) -> bool {
        self == SortKey::DateAdded
    }
}

/// A Book's sort keys, computed once so that sorting a large Library does
/// not fold text on every comparison. From [`Book::sort_keys`].
#[derive(Debug, Clone, Default)]
pub struct SortKeys {
    id: i64,
    title: String,
    authors: String,
    /// Books outside a series sort after those in one.
    series: Option<(String, f64)>,
    formats: String,
    date_added: i64,
}

impl SortKeys {
    /// Compares by `key`, then by title, then by id, so the order is total.
    /// Date added ties go straight to the id, i.e. Import order, since a
    /// whole Import usually lands in the same second.
    pub fn compare(&self, other: &SortKeys, key: SortKey) -> Ordering {
        let primary = match key {
            SortKey::Title => Ordering::Equal,
            SortKey::Authors => self.authors.cmp(&other.authors),
            SortKey::Series => match (&self.series, &other.series) {
                (Some((a, i)), Some((b, j))) => a.cmp(b).then(i.total_cmp(j)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            SortKey::Formats => self.formats.cmp(&other.formats),
            SortKey::DateAdded => self
                .date_added
                .cmp(&other.date_added)
                .then(self.id.cmp(&other.id)),
        };
        primary
            .then_with(|| self.title.cmp(&other.title))
            .then(self.id.cmp(&other.id))
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
    fn sort_key_round_trips_name() {
        for key in SortKey::ALL {
            assert_eq!(SortKey::parse(key.as_str()), Some(key));
        }
        assert_eq!(SortKey::parse("size"), None);
    }

    fn book(id: i64, title: &str, author: &str) -> Book {
        Book {
            id: BookId(id),
            metadata: Metadata {
                title: title.into(),
                authors: vec![Author::new(author)],
                ..Default::default()
            },
            date_added: 0,
            path: String::new(),
            cover: None,
            formats: Vec::new(),
        }
    }

    fn sorted_titles(books: &[Book], key: SortKey) -> Vec<String> {
        let mut keyed: Vec<(SortKeys, &str)> = books
            .iter()
            .map(|b| (b.sort_keys(), b.metadata.title.as_str()))
            .collect();
        keyed.sort_by(|a, b| a.0.compare(&b.0, key));
        keyed.into_iter().map(|(_, t)| t.to_owned()).collect()
    }

    #[test]
    fn sorts_titles_ignoring_articles_and_accents() {
        let books = [
            book(1, "The Hobbit", "J. R. R. Tolkien"),
            book(2, "Émile", "Jean-Jacques Rousseau"),
            book(3, "Dune", "Frank Herbert"),
        ];
        assert_eq!(
            sorted_titles(&books, SortKey::Title),
            ["Dune", "Émile", "The Hobbit"]
        );
    }

    #[test]
    fn sorts_authors_by_sort_name_then_title() {
        let books = [
            book(1, "Persuasion", "Jane Austen"),
            book(2, "Dune", "Frank Herbert"),
            book(3, "Emma", "Jane Austen"),
        ];
        assert_eq!(
            sorted_titles(&books, SortKey::Authors),
            ["Emma", "Persuasion", "Dune"]
        );
    }

    #[test]
    fn sorts_series_by_name_then_numeric_index_with_no_series_last() {
        let mut books = [
            book(1, "Standalone", "A"),
            book(2, "Tenth", "A"),
            book(3, "Second", "A"),
            book(4, "Other", "A"),
        ];
        for (i, series, index) in [(1, "Alpha", 10.0), (2, "Alpha", 2.0), (3, "Beta", 1.0)] {
            books[i].metadata.series = Some(series.into());
            books[i].metadata.series_index = Some(index);
        }
        assert_eq!(
            sorted_titles(&books, SortKey::Series),
            ["Second", "Tenth", "Other", "Standalone"]
        );
    }

    #[test]
    fn sorts_date_added_with_ties_in_import_order() {
        let mut books = [book(1, "B", "X"), book(2, "A", "X"), book(3, "C", "X")];
        books[0].date_added = 5;
        books[1].date_added = 5;
        books[2].date_added = 1;
        assert_eq!(sorted_titles(&books, SortKey::DateAdded), ["C", "B", "A"]);
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
