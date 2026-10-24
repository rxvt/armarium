# Books split at Import are Merged by hand, not matched more cleverly

Import adds a file to an existing Book only when the normalized title and first author match exactly; anything else becomes a new Book, and the user joins Books that are the same work with Merge. The splits seen in practice were PDFs with no embedded Metadata, whose title fell back to a Calibre-style filename (`Automate the Boring Stuff with Python_ Pra - Al Sweigart`, author Unknown). We chose not to make matching smarter because a wrong automatic match silently files one work's Format under another Book, which is worse than a visible duplicate. Smarter matching also could not fix Books already split.

## Considered Options

- **Split `Title - Author` filenames into title and author before matching.** Fixes only some cases: Calibre truncates long titles, so they still don't match, and the dash is ambiguous in titles that contain one.
- **Fuzzy matching (prefix, edit distance, ISBN).** Same risk of false merges, harder to explain, and ISBNs are rarely present in the files that split.

## Consequences

- Import stays predictable: an exact match or a new Book, never a guess.
- Merging Books orphans the merged-away Books' copies on a Device, as deleting does ([ADR 0005](./0005-device-books-matched-by-filename-tag.md)); Sending the kept Book again restores its tick.
- Smarter matching can still be added later, but only as a suggestion the user confirms, not as an automatic merge.
