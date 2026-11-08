//! Portable raster/SVG preparation; called only by background jobs.
use crate::{limits as reader_limits, work::Cancellation};
use anyhow::{Result, ensure};
use std::{io::Cursor, path::Path};

pub fn from_file_cancellable(
    path: &Path,
    cancel: &Cancellation,
) -> Result<crate::media::RasterFrame> {
    let png = thumbnail_png_cancellable(path, reader_limits::INLINE_IMAGE_PREVIEW_EDGE, cancel)?;
    crate::media::decode_png(&png, cancel)
}
pub(crate) fn thumbnail_png(path: &Path, max_edge: u32) -> Result<Vec<u8>> {
    thumbnail_png_cancellable(path, max_edge, &Cancellation::default())
}
pub(crate) fn thumbnail_png_cancellable(
    path: &Path,
    max_edge: u32,
    cancel: &Cancellation,
) -> Result<Vec<u8>> {
    cancel.check()?;
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("qoi"))
    {
        let converted = crate::qoi::prepare(path, cancel)?;
        return thumbnail_png_cancellable(converted.path(), max_edge, cancel);
    }
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("icns"))
    {
        let bytes = std::fs::read(path)?;
        let png = extract_icns_png(&bytes)?;
        return thumbnail_png_from_bytes(&png, max_edge, cancel);
    }
    if let Some(size) = crate::svg_preview::dimensions(path, cancel)? {
        let prepared = crate::svg_preview::prepare_image(path, cancel, false)?;
        let options = resvg::usvg::Options {
            fontdb: svg_fontdb().clone(),
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_data(&std::fs::read(prepared.path())?, &options)?;
        let scale = (max_edge as f32 / (size.0 as f32).max(size.1 as f32)).min(1.);
        let width = (size.0 as f32 * scale).ceil().max(1.) as u32;
        let height = (size.1 as f32 * scale).ceil().max(1.) as u32;
        let required = u64::from(width) * u64::from(height) * 4;
        let _memory = crate::resources::pool().reserve_working(required, required)?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| anyhow::anyhow!("Image dimensions overflow"))?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        cancel.check()?;
        return Ok(pixmap.encode_png()?);
    }
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let (width, height) = reader.into_dimensions()?;
    ensure!(width > 0 && height > 0, "Invalid image dimensions");
    let required = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(8))
        .ok_or_else(|| anyhow::anyhow!("Image dimensions overflow"))?;
    let _memory = crate::resources::pool().reserve_working(required, required)?;
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(required.max(reader_limits::INLINE_IMAGE_DECODED_BYTES));
    reader.limits(limits);
    let image = reader.decode()?;
    cancel.check()?;
    encode_thumbnail(image, width, height, max_edge, cancel)
}

fn thumbnail_png_from_bytes(bytes: &[u8], max_edge: u32, cancel: &Cancellation) -> Result<Vec<u8>> {
    cancel.check()?;
    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let (width, height) = reader.into_dimensions()?;
    ensure!(width > 0 && height > 0, "Invalid image dimensions");
    let required = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(8))
        .ok_or_else(|| anyhow::anyhow!("Image dimensions overflow"))?;
    let _memory = crate::resources::pool().reserve_working(required, required)?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(required.max(reader_limits::INLINE_IMAGE_DECODED_BYTES));
    reader.limits(limits);
    let image = reader.decode()?;
    cancel.check()?;
    encode_thumbnail(image, width, height, max_edge, cancel)
}

fn encode_thumbnail(
    image: image::DynamicImage,
    width: u32,
    height: u32,
    max_edge: u32,
    cancel: &Cancellation,
) -> Result<Vec<u8>> {
    let thumbnail = if width <= max_edge && height <= max_edge {
        image
    } else {
        image.thumbnail(max_edge, max_edge)
    };
    let thumbnail = match thumbnail {
        image::DynamicImage::ImageRgb32F(_) | image::DynamicImage::ImageRgba32F(_) => {
            image::DynamicImage::ImageRgba8(thumbnail.to_rgba8())
        }
        other => other,
    };
    let mut output = Cursor::new(Vec::new());
    thumbnail.write_to(&mut output, image::ImageFormat::Png)?;
    cancel.check()?;
    Ok(output.into_inner())
}

fn extract_icns_png(bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() >= 8 && &bytes[0..4] == b"icns",
        "Invalid ICNS magic"
    );
    let total_len = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
    ensure!(bytes.len() >= total_len, "Truncated ICNS file");
    let mut offset = 8;
    let mut largest_png: Option<Vec<u8>> = None;
    while offset + 8 <= total_len {
        let chunk_len = u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into()?) as usize;
        ensure!(
            chunk_len >= 8 && offset + chunk_len <= total_len,
            "Invalid ICNS chunk length"
        );
        let payload = &bytes[offset + 8..offset + chunk_len];
        if payload.starts_with(b"\x89PNG\r\n\x1a\n")
            && largest_png.as_ref().map_or(0, |p| p.len()) < payload.len()
        {
            largest_png = Some(payload.to_vec());
        }
        offset += chunk_len;
    }
    largest_png.ok_or_else(|| anyhow::anyhow!("No PNG icon found in ICNS"))
}

fn svg_fontdb() -> &'static std::sync::Arc<resvg::usvg::fontdb::Database> {
    static FONT_DB: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    FONT_DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        std::sync::Arc::new(db)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_and_svg_previews_are_portable() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let png = directory.path().join("sample.png");
        image::RgbaImage::from_pixel(80, 40, image::Rgba([10, 30, 80, 255])).save(&png)?;
        let bytes = thumbnail_png(&png, 20)?;
        assert_eq!(image::load_from_memory(&bytes)?.width(), 20);
        let svg = directory.path().join("sample.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="red"/></svg>"#,
        )?;
        let bytes = thumbnail_png(&svg, 20)?;
        let image = image::load_from_memory(&bytes)?;
        assert!(image.width() <= 20 && image.height() <= 20);
        Ok(())
    }
}
