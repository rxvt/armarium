# Managed Library rather than indexing folders in place

Importing copies files into a Library folder the app owns (default `~/Books`), laid out as `Author/Title (id)/Title - Author.ext` and renamed when Metadata changes, with `library.db` (SQLite) stored inside it. We chose this over indexing the user's existing folders because a Book with several Formats, its Cover, and future device-converted Formats need one stable home, and in-place indexes break whenever files are moved outside the app. The Library folder is self-contained: copying it is a full backup.

## Consequences

- Files edited, moved or deleted inside the Library folder outside the app are not detected in v1; a Book is only flagged as missing when opened.
