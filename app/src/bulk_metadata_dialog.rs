//! The Metadata edit dialog for several Books at once: their Authors, Series,
//! Publisher and Language. A field all the Books share shows its value, one
//! they differ in says "Mixed". Only the fields the user changes are saved.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use armarium_core::model::{Author, Book, FieldEdit, Metadata, MetadataEdit};
use gtk::glib;

use crate::metadata_dialog::AuthorRows;

/// Saves the edit to every Book. An `Err` is shown in the dialog, which
/// stays open.
pub type SaveFn = Box<dyn Fn(MetadataEdit) -> Result<(), String>>;

/// A field's value across the Books being edited.
#[derive(Debug, Clone, PartialEq)]
enum Initial<T> {
    Shared(T),
    Mixed,
}

impl<T: PartialEq> Initial<T> {
    fn of(books: &[Book], field: impl Fn(&Metadata) -> T) -> Initial<T> {
        let mut values = books.iter().map(|b| field(&b.metadata));
        match values.next() {
            Some(first) if values.all(|v| v == first) => Initial::Shared(first),
            _ => Initial::Mixed,
        }
    }
}

/// An optional text field. Typing sets it for every Book; only its clear
/// button clears it, so emptying the text by hand leaves the Books alone.
struct FieldRow {
    row: adw::EntryRow,
    /// Says "Mixed", "Unchanged" or "Cleared" while the text is empty.
    status: gtk::Label,
    clear: gtk::Button,
    initial: Initial<Option<String>>,
    cleared: Cell<bool>,
}

impl FieldRow {
    fn new(row: &adw::EntryRow, initial: Initial<Option<String>>) -> FieldRow {
        if let Initial::Shared(Some(value)) = &initial {
            row.set_text(value);
        }
        let status = gtk::Label::builder().css_classes(["dimmed"]).build();
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text(format!("Clear {}", row.title()))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&status);
        row.add_suffix(&clear);
        FieldRow {
            row: row.clone(),
            status,
            clear,
            initial,
            cleared: Cell::new(false),
        }
    }

    fn edit(&self) -> FieldEdit {
        let text = self.row.text().trim().to_owned();
        if text.is_empty() {
            if self.cleared.get() {
                FieldEdit::Clear
            } else {
                FieldEdit::Keep
            }
        } else if self.initial == Initial::Shared(Some(text.clone())) {
            FieldEdit::Keep
        } else {
            FieldEdit::Set(text)
        }
    }

    /// Shows what Save would do to the field while its text is empty.
    fn update_status(&self) {
        let empty = self.row.text().trim().is_empty();
        let status = match (&self.initial, self.cleared.get()) {
            _ if !empty => None,
            (_, true) => Some("Cleared"),
            (Initial::Mixed, false) => Some("Mixed"),
            (Initial::Shared(Some(_)), false) => Some("Unchanged"),
            (Initial::Shared(None), false) => None,
        };
        self.status.set_label(status.unwrap_or_default());
        self.status.set_visible(status.is_some());
        let nothing_to_clear =
            empty && (self.cleared.get() || self.initial == Initial::Shared(None));
        self.clear.set_sensitive(!nothing_to_clear);
    }
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rxvt/Armarium/ui/bulk-metadata-dialog.ui")]
    pub struct BulkMetadataDialog {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub authors_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub authors_placeholder: TemplateChild<gtk::Label>,
        #[template_child]
        pub series_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub publisher_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub language_row: TemplateChild<adw::EntryRow>,

        /// The authors all the Books share, or `Mixed`.
        pub(super) initial_authors: RefCell<Option<Initial<Vec<Author>>>>,
        pub authors: RefCell<Vec<AuthorRows>>,
        /// Series, Publisher and Language, in that order.
        pub(super) fields: RefCell<Vec<FieldRow>>,
        pub on_save: RefCell<Option<SaveFn>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BulkMetadataDialog {
        const NAME: &'static str = "ArmariumBulkMetadataDialog";
        type Type = super::BulkMetadataDialog;
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
    impl BulkMetadataDialog {
        #[template_callback]
        fn on_cancel(&self) {
            self.obj().close();
        }

        #[template_callback]
        fn on_add_author(&self) {
            let obj = self.obj();
            let row = obj.add_author(&Author::new(""));
            row.grab_focus();
            obj.validate();
        }
    }

    impl ObjectImpl for BulkMetadataDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.save_button.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.save()
            ));
        }
    }

    impl WidgetImpl for BulkMetadataDialog {}
    impl AdwDialogImpl for BulkMetadataDialog {}
}

