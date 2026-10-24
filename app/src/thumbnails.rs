//! Cover thumbnails for the grid: scaled copies of each `cover.jpg` cached
//! under `~/.cache/armarium/thumbnails` (`armarium-devel` in debug builds) (rebuildable), loaded off the main
//! thread and kept in a small in-memory cache.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use armarium_core::cover;
use gtk::prelude::*;
use gtk::{gdk, gio};

use crate::profile;

/// Longest side of a cached thumbnail; enough for the largest tile at 2× scale.
const THUMB_MAX_SIDE: u32 = 720;
/// Textures kept in memory; tiles scrolled far away reload from disk.
const MEMORY_ENTRIES: usize = 300;
/// How far a Cover's aspect ratio may stray from 2:3 and still fill its
/// picture; cropping that little is invisible, and it avoids slivers of
/// backdrop from rounding.
const FILL_TOLERANCE: f64 = 0.03;

pub struct Thumbnails {
    dir: PathBuf,
    memory: RefCell<(HashMap<PathBuf, gdk::Texture>, VecDeque<PathBuf>)>,
}

impl Thumbnails {
    pub fn new() -> Thumbnails {
        Thumbnails {
            dir: profile::cache_dir().join("thumbnails"),
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

/// Shows `texture` in a 2:3 Cover `picture`, or clears it. A Cover close to
/// 2:3 fills the picture; any other is shown whole on a neutral backdrop
/// rather than cropped.
pub fn show_cover(picture: &gtk::Picture, texture: Option<&gdk::Texture>) {
    let letterbox = texture.is_some_and(|t| !fills_2_3(t.width(), t.height()));
    if letterbox {
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.add_css_class("cover-backdrop");
    } else {
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.remove_css_class("cover-backdrop");
    }
    picture.set_paintable(texture);
}

/// `picture` held to a 2:3 Cover `width` pixels wide. A Picture asks for its
/// image's full size; the clamps hold it to the Cover's.
pub fn sized_cover(picture: &gtk::Picture, width: i32) -> gtk::Widget {
    let clamp = |child: &gtk::Widget, orientation, size| {
        adw::Clamp::builder()
            .orientation(orientation)
            .maximum_size(size)
            .tightening_threshold(size)
            .child(child)
            .build()
            .upcast::<gtk::Widget>()
    };
    clamp(
        &clamp(picture.upcast_ref(), gtk::Orientation::Horizontal, width),
        gtk::Orientation::Vertical,
        width * 3 / 2,
    )
}

fn fills_2_3(width: i32, height: i32) -> bool {
    if width <= 0 || height <= 0 {
        return true;
    }
    let ratio = (f64::from(width) / f64::from(height)) / (2.0 / 3.0);
    (ratio - 1.0).abs() <= FILL_TOLERANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(width: i32, height: i32) -> gdk::Texture {
        let bytes = gtk::glib::Bytes::from_owned(vec![0u8; (width * height * 4) as usize]);
        gdk::MemoryTexture::new(
            width,
            height,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            width as usize * 4,
        )
        .upcast()
    }

    /// One test, since GTK widgets belong to the thread that initialised
    /// GTK. Skipped without a display.
    #[test]
    fn show_cover_letterboxes_only_other_shapes() {
        if gtk::init().is_err() {
            return;
        }
        let picture = gtk::Picture::new();
        let shown = |picture: &gtk::Picture| {
            (
                picture.content_fit(),
                picture.has_css_class("cover-backdrop"),
                picture.paintable().is_some(),
            )
        };

        show_cover(&picture, Some(&texture(974, 1200)));
        assert_eq!(shown(&picture), (gtk::ContentFit::Contain, true, true));

        show_cover(&picture, Some(&texture(600, 900)));
        assert_eq!(shown(&picture), (gtk::ContentFit::Cover, false, true));

        show_cover(&picture, Some(&texture(900, 900)));
        show_cover(&picture, None);
        assert_eq!(shown(&picture), (gtk::ContentFit::Cover, false, false));
    }

    #[test]
    fn covers_near_2_3_fill() {
        assert!(fills_2_3(600, 900));
        assert!(fills_2_3(1000, 1530));
        assert!(fills_2_3(1000, 1480));
    }

    #[test]
    fn other_shapes_are_letterboxed() {
        assert!(!fills_2_3(974, 1200));
        assert!(!fills_2_3(900, 900));
        assert!(!fills_2_3(500, 900));
    }
}
