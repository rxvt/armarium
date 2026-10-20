# Armarium — v1 Plan

Armarium is a personal, Linux-native ebook library for DRM-free books, in the spirit of Calibre but deliberately minimal. This document records what v1 is. Vocabulary is defined in [GLOSSARY.md](../GLOSSARY.md); hard-to-reverse decisions live in [docs/adr/](./adr/).

- **App ID**: `io.github.rxvt.Armarium`
- **Target**: a single user on Omarchy (Hyprland/Wayland, Arch). Run via `cargo run`; an Arch PKGBUILD may follow. No Flatpak (sandboxing gets in the way of future USB device access).

## Technology

- GTK4 + libadwaita via gtk4-rs, UI in Blueprint, custom widgets via GObject subclassing (no Relm4). See [ADR 0001](./adr/0001-gtk4-libadwaita-in-rust.md).
- Follows the system light/dark scheme through libadwaita; no custom palette.
- Cargo workspace:
  - **`core`**: Library, database, Import, metadata extraction, Cover extraction. No GTK widgets. Unit-tested.
  - **`app`**: the GTK UI on top of `core`.
- Crates: `rusqlite` (hand-written SQL, numbered migrations, no ORM), `zip` + `quick-xml` (EPUB OPF/cover parsing), `poppler-rs` (PDF metadata and page-1 render), `gio` (Trash), `image` (thumbnails).
- Requires `blueprint-compiler` (Arch `extra` repo) and a Rust toolchain via `rustup` (see [Toolchain](#toolchain)).

### Dependency policy

Vetted on crates.io in October 2026: each crate has a long history, tens of thousands to hundreds of millions of downloads, and recognised maintainers.

| Crate | Maintained by | Note |
|---|---|---|
| `gtk4`, `glib`, `gio` | gtk-rs project | |
| `libadwaita` | GNOME World/Rust | |
| `poppler-rs` | GNOME World/Rust (gtk-rs maintainers) | **Not** `poppler`, an older third-party crate with a confusingly similar name |
| `rusqlite` | rusqlite org | |
| `zip` | zip-rs/zip2 | Maintainer changed in 2024 and major versions move fast; pin it |
| `quick-xml` | tafia/quick-xml | |
| `image` | image-rs org | |

Rules:

- Spell crate names exactly; double-check any new crate's owners and repo before adding it.
- Commit `Cargo.lock`; review `cargo tree` when the dependency graph changes.
- Run `cargo deny` (advisories, licences, allowed sources: crates.io only) and `cargo audit` against RustSec.
- Turn off crate features we do not need; prefer system libraries from Arch's signed repos (GTK, poppler, SQLite) over vendored C sources.
- Any crate added later goes through the same check.

### Cooldown: nothing younger than 7 days

No crate version, toolchain or tool is used until it has been public for at least 7 days.

- **Cargo**: `.cargo/config.toml` sets `registry.global-min-publish-age = "7 days"` with `resolver.incompatible-publish-age = "deny"`, so the resolver skips young versions, including transitive ones. Versions already in `Cargo.lock` are kept. Requires Cargo 1.100+ (older Cargo ignores the key).
- **Lockfile check**: a stdlib-only Python pre-commit hook reads `Cargo.lock`, looks up each version's publish date on crates.io and blocks the commit if any is under 7 days old. Deliberate exceptions (e.g. an urgent security fix, admitted with `CARGO_RESOLVER_INCOMPATIBLE_PUBLISH_AGE=allow cargo update -p <crate>`) must be listed with a reason in `cooldown-exceptions.toml`; the hook flags entries once they are old enough to remove. Publish times come from the crates.io sparse index (`pubtime`, the same data Cargo uses); versions that pass are cached in `.git/cooldown-cache.json`. Unreachable index or non-crates.io sources block the commit. Lives in `.githooks/pre-commit` → `scripts/cooldown_check.py` (tests: `python3 -I scripts/test_cooldown_check.py`). Enable once per clone: `just hooks`.
- **Slower checks** (`cargo deny check`, `cargo audit`, clippy, tests) run from `just check` (policy in `deny.toml`), not the commit hook, since they need network access to the advisory database.
- **Tools**: `cargo-deny`, `cargo-audit` and `blueprint-compiler` come from Arch's `extra` repo, never `cargo install` (which the cooldown does not cover).

### Toolchain

- Managed by `rustup` (Arch `extra`), not mise or the Arch `rust` package. `rust-toolchain.toml` is the single source of truth; run `rustup toolchain install` in the repo to fetch it.
- Until Rust 1.100 is a week old: pinned to `nightly-2026-10-08` (1.101.0-nightly, includes stabilised `min-publish-age`; clippy, rustfmt, rust-src and rust-analyzer all available).
- From about 2026-11-19 (1.100 released 2026-11-12): switch `channel` to `"1.100.0"` and uninstall the nightly. Never use floating `stable`/`nightly`; bump to exact versions at least a week after release.
- Verify with `cargo -Zunstable-options config get registry.global-min-publish-age` (nightly) and the "as of 7 days ago" note when Cargo locks.
- Package installs (`pacman`, `rustup`) are run by the user, not by Claude.

## Scope

### In v1

- Import EPUB and PDF via file picker or drag-and-drop (files or folders, recursive).
- Cover grid view and list view, with a toggle.
- Live search over title, authors and series.
- Sorting (default: date added, newest first).
- Book details sidebar; Metadata editing in a dialog; manual Cover replacement.
- Open a Format in the external Reader.
- Delete Books or individual Formats (to Trash), including multi-select delete.

### Not in v1

- Kindle or any other Device support (see [Later](#later)).
- Formats beyond EPUB and PDF (CBZ, TXT, MOBI/AZW3, …). Format handling should be easy to extend.
- Built-in Reader.
- Online metadata lookup.
- Tags, ratings, reading status/progress.
- Bulk Metadata editing.
- Writing Metadata back into files.
- Detecting changes made to the Library folder outside the app.
- Multiple Libraries.

## Library

See [ADR 0002](./adr/0002-managed-library.md) and [ADR 0003](./adr/0003-database-is-metadata-source-of-truth.md).

- One Library, default `~/Books`, created silently on first Import. Location changeable in Preferences: pointing at a folder opens the `library.db` there or creates a new empty Library. The app never moves an existing Library.
- `library.db` (SQLite) lives inside the Library folder. Cover thumbnails are cached in `~/.cache` (rebuildable).
- Layout: `Author/Title (id)/Title - Author.ext`, plus `cover.jpg` per Book. Uses the first author only; filesystem-unsafe characters stripped; long names truncated. Folders and files are renamed when title or authors are edited.
- Files outside the app are never modified; Import copies, never moves.
- A Book whose Format file is gone is only flagged as missing when the user tries to open it.

## Books, Formats and Metadata

- A Book has one or more Formats, at most one per file type.
- Metadata fields: title; authors (multiple, each with an auto-derived, editable sort name such as "Herbert, Frank"); series and series index; publisher; published date; language; ISBN; description; date added.
- Embedded metadata is read once at Import; afterwards the database is the source of truth.
- Cover: the EPUB's declared cover image, else page 1 of the PDF. EPUB wins when both exist. No cover found → a generated title/author placeholder. User can replace it.

## Import

Runs in the background with a progress indicator; finishes with a summary toast (e.g. "12 added · 3 merged as formats · 2 skipped"), expandable to per-file details.

For each EPUB/PDF found:

1. Byte-identical to an existing Format → skip.
2. Title + author (case/punctuation-normalized) match an existing Book → add as a new Format; if that Book already has this file type → skip.
3. Otherwise → new Book.
4. Missing title → use the filename; missing author → "Unknown".
5. Corrupt/unreadable → skip, with the reason in the summary.

Merging is automatic, never prompted.

## UI

- **Empty state**: status page with an "Import books" button.
- **Header bar**: Import button (left), search entry (centre), grid/list toggle and primary menu with Preferences and About (right).
- **Grid**: virtualized cover grid; cover size adjustable with Ctrl+scroll / Ctrl+±, remembered across sessions.
- **List**: columns for Cover, Title, Authors, Series, Formats, Date added; sortable.
- **Selection**: single selection shows the details sidebar (Cover, Metadata, Formats with Open buttons, Edit). Ctrl/Shift multi-select is allowed for delete only.
- **Open**: double-click / Enter opens the EPUB if present, else the PDF, via the default Reader (`xdg-open`). Context menu: open a specific Format, show in file manager, delete.
- **Search**: filters as you type; case- and diacritic-insensitive ("bronte" finds "Brontë").
- **Preferences**: Library location only.

## Testing

- `core` is covered by automated tests that generate minimal EPUB/PDF fixtures in code; no real books committed.
- A few public-domain (Project Gutenberg) books, kept outside the repo, for manual testing.
- The UI is tested manually.

## Later

- **Kindle over USB**. The user's Kindle is a 2017 Paperwhite, which mounts as USB mass storage (not MTP). Kindles don't accept EPUB over USB, so Books will need converting to **AZW3** (not the deprecated MOBI). The likely route is shelling out to Calibre's `ebook-convert`. The converted file is not a Format ([ADR 0004](./adr/0004-device-copies-are-not-formats.md)); Books on the Device are matched by a filename tag ([ADR 0005](./adr/0005-device-books-matched-by-filename-tag.md)).
  - **Connecting**: udiskie mounts the Kindle; Armarium watches mounts (GIO volume monitor), recognises a Kindle by its `documents/` and `system/` folders, never mounts anything itself, and offers Eject (the Kindle only re-indexes after a safe eject). One Device at a time. A header-bar indicator shows its name, free space and Eject.
  - **Send**: EPUB present → convert to AZW3 with `ebook-convert --share-not-sync`, passing the current Metadata and Cover (personal-document mode keeps the embedded Cover; otherwise the Kindle overwrites it with a blank one from Amazon). PDF only → copied as-is, embedded title shown by the Kindle. Calibre is an optional runtime dependency; without it only EPUB Sends fail. Runs in the background like Import, with a summary toast; free space is checked before the batch starts. Sending a Book already on the Device replaces the copy and renames its `.sdr` so reading position and highlights survive.
  - **Remove**: deletes the tagged file and its `.sdr` (after a confirmation that warns highlights are lost). Untagged files are never touched. Deleting a Book from the Library leaves its Device copy alone.
  - **UI**: Send/Remove in the context menu and details sidebar while a Kindle is connected, multi-select allowed. A green tick bottom-right on the Cover (grid and list) and "On Kindle" in the sidebar, only while connected.
- Other Devices, tags/collections, online metadata, writing Metadata into files, folder watching, matching Omarchy theme colours.
