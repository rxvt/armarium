//! Keeps development builds apart from the installed app. Debug builds use
//! their own application id (so both can run at once) and their own settings,
//! thumbnail cache and default Library, so work in progress never touches the
//! real Library. They also skip single-instance behaviour, so builds from
//! different git worktrees can run side by side.

use std::path::PathBuf;

use gtk::{gio, glib};

pub const DEVEL: bool = cfg!(debug_assertions);

pub const APP_ID: &str = if DEVEL {
    "io.github.rxvt.Armarium.Devel"
} else {
    "io.github.rxvt.Armarium"
};

/// Debug builds are not single-instance: otherwise a second launch, say from
/// another worktree, would just raise the window already open.
pub const APP_FLAGS: gio::ApplicationFlags = if DEVEL {
    gio::ApplicationFlags::NON_UNIQUE
} else {
    gio::ApplicationFlags::FLAGS_NONE
};

/// Window title for debug builds. It names the checkout they were built from,
/// so windows from different worktrees can be told apart.
pub fn devel_window_title() -> String {
    format!("Armarium (Development) – {}", checkout_name())
}

/// The folder holding the workspace this build came from: the worktree name,
/// or the main checkout's folder.
fn checkout_name() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|dir| dir.file_name())
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// Folder name under the XDG config, cache and data directories.
const DIR_NAME: &str = if DEVEL { "armarium-devel" } else { "armarium" };

pub fn config_dir() -> PathBuf {
    glib::user_config_dir().join(DIR_NAME)
}

pub fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join(DIR_NAME)
}

/// `~/Books` for the installed app; a Library under the XDG data directory
/// for development builds.
pub fn default_library() -> PathBuf {
    if DEVEL {
        glib::user_data_dir().join(DIR_NAME).join("Books")
    } else {
        glib::home_dir().join("Books")
    }
}
