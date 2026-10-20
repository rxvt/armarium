# Books on a Device are matched by a tag in the filename

Armarium works out which Books are on the Kindle only while it is connected, by reading `documents/`, so that nothing is remembered that might be out of date. Each Sent file is named `<Title> - <Author> [armarium-<book id>].<ext>`, directly in `documents/`. The tag is the only thing matched, and files without it (store purchases, other sideloads) are ignored and never touched. We rejected an Armarium subfolder because some Kindles reportedly fail to show books in subfolders, and we rejected a sent-books record in `library.db` or a manifest on the device because either can drift from what is really there.

## Consequences

- Changing the tag format orphans every book already on the Device.
- Book ids are only unique within one Library, so switching Libraries in Preferences can produce false ticks.
