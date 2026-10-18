# The database, not the files, is the source of truth for Metadata

Embedded metadata (EPUB OPF, PDF info) is read once at Import; after that, edits live only in `library.db` and the original files are left byte-for-byte untouched. This keeps Import/edit simple and never risks corrupting a user's file. Writing Metadata back into Formats may later be added as an explicit action, but code should never assume a Format's embedded metadata matches the Library.
