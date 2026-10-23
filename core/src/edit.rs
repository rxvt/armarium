//! Changing Books after Import: Metadata edits (which rename folders and
//! files), Cover replacement, and deleting to the Trash.

use std::fs;
use std::path::Path;

use crate::library::Library;
use crate::model::{Author, Book, BookId, CoverSource, FormatId, Metadata, MetadataEdit};
use crate::{Error, Result, cover, naming};

impl Library {
    /// Replaces a Book's Metadata, renaming its folder and Format files to
    /// match. An empty author list becomes "Unknown"; an empty title is
    /// rejected. Returns the updated Book.
    pub fn update_metadata(&mut self, id: BookId, mut metadata: Metadata) -> Result<Book> {
        metadata.title = metadata.title.trim().to_owned();
        if metadata.title.is_empty() {
            return Err(Error::Invalid("a book needs a title".into()));
        }
        metadata.authors.retain(|a| !a.name.trim().is_empty());
        if metadata.authors.is_empty() {
            metadata.authors.push(Author::new(naming::UNKNOWN_AUTHOR));
        }

        let book = self.existing_book(id)?;
        let old_dir = self.book_dir(&book);
        let new_rel = naming::book_dir(&metadata, id);
        let new_dir = self.root().join(&new_rel);
        let renames: Vec<(FormatId, String, String)> = book
            .formats
            .iter()
            .map(|f| {
                (
                    f.id,
                    f.file_name.clone(),
                    naming::format_file_name(&metadata, f.kind),
                )
            })
            .collect();

        // Move files first, then record the new names; undo the moves if
        // recording fails, so the database never points at missing files.
        let moved_dir = old_dir != new_dir && old_dir.exists();
        if moved_dir {
            if let Some(parent) = new_dir.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&old_dir, &new_dir)?;
        }
        let mut renamed = Vec::new();
        let mut result: Result<()> = Ok(());
        for (_, old, new) in &renames {
            if old != new && new_dir.join(old).exists() {
                if let Err(e) = fs::rename(new_dir.join(old), new_dir.join(new)) {
                    result = Err(e.into());
                    break;
                }
                renamed.push((old, new));
            }
        }
        let db = self.db.as_ref().unwrap();
        let result = result.and_then(|()| {
            db.update_metadata(id, &metadata)?;
            db.set_book_path(id, &new_rel)?;
            for (format, _, new) in &renames {
                db.set_format_file_name(*format, new)?;
            }
            Ok(())
        });

        if let Err(e) = result {
            for (old, new) in renamed {
                let _ = fs::rename(new_dir.join(new), new_dir.join(old));
            }
            if moved_dir {
                let _ = fs::rename(&new_dir, &old_dir);
            }
            // Put back whatever the database had, in case it was half-written.
            let _ = db.update_metadata(id, &book.metadata);
            let _ = db.set_book_path(id, &book.path);
            for f in &book.formats {
                let _ = db.set_format_file_name(f.id, &f.file_name);
            }
            remove_if_empty(new_dir.parent());
            return Err(e);
        }
        if moved_dir {
            remove_if_empty(old_dir.parent());
        }
        self.existing_book(id)
    }

    /// Makes `edit` to each Book's Metadata in turn, renaming as
    /// [`update_metadata`](Library::update_metadata) does. Stops at the first
    /// failure, returning how many Books were updated before it; those stay
    /// updated.
    pub fn edit_metadata(
        &mut self,
        ids: &[BookId],
        edit: &MetadataEdit,
    ) -> std::result::Result<(), (usize, Error)> {
        for (updated, &id) in ids.iter().enumerate() {
            let result = self.existing_book(id).and_then(|book| {
                let mut metadata = book.metadata;
                edit.apply(&mut metadata);
                self.update_metadata(id, metadata)
            });
            if let Err(e) = result {
                return Err((updated, e));
            }
        }
        Ok(())
    }

    /// Replaces a Book's Cover with the image at `image`. The new Cover is
    /// never overridden by a later Import.
    pub fn set_cover(&mut self, id: BookId, image: &Path) -> Result<()> {
        let book = self.existing_book(id)?;
        let dir = self.book_dir(&book);
        fs::create_dir_all(&dir)?;
        cover::save(&fs::read(image)?, &dir.join(naming::COVER_FILE_NAME))?;
        self.db
            .as_ref()
            .unwrap()
            .set_cover_source(id, Some(CoverSource::User))?;
        Ok(())
    }

    /// Moves each Book's folder to the Trash and forgets the Book. Stops at
    /// the first failure; Books before it stay deleted.
    pub fn delete_books(&mut self, ids: &[BookId]) -> Result<()> {
        for &id in ids {
            let book = self.existing_book(id)?;
            let dir = self.book_dir(&book);
            if dir.exists() {
                (self.trash)(&dir)?;
            }
            self.db.as_ref().unwrap().remove_book(id)?;
            remove_if_empty(dir.parent());
        }
        Ok(())
    }

    /// Moves one Format's file to the Trash. Deleting a Book's last Format
    /// deletes the Book.
    pub fn delete_format(&mut self, id: BookId, format: FormatId) -> Result<()> {
        let book = self.existing_book(id)?;
        let Some(f) = book.formats.iter().find(|f| f.id == format) else {
            return Err(Error::Invalid(format!(
                "book {} has no format {}",
                id.0, format.0
            )));
        };
        if book.formats.len() == 1 {
            return self.delete_books(&[id]);
        }
        let path = self.format_path(&book, f);
        if path.exists() {
            (self.trash)(&path)?;
        }
        self.db.as_ref().unwrap().remove_format(format)?;
        Ok(())
    }

    pub(crate) fn existing_book(&self, id: BookId) -> Result<Book> {
        self.book(id)?
            .ok_or_else(|| Error::Invalid(format!("no book with id {}", id.0)))
    }
}

