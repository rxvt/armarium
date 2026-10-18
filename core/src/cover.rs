//! Saving a Book's `cover.jpg`, and thumbnails of it for the UI.

use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, RgbImage};

use crate::{Error, Result};

/// Covers are stored no larger than this on their longest side.
pub const MAX_SIDE: u32 = 1200;
const JPEG_QUALITY: u8 = 90;

/// Decodes `encoded` (JPEG, PNG, GIF or WebP), scales it to fit within
/// [`MAX_SIDE`], flattens any transparency onto white and writes it to `dest`
/// as JPEG, replacing an existing file atomically.
pub fn save(encoded: &[u8], dest: &Path) -> Result<()> {
    let image = ImageReader::new(std::io::Cursor::new(encoded))
        .with_guessed_format()?
        .decode()
        .map_err(|e| Error::Unreadable(format!("cover image: {e}")))?;
    write_jpeg(&fit(image, MAX_SIDE, FilterType::Lanczos3), dest)
}

/// Writes a copy of the Cover at `cover` scaled to fit within `max_side` to
/// `dest`, creating its folder. Meant for a rebuildable cache.
pub fn thumbnail(cover: &Path, dest: &Path, max_side: u32) -> Result<()> {
    let image = ImageReader::open(cover)?
        .with_guessed_format()?
        .decode()
        .map_err(|e| Error::Unreadable(format!("cover image: {e}")))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_jpeg(&fit(image, max_side, FilterType::Triangle), dest)
}

fn fit(image: DynamicImage, max_side: u32, filter: FilterType) -> DynamicImage {
    if image.width().max(image.height()) > max_side {
        image.resize(max_side, max_side, filter)
    } else {
        image
    }
}

fn write_jpeg(image: &DynamicImage, dest: &Path) -> Result<()> {
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&flatten(image))
        .map_err(|e| Error::Unreadable(format!("cover image: {e}")))?;

    let tmp = dest.with_extension("jpg.tmp");
    std::fs::write(&tmp, jpeg)?;
    std::fs::rename(&tmp, dest).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn flatten(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over_white = |c: u8| ((c as u16 * a as u16 + 255 * (255 - a as u16)) / 255) as u8;
        image::Rgb([over_white(r), over_white(g), over_white(b)])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::TempDir;
    use image::{GenericImageView, ImageFormat, Rgba, RgbaImage};

    fn encode(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn writes_jpeg_scaled_to_fit() {
        let dir = TempDir::new();
        let dest = dir.path().join("cover.jpg");
        let big =
            DynamicImage::ImageRgb8(RgbImage::from_pixel(1000, 2400, image::Rgb([200, 10, 10])));
        save(&encode(big, ImageFormat::Png), &dest).unwrap();

        let saved = image::open(&dest).unwrap();
        assert!(
            std::fs::read(&dest)
                .unwrap()
                .starts_with(&[0xFF, 0xD8, 0xFF])
        );
        assert_eq!((saved.width(), saved.height()), (500, 1200));
        assert!(!dir.path().join("cover.jpg.tmp").exists());
    }

    #[test]
    fn keeps_small_images_and_flattens_transparency_onto_white() {
        let dir = TempDir::new();
        let dest = dir.path().join("cover.jpg");
        let clear = DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 60, Rgba([0, 0, 0, 0])));
        save(&encode(clear, ImageFormat::Png), &dest).unwrap();

        let saved = image::open(&dest).unwrap().to_rgb8();
        assert_eq!(saved.dimensions(), (40, 60));
        assert!(saved.get_pixel(20, 30).0.iter().all(|&c| c > 250));
    }

    #[test]
    fn thumbnails_are_scaled_copies_in_new_folders() {
        let dir = TempDir::new();
        let cover = dir.path().join("cover.jpg");
        let big = DynamicImage::ImageRgb8(RgbImage::from_pixel(800, 1200, image::Rgb([0, 90, 0])));
        save(&encode(big, ImageFormat::Png), &cover).unwrap();

        let thumb = dir.path().join("cache/thumbs/1.jpg");
        thumbnail(&cover, &thumb, 300).unwrap();
        assert_eq!(image::open(&thumb).unwrap().dimensions(), (200, 300));
        assert!(cover.exists());
    }

    #[test]
    fn rejects_garbage_and_leaves_existing_cover() {
        let dir = TempDir::new();
        let dest = dir.path().join("cover.jpg");
        std::fs::write(&dest, b"old").unwrap();
        assert!(matches!(
            save(b"definitely not an image", &dest),
            Err(Error::Unreadable(_))
        ));
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    }
}
