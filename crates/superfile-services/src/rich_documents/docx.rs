//! Prepare a continuous Word document without a print-layout conversion.
pub use office_oxide::ir;

use crate::{resources, snapshot::MappedSnapshot, work::Cancellation};
use anyhow::{Result, ensure};
use std::{io::Cursor, path::Path};

pub struct WordDocument {
    pub blocks: Vec<ir::Element>,
    pub memory: resources::WorkingReservation,
    pub partial: bool,
}

pub(super) fn prepare(path: &Path, cancel: &Cancellation) -> Result<WordDocument> {
    let snapshot = MappedSnapshot::open(path, cancel)?;
    let expanded = super::package_size(snapshot.as_ref(), cancel)?;
    let required = expanded.saturating_mul(4).max(1024 * 1024);
    let memory = resources::pool().reserve_working(required, required)?;
    let mut options = office_oxide::options::ParseOptions::default();
    options.limits.memory_bytes = required;
    options.lazy_pictures = true;
    options.include_deleted_text = false;
    options.include_hidden_text = false;
    let document = office_oxide::Document::from_reader_with(
        Cursor::new(snapshot),
        office_oxide::DocumentFormat::Docx,
        &options,
    )?;
    cancel.check()?;
    let document = document.to_ir();
    let partial = document.metadata.text_truncated;
    cancel.check()?;
    let blocks: Vec<_> = document
        .sections
        .into_iter()
        .flat_map(|section| section.elements)
        .collect();
    ensure!(
        !blocks.is_empty(),
        "The Word document has no displayable content"
    );
    Ok(WordDocument {
        blocks,
        memory,
        partial,
    })
}

pub fn image_preview(bytes: &[u8], cancel: &Cancellation) -> Result<crate::media::RasterFrame> {
    let (png, _) = crate::inline_images::thumbnail_png_from_bytes(
        bytes,
        crate::limits::IMAGE_PREVIEW_EDGE,
        cancel,
    )?;
    crate::media::decode_png(&png, cancel)
}
