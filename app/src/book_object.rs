//! A GObject wrapping a core [`Book`], so Books can live in a `gio::ListStore`.

use std::cell::{Cell, Ref, RefCell};
use std::cmp::Ordering;

use armarium_core::model::{self, Book, SortKey, SortKeys};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::BookObject)]
    pub struct BookObject {
        pub book: RefCell<Option<Book>>,
        pub search_text: RefCell<String>,
        pub sort_keys: RefCell<SortKeys>,
        /// Whether the connected Device has a copy.
        #[property(get, set)]
        pub on_device: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookObject {
        const NAME: &'static str = "ArmariumBookObject";
        type Type = super::BookObject;
    }

    #[glib::derived_properties]
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

/// Shows `widget` only while the Book at `item` is on the connected Device.
pub fn bind_on_device(item: &gtk::ListItem, widget: &impl IsA<gtk::Widget>) {
    item.property_expression("item")
        .chain_property::<BookObject>("on-device")
        .bind(
            widget.upcast_ref::<gtk::Widget>(),
            "visible",
            gtk::Widget::NONE,
        );
}
