//! Blocking system association queries and explicit updates. Run on a background executor.
use crate::{storage, work::Cancellation};
use anyhow::Result;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
use std::path::Path;
pub use superfile_core::file_associations::{Application, Association, Snapshot};
#[cfg(windows)]
use windows as platform;

#[cfg(any(target_os = "linux", test))]
mod globs;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[derive(Clone, Debug)]
pub enum ChangeResult {
    Applied(Application),
    /// A submitted change can be verified later without repeating the write.
    ConfirmationPending {
        read_handler: fn(&str) -> Option<String>,
    },
    SystemSettingsRequired,
}

const OFFER_RECORD: &str = "default-application-offer";

pub struct DefaultOffer {
    identifier: String,
    extension: String,
    application: Application,
}

impl DefaultOffer {
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn extension(&self) -> &str {
        &self.extension
    }

    pub fn remember(&self) -> Result<()> {
        storage::set_record(&storage::open()?, OFFER_RECORD, &self.identifier, "shown")
    }

    pub fn apply(&self) -> Result<ChangeResult> {
        set_default(&self.identifier, &self.application)
    }
}

pub fn default_offer(path: &Path) -> Result<Option<DefaultOffer>> {
    let Some(extension) = offered_extension(path) else {
        return Ok(None);
    };
    if !path.is_file() {
        return Ok(None);
    }
    let Some((identifier, application)) = platform::offer_type(path, &extension)? else {
        return Ok(None);
    };
    validate_identifier(&identifier)?;
    if storage::record(&storage::open()?, OFFER_RECORD, &identifier)?.is_some() {
        return Ok(None);
    }
    Ok(Some(DefaultOffer {
        identifier,
        extension,
        application,
    }))
}

fn offered_extension(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    superfile_core::formats::CATALOG_SUFFIXES
        .contains(&extension.as_str())
        .then_some(extension)
}

pub fn applications(identifier: &str, cancel: &Cancellation) -> Result<Vec<Application>> {
    validate_identifier(identifier)?;
    cancel.check()?;
    let mut applications = platform::applications(identifier, cancel)?;
    applications.sort_by_cached_key(|app| app.name.to_lowercase());
    let mut unique = Vec::<Application>::new();
    for application in applications {
        if !unique.iter().any(|app| app.same_handler(&application)) {
            unique.push(application);
        }
    }
    cancel.check()?;
    Ok(unique)
}

// A submitted write runs to completion even if the originating view is closed.
pub fn set_default(identifier: &str, application: &Application) -> Result<ChangeResult> {
    validate_identifier(identifier)?;
    let cancel = Cancellation::default();
    let candidates = applications(identifier, &cancel)?;
    let selected = candidates
        .iter()
        .find(|candidate| candidate.same_handler(application))
        .ok_or_else(|| {
            anyhow::anyhow!("The application is no longer registered for this file type")
        })?;
    platform::set_default(identifier, selected)
}

fn validate_identifier(identifier: &str) -> Result<()> {
    anyhow::ensure!(
        !identifier.is_empty() && identifier.len() <= 4096 && !identifier.contains('\0'),
        "Invalid system file type identifier"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn verified(application: Application, expected: &Application) -> Result<ChangeResult> {
    anyhow::ensure!(
        application.same_handler(expected),
        "The system did not confirm the requested default application; refresh to see the current setting"
    );
    Ok(ChangeResult::Applied(application))
}

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
    fn offers_only_use_named_extensions_from_the_supported_format_catalog() {
        assert_eq!(
            offered_extension(Path::new("paper.PDF")).as_deref(),
            Some("pdf")
        );
        for name in ["README", ".gitignore", "file.unknown-type", "program.exe"] {
            assert_eq!(offered_extension(Path::new(name)), None);
        }
    }

    #[test]
    fn cancelled_scan_does_not_enter_native_discovery() {
        let (cancel, guard) = Cancellation::task();
        drop(guard);
        assert!(scan(&cancel).is_err());
        assert!(applications("public.text", &cancel).is_err());
    }

    #[test]
    fn invalid_identifiers_are_rejected_before_native_queries_or_writes() {
        let app = Application {
            name: "Test".into(),
            identifier: Some("test".into()),
            path: None,
        };
        for identifier in ["", "public.text\0injected"] {
            assert!(applications(identifier, &Cancellation::default()).is_err());
            assert!(set_default(identifier, &app).is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn updates_require_the_native_handler_to_match_the_requested_application() {
        let expected = Application {
            name: "Test".into(),
            identifier: Some("test.expected".into()),
            path: None,
        };
        let other = Application {
            identifier: Some("test.other".into()),
            ..expected.clone()
        };
        assert!(verified(other, &expected).is_err());
        assert!(matches!(
            verified(expected.clone(), &expected).unwrap(),
            ChangeResult::Applied(_)
        ));
    }
}
