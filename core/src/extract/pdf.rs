//! PDF: Metadata from the document info dictionary, Cover rendered from
//! page 1. Uses the system poppler-glib.

use std::io::Cursor;
use std::path::Path;

use image::{ImageFormat, RgbImage};
use poppler::Document;

use super::{Extracted, non_empty};
use crate::model::{Author, Metadata};
use crate::{Error, Result};

/// Longest side of the rendered page-1 Cover, in pixels.
const COVER_MAX_SIDE: f64 = 1200.0;

pub fn extract(path: &Path) -> Result<Extracted> {
    let uri = file_uri(&std::path::absolute(path)?);
    let doc = Document::from_file(&uri, None).map_err(|e| {
        if e.matches(poppler::Error::Encrypted) {
            unreadable("password-protected")
        } else {
            unreadable(e.message())
        }
    })?;

    let metadata = Metadata {
        title: doc.title().map(|t| clean_title(&t)).unwrap_or_default(),
        authors: doc.author().map(|a| split_authors(&a)).unwrap_or_default(),
        description: doc.subject().and_then(non_empty),
        ..Default::default()
    };
    // A page that will not render is not worth failing the Import over.
    let cover = render_first_page(&doc).ok();

    Ok(Extracted { metadata, cover })
}

fn unreadable(reason: impl std::fmt::Display) -> Error {
    Error::Unreadable(format!("PDF: {reason}"))
}

/// Word and similar tools prefix the title with the program name.
fn clean_title(title: &str) -> String {
    let title = title.trim();
    for prefix in ["Microsoft Word - ", "Microsoft PowerPoint - "] {
        if let Some(rest) = title.strip_prefix(prefix) {
            return rest.trim().to_owned();
        }
    }
    title.to_owned()
}

/// The Author entry is one string; multiple authors are usually separated
/// by ";", "&" or " and ". Commas are left alone since "Herbert, Frank" is
/// one name.
fn split_authors(raw: &str) -> Vec<Author> {
    raw.split([';', '&'])
        .flat_map(|part| part.split(" and "))
        .filter_map(non_empty)
        .map(Author::new)
        .collect()
}

/// Renders page 1 on white and returns it PNG-encoded.
fn render_first_page(doc: &Document) -> std::result::Result<Vec<u8>, String> {
    let page = doc.page(0).ok_or("no pages")?;
    let (width, height) = page.size();
    if !(width > 0.0 && height > 0.0) {
        return Err("empty page".into());
    }
    let scale = COVER_MAX_SIDE / width.max(height);
    let (w, h) = (
        (width * scale).round() as i32,
        (height * scale).round() as i32,
    );

    let mut surface = cairo::ImageSurface::create(cairo::Format::Rgb24, w.max(1), h.max(1))
        .map_err(|e| e.to_string())?;
    {
        let cx = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        cx.set_source_rgb(1.0, 1.0, 1.0);
        cx.paint().map_err(|e| e.to_string())?;
        cx.scale(scale, scale);
        page.render(&cx);
    }
    surface.flush();

    let (w, h, stride) = (
        surface.width() as u32,
        surface.height() as u32,
        surface.stride() as usize,
    );
    let data = surface.data().map_err(|e| e.to_string())?;
    // Rgb24 stores each pixel as a native-endian u32: 0x00RRGGBB.
    let mut rgb = RgbImage::new(w, h);
    for (y, row) in data.chunks(stride).take(h as usize).enumerate() {
        for x in 0..w as usize {
            let px = u32::from_ne_bytes(row[x * 4..x * 4 + 4].try_into().unwrap());
            rgb.put_pixel(
                x as u32,
                y as u32,
                image::Rgb([(px >> 16) as u8, (px >> 8) as u8, px as u8]),
            );
        }
    }

    let mut png = Vec::new();
    rgb.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png)
}

/// `file://` URI for an absolute path, percent-encoding everything but
/// unreserved characters and "/".
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{PdfFixture, TempDir};

    fn extract_fixture(pdf: &PdfFixture, file_name: &str) -> Result<Extracted> {
        let dir = TempDir::new();
        let path = dir.path().join(file_name);
        std::fs::write(&path, pdf.build()).unwrap();
        extract(&path)
    }

    #[test]
    fn reads_info_dictionary() {
        let pdf = PdfFixture::new()
            .info("Title", "Microsoft Word - Dune")
            .info(
                "Author",
                "Frank Herbert; Brian Herbert and Kevin J. Anderson",
            )
            .info("Subject", "A desert planet.");
        let metadata = extract_fixture(&pdf, "dune.pdf").unwrap().metadata;
        assert_eq!(metadata.title, "Dune");
        let names: Vec<_> = metadata.authors.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            ["Frank Herbert", "Brian Herbert", "Kevin J. Anderson"]
        );
        assert_eq!(metadata.authors[0].sort_name, "Herbert, Frank");
        assert_eq!(metadata.description.as_deref(), Some("A desert planet."));
    }

    #[test]
    fn decodes_unicode_strings_and_tolerates_missing_info() {
        let pdf = PdfFixture::new().info_utf16("Author", "Charlotte Brontë");
        let metadata = extract_fixture(&pdf, "jane eyre (1).pdf").unwrap().metadata;
        assert_eq!(metadata.title, "");
        assert_eq!(metadata.authors, vec![Author::new("Charlotte Brontë")]);
    }

    #[test]
    fn renders_page_one_as_cover() {
        let cover = extract_fixture(&PdfFixture::new(), "c.pdf")
            .unwrap()
            .cover
            .unwrap();
        let img = image::load_from_memory_with_format(&cover, ImageFormat::Png)
            .unwrap()
            .to_rgb8();
        // The fixture page is 300x600 pt with its left half filled black.
        assert_eq!((img.width(), img.height()), (600, 1200));
        assert_eq!(img.get_pixel(100, 600).0, [0, 0, 0]);
        assert_eq!(img.get_pixel(500, 600).0, [255, 255, 255]);
    }

    #[test]
    fn rejects_files_that_are_not_pdfs() {
        let dir = TempDir::new();
        let path = dir.path().join("fake.pdf");
        std::fs::write(&path, b"not a pdf").unwrap();
        let err = extract(&path).unwrap_err();
        assert!(matches!(err, Error::Unreadable(_)), "{err}");
    }

    #[test]
    fn encodes_file_uris() {
        assert_eq!(
            file_uri(Path::new("/a b/Brontë#1.pdf")),
            "file:///a%20b/Bront%C3%AB%231.pdf"
        );
    }
}
