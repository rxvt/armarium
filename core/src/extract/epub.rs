//! EPUB 2 and 3: Metadata from the OPF package document, Cover from the
//! manifest.

use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use zip::ZipArchive;

use super::{Extracted, non_empty, normalize_isbn};
use crate::model::{Author, Metadata};
use crate::xml::{self, Element};
use crate::{Error, Result, text};

/// Generous caps so a hostile archive cannot make us allocate without bound.
const MAX_XML_BYTES: u64 = 16 << 20;
const MAX_COVER_BYTES: u64 = 64 << 20;

pub fn extract(path: &Path) -> Result<Extracted> {
    let mut archive = ZipArchive::new(File::open(path)?)
        .map_err(|e| unreadable(format!("not a ZIP archive: {e}")))?;

    let container = xml::parse(&read_entry(
        &mut archive,
        "META-INF/container.xml",
        MAX_XML_BYTES,
    )?)
    .map_err(|e| unreadable(format!("bad container.xml: {e}")))?;
    let opf_path = rootfile_path(&container)
        .ok_or_else(|| unreadable("container.xml names no package document"))?;
    let package = xml::parse(&read_entry(&mut archive, &opf_path, MAX_XML_BYTES)?)
        .map_err(|e| unreadable(format!("bad package document: {e}")))?;
    let metadata_el = package
        .child("metadata")
        .ok_or_else(|| unreadable("package document has no metadata"))?;

    let metadata = read_metadata(metadata_el);
    let cover = cover_href(&package)
        .map(|href| resolve_href(&opf_path, &href))
        // A missing or oversized cover is not worth failing the Import over.
        .and_then(|entry| read_entry(&mut archive, &entry, MAX_COVER_BYTES).ok());

    Ok(Extracted { metadata, cover })
}

fn unreadable(reason: impl Into<String>) -> Error {
    Error::Unreadable(format!("EPUB: {}", reason.into()))
}

pub(crate) fn read_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let entry = archive
        .by_name(name)
        .map_err(|e| unreadable(format!("cannot read {name}: {e}")))?;
    if entry.size() > limit {
        return Err(unreadable(format!("{name} is too large")));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| unreadable(format!("cannot read {name}: {e}")))?;
    Ok(bytes)
}

pub(crate) fn rootfile_path(container: &Element) -> Option<String> {
    let rootfiles: Vec<&Element> = container
        .child("rootfiles")?
        .children_named("rootfile")
        .collect();
    rootfiles
        .iter()
        .find(|r| r.attr("media-type") == Some("application/oebps-package+xml"))
        .or(rootfiles.first())
        .and_then(|r| r.attr("full-path"))
        .map(str::to_owned)
}

fn read_metadata(metadata: &Element) -> Metadata {
    let refines = Refines(metadata);

    let titles: Vec<&Element> = metadata.children_named("title").collect();
    let title = titles
        .iter()
        .find(|t| refines.get(t, "title-type").as_deref() == Some("main"))
        .or(titles.first())
        .map(|t| t.text_content())
        .unwrap_or_default();

    let (series, series_index) = read_series(metadata, &refines);

    Metadata {
        title,
        authors: read_authors(metadata, &refines),
        series,
        series_index,
        publisher: first_text(metadata, "publisher"),
        published: first_text(metadata, "date"),
        language: first_text(metadata, "language"),
        isbn: metadata.children_named("identifier").find_map(|id| {
            let value = id.text_content();
            let schemed = id
                .attr("scheme")
                .is_some_and(|s| s.eq_ignore_ascii_case("isbn"));
            let urn = value
                .get(..9)
                .is_some_and(|p| p.eq_ignore_ascii_case("urn:isbn:"));
            // Bare numbers without a scheme are often other IDs; trust only 13-digit Bookland ones.
            let bookland = value.starts_with("978") || value.starts_with("979");
            if schemed || urn || bookland {
                normalize_isbn(&value)
            } else {
                None
            }
        }),
        description: metadata
            .child("description")
            .map(|d| strip_tags(&d.text_content()))
            .and_then(non_empty),
    }
}

