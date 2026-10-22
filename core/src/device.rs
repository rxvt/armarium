//! A Kindle mounted over USB, which Books can be Sent to and Removed from.
//! Books on it are found by the tag in their file names ([ADR 0005]).
//!
//! [ADR 0005]: ../../docs/adr/0005-device-books-matched-by-filename-tag.md

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::library::Library;
use crate::model::{Book, BookId, FormatKind};
use crate::{Error, Result, azw3, naming};

const DOCUMENTS: &str = "documents";
const SYSTEM: &str = "system";
const THUMBNAILS: &str = "system/thumbnails";

#[derive(Debug, Clone, PartialEq)]
pub enum SendOutcome {
    Sent,
    Failed(String),
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SendReport {
    /// One entry per Book, in the order they were given.
    pub items: Vec<(BookId, SendOutcome)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Converting,
    Copying,
}

#[derive(Debug, Clone, Copy)]
pub struct Progress<'a> {
    pub stage: Stage,
    /// Books finished in this stage so far.
    pub done: usize,
    pub total: usize,
    /// The Book about to be converted or copied.
    pub current: &'a Book,
}

#[derive(Debug, Clone)]
pub struct Device {
    root: PathBuf,
    /// Stands in for the filesystem's free space in tests.
    #[cfg(test)]
    free_space: Option<u64>,
}

