//! The Cover picker: a grid of Cover Candidates for a Book, an image file the
//! user uploads first and covers found in Apple Books after it. Choosing one
//! hands it back to the Metadata dialog; nothing is saved here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use armarium_core::cover_search::{self, CoverCandidate};
use armarium_core::model::Metadata;
use gtk::{gdk, gio, glib};

use crate::cover_search as online;
use crate::thumbnails;

const TILE_WIDTH: i32 = 120;

/// Receives the chosen image, still encoded, and a texture of it to preview.
pub type ChooseFn = Box<dyn Fn(Vec<u8>, gdk::Texture)>;

struct Tile {
    candidate: CoverCandidate,
    spinner: adw::Spinner,
}

struct Picker {
    dialog: adw::Dialog,
    toasts: adw::ToastOverlay,
    status: gtk::Label,
    grid: gtk::FlowBox,
    /// The Book's title, to put covers with the same title first.
    title: String,
    tiles: RefCell<Vec<Tile>>,
    /// Bumped by each search, so a slow earlier one doesn't show its results.
    search_id: Cell<u32>,
    downloading: Cell<bool>,
    /// Set once an image is handed back, so a download still finishing while
    /// the dialog closes can't replace it.
    chosen: Cell<bool>,
    on_choose: ChooseFn,
}

/// Shows the picker for a Book with `metadata` (as currently edited) and
/// searches Apple Books for it straight away.
pub fn present(parent: &impl IsA<gtk::Widget>, metadata: &Metadata, on_choose: ChooseFn) {
    let entry = gtk::SearchEntry::builder()
        .text(cover_search::search_term(metadata))
        .placeholder_text("Search Apple Books")
        .hexpand(true)
        .build();
    let header = adw::HeaderBar::builder()
        .title_widget(
            &adw::Clamp::builder()
                .maximum_size(360)
                .child(&entry)
                .build(),
        )
        .build();

    let status = gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .margin_top(6)
        .margin_start(18)
        .margin_end(18)
        .css_classes(["dim-label"])
        .build();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(8)
        .row_spacing(12)
        .column_spacing(12)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .valign(gtk::Align::Start)
        .build();
    grid.append(&upload_tile());
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&status);
    content.append(&grid);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scrolled));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    let dialog = adw::Dialog::builder()
        .title("Choose Cover")
        .content_width(640)
        .content_height(640)
        .child(&toasts)
        .build();

    let picker = Rc::new(Picker {
        dialog: dialog.clone(),
        toasts,
        status,
        grid: grid.clone(),
        title: metadata.title.clone(),
        tiles: RefCell::default(),
        search_id: Cell::new(0),
        downloading: Cell::new(false),
        chosen: Cell::new(false),
        on_choose,
    });
    // The widgets' handlers hold the picker weakly; this is the strong
    // reference, dropped once the dialog closes.
    let holder = RefCell::new(Some(picker.clone()));
    dialog.connect_closed(move |_| drop(holder.take()));

    let weak = Rc::downgrade(&picker);
    entry.connect_activate(glib::clone!(
        #[strong]
        weak,
        move |entry| {
            if let Some(picker) = weak.upgrade() {
                picker.search(entry.text().as_str());
            }
        }
    ));
    grid.connect_child_activated(move |_, child| {
        if let Some(picker) = weak.upgrade() {
            match child.index() {
                0 => picker.upload(),
                i => picker.download(i as usize - 1),
            }
        }
    });

    picker.search(entry.text().as_str());
    dialog.present(Some(parent));
}

impl Picker {
    fn search(self: &Rc<Self>, term: &str) {
        let id = self.search_id.get().wrapping_add(1);
        self.search_id.set(id);
        for _ in self.tiles.take() {
            if let Some(child) = self.grid.child_at_index(1) {
                self.grid.remove(&child);
            }
        }
        if term.trim().is_empty() {
            self.set_status(Some("Search for the book’s title and author"));
            return;
        }
        self.set_status(Some("Searching Apple Books…"));

        let (weak, term) = (Rc::downgrade(self), term.to_owned());
        glib::spawn_future_local(async move {
            let Some(title) = weak.upgrade().map(|p| p.title.clone()) else {
                return;
            };
            let result = online::search(&term, &title).await;
            let Some(picker) = weak.upgrade().filter(|p| p.search_id.get() == id) else {
                return;
            };
            match result {
                Err(e) => picker.set_status(Some(&e)),
                Ok(found) if found.is_empty() => picker.set_status(Some("No covers found")),
                Ok(found) => {
                    picker.set_status(None);
                    for candidate in found.iter() {
                        picker.add_tile(candidate.clone());
                    }
                }
            }
        });
    }

