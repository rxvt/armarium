# Armarium

A personal desktop ebook library for DRM-free books: it collects book files into one managed place, describes them, and lets you browse them as a cover grid or a list.

## Language

**Library**:
The single collection of Books the app owns, including the folder that holds their files.
_Avoid_: Collection, shelf, catalog

**Book**:
One work (e.g. *Dune*) with its own metadata and cover, regardless of how many files represent it.
_Avoid_: Title, item, ebook, file

**Format**:
One file belonging to a Book in a specific file type (EPUB, PDF). A Book has one or more Formats, at most one per file type.
_Avoid_: File, edition, copy

**Metadata**:
The descriptive facts about a Book (title, authors, series, etc.) as recorded by the Library, which may differ from what is embedded inside its Formats.
_Avoid_: Tags, info, properties

**Series**:
A named sequence that a Book may belong to, at a Number in Series. A Book belongs to at most one Series.
_Avoid_: Collection, saga, cycle

**Cover**:
The single image representing a Book in the Library.
_Avoid_: Thumbnail, artwork

**Cover Candidate**:
An image offered to the user as a possible Cover for a Book, either from a file they choose or found online. It becomes the Cover only when chosen.
_Avoid_: Suggestion, result, thumbnail

**Import**:
Bringing a file from outside into the Library, either as a new Book or as a new Format of an existing Book.
_Avoid_: Add, upload, ingest

**Merge**:
Combining two or more Books that are the same work into one Book, which keeps one of their Metadata and Covers and gains all of their Formats. The other Books cease to exist; no file is lost except a duplicate Format, which goes to the Trash.
_Avoid_: Combine, group, join, link

**Reader**:
The external application the user's system opens a Format with.
_Avoid_: Viewer

**Device**:
An external e-reader, connected over USB, that the Library can put Books on. The user's Kindle is the only one so far.
_Avoid_: Kindle (when meaning Devices in general), reader, e-reader

**Send**:
Putting one of a Book's Formats onto a connected Device, converted if the Device can't read it as it is. A Device holds at most one copy of a Book. The Book stays in the Library.
_Avoid_: Push, sync, transfer, upload, export

**Remove**:
Taking a Book that was Sent off a Device. The Library is not touched. Different from deleting a Book, which moves its files to the Trash.
_Avoid_: Delete (for Devices), unsync, eject
