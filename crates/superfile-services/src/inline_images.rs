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
#[cfg(test)]
pub(crate) fn thumbnail_png(path: &Path, max_edge: u32) -> Result<Vec<u8>> {
    thumbnail_png_cancellable(path, max_edge, &Cancellation::default())
}
pub(crate) fn thumbnail_png_cancellable(
    path: &Path,
    max_edge: u32,
    cancel: &Cancellation,
) -> Result<Vec<u8>> {
    thumbnail_with_dimensions(path, max_edge, cancel).map(|(png, _)| png)
}
pub(crate) fn thumbnail_with_dimensions(
    path: &Path,
    max_edge: u32,
    cancel: &Cancellation,
) -> Result<(Vec<u8>, (f64, f64))> {
    cancel.check()?;
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("qoi"))
    {
        let (converted, size) = crate::qoi::prepare_with_size(path, cancel)?;
        return Ok((
            thumbnail_png_cancellable(converted.path(), max_edge, cancel)?,
            (size.0.into(), size.1.into()),
        ));
    }
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("icns"))
    {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let required = len
            .checked_mul(3)
            .ok_or_else(|| anyhow::anyhow!("ICNS size overflow"))?;
        let _working = crate::resources::pool().reserve_working(required, required)?;
        let mut bytes = Vec::new();
        file.take(len).read_to_end(&mut bytes)?;
        let png = extract_icns_png(&bytes)?;
        return thumbnail_png_from_bytes(&png, max_edge, cancel);
    }
    // Binary image metadata may contain XML-like bytes. Recognize rasters before
    // asking the XML reader for SVG dimensions.
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    if reader.format().is_none()
        && let Some(size) = crate::svg_preview::dimensions(path, cancel)?
    {
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
        return Ok((pixmap.encode_png()?, size));
    }
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

/// Resolve a table cell value into an image URL when it represents a supported
/// data URI, HTTP(S) image link, or existing local image file.
/// Performs filesystem checks and must only run on a background worker.
pub fn resolve_cell_image_url(value: &str, base_dir: Option<&Path>) -> Option<url::Url> {
    let candidate = extract_cell_image_target(value)?;
    let lower = candidate.to_ascii_lowercase();
    if lower.starts_with("data:image/") {
        return url::Url::parse(candidate).ok();
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let url = url::Url::parse(candidate).ok()?;
        if !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        return crate::media::supports(Path::new(url.path())).then_some(url);
    }
    if lower.starts_with("file://") {
        let url = url::Url::parse(candidate).ok()?;
        let path = url.to_file_path().ok()?;
        return (crate::media::supports(&path) && path.is_file()).then_some(url);
    }
    let raw_path = Path::new(candidate);
    if !crate::media::supports(raw_path) {
        return None;
    }
    let resolved = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base_dir?.join(raw_path)
    };
    resolved
        .is_file()
        .then(|| url::Url::from_file_path(resolved).ok())
        .flatten()
}

fn extract_cell_image_target(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains(['\n', '\r']) {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("![")
        && let Some((_, tail)) = rest.split_once("](")
        && let Some(inner) = tail.strip_suffix(')')
    {
        let target = inner.split_whitespace().next()?.trim();
        return (!target.is_empty()).then_some(target);
    }
    if trimmed
        .as_bytes()
        .get(..4)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"<img"))
    {
        for quote in ['"', '\''] {
            let pattern = format!("src={quote}");
            if let Some(start) = trimmed.to_ascii_lowercase().find(&pattern)
                && let Some(after) = trimmed.get(start + pattern.len()..)
                && let Some(end) = after.find(quote)
            {
                let target = after[..end].trim();
                return (!target.is_empty()).then_some(target);
            }
        }
    }
    Some(trimmed)
}

/// Decode only the static raster data URLs emitted by notebook outputs.
pub fn from_data_uri(value: &str, cancel: &Cancellation) -> Result<crate::media::RasterFrame> {
    use base64::Engine;
    let (mime, data) = value
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("Malformed inline image"))?;
    let expected = match mime {
        "data:image/png;base64" => image::ImageFormat::Png,
        "data:image/jpeg;base64" => image::ImageFormat::Jpeg,
        "data:image/webp;base64" => image::ImageFormat::WebP,
        "data:image/gif;base64" => image::ImageFormat::Gif,
        _ => anyhow::bail!("Unsupported embedded image encoding"),
    };
    let required = u64::try_from(data.len())?;
    let _input = crate::resources::pool().reserve_working(required, required)?;
    cancel.check()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
    ensure!(
        image::guess_format(&bytes)? == expected,
        "Embedded image type mismatch"
    );
    let (png, _) =
        thumbnail_png_from_bytes(&bytes, reader_limits::INLINE_IMAGE_PREVIEW_EDGE, cancel)?;
    crate::media::decode_png(&png, cancel)
}

