# Armarium

A personal desktop ebook library for DRM-free books on Linux. Built with GTK4 and libadwaita in Rust.

Armarium collects your EPUBs and PDFs into one managed folder, keeps their metadata and covers, and lets you browse them as a cover grid or a list.

## Features

- **Import** files or whole folders. Files are copied, never moved. Duplicates are skipped, and a PDF and EPUB of the same book are merged into one Book.
- **Browse** as a resizable cover grid or a sortable list.
- **Search** as you type, ignoring case and accents ("bronte" finds "Brontë").
- **Edit** metadata (title, authors, series, publisher, ISBN and more) and replace covers.
- **Open** books in your default reader, or show them in your file manager.
- **Delete** to the Trash.

## Library

The Library lives in `~/Books` by default; you can change this in Preferences. It holds the book files, one folder per Book, plus `library.db` (SQLite), which is the source of truth for metadata. Cover thumbnails are cached under `~/.cache/armarium`.

## Building

Requirements:

- [rustup](https://rustup.rs) (the toolchain is pinned in `rust-toolchain.toml`)
- [just](https://github.com/casey/just)
- GTK 4.22+, libadwaita 1.9+, poppler-glib, SQLite
- `blueprint-compiler`

```sh
rustup toolchain install
just install    # installs to ~/.local, with an app launcher entry
```

`just uninstall` removes it again; your Library and settings are kept.

## Development

Run `just` to list every task. The main ones:

```sh
just run        # development build
just test       # all automated tests
just check      # everything CI runs
just hooks      # enable the repo's git hooks (once per clone)
just try DIR    # fresh scratch Library with books from DIR, then open the app
```

Development builds use their own app id, settings, cache and Library, so they never touch the installed app's data.

Vocabulary is defined in [GLOSSARY.md](GLOSSARY.md), the v1 scope in [docs/v1-plan.md](docs/v1-plan.md), and lasting decisions in [docs/adr/](docs/adr/).

## License

[GPL-3.0-or-later](LICENSE)
