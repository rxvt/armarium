# Books on a Device are matched by a tag in the filename

Armarium works out which Books are on a Device only while it is connected, by reading its `Books/` folder, so that nothing is remembered that might be out of date. Each Sent file is named `Books/<Author>/<Title> [armarium-<book id>].<ext>`. Only the tag is matched; the folder and the rest of the name are for browsing in KOReader and may go stale after Metadata edits. Files without the tag, and anything outside `Books/`, are ignored and never touched. We rejected a sent-books record in `library.db` and a manifest on the device, because either can drift from what is really there.

## Consequences

- Changing the tag format orphans every book already on a Device.
- Book ids are only unique within one Library, so switching Libraries in Preferences can produce false ticks.
- KOReader ties reading position to the file's path (and its cover cache never notices a replaced file), so Armarium never renames or replaces a Sent file. Device filenames can go stale after Metadata edits.
