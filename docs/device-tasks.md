# Devices over USB: Tasks

The work behind the Device design in [v1-plan.md](./v1-plan.md#later), split into slices that each end in something testable. Vocabulary is in [GLOSSARY.md](../GLOSSARY.md); see also [ADR 0005](./adr/0005-device-books-matched-by-filename-tag.md) and [ADR 0006](./adr/0006-koreader-devices-open-formats-only.md).

```
1 ──► 2
│
├──► 3 ──► 4
│          │
└──────────┴──► 5
                     6 (after all)
```

## 1. `core`: recognise a Device and read which Books are on it

A new `core::device` module, unit-tested against temporary folders laid out like each platform.

- `Device::at(mount_root) -> Option<Device>`: KOReader is installed at `koreader/` (Kindle), `.adds/koreader/` (Kobo) or `applications/koreader/` (PocketBook), each holding `koreader.sh`. The platform gives the Device its display name.
- The filename tag lives in `naming`: build `Books/<Author>/<Title> [armarium-<id>].<ext>` (same sanitising and truncation as the Library) and parse a `BookId` back out of a name. Names without the tag yield nothing.
- `Device::books() -> HashMap<BookId, PathBuf>`: tagged files anywhere under `Books/`, skipping `.sdr` folders, temporary files and everything untagged.
- `Device::free_space()` via `gio::File::query_filesystem_info`.
- `Device::clean_up()`: deletes temporary files left by an interrupted Send.

**Done when** tests cover each platform's detection, a drive with no KOReader, tag round-trips (odd titles, long names, Unicode), untagged and look-alike names being ignored, and leftovers being cleaned.

## 2. `app`: connection, ticks and Eject

- Watch mounts with `gio::VolumeMonitor`; on a mount, check `Device::at`; on unmount, drop it. One Device at a time: the first wins.
- For development, an `ARMARIUM_DEVICE=<folder>` override (dev builds only) treats a plain folder as a connected Device, plus a `just` task that creates a scratch one under `target/scratch`.
- Header-bar indicator: platform name, free space, Eject (`gio::Mount::eject_with_operation`), with errors as a toast.
- Green tick bottom-right on the Cover in the grid tile and on the list's Cover column, and "On Kindle" (platform name) in the details sidebar. Shown only while connected, refreshed on connect, disconnect and after any Send or Remove.

**Done when** a hand-named `Books/Frank Herbert/Dune [armarium-42].epub` in a scratch Device ticks Book 42, and unplugging the real Kindle clears the ticks.

## 3. `core`: Send

- `Device::send(library, book)`: picks the EPUB if there is one, otherwise the PDF; copies it to a temporary name in the author folder, `fsync`s, then renames it to the tagged name.
- A batch API that checks total size against free space first and refuses the whole batch if it won't fit; otherwise each Book succeeds or fails on its own, with the reason in a report like `ImportReport`. Books already on the Device are skipped.

**Done when** tests cover a fresh Send, Format choice, a Book already on the Device, a batch that doesn't fit, and a failure part-way through leaving no tagged partial file.

## 4. `app`: Send from the UI

- "Send to Kindle" in the context menu and the details sidebar, visible while a Device is connected and the Book isn't already on it; works on a multi-selection.
- Runs in the background like Import (`gio::spawn_blocking`), one Send at a time, with progress; Eject is disabled until it finishes.
- Summary toast ("5 sent · 1 failed") with expandable details, reusing the Import summary pattern.

**Done when** Sending a few Books to the real Kindle shows ticks, and after Eject they open in KOReader.

## 5. Remove

- `core`: `Device::remove(book_id)` deletes the tagged file and its `<name>.sdr`, then the author folder if empty; never touches untagged files, `Books/` itself or `koreader/`.
- `app`: "Remove from Kindle" in the context menu and sidebar for ticked Books, multi-select allowed, with a confirmation saying KOReader's reading position and highlights for them will be lost.

**Done when** tests cover removing a file with and without an `.sdr`, empty and non-empty author folders, and an untagged file with a similar name surviving.

## 6. Check on the real Kindle and update the docs

- On the jailbroken Paperwhite: Sent EPUBs and PDFs open in KOReader with their Covers; the stock Kindle library ignores `Books/`; Remove leaves no orphaned sidecar.
- Move Device support from "Later" into the plan's scope and add it to the README's Features.
