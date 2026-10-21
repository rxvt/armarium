# Devices run KOReader, and Books are Sent as they are

Devices are e-readers running KOReader: jailbroken Kindles, Kobos and PocketBooks. They are detected by a KOReader install on the mounted drive (`koreader/`, `.adds/koreader/` or `applications/koreader/`). Send copies one of a Book's Formats unchanged (the EPUB if there is one, otherwise the PDF), into `Books/` at the drive root. This replaces the earlier plan to convert EPUBs to AZW3 for a stock Kindle ([ADR 0004](./0004-device-copies-are-not-formats.md)). That plan would have needed Calibre as a dependency and Amazon's reverse-engineered formats; KOReader reads EPUB and PDF directly.

## Consequences

- Stock Kindles, and any e-reader without KOReader, are not supported.
- Any Format the Library gains later can be Sent with no Device-side changes.
