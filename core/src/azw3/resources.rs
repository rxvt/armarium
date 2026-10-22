// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer2/resources.py
// and rescale_image() in src/calibre/ebooks/mobi/utils.py
// (calibre 9.16, commit 0b8c0c2).

//! Image and font records, the Cover and its thumbnail.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage};

use super::oeb::{Data, Oeb};
use super::utils::write_font_record;

/// The thumbnail size Amazon (and kindlegen 2.5) use, even on a Paperwhite.
const MAX_THUMB_SIZE: usize = 16 * 1024;
const MAX_THUMB_DIMEN: (u32, u32) = (180, 240);

/// Stands in for images no text refers to, so record numbers stay put.
const PLACEHOLDER_GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\xf0\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00!\xfe calibre-placeholder-gif-for-azw3\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";

const RASTER_IMAGES: &[&str] = &["image/gif", "image/jpeg", "image/png", "image/webp"];

#[derive(Debug, Default)]
pub struct Resources {
    /// Item href → record number among the resources, counting from 1.
    pub item_map: HashMap<String, usize>,
    /// Item href → MIME type of its record.
    pub mime_map: HashMap<String, String>,
    pub records: Vec<Vec<u8>>,
    used_image_indices: HashSet<usize>,
    image_indices: HashSet<usize>,
    pub cover_offset: Option<usize>,
    pub thumbnail_offset: Option<usize>,
    pub has_fonts: bool,
}

impl Resources {
    /// Every raster image and TrueType/OpenType font in the manifest. With
    /// `cover`, the Library's Cover replaces the EPUB's cover image (or is
    /// added if there is none).
    pub fn new(oeb: &Oeb, cover: Option<&[u8]>) -> Resources {
        let mut res = Resources::default();
        let mut index = 1;
        let cover_href = oeb.cover_image.map(|i| oeb.items[i].href.as_str());

        let mut images: Vec<(&str, &[u8])> = oeb
            .items
            .iter()
            .filter(|i| RASTER_IMAGES.contains(&i.media_type.as_str()))
            .filter_map(|i| match &i.data {
                Data::Raw(bytes) => Some((i.href.as_str(), bytes.as_slice())),
                _ => None,
            })
            .collect();
        // An href no item has, so the Library's Cover gets a record even
        // when the EPUB has no cover image.
        const ADDED_COVER: &str = "\0cover";
        let cover_href = match (cover, cover_href) {
            (Some(c), Some(href)) => {
                images
                    .iter_mut()
                    .filter(|(h, _)| *h == href)
                    .for_each(|(_, b)| *b = c);
                Some(href)
            }
            (Some(c), None) => {
                images.push((ADDED_COVER, c));
                Some(ADDED_COVER)
            }
            (None, href) => href,
        };

        for (href, bytes) in images {
            // A bad image is left out; the text keeps its original link.
            let Some((data, format)) = process_image(bytes) else {
                continue;
            };
            res.image_indices.insert(res.records.len());
            res.records.push(data);
            res.item_map.insert(href.to_owned(), index);
            res.mime_map
                .insert(href.to_owned(), format!("image/{format}"));
            index += 1;

            if Some(href) == cover_href {
                let cover_offset = index - 2;
                res.cover_offset = Some(cover_offset);
                res.used_image_indices.insert(cover_offset);
                if let Some(thumb) = thumbnail(res.records.last().unwrap()) {
                    res.image_indices.insert(res.records.len());
                    res.records.push(thumb);
                    res.thumbnail_offset = Some(index - 1);
                    res.used_image_indices.insert(index - 1);
                    index += 1;
                }
            }
        }

        for item in &oeb.items {
            let ext = item
                .href
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if (ext == "ttf" || ext == "otf")
                && !oeb.obfuscated.contains(&item.href)
                && let Data::Raw(bytes) = &item.data
            {
                res.records.push(write_font_record(bytes));
                res.item_map.insert(item.href.clone(), res.records.len());
                res.has_fonts = true;
            }
        }
        res
    }

    pub fn is_font(&self, index: usize) -> bool {
        self.records[index - 1].starts_with(b"FONT")
    }

    /// The resource records, with images nothing uses replaced by a
    /// placeholder.
    pub fn serialize(mut self, used_images: &HashSet<String>) -> Vec<Vec<u8>> {
        let mut used = self.used_image_indices.clone();
        for (href, &index) in &self.item_map {
            if used_images.contains(href) {
                used.insert(index - 1);
            }
        }
        for &i in self.image_indices.difference(&used) {
            self.records[i] = PLACEHOLDER_GIF.to_vec();
        }
        self.records
    }
}

