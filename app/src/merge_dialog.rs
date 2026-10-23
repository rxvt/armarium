//! The Merge dialog: the Books being Merged, each with its Cover, authors and
//! Formats, to pick the one to keep. It says which Formats will go to the
//! Trash before the user confirms, as a Merge cannot be undone.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use armarium_core::merge;
use armarium_core::model::{Book, BookId};
use gtk::glib;

use crate::book_tile::authors_line;
use crate::thumbnails::Thumbnails;

const COVER_WIDTH: i32 = 32;

/// Merges the other Books into the kept one.
pub type MergeFn = Box<dyn Fn(BookId, Vec<BookId>)>;

/// Shows the dialog for `books`, two or more, each with its `cover.jpg` if
/// it has one. `on_merge` runs once the user confirms.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    books: Vec<(Book, Option<PathBuf>)>,
    thumbnails: &Rc<Thumbnails>,
    on_merge: MergeFn,
) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let mut checks: Vec<gtk::CheckButton> = Vec::new();
    for (book, cover) in &books {
        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Center)
            .build();
        if let Some(first) = checks.first() {
            check.set_group(Some(first));
        }
        let formats: Vec<&str> = book.formats.iter().map(|f| f.kind.label()).collect();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&book.metadata.title))
            .subtitle(glib::markup_escape_text(&format!(
                "{} · {}",
                authors_line(book),
                formats.join(", ")
            )))
            .activatable_widget(&check)
            .build();
        // Prefixes stack from the start, so the button ends up first.
        row.add_prefix(&cover_picture(cover.clone(), thumbnails));
        row.add_prefix(&check);
        list.append(&row);
        checks.push(check);
    }
    let books: Rc<Vec<Book>> = Rc::new(books.into_iter().map(|(book, _)| book).collect());
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(420)
        .build();

    let dialog = adw::AlertDialog::builder()
        .heading(format!("Merge {} Books?", books.len()))
        .extra_child(&scrolled)
        .build();
    dialog.add_responses(&[("cancel", "_Cancel"), ("merge", "_Merge")]);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let kept = {
        let checks = checks.clone();
        move || checks.iter().position(|c| c.is_active()).unwrap_or(0)
    };
    let update = {
        let (dialog, books, kept) = (dialog.clone(), books.clone(), kept.clone());
        move || {
            let (body, destructive) = describe(&books, kept());
            dialog.set_body(&body);
            dialog.set_response_appearance(
                "merge",
                if destructive {
                    adw::ResponseAppearance::Destructive
                } else {
                    adw::ResponseAppearance::Suggested
                },
            );
        }
    };
    checks[merge::default_keep(&books)].set_active(true);
    update();
    for check in &checks {
        let update = update.clone();
        check.connect_toggled(move |check| {
            if check.is_active() {
                update();
            }
        });
    }

    dialog.connect_response(Some("merge"), move |_, _| {
        let keep = kept();
        let others = books
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != keep)
            .map(|(_, book)| book.id)
            .collect();
        on_merge(books[keep].id, others);
    });
    dialog.present(Some(parent));
}

/// The dialog's text when keeping `books[keep]`, and whether Merging moves
/// anything to the Trash.
fn describe(books: &[Book], keep: usize) -> (String, bool) {
    let others: Vec<Book> = books
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != keep)
        .map(|(_, book)| book.clone())
        .collect();
    let plan = merge::plan(&books[keep], &others);
    let mut body = "The book you choose keeps its metadata and cover and gains the others’ \
                    formats. The others are removed from the library. This can’t be undone."
        .to_owned();
    if !plan.duplicates.is_empty() {
        body.push_str("\n\nThe kept book already has these formats, so they go to the Trash:");
        for (book, format) in &plan.duplicates {
            body.push_str(&format!(
                "\n• {} of “{}”",
                format.kind.label(),
                book.metadata.title
            ));
        }
    }
    (body, !plan.duplicates.is_empty())
}

/// The Book's Cover at 2:3, `COVER_WIDTH` pixels wide, or a placeholder.
fn cover_picture(cover: Option<PathBuf>, thumbnails: &Rc<Thumbnails>) -> gtk::Widget {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(COVER_WIDTH)
        .height_request(COVER_WIDTH * 3 / 2)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["cover", "list-cover", "cover-placeholder"])
        .build();
    // A Picture asks for its image's full size; the clamps hold it to the
    // Cover's.
    let clamp = |child: &gtk::Widget, orientation, size| {
        adw::Clamp::builder()
            .orientation(orientation)
            .maximum_size(size)
            .tightening_threshold(size)
            .child(child)
            .build()
            .upcast::<gtk::Widget>()
    };
    let sized = clamp(
        &clamp(
            picture.upcast_ref(),
            gtk::Orientation::Horizontal,
            COVER_WIDTH,
        ),
        gtk::Orientation::Vertical,
        COVER_WIDTH * 3 / 2,
    );
    sized.set_valign(gtk::Align::Center);
    if let Some(cover) = cover {
        let thumbnails = thumbnails.clone();
        let weak = picture.downgrade();
        glib::spawn_future_local(async move {
            if let Some(texture) = thumbnails.load(cover).await
                && let Some(picture) = weak.upgrade()
            {
                picture.remove_css_class("cover-placeholder");
                picture.set_paintable(Some(&texture));
            }
        });
    }
    sized
}
