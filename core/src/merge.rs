//! Merge: joining Books that are the same work into one, which keeps its own
//! Metadata and Cover and gains the others' Formats ([ADR 0008]).
//!
//! [ADR 0008]: ../../docs/adr/0008-split-books-are-merged-by-hand.md

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::edit::remove_if_empty;
use crate::library::Library;
use crate::model::{Book, BookId, Format, FormatId, FormatKind};
use crate::{Error, Result, naming};

/// What Merging Books into a kept one does with their Formats.
#[derive(Debug, Clone, PartialEq)]
pub struct MergePlan<'a> {
    /// Formats that move to the kept Book.
    pub moves: Vec<(&'a Book, &'a Format)>,
    /// Formats of a type the kept Book already has, or gains from a Book
    /// before them, which go to the Trash.
    pub duplicates: Vec<(&'a Book, &'a Format)>,
}

/// The plan for Merging `others`, in order, into `keep`.
pub fn plan<'a>(keep: &Book, others: &'a [Book]) -> MergePlan<'a> {
    let mut kinds: HashSet<FormatKind> = keep.formats.iter().map(|f| f.kind).collect();
    let mut plan = MergePlan {
        moves: Vec::new(),
        duplicates: Vec::new(),
    };
    for book in others {
        for format in &book.formats {
            if kinds.insert(format.kind) {
                plan.moves.push((book, format));
            } else {
                plan.duplicates.push((book, format));
            }
        }
    }
    plan
}

/// Which of `books` to keep unless the user picks another: the first with
/// an EPUB, as its Metadata is usually the better, or else the first.
pub fn default_keep(books: &[Book]) -> usize {
    books
        .iter()
        .position(|b| b.format(FormatKind::Epub).is_some())
        .unwrap_or(0)
}

impl Library {
    /// Merges each of `others`, in turn, into `keep`. Their Formats move to
    /// `keep`'s folder under its names, except one of a type `keep` already
    /// has, which goes to the Trash with the rest of the Book's folder.
    /// `keep`'s Metadata and Cover stay as they are, and its Date Added
    /// becomes the earliest. Stops at the first failure; Books before it
    /// stay merged.
    pub fn merge_books(&mut self, keep: BookId, others: &[BookId]) -> Result<()> {
        let mut seen = HashSet::from([keep]);
        if others.is_empty() || !others.iter().all(|id| seen.insert(*id)) {
            return Err(Error::Invalid(
                "merging needs two or more different books".into(),
            ));
        }
        let others: Vec<Book> = others
            .iter()
            .map(|&id| self.existing_book(id))
            .collect::<Result<_>>()?;
        for other in &others {
            let kept = self.existing_book(keep)?;
            self.merge_book(&kept, other)?;
        }
        Ok(())
    }