fn thumbnail_png_from_bytes(
    bytes: &[u8],
    max_edge: u32,
    cancel: &Cancellation,
) -> Result<(Vec<u8>, (f64, f64))> {
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
) -> Result<(Vec<u8>, (f64, f64))> {
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
    Ok((output.into_inner(), (width.into(), height.into())))
}

fn extract_icns_png(bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() >= 8 && &bytes[0..4] == b"icns",
        "Invalid ICNS magic"
    );
    let total_len = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
    ensure!(bytes.len() >= total_len, "Truncated ICNS file");
    let mut offset = 8;
    let mut largest_png: Option<&[u8]> = None;
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
            largest_png = Some(payload);
        }
        offset += chunk_len;
    }
    ensure!(offset == total_len, "Truncated ICNS chunk");
    if let Some(png) = largest_png {
        return Ok(png.to_vec());
    }
    let family = icns::IconFamily::read(&bytes[..total_len])?;
    let mut icons = family.available_icons();
    icons.sort_by_key(|icon| std::cmp::Reverse(icon.pixel_width() * icon.pixel_height()));
    for icon in icons {
        if let Ok(image) = family.get_icon_with_type(icon) {
            let mut png = Vec::new();
            image.write_png(&mut png)?;
            return Ok(png);
        }
    }
    anyhow::bail!("No decodable icon in ICNS")
}

pub(crate) fn svg_fontdb() -> &'static std::sync::Arc<resvg::usvg::fontdb::Database> {
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
    fn legacy_icns_decodes_rgb_and_mask_without_a_png_chunk() -> Result<()> {
        let mut family = icns::IconFamily::new();
        let rgba = [27, 83, 115, 200].repeat(16 * 16);
        let image = icns::Image::from_data(icns::PixelFormat::RGBA, 16, 16, rgba)?;
        family.add_icon_with_type(&image, icns::IconType::RGB24_16x16)?;
        let mut bytes = Vec::new();
        family.write(&mut bytes)?;
        let decoded = image::load_from_memory(&extract_icns_png(&bytes)?)?.into_rgba8();
        assert_eq!(decoded.dimensions(), (16, 16));
        assert_eq!(decoded.get_pixel(3, 7).0, [27, 83, 115, 200]);
        Ok(())
    }

    #[test]
    fn raster_metadata_is_not_parsed_as_xml() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("metadata.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&path)?, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.add_text_chunk("Description".into(), "A & B <diagram>".into())?;
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&[128; 16])?;
        writer.finish()?;
        let bytes = thumbnail_png(&path, 1)?;
        assert_eq!(image::load_from_memory(&bytes)?.width(), 1);
        Ok(())
    }
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

    #[test]
    fn resolves_table_cell_image_links_and_local_paths() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let local = dir.path().join("shot.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255])).save(&local)?;
        assert_eq!(
            resolve_cell_image_url("shot.png", Some(dir.path())),
            url::Url::from_file_path(&local).ok()
        );
        assert_eq!(
            resolve_cell_image_url("![alt](shot.png)", Some(dir.path())),
            url::Url::from_file_path(&local).ok()
        );
        assert_eq!(
            resolve_cell_image_url("missing.png", Some(dir.path())),
            None
        );
        assert!(resolve_cell_image_url("https://example.com/a/b.png?w=100", None).is_some());
        assert!(resolve_cell_image_url("https://example.com/page.html", None).is_none());
        assert!(resolve_cell_image_url("data:image/png;base64,iVBORw0KGgo=", None).is_some());
        assert_eq!(resolve_cell_image_url("中文标题", Some(dir.path())), None);
        assert_eq!(
            resolve_cell_image_url(r#"<IMG SRC="shot.png">"#, Some(dir.path())),
            url::Url::from_file_path(&local).ok()
        );
        Ok(())
    }
}
