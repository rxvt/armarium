//! The main window: header bar, cover grid and list, details sidebar,
//! Import, and opening and deleting Books.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use armarium_core::Library;
use armarium_core::import::{ImportReport, Outcome};
use armarium_core::model::{Book, BookId, Format, FormatKind, Metadata};
use gtk::{gdk, gio, glib};

use crate::book_details::BookDetails;
use crate::book_list;
use crate::book_object::BookObject;
use crate::book_tile::BookTile;
use crate::metadata_dialog::MetadataDialog;
use crate::profile;
use crate::settings::Settings;
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
        pub thumbnails: RefCell<Option<Rc<Thumbnails>>>,
        pub cover_width: Cell<i32>,
        pub grid_factory: OnceCell<gtk::SignalListItemFactory>,
        pub settings: OnceCell<Settings>,
        pub importing: Cell<bool>,
        /// Set when the details sidebar was toggled off with I.
        pub details_hidden: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "ArmariumWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            BookDetails::ensure_type();
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
            imp.details.show(&book, cover, &thumbnails);
        } else {
            imp.details.show_selection_count(count);
        }
        let has_books = imp.store.borrow().as_ref().is_some_and(|s| s.n_items() > 0);
        imp.split_view
            .set_show_sidebar(has_books && !imp.details_hidden.get());
        for action in [
            "win.open-format",
            "win.show-in-files",
            "win.edit",
            "win.delete-format",
        ] {
            self.action_set_enabled(action, single.is_some());
        }
        self.action_set_enabled("win.delete", count > 0);
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
        let date_added =
            book_list::add_columns(&imp.column_view, thumbnails, cover_path, setup_cell);
        imp.column_view
            .sort_by_column(Some(&date_added), gtk::SortType::Descending);
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
        let Some(root) = self.with_library(|lib| lib.root().to_owned()) else {
            return;
        };
        imp.importing.set(true);
        self.action_set_enabled("win.import-files", false);
        self.action_set_enabled("win.import-folder", false);

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
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        for (path, outcome) in &report.items {
            let status = match outcome {
                Outcome::Added(_) => "Added".to_owned(),
                Outcome::Merged(_) => "Added as a new format".to_owned(),
                Outcome::Skipped(reason) => format!("Skipped: {reason}"),
            };
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&name))
                .subtitle(glib::markup_escape_text(&status))
                .tooltip_text(path.display().to_string())
                .build();
            list.append(&row);
        }
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(420)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading("Import Details")
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

    fn edit_selected(&self) {
        let Some(obj) = self.selected_book() else {
            return;
        };
        let book = obj.book();
        let cover = self.with_library(|lib| lib.cover_path(&book)).flatten();
        let thumbnails = self.imp().thumbnails.borrow().clone().unwrap();
        let id = book.id;
        let on_save = Box::new(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or_else]
            || Ok(()),
            move |metadata, cover| win.save_metadata(id, metadata, cover)
        ));
        MetadataDialog::new(&book, cover, &thumbnails, on_save).present(Some(self));
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
