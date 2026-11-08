//! Prepare native preview data off the UI thread; launch the settings helper only.
use anyhow::{Context, Result, ensure};
use std::path::Path;

pub fn is_media(path: &Path) -> bool {
    superfile_core::formats::FormatSet::NativeMedia.matches(path)
}
pub fn folder_has_media(path: &Path) -> Result<bool> {
    folder_has_media_cancellable(path, &crate::work::Cancellation::default())
}
pub fn folder_has_media_cancellable(
    path: &Path,
    cancel: &crate::work::Cancellation,
) -> Result<bool> {
    cancel.check()?;
    for entry in std::fs::read_dir(path)? {
        cancel.check()?;
        let entry = entry?;
        if is_media(&entry.path()) && entry.file_type()?.is_file() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn is_image(path: &Path) -> bool {
    crate::media::supports(path)
        && !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"))
}

pub fn is_font(path: &Path) -> bool {
    superfile_core::formats::FormatSet::NativeFont.matches(path)
}

pub fn is_office(path: &Path) -> bool {
    superfile_core::formats::FormatSet::NativeOffice.matches(path)
}

pub fn is_iwork(path: &Path) -> bool {
    superfile_core::formats::FormatSet::Iwork.matches(path)
}

/// `.key` is also used by plain-text cryptographic keys. Only document packages
/// and ZIP containers should be handed to Keynote's Quick Look provider.
pub fn can_open(path: &Path) -> bool {
    if !supports(path) {
        return false;
    }
    if path.is_dir() {
        return is_iwork(path);
    }
    if !path.is_file() {
        return false;
    }
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("key"))
    {
        use std::io::Read;
        let mut prefix = [0; 4];
        return std::fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut prefix))
            .is_ok()
            && prefix == *b"PK\x03\x04";
    }
    true
}

pub fn supports(path: &Path) -> bool {
    is_media(path)
        || is_image(path)
        || is_font(path)
        || is_office(path)
        || superfile_core::formats::FormatSet::Pdf.matches(path)
}

pub fn prepare_matroska_file(source: &Path, destination: &Path) -> Result<()> {
    let directory = destination
        .parent()
        .context("Media destination has no parent")?;
    ensure!(!destination.exists(), "Media destination already exists");
    let file = crate::matroska::prepare(source, directory, &crate::work::Cancellation::default())?;
    file.persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

pub fn open_associations(_cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<()> {
    #[cfg(windows)]
    return crate::platform::open("ms-settings:defaultapps");
    #[cfg(target_os = "macos")]
    return crate::platform::open("x-apple.systempreferences:com.apple.preference.general");
    #[cfg(target_os = "linux")]
    crate::platform::open(crate::platform::home()?)
}
