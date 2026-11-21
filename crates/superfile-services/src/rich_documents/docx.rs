//! Parse Word documents for the continuous reader.
pub use lo_core as model;

use super::fixed_layout::inspect_layout;
use crate::{
    resources::{self, WorkingReservation},
    snapshot::MappedSnapshot,
    work::Cancellation,
};
use anyhow::{Result, ensure};
use std::path::Path;

pub struct WordDocument {
    pub blocks: Vec<model::Block>,
    pub memory: WorkingReservation,
}

pub(super) fn prepare(path: &Path, cancel: &Cancellation) -> Result<WordDocument> {
    let snapshot = MappedSnapshot::open(path, cancel)?;
    let (required, references_images) = inspect_layout(snapshot.as_ref(), cancel)?;
    let memory = resources::pool().reserve_working(required, required)?;
    cancel.check()?;
    let document = lo_writer::from_docx_bytes_for_web(snapshot.as_ref())?;
    cancel.check()?;
    ensure!(
        !references_images || !document.plain_text().trim().is_empty(),
        "The document's embedded drawings could not be read"
    );
    Ok(WordDocument {
        blocks: document.body,
        memory,
    })
}