impl Device {
    /// The Kindle mounted at `mount_root`, recognised by its `documents/`
    /// and `system/` folders, or `None` for any other drive.
    pub fn at(mount_root: &Path) -> Option<Device> {
        (mount_root.join(DOCUMENTS).is_dir() && mount_root.join(SYSTEM).is_dir()).then(|| Device {
            root: mount_root.to_owned(),
            #[cfg(test)]
            free_space: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The Books on the Device and their files: every tagged file directly
    /// in `documents/`. Anything untagged is not Armarium's and is ignored.
    pub fn books(&self) -> Result<HashMap<BookId, PathBuf>> {
        let mut books = HashMap::new();
        for entry in fs::read_dir(self.documents())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            if let Some(id) = entry.file_name().to_str().and_then(naming::book_id_in) {
                books.insert(id, entry.path());
            }
        }
        Ok(books)
    }

    #[cfg(test)]
    fn with_free_space(self, bytes: u64) -> Device {
        Device {
            free_space: Some(bytes),
            ..self
        }
    }

    /// Sends `books`, converting EPUBs to AZW3 with each Book's current
    /// Metadata and Cover. Everything is converted first, and the whole batch
    /// is refused if it won't fit; otherwise each Book succeeds or fails on
    /// its own.
    pub fn send(
        &self,
        library: &Library,
        books: &[Book],
        mut progress: impl FnMut(Progress),
    ) -> Result<SendReport> {
        let on_device = self.books()?;
        let total = books.len();
        let mut report_progress = |stage, done, current| {
            progress(Progress {
                stage,
                done,
                total,
                current,
            })
        };
        let prepared: Vec<_> = books
            .iter()
            .enumerate()
            .map(|(done, book)| {
                report_progress(Stage::Converting, done, book);
                prepare(library, book)
            })
            .collect();
        let needed: u64 = prepared.iter().flatten().map(Prepared::size).sum();
        let free = self.free_space()?;
        if needed > free {
            return Err(Error::Invalid(format!(
                "not enough space on the Kindle: needs {}, {} free",
                megabytes(needed),
                megabytes(free)
            )));
        }
        let mut report = SendReport::default();
        for (done, (book, copy)) in books.iter().zip(prepared).enumerate() {
            report_progress(Stage::Copying, done, book);
            let outcome =
                match copy.and_then(|copy| self.write(book, &copy, on_device.get(&book.id))) {
                    Ok(()) => SendOutcome::Sent,
                    Err(_) if Device::at(&self.root).is_none() => break,
                    Err(e) => SendOutcome::Failed(e.to_string()),
                };
            report.items.push((book.id, outcome));
        }
        for book in &books[report.items.len()..] {
            let outcome = SendOutcome::Failed("Kindle disconnected".into());
            report.items.push((book.id, outcome));
        }
        Ok(report)
    }

    /// Writes `copy` under its tagged name, replacing `old`, the Book's
    /// current copy if any, and moving `old`'s reading data (its `.sdr`
    /// folder) along if the name has changed.
    fn write(&self, book: &Book, copy: &Prepared, old: Option<&PathBuf>) -> Result<()> {
        let documents = self.documents();
        let part = documents.join(naming::device_part_file_name(book.id));
        let path = documents.join(naming::device_file_name(
            &book.metadata,
            book.id,
            copy.extension,
        ));
        let written = File::create(&part)
            .and_then(|mut file| {
                file.write_all(&copy.bytes)?;
                file.sync_all()
            })
            .and_then(|()| fs::rename(&part, &path));
        if written.is_err() {
            let _ = fs::remove_file(&part);
        }
        written?;
        if let Some(old) = old.filter(|&old| *old != path) {
            fs::remove_file(old)?;
            let (old_sdr, sdr) = (old.with_extension("sdr"), path.with_extension("sdr"));
            if old_sdr.is_dir() && !sdr.exists() {
                fs::rename(old_sdr, sdr)?;
            }
        }

        if let Some(cover) = &copy.thumbnail {
            let thumbnails = self.root.join(THUMBNAILS);
            fs::create_dir_all(&thumbnails)?;
            fs::write(
                thumbnails.join(format!("thumbnail_{}_EBOK_portrait.jpg", asin(book.id))),
                cover,
            )?;
        }
        Ok(())
    }

    /// Bytes free on the Device's filesystem.
    pub fn free_space(&self) -> Result<u64> {
        #[cfg(test)]
        if let Some(bytes) = self.free_space {
            return Ok(bytes);
        }
        use gio::prelude::FileExt;
        let info = gio::File::for_path(&self.root)
            .query_filesystem_info(gio::FILE_ATTRIBUTE_FILESYSTEM_FREE, gio::Cancellable::NONE)
            .map_err(|e| Error::Io(io::Error::other(e.message().to_owned())))?;
        Ok(info.attribute_uint64(gio::FILE_ATTRIBUTE_FILESYSTEM_FREE))
    }

    /// Deletes the partial copies an interrupted Send left in `documents/`.
    pub fn clean_up(&self) -> Result<()> {
        for entry in fs::read_dir(self.documents())? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(naming::is_device_part_file)
            {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    fn documents(&self) -> PathBuf {
        self.root.join(DOCUMENTS)
    }
}

/// A Book's copy for the Device, ready to write.
struct Prepared {
    bytes: Vec<u8>,
    extension: &'static str,
    /// The Cover for the Kindle's library, for an AZW3.
    thumbnail: Option<Vec<u8>>,
}

impl Prepared {
    fn size(&self) -> u64 {
        (self.bytes.len() + self.thumbnail.as_ref().map_or(0, Vec::len)) as u64
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

fn prepare(library: &Library, book: &Book) -> Result<Prepared> {
    let Some(epub) = book.format(FormatKind::Epub) else {
        let pdf = book
            .format(FormatKind::Pdf)
            .ok_or_else(|| Error::Invalid("the book has no EPUB or PDF".into()))?;
        return Ok(Prepared {
            bytes: fs::read(library.format_path(book, pdf))?,
            extension: FormatKind::Pdf.extension(),
            thumbnail: None,
        });
    };
    let cover = library.cover_path(book).map(fs::read).transpose()?;
    let asin = asin(book.id);
    let bytes = azw3::convert(
        &library.format_path(book, epub),
        &book.metadata,
        cover.as_deref(),
        Some(&asin),
    )?;
    Ok(Prepared {
        bytes,
        extension: "azw3",
        thumbnail: cover,
    })
}

/// The ASIN a Book's AZW3 is given: UUID-shaped, as the Kindle expects, and
/// the same on every Send so its thumbnail is replaced rather than piling up.
/// The first 16 digits spell "armarium" in ASCII; the rest are the Book id.
fn asin(id: BookId) -> String {
    format!("61726d61-7269-756d-8000-{:012x}", id.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{EpubFixture, PdfFixture, TempDir};
    use crate::library::Library;
    use crate::model::Book;
    use std::fs;

    fn kindle() -> TempDir {
        let dir = TempDir::new();
        fs::create_dir(dir.path().join("documents")).unwrap();
        fs::create_dir(dir.path().join("system")).unwrap();
        dir
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(20, 30))
            .write_to(&mut io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn epub(title: &str, chapter: &str) -> Vec<u8> {
        EpubFixture::new()
            .metadata(&format!(
                "<dc:title>{title}</dc:title><dc:creator>Frank Herbert</dc:creator>"
            ))
            .manifest(
                r#"<item id="c" href="c.png" media-type="image/png" properties="cover-image"/>
                <item id="t" href="t.xhtml" media-type="application/xhtml+xml"/>"#,
            )
            .spine(r#"<itemref idref="t"/>"#)
            .file("c.png", &png())
            .file("t.xhtml", chapter.as_bytes())
            .build()
    }

    const CHAPTER: &str = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Fear is the mind-killer.</p></body></html>"#;

    /// A Library and a Kindle, both in temporary folders.
    struct Setup {
        _dir: TempDir,
        kindle: TempDir,
        library: Library,
    }

    impl Setup {
        fn new() -> Setup {
            let dir = TempDir::new();
            let library = Library::open(dir.path().join("Library")).unwrap();
            Setup {
                _dir: dir,
                kindle: kindle(),
                library,
            }
        }

        /// Imports a file with the given name and contents as a new Book.
        fn book(&mut self, name: &str, bytes: &[u8]) -> Book {
            let path = self.library.root().parent().unwrap().join(name);
            fs::write(&path, bytes).unwrap();
            let report = self.library.import(&[path], |_| {}).unwrap();
            let crate::import::Outcome::Added(id) = report.items[0].1 else {
                panic!("{report:?}");
            };
            self.library.book(id).unwrap().unwrap()
        }

        fn device(&self) -> Device {
            Device::at(self.kindle.path()).unwrap()
        }

        fn documents(&self) -> PathBuf {
            self.kindle.path().join("documents")
        }

        fn send(&self, books: &[Book]) -> SendReport {
            self.device().send(&self.library, books, |_| {}).unwrap()
        }
    }

    #[test]
    fn sends_an_epub_as_an_azw3_with_its_cover_thumbnail() {
        let mut s = Setup::new();
        let dune = s.book("dune.epub", &epub("Dune", CHAPTER));

        let report = s.send(std::slice::from_ref(&dune));

        assert_eq!(report.items, vec![(dune.id, SendOutcome::Sent)]);
        let file = s.documents().join(format!(
            "Dune - Frank Herbert [armarium-{}].azw3",
            dune.id.0
        ));
        assert_eq!(&fs::read(&file).unwrap()[60..68], b"BOOKMOBI");
        assert_eq!(
            s.device().books().unwrap(),
            HashMap::from([(dune.id, file)])
        );
        let thumbnail = s.kindle.path().join(format!(
            "system/thumbnails/thumbnail_61726d61-7269-756d-8000-{:012x}_EBOK_portrait.jpg",
            dune.id.0
        ));
        assert_eq!(
            fs::read(thumbnail).unwrap(),
            fs::read(s.library.cover_path(&dune).unwrap()).unwrap()
        );
    }

    #[test]
    fn recognises_a_kindle_by_its_folders() {
        let dir = kindle();
        let device = Device::at(dir.path()).unwrap();
        assert_eq!(device.root(), dir.path());
    }

    #[test]
    fn sending_again_replaces_the_copy_and_keeps_its_reading_data() {
        let mut s = Setup::new();
        let mut dune = s.book("dune.epub", &epub("Dune", CHAPTER));
        s.send(std::slice::from_ref(&dune));
        let old = format!("Dune - Frank Herbert [armarium-{}]", dune.id.0);
        fs::create_dir(s.documents().join(format!("{old}.sdr"))).unwrap();
        fs::write(s.documents().join(format!("{old}.sdr/position")), "p. 42").unwrap();

        // Unchanged: the copy is replaced in place.
        s.send(std::slice::from_ref(&dune));
        assert!(s.documents().join(format!("{old}.azw3")).is_file());
        assert!(s.documents().join(format!("{old}.sdr/position")).is_file());

        // Retitled: the copy and its reading data move to the new name.
        dune.metadata.title = "Dune Messiah".into();
        s.send(std::slice::from_ref(&dune));
        let new = format!("Dune Messiah - Frank Herbert [armarium-{}]", dune.id.0);
        let mut names: Vec<_> = fs::read_dir(s.documents())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [format!("{new}.azw3"), format!("{new}.sdr")]);
        assert_eq!(
            fs::read_to_string(s.documents().join(format!("{new}.sdr/position"))).unwrap(),
            "p. 42"
        );
    }

    #[test]
    fn an_epub_that_wont_convert_fails_on_its_own() {
        let mut s = Setup::new();
        let broken = s.book("broken.epub", &epub("Broken", "<html><body><p>unclosed"));
        let pdf = PdfFixture::new().info("Title", "Emma").build();
        let emma = s.book("emma.pdf", &pdf);

        let report = s.send(&[broken.clone(), emma.clone()]);

        let [(id, SendOutcome::Failed(reason)), (_, SendOutcome::Sent)] = &report.items[..] else {
            panic!("{report:?}");
        };
        assert_eq!(*id, broken.id);
        assert!(!reason.is_empty());
        assert_eq!(
            s.device().books().unwrap().into_keys().collect::<Vec<_>>(),
            [emma.id]
        );
    }

    #[test]
    fn a_failed_write_leaves_no_partial_copy() {
        let mut s = Setup::new();
        let dune = s.book("dune.epub", &epub("Dune", CHAPTER));
        let blocker = s.documents().join(format!(
            "Dune - Frank Herbert [armarium-{}].azw3",
            dune.id.0
        ));
        fs::create_dir_all(blocker.join("in the way")).unwrap();

        let report = s.send(std::slice::from_ref(&dune));

        assert!(matches!(report.items[..], [(_, SendOutcome::Failed(_))]));
        let names: Vec<_> = fs::read_dir(s.documents())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [blocker.file_name().unwrap().to_str().unwrap()]);
    }

    #[test]
    fn a_batch_that_wont_fit_is_refused() {
        let mut s = Setup::new();
        let dune = s.book("dune.epub", &epub("Dune", CHAPTER));
        let emma = s.book("emma.pdf", &PdfFixture::new().info("Title", "Emma").build());
        let device = s.device().with_free_space(1000);

        let result = device.send(&s.library, &[dune, emma], |_| {});

        assert!(matches!(result, Err(Error::Invalid(_))), "{result:?}");
        assert_eq!(fs::read_dir(s.documents()).unwrap().count(), 0);
    }

    #[test]
    fn a_kindle_unplugged_part_way_stops_the_batch() {
        let mut s = Setup::new();
        let books: Vec<Book> = ["A", "B", "C"]
            .iter()
            .map(|title| {
                let pdf = PdfFixture::new().info("Title", title).build();
                s.book(&format!("{title}.pdf"), &pdf)
            })
            .collect();
        let root = s.kindle.path().to_owned();
        let mut seen = Vec::new();

        let report = s
            .device()
            .send(&s.library, &books, |p| {
                seen.push((p.stage, p.done, p.total, p.current.id));
                if p.stage == Stage::Copying && p.done == 1 {
                    fs::remove_dir_all(&root).unwrap();
                }
            })
            .unwrap();

        let ids: Vec<_> = books.iter().map(|b| b.id).collect();
        assert_eq!(
            seen,
            [
                (Stage::Converting, 0, 3, ids[0]),
                (Stage::Converting, 1, 3, ids[1]),
                (Stage::Converting, 2, 3, ids[2]),
                (Stage::Copying, 0, 3, ids[0]),
                (Stage::Copying, 1, 3, ids[1]),
            ]
        );
        let disconnected = || SendOutcome::Failed("Kindle disconnected".into());
        assert_eq!(
            report.items,
            [
                (ids[0], SendOutcome::Sent),
                (ids[1], disconnected()),
                (ids[2], disconnected()),
            ]
        );
    }

    #[test]
    fn sends_a_pdf_as_it_is() {
        let mut s = Setup::new();
        let pdf = PdfFixture::new()
            .info("Title", "Emma")
            .info("Author", "Jane Austen")
            .build();
        let emma = s.book("emma.pdf", &pdf);

        let report = s.send(std::slice::from_ref(&emma));

        assert_eq!(report.items, vec![(emma.id, SendOutcome::Sent)]);
        let file = s
            .documents()
            .join(format!("Emma - Jane Austen [armarium-{}].pdf", emma.id.0));
        assert_eq!(fs::read(file).unwrap(), pdf);
        assert!(!s.kindle.path().join("system/thumbnails").exists());
    }

    #[test]
    fn books_are_the_tagged_files_in_documents() {
        let dir = kindle();
        let docs = dir.path().join("documents");
        for file in [
            "Dune - Frank Herbert [armarium-42].azw3",
            "Emma - Jane Austen [armarium-7].pdf",
            "Bought From Amazon.azw3",
            ".armarium-8.part",
        ] {
            fs::write(docs.join(file), "").unwrap();
        }
        fs::create_dir(docs.join("Dune - Frank Herbert [armarium-42].sdr")).unwrap();
        fs::create_dir(docs.join("Old [armarium-3].sdr")).unwrap();
        fs::create_dir(docs.join("Folder")).unwrap();
        fs::write(docs.join("Folder/Nested [armarium-9].azw3"), "").unwrap();

        let books = Device::at(dir.path()).unwrap().books().unwrap();
        assert_eq!(
            books,
            HashMap::from([
                (
                    BookId(42),
                    docs.join("Dune - Frank Herbert [armarium-42].azw3")
                ),
                (BookId(7), docs.join("Emma - Jane Austen [armarium-7].pdf")),
            ])
        );
    }

    #[test]
    fn clean_up_deletes_only_leftover_partial_files() {
        let dir = kindle();
        let docs = dir.path().join("documents");
        let leftovers = [".armarium-8.part", ".armarium-123.part"];
        let keepers = [
            "Dune - Frank Herbert [armarium-42].azw3",
            "Bought From Amazon.azw3",
            "download.part",
            ".armarium-notes",
            ".armarium-.part",
            ".armarium-8.part.azw3",
        ];
        for file in leftovers.iter().chain(&keepers) {
            fs::write(docs.join(file), "").unwrap();
        }
        fs::create_dir(docs.join(".armarium-9.part")).unwrap();

        Device::at(dir.path()).unwrap().clean_up().unwrap();

        for file in leftovers {
            assert!(!docs.join(file).exists(), "{file}");
        }
        for file in keepers {
            assert!(docs.join(file).is_file(), "{file}");
        }
        assert!(docs.join(".armarium-9.part").is_dir());
    }

    #[test]
    fn reports_free_space() {
        let dir = kindle();
        assert!(Device::at(dir.path()).unwrap().free_space().unwrap() > 0);
    }

    #[test]
    fn other_drives_are_not_kindles() {
        let plain = TempDir::new();
        assert!(Device::at(plain.path()).is_none());

        fs::create_dir(plain.path().join("documents")).unwrap();
        assert!(Device::at(plain.path()).is_none());

        fs::write(plain.path().join("system"), "").unwrap();
        assert!(Device::at(plain.path()).is_none());

        assert!(Device::at(&plain.path().join("missing")).is_none());
    }
}
