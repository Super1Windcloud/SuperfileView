//! Extract on the target volume, validate the payload, and swap with rollback.
use super::*;
#[cfg(test)]
mod tests;
use gpui_updater_core::install::{OwnershipHints, ownership};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path},
};

const MAX_EXPANDED: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 65_536;

pub struct PreparedUpdate {
    pub(super) download: Option<tempfile::TempDir>,
    staging: tempfile::TempDir,
    payload: PathBuf,
    target: PathBuf,
    pub artifact: PathBuf,
    pub install_blocker: Option<String>,
    version: Version,
}
#[derive(Clone, Debug)]
pub struct Installed {
    pub executable: PathBuf,
    pub backup: PathBuf,
}

pub(super) fn running_root() -> Result<PathBuf> {
    let executable = std::env::current_exe()?.canonicalize()?;
    #[cfg(target_os = "macos")]
    if let Some(root) = executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
    {
        return Ok(root.to_owned());
    }
    Ok(executable)
}

fn blocker(target: &Path, allow_install: bool) -> Option<String> {
    if !allow_install {
        return Some("Local builds can check and download updates, but cannot replace the development executable.".into());
    }
    if target
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation")
    {
        return Some(
            "Move Superfile out of the disk image or translocated folder before updating.".into(),
        );
    }
    #[cfg(target_os = "macos")]
    if target
        .extension()
        .is_none_or(|extension| extension != "app")
    {
        return Some("Install the Superfile app bundle before using in-app updates.".into());
    }
    let owner = ownership(target, &OwnershipHints::new().query_package_database(false));
    if !owner.is_self_update() {
        return Some(owner.update_command().unwrap_or_else(|| {
            "Use the package manager that installed Superfile to update it.".into()
        }));
    }
    // Also protect system-wide Linux locations even without invoking a package CLI.
    #[cfg(target_os = "linux")]
    if ["/usr", "/bin", "/sbin", "/opt", "/nix", "/snap"]
        .iter()
        .any(|root| target.starts_with(root))
    {
        return Some("Use the package manager that installed Superfile to update it.".into());
    }
    None
}

pub(super) fn prepare(
    artifact: &Path,
    release: &Release,
    allow_install: bool,
    cancel: &CancelToken,
) -> Result<PreparedUpdate> {
    let target = running_root()?;
    prepare_at(artifact, release, target, allow_install, cancel)
}

fn prepare_at(
    artifact: &Path,
    release: &Release,
    target: PathBuf,
    allow_install: bool,
    cancel: &CancelToken,
) -> Result<PreparedUpdate> {
    cancelled(cancel)?;
    let install_blocker = blocker(&target, allow_install);
    let parent = if install_blocker.is_none() {
        target
            .parent()
            .context("Install target has no parent directory")?
            .to_owned()
    } else {
        crate::platform::cache_directory()?.join("updates")
    };
    fs::create_dir_all(&parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".superfile-update-")
        .tempdir_in(parent)?;
    extract(artifact, staging.path(), cancel)?;
    let payload = payload(staging.path(), &release.asset.name)?;
    validate_payload(&payload, &release.version)?;
    cancelled(cancel)?;
    Ok(PreparedUpdate {
        download: None,
        staging,
        payload,
        target,
        artifact: artifact.to_owned(),
        install_blocker,
        version: release.version.clone(),
    })
}

fn safe_relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && !path.to_string_lossy().contains('\\')
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
        "Unsafe path in update archive: {}",
        path.display()
    );
    Ok(())
}

fn copy_entry(
    reader: &mut impl Read,
    path: &Path,
    size: u64,
    total: &mut u64,
    cancel: &CancelToken,
) -> Result<()> {
    *total = total
        .checked_add(size)
        .context("Expanded update size overflow")?;
    ensure!(*total <= MAX_EXPANDED, "Expanded update exceeds 4 GiB");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut left = size;
    let mut buffer = vec![0; 128 * 1024];
    while left > 0 {
        cancelled(cancel)?;
        let count = reader.read(&mut buffer[..left.min(128 * 1024) as usize])?;
        ensure!(count != 0, "Truncated update archive");
        output.write_all(&buffer[..count])?;
        left -= count as u64;
    }
    output.sync_all()?;
    ensure!(
        reader.read(&mut [0])? == 0,
        "Update archive entry size mismatch"
    );
    Ok(())
}