glib::wrapper! {
    pub struct BulkMetadataDialog(ObjectSubclass<imp::BulkMetadataDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl BulkMetadataDialog {
    pub fn new(books: &[Book], on_save: SaveFn) -> BulkMetadataDialog {
        let obj: BulkMetadataDialog = glib::Object::new();
        let imp = obj.imp();
        obj.set_title(&format!("Edit {} Books", books.len()));

        let authors = Initial::of(books, |m| m.authors.clone());
        if let Initial::Shared(authors) = &authors {
            for author in authors {
                obj.add_author(author);
            }
        }
        imp.authors_placeholder.set_label(match authors {
            Initial::Shared(_) => "No Authors",
            Initial::Mixed => "Mixed",
        });
        imp.initial_authors.replace(Some(authors));

        let fields = [
            (&*imp.series_row, Initial::of(books, |m| m.series.clone())),
            (
                &*imp.publisher_row,
                Initial::of(books, |m| m.publisher.clone()),
            ),
            (
                &*imp.language_row,
                Initial::of(books, |m| m.language.clone()),
            ),
        ];
        for (i, (row, initial)) in fields.into_iter().enumerate() {
            let field = FieldRow::new(row, initial);
            row.connect_changed(glib::clone!(
                #[weak]
                obj,
                move |row| {
                    if !row.text().trim().is_empty() {
                        obj.imp().fields.borrow()[i].cleared.set(false);
                    }
                    obj.validate();
                }
            ));
            field.clear.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.clear_field(i)
            ));
            imp.fields.borrow_mut().push(field);
        }
        imp.on_save.replace(Some(on_save));
        obj.validate();
        obj
    }

    /// Adds rows for `author`.
    fn add_author(&self, author: &Author) -> adw::EntryRow {
        let rows = AuthorRows::new(author);
        let name = rows.name.clone();
        for row in [&rows.name, &rows.sort_name] {
            row.connect_changed(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |_| dialog.validate()
            ));
        }
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
        drop(authors);
        self.validate();
    }

    /// Empties the field at `i` and marks it to be cleared on every Book.
    fn clear_field(&self, i: usize) {
        let row = {
            let fields = self.imp().fields.borrow();
            fields[i].cleared.set(true);
            fields[i].row.clone()
        };
        row.set_text("");
        self.validate();
    }

    /// The new author list, or `None` while the rows still hold what the
    /// Books had (nothing, when their authors differ).
    fn edited_authors(&self) -> Option<Vec<Author>> {
        let imp = self.imp();
        let rows = imp.authors.borrow();
        let initial = imp.initial_authors.borrow();
        let initial: &[Author] = match initial.as_ref() {
            Some(Initial::Shared(authors)) => authors,
            _ => &[],
        };
        let unchanged = rows.len() == initial.len()
            && rows.iter().zip(initial).all(|(rows, author)| {
                rows.name.text() == author.name && rows.sort_name.text() == author.sort_name
            });
        if unchanged {
            return None;
        }
        Some(rows.iter().filter_map(AuthorRows::author).collect())
    }

    fn edit(&self) -> MetadataEdit {
        let fields = self.imp().fields.borrow();
        let field = |i: usize| fields.get(i).map_or(FieldEdit::Keep, FieldRow::edit);
        MetadataEdit {
            authors: self.edited_authors(),
            series: field(0),
            publisher: field(1),
            language: field(2),
        }
    }

    /// Save needs a change, and at least one author if they were edited.
    fn validate(&self) {
        let imp = self.imp();
        for field in imp.fields.borrow().iter() {
            field.update_status();
        }
        let edit = self.edit();
        let has_authors = edit.authors.as_ref().is_none_or(|a| !a.is_empty());
        imp.save_button
            .set_sensitive(!edit.is_empty() && has_authors);
    }

    fn save(&self) {
        let imp = self.imp();
        let edit = self.edit();
        let result = match imp.on_save.borrow().as_ref() {
            Some(save) => save(edit),
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
