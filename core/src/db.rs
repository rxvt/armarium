//! `library.db`: the Library's SQLite database and the source of truth for
//! Metadata (ADR 0003). Hand-written SQL, numbered migrations, no ORM.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::model::{Author, Book, BookId, CoverSource, Format, FormatId, FormatKind, Metadata};

pub use rusqlite::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// Migration `n` (1-based) moves the schema from version `n - 1` to `n`,
/// tracked in `PRAGMA user_version`. Never edit a released migration; append.
const MIGRATIONS: &[&str] = &[
    // 1: initial schema.
    "
    CREATE TABLE books (
        id           INTEGER PRIMARY KEY,
        title        TEXT    NOT NULL,
        series       TEXT,
        series_index REAL,
        publisher    TEXT,
        published    TEXT,
        language     TEXT,
        isbn         TEXT,
        description  TEXT,
        date_added   INTEGER NOT NULL,
        path         TEXT    NOT NULL,
        match_key    TEXT    NOT NULL,
        cover        TEXT    -- CoverSource, NULL when there is no cover.jpg
    );
    CREATE INDEX books_match_key ON books (match_key);

    CREATE TABLE authors (
        id        INTEGER PRIMARY KEY,
        name      TEXT NOT NULL UNIQUE,
        sort_name TEXT NOT NULL
    );

    CREATE TABLE book_authors (
        book_id   INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
        author_id INTEGER NOT NULL REFERENCES authors (id),
        position  INTEGER NOT NULL,
        PRIMARY KEY (book_id, position)
    );
    CREATE INDEX book_authors_author ON book_authors (author_id);

    CREATE TABLE formats (
        id        INTEGER PRIMARY KEY,
        book_id   INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
        kind      TEXT    NOT NULL,
        file_name TEXT    NOT NULL,
        size      INTEGER NOT NULL,
        UNIQUE (book_id, kind)
    );
    CREATE INDEX formats_size ON formats (size);
    ",
];

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens (creating if needed) the database at `path` and migrates it.
    pub fn open(path: &Path) -> Result<Database> {
        Database::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Database> {
        Database::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Database> {
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn)?;
        Ok(Database { conn })
    }

    pub fn schema_version(&self) -> Result<usize> {
        schema_version(&self.conn)
    }

    /// Adds a Book with no Formats and an empty path; set the path with
    /// [`Database::set_book_path`] once the folder name (which includes the
    /// id) is known.
    pub fn insert_book(&self, metadata: &Metadata, date_added: i64) -> Result<BookId> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO books (title, series, series_index, publisher, published,
                                language, isbn, description, date_added, path, match_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '', ?10)",
            params![
                metadata.title,
                metadata.series,
                metadata.series_index,
                metadata.publisher,
                metadata.published,
                metadata.language,
                metadata.isbn,
                metadata.description,
                date_added,
                metadata.match_key(),
            ],
        )?;
        let id = BookId(tx.last_insert_rowid());
        // Keep sort names the user may already have edited.
        set_authors(&tx, id, &metadata.authors, false)?;
        tx.commit()?;
        Ok(id)
    }

    pub fn set_book_path(&self, id: BookId, path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE books SET path = ?1 WHERE id = ?2",
            params![path, id.0],
        )?;
        Ok(())
    }

    pub fn set_cover_source(&self, id: BookId, source: Option<CoverSource>) -> Result<()> {
        self.conn.execute(
            "UPDATE books SET cover = ?1 WHERE id = ?2",
            params![source.map(CoverSource::as_str), id.0],
        )?;
        Ok(())
    }

    /// Replaces a Book's Metadata. Author sort names given here overwrite the
    /// stored ones, since they come from the user.
    pub fn update_metadata(&self, id: BookId, metadata: &Metadata) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE books SET title = ?1, series = ?2, series_index = ?3, publisher = ?4,
                              published = ?5, language = ?6, isbn = ?7, description = ?8,
                              match_key = ?9
             WHERE id = ?10",
            params![
                metadata.title,
                metadata.series,
                metadata.series_index,
                metadata.publisher,
                metadata.published,
                metadata.language,
                metadata.isbn,
                metadata.description,
                metadata.match_key(),
                id.0,
            ],
        )?;
        tx.execute("DELETE FROM book_authors WHERE book_id = ?1", [id.0])?;
        set_authors(&tx, id, &metadata.authors, true)?;
        remove_orphan_authors(&tx)?;
        tx.commit()
    }

    pub fn add_format(
        &self,
        book: BookId,
        kind: FormatKind,
        file_name: &str,
        size: u64,
    ) -> Result<FormatId> {
        self.conn.execute(
            "INSERT INTO formats (book_id, kind, file_name, size) VALUES (?1, ?2, ?3, ?4)",
            params![book.0, kind.extension(), file_name, size as i64],
        )?;
        Ok(FormatId(self.conn.last_insert_rowid()))
    }

    pub fn set_format_file_name(&self, id: FormatId, file_name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE formats SET file_name = ?1 WHERE id = ?2",
            params![file_name, id.0],
        )?;
        Ok(())
    }

    pub fn remove_format(&self, id: FormatId) -> Result<()> {
        self.conn
            .execute("DELETE FROM formats WHERE id = ?1", [id.0])?;
        Ok(())
    }

    pub fn remove_book(&self, id: BookId) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM books WHERE id = ?1", [id.0])?;
        remove_orphan_authors(&tx)?;
        tx.commit()
    }

    /// Merges `other` into `keep`: each Format in `moves` becomes `keep`'s
    /// under its new file name, `keep`'s Date Added becomes the earlier of
    /// the two, and `other` is removed with any Formats left on it.
    /// `finish` runs before the change is committed; if it fails, nothing
    /// is changed.
    pub fn merge_book<E: From<Error>>(
        &self,
        keep: BookId,
        other: BookId,
        moves: &[(FormatId, String)],
        finish: impl FnOnce() -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        let tx = self.conn.unchecked_transaction()?;
        for (format, file_name) in moves {
            tx.execute(
                "UPDATE formats SET book_id = ?1, file_name = ?2 WHERE id = ?3 AND book_id = ?4",
                params![keep.0, file_name, format.0, other.0],
            )?;
        }
        tx.execute(
            "UPDATE books SET date_added = min(date_added,
                 (SELECT date_added FROM books WHERE id = ?2))
             WHERE id = ?1",
            params![keep.0, other.0],
        )?;
        tx.execute("DELETE FROM books WHERE id = ?1", [other.0])?;
        remove_orphan_authors(&tx)?;
        finish()?;
        tx.commit()?;
        Ok(())
    }

    /// The Book whose title and first author match `metadata`'s, if any.
    pub fn find_matching_book(&self, metadata: &Metadata) -> Result<Option<BookId>> {
        self.conn
            .query_row(
                "SELECT id FROM books WHERE match_key = ?1 ORDER BY id LIMIT 1",
                [metadata.match_key()],
                |row| row.get(0).map(BookId),
            )
            .optional()
    }

    /// All Formats of exactly `size` bytes, as candidates for a byte-for-byte
    /// duplicate check.
    pub fn formats_with_size(&self, size: u64) -> Result<Vec<(BookId, Format)>> {
        let mut stmt = self.conn.prepare(
            "SELECT book_id, id, kind, file_name, size FROM formats WHERE size = ?1 ORDER BY id",
        )?;
        stmt.query_map([size as i64], |row| {
            Ok((BookId(row.get(0)?), format_from_row(row, 1)?))
        })?
        .collect()
    }

    pub fn book(&self, id: BookId) -> Result<Option<Book>> {
        Ok(self.load_books(Some(id))?.pop())
    }

    /// Every Book, newest first.
    pub fn books(&self) -> Result<Vec<Book>> {
        self.load_books(None)
    }

    fn load_books(&self, only: Option<BookId>) -> Result<Vec<Book>> {
        let filter = only.map(|id| id.0);
        let mut books: Vec<Book> = self
            .conn
            .prepare(
                "SELECT id, title, series, series_index, publisher, published, language,
                        isbn, description, date_added, path, cover
                 FROM books WHERE ?1 IS NULL OR id = ?1
                 ORDER BY date_added DESC, id DESC",
            )?
            .query_map([filter], |row| {
                Ok(Book {
                    id: BookId(row.get(0)?),
                    metadata: Metadata {
                        title: row.get(1)?,
                        authors: Vec::new(),
                        series: row.get(2)?,
                        series_index: row.get(3)?,
                        publisher: row.get(4)?,
                        published: row.get(5)?,
                        language: row.get(6)?,
                        isbn: row.get(7)?,
                        description: row.get(8)?,
                    },
                    date_added: row.get(9)?,
                    path: row.get(10)?,
                    cover: row
                        .get::<_, Option<String>>(11)?
                        .as_deref()
                        .and_then(CoverSource::parse),
                    formats: Vec::new(),
                })
            })?
            .collect::<Result<_>>()?;

        let index: HashMap<BookId, usize> =
            books.iter().enumerate().map(|(i, b)| (b.id, i)).collect();

        let mut stmt = self.conn.prepare(
            "SELECT ba.book_id, a.name, a.sort_name
             FROM book_authors ba JOIN authors a ON a.id = ba.author_id
             WHERE ?1 IS NULL OR ba.book_id = ?1
             ORDER BY ba.book_id, ba.position",
        )?;
        let mut rows = stmt.query([filter])?;
        while let Some(row) = rows.next()? {
            if let Some(&i) = index.get(&BookId(row.get(0)?)) {
                books[i].metadata.authors.push(Author {
                    name: row.get(1)?,
                    sort_name: row.get(2)?,
                });
            }
        }

        let mut stmt = self.conn.prepare(
            "SELECT book_id, id, kind, file_name, size FROM formats
             WHERE ?1 IS NULL OR book_id = ?1
             ORDER BY book_id, kind",
        )?;
        let mut rows = stmt.query([filter])?;
        while let Some(row) = rows.next()? {
            if let Some(&i) = index.get(&BookId(row.get(0)?)) {
                books[i].formats.push(format_from_row(row, 1)?);
            }
        }

        Ok(books)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current = schema_version(conn)?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<usize> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version as usize)
}

