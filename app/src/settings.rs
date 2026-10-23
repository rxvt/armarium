//! Preferences remembered across sessions, in a GLib key file at
//! `~/.config/armarium/settings.ini` (`armarium-devel` in debug builds) (or `$ARMARIUM_SETTINGS`, for trying the
//! app without touching it). A missing or unreadable file means defaults; the
//! app never fails to start over it.

use std::path::PathBuf;

use gtk::glib;

use crate::profile;

const GROUP: &str = "armarium";

pub struct Settings {
    path: PathBuf,
    file: glib::KeyFile,
}

impl Settings {
    pub fn load() -> Settings {
        let path = std::env::var_os("ARMARIUM_SETTINGS")
            .map(PathBuf::from)
            .unwrap_or_else(|| profile::config_dir().join("settings.ini"));
        let file = glib::KeyFile::new();
        if path.exists()
            && let Err(e) = file.load_from_file(&path, glib::KeyFileFlags::KEEP_COMMENTS)
        {
            eprintln!("ignoring {}: {e}", path.display());
        }
        Settings { path, file }
    }

    /// The Library folder chosen in Preferences, if any.
    pub fn library(&self) -> Option<PathBuf> {
        self.file
            .string(GROUP, "library")
            .ok()
            .map(|s| PathBuf::from(s.as_str()))
    }

    pub fn set_library(&self, path: &std::path::Path) {
        self.file
            .set_string(GROUP, "library", &path.to_string_lossy());
        self.save();
    }

    pub fn cover_width(&self) -> Option<i32> {
        self.file.integer(GROUP, "cover-width").ok()
    }

    pub fn set_cover_width(&self, width: i32) {
        self.file.set_integer(GROUP, "cover-width", width);
        self.save();
    }

    /// "grid" or "list".
    pub fn view(&self) -> Option<String> {
        self.file.string(GROUP, "view").ok().map(String::from)
    }

    pub fn set_view(&self, view: &str) {
        self.file.set_string(GROUP, "view", view);
        self.save();
    }

    /// The sort key's name ([`SortKey::as_str`]) and whether it is descending.
    ///
    /// [`SortKey::as_str`]: armarium_core::model::SortKey::as_str
    pub fn sort(&self) -> Option<(String, bool)> {
        let key = self.file.string(GROUP, "sort").ok()?;
        let descending = self.file.boolean(GROUP, "sort-descending").ok()?;
        Some((key.into(), descending))
    }

    pub fn set_sort(&self, key: &str, descending: bool) {
        self.file.set_string(GROUP, "sort", key);
        self.file.set_boolean(GROUP, "sort-descending", descending);
        self.save();
    }

    fn save(&self) {
        let result = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                self.file
                    .save_to_file(&self.path)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            eprintln!("could not save {}: {e}", self.path.display());
        }
    }
}
