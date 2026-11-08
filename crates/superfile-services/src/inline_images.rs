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
    if let Some(size) = crate::svg_preview::dimensions(path, cancel)? {
        let prepared = crate::svg_preview::prepare_thumbnail(path, size, max_edge, cancel)?;
        let mut options = resvg::usvg::Options::default();
        options.fontdb_mut().load_system_fonts();
        let tree = resvg::usvg::Tree::from_data(&std::fs::read(prepared.path())?, &options)?;
        let size = tree.size();
        let scale = (max_edge as f32 / size.width().max(size.height())).min(1.);
        let width = (size.width() * scale).ceil().max(1.) as u32;
        let height = (size.height() * scale).ceil().max(1.) as u32;
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
    let thumbnail = if width <= max_edge && height <= max_edge {
        image
    } else {
        image.thumbnail(max_edge, max_edge)
    };
    let mut output = Cursor::new(Vec::new());
    thumbnail.write_to(&mut output, image::ImageFormat::Png)?;
    cancel.check()?;
    Ok(output.into_inner())
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