fn set_authors(
    conn: &Connection,
    book: BookId,
    authors: &[Author],
    overwrite_sort_names: bool,
) -> Result<()> {
    let upsert = if overwrite_sort_names {
        "INSERT INTO authors (name, sort_name) VALUES (?1, ?2)
         ON CONFLICT (name) DO UPDATE SET sort_name = excluded.sort_name"
    } else {
        "INSERT INTO authors (name, sort_name) VALUES (?1, ?2)
         ON CONFLICT (name) DO NOTHING"
    };
    for (position, author) in authors.iter().enumerate() {
        conn.execute(upsert, params![author.name, author.sort_name])?;
        conn.execute(
            "INSERT INTO book_authors (book_id, author_id, position)
             SELECT ?1, id, ?2 FROM authors WHERE name = ?3",
            params![book.0, position as i64, author.name],
        )?;
    }
    Ok(())
}

fn remove_orphan_authors(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM authors WHERE id NOT IN (SELECT author_id FROM book_authors)",
        [],
    )?;
    Ok(())
}

fn format_from_row(row: &Row, first: usize) -> Result<Format> {
    let kind: String = row.get(first + 1)?;
    Ok(Format {
        id: FormatId(row.get(first)?),
        kind: FormatKind::from_extension(&kind).ok_or_else(|| {
            Error::FromSqlConversionFailure(
                first + 1,
                rusqlite::types::Type::Text,
                format!("unknown format kind {kind:?}").into(),
            )
        })?,
        file_name: row.get(first + 2)?,
        size: row.get::<_, i64>(first + 3)? as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dune() -> Metadata {
        Metadata {
            title: "Dune".into(),
            authors: vec![Author::new("Frank Herbert")],
            series: Some("Dune".into()),
            series_index: Some(1.0),
            publisher: Some("Chilton".into()),
            published: Some("1965".into()),
            language: Some("en".into()),
            isbn: Some("9780441013593".into()),
            description: Some("Spice.".into()),
        }
    }

    fn book_by(db: &Database, title: &str, authors: &[&str], date_added: i64) -> BookId {
        let metadata = Metadata {
            title: title.into(),
            authors: authors.iter().map(|&a| Author::new(a)).collect(),
            ..Default::default()
        };
        db.insert_book(&metadata, date_added).unwrap()
    }

    #[test]
    fn migrates_new_database_to_latest() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
    }

    #[test]
    fn reopening_keeps_data_and_does_not_rerun_migrations() {
        let dir = crate::fixtures::TempDir::new();
        let path = dir.path().join("library.db");

        let id = Database::open(&path)
            .unwrap()
            .insert_book(&dune(), 1)
            .unwrap();
        let db = Database::open(&path).unwrap();
        assert_eq!(db.book(id).unwrap().unwrap().metadata, dune());
    }

    #[test]
    fn book_round_trips_metadata_path_and_formats() {
        let db = Database::open_in_memory().unwrap();
        let id = db.insert_book(&dune(), 1_700_000_000).unwrap();
        db.set_book_path(id, "Frank Herbert/Dune (1)").unwrap();
        let pdf = db
            .add_format(id, FormatKind::Pdf, "Dune - Frank Herbert.pdf", 20)
            .unwrap();
        let epub = db
            .add_format(id, FormatKind::Epub, "Dune - Frank Herbert.epub", 10)
            .unwrap();

        let book = db.book(id).unwrap().unwrap();
        assert_eq!(book.metadata, dune());
        assert_eq!(book.date_added, 1_700_000_000);
        assert_eq!(book.path, "Frank Herbert/Dune (1)");
        assert_eq!(book.cover, None);
        assert_eq!(
            book.formats,
            vec![
                Format {
                    id: epub,
                    kind: FormatKind::Epub,
                    file_name: "Dune - Frank Herbert.epub".into(),
                    size: 10
                },
                Format {
                    id: pdf,
                    kind: FormatKind::Pdf,
                    file_name: "Dune - Frank Herbert.pdf".into(),
                    size: 20
                },
            ]
        );
        assert_eq!(db.book(BookId(999)).unwrap(), None);
    }

    #[test]
    fn books_are_newest_first_with_authors_in_order() {
        let db = Database::open_in_memory().unwrap();
        let old = book_by(&db, "Good Omens", &["Terry Pratchett", "Neil Gaiman"], 1);
        let new = book_by(&db, "Coraline", &["Neil Gaiman"], 2);

        let books = db.books().unwrap();
        assert_eq!(
            books.iter().map(|b| b.id).collect::<Vec<_>>(),
            vec![new, old]
        );
        let names: Vec<_> = books[1]
            .metadata
            .authors
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["Terry Pratchett", "Neil Gaiman"]);
    }

    #[test]
    fn cover_source_round_trips() {
        let db = Database::open_in_memory().unwrap();
        let id = db.insert_book(&dune(), 1).unwrap();
        for source in [Some(CoverSource::Pdf), Some(CoverSource::User), None] {
            db.set_cover_source(id, source).unwrap();
            assert_eq!(db.book(id).unwrap().unwrap().cover, source);
        }
    }

    #[test]
    fn at_most_one_format_per_kind() {
        let db = Database::open_in_memory().unwrap();
        let id = db.insert_book(&dune(), 1).unwrap();
        db.add_format(id, FormatKind::Epub, "a.epub", 1).unwrap();
        assert!(db.add_format(id, FormatKind::Epub, "b.epub", 2).is_err());
    }

    #[test]
    fn finds_matching_book_by_normalized_title_and_first_author() {
        let db = Database::open_in_memory().unwrap();
        let id = db.insert_book(&dune(), 1).unwrap();
        let mut other = Metadata {
            title: "DUNE!".into(),
            authors: vec![Author::new("frank  herbert")],
            ..Default::default()
        };
        assert_eq!(db.find_matching_book(&other).unwrap(), Some(id));
        other.authors = vec![Author::new("Brian Herbert")];
        assert_eq!(db.find_matching_book(&other).unwrap(), None);
    }

    #[test]
    fn formats_with_size_lists_candidates() {
        let db = Database::open_in_memory().unwrap();
        let a = book_by(&db, "A", &[], 1);
        let b = book_by(&db, "B", &[], 1);
        let fa = db.add_format(a, FormatKind::Epub, "a.epub", 100).unwrap();
        db.add_format(a, FormatKind::Pdf, "a.pdf", 200).unwrap();
        let fb = db.add_format(b, FormatKind::Pdf, "b.pdf", 100).unwrap();

        let found: Vec<_> = db
            .formats_with_size(100)
            .unwrap()
            .into_iter()
            .map(|(book, f)| (book, f.id))
            .collect();
        assert_eq!(found, vec![(a, fa), (b, fb)]);
    }

    #[test]
    fn import_keeps_edited_sort_name_but_edit_overwrites_it() {
        let db = Database::open_in_memory().unwrap();
        let first = book_by(&db, "Dune", &["Frank Herbert"], 1);
        let mut edited = db.book(first).unwrap().unwrap().metadata;
        edited.authors[0].sort_name = "HERBERT".into();
        db.update_metadata(first, &edited).unwrap();

        let second = book_by(&db, "Dune Messiah", &["Frank Herbert"], 2);
        let book = db.book(second).unwrap().unwrap();
        assert_eq!(book.metadata.authors[0].sort_name, "HERBERT");
    }

    #[test]
    fn update_metadata_replaces_fields_and_match_key() {
        let db = Database::open_in_memory().unwrap();
        let id = db.insert_book(&dune(), 1).unwrap();
        let mut changed = dune();
        changed.title = "Dune Messiah".into();
        changed.series_index = Some(2.0);
        changed.isbn = None;
        db.update_metadata(id, &changed).unwrap();

        assert_eq!(db.book(id).unwrap().unwrap().metadata, changed);
        assert_eq!(db.find_matching_book(&changed).unwrap(), Some(id));
        assert_eq!(db.find_matching_book(&dune()).unwrap(), None);
    }

    #[test]
    fn removing_books_and_formats_cleans_up_orphan_authors() {
        let db = Database::open_in_memory().unwrap();
        let id = db.insert_book(&dune(), 1).unwrap();
        let other = book_by(&db, "Coraline", &["Neil Gaiman"], 1);
        let f = db.add_format(id, FormatKind::Epub, "a.epub", 1).unwrap();

        db.remove_format(f).unwrap();
        assert!(db.book(id).unwrap().unwrap().formats.is_empty());

        db.remove_book(id).unwrap();
        assert_eq!(db.book(id).unwrap(), None);
        let authors: i64 = db
            .conn
            .query_row("SELECT count(*) FROM authors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(authors, 1);
        assert!(db.book(other).unwrap().is_some());
    }

    #[test]
    fn merge_book_moves_formats_and_keeps_the_earliest_date_added() {
        let db = Database::open_in_memory().unwrap();
        let keep = book_by(&db, "Dune", &["Frank Herbert"], 20);
        let other = book_by(&db, "Dune_ Fr", &["Unknown"], 10);
        db.add_format(keep, FormatKind::Epub, "keep.epub", 1)
            .unwrap();
        let pdf = db
            .add_format(other, FormatKind::Pdf, "other.pdf", 2)
            .unwrap();
        db.add_format(other, FormatKind::Epub, "other.epub", 3)
            .unwrap();

        db.merge_book(keep, other, &[(pdf, "keep.pdf".into())], || {
            Ok::<_, Error>(())
        })
        .unwrap();

        let book = db.book(keep).unwrap().unwrap();
        assert_eq!(book.date_added, 10);
        let names: Vec<_> = book.formats.iter().map(|f| f.file_name.as_str()).collect();
        assert_eq!(names, ["keep.epub", "keep.pdf"]);
        assert_eq!(db.book(other).unwrap(), None);
        let authors: Vec<String> = db
            .conn
            .prepare("SELECT name FROM authors")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(authors, ["Frank Herbert"]);
    }

    #[test]
    fn a_failed_merge_book_changes_nothing() {
        let db = Database::open_in_memory().unwrap();
        let keep = book_by(&db, "Dune", &[], 20);
        let other = book_by(&db, "Dune_ Fr", &[], 10);
        let pdf = db
            .add_format(other, FormatKind::Pdf, "other.pdf", 2)
            .unwrap();
        let before = db.books().unwrap();

        let result = db.merge_book(keep, other, &[(pdf, "keep.pdf".into())], || {
            Err(crate::Error::Invalid("no".into()))
        });

        assert!(result.is_err());
        assert_eq!(db.books().unwrap(), before);
    }
}