fn extract(artifact: &Path, destination: &Path, cancel: &CancelToken) -> Result<()> {
    let mut total = 0;
    if artifact
        .extension()
        .is_some_and(|extension| extension == "zip")
    {
        let mut archive = zip::ZipArchive::new(fs::File::open(artifact)?)?;
        ensure!(
            archive.len() <= MAX_ENTRIES,
            "Too many update archive entries"
        );
        for index in 0..archive.len() {
            cancelled(cancel)?;
            let mut entry = archive.by_index(index)?;
            let path = entry.enclosed_name().context("Unsafe ZIP path")?;
            safe_relative(&path)?;
            ensure!(
                entry
                    .unix_mode()
                    .is_none_or(|mode| matches!(mode & 0o170000, 0 | 0o100000 | 0o040000)),
                "Update archives must contain only regular files and directories"
            );
            let path = destination.join(path);
            if entry.is_dir() {
                fs::create_dir_all(path)?;
            } else {
                let size = entry.size();
                copy_entry(&mut entry, &path, size, &mut total, cancel)?;
            }
        }
    } else {
        let mut archive =
            tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(artifact)?));
        for (index, entry) in archive.entries()?.enumerate() {
            ensure!(index < MAX_ENTRIES, "Too many update archive entries");
            cancelled(cancel)?;
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            safe_relative(&path)?;
            let kind = entry.header().entry_type();
            ensure!(
                kind.is_file() || kind.is_dir(),
                "Update archives must contain only regular files and directories"
            );
            let path = destination.join(path);
            if kind.is_dir() {
                fs::create_dir_all(path)?;
            } else {
                let size = entry.size();
                let mode = entry.header().mode()?;
                copy_entry(&mut entry, &path, size, &mut total, cancel)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o777))?;
                }
                #[cfg(not(unix))]
                let _ = mode;
            }
        }
    }
    Ok(())
}

fn payload(root: &Path, name: &str) -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        Ok(root.join("Superfile.app"))
    } else if cfg!(windows) {
        Ok(root.join("superfile.exe"))
    } else {
        Ok(root
            .join(
                name.strip_suffix(".tar.gz")
                    .context("Expected a tar.gz update archive")?,
            )
            .join("superfile"))
    }
}

fn validate_payload(payload: &Path, version: &Version) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        ensure!(
            fs::metadata(payload.join("Contents/Info.plist"))?.len() <= 1024 * 1024,
            "Update Info.plist exceeds 1 MiB"
        );
        let info = plist_parser::Value::from_file(payload.join("Contents/Info.plist"))?;
        let info = info.as_dictionary().context("Invalid app Info.plist")?;
        ensure!(
            info.get("CFBundleIdentifier")
                .and_then(plist_parser::Value::as_string)
                == Some("app.superfile.desktop"),
            "Wrong update bundle identifier"
        );
        ensure!(
            info.get("CFBundleExecutable")
                .and_then(plist_parser::Value::as_string)
                == Some("superfile"),
            "Wrong update executable"
        );
        ensure!(
            info.get("CFBundleShortVersionString")
                .and_then(plist_parser::Value::as_string)
                == Some(version.to_string().as_str()),
            "Update bundle version does not match manifest"
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = version;
    let executable = if cfg!(target_os = "macos") {
        payload.join("Contents/MacOS/superfile")
    } else {
        payload.to_owned()
    };
    let metadata = fs::symlink_metadata(&executable)?;
    ensure!(
        metadata.is_file() && metadata.len() > 0,
        "Missing update executable"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "Update executable is not executable"
        );
    }
    Ok(())
}

pub(super) fn commit(update: &PreparedUpdate) -> Result<Installed> {
    ensure!(
        update.install_blocker.is_none(),
        "{}",
        update.install_blocker.as_deref().unwrap_or_default()
    );
    ensure!(
        running_root()? == update.target,
        "Application install location changed"
    );
    ensure!(
        update.staging.path().parent() == update.target.parent(),
        "Update staging must be on the target volume"
    );
    validate_payload(&update.payload, &update.version)?;
    swap(&update.payload, &update.target)
}

fn swap(payload: &Path, target: &Path) -> Result<Installed> {
    let parent = target.parent().context("Install target has no parent")?;
    ensure!(
        !fs::symlink_metadata(target)?.file_type().is_symlink(),
        "Refusing to replace a symbolic-link install target"
    );
    let backup = tempfile::Builder::new()
        .prefix(".superfile-previous-")
        .tempdir_in(parent)?
        .keep();
    let previous = backup.join(target.file_name().context("Invalid install target")?);
    fs::rename(target, &previous).context("Cannot move the running application aside")?;
    if let Err(error) = fs::rename(payload, target) {
        return match fs::rename(&previous, target) {
            Ok(()) => Err(error)
                .context("Cannot install the update; the original application was restored"),
            Err(rollback) => Err(anyhow::anyhow!(
                "Install failed: {error}; rollback failed: {rollback}. Original application is preserved at {}",
                previous.display()
            )),
        };
    }
    let executable = if cfg!(target_os = "macos") {
        target.join("Contents/MacOS/superfile")
    } else {
        target.to_owned()
    };
    Ok(Installed {
        executable,
        backup: previous,
    })
}
