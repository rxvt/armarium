mod book_details;
mod book_list;
mod book_object;
mod book_tile;
mod metadata_dialog;
mod settings;
mod thumbnails;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "io.github.rxvt.Armarium";

fn main() -> glib::ExitCode {
    gio::resources_register_include!("armarium.gresource").expect("bundled resources are valid");

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .resource_base_path("/io/github/rxvt/Armarium")
        .build();
    app.connect_startup(setup_actions);
    app.connect_activate(|app| {
        let window = app
            .active_window()
            .unwrap_or_else(|| window::Window::new(app).upcast());
        window.present();
        if let Some(path) = std::env::var_os("ARMARIUM_SCREENSHOT") {
            screenshot_after_startup(&window, path.into());
        }
    });
    app.run()
}

/// Development aid: renders `window` to a PNG at `path` once it has settled,
/// then quits. Lets the UI be checked without access to the screen.
fn screenshot_after_startup(window: &gtk::Window, path: std::path::PathBuf) {
    let window = window.clone();
    glib::timeout_add_seconds_local_once(2, move || {
        let (w, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w as f64, h as f64);
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
            let texture = renderer.render_texture(node, None);
            if let Err(e) = texture.save_to_png(&path) {
                eprintln!("screenshot failed: {e}");
            }
        } else {
            eprintln!("screenshot failed: the window has nothing to render");
        }
        // Not close(), which would only close an open dialog.
        window.destroy();
    });
}

fn setup_actions(app: &adw::Application) {
    let about = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| {
            let dialog = adw::AboutDialog::builder()
                .application_name("Armarium")
                .application_icon("accessories-dictionary")
                .developer_name("Shane Anderson")
                .version(env!("CARGO_PKG_VERSION"))
                .comments("A personal library for DRM-free ebooks")
                .website("https://github.com/rxvt/armarium")
                .issue_url("https://github.com/rxvt/armarium/issues")
                .license_type(gtk::License::Gpl30)
                .build();
            dialog.present(app.active_window().as_ref());
        })
        .build();
    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &adw::Application, _, _| app.quit())
        .build();
    app.add_action_entries([about, quit]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
}
