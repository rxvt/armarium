//! The details sidebar for the selected Book: Cover, title and authors,
//! Formats with Open and Delete buttons, and the rest of its Metadata. With
//! no single Book selected it says so instead.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use armarium_core::model::Book;
use gtk::glib;

use crate::book_tile::{BookTile, authors_line};
use crate::thumbnails::Thumbnails;

const COVER_WIDTH: i32 = 200;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rxvt/Armarium/ui/book-details.ui")]
    pub struct BookDetails {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub none_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub cover: TemplateChild<BookTile>,
        #[template_child]
        pub title_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub authors_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub formats_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub details_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub description_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub description_label: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookDetails {
        const NAME: &'static str = "ArmariumBookDetails";
        type Type = super::BookDetails;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            BookTile::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookDetails {
        fn constructed(&self) {
            self.parent_constructed();
            self.cover.set_cover_width(COVER_WIDTH);
            self.cover.set_caption_visible(false);
        }
    }

    impl WidgetImpl for BookDetails {}
    impl BinImpl for BookDetails {}
}

glib::wrapper! {
    pub struct BookDetails(ObjectSubclass<imp::BookDetails>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BookDetails {
    pub fn new() -> BookDetails {
        glib::Object::new()
    }

    /// Shows `book`. The Format buttons fire `win.open-format` and
    /// `win.delete-format` with the Format's extension, acting on the
    /// selected Book.
    pub fn show(&self, book: &Book, cover: Option<PathBuf>, thumbnails: &Rc<Thumbnails>) {
        let imp = self.imp();
        imp.stack.set_visible_child_name("book");
        imp.cover.bind(book, cover, thumbnails);
        imp.title_label.set_label(&book.metadata.title);
        imp.authors_label.set_label(&authors_line(book));

        imp.formats_list.remove_all();
        for format in &book.formats {
            let ext = format.kind.extension();
            let open = gtk::Button::builder()
                .label("Open")
                .valign(gtk::Align::Center)
                .action_name("win.open-format")
                .action_target(&ext.to_variant())
                .build();
            let delete = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(format!("Move {} to Trash…", format.kind.label()))
                .valign(gtk::Align::Center)
                .action_name("win.delete-format")
                .action_target(&ext.to_variant())
                .css_classes(["flat"])
                .build();
            let row = adw::ActionRow::builder()
                .title(format.kind.label())
                .subtitle(glib::format_size(format.size).as_str())
                .tooltip_text(&format.file_name)
                .build();
            row.add_suffix(&open);
            row.add_suffix(&delete);
            imp.formats_list.append(&row);
        }

        let m = &book.metadata;
        let series = m.series.as_ref().map(|s| match m.series_index {
            Some(i) => format!("{s}, book {i}"),
            None => s.clone(),
        });
        let added = glib::DateTime::from_unix_local(book.date_added)
            .and_then(|d| d.format("%x"))
            .ok()
            .map(String::from);
        imp.details_list.remove_all();
        for (label, value) in [
            ("Series", series),
            ("Publisher", m.publisher.clone()),
            ("Published", m.published.clone()),
            ("Language", m.language.clone()),
            ("ISBN", m.isbn.clone()),
            ("Date Added", added),
        ] {
            let Some(value) = value.filter(|v| !v.is_empty()) else {
                continue;
            };
            let row = adw::ActionRow::builder()
                .title(label)
                .subtitle(glib::markup_escape_text(&value).as_str())
                .subtitle_selectable(true)
                .css_classes(["property"])
                .build();
            imp.details_list.append(&row);
        }

        let description = m.description.as_deref().unwrap_or_default();
        imp.description_label.set_label(description);
        imp.description_group.set_visible(!description.is_empty());
    }

    /// Shows that `count` Books (none, or several) are selected.
    pub fn show_selection_count(&self, count: u64) {
        let imp = self.imp();
        let title = match count {
            0 => "No Book Selected".to_owned(),
            n => format!("{n} Books Selected"),
        };
        imp.none_page.set_title(&title);
        imp.stack.set_visible_child_name("none");
    }
}
