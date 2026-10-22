# EPUBs are converted to AZW3 by a Rust port of Calibre's writer

Devices are stock Kindles again, which don't read EPUB, so Send converts a Book's EPUB to AZW3 (KF8). This reverses [ADR 0006](./0006-koreader-devices-open-formats-only.md) and reinstates [ADR 0004](./0004-device-copies-are-not-formats.md) and [ADR 0005](./0005-device-books-matched-by-filename-tag.md) as first written. The original plan was to shell out to Calibre's `ebook-convert`, but we don't want Calibre as a dependency, so `core::azw3` is a port of Calibre's AZW3 output instead: the KF8 writer (`ebooks/mobi/writer8`), PalmDoc compression, resources and language codes, from Calibre 9.16 (commit `0b8c0c2`), by Kovid Goyal. The EPUB side is our own and much simpler than Calibre's conversion pipeline.

## Consequences

- Calibre is GPL-3.0 with no "or later", so the ported files are `GPL-3.0-only`, keep Kovid Goyal's copyright and name the Calibre file they came from. Armarium as a whole can only be distributed under GPLv3. [NOTICE](../../NOTICE) lists the ported files.
- Unlike Calibre, conversion does not flatten CSS, detect chapters, split files on page breaks or repair broken markup. An EPUB whose XHTML is not well-formed fails to convert, with the reason in the Send summary.
- Behaviour should track Calibre's: when a converted book misbehaves on the Kindle, compare with `ebook-convert` and Calibre's MOBI inspector (`calibre-debug -m`) first.