/// The image as it goes into the book, and its format name: WebP becomes
/// PNG, which Kindles can show, and JPEGs are fixed for Amazon's renderer.
/// `None` if it cannot be read.
fn process_image(data: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let format = image::guess_format(data).ok()?;
    match format {
        ImageFormat::Jpeg => Some((process_jpeg_for_amazon(data)?, "jpeg")),
        ImageFormat::Png => Some((data.to_vec(), "png")),
        ImageFormat::Gif => Some((data.to_vec(), "gif")),
        ImageFormat::WebP => {
            let img = image::load_from_memory_with_format(data, format).ok()?;
            let mut png = Vec::new();
            img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .ok()?;
            Some((png, "png"))
        }
        _ => None,
    }
}

/// Amazon's renderer cannot show JPEGs without JFIF metadata, and does not
/// show images with EXIF data on the cover screen, so those are re-encoded,
/// rotated upright first.
fn process_jpeg_for_amazon(data: &[u8]) -> Option<Vec<u8>> {
    let (jfif, exif) = jpeg_markers(data);
    if jfif && !exif {
        return Some(data.to_vec());
    }
    let mut decoder = ImageReader::with_format(Cursor::new(data), ImageFormat::Jpeg)
        .into_decoder()
        .ok()?;
    let orientation = decoder.orientation().ok()?;
    let mut img = DynamicImage::from_decoder(decoder).ok()?;
    img.apply_orientation(orientation);
    encode_jpeg(&img, 90)
}

/// Whether a JPEG has a JFIF APP0 segment and an EXIF APP1 segment.
fn jpeg_markers(data: &[u8]) -> (bool, bool) {
    let (mut jfif, mut exif) = (false, false);
    let mut i = 2;
    while i + 4 <= data.len() && data[i] == 0xFF {
        let marker = data[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            break; // start of scan, end of image
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        let payload = &data[(i + 4).min(data.len())..(i + 2 + len).min(data.len())];
        match marker {
            0xE0 if payload.starts_with(b"JFIF\0") => jfif = true,
            0xE1 if payload.starts_with(b"Exif\0") => exif = true,
            _ => {}
        }
        i += 2 + len;
    }
    (jfif, exif)
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    // Transparency goes onto white.
    let rgba = img.to_rgba8();
    let rgb = RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let blend = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
        image::Rgb([blend(r), blend(g), blend(b)])
    });
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

/// The Cover scaled to fit 180×240 as a JPEG of at most 16 KiB.
fn thumbnail(cover: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(cover).ok()?;
    let (w, h) = MAX_THUMB_DIMEN;
    let mut img = if img.width() > w || img.height() > h {
        img.resize(w, h, FilterType::Lanczos3)
    } else {
        img
    };
    let mut quality = 90;
    let mut data = encode_jpeg(&img, quality)?;
    while data.len() > MAX_THUMB_SIZE && quality >= 10 {
        quality -= 5;
        data = encode_jpeg(&img, quality)?;
    }
    let mut scale = 0.9;
    while data.len() > MAX_THUMB_SIZE && scale >= 0.05 {
        let (sw, sh) = (
            (img.width() as f64 * scale) as u32,
            (img.height() as f64 * scale) as u32,
        );
        img = img.resize_exact(sw.max(1), sh.max(1), FilterType::Lanczos3);
        data = encode_jpeg(&img, quality)?;
        scale -= 0.05;
    }
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        }));
        encode_jpeg(&img, 90).unwrap()
    }

    #[test]
    fn leaves_jfif_jpegs_alone_and_fixes_exif_ones() {
        let plain = jpeg(8, 8);
        assert_eq!(jpeg_markers(&plain), (true, false));
        assert_eq!(process_jpeg_for_amazon(&plain).unwrap(), plain);

        // Swap the JFIF segment for an empty EXIF one.
        let mut exif = plain[..2].to_vec();
        let tiff = b"II*\0\x08\0\0\0\0\0\0\0\0\0";
        let payload = [&b"Exif\0\0"[..], tiff].concat();
        exif.extend([0xFF, 0xE1]);
        exif.extend(((payload.len() + 2) as u16).to_be_bytes());
        exif.extend(&payload);
        let app0_len = u16::from_be_bytes([plain[4], plain[5]]) as usize;
        exif.extend(&plain[4 + app0_len..]);
        assert_eq!(jpeg_markers(&exif), (false, true));
        let fixed = process_jpeg_for_amazon(&exif).unwrap();
        assert_eq!(jpeg_markers(&fixed), (true, false));
    }

    #[test]
    fn thumbnails_are_small() {
        let thumb = thumbnail(&jpeg(1200, 1800)).unwrap();
        assert!(thumb.len() <= MAX_THUMB_SIZE);
        let img = image::load_from_memory(&thumb).unwrap();
        assert!(img.width() <= 180 && img.height() <= 240);
    }
}
