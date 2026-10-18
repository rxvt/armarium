//! The main window: header bar, cover grid and list, Import and opening Books.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use armarium_core::Library;
use armarium_core::import::{ImportReport, Outcome};
use armarium_core::model::FormatKind;
use gtk::{gdk, gio, glib};

use crate::book_list;
use crate::book_object::BookObject;
use crate::book_tile::BookTile;
use crate::thumbnails::Thumbnails;

const DEFAULT_COVER_WIDTH: i32 = 160;

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
        pub progress_bar: TemplateChild<gtk::ProgressBar>,

        pub library: RefCell<Option<Library>>,
        pub store: RefCell<Option<gio::ListStore>>,
        pub filter: RefCell<Option<gtk::CustomFilter>>,
        pub filtered: RefCell<Option<gtk::FilterListModel>>,
        pub thumbnails: RefCell<Option<Rc<Thumbnails>>>,
        pub cover_width: Cell<i32>,
        pub importing: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "ArmariumWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
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
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.cover_width.set(DEFAULT_COVER_WIDTH);
            self.thumbnails.replace(Some(Rc::new(Thumbnails::new())));
            obj.setup_models();
            obj.setup_grid();
            obj.setup_list();
            obj.setup_search();
            obj.setup_drop();
            obj.open_library(default_library_root());
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

/// `~/Books`, or `$ARMARIUM_LIBRARY` for trying the app on a scratch Library.
fn default_library_root() -> PathBuf {
    std::env::var_os("ARMARIUM_LIBRARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| glib::home_dir().join("Books"))
}

impl Window {
    pub fn new(app: &adw::Application) -> Window {
        glib::Object::builder().property("application", app).build()
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

        imp.store.replace(Some(store));
        imp.filter.replace(Some(filter));
        imp.filtered.replace(Some(filtered));
    }

    fn setup_grid(&self) {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, item| {
                let tile = BookTile::new();
                tile.set_cover_width(win.imp().cover_width.get());
                item.downcast_ref::<gtk::ListItem>()
                    .unwrap()
                    .set_child(Some(&tile));
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
        grid.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |grid, position| win.open_at(grid.model(), position)
        ));
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
        let date_added = book_list::add_columns(&imp.column_view, thumbnails, cover_path);
        imp.column_view
            .sort_by_column(Some(&date_added), gtk::SortType::Descending);
        imp.column_view.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |view, position| win.open_at(view.model(), position)
        ));
        imp.view_toggle.connect_active_name_notify(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.update_page()
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
        imp.search_entry.set_key_capture_widget(Some(self));
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
        let objects: Vec<BookObject> = books.into_iter().map(BookObject::new).collect();
        let store = self.store();
        store.splice(0, store.n_items(), &objects);
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
                    .unwrap_or_else(default_library_root);
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
        let Some(format) = book
            .format(FormatKind::Epub)
            .or_else(|| book.format(FormatKind::Pdf))
        else {
            return;
        };
        let Some(path) = self.with_library(|lib| lib.format_path(&book, format)) else {
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
                move |result| {
                    if let Err(e) = result
                        && !e.matches(gtk::DialogError::Dismissed)
                    {
                        win.toast(&format!("Could not open the book: {}", e.message()));
                    }
                }
            ),
        );
    }
}

fn file_paths(files: &gio::ListModel) -> Vec<PathBuf> {
    (0..files.n_items())
        .filter_map(|i| files.item(i).and_downcast::<gio::File>())
        .filter_map(|f| f.path())
        .collect()
}
