//! The Library: a folder the app owns, holding `library.db` and one folder
//! per Book (ADR 0002).

use std::path::{Path, PathBuf};

use crate::Result;
use crate::db::Database;
use crate::model::{Book, BookId, Format};
use crate::naming;

pub const DB_FILE_NAME: &str = "library.db";

/// Moves a file or folder to the Trash.
pub type TrashFn = Box<dyn Fn(&Path) -> std::io::Result<()> + Send>;

pub struct Library {
    root: PathBuf,
    /// `None` until the first Import creates the Library.
    pub(crate) db: Option<Database>,
    pub(crate) trash: TrashFn,
}

impl Library {
    /// Opens the Library at `root` if it has a `library.db`; otherwise returns
    /// an empty Library that is created on the first Import.
    pub fn open(root: impl Into<PathBuf>) -> Result<Library> {
        let root = root.into();
        let db_path = root.join(DB_FILE_NAME);
        let db = if db_path.exists() {
            Some(Database::open(&db_path)?)
        } else {
            None
        };
        Ok(Library {
            root,
            db,
            trash: Box::new(trash_with_gio),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every Book, newest first.
    pub fn books(&self) -> Result<Vec<Book>> {
        match &self.db {
            Some(db) => Ok(db.books()?),
            None => Ok(Vec::new()),
        }
    }

    pub fn book(&self, id: BookId) -> Result<Option<Book>> {
        match &self.db {
            Some(db) => Ok(db.book(id)?),
            None => Ok(None),
        }
    }

    pub fn book_dir(&self, book: &Book) -> PathBuf {
        self.root.join(&book.path)
    }

    pub fn format_path(&self, book: &Book, format: &Format) -> PathBuf {
        self.book_dir(book).join(&format.file_name)
    }

    /// The Book's `cover.jpg`, or `None` if it has none and needs a placeholder.
    pub fn cover_path(&self, book: &Book) -> Option<PathBuf> {
        book.cover
            .map(|_| self.book_dir(book).join(naming::COVER_FILE_NAME))
    }

    pub(crate) fn db_or_create(&mut self) -> Result<&Database> {
        if self.db.is_none() {
            std::fs::create_dir_all(&self.root)?;
            self.db = Some(Database::open(&self.root.join(DB_FILE_NAME))?);
        }
        Ok(self.db.as_ref().unwrap())
    }
}

fn trash_with_gio(path: &Path) -> std::io::Result<()> {
    use gio::prelude::FileExt;
    gio::File::for_path(path)
        .trash(gio::Cancellable::NONE)
        .map_err(|e| std::io::Error::other(e.message().to_owned()))
}
