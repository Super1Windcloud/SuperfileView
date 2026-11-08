//! An immutable private snapshot allows random access without reading a whole file into RAM.
#![allow(unsafe_code)]
use crate::work::Cancellation;
use anyhow::{Result, ensure};
use std::{fs::File, path::Path};

pub(crate) struct MappedSnapshot {
    mapping: memmap2::Mmap,
    // This unnamed, private file is never exposed or modified after mapping.
    _file: File,
}
impl AsRef<[u8]> for MappedSnapshot {
    fn as_ref(&self) -> &[u8] {
        &self.mapping
    }
}
impl MappedSnapshot {
    pub(crate) fn open(path: &Path, cancel: &Cancellation) -> Result<Self> {
        cancel.check()?;
        let mut source = File::open(path)?;
        let before = source.metadata()?;
        ensure!(before.is_file() && before.len() > 0, "Document is empty");
        let mut snapshot = tempfile::tempfile()?;
        cancel.copy(&mut source, &mut snapshot)?;
        let after = source.metadata()?;
        ensure!(
            before.len() == after.len() && before.modified().ok() == after.modified().ok(),
            "Document changed while preparing its snapshot"
        );
        snapshot.sync_all()?;
        cancel.check()?;
        // SAFETY: the mapped file is an unnamed snapshot owned exclusively by this
        // object. No writable handle or path escapes, and all writes finished above.
        // The mapping is dropped before the private file. Original-file edits cannot
        // truncate or modify this mapping.
        let mapping = unsafe { memmap2::MmapOptions::new().map(&snapshot)? };
        Ok(Self {
            mapping,
            _file: snapshot,
        })
    }
}
