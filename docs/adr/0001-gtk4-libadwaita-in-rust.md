# GTK4 + libadwaita, written in Rust

The app targets a single user on Omarchy (Hyprland, GNOME/GTK apps, Adwaita-dark). We chose GTK4 + libadwaita via gtk4-rs because `GtkGridView`/`GtkColumnView` are virtualized list views that map directly onto the cover grid and list view, and libadwaita follows the system light/dark scheme so the app blends with the rest of the desktop. Rust gives a single fast binary for the heavy lifting (EPUB/PDF parsing, cover rendering, SQLite, later USB device I/O).

## Considered Options

- **Qt6** (what Calibre uses): mature and dense, but visually foreign next to GTK apps on this desktop.
- **Python + PyGObject**: fastest to prototype, but slower for large libraries and bulk cover generation.
- **iced / Slint / egui**: no ready-made virtualized grid; thinner accessibility and native integration.
