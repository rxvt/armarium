//! Cover thumbnails for the grid: scaled copies of each `cover.jpg` cached
//! under `~/.cache/armarium/thumbnails` (rebuildable), loaded off the main
//! thread and kept in a small in-memory cache.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use armarium_core::cover;
use gtk::{gdk, gio, glib};

/// Longest side of a cached thumbnail; enough for the largest tile at 2× scale.
const THUMB_MAX_SIDE: u32 = 720;
/// Textures kept in memory; tiles scrolled far away reload from disk.
const MEMORY_ENTRIES: usize = 300;

pub struct Thumbnails {
    dir: PathBuf,
    memory: RefCell<(HashMap<PathBuf, gdk::Texture>, VecDeque<PathBuf>)>,
}

impl Thumbnails {
    pub fn new() -> Thumbnails {
        Thumbnails {
            dir: glib::user_cache_dir().join("armarium").join("thumbnails"),
            memory: RefCell::default(),
        }
    }

    /// The thumbnail texture for the Cover at `cover`, creating the cached
    /// file if needed. `None` if the Cover cannot be read.
    pub async fn load(&self, cover: PathBuf) -> Option<gdk::Texture> {
        if let Some(texture) = self.memory.borrow().0.get(&cover) {
            return Some(texture.clone());
        }
        let dir = self.dir.clone();
        let key = cover.clone();
        let texture = gio::spawn_blocking(move || load_blocking(&dir, &cover))
            .await
            .ok()
            .flatten()?;

        let (map, order) = &mut *self.memory.borrow_mut();
        if map.insert(key.clone(), texture.clone()).is_none() {
            order.push_back(key);
            if order.len() > MEMORY_ENTRIES
                && let Some(old) = order.pop_front()
            {
                map.remove(&old);
            }
        }
        Some(texture)
    }

    /// Forgets the in-memory texture for `cover`, e.g. after it was replaced.
    pub fn forget(&self, cover: &Path) {
        let (map, order) = &mut *self.memory.borrow_mut();
        map.remove(cover);
        order.retain(|p| p != cover);
    }
}

fn load_blocking(dir: &Path, cover: &Path) -> Option<gdk::Texture> {
    // Keyed on the Cover's path and modification time, so a replaced Cover
    // gets a fresh thumbnail.
    let modified = std::fs::metadata(cover).ok()?.modified().ok()?;
    let mut hasher = DefaultHasher::new();
    cover.hash(&mut hasher);
    modified
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .hash(&mut hasher);
    let thumb = dir.join(format!("{:016x}.jpg", hasher.finish()));

    if !thumb.exists() {
        cover::thumbnail(cover, &thumb, THUMB_MAX_SIDE).ok()?;
    }
    gdk::Texture::from_filename(&thumb).ok()
}
