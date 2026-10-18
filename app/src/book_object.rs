//! A GObject wrapping a core [`Book`], so Books can live in a `gio::ListStore`.

use std::cell::{Ref, RefCell};

use armarium_core::model::{self, Book};
use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BookObject {
        pub book: RefCell<Option<Book>>,
        pub search_text: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookObject {
        const NAME: &'static str = "ArmariumBookObject";
        type Type = super::BookObject;
    }

    impl ObjectImpl for BookObject {}
}

glib::wrapper! {
    pub struct BookObject(ObjectSubclass<imp::BookObject>);
}

impl BookObject {
    pub fn new(book: Book) -> BookObject {
        let obj: BookObject = glib::Object::new();
        obj.imp().search_text.replace(book.search_text());
        obj.imp().book.replace(Some(book));
        obj
    }

    pub fn book(&self) -> Ref<'_, Book> {
        Ref::map(self.imp().book.borrow(), |b| {
            b.as_ref().expect("BookObject has a Book")
        })
    }

    pub fn matches(&self, query: &str) -> bool {
        model::search_matches(&self.imp().search_text.borrow(), query)
    }
}
