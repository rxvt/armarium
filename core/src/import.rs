//! Import: copying EPUB and PDF files into the Library, as new Books or as
//! new Formats of existing ones. Never prompts; never modifies the source.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::extract::{self, Extracted};
use crate::library::Library;
use crate::model::{Author, Book, BookId, CoverSource, FormatKind};
use crate::{Result, cover, naming};

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Added(BookId),
    /// Added as a new Format of an existing Book.
    Merged(BookId),
    Skipped(SkipReason),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SkipReason {
    /// Byte-for-byte identical to a Format already in the Library.
    Duplicate,
    /// The matching Book already has a Format of this type.
    AlreadyHasFormat(FormatKind),
    Unsupported,
    Unreadable(String),
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::Duplicate => f.write_str("already in the library"),
            SkipReason::AlreadyHasFormat(kind) => {
                write!(f, "the book already has a {} file", kind.label())
            }
            SkipReason::Unsupported => f.write_str("not an EPUB or PDF"),
            SkipReason::Unreadable(reason) => f.write_str(reason),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ImportReport {
    /// One entry per file considered, in the order they were imported.
    pub items: Vec<(PathBuf, Outcome)>,
}

impl ImportReport {
    pub fn added(&self) -> usize {
        self.count(|o| matches!(o, Outcome::Added(_)))
    }

    pub fn merged(&self) -> usize {
        self.count(|o| matches!(o, Outcome::Merged(_)))
    }

    pub fn skipped(&self) -> usize {
        self.count(|o| matches!(o, Outcome::Skipped(_)))
    }

    fn count(&self, f: impl Fn(&Outcome) -> bool) -> usize {
        self.items.iter().filter(|(_, o)| f(o)).count()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Progress<'a> {
    /// Files finished so far.
    pub done: usize,
    pub total: usize,
    /// The file about to be imported.
    pub current: &'a Path,
}

impl Library {
    /// Imports `paths`: files, or folders searched recursively for EPUBs and
    /// PDFs. Creates the Library if needed. `progress` is called before each
    /// file. Per-file failures are reported in the result, not as errors.
    pub fn import(
        &mut self,
        paths: &[PathBuf],
        mut progress: impl FnMut(Progress),
    ) -> Result<ImportReport> {
        self.db_or_create()?;
        let mut report = ImportReport::default();
        let files = collect(paths, &mut report);
        let total = files.len() + report.items.len();
        for (path, kind) in files {
            progress(Progress {
                done: report.items.len(),
                total,
                current: &path,
            });
            let outcome = match kind {
                None => Outcome::Skipped(SkipReason::Unsupported),
                Some(kind) => self
                    .import_file(&path, kind)
                    .unwrap_or_else(|e| Outcome::Skipped(SkipReason::Unreadable(e.to_string()))),
            };
            report.items.push((path, outcome));
        }
        Ok(report)
    }

    fn import_file(&mut self, path: &Path, kind: FormatKind) -> Result<Outcome> {
        let size = fs::metadata(path)?.len();
        let db = self.db.as_ref().unwrap();
        for (book_id, format) in db.formats_with_size(size)? {
            if let Some(book) = db.book(book_id)?
                && same_contents(path, &self.format_path(&book, &format))
            {
                return Ok(Outcome::Skipped(SkipReason::Duplicate));
            }
        }

        let Extracted {
            mut metadata,
            cover,
        } = match extract::extract(kind, path) {
            Ok(extracted) => extracted,
            Err(crate::Error::Unreadable(reason)) => {
                return Ok(Outcome::Skipped(SkipReason::Unreadable(reason)));
            }
            Err(e) => return Err(e),
        };
        if metadata.title.trim().is_empty() {
            metadata.title = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if metadata.authors.is_empty() {
            metadata.authors.push(Author::new(naming::UNKNOWN_AUTHOR));
        }

        if let Some(id) = db.find_matching_book(&metadata)? {
            let book = db.book(id)?.expect("matched Book exists");
            if book.format(kind).is_some() {
                return Ok(Outcome::Skipped(SkipReason::AlreadyHasFormat(kind)));
            }
            self.add_format_file(&book, kind, path, size)?;
            self.offer_cover(&book, kind, cover.as_deref())?;
            return Ok(Outcome::Merged(id));
        }

        let id = db.insert_book(&metadata, now())?;
        let rel_dir = naming::book_dir(&metadata, id);
        db.set_book_path(id, &rel_dir)?;
        let book = db.book(id)?.expect("inserted Book exists");
        let dir = self.book_dir(&book);
        let filled = fs::create_dir_all(&dir)
            .map_err(Into::into)
            .and_then(|()| self.add_format_file(&book, kind, path, size))
            .and_then(|()| self.offer_cover(&book, kind, cover.as_deref()));
        if let Err(e) = filled {
            let _ = self.db.as_ref().unwrap().remove_book(id);
            let _ = fs::remove_dir_all(&dir);
            if let Some(author_dir) = dir.parent() {
                let _ = fs::remove_dir(author_dir); // only if now empty
            }
            return Err(e);
        }
        Ok(Outcome::Added(id))
    }

    /// Copies `src` into the Book's folder and records it as a Format.
    fn add_format_file(&self, book: &Book, kind: FormatKind, src: &Path, size: u64) -> Result<()> {
        let dir = self.book_dir(book);
        fs::create_dir_all(&dir)?;
        let file_name = naming::format_file_name(&book.metadata, kind);
        let dest = dir.join(&file_name);
        let tmp = dir.join(format!(".{file_name}.part"));
        let copied = fs::copy(src, &tmp).and_then(|_| fs::rename(&tmp, &dest));
        if let Err(e) = copied {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        if let Err(e) = self
            .db
            .as_ref()
            .unwrap()
            .add_format(book.id, kind, &file_name, size)
        {
            let _ = fs::remove_file(&dest);
            return Err(e.into());
        }
        Ok(())
    }

    /// Saves `encoded` as the Book's Cover if it has none, or if this Format's
    /// Cover outranks the current one. A Cover that will not decode is ignored.
    fn offer_cover(&self, book: &Book, kind: FormatKind, encoded: Option<&[u8]>) -> Result<()> {
        let Some(encoded) = encoded else {
            return Ok(());
        };
        if book.cover.is_some_and(|current| !current.yields_to(kind)) {
            return Ok(());
        }
        let dest = self.book_dir(book).join(naming::COVER_FILE_NAME);
        match cover::save(encoded, &dest) {
            Ok(()) => {
                let source = Some(CoverSource::from_format(kind));
                self.db
                    .as_ref()
                    .unwrap()
                    .set_cover_source(book.id, source)?;
                Ok(())
            }
            Err(crate::Error::Unreadable(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// Expands `paths` into the files to import, sorted within each folder.
/// Folders contribute only EPUBs and PDFs; files named directly are kept
/// even if unsupported, so the report can say why they were skipped.
/// Unreadable folders are recorded in `report` straight away.
fn collect(paths: &[PathBuf], report: &mut ImportReport) -> Vec<(PathBuf, Option<FormatKind>)> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk(path, &mut files, report);
        } else {
            files.push((path.clone(), kind_of(path)));
        }
    }
    files
}

fn walk(dir: &Path, files: &mut Vec<(PathBuf, Option<FormatKind>)>, report: &mut ImportReport) {
    let mut entries: Vec<_> = match fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            report.items.push((
                dir.to_owned(),
                Outcome::Skipped(SkipReason::Unreadable(e.to_string())),
            ));
            return;
        }
    };
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        // Do not follow symlinked folders, which could loop.
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir {
            walk(&path, files, report);
        } else if let Some(kind) = kind_of(&path)
            && path.is_file()
        {
            files.push((path, Some(kind)));
        }
    }
}

fn kind_of(path: &Path) -> Option<FormatKind> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(FormatKind::from_extension)
}

/// Whether two files have identical bytes. A Library file that cannot be
/// read counts as different.
fn same_contents(a: &Path, b: &Path) -> bool {
    fn compare(a: &Path, b: &Path) -> io::Result<bool> {
        let (mut a, mut b) = (File::open(a)?, File::open(b)?);
        let (mut buf_a, mut buf_b) = (vec![0; 64 * 1024], vec![0; 64 * 1024]);
        loop {
            let n = read_full(&mut a, &mut buf_a)?;
            if n != read_full(&mut b, &mut buf_b)? || buf_a[..n] != buf_b[..n] {
                return Ok(false);
            }
            if n == 0 {
                return Ok(true);
            }
        }
    }
    compare(a, b).unwrap_or(false)
}

/// Reads until `buf` is full or EOF, so chunk boundaries line up.
fn read_full(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{EpubFixture, PdfFixture, TempDir};
    use image::{DynamicImage, ImageFormat, RgbImage};

    struct Setup {
        _dir: TempDir,
        src: PathBuf,
        library: Library,
    }

    fn setup() -> Setup {
        let dir = TempDir::new();
        let src = dir.path().join("incoming");
        fs::create_dir_all(&src).unwrap();
        let library = Library::open(dir.path().join("Books")).unwrap();
        Setup {
            _dir: dir,
            src,
            library,
        }
    }

    impl Setup {
        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.src.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            path
        }

        fn import(&mut self, paths: &[PathBuf]) -> Vec<Outcome> {
            let report = self.library.import(paths, |_| {}).unwrap();
            report.items.into_iter().map(|(_, o)| o).collect()
        }

        fn only_book(&self) -> Book {
            let mut books = self.library.books().unwrap();
            assert_eq!(books.len(), 1, "{books:#?}");
            books.pop().unwrap()
        }
    }

    fn png(rgb: [u8; 3]) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(RgbImage::from_pixel(20, 30, image::Rgb(rgb)))
            .write_to(&mut io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn dune_epub() -> EpubFixture {
        EpubFixture::new()
            .metadata("<dc:title>Dune</dc:title><dc:creator>Frank Herbert</dc:creator>")
            .manifest(
                r#"<item id="c" href="c.png" media-type="image/png" properties="cover-image"/>"#,
            )
            .file("c.png", &png([200, 0, 0]))
    }

    fn dune_pdf() -> PdfFixture {
        PdfFixture::new()
            .info("Title", "DUNE")
            .info("Author", "frank herbert")
    }

    fn cover_pixel(s: &Setup, book: &Book) -> [u8; 3] {
        let path = s.library.cover_path(book).unwrap();
        image::open(path).unwrap().to_rgb8().get_pixel(10, 15).0
    }

    #[test]
    fn library_is_created_on_first_import_only() {
        let mut s = setup();
        assert!(!s.library.root().exists());
        assert!(s.library.books().unwrap().is_empty());

        s.import(&[]);
        assert!(s.library.root().join("library.db").exists());
    }

    #[test]
    fn adds_new_book_with_file_and_cover_in_layout() {
        let mut s = setup();
        let src = s.write("dune.epub", &dune_epub().build());
        let outcomes = s.import(std::slice::from_ref(&src));

        let book = s.only_book();
        assert_eq!(outcomes, vec![Outcome::Added(book.id)]);
        assert_eq!(book.path, format!("Frank Herbert/Dune ({})", book.id.0));
        let epub = book.format(FormatKind::Epub).unwrap();
        let dest = s.library.format_path(&book, epub);
        assert!(dest.ends_with("Dune - Frank Herbert.epub"));
        assert_eq!(fs::read(&dest).unwrap(), fs::read(&src).unwrap());
        assert_eq!(epub.size, fs::metadata(&src).unwrap().len());
        assert_eq!(book.cover, Some(CoverSource::Epub));
        assert!(cover_pixel(&s, &book)[0] > 180);
        assert!(src.exists(), "source must be copied, not moved");
    }

    #[test]
    fn skips_byte_identical_files() {
        let mut s = setup();
        let a = s.write("a.epub", &dune_epub().build());
        let b = s.write("copy/of/a.epub", &dune_epub().build());
        s.import(&[a]);
        assert_eq!(
            s.import(&[b]),
            vec![Outcome::Skipped(SkipReason::Duplicate)]
        );
        assert_eq!(s.library.books().unwrap().len(), 1);
    }

    #[test]
    fn merges_matching_title_and_author_as_new_format() {
        let mut s = setup();
        let epub = s.write("dune.epub", &dune_epub().build());
        let pdf = s.write("dune.pdf", &dune_pdf().build());
        let outcomes = s.import(&[epub, pdf]);

        let book = s.only_book();
        assert_eq!(
            outcomes,
            vec![Outcome::Added(book.id), Outcome::Merged(book.id)]
        );
        let pdf = book.format(FormatKind::Pdf).unwrap();
        assert_eq!(pdf.file_name, "Dune - Frank Herbert.pdf");
        assert!(s.library.format_path(&book, pdf).exists());
        // The EPUB's Cover stays.
        assert_eq!(book.cover, Some(CoverSource::Epub));
    }

    #[test]
    fn epub_cover_replaces_pdf_cover_but_not_a_users() {
        let mut s = setup();
        let pdf = s.write("dune.pdf", &dune_pdf().build());
        let epub = s.write("dune.epub", &dune_epub().build());
        s.import(&[pdf]);
        let book = s.only_book();
        assert_eq!(book.cover, Some(CoverSource::Pdf));
        assert_eq!(book.metadata.title, "DUNE");

        s.import(&[epub]);
        let book = s.only_book();
        assert_eq!(book.cover, Some(CoverSource::Epub));
        assert!(cover_pixel(&s, &book)[0] > 180);

        // A user-chosen Cover is never replaced.
        let mut s = setup();
        let pdf = s.write("dune.pdf", &dune_pdf().build());
        s.import(&[pdf]);
        let book = s.only_book();
        s.library
            .db
            .as_ref()
            .unwrap()
            .set_cover_source(book.id, Some(CoverSource::User))
            .unwrap();
        let before = fs::read(s.library.cover_path(&book).unwrap()).unwrap();
        let epub = s.write("dune.epub", &dune_epub().build());
        s.import(&[epub]);
        let book = s.only_book();
        assert_eq!(book.cover, Some(CoverSource::User));
        assert_eq!(
            fs::read(s.library.cover_path(&book).unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn skips_second_file_of_same_type_for_a_book() {
        let mut s = setup();
        let a = s.write("a.pdf", &dune_pdf().build());
        let b = s.write(
            "b.pdf",
            &dune_pdf().info("Subject", "different bytes").build(),
        );
        let outcomes = s.import(&[a, b]);
        assert_eq!(
            outcomes[1],
            Outcome::Skipped(SkipReason::AlreadyHasFormat(FormatKind::Pdf))
        );
    }

    #[test]
    fn fills_missing_title_from_file_name_and_author_as_unknown() {
        let mut s = setup();
        let src = s.write("The Raven.pdf", &PdfFixture::new().build());
        s.import(&[src]);
        let book = s.only_book();
        assert_eq!(book.metadata.title, "The Raven");
        assert_eq!(book.metadata.authors, vec![Author::new("Unknown")]);
        assert!(book.path.starts_with("Unknown/The Raven ("));
    }

    #[test]
    fn walks_folders_and_reports_unreadable_and_unsupported_files() {
        let mut s = setup();
        s.write("shelf/a/dune.epub", &dune_epub().build());
        s.write("shelf/b/broken.pdf", b"%PDF-garbage");
        s.write("shelf/notes.txt", b"ignored inside folders");
        let txt = s.write("readme.txt", b"named directly");

        let mut seen = Vec::new();
        let report = s
            .library
            .import(&[s.src.join("shelf"), txt], |p| {
                seen.push((p.done, p.total))
            })
            .unwrap();
        assert_eq!(seen, vec![(0, 3), (1, 3), (2, 3)]);

        let names: Vec<_> = report
            .items
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["dune.epub", "broken.pdf", "readme.txt"]);
        assert!(matches!(report.items[0].1, Outcome::Added(_)));
        assert!(
            matches!(&report.items[1].1, Outcome::Skipped(SkipReason::Unreadable(r)) if r.starts_with("PDF:"))
        );
        assert_eq!(report.items[2].1, Outcome::Skipped(SkipReason::Unsupported));
        assert_eq!(
            (report.added(), report.merged(), report.skipped()),
            (1, 0, 2)
        );
        assert_eq!(s.library.books().unwrap().len(), 1);
    }

    #[test]
    fn reopened_library_sees_imported_books() {
        let mut s = setup();
        let src = s.write("dune.epub", &dune_epub().build());
        s.import(&[src]);
        let reopened = Library::open(s.library.root()).unwrap();
        assert_eq!(reopened.books().unwrap().len(), 1);
    }
}
