//! The main window: header bar, cover grid and list, details sidebar,
//! Import, opening and deleting Books, and the connected Device and Sending
//! Books to it.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use armarium_core::Library;
use armarium_core::device::{Device, SendOutcome, SendReport, Stage};
use armarium_core::import::{ImportReport, Outcome};
use armarium_core::model::{Book, BookId, Format, FormatKind, Metadata, MetadataEdit, SortKey};
use armarium_core::suggest::Suggestions;
use gtk::{gdk, gio, glib};

use crate::book_details::BookDetails;
use crate::book_list;
use crate::book_object::BookObject;
use crate::book_tile::BookTile;
use crate::bulk_metadata_dialog::BulkMetadataDialog;
use crate::device_watch::{Connected, DeviceWatch};
use crate::metadata_dialog::MetadataDialog;
use crate::profile;
use crate::settings::Settings;
use crate::steady_content::SteadyContent;
use crate::thumbnails::Thumbnails;

const DEFAULT_COVER_WIDTH: i32 = 160;
/// Cover widths Ctrl+scroll and Ctrl+± step through.
const COVER_WIDTHS: std::ops::RangeInclusive<i32> = 96..=320;
const COVER_WIDTH_STEP: i32 = 32;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rxvt/Armarium/ui/window.ui")]
    pub struct Window {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub device_indicator: TemplateChild<gtk::Box>,
        #[template_child]
        pub device_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub device_free: TemplateChild<gtk::Label>,
        #[template_child]
        pub device_eject: TemplateChild<gtk::Button>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub grid_view: TemplateChild<gtk::GridView>,
        #[template_child]
        pub column_view: TemplateChild<gtk::ColumnView>,
        #[template_child]
        pub view_toggle: TemplateChild<adw::ToggleGroup>,
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,
        #[template_child]
        pub details: TemplateChild<BookDetails>,
        #[template_child]
        pub progress_bar: TemplateChild<gtk::ProgressBar>,

        pub library: RefCell<Option<Library>>,
        pub store: RefCell<Option<gio::ListStore>>,
        pub filter: RefCell<Option<gtk::CustomFilter>>,
        pub filtered: RefCell<Option<gtk::FilterListModel>>,
        pub selection: RefCell<Option<gtk::MultiSelection>>,
        /// The list's sortable columns, which hold the order for both views.
        pub sort_columns: RefCell<Vec<(SortKey, gtk::ColumnViewColumn)>>,
        /// The key the views are sorted by.
        pub sort_key: Cell<Option<SortKey>>,
        pub thumbnails: RefCell<Option<Rc<Thumbnails>>>,
        pub cover_width: Cell<i32>,
        pub grid_factory: OnceCell<gtk::SignalListItemFactory>,
        pub settings: OnceCell<Settings>,
        pub importing: Cell<bool>,
        pub sending: Cell<bool>,
        /// Set when the details sidebar was toggled off with I.
        pub details_hidden: Cell<bool>,
        pub device_watch: OnceCell<DeviceWatch>,
        pub device: RefCell<Option<Connected>>,
        /// The Books on the connected Device.
        pub on_device: RefCell<HashSet<BookId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "ArmariumWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            BookDetails::ensure_type();
            SteadyContent::ensure_type();
            klass.bind_template();

            klass.install_action("win.import-files", None, |win, _, _| win.choose_files());
            klass.install_action("win.import-folder", None, |win, _, _| win.choose_folders());
            klass.add_binding_action(
                gdk::Key::o,
                gdk::ModifierType::CONTROL_MASK,
                "win.import-files",
            );
            klass.add_binding_action(gdk::Key::f, gdk::ModifierType::CONTROL_MASK, "win.search");
            klass.install_action("win.search", None, |win, _, _| {
                win.imp().search_entry.grab_focus();
            });
            klass.install_action(
                "win.open-format",
                Some(glib::VariantTy::STRING),
                |win, _, ext| {
                    if let Some(ext) = ext.and_then(|v| v.get::<String>()) {
                        win.open_selected_format(&ext);
                    }
                },
            );
            klass.install_action("win.show-in-files", None, |win, _, _| win.show_in_files());
            klass.install_action("win.edit", None, |win, _, _| win.edit_selected());
            klass.install_action("win.toggle-details", None, |win, _, _| win.toggle_details());
            klass.install_action("win.preferences", None, |win, _, _| win.show_preferences());
            klass.add_binding_action(
                gdk::Key::comma,
                gdk::ModifierType::CONTROL_MASK,
                "win.preferences",
            );
            klass.install_action("win.zoom-in", None, |win, _, _| win.zoom(1));
            klass.install_action("win.zoom-out", None, |win, _, _| win.zoom(-1));
            klass.install_action("win.zoom-reset", None, |win, _, _| {
                win.set_cover_width(DEFAULT_COVER_WIDTH)
            });
            for (key, action) in [
                (gdk::Key::plus, "win.zoom-in"),
                (gdk::Key::equal, "win.zoom-in"),
                (gdk::Key::KP_Add, "win.zoom-in"),
                (gdk::Key::minus, "win.zoom-out"),
                (gdk::Key::KP_Subtract, "win.zoom-out"),
                (gdk::Key::_0, "win.zoom-reset"),
                (gdk::Key::KP_0, "win.zoom-reset"),
            ] {
                klass.add_binding_action(key, gdk::ModifierType::CONTROL_MASK, action);
            }
            klass.install_action("win.delete", None, |win, _, _| win.confirm_delete_books());
            klass.install_action("win.eject-device", None, |win, _, _| win.eject_device());
            klass.install_action("win.send-to-device", None, |win, _, _| win.send_selected());
            klass.install_action(
                "win.delete-format",
                Some(glib::VariantTy::STRING),
                |win, _, ext| {
                    if let Some(ext) = ext.and_then(|v| v.get::<String>()) {
                        win.confirm_delete_format(&ext);
                    }
                },
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            let settings = Settings::load();
            self.cover_width
                .set(settings.cover_width().map_or(DEFAULT_COVER_WIDTH, |w| {
                    w.clamp(*COVER_WIDTHS.start(), *COVER_WIDTHS.end())
                }));
            if let Some(view) = settings.view() {
                self.view_toggle.set_active_name(Some(&view));
            }
            let _ = self.settings.set(settings);
            if profile::DEVEL {
                obj.add_css_class("devel");
                obj.set_title(Some("Armarium (Development)"));
            }
            self.thumbnails.replace(Some(Rc::new(Thumbnails::new())));
            obj.setup_models();
            obj.setup_grid();
            obj.setup_list();
            obj.setup_search();
            obj.setup_drop();
            obj.open_library(obj.library_root());
            obj.watch_devices();
        }
    }

    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Window {
    pub fn new(app: &adw::Application) -> Window {
        glib::Object::builder().property("application", app).build()
    }

    fn settings(&self) -> &Settings {
        self.imp().settings.get().expect("settings are loaded")
    }

    /// `$ARMARIUM_LIBRARY` (for trying the app on a scratch Library), else
    /// the folder chosen in Preferences, else the default Library (`~/Books`
    /// for the installed app).
    fn library_root(&self) -> PathBuf {
        std::env::var_os("ARMARIUM_LIBRARY")
            .map(PathBuf::from)
            .or_else(|| self.settings().library())
            .unwrap_or_else(profile::default_library)
    }

    fn store(&self) -> gio::ListStore {
        self.imp()
            .store
            .borrow()
            .clone()
            .expect("models are set up")
    }

    fn setup_models(&self) {
        let imp = self.imp();
        let store = gio::ListStore::new::<BookObject>();
        let entry = imp.search_entry.get();
        let filter = gtk::CustomFilter::new(move |item| {
            let query = entry.text();
            item.downcast_ref::<BookObject>()
                .is_some_and(|book| book.matches(&query))
        });
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        filtered.connect_items_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _, _, _| win.update_page()
        ));
        // The list's column headers choose the order for both views.
        let sorted = gtk::SortListModel::new(Some(filtered.clone()), imp.column_view.sorter());
        let selection = gtk::MultiSelection::new(Some(sorted));
        imp.grid_view.set_model(Some(&selection));
        imp.column_view.set_model(Some(&selection));
        selection.connect_selection_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _, _| win.update_selection()
        ));
        selection.connect_items_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _, _, _| win.update_selection()
        ));

        imp.store.replace(Some(store));
        imp.filter.replace(Some(filter));
        imp.filtered.replace(Some(filtered));
        imp.selection.replace(Some(selection));
        self.update_selection();
    }

    fn selection(&self) -> gtk::MultiSelection {
        self.imp()
            .selection
            .borrow()
            .clone()
            .expect("models are set up")
    }

    fn selected_books(&self) -> Vec<BookObject> {
        let selection = self.selection();
        let set = selection.selection();
        (0..set.size() as u32)
            .filter_map(|i| selection.item(set.nth(i)).and_downcast::<BookObject>())
            .collect()
    }

    /// The selected Book, when exactly one is selected.
    fn selected_book(&self) -> Option<BookObject> {
        match <[BookObject; 1]>::try_from(self.selected_books()) {
            Ok([book]) => Some(book),
            Err(_) => None,
        }
    }

    /// Fills the details sidebar from the selection and enables the actions
    /// that apply. The sidebar stays open whatever is selected, so the grid
    /// beside it keeps its columns.
    fn update_selection(&self) {
        let imp = self.imp();
        let count = self.selection().selection().size();
        let single = self.selected_book();
        if let Some(obj) = &single {
            let book = obj.book();
            let cover = self.with_library(|lib| lib.cover_path(&book)).flatten();
            let thumbnails = imp.thumbnails.borrow().clone().unwrap();
            imp.details.show(&book, obj.on_device(), cover, &thumbnails);
        } else {
            imp.details.show_selection_count(count);
        }
        let has_books = imp.store.borrow().as_ref().is_some_and(|s| s.n_items() > 0);
        imp.split_view
            .set_show_sidebar(has_books && !imp.details_hidden.get());
        for action in ["win.open-format", "win.show-in-files", "win.delete-format"] {
            self.action_set_enabled(action, single.is_some());
        }
        self.action_set_enabled("win.edit", count > 0);
        self.action_set_enabled("win.delete", count > 0);
        let connected = imp.device.borrow().is_some();
        imp.details.set_can_send(connected && count > 0);
        self.action_set_enabled(
            "win.send-to-device",
            connected && count > 0 && !imp.sending.get() && !imp.importing.get(),
        );
    }

    fn toggle_details(&self) {
        let imp = self.imp();
        imp.details_hidden.set(!imp.details_hidden.get());
        self.update_selection();
    }

    fn setup_grid(&self) {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let tile = BookTile::new();
                tile.set_cover_width(win.imp().cover_width.get());
                tile.bind_on_device(item);
                win.attach_context_menu(tile.upcast_ref(), item);
                item.set_child(Some(&tile));
            }
        ));
        factory.connect_bind(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let (Some(tile), Some(obj)) = (
                    item.child().and_downcast::<BookTile>(),
                    item.item().and_downcast::<BookObject>(),
                ) else {
                    return;
                };
                let book = obj.book();
                let cover = win.with_library(|lib| lib.cover_path(&book)).flatten();
                let thumbnails = win.imp().thumbnails.borrow().clone().unwrap();
                tile.bind(&book, cover, &thumbnails);
            }
        ));
        factory.connect_unbind(|_, item| {
            if let Some(tile) = item
                .downcast_ref::<gtk::ListItem>()
                .unwrap()
                .child()
                .and_downcast::<BookTile>()
            {
                tile.unbind();
            }
        });
        let grid = &self.imp().grid_view;
        grid.set_factory(Some(&factory));
        let _ = self.imp().grid_factory.set(factory);
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        scroll.connect_scroll(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |scroll, _, dy| {
                if !scroll
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK)
                {
                    return glib::Propagation::Proceed;
                }
                win.zoom(if dy < 0.0 { 1 } else { -1 });
                glib::Propagation::Stop
            }
        ));
        grid.add_controller(scroll);
        grid.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |grid, position| win.open_at(grid.model(), position)
        ));
        grid.add_controller(view_shortcuts());
    }

    fn setup_list(&self) {
        let imp = self.imp();
        let thumbnails = imp.thumbnails.borrow().clone().unwrap();
        let cover_path: book_list::CoverPath = Rc::new(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            None,
            move |book| win.with_library(|lib| lib.cover_path(book)).flatten()
        ));
        let setup_cell: book_list::SetupCell = Rc::new(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |widget, cell| win.attach_context_menu(widget, cell.upcast_ref())
        ));
        let columns = book_list::add_columns(&imp.column_view, thumbnails, cover_path, setup_cell);
        self.setup_sort(columns);
        imp.column_view.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |view, position| win.open_at(view.model(), position)
        ));
        imp.column_view.add_controller(view_shortcuts());
        imp.view_toggle.connect_active_name_notify(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |toggle| {
                if let Some(view) = toggle.active_name() {
                    win.settings().set_view(&view);
                }
                win.update_page();
            }
        ));
    }

    /// The Sort button's `win.sort-by` and `win.sort-order` and the list's
    /// column headers choose one order, which settings remember. It starts
    /// at Date added, newest first.
    fn setup_sort(&self, columns: Vec<(SortKey, gtk::ColumnViewColumn)>) {
        let imp = self.imp();
        let (key, descending) = self
            .settings()
            .sort()
            .and_then(|(key, descending)| Some((SortKey::parse(&key)?, descending)))
            .unwrap_or((SortKey::DateAdded, true));
        imp.sort_columns.replace(columns);

        let sort_by = gio::SimpleAction::new_stateful(
            "sort-by",
            Some(glib::VariantTy::STRING),
            &key.as_str().to_variant(),
        );
        sort_by.connect_change_state(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, value| {
                if let Some(key) = value
                    .and_then(|v| v.get::<String>())
                    .and_then(|s| SortKey::parse(&s))
                {
                    win.sort_by(key, key.starts_descending());
                }
            }
        ));
        let sort_order = gio::SimpleAction::new_stateful(
            "sort-order",
            Some(glib::VariantTy::STRING),
            &order_name(descending).to_variant(),
        );
        sort_order.connect_change_state(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, value| {
                if let (Some(order), Some(key)) = (
                    value.and_then(|v| v.get::<String>()),
                    win.imp().sort_key.get(),
                ) {
                    win.sort_by(key, order == order_name(true));
                }
            }
        ));
        self.add_action(&sort_by);
        self.add_action(&sort_order);

        if let Some(sorter) = imp
            .column_view
            .sorter()
            .and_downcast::<gtk::ColumnViewSorter>()
        {
            sorter.connect_changed(glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |sorter, _| win.sort_changed(sorter)
            ));
        }
        imp.sort_key.set(Some(key));
        self.sort_by(key, descending);
    }

    fn sort_by(&self, key: SortKey, descending: bool) {
        let column = self
            .imp()
            .sort_columns
            .borrow()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, c)| c.clone());
        let order = if descending {
            gtk::SortType::Descending
        } else {
            gtk::SortType::Ascending
        };
        self.imp()
            .column_view
            .sort_by_column(column.as_ref(), order);
    }

    /// Follows a new order, from the Sort button or a column header, into
    /// the button's actions and settings. A header click on a new column
    /// always sorts ascending, so one for a key that starts descending is
    /// flipped.
    fn sort_changed(&self, sorter: &gtk::ColumnViewSorter) {
        let imp = self.imp();
        let Some(column) = sorter.primary_sort_column() else {
            return;
        };
        let Some(key) = imp
            .sort_columns
            .borrow()
            .iter()
            .find(|(_, c)| *c == column)
            .map(|(k, _)| *k)
        else {
            return;
        };
        let descending = sorter.primary_sort_order() == gtk::SortType::Descending;
        if imp.sort_key.replace(Some(key)) != Some(key) && key.starts_descending() && !descending {
            self.sort_by(key, true);
            return;
        }
        for (name, state) in [
            ("sort-by", key.as_str()),
            ("sort-order", order_name(descending)),
        ] {
            if let Some(action) = self.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                action.set_state(&state.to_variant());
            }
        }
        self.settings().set_sort(key.as_str(), descending);
    }

    fn setup_search(&self) {
        let imp = self.imp();
        imp.search_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| {
                if let Some(filter) = win.imp().filter.borrow().as_ref() {
                    filter.changed(gtk::FilterChange::Different);
                }
            }
        ));
        imp.search_entry
            .connect_stop_search(|entry| entry.set_text(""));
    }

    fn setup_drop(&self) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        target.connect_drop(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let Ok(files) = value.get::<gdk::FileList>() else {
                    return false;
                };
                let paths: Vec<PathBuf> = files.files().iter().filter_map(|f| f.path()).collect();
                win.import(paths);
                true
            }
        ));
        self.imp().stack.add_controller(target);
    }

    fn with_library<T>(&self, f: impl FnOnce(&Library) -> T) -> Option<T> {
        self.imp().library.borrow().as_ref().map(f)
    }

    fn with_library_mut<T>(&self, f: impl FnOnce(&mut Library) -> T) -> Option<T> {
        self.imp().library.borrow_mut().as_mut().map(f)
    }

    fn open_library(&self, root: PathBuf) {
        match Library::open(root) {
            Ok(library) => {
                self.imp().library.replace(Some(library));
                self.reload();
            }
            Err(e) => self.toast(&format!("Could not open the library: {e}")),
        }
    }

    /// Reloads every Book from the database.
    fn reload(&self) {
        let books = match self.with_library(|lib| lib.books()) {
            Some(Ok(books)) => books,
            Some(Err(e)) => {
                self.toast(&format!("Could not read the library: {e}"));
                return;
            }
            None => Vec::new(),
        };
        let selected: HashSet<BookId> = self
            .selected_books()
            .iter()
            .map(|obj| obj.book().id)
            .collect();
        let objects: Vec<BookObject> = books.into_iter().map(BookObject::new).collect();
        let store = self.store();
        store.splice(0, store.n_items(), &objects);
        self.update_ticks();
        // Keep the same Books selected, e.g. after deleting one Format.
        let selection = self.selection();
        for position in 0..selection.n_items() {
            if let Some(obj) = selection.item(position).and_downcast::<BookObject>()
                && selected.contains(&obj.book().id)
            {
                selection.select_item(position, false);
            }
        }
        self.update_page();
    }

    fn update_page(&self) {
        let imp = self.imp();
        let total = imp.store.borrow().as_ref().map_or(0, |s| s.n_items());
        let shown = imp.filtered.borrow().as_ref().map_or(0, |f| f.n_items());
        let page = match (total, shown) {
            (0, _) => "empty",
            (_, 0) => "no-results",
            _ => &imp
                .view_toggle
                .active_name()
                .unwrap_or_else(|| "grid".into()),
        };
        imp.stack.set_visible_child_name(page);
    }

    fn toast(&self, title: &str) {
        let toast = adw::Toast::builder().title(title).use_markup(false).build();
        self.imp().toast_overlay.add_toast(toast);
    }

    fn choose_files(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("EPUB and PDF"));
        for kind in FormatKind::ALL {
            filter.add_suffix(kind.extension());
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Import Books")
            .accept_label("_Import")
            .filters(&filters)
            .build();
        dialog.open_multiple(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |result| {
                    if let Ok(files) = result {
                        win.import(file_paths(&files));
                    }
                }
            ),
        );
    }

    fn choose_folders(&self) {
        let dialog = gtk::FileDialog::builder()
            .title("Import Folders")
            .accept_label("_Import")
            .build();
        dialog.select_multiple_folders(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |result| {
                    if let Ok(folders) = result {
                        win.import(file_paths(&folders));
                    }
                }
            ),
        );
    }

    /// Imports `paths` on a worker thread with its own database connection,
    /// showing progress, then reloads and shows a summary toast.
    fn import(&self, paths: Vec<PathBuf>) {
        let imp = self.imp();
        if paths.is_empty() {
            return;
        }
        if imp.importing.get() {
            self.toast("An import is already running");
            return;
        }
        if imp.sending.get() {
            self.toast("Wait for the books to be sent to the Kindle");
            return;
        }
        let Some(root) = self.with_library(|lib| lib.root().to_owned()) else {
            return;
        };
        imp.importing.set(true);
        self.action_set_enabled("win.import-files", false);
        self.action_set_enabled("win.import-folder", false);
        self.update_selection();

        let progress = Arc::new(Mutex::new((0usize, 0usize)));
        imp.progress_bar.set_fraction(0.0);
        imp.progress_bar.set_visible(true);
        let ticker = glib::timeout_add_local(
            Duration::from_millis(100),
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                #[strong]
                progress,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    let (done, total) = *progress.lock().unwrap();
                    if total > 0 {
                        win.imp()
                            .progress_bar
                            .set_fraction(done as f64 / total as f64);
                    }
                    glib::ControlFlow::Continue
                }
            ),
        );

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = win)]
            self,
            async move {
                let result = gio::spawn_blocking(move || {
                    let mut library = Library::open(root)?;
                    library.import(&paths, |p| *progress.lock().unwrap() = (p.done, p.total))
                })
                .await;
                ticker.remove();
                let imp = win.imp();
                imp.progress_bar.set_visible(false);
                imp.importing.set(false);
                win.action_set_enabled("win.import-files", true);
                win.action_set_enabled("win.import-folder", true);

                // The Import may have created the Library, so reopen it.
                let root = win
                    .with_library(|lib| lib.root().to_owned())
                    .unwrap_or_else(|| win.library_root());
                win.open_library(root);
                match result {
                    Ok(Ok(report)) => win.show_import_summary(report),
                    Ok(Err(e)) => win.toast(&format!("Import failed: {e}")),
                    Err(_) => win.toast("Import failed unexpectedly"),
                }
            }
        ));
    }

    fn show_import_summary(&self, report: ImportReport) {
        let mut parts = Vec::new();
        if report.added() > 0 {
            parts.push(format!("{} added", report.added()));
        }
        if report.merged() > 0 {
            parts.push(format!("{} merged as formats", report.merged()));
        }
        if report.skipped() > 0 {
            parts.push(format!("{} skipped", report.skipped()));
        }
        if parts.is_empty() {
            parts.push("No EPUB or PDF files found".into());
        }
        let toast = adw::Toast::builder()
            .title(parts.join(" · "))
            .timeout(8)
            .build();
        if !report.items.is_empty() {
            toast.set_button_label(Some("Details"));
            let report = Rc::new(report);
            toast.connect_button_clicked(glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |_| win.show_import_details(&report)
            ));
        }
        self.imp().toast_overlay.add_toast(toast);
    }

    fn show_import_details(&self, report: &ImportReport) {
        let rows = report.items.iter().map(|(path, outcome)| {
            let status = match outcome {
                Outcome::Added(_) => "Added".to_owned(),
                Outcome::Merged(_) => "Added as a new format".to_owned(),
                Outcome::Skipped(reason) => format!("Skipped: {reason}"),
            };
            let name = path.file_name().unwrap_or(path.as_os_str());
            adw::ActionRow::builder()
                .title(glib::markup_escape_text(&name.to_string_lossy()))
                .subtitle(glib::markup_escape_text(&status))
                .tooltip_text(path.display().to_string())
                .build()
        });
        self.show_details_dialog("Import Details", rows);
    }

    /// A dialog listing what happened to each item of an Import or Send.
    fn show_details_dialog(&self, heading: &str, rows: impl IntoIterator<Item = adw::ActionRow>) {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        for row in rows {
            list.append(&row);
        }
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(420)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .extra_child(&scrolled)
            .build();
        dialog.add_response("close", "_Close");
        dialog.present(Some(self));
    }

    fn open_at(&self, model: Option<gtk::SelectionModel>, position: u32) {
        if let Some(obj) = model
            .and_then(|m| m.item(position))
            .and_downcast::<BookObject>()
        {
            self.open_book(&obj);
        }
    }

    /// Opens the Book's EPUB if it has one, else its PDF, in the default Reader.
    fn open_book(&self, obj: &BookObject) {
        let book = obj.book();
        if let Some(format) = book
            .format(FormatKind::Epub)
            .or_else(|| book.format(FormatKind::Pdf))
        {
            self.open_format(&book, format);
        }
    }

    fn open_selected_format(&self, ext: &str) {
        let (Some(obj), Some(kind)) = (self.selected_book(), FormatKind::from_extension(ext))
        else {
            return;
        };
        let book = obj.book();
        if let Some(format) = book.format(kind) {
            self.open_format(&book, format);
        }
    }

    fn open_format(&self, book: &Book, format: &Format) {
        let Some(path) = self.with_library(|lib| lib.format_path(book, format)) else {
            return;
        };
        if !path.exists() {
            self.toast(&format!(
                "“{}” is missing from the library folder",
                format.file_name
            ));
            return;
        }
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&path)));
        launcher.launch(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |result| win.report_launch(result, "Could not open the book")
            ),
        );
    }

    /// Opens the file manager on the selected Book's first Format, or its
    /// folder if the Format file is gone.
    fn show_in_files(&self) {
        let Some(obj) = self.selected_book() else {
            return;
        };
        let book = obj.book();
        let Some((dir, file)) = self.with_library(|lib| {
            let file = book.formats.first().map(|f| lib.format_path(&book, f));
            (lib.book_dir(&book), file)
        }) else {
            return;
        };
        let callback = glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |result| win.report_launch(result, "Could not show the book")
        );
        match file.filter(|f| f.exists()) {
            Some(file) => gtk::FileLauncher::new(Some(&gio::File::for_path(file)))
                .open_containing_folder(Some(self), gio::Cancellable::NONE, callback),
            None => gtk::FileLauncher::new(Some(&gio::File::for_path(dir))).launch(
                Some(self),
                gio::Cancellable::NONE,
                callback,
            ),
        }
    }

    fn report_launch(&self, result: Result<(), glib::Error>, what: &str) {
        if let Err(e) = result
            && !e.matches(gtk::DialogError::Dismissed)
        {
            self.toast(&format!("{what}: {}", e.message()));
        }
    }

    /// Right-clicking `widget`, which shows the item at `item`'s position,
    /// selects that Book (unless it is already part of the selection) and
    /// pops up a menu for the selection.
    fn attach_context_menu(&self, widget: &gtk::Widget, item: &gtk::ListItem) {
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let item = item.downgrade();
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[weak]
            widget,
            move |gesture, _, x, y| {
                let Some(item) = item.upgrade() else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let selection = win.selection();
                if !selection.is_selected(item.position()) {
                    selection.select_item(item.position(), true);
                }
                win.popup_context_menu(&widget, x, y);
            }
        ));
        widget.add_controller(click);
    }

    fn popup_context_menu(&self, widget: &gtk::Widget, x: f64, y: f64) {
        let books = self.selected_books();
        let menu = gio::Menu::new();
        if let [obj] = books.as_slice() {
            let open = gio::Menu::new();
            for format in &obj.book().formats {
                open.append(
                    Some(&format!("Open {}", format.kind.label())),
                    Some(&format!("win.open-format::{}", format.kind.extension())),
                );
            }
            menu.append_section(None, &open);
            let book = gio::Menu::new();
            book.append(Some("Edit Metadata…"), Some("win.edit"));
            book.append(Some("Show in Files"), Some("win.show-in-files"));
            menu.append_section(None, &book);
        } else if books.len() > 1 {
            let edit = gio::Menu::new();
            edit.append(
                Some(&format!("Edit Metadata of {} Books…", books.len())),
                Some("win.edit"),
            );
            menu.append_section(None, &edit);
        }
        if self.imp().device.borrow().is_some() {
            let send = gio::Menu::new();
            let label = match books.len() {
                1 => "Send to Kindle".to_owned(),
                n => format!("Send {n} Books to Kindle"),
            };
            send.append(Some(&label), Some("win.send-to-device"));
            menu.append_section(None, &send);
        }
        let delete = gio::Menu::new();
        let label = match books.len() {
            1 => "Move to Trash…".to_owned(),
            n => format!("Move {n} Books to Trash…"),
        };
        delete.append(Some(&label), Some("win.delete"));
        menu.append_section(None, &delete);

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(widget);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            // Unparent once the menu item's action has run.
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    }

    /// Makes grid Covers one step larger (`steps` > 0) or smaller.
    fn zoom(&self, steps: i32) {
        self.set_cover_width(self.imp().cover_width.get() + steps * COVER_WIDTH_STEP);
    }

    /// Resizes the grid's Covers and remembers the size.
    fn set_cover_width(&self, width: i32) {
        let imp = self.imp();
        let width = width.clamp(*COVER_WIDTHS.start(), *COVER_WIDTHS.end());
        if width == imp.cover_width.get() {
            return;
        }
        imp.cover_width.set(width);
        self.settings().set_cover_width(width);
        // Setting the factory again rebuilds every tile at the new size.
        imp.grid_view.set_factory(gtk::ListItemFactory::NONE);
        imp.grid_view.set_factory(imp.grid_factory.get());
    }

    fn show_preferences(&self) {
        let location = adw::ActionRow::builder()
            .title("Location")
            .subtitle(glib::markup_escape_text(&self.library_root().display().to_string()).as_str())
            .subtitle_selectable(true)
            .build();
        let change = gtk::Button::builder()
            .label("_Change…")
            .use_underline(true)
            .valign(gtk::Align::Center)
            .build();
        location.add_suffix(&change);
        let group = adw::PreferencesGroup::builder()
            .title("Library")
            .description(
                "Imported books are copied into this folder. Choosing another folder opens \
                 the library there, or starts a new one; books are never moved.",
            )
            .build();
        group.add(&location);
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let dialog = adw::PreferencesDialog::new();
        dialog.add(&page);
        change.connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[weak]
            location,
            move |_| win.choose_library(&location)
        ));
        dialog.present(Some(self));
    }

    fn choose_library(&self, location: &adw::ActionRow) {
        if self.imp().importing.get() {
            self.toast("Wait for the import to finish");
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title("Choose Library Folder")
            .accept_label("_Choose")
            .initial_folder(&gio::File::for_path(self.library_root()))
            .build();
        dialog.select_folder(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                #[weak]
                location,
                move |result| {
                    let Some(path) = result.ok().and_then(|f| f.path()) else {
                        return;
                    };
                    win.switch_library(&path);
                    location.set_subtitle(&glib::markup_escape_text(&path.display().to_string()));
                }
            ),
        );
    }

    /// Opens the Library at `path` (an empty one if it has none yet) and
    /// remembers it.
    fn switch_library(&self, path: &std::path::Path) {
        self.settings().set_library(path);
        // Ids from the old Library mean nothing in the new one.
        self.selection().unselect_all();
        self.open_library(path.to_owned());
    }

    /// Edits the selected Book's Metadata, or several Books' at once.
    fn edit_selected(&self) {
        let objs = self.selected_books();
        match objs.as_slice() {
            [] => {}
            [obj] => self.edit_book(obj),
            _ => self.edit_books(&objs),
        }
    }

    fn edit_book(&self, obj: &BookObject) {
        let book = obj.book();
        let cover = self.with_library(|lib| lib.cover_path(&book)).flatten();
        let thumbnails = self.imp().thumbnails.borrow().clone().unwrap();
        let id = book.id;
        let names: Vec<String> = self
            .store()
            .iter::<BookObject>()
            .flatten()
            .filter_map(|obj| obj.book().metadata.series.clone())
            .collect();
        let series = Suggestions::new(names.iter().map(String::as_str));
        let on_save = Box::new(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or_else]
            || Ok(()),
            move |metadata, cover| win.save_metadata(id, metadata, cover)
        ));
        MetadataDialog::new(&book, series, cover, &thumbnails, on_save).present(Some(self));
    }

    fn edit_books(&self, objs: &[BookObject]) {
        let books: Vec<Book> = objs.iter().map(|obj| obj.book().clone()).collect();
        let ids: Vec<BookId> = books.iter().map(|book| book.id).collect();
        let on_save = Box::new(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or_else]
            || Ok(()),
            move |edit| win.save_metadata_edit(&ids, &edit)
        ));
        BulkMetadataDialog::new(&books, on_save).present(Some(self));
    }

    /// Makes `edit` to each Book, then reloads. A failure stops the rest and
    /// says how many Books were updated before it.
    fn save_metadata_edit(&self, ids: &[BookId], edit: &MetadataEdit) -> Result<(), String> {
        let result = self.with_library_mut(|lib| lib.edit_metadata(ids, edit));
        self.reload();
        match result {
            Some(Err((updated, e))) => {
                Err(format!("Updated {updated} of {} Books: {e}", ids.len()))
            }
            _ => Ok(()),
        }
    }

    /// Saves edited Metadata and, if given, a new Cover, then reloads.
    fn save_metadata(
        &self,
        id: BookId,
        metadata: Metadata,
        cover: Option<PathBuf>,
    ) -> Result<(), String> {
        let result = self.with_library_mut(|lib| {
            let book = lib.update_metadata(id, metadata)?;
            if let Some(image) = &cover {
                lib.set_cover(id, image)?;
            }
            Ok::<_, armarium_core::Error>(lib.cover_path(&book))
        });
        // The Cover keeps its path, so drop the old texture from memory.
        if let Some(Ok(Some(path))) = &result
            && cover.is_some()
        {
            self.imp()
                .thumbnails
                .borrow()
                .as_ref()
                .unwrap()
                .forget(path);
        }
        self.reload();
        match result {
            Some(Err(e)) => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    fn watch_devices(&self) {
        self.action_set_enabled("win.eject-device", false);
        let watch = DeviceWatch::start(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |connected| win.set_device(connected)
        ));
        let _ = self.imp().device_watch.set(watch);
    }

    /// Connects `connected`, or with `None` forgets the Device.
    fn set_device(&self, connected: Option<Connected>) {
        if let Some(c) = &connected
            && let Err(e) = c.device.clean_up()
        {
            eprintln!("could not clean up {}: {e}", c.device.root().display());
        }
        self.imp().device.replace(connected);
        self.refresh_device();
    }

    /// Reads which Books are on the Device, and its free space, again.
    fn refresh_device(&self) {
        let imp = self.imp();
        let mut on_device = HashSet::new();
        let connected = imp.device.borrow();
        if let Some(c) = connected.as_ref() {
            match c.device.books() {
                Ok(books) => on_device.extend(books.into_keys()),
                Err(e) => self.toast(&format!("Could not read the books on {}: {e}", c.name)),
            }
            let free = c
                .device
                .free_space()
                .map(|bytes| format!("{} free", glib::format_size(bytes)))
                .unwrap_or_default();
            imp.device_name.set_label(&c.name);
            imp.device_free.set_label(&free);
            imp.device_eject
                .set_tooltip_text(Some(&format!("Eject {}", c.name)));
        }
        imp.device_indicator.set_visible(connected.is_some());
        self.action_set_enabled(
            "win.eject-device",
            connected.is_some() && !imp.sending.get(),
        );
        drop(connected);
        imp.on_device.replace(on_device);
        self.update_ticks();
        self.update_selection();
    }

    fn update_ticks(&self) {
        let on_device = self.imp().on_device.borrow();
        for obj in self.store().iter::<BookObject>().flatten() {
            let id = obj.book().id;
            obj.set_on_device(on_device.contains(&id));
        }
    }

    /// Ejects the Device so it can be unplugged. The Kindle only looks for
    /// new Books after a safe eject.
    fn eject_device(&self) {
        let Some((name, mount)) =
            (self.imp().device.borrow().as_ref()).map(|c| (c.name.clone(), c.mount.clone()))
        else {
            return;
        };
        let Some(mount) = mount else {
            // A folder standing in for a Device just goes away.
            self.set_device(None);
            return;
        };
        let done = glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |result: Result<(), glib::Error>| match result {
                Ok(()) => win.toast(&format!("{name} can be unplugged")),
                Err(e) if e.matches(gio::IOErrorEnum::FailedHandled) => {}
                Err(e) => win.toast(&format!("Could not eject {name}: {}", e.message())),
            }
        );
        let operation = gtk::MountOperation::new(Some(self));
        let flags = gio::MountUnmountFlags::NONE;
        if mount.can_eject() {
            mount.eject_with_operation(flags, Some(&operation), gio::Cancellable::NONE, done);
        } else {
            mount.unmount_with_operation(flags, Some(&operation), gio::Cancellable::NONE, done);
        }
    }

    /// Sends the selected Books to the Device on a worker thread with its
    /// own database connection, showing progress, then shows a summary toast.
    /// Eject waits until it finishes.
    fn send_selected(&self) {
        let imp = self.imp();
        if imp.sending.get() || imp.importing.get() {
            return;
        }
        let books: Vec<Book> = self
            .selected_books()
            .iter()
            .map(|obj| obj.book().clone())
            .collect();
        let Some(root) = self.with_library(|lib| lib.root().to_owned()) else {
            return;
        };
        let Some(device) = imp.device.borrow().as_ref().map(|c| c.device.clone()) else {
            return;
        };
        if books.is_empty() {
            return;
        }
        // The Kindle may have gone without its mount being removed yet.
        if Device::at(device.root()).is_none() {
            self.toast("Kindle disconnected");
            return;
        }
        imp.sending.set(true);
        self.action_set_enabled("win.eject-device", false);
        self.update_selection();

        // Converting takes the first half of the bar and copying the second.
        let progress = Arc::new(Mutex::new(0.0));
        imp.progress_bar.set_fraction(0.0);
        imp.progress_bar.set_visible(true);
        let ticker = glib::timeout_add_local(
            Duration::from_millis(100),
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                #[strong]
                progress,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    win.imp()
                        .progress_bar
                        .set_fraction(*progress.lock().unwrap());
                    glib::ControlFlow::Continue
                }
            ),
        );

        let titles: Vec<(BookId, String)> = books
            .iter()
            .map(|book| (book.id, book.metadata.title.clone()))
            .collect();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = win)]
            self,
            async move {
                let result = gio::spawn_blocking(move || {
                    let library = Library::open(root)?;
                    device.send(&library, &books, |p| {
                        let half = match p.stage {
                            Stage::Converting => 0.0,
                            Stage::Copying => 0.5,
                        };
                        *progress.lock().unwrap() = half + p.done as f64 / p.total as f64 / 2.0;
                    })
                })
                .await;
                ticker.remove();
                let imp = win.imp();
                imp.progress_bar.set_visible(false);
                imp.sending.set(false);
                win.refresh_device();
                match result {
                    Ok(Ok(report)) => win.show_send_summary(report, titles),
                    Ok(Err(e)) => win.toast(&format!("Nothing was sent: {e}")),
                    Err(_) => win.toast("Sending failed unexpectedly"),
                }
            }
        ));
    }

    /// `titles` names the Books in `report`.
    fn show_send_summary(&self, report: SendReport, titles: Vec<(BookId, String)>) {
        let failed = report
            .items
            .iter()
            .filter(|(_, outcome)| matches!(outcome, SendOutcome::Failed(_)))
            .count();
        let sent = report.items.len() - failed;
        let mut parts = Vec::new();
        if sent > 0 {
            parts.push(format!("{sent} sent"));
        }
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        let toast = adw::Toast::builder()
            .title(parts.join(" · "))
            .timeout(8)
            .build();
        toast.set_button_label(Some("Details"));
        let report = Rc::new((report, titles));
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.show_send_details(&report.0, &report.1)
        ));
        self.imp().toast_overlay.add_toast(toast);
    }

    fn show_send_details(&self, report: &SendReport, titles: &[(BookId, String)]) {
        let rows = report.items.iter().map(|(id, outcome)| {
            let title = titles
                .iter()
                .find(|(t, _)| t == id)
                .map_or("", |(_, title)| title);
            let status = match outcome {
                SendOutcome::Sent => "Sent".to_owned(),
                SendOutcome::Failed(reason) => format!("Failed: {reason}"),
            };
            adw::ActionRow::builder()
                .title(glib::markup_escape_text(title))
                .subtitle(glib::markup_escape_text(&status))
                .build()
        });
        self.show_details_dialog("Send Details", rows);
    }

    fn confirm_delete_books(&self) {
        let books = self.selected_books();
        let (heading, body) = match books.as_slice() {
            [] => return,
            [obj] => (
                format!("Move “{}” to the Trash?", obj.book().metadata.title),
                "Its files are moved to the Trash and it is removed from the library.",
            ),
            _ => (
                format!("Move {} Books to the Trash?", books.len()),
                "Their files are moved to the Trash and they are removed from the library.",
            ),
        };
        let ids: Vec<BookId> = books.iter().map(|obj| obj.book().id).collect();
        self.confirm_trash(&heading, body, move |win| {
            win.with_library_mut(|lib| lib.delete_books(&ids))
        });
    }

    fn confirm_delete_format(&self, ext: &str) {
        let (Some(obj), Some(kind)) = (self.selected_book(), FormatKind::from_extension(ext))
        else {
            return;
        };
        let book = obj.book();
        let Some(format) = book.format(kind) else {
            return;
        };
        let heading = format!(
            "Move the {} of “{}” to the Trash?",
            kind.label(),
            book.metadata.title
        );
        let body = if book.formats.len() == 1 {
            "It is the book’s only format, so the book is removed from the library too."
        } else {
            "The book keeps its other formats."
        };
        let (id, format) = (book.id, format.id);
        self.confirm_trash(&heading, body, move |win| {
            win.with_library_mut(|lib| lib.delete_format(id, format))
        });
    }

    /// Asks before moving files to the Trash, then runs `delete` and reloads.
    fn confirm_trash(
        &self,
        heading: &str,
        body: &str,
        delete: impl Fn(&Window) -> Option<armarium_core::Result<()>> + 'static,
    ) {
        let dialog = adw::AlertDialog::new(Some(heading), Some(body));
        dialog.add_responses(&[("cancel", "_Cancel"), ("delete", "_Move to Trash")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog.connect_response(
            Some("delete"),
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |_, _| {
                    if let Some(Err(e)) = delete(&win) {
                        win.toast(&format!("Could not move to the Trash: {e}"));
                    }
                    win.reload();
                }
            ),
        );
        dialog.present(Some(self));
    }
}

/// The `win.sort-order` state for a direction.
fn order_name(descending: bool) -> &'static str {
    if descending {
        "descending"
    } else {
        "ascending"
    }
}

/// Shortcuts on the selection in either view: Delete moves the selected
/// Books to the Trash and I toggles the details sidebar.
fn view_shortcuts() -> gtk::ShortcutController {
    let controller = gtk::ShortcutController::new();
    for (key, action) in [
        (gdk::Key::Delete, "win.delete"),
        (gdk::Key::i, "win.toggle-details"),
    ] {
        controller.add_shortcut(gtk::Shortcut::new(
            Some(gtk::KeyvalTrigger::new(key, gdk::ModifierType::empty())),
            Some(gtk::NamedAction::new(action)),
        ));
    }
    controller
}

fn file_paths(files: &gio::ListModel) -> Vec<PathBuf> {
    (0..files.n_items())
        .filter_map(|i| files.item(i).and_downcast::<gio::File>())
        .filter_map(|f| f.path())
        .collect()
}