    fn merge_book(&mut self, keep: &Book, other: &Book) -> Result<()> {
        let keep_dir = self.book_dir(keep);
        let other_dir = self.book_dir(other);
        let mut moves: Vec<(FormatId, String)> = Vec::new();
        let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
        let undo = |renames: &[(PathBuf, PathBuf)]| {
            for (from, to) in renames.iter().rev() {
                let _ = fs::rename(to, from);
            }
        };

        // Move files first, then record the move; undo the moves if
        // recording fails, so the database never points at missing files.
        let mut result: Result<()> = fs::create_dir_all(&keep_dir).map_err(Error::from);
        for format in &other.formats {
            if result.is_err() {
                break;
            }
            if keep.format(format.kind).is_some() {
                continue;
            }
            let name = naming::format_file_name(&keep.metadata, format.kind);
            let (from, to) = (self.format_path(other, format), keep_dir.join(&name));
            moves.push((format.id, name));
            if !from.exists() {
                continue;
            }
            if to.exists() {
                result = Err(Error::Invalid(format!("{} is in the way", to.display())));
            } else if let Err(e) = fs::rename(&from, &to) {
                result = Err(e.into());
            } else {
                renames.push((from, to));
            }
        }
        if let Err(e) = result {
            undo(&renames);
            return Err(e);
        }

        // What is left of the other Book, its duplicate Formats and Cover,
        // goes to the Trash just before the database change is committed.
        let trash = &self.trash;
        let db = self.db.as_ref().unwrap();
        let result = db.merge_book(keep.id, other.id, &moves, || {
            if other_dir.exists() {
                trash(&other_dir)?;
            }
            Ok::<(), Error>(())
        });
        if let Err(e) = result {
            undo(&renames);
            return Err(e);
        }
        remove_if_empty(other_dir.parent());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{EpubFixture, PdfFixture, TempDir};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    struct Setup {
        dir: TempDir,
        library: Library,
        trash_dir: PathBuf,
        trashed: Arc<Mutex<Vec<PathBuf>>>,
    }

    fn setup() -> Setup {
        let dir = TempDir::new();
        let mut library = Library::open(dir.path().join("Books")).unwrap();
        let trash_dir = dir.path().join("Trash");
        fs::create_dir_all(&trash_dir).unwrap();
        let trashed = Arc::new(Mutex::new(Vec::new()));
        let (log, to) = (trashed.clone(), trash_dir.clone());
        library.trash = Box::new(move |path| {
            let mut log = log.lock().unwrap();
            log.push(path.to_owned());
            // Numbered, as a reused Book id can repeat a folder name.
            let slot = to.join(log.len().to_string());
            fs::create_dir(&slot)?;
            fs::rename(path, slot.join(path.file_name().unwrap()))
        });
        Setup {
            dir,
            library,
            trash_dir,
            trashed,
        }
    }

    impl Setup {
        /// Imports a file with the given name and contents as a new Book.
        fn book(&mut self, name: &str, bytes: &[u8]) -> Book {
            let path = self.dir.path().join(name);
            fs::write(&path, bytes).unwrap();
            let report = self.library.import(&[path], |_| {}).unwrap();
            let crate::import::Outcome::Added(id) = report.items[0].1 else {
                panic!("{report:?}");
            };
            self.library.book(id).unwrap().unwrap()
        }

        fn dune_epub(&mut self) -> Book {
            let epub = EpubFixture::new()
                .metadata("<dc:title>Dune</dc:title><dc:creator>Frank Herbert</dc:creator>")
                .build();
            self.book("dune.epub", &epub)
        }

        /// A PDF with no Metadata, named as Calibre names files, so Import
        /// doesn't match it with the EPUB. `n` tells several apart.
        fn dune_pdf(&mut self, n: u32) -> Book {
            let pdf = PdfFixture::new().info("Subject", &n.to_string()).build();
            self.book(&format!("Dune_ Fr {n} - Frank Herbert.pdf"), &pdf)
        }

        fn kinds(&self, id: BookId) -> Vec<FormatKind> {
            let book = self.library.book(id).unwrap().unwrap();
            book.formats.iter().map(|f| f.kind).collect()
        }
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn merging_moves_formats_to_the_kept_book_under_its_names() {
        let mut s = setup();
        let epub = s.dune_epub();
        let pdf = s.dune_pdf(1);
        assert_eq!(pdf.metadata.authors[0].name, "Unknown");
        let pdf_dir = s.library.book_dir(&pdf);

        s.library.merge_books(epub.id, &[pdf.id]).unwrap();

        let kept = s.library.book(epub.id).unwrap().unwrap();
        assert_eq!(kept.metadata, epub.metadata);
        assert_eq!(kept.cover, epub.cover);
        assert_eq!(s.kinds(epub.id), [FormatKind::Epub, FormatKind::Pdf]);
        assert_eq!(s.library.book(pdf.id).unwrap(), None);
        assert_eq!(s.library.books().unwrap().len(), 1);
        assert_eq!(
            file_names(&s.library.book_dir(&kept)),
            ["Dune - Frank Herbert.epub", "Dune - Frank Herbert.pdf"]
        );
        assert_eq!(*s.trashed.lock().unwrap(), std::slice::from_ref(&pdf_dir));
        assert!(
            !pdf_dir.parent().unwrap().exists(),
            "empty author folder is removed"
        );
    }

    #[test]
    fn a_duplicate_format_goes_to_the_trash_and_the_kept_one_stays() {
        let mut s = setup();
        let epub = s.dune_epub();
        let first = s.dune_pdf(1);
        s.library.merge_books(epub.id, &[first.id]).unwrap();
        let kept = s.library.book(epub.id).unwrap().unwrap();
        let kept_pdf = s
            .library
            .format_path(&kept, kept.format(FormatKind::Pdf).unwrap());
        let before = fs::read(&kept_pdf).unwrap();

        let second = s.dune_pdf(2);
        let second_dir = s.library.book_dir(&second);
        let second_pdf = second.formats[0].file_name.clone();
        s.library.merge_books(epub.id, &[second.id]).unwrap();

        assert_eq!(s.kinds(epub.id), [FormatKind::Epub, FormatKind::Pdf]);
        assert_eq!(fs::read(&kept_pdf).unwrap(), before);
        assert_eq!(s.library.book(second.id).unwrap(), None);
        let trashed = s.trash_dir.join("2").join(second_dir.file_name().unwrap());
        assert!(file_names(&trashed).contains(&second_pdf));
    }

    #[test]
    fn merging_several_keeps_the_earliest_date_added() {
        let mut s = setup();
        let pdf = s.dune_pdf(1);
        let epub = s.dune_epub();
        let other_pdf = s.dune_pdf(2);

        s.library
            .merge_books(epub.id, &[pdf.id, other_pdf.id])
            .unwrap();

        let books = s.library.books().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].id, epub.id);
        assert_eq!(books[0].date_added, pdf.date_added);
        assert_eq!(s.kinds(epub.id), [FormatKind::Epub, FormatKind::Pdf]);
        assert_eq!(s.trashed.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_failed_trash_leaves_both_books_as_they_were() {
        let mut s = setup();
        let epub = s.dune_epub();
        let pdf = s.dune_pdf(1);
        s.library.trash = Box::new(|_| Err(std::io::Error::other("no trash here")));

        assert!(s.library.merge_books(epub.id, &[pdf.id]).is_err());

        assert_eq!(s.library.book(epub.id).unwrap().unwrap(), epub);
        assert_eq!(s.library.book(pdf.id).unwrap().unwrap(), pdf);
        for book in [&epub, &pdf] {
            for f in &book.formats {
                assert!(s.library.format_path(book, f).is_file());
            }
        }
    }

    #[test]
    fn a_file_in_the_way_stops_the_merge() {
        let mut s = setup();
        let epub = s.dune_epub();
        let pdf = s.dune_pdf(1);
        let in_the_way = s.library.book_dir(&epub).join("Dune - Frank Herbert.pdf");
        fs::write(&in_the_way, "not mine").unwrap();

        assert!(matches!(
            s.library.merge_books(epub.id, &[pdf.id]),
            Err(Error::Invalid(_))
        ));
        assert_eq!(fs::read(&in_the_way).unwrap(), b"not mine");
        assert_eq!(s.library.book(pdf.id).unwrap().unwrap(), pdf);
        assert!(s.trashed.lock().unwrap().is_empty());
    }

    #[test]
    fn merging_needs_other_distinct_books() {
        let mut s = setup();
        let epub = s.dune_epub();
        let pdf = s.dune_pdf(1);
        for others in [&[][..], &[epub.id], &[pdf.id, pdf.id], &[BookId(999)]] {
            assert!(
                matches!(
                    s.library.merge_books(epub.id, others),
                    Err(Error::Invalid(_))
                ),
                "{others:?}"
            );
        }
        assert_eq!(s.library.books().unwrap().len(), 2);
    }

    fn book(id: i64, kinds: &[FormatKind]) -> Book {
        Book {
            id: BookId(id),
            metadata: Default::default(),
            date_added: 0,
            path: String::new(),
            cover: None,
            formats: kinds
                .iter()
                .enumerate()
                .map(|(i, &kind)| Format {
                    id: FormatId(id * 10 + i as i64),
                    kind,
                    file_name: String::new(),
                    size: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn plan_moves_new_types_and_trashes_duplicates() {
        use FormatKind::{Epub, Pdf};
        let keep = book(1, &[Pdf]);
        let others = [book(2, &[Epub, Pdf]), book(3, &[Epub])];

        let plan = plan(&keep, &others);

        let ids = |list: &[(&Book, &Format)]| -> Vec<(i64, FormatKind)> {
            list.iter().map(|(b, f)| (b.id.0, f.kind)).collect()
        };
        assert_eq!(ids(&plan.moves), [(2, Epub)]);
        assert_eq!(ids(&plan.duplicates), [(2, Pdf), (3, Epub)]);
    }

    #[test]
    fn keeps_the_first_book_with_an_epub_by_default() {
        use FormatKind::{Epub, Pdf};
        assert_eq!(default_keep(&[book(1, &[Pdf]), book(2, &[Pdf, Epub])]), 1);
        assert_eq!(default_keep(&[book(1, &[Pdf]), book(2, &[Pdf])]), 0);
    }
}
