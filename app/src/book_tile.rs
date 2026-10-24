//! One Book in the cover grid: Cover (or a title/author placeholder), title
//! and authors, and a tick while it is on the connected Device.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use armarium_core::model::{Book, BookId};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::book_object;
use crate::thumbnails::{self, Thumbnails};

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rxvt/Armarium/ui/book-tile.ui")]
    pub struct BookTile {
        #[template_child]
        pub cover: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub device_tick: TemplateChild<gtk::Image>,
        #[template_child]
        pub placeholder: TemplateChild<gtk::Box>,
        #[template_child]
        pub placeholder_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub placeholder_author: TemplateChild<gtk::Label>,
        #[template_child]
        pub title_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub author_label: TemplateChild<gtk::Label>,
        /// The Book currently shown, so a slow thumbnail load for a Book
        /// that has since scrolled away is dropped.
        pub book: Cell<Option<BookId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookTile {
        const NAME: &'static str = "ArmariumBookTile";
        type Type = super::BookTile;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookTile {}
    impl WidgetImpl for BookTile {}
    impl BoxImpl for BookTile {}
}

glib::wrapper! {
    pub struct BookTile(ObjectSubclass<imp::BookTile>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl BookTile {
    pub fn new() -> BookTile {
        glib::Object::new()
    }

    /// Covers are drawn at 2:3, `width` pixels wide.
    pub fn set_cover_width(&self, width: i32) {
        let imp = self.imp();
        imp.cover.set_size_request(width, width * 3 / 2);
        self.set_size_request(width, -1);
    }

    /// Hides the title and authors under the Cover, for the details sidebar.
    pub fn set_caption_visible(&self, visible: bool) {
        self.imp().title_label.set_visible(visible);
        self.imp().author_label.set_visible(visible);
    }

    /// Ticks the Cover while the Book at `item` is on the connected Device.
    pub fn bind_on_device(&self, item: &gtk::ListItem) {
        book_object::bind_on_device(item, &*self.imp().device_tick);
    }

    pub fn bind(&self, book: &Book, cover: Option<PathBuf>, thumbnails: &Rc<Thumbnails>) {
        let imp = self.imp();
        let authors = authors_line(book);
        imp.book.set(Some(book.id));
        imp.title_label.set_label(&book.metadata.title);
        imp.title_label.set_tooltip_text(Some(&book.metadata.title));
        imp.author_label.set_label(&authors);
        imp.placeholder_title.set_label(&book.metadata.title);
        imp.placeholder_author.set_label(&authors);
        thumbnails::show_cover(&imp.picture, None);
        imp.placeholder.set_visible(true);

        let Some(cover) = cover else { return };
        let id = book.id;
        let tile = self.downgrade();
        let thumbnails = thumbnails.clone();
        glib::spawn_future_local(async move {
            let texture = thumbnails.load(cover).await;
            if let Some(tile) = tile.upgrade()
                && tile.imp().book.get() == Some(id)
                && let Some(texture) = texture
            {
                thumbnails::show_cover(&tile.imp().picture, Some(&texture));
                tile.imp().placeholder.set_visible(false);
            }
        });
    }

    pub fn unbind(&self) {
        self.imp().book.set(None);
        thumbnails::show_cover(&self.imp().picture, None);
    }
}

pub fn authors_line(book: &Book) -> String {
    book.metadata
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
