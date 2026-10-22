# Kindle over USB: Tasks

The work behind the Kindle design in [v1-plan.md](./v1-plan.md#later), split into slices that each end in something testable. Vocabulary is in [GLOSSARY.md](../GLOSSARY.md); see also [ADR 0004](./adr/0004-device-copies-are-not-formats.md), [ADR 0005](./adr/0005-device-books-matched-by-filename-tag.md) and [ADR 0007](./adr/0007-epub-to-azw3-port-of-calibre.md).

Already done: EPUB to AZW3 conversion, `armarium_core::azw3::convert` (try it with `just azw3 BOOK.epub OUT.azw3`).

```
1 ──► 2
│
├──► 3 ──► 4
│          │
└──────────┴──► 5
                     6 (after all)
```

## 1. `core`: recognise a Kindle and read which Books are on it

A new `core::device` module, unit-tested against temporary folders laid out like a Kindle.

- `Device::at(mount_root) -> Option<Device>`: a Kindle has `documents/` and `system/` folders.
- The filename tag lives in `naming`: build `documents/<Title> - <Author> [armarium-<id>].<ext>` (same sanitising and truncation as the Library) and parse a `BookId` back out of a name. Names without the tag yield nothing.
- `Device::books() -> HashMap<BookId, PathBuf>`: tagged files directly in `documents/`, skipping `.sdr` folders, temporary files and everything untagged.
- `Device::free_space()` via `gio::File::query_filesystem_info`.
- `Device::clean_up()`: deletes temporary files left by an interrupted Send.

**Done when** tests cover detection, a drive that isn't a Kindle, tag round-trips (odd titles, long names, Unicode), untagged and look-alike names being ignored, and leftovers being cleaned.

## 2. `app`: connection, ticks and Eject

- Watch mounts with `gio::VolumeMonitor`; on a mount, check `Device::at`; on unmount, drop it. One Device at a time: the first wins.
- For development, an `ARMARIUM_DEVICE=<folder>` override (dev builds only) treats a plain folder as a connected Device, plus a `just` task that creates a scratch one under `target/scratch`.
- Header-bar indicator: name, free space, Eject (`gio::Mount::eject_with_operation`; the Kindle only re-indexes after a safe eject), with errors as a toast.
- Green tick bottom-right on the Cover in the grid tile and on the list's Cover column, and "On Kindle" in the details sidebar. Shown only while connected, refreshed on connect, disconnect and after any Send or Remove.

**Done when** a hand-named `documents/Dune - Frank Herbert [armarium-42].azw3` in a scratch Device ticks Book 42, and unplugging the real Kindle clears the ticks.

## 3. `core`: Send

- `Device::send(library, book)`: with an EPUB, `azw3::convert` it using the Book's current Metadata and `cover.jpg`; with only a PDF, copy it as-is. Write to a temporary name in `documents/`, `fsync`, then rename to the tagged name.
- For an AZW3, also write the Cover as `system/thumbnails/thumbnail_<ASIN>_EBOK_portrait.jpg` (the ASIN is the book's UUID, EXTH 113), as Calibre's Kindle driver does, so the library keeps the Cover after the Kindle has been online and fails to fetch one from Amazon.
- Sending a Book already on the Device replaces its copy and renames its `.sdr` to match, so reading position and highlights survive.
- No cache: every Send converts again, so the copy always reflects the current Metadata and Cover.
- A batch API that converts first, checks the total size against free space and refuses the whole batch if it won't fit; otherwise each Book succeeds or fails on its own (an EPUB that won't convert is a failure, with the reason), in a report like `ImportReport`.

**Done when** tests cover a fresh Send of each Format, a replaced copy keeping its `.sdr`, an EPUB that fails to convert, a batch that doesn't fit, and a failure part-way through leaving no tagged partial file.

## 4. `app`: Send from the UI

- "Send to Kindle" in the context menu and the details sidebar, visible while a Device is connected; works on a multi-selection.
- Runs in the background like Import (`gio::spawn_blocking`), one Send at a time, with progress; Eject is disabled until it finishes.
- Summary toast ("5 sent · 1 failed") with expandable details, reusing the Import summary pattern.
- On activation, check again that the Device is still mounted and still a Kindle; if not, show a "Kindle disconnected" error toast and Send nothing.
- If the Device disappears part-way through a batch, stop: Books already Sent are successes, the rest fail with "Kindle disconnected" in the summary.

**Done when** Sending a few Books to the real Kindle shows ticks, and after Eject they appear in the Kindle's library with their Covers.

## 5. Remove

- `core`: `Device::remove(book_id)` deletes the tagged file and its `<name>.sdr`; never touches untagged files or anything outside `documents/`.
- `app`: "Remove from Kindle" in the context menu and sidebar for ticked Books, multi-select allowed, with a confirmation saying reading position and highlights for them will be lost.

**Done when** tests cover removing a file with and without an `.sdr`, and an untagged file with a similar name surviving.

## 6. Check on the real Kindle and update the docs

- On the 2017 Paperwhite: Sent AZW3s open with their Covers (also after the Kindle has been online), the Table of Contents and internal links work, and Sent PDFs open; Remove leaves no orphaned `.sdr`.
- Move Device support from "Later" into the plan's scope and add it to the README's Features.