/// Removes an author folder once its last Book has gone.
pub(crate) fn remove_if_empty(dir: Option<&Path>) {
    if let Some(dir) = dir {
        let _ = fs::remove_dir(dir); // fails, harmlessly, unless empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{EpubFixture, PdfFixture, TempDir};
    use crate::model::{FieldEdit, FormatKind};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    struct Setup {
        dir: TempDir,
        library: Library,
        trashed: Arc<Mutex<Vec<PathBuf>>>,
    }

    /// A Library holding one Book "Dune" by Frank Herbert with an EPUB and a PDF.
    fn setup() -> (Setup, Book) {
        let dir = TempDir::new();
        let mut library = Library::open(dir.path().join("Books")).unwrap();
        let trash_dir = dir.path().join("Trash");
        fs::create_dir_all(&trash_dir).unwrap();
        let trashed = Arc::new(Mutex::new(Vec::new()));
        let log = trashed.clone();
        library.trash = Box::new(move |path| {
            log.lock().unwrap().push(path.to_owned());
            fs::rename(path, trash_dir.join(path.file_name().unwrap()))
        });

        let epub = dir.path().join("dune.epub");
        let pdf = dir.path().join("dune.pdf");
        fs::write(
            &epub,
            EpubFixture::new()
                .metadata("<dc:title>Dune</dc:title><dc:creator>Frank Herbert</dc:creator>")
                .build(),
        )
        .unwrap();
        fs::write(
            &pdf,
            PdfFixture::new()
                .info("Title", "Dune")
                .info("Author", "Frank Herbert")
                .build(),
        )
        .unwrap();
        library.import(&[epub, pdf], |_| {}).unwrap();
        let book = library.books().unwrap().pop().unwrap();
        assert_eq!(book.formats.len(), 2);
        (
            Setup {
                dir,
                library,
                trashed,
            },
            book,
        )
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
    fn editing_title_and_author_renames_folder_and_files() {
        let (mut s, book) = setup();
        let old_dir = s.library.book_dir(&book);
        let mut metadata = book.metadata.clone();
        metadata.title = "Dune Messiah".into();
        metadata.authors = vec![Author::new("F. Herbert")];
        metadata.series = Some("Dune".into());

        let updated = s
            .library
            .update_metadata(book.id, metadata.clone())
            .unwrap();
        assert_eq!(updated.metadata, metadata);
        assert_eq!(
            updated.path,
            format!("F. Herbert/Dune Messiah ({})", book.id.0)
        );
        let dir = s.library.book_dir(&updated);
        assert_eq!(
            file_names(&dir),
            [
                "Dune Messiah - F. Herbert.epub",
                "Dune Messiah - F. Herbert.pdf",
                "cover.jpg"
            ]
        );
        for f in &updated.formats {
            assert!(s.library.format_path(&updated, f).exists());
        }
        assert!(!old_dir.exists());
        assert!(
            !old_dir.parent().unwrap().exists(),
            "empty author folder is removed"
        );
    }

    #[test]
    fn editing_other_fields_leaves_files_alone() {
        let (mut s, book) = setup();
        let mut metadata = book.metadata.clone();
        metadata.publisher = Some("Chilton".into());
        let updated = s.library.update_metadata(book.id, metadata).unwrap();
        assert_eq!(updated.path, book.path);
        assert_eq!(updated.formats, book.formats);
    }

    #[test]
    fn rejects_empty_title_and_defaults_empty_authors() {
        let (mut s, book) = setup();
        let mut metadata = book.metadata.clone();
        metadata.title = "  ".into();
        assert!(matches!(
            s.library.update_metadata(book.id, metadata),
            Err(Error::Invalid(_))
        ));
        assert_eq!(s.library.book(book.id).unwrap().unwrap(), book);

        let mut metadata = book.metadata.clone();
        metadata.authors = vec![Author::new(" ")];
        let updated = s.library.update_metadata(book.id, metadata).unwrap();
        assert_eq!(updated.metadata.authors, vec![Author::new("Unknown")]);
    }

    /// Imports a second Book, "Emma" by Jane Austen, as an EPUB.
    fn add_emma(s: &mut Setup) -> Book {
        let epub = s.dir.path().join("emma.epub");
        fs::write(
            &epub,
            EpubFixture::new()
                .metadata("<dc:title>Emma</dc:title><dc:creator>Jane Austen</dc:creator>")
                .build(),
        )
        .unwrap();
        s.library.import(&[epub], |_| {}).unwrap();
        let emma = s.library.books().unwrap().remove(0);
        assert_eq!(emma.metadata.title, "Emma");
        emma
    }

    #[test]
    fn editing_several_books_changes_only_the_edited_fields() {
        let (mut s, dune) = setup();
        let emma = add_emma(&mut s);
        let mut metadata = dune.metadata.clone();
        metadata.series_index = Some(1.0);
        metadata.isbn = Some("9780441013593".into());
        s.library.update_metadata(dune.id, metadata).unwrap();

        let edit = MetadataEdit {
            authors: Some(vec![Author {
                name: "Anon".into(),
                sort_name: "Anonymous".into(),
            }]),
            series: FieldEdit::Set("Classics".into()),
            ..Default::default()
        };
        s.library.edit_metadata(&[dune.id, emma.id], &edit).unwrap();

        let dune = s.library.book(dune.id).unwrap().unwrap();
        let emma = s.library.book(emma.id).unwrap().unwrap();
        for (book, title) in [(&dune, "Dune"), (&emma, "Emma")] {
            assert_eq!(book.metadata.title, title);
            assert_eq!(book.metadata.authors[0].sort_name, "Anonymous");
            assert_eq!(book.metadata.series.as_deref(), Some("Classics"));
            assert_eq!(book.path, format!("Anon/{title} ({})", book.id.0));
            assert!(s.library.book_dir(book).exists());
        }
        assert_eq!(dune.metadata.series_index, Some(1.0));
        assert_eq!(dune.metadata.isbn.as_deref(), Some("9780441013593"));
        assert_eq!(emma.metadata.series_index, None);
    }

    #[test]
    fn editing_several_books_stops_at_the_first_failure() {
        let (mut s, dune) = setup();
        let emma = add_emma(&mut s);
        let edit = MetadataEdit {
            publisher: FieldEdit::Set("Penguin".into()),
            ..Default::default()
        };
        let Err((updated, e)) = s
            .library
            .edit_metadata(&[dune.id, BookId(999), emma.id], &edit)
        else {
            panic!("an unknown Book should fail");
        };
        assert_eq!(updated, 1);
        assert!(matches!(e, Error::Invalid(_)));
        let publisher = |id| s.library.book(id).unwrap().unwrap().metadata.publisher;
        assert_eq!(publisher(dune.id).as_deref(), Some("Penguin"));
        assert_eq!(publisher(emma.id), None);
    }

    #[test]
    fn user_cover_replaces_current_one() {
        let (mut s, book) = setup();
        let image = s.dir.path().join("new.png");
        image::RgbImage::from_pixel(10, 10, image::Rgb([0, 0, 255]))
            .save(&image)
            .unwrap();
        s.library.set_cover(book.id, &image).unwrap();

        let book = s.library.book(book.id).unwrap().unwrap();
        assert_eq!(book.cover, Some(CoverSource::User));
        let saved = image::open(s.library.cover_path(&book).unwrap())
            .unwrap()
            .to_rgb8();
        assert!(saved.get_pixel(5, 5).0[2] > 200);

        let garbage = s.dir.path().join("garbage.png");
        fs::write(&garbage, b"nope").unwrap();
        assert!(s.library.set_cover(book.id, &garbage).is_err());
    }

    #[test]
    fn deleting_a_format_trashes_its_file() {
        let (mut s, book) = setup();
        let pdf = book.format(FormatKind::Pdf).unwrap();
        let path = s.library.format_path(&book, pdf);
        s.library.delete_format(book.id, pdf.id).unwrap();

        assert_eq!(*s.trashed.lock().unwrap(), std::slice::from_ref(&path));
        assert!(!path.exists());
        let book = s.library.book(book.id).unwrap().unwrap();
        assert_eq!(
            book.formats.iter().map(|f| f.kind).collect::<Vec<_>>(),
            [FormatKind::Epub]
        );
    }

    #[test]
    fn deleting_the_last_format_deletes_the_book() {
        let (mut s, book) = setup();
        let [a, b] = [&book.formats[0], &book.formats[1]];
        s.library.delete_format(book.id, a.id).unwrap();
        s.library.delete_format(book.id, b.id).unwrap();
        assert_eq!(s.library.book(book.id).unwrap(), None);
    }

    #[test]
    fn deleting_books_trashes_folders() {
        let (mut s, book) = setup();
        let dir = s.library.book_dir(&book);
        s.library.delete_books(&[book.id]).unwrap();

        assert_eq!(*s.trashed.lock().unwrap(), std::slice::from_ref(&dir));
        assert!(!dir.exists());
        assert!(s.library.books().unwrap().is_empty());
        assert!(matches!(
            s.library.delete_books(&[book.id]),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn failed_trash_keeps_the_book() {
        let (mut s, book) = setup();
        s.library.trash = Box::new(|_| Err(std::io::Error::other("no trash here")));
        assert!(s.library.delete_books(&[book.id]).is_err());
        assert!(s.library.book(book.id).unwrap().is_some());
        assert!(s.library.book_dir(&book).exists());
    }
}
