//! A GObject wrapping a core [`Book`], so Books can live in a `gio::ListStore`.

use std::cell::{Ref, RefCell};
use std::cmp::Ordering;

use armarium_core::model::{self, Book, SortKey, SortKeys};
use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BookObject {
        pub book: RefCell<Option<Book>>,
        pub search_text: RefCell<String>,
        pub sort_keys: RefCell<SortKeys>,
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
        obj.imp().sort_keys.replace(book.sort_keys());
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

    pub fn compare(&self, other: &BookObject, key: SortKey) -> Ordering {
        self.imp()
            .sort_keys
            .borrow()
            .compare(&other.imp().sort_keys.borrow(), key)
    }
}
