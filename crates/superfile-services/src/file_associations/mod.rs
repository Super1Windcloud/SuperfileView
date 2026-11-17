//! Blocking, read-only system association discovery. Call from a background executor.
use crate::work::Cancellation;
use anyhow::Result;
pub use superfile_core::file_associations::{Application, Association, Snapshot};

#[cfg(any(target_os = "linux", test))]
mod globs;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub fn scan(cancel: &Cancellation) -> Result<Snapshot> {
    cancel.check()?;
    #[cfg(target_os = "macos")]
    let mut snapshot = macos::scan(cancel)?;
    #[cfg(windows)]
    let mut snapshot = windows::scan(cancel)?;
    #[cfg(target_os = "linux")]
    let mut snapshot = linux::scan(cancel)?;
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    let mut snapshot =
        { anyhow::bail!("System file associations are unavailable on this platform") };
    cancel.check()?;
    snapshot
        .associations
        .sort_by_cached_key(|entry| entry.label().to_lowercase());
    let mut seen = std::collections::BTreeSet::new();
    snapshot
        .associations
        .retain(|entry| seen.insert(entry.identifier.clone()));
    Ok(snapshot)
}

fn warning(snapshot: &mut Snapshot, message: impl Into<String>) {
    // Keep diagnostics bounded without truncating the file-type list.
    let message = message.into();
    if snapshot.warnings.len() < 100 && !snapshot.warnings.contains(&message) {
        snapshot.warnings.push(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_scan_does_not_enter_native_discovery() {
        let (cancel, guard) = Cancellation::task();
        drop(guard);
        assert!(scan(&cancel).is_err());
    }
}
