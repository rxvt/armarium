//! A Kindle mounted over USB, which Books can be Sent to and Removed from.
//! Books on it are found by the tag in their file names ([ADR 0005]).
//!
//! [ADR 0005]: ../../docs/adr/0005-device-books-matched-by-filename-tag.md

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::BookId;
use crate::naming;
use crate::{Error, Result};

const DOCUMENTS: &str = "documents";
const SYSTEM: &str = "system";

#[derive(Debug, Clone)]
pub struct Device {
    root: PathBuf,
}

impl Device {
    /// The Kindle mounted at `mount_root`, recognised by its `documents/`
    /// and `system/` folders, or `None` for any other drive.
    pub fn at(mount_root: &Path) -> Option<Device> {
        (mount_root.join(DOCUMENTS).is_dir() && mount_root.join(SYSTEM).is_dir()).then(|| Device {
            root: mount_root.to_owned(),
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

    /// Bytes free on the Device's filesystem.
    pub fn free_space(&self) -> Result<u64> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::TempDir;
    use std::fs;

    fn kindle() -> TempDir {
        let dir = TempDir::new();
        fs::create_dir(dir.path().join("documents")).unwrap();
        fs::create_dir(dir.path().join("system")).unwrap();
        dir
    }

    #[test]
    fn recognises_a_kindle_by_its_folders() {
        let dir = kindle();
        let device = Device::at(dir.path()).unwrap();
        assert_eq!(device.root(), dir.path());
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
