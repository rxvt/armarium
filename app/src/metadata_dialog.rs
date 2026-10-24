//! The Metadata edit dialog for one Book, which can also replace its Cover.
//! Nothing changes until Save.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use armarium_core::model::{Author, Book, Metadata};
use armarium_core::suggest::Suggestions;
use armarium_core::text;
use gtk::{gio, glib};

use crate::suggestion_popover;
use crate::thumbnails::{self, Thumbnails};

/// Saves the edited Metadata and, if one was chosen, the new Cover image.
/// An `Err` is shown in the dialog, which stays open.
pub type SaveFn = Box<dyn Fn(Metadata, Option<PathBuf>) -> Result<(), String>>;

/// One author's name and sort name rows, with a button to remove them.
pub struct AuthorRows {
    pub name: adw::EntryRow,
    pub sort_name: adw::EntryRow,
    pub remove: gtk::Button,
}

impl AuthorRows {
    /// Rows for `author`. The sort name follows the name as it is typed
    /// until the user edits the sort name itself.
    pub fn new(author: &Author) -> AuthorRows {
        let name = adw::EntryRow::builder()
            .title("Name")
            .text(&author.name)
            .build();
        let sort_name = adw::EntryRow::builder()
            .title("Sort As")
            .text(&author.sort_name)
            .build();
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .tooltip_text("Remove Author")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        name.add_suffix(&remove);

        let customised = Rc::new(Cell::new(
            author.sort_name != text::author_sort_name(&author.name),
        ));
        let deriving = Rc::new(Cell::new(false));
        name.connect_changed(glib::clone!(
            #[weak]
            sort_name,
            #[strong]
            customised,
            #[strong]
            deriving,
            move |name| {
                if !customised.get() {
                    deriving.set(true);
                    sort_name.set_text(&text::author_sort_name(&name.text()));
                    deriving.set(false);
                }
            }
        ));
        sort_name.connect_changed(move |_| {
            if !deriving.get() {
                customised.set(true);
            }
        });
        AuthorRows {
            name,
            sort_name,
            remove,
        }
    }

    /// The author as entered, or `None` while the name is empty.
    pub fn author(&self) -> Option<Author> {
        let name = self.name.text().trim().to_owned();
        if name.is_empty() {
            return None;
        }
        let sort_name = Some(self.sort_name.text().trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| text::author_sort_name(&name));
        Some(Author { name, sort_name })
    }
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rxvt/Armarium/ui/metadata-dialog.ui")]
    pub struct MetadataDialog {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub cover_picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub title_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub authors_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub series_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub series_index_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub publisher_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub published_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub language_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub isbn_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub description_view: TemplateChild<gtk::TextView>,

        pub authors: RefCell<Vec<AuthorRows>>,
        pub new_cover: RefCell<Option<PathBuf>>,
        pub on_save: RefCell<Option<SaveFn>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MetadataDialog {
        const NAME: &'static str = "ArmariumMetadataDialog";
        type Type = super::MetadataDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl MetadataDialog {
        #[template_callback]
        fn on_cancel(&self) {
            self.obj().close();
        }

        #[template_callback]
        fn on_add_author(&self) {
            let row = self.obj().add_author(&Author::new(""));
            row.grab_focus();
        }

        #[template_callback]
        fn on_replace_cover(&self) {
            self.obj().choose_cover();
        }
    }

    impl ObjectImpl for MetadataDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            for row in [&*self.title_row, &*self.series_index_row] {
                row.connect_changed(glib::clone!(
                    #[weak]
                    obj,
                    move |_| obj.validate()
                ));
            }
            self.save_button.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.save()
            ));
        }
    }

    impl WidgetImpl for MetadataDialog {}
    impl AdwDialogImpl for MetadataDialog {}
}