fn read_authors(metadata: &Element, refines: &Refines) -> Vec<Author> {
    let creators: Vec<(&Element, Option<String>)> = metadata
        .children_named("creator")
        .map(|c| {
            (
                c,
                c.attr("role")
                    .map(str::to_owned)
                    .or_else(|| refines.get(c, "role")),
            )
        })
        .collect();
    let is_author = |role: &Option<String>| {
        role.as_deref()
            .is_none_or(|r| r.eq_ignore_ascii_case("aut"))
    };
    // If every creator has some other role (editor, translator, …), they are still better than nothing.
    let any_author = creators.iter().any(|(_, role)| is_author(role));

    creators
        .iter()
        .filter(|(_, role)| !any_author || is_author(role))
        .filter_map(|(c, _)| {
            let name = non_empty(c.text_content())?;
            let sort_name = c
                .attr("file-as")
                .and_then(non_empty)
                .or_else(|| refines.get(c, "file-as"))
                .unwrap_or_else(|| text::author_sort_name(&name));
            Some(Author { name, sort_name })
        })
        .collect()
}

/// Series from EPUB 3 `belongs-to-collection`, else Calibre's `calibre:series`.
fn read_series(metadata: &Element, refines: &Refines) -> (Option<String>, Option<f64>) {
    let epub3 = metadata.children_named("meta").find(|m| {
        m.attr("property") == Some("belongs-to-collection")
            && refines
                .get(m, "collection-type")
                .is_none_or(|t| t == "series")
    });
    if let Some(collection) = epub3
        && let Some(name) = non_empty(collection.text_content())
    {
        let index = refines
            .get(collection, "group-position")
            .and_then(|p| p.parse().ok());
        return (Some(name), index);
    }

    let calibre = |name: &str| {
        metadata
            .children_named("meta")
            .find(|m| m.attr("name") == Some(name))
            .and_then(|m| m.attr("content"))
            .and_then(non_empty)
    };
    match calibre("calibre:series") {
        Some(name) => (
            Some(name),
            calibre("calibre:series_index").and_then(|i| i.parse().ok()),
        ),
        None => (None, None),
    }
}

/// EPUB 3 `<meta refines="#id" property="…">` lookups.
struct Refines<'a>(&'a Element);

impl Refines<'_> {
    fn get(&self, target: &Element, property: &str) -> Option<String> {
        let id = target.attr("id")?;
        self.0
            .children_named("meta")
            .find(|m| {
                m.attr("property") == Some(property)
                    && m.attr("refines").and_then(|r| r.strip_prefix('#')) == Some(id)
            })
            .and_then(|m| non_empty(m.text_content()))
    }
}

fn first_text(metadata: &Element, name: &str) -> Option<String> {
    metadata
        .child(name)
        .and_then(|e| non_empty(e.text_content()))
}

