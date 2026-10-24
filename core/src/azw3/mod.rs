//! Converting an EPUB to AZW3 (KF8), the format stock Kindles read.
//!
//! This is a port of calibre's AZW3 output (by Kovid Goyal, GPL-3.0). The
//! files that carry over calibre's code say so at the top and keep its
//! copyright; see also NOTICE at the root of the repository. The EPUB side
//! is Armarium's own and much simpler than calibre's conversion pipeline:
//! stylesheets go into the book as they are rather than being flattened.

mod css;
mod dom;
mod entities;
mod exth;
mod index;
mod langcodes;
mod mobi;
mod oeb;
mod palmdoc;
mod resources;
mod skeleton;
mod tbs;
mod toc;
mod utils;
mod writer;

use std::path::Path;

use crate::Result;
use crate::model::Metadata;

/// Converts the EPUB at `epub` to AZW3, describing the book with `metadata`
/// and, if given, `cover` (an encoded image) rather than what the EPUB says.
///
/// The book is an `EBOK` with `asin` as its ASIN, or else the EPUB's UUID
/// (random if it has none), as Calibre writes it by default: a Kindle only
/// shows a library thumbnail for books with an ASIN, named after it. The
/// Kindle fills that thumbnail with a placeholder rather than the embedded
/// Cover, so Send has to write it too.
pub fn convert(
    epub: &Path,
    metadata: &Metadata,
    cover: Option<&[u8]>,
    asin: Option<&str>,
) -> Result<Vec<u8>> {
    let mut oeb = oeb::Oeb::read(epub)?;
    if let Some(asin) = asin {
        oeb.uuid = Some(asin.to_owned());
    }
    toc::add_inline_toc(&mut oeb);
    let resources = resources::Resources::new(&oeb, cover);
    let kf8 = writer::Writer::new(&mut oeb, &resources, true).write()?;
    Ok(mobi::write(kf8, resources, &oeb, metadata, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{EpubFixture, TempDir};
    use crate::model::Author;
    use utils::decint;

    fn png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(60, 90, image::Rgb([200, 40, 40]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn epub() -> Vec<u8> {
        let chapter = |n: usize, body: &str| {
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>{n}</title>
<link rel="stylesheet" type="text/css" href="../style.css"/></head>
<body><h1 id="c{n}">Chapter {n}</h1>{body}</body></html>"#
            )
        };
        let long: String = (0..400)
            .map(|i| format!("<p>Paragraph {i}&nbsp;of a fairly long chapter, with “text”.</p>\n"))
            .collect();
        EpubFixture::new()
            .opf_path("OEBPS/content.opf")
            .metadata(
                r#"<dc:title>Ignored</dc:title><dc:identifier>urn:uuid:0d5c1a3e-0000-4000-8000-000000000001</dc:identifier>
                <meta name="cover" content="cover"/>"#,
            )
            .manifest(
                r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
                <item id="c1" href="Text/one.xhtml" media-type="application/xhtml+xml"/>
                <item id="c2" href="Text/two.xhtml" media-type="application/xhtml+xml"/>
                <item id="css" href="style.css" media-type="text/css"/>
                <item id="cover" href="Images/cover.png" media-type="image/png"/>"#,
            )
            .spine(r#"<itemref idref="c1"/><itemref idref="c2"/>"#)
            .file(
                "OEBPS/toc.ncx",
                br##"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
                <navPoint><navLabel><text>One</text></navLabel><content src="Text/one.xhtml"/></navPoint>
                <navPoint><navLabel><text>Two</text></navLabel><content src="Text/two.xhtml#c2"/></navPoint>
                </navMap></ncx>"##,
            )
            .file("OEBPS/Text/one.xhtml", chapter(1, &long).as_bytes())
            .file(
                "OEBPS/Text/two.xhtml",
                chapter(2, r#"<p><a href="one.xhtml#c1">Back</a><img src="../Images/cover.png"/></p>"#).as_bytes(),
            )
            .file("OEBPS/style.css", b"h1 { background: url(Images/cover.png) }")
            .file("OEBPS/Images/cover.png", &png())
            .build()
    }

    /// The records of a Palm database.
    fn records(db: &[u8]) -> Vec<&[u8]> {
        let n = u16::from_be_bytes([db[76], db[77]]) as usize;
        let offset =
            |i: usize| u32::from_be_bytes(db[78 + 8 * i..82 + 8 * i].try_into().unwrap()) as usize;
        (0..n)
            .map(|i| &db[offset(i)..if i + 1 < n { offset(i + 1) } else { db.len() }])
            .collect()
    }

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_be_bytes(b[i..i + 4].try_into().unwrap())
    }

    /// The decompressed text, read as a Kindle would.
    fn text(records: &[&[u8]]) -> Vec<u8> {
        let r0 = records[0];
        let count = u16::from_be_bytes([r0[8], r0[9]]) as usize;
        let flags = u32_at(r0, 240);
        let mut text = Vec::new();
        for rec in &records[1..=count] {
            let mut rec = &rec[..];
            if flags & 0b10 != 0 {
                let (size, _) = decint(rec, false);
                rec = &rec[..rec.len() - size as usize];
            }
            let overlap = (rec[rec.len() - 1] & 0b11) as usize + 1;
            rec = &rec[..rec.len() - overlap];
            text.extend(palmdoc::decompress(rec));
        }
        assert_eq!(text.len(), u32_at(r0, 4) as usize);
        text
    }

    #[test]
    fn converts_an_epub_that_reads_back() {
        let dir = TempDir::new();
        let path = dir.path().join("book.epub");
        std::fs::write(&path, epub()).unwrap();
        let metadata = Metadata {
            title: "Dune".into(),
            authors: vec![Author::new("Frank Herbert")],
            language: Some("en".into()),
            ..Metadata::default()
        };

        let db = convert(&path, &metadata, None, None).unwrap();
        assert_eq!(&db[60..68], b"BOOKMOBI");
        let records = records(&db);
        let r0 = records[0];
        assert_eq!(&r0[16..20], b"MOBI");
        let title_offset = u32_at(r0, 84) as usize;
        assert_eq!(&r0[title_offset..title_offset + 4], b"Dune");

        let text = String::from_utf8(text(&records)).unwrap();
        assert!(text.contains("Paragraph 399\u{a0}of a fairly long chapter"));
        assert!(text.contains(
            r#"<link rel="stylesheet" type="text/css" href="kindle:flow:0001?mime=text/css"/>"#
        ));
        assert!(text.contains("h1 { background: url(kindle:embed:0001?mime=image/png) }"));
        assert!(text.contains(r#"<img src="kindle:embed:0001?mime=image/png"/>"#));
        // "Back" points at the start of the first chunk; its placeholder was
        // the first link's, ending in 1.
        assert!(text.contains(r#"<a href="kindle:pos:fid:0000:off:0000000000""#));
        assert!(!text.contains("off:0000000001"), "link placeholder left");
        assert!(text.contains("Table of Contents"));

        // The NCX, chunk and skeleton indices and the first resource are where
        // the header says.
        for (field, magic) in [
            (244, &b"INDX"[..]),
            (248, b"INDX"),
            (252, b"INDX"),
            (192, b"FDST"),
        ] {
            assert_eq!(&records[u32_at(r0, field) as usize][..4], magic);
        }
        let first_resource = records[u32_at(r0, 108) as usize];
        assert_eq!(
            image::guess_format(first_resource).unwrap(),
            image::ImageFormat::Png
        );
        assert_eq!(&records.last().unwrap()[..], b"\xe9\x8e\r\n");
    }

    #[test]
    fn the_library_cover_replaces_the_books() {
        let dir = TempDir::new();
        let path = dir.path().join("book.epub");
        std::fs::write(&path, epub()).unwrap();
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(30, 40))
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();

        let db = convert(&path, &Metadata::default(), Some(&jpeg), None).unwrap();
        let records = records(&db);
        let first_resource = records[u32_at(records[0], 108) as usize];
        assert_eq!(
            image::guess_format(first_resource).unwrap(),
            image::ImageFormat::Jpeg
        );
        let text = String::from_utf8(text(&records)).unwrap();
        assert!(text.contains("kindle:embed:0001?mime=image/jpeg"));
        assert!(records[0].windows(4).any(|w| w == b"EBOK"));
    }

    #[test]
    fn images_declared_as_image_jpg_are_embedded() {
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(30, 40))
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let epub = EpubFixture::new()
            .manifest(
                r#"<item id="c1" href="one.xhtml" media-type="application/xhtml+xml"/>
                <item id="pic" href="pic.jpg" media-type="image/jpg"/>"#,
            )
            .spine(r#"<itemref idref="c1"/>"#)
            .file(
                "one.xhtml",
                br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>1</title></head>
<body><p><img src="pic.jpg"/></p></body></html>"#,
            )
            .file("pic.jpg", &jpeg)
            .build();
        let dir = TempDir::new();
        let path = dir.path().join("book.epub");
        std::fs::write(&path, epub).unwrap();

        let db = convert(&path, &Metadata::default(), None, None).unwrap();
        let text = String::from_utf8(text(&records(&db))).unwrap();
        assert!(text.contains(r#"<img src="kindle:embed:0001?mime=image/jpeg"/>"#));
    }

    #[test]
    fn the_asin_is_the_books_uuid_unless_given() {
        let dir = TempDir::new();
        let path = dir.path().join("book.epub");
        std::fs::write(&path, epub()).unwrap();
        let has = |db: &[u8], s: &str| records(db)[0].windows(s.len()).any(|w| w == s.as_bytes());

        let db = convert(&path, &Metadata::default(), None, None).unwrap();
        assert!(has(&db, "0d5c1a3e-0000-4000-8000-000000000001"));

        let db = convert(&path, &Metadata::default(), None, Some("given-asin")).unwrap();
        assert!(has(&db, "given-asin"));
        assert!(!has(&db, "0d5c1a3e-0000-4000-8000-000000000001"));
    }

    #[test]
    fn rejects_files_that_are_not_epubs() {
        let dir = TempDir::new();
        let path = dir.path().join("book.epub");
        std::fs::write(&path, b"not a zip").unwrap();
        assert!(matches!(
            convert(&path, &Metadata::default(), None, None),
            Err(crate::Error::Unreadable(_))
        ));
    }
}
