# Covers are found online in Apple Books, fetched with the system libsoup

Replacing a Book's Cover offers Cover Candidates from Apple Books alongside uploading an image file. This is the app's first network access. Apple Books has the best ebook artwork we know of (often 1600×2400 or larger), and its public iTunes Search API (`https://itunes.apple.com/search?media=ebook`) needs no key; the artwork URL's size segment is rewritten to fetch a small thumbnail for the picker and the full-size image for the chosen Cover, as in [lister](https://github.com/rxvt/lister). We fetch it with `soup3`, the gtk-rs bindings to the system libsoup3, because it runs asynchronously on the GLib main loop the app already uses and takes TLS and CA certificates from the system's glib-networking, so no crypto code is compiled into the app.

## Considered Options

- **`ureq` or `reqwest` with rustls**: pure Rust, but brings in `ring` and `rustls-webpki` (ISC) and usually `webpki-roots` (CDLA-Permissive-2.0), outside the licences `deny.toml` allows; `reqwest` also brings in tokio.
- **`ureq` with `native-tls`**: system OpenSSL and allowed licences, but blocking, so every request needs a worker thread.
- **Open Library or Google Books covers**: smaller or lower-resolution artwork for ebooks.

## Consequences

- Nothing is sent until the user opens the Cover picker; the search sends the Book's title and first author's surname to Apple.
- Searches are cached in memory for the run of the app, since Apple allows about 20 requests a minute.
- Apple can change or withdraw the API without notice; uploading an image file keeps working without it.
- Running the app now needs libsoup3 and glib-networking, both already present wherever GTK is.