/// Descriptions are often escaped HTML; keep only the text. Block-level tags
/// become spaces, inline ones vanish.
fn strip_tags(s: &str) -> String {
    const BLOCK: &[&str] = &[
        "p",
        "br",
        "div",
        "li",
        "ul",
        "ol",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "blockquote",
    ];
    let mut out = String::with_capacity(s.len());
    let mut tag: Option<String> = None;
    for c in s.chars() {
        match (&mut tag, c) {
            (None, '<') => tag = Some(String::new()),
            (Some(t), '>') => {
                let name = t
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next();
                if name.is_some_and(|n| BLOCK.contains(&n.to_ascii_lowercase().as_str())) {
                    out.push(' ');
                }
                tag = None;
            }
            (Some(t), c) => t.push(c),
            (None, c) => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The manifest href of the cover image: EPUB 3 `cover-image` property,
/// else the EPUB 2 `<meta name="cover">`, else an image item named like a cover.
pub(crate) fn cover_href(package: &Element) -> Option<String> {
    let items: Vec<&Element> = package.child("manifest")?.children_named("item").collect();
    let is_image = |item: &&&Element| {
        item.attr("media-type")
            .is_some_and(|t| t.starts_with("image/"))
    };
    let href = |item: &Element| item.attr("href").map(str::to_owned);

    if let Some(item) = items.iter().find(|i| {
        i.attr("properties")
            .is_some_and(|p| p.split_whitespace().any(|p| p == "cover-image"))
    }) {
        return href(item);
    }

    let meta_cover = package
        .child("metadata")?
        .children_named("meta")
        .find(|m| m.attr("name") == Some("cover"))
        .and_then(|m| m.attr("content"));
    if let Some(target) = meta_cover
        && let Some(item) = items
            .iter()
            .filter(is_image)
            // Some books put the href here instead of the item id.
            .find(|i| i.attr("id") == Some(target) || i.attr("href") == Some(target))
    {
        return href(item);
    }

    items
        .iter()
        .filter(is_image)
        .find(|i| {
            let named = |a| {
                i.attr(a)
                    .is_some_and(|v| v.to_ascii_lowercase().contains("cover"))
            };
            named("id") || named("href")
        })
        .and_then(|i| href(i))
}

/// Resolves a manifest href (relative to the OPF, percent-encoded) to a ZIP
/// entry name.
pub(crate) fn resolve_href(opf_path: &str, href: &str) -> String {
    let href = percent_decode(href.split('#').next().unwrap_or(href));
    let mut parts: Vec<&str> = opf_path.split('/').collect();
    parts.pop();
    for segment in href.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, EpubFixture};

    fn extract_fixture(epub: &EpubFixture) -> Result<Extracted> {
        let dir = fixtures::TempDir::new();
        let path = dir.path().join("book.epub");
        std::fs::write(&path, epub.build()).unwrap();
        extract(&path)
    }

    #[test]
    fn reads_epub2_metadata() {
        let epub = EpubFixture::new().metadata(
            r#"<dc:title>Wuthering Heights</dc:title>
               <dc:creator opf:role="aut" opf:file-as="Brontë, Emily">Emily Brontë</dc:creator>
               <dc:creator opf:role="edt">Some Editor</dc:creator>
               <dc:publisher>Thomas Cautley Newby</dc:publisher>
               <dc:date opf:event="publication">1847</dc:date>
               <dc:language>en</dc:language>
               <dc:identifier opf:scheme="ISBN">978-0-14-143955-6</dc:identifier>
               <dc:description>&lt;p&gt;A tale of &lt;i&gt;passion&lt;/i&gt;.&lt;/p&gt;</dc:description>
               <meta name="calibre:series" content="Classics"/>
               <meta name="calibre:series_index" content="3.5"/>"#,
        );

        assert_eq!(
            extract_fixture(&epub).unwrap().metadata,
            Metadata {
                title: "Wuthering Heights".into(),
                authors: vec![Author {
                    name: "Emily Brontë".into(),
                    sort_name: "Brontë, Emily".into()
                }],
                series: Some("Classics".into()),
                series_index: Some(3.5),
                publisher: Some("Thomas Cautley Newby".into()),
                published: Some("1847".into()),
                language: Some("en".into()),
                isbn: Some("9780141439556".into()),
                description: Some("A tale of passion.".into()),
            }
        );
    }

    #[test]
    fn reads_epub3_refines() {
        let epub = EpubFixture::new().metadata(
            r##"<dc:title id="sub">The Return of the King</dc:title>
                <dc:title id="main">The Lord of the Rings</dc:title>
                <meta refines="#sub" property="title-type">subtitle</meta>
                <meta refines="#main" property="title-type">main</meta>
                <dc:creator id="c1">J. R. R. Tolkien</dc:creator>
                <meta refines="#c1" property="role" scheme="marc:relators">aut</meta>
                <meta refines="#c1" property="file-as">Tolkien, J. R. R.</meta>
                <dc:creator id="c2">Alan Lee</dc:creator>
                <meta refines="#c2" property="role" scheme="marc:relators">ill</meta>
                <meta property="belongs-to-collection" id="s">The Lord of the Rings</meta>
                <meta refines="#s" property="collection-type">series</meta>
                <meta refines="#s" property="group-position">3</meta>
                <dc:identifier>urn:uuid:0b8e7a52-1234</dc:identifier>"##,
        );

        let metadata = extract_fixture(&epub).unwrap().metadata;
        assert_eq!(metadata.title, "The Lord of the Rings");
        assert_eq!(
            metadata.authors,
            vec![Author {
                name: "J. R. R. Tolkien".into(),
                sort_name: "Tolkien, J. R. R.".into()
            }]
        );
        assert_eq!(metadata.series.as_deref(), Some("The Lord of the Rings"));
        assert_eq!(metadata.series_index, Some(3.0));
        assert_eq!(metadata.isbn, None);
    }

    #[test]
    fn missing_fields_are_empty_and_sort_names_derived() {
        let epub = EpubFixture::new().metadata(
            "<dc:title>  </dc:title><dc:creator>Frank Herbert</dc:creator><dc:language></dc:language>",
        );
        let metadata = extract_fixture(&epub).unwrap().metadata;
        assert_eq!(metadata.title, "");
        assert_eq!(metadata.authors, vec![Author::new("Frank Herbert")]);
        assert_eq!(metadata.language, None);
    }

    #[test]
    fn non_author_creators_are_used_when_no_author_is_listed() {
        let epub = EpubFixture::new().metadata(
            r#"<dc:title>Anthology</dc:title><dc:creator opf:role="edt">Ed Itor</dc:creator>"#,
        );
        let names: Vec<_> = extract_fixture(&epub)
            .unwrap()
            .metadata
            .authors
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["Ed Itor"]);
    }

    #[test]
    fn finds_epub3_cover_image() {
        let epub = EpubFixture::new()
            .opf_path("OEBPS/content.opf")
            .manifest(r#"<item id="x" href="images/my%20cover.jpg" media-type="image/jpeg" properties="cover-image"/>"#)
            .file("OEBPS/images/my cover.jpg", b"JPEG!");
        assert_eq!(
            extract_fixture(&epub).unwrap().cover.as_deref(),
            Some(&b"JPEG!"[..])
        );
    }

    #[test]
    fn finds_epub2_meta_cover_with_relative_path() {
        let epub = EpubFixture::new()
            .opf_path("OPS/package/content.opf")
            .metadata(r#"<dc:title>T</dc:title><meta name="cover" content="cov"/>"#)
            .manifest(
                r#"<item id="other" href="../img/other.png" media-type="image/png"/>
                   <item id="cov" href="../img/front.png" media-type="image/png"/>"#,
            )
            .file("OPS/img/front.png", b"PNG!");
        assert_eq!(
            extract_fixture(&epub).unwrap().cover.as_deref(),
            Some(&b"PNG!"[..])
        );
    }

    #[test]
    fn falls_back_to_image_named_cover_and_tolerates_missing_cover_file() {
        let named = EpubFixture::new()
            .manifest(r#"<item id="i1" href="a.png" media-type="image/png"/><item id="i2" href="Cover.jpeg" media-type="image/jpeg"/>"#)
            .file("Cover.jpeg", b"C");
        assert_eq!(
            extract_fixture(&named).unwrap().cover.as_deref(),
            Some(&b"C"[..])
        );

        let dangling = EpubFixture::new().manifest(
            r#"<item id="c" href="gone.jpg" media-type="image/jpeg" properties="cover-image"/>"#,
        );
        assert_eq!(extract_fixture(&dangling).unwrap().cover, None);

        assert_eq!(extract_fixture(&EpubFixture::new()).unwrap().cover, None);
    }

    #[test]
    fn rejects_files_that_are_not_epubs() {
        let dir = fixtures::TempDir::new();
        let path = dir.path().join("fake.epub");
        std::fs::write(&path, b"not a zip").unwrap();
        assert!(matches!(extract(&path), Err(Error::Unreadable(_))));

        let no_container = EpubFixture::new().without_container();
        let err = extract_fixture(&no_container).unwrap_err();
        assert!(err.to_string().contains("container.xml"), "{err}");
    }

    #[test]
    fn strips_html_from_descriptions() {
        assert_eq!(
            strip_tags("<p>One <b>bold</b>.</p><p>Two<br/>three</p>"),
            "One bold. Two three"
        );
    }

    #[test]
    fn resolves_hrefs_against_opf_directory() {
        assert_eq!(resolve_href("content.opf", "a/b.jpg"), "a/b.jpg");
        assert_eq!(
            resolve_href("OEBPS/content.opf", "./img/a%26b.jpg"),
            "OEBPS/img/a&b.jpg"
        );
        assert_eq!(resolve_href("A/B/c.opf", "../../x.png#frag"), "x.png");
    }
}