glib::wrapper! {
    pub struct MetadataDialog(ObjectSubclass<imp::MetadataDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl MetadataDialog {
    /// `series` are the Library's Series names, offered as the user types one.
    pub fn new(
        book: &Book,
        series: Suggestions,
        cover: Option<PathBuf>,
        thumbnails: &Rc<Thumbnails>,
        on_save: SaveFn,
    ) -> MetadataDialog {
        let obj: MetadataDialog = glib::Object::new();
        let imp = obj.imp();
        let m = &book.metadata;
        imp.title_row.set_text(&m.title);
        for author in &m.authors {
            obj.add_author(author);
        }
        imp.series_row
            .set_text(m.series.as_deref().unwrap_or_default());
        suggestion_popover::attach(&imp.series_row, series);
        imp.series_index_row
            .set_text(&m.series_index.map(|i| i.to_string()).unwrap_or_default());
        imp.publisher_row
            .set_text(m.publisher.as_deref().unwrap_or_default());
        imp.published_row
            .set_text(m.published.as_deref().unwrap_or_default());
        imp.language_row
            .set_text(m.language.as_deref().unwrap_or_default());
        imp.isbn_row.set_text(m.isbn.as_deref().unwrap_or_default());
        imp.description_view
            .buffer()
            .set_text(m.description.as_deref().unwrap_or_default());
        imp.on_save.replace(Some(on_save));

        if let Some(cover) = cover {
            let thumbnails = thumbnails.clone();
            let picture = imp.cover_picture.downgrade();
            glib::spawn_future_local(async move {
                if let (Some(texture), Some(picture)) =
                    (thumbnails.load(cover).await, picture.upgrade())
                {
                    thumbnails::show_cover(&picture, Some(&texture));
                    picture.remove_css_class("cover-placeholder");
                }
            });
        }
        obj.validate();
        obj
    }

    /// Adds rows for `author`.
    fn add_author(&self, author: &Author) -> adw::EntryRow {
        let rows = AuthorRows::new(author);
        let name = rows.name.clone();
        rows.remove.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            #[weak]
            name,
            move |_| dialog.remove_author(&name)
        ));

        let imp = self.imp();
        imp.authors_list.append(&rows.name);
        imp.authors_list.append(&rows.sort_name);
        imp.authors.borrow_mut().push(rows);
        name
    }

    fn remove_author(&self, name: &adw::EntryRow) {
        let imp = self.imp();
        let mut authors = imp.authors.borrow_mut();
        if let Some(i) = authors.iter().position(|a| &a.name == name) {
            let rows = authors.remove(i);
            imp.authors_list.remove(&rows.name);
            imp.authors_list.remove(&rows.sort_name);
        }
    }

    fn choose_cover(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images"));
        for ext in ["jpg", "jpeg", "png", "gif", "webp"] {
            filter.add_suffix(ext);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Replace Cover")
            .accept_label("_Choose")
            .filters(&filters)
            .build();
        let parent = self.root().and_downcast::<gtk::Window>();
        dialog.open(
            parent.as_ref(),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |result| {
                    let Some(path) = result.ok().and_then(|f| f.path()) else {
                        return;
                    };
                    let imp = dialog.imp();
                    match gtk::gdk::Texture::from_filename(&path) {
                        Ok(texture) => {
                            thumbnails::show_cover(&imp.cover_picture, Some(&texture));
                            imp.cover_picture.remove_css_class("cover-placeholder");
                            imp.new_cover.replace(Some(path));
                        }
                        Err(e) => {
                            dialog.toast(&format!("Could not read the image: {}", e.message()))
                        }
                    }
                }
            ),
        );
    }

    fn series_index(&self) -> Result<Option<f64>, ()> {
        let text = self.imp().series_index_row.text();
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        text.replace(',', ".").parse().map(Some).map_err(|_| ())
    }

    /// Save needs a title and a numeric (or empty) number in series.
    fn validate(&self) {
        let imp = self.imp();
        let index_ok = self.series_index().is_ok();
        if index_ok {
            imp.series_index_row.remove_css_class("error");
        } else {
            imp.series_index_row.add_css_class("error");
        }
        let has_title = !imp.title_row.text().trim().is_empty();
        imp.save_button.set_sensitive(has_title && index_ok);
    }

    fn metadata(&self) -> Metadata {
        let imp = self.imp();
        let text =
            |row: &adw::EntryRow| Some(row.text().trim().to_owned()).filter(|t| !t.is_empty());
        let buffer = imp.description_view.buffer();
        let description = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .trim()
            .to_owned();
        let authors = imp
            .authors
            .borrow()
            .iter()
            .filter_map(AuthorRows::author)
            .collect();
        Metadata {
            title: imp.title_row.text().trim().to_owned(),
            authors,
            series: text(&imp.series_row),
            series_index: self.series_index().ok().flatten(),
            publisher: text(&imp.publisher_row),
            published: text(&imp.published_row),
            language: text(&imp.language_row),
            isbn: text(&imp.isbn_row),
            description: Some(description).filter(|d| !d.is_empty()),
        }
    }

    fn save(&self) {
        let imp = self.imp();
        let metadata = self.metadata();
        let cover = imp.new_cover.borrow().clone();
        let result = match imp.on_save.borrow().as_ref() {
            Some(save) => save(metadata, cover),
            None => Ok(()),
        };
        match result {
            Ok(()) => {
                self.close();
            }
            Err(e) => self.toast(&e),
        }
    }

    fn toast(&self, title: &str) {
        let toast = adw::Toast::builder().title(title).use_markup(false).build();
        self.imp().toast_overlay.add_toast(toast);
    }
}
