# Copies converted for a Device are not Formats

Sending an EPUB to the Kindle needs an AZW3. We convert at Send time, using the Book's current Metadata and Cover, and treat the result as a throwaway build product (cached at most), not as another Format in the Library. The v1 plan originally had it the other way round. A stored AZW3 would go stale on every Metadata or Cover edit, because the database is the source of truth ([ADR 0003](./0003-database-is-metadata-source-of-truth.md)), and it would also clutter the Formats list and Import duplicate checks.

## Consequences

- Sending an EPUB costs a conversion (via Calibre's `ebook-convert`, an optional runtime dependency) unless a cached copy is still current.
