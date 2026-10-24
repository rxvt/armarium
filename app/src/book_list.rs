//! The list view's columns: Cover, Title, Authors, Series, Formats and Date
//! added, each sortable.

use std::path::PathBuf;
use std::rc::Rc;

use armarium_core::model::{Book, SortKey};
use gtk::glib;
use gtk::prelude::*;

use crate::book_object::{self, BookObject};
use crate::book_tile::authors_line;
use crate::thumbnails::{self, Thumbnails};

/// Finds a Book's `cover.jpg`, which needs the window's Library.
pub type CoverPath = Rc<dyn Fn(&Book) -> Option<PathBuf>>;

/// Called with each cell's widget as it is created, e.g. to add a context menu.
pub type SetupCell = Rc<dyn Fn(&gtk::Widget, &gtk::ColumnViewCell)>;

const COVER_WIDTH: i32 = 32;

/// Adds the columns to `view` and returns the sortable ones with their keys.
pub fn add_columns(
    view: &gtk::ColumnView,
    thumbnails: Rc<Thumbnails>,
    cover_path: CoverPath,
    setup: SetupCell,
) -> Vec<(SortKey, gtk::ColumnViewColumn)> {
    view.append_column(&cover_column(thumbnails, cover_path, setup.clone()));
    let text_column =
        |title: &str, key, text: fn(&Book) -> String| text_column(title, key, text, setup.clone());
    let title = text_column("Title", SortKey::Title, |b| b.metadata.title.clone());
    title.set_expand(true);
    let authors = text_column("Authors", SortKey::Authors, authors_line);
    authors.set_expand(true);
    let columns = vec![
        (SortKey::Title, title),
        (SortKey::Authors, authors),
        (
            SortKey::Series,
            text_column("Series", SortKey::Series, series_line),
        ),
        (
            SortKey::Formats,
            text_column("Formats", SortKey::Formats, formats_line),
        ),
        (
            SortKey::DateAdded,
            text_column("Date Added", SortKey::DateAdded, date_line),
        ),
    ];
    for (_, column) in &columns {
        view.append_column(column);
    }
    columns
}

fn sorter(key: SortKey) -> gtk::CustomSorter {
    gtk::CustomSorter::new(move |a, b| {
        let (Some(a), Some(b)) = (
            a.downcast_ref::<BookObject>(),
            b.downcast_ref::<BookObject>(),
        ) else {
            return gtk::Ordering::Equal;
        };
        a.compare(b, key).into()
    })
}

fn text_column(
    title: &str,
    key: SortKey,
    text: fn(&Book) -> String,
    setup: SetupCell,
) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, cell| {
        let cell = cell.downcast_ref::<gtk::ColumnViewCell>().unwrap();
        // Fills the cell, so a right-click anywhere in it reaches the label.
        let label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .vexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        setup(label.upcast_ref(), cell);
        cell.set_child(Some(&label));
    });
    factory.connect_bind(move |_, cell| {
        let cell = cell.downcast_ref::<gtk::ColumnViewCell>().unwrap();
        let (Some(label), Some(obj)) = (
            cell.child().and_downcast::<gtk::Label>(),
            cell.item().and_downcast::<BookObject>(),
        ) else {
            return;
        };
        let text = text(&obj.book());
        label.set_tooltip_text(Some(&text));
        label.set_label(&text);
    });
    let column = gtk::ColumnViewColumn::new(Some(title), Some(factory));
    column.set_resizable(true);
    column.set_sorter(Some(&sorter(key)));
    column
}

fn cover_column(
    thumbnails: Rc<Thumbnails>,
    cover_path: CoverPath,
    setup: SetupCell,
) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, cell| {
        let cell = cell.downcast_ref::<gtk::ColumnViewCell>().unwrap();
        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .width_request(COVER_WIDTH)
            .height_request(COVER_WIDTH * 3 / 2)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["cover", "list-cover"])
            .build();
        let tick = gtk::Image::builder()
            .icon_name("object-select-symbolic")
            .tooltip_text("On Kindle")
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .css_classes(["device-tick", "list-cover-tick"])
            .build();
        book_object::bind_on_device(cell.upcast_ref(), &tick);
        let overlay = gtk::Overlay::builder()
            .child(&picture)
            .halign(gtk::Align::Center)
            .build();
        overlay.add_overlay(&tick);
        setup(overlay.upcast_ref(), cell);
        cell.set_child(Some(&overlay));
    });
    factory.connect_bind(move |_, cell| {
        let cell = cell.downcast_ref::<gtk::ColumnViewCell>().unwrap();
        let (Some(picture), Some(obj)) = (
            cover_picture(cell),
            cell.item().and_downcast::<BookObject>(),
        ) else {
            return;
        };
        thumbnails::show_cover(&picture, None);
        picture.add_css_class("cover-placeholder");
        let Some(cover) = cover_path(&obj.book()) else {
            return;
        };
        let cell = cell.downgrade();
        let thumbnails = thumbnails.clone();
        glib::spawn_future_local(async move {
            let texture = thumbnails.load(cover).await;
            // Drop the result if the row has since been reused for another Book.
            if let Some(cell) = cell.upgrade()
                && cell.item().as_ref() == Some(obj.upcast_ref())
                && let Some(texture) = texture
            {
                thumbnails::show_cover(&picture, Some(&texture));
                picture.remove_css_class("cover-placeholder");
            }
        });
    });
    factory.connect_unbind(|_, cell| {
        if let Some(picture) = cover_picture(cell.downcast_ref().unwrap()) {
            thumbnails::show_cover(&picture, None);
        }
    });
    let column = gtk::ColumnViewColumn::new(None, Some(factory));
    column.set_fixed_width(COVER_WIDTH + 24);
    column
}

fn cover_picture(cell: &gtk::ColumnViewCell) -> Option<gtk::Picture> {
    cell.child()
        .and_downcast::<gtk::Overlay>()?
        .child()
        .and_downcast()
}

/// "Dune 2", "Discworld 1.5", or "" outside a series.
fn series_line(book: &Book) -> String {
    let m = &book.metadata;
    match (&m.series, m.series_index) {
        (Some(series), Some(index)) => format!("{series} {index}"),
        (Some(series), None) => series.clone(),
        (None, _) => String::new(),
    }
}

fn formats_line(book: &Book) -> String {
    book.formats
        .iter()
        .map(|f| f.kind.label())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The date in the user's locale format.
fn date_line(book: &Book) -> String {
    glib::DateTime::from_unix_local(book.date_added)
        .and_then(|d| d.format("%x"))
        .map(String::from)
        .unwrap_or_default()
}