    fn set_status(&self, text: Option<&str>) {
        self.status.set_visible(text.is_some());
        self.status.set_label(text.unwrap_or_default());
    }

    fn add_tile(&self, candidate: CoverCandidate) {
        let picture = cover_picture();
        let spinner = adw::Spinner::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .width_request(32)
            .height_request(32)
            .visible(false)
            .build();
        let overlay = gtk::Overlay::builder()
            .child(&thumbnails::sized_cover(&picture, TILE_WIDTH))
            .tooltip_text(format!("{} — {}", candidate.title, candidate.author))
            .build();
        overlay.add_overlay(&spinner);
        self.grid.append(&overlay);

        let (weak, url) = (picture.downgrade(), candidate.thumbnail_url.clone());
        glib::spawn_future_local(async move {
            let texture = online::image(&url)
                .await
                .ok()
                .and_then(|bytes| gdk::Texture::from_bytes(&bytes).ok());
            if let (Some(texture), Some(picture)) = (texture, weak.upgrade()) {
                picture.remove_css_class("cover-placeholder");
                thumbnails::show_cover(&picture, Some(&texture));
            }
        });
        self.tiles.borrow_mut().push(Tile { candidate, spinner });
    }

    /// Downloads the full-size image of tile `i` and chooses it.
    fn download(self: &Rc<Self>, i: usize) {
        let Some((url, spinner)) = self
            .tiles
            .borrow()
            .get(i)
            .map(|t| (t.candidate.full_url.clone(), t.spinner.clone()))
        else {
            return;
        };
        if self.downloading.replace(true) {
            return;
        }
        // A new search drops this tile, and the download with it.
        let search_id = self.search_id.get();
        spinner.set_visible(true);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = online::image(&url).await;
            spinner.set_visible(false);
            let Some(picker) = weak.upgrade() else {
                return;
            };
            picker.downloading.set(false);
            if picker.search_id.get() != search_id {
                return;
            }
            match result {
                Ok(bytes) => picker.choose(bytes.to_vec()),
                Err(e) => picker.toast(&e),
            }
        });
    }

    fn upload(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images"));
        for ext in ["jpg", "jpeg", "png", "gif", "webp"] {
            filter.add_suffix(ext);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let file_dialog = gtk::FileDialog::builder()
            .title("Upload Cover")
            .accept_label("_Choose")
            .filters(&filters)
            .build();
        let parent = self.dialog.root().and_downcast::<gtk::Window>();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(file) = file_dialog.open_future(parent.as_ref()).await else {
                return;
            };
            let contents = file.load_contents_future().await;
            let Some(picker) = weak.upgrade() else {
                return;
            };
            match contents {
                Ok((bytes, _etag)) => picker.choose(bytes.to_vec()),
                Err(e) => picker.toast(&format!("Could not read the image: {}", e.message())),
            }
        });
    }

    /// Hands `bytes` back if they are an image GTK can show, and closes. Only
    /// the first image chosen counts.
    fn choose(&self, bytes: Vec<u8>) {
        if self.chosen.get() {
            return;
        }
        match gdk::Texture::from_bytes(&glib::Bytes::from(&bytes)) {
            Ok(texture) => {
                self.chosen.set(true);
                (self.on_choose)(bytes, texture);
                self.dialog.close();
            }
            Err(e) => self.toast(&format!("Could not read the image: {}", e.message())),
        }
    }

    fn toast(&self, title: &str) {
        let toast = adw::Toast::builder().title(title).use_markup(false).build();
        self.toasts.add_toast(toast);
    }
}

fn cover_picture() -> gtk::Picture {
    gtk::Picture::builder()
        .can_shrink(true)
        .width_request(TILE_WIDTH)
        .height_request(TILE_WIDTH * 3 / 2)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["cover", "cover-placeholder"])
        .build()
}

/// The first tile, which opens a file chooser.
fn upload_tile() -> gtk::Widget {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    content.append(
        &gtk::Image::builder()
            .icon_name("document-open-symbolic")
            .pixel_size(32)
            .build(),
    );
    content.append(&gtk::Label::new(Some("Upload Image")));
    let tile = gtk::Box::builder()
        .width_request(TILE_WIDTH)
        .height_request(TILE_WIDTH * 3 / 2)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .tooltip_text("Choose an image file")
        .css_classes(["card", "cover"])
        .build();
    content.set_hexpand(true);
    tile.append(&content);
    tile.upcast()
}
