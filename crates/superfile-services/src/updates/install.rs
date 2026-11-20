//! Extract on the target volume, validate the payload, and swap with rollback.
use super::*;
#[cfg(windows)]
mod scoop;
#[cfg(test)]
mod tests;
use gpui_updater_core::install::{OwnershipHints, ownership};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path},
    time::{Duration, SystemTime},
};

const MAX_EXPANDED: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 65_536;
const BACKUP_PREFIX: &str = ".superfile-previous-";
const BACKUP_RECEIPT: &str = "update-receipt";
const STAGING_PREFIX: &str = ".superfile-update-";
const STAGING_MIN_AGE: Duration = Duration::from_secs(10 * 60);

pub struct PreparedUpdate {
    pub(super) download: Option<tempfile::TempDir>,
    staging: tempfile::TempDir,
    payload: PathBuf,
    target: PathBuf,
    pub artifact: PathBuf,
    pub install_blocker: Option<String>,
    version: Version,
    #[cfg(windows)]
    scoop: Option<scoop::Installation>,
}

impl PreparedUpdate {
    pub fn discard(self) {
        for directory in [Some(self.staging), self.download].into_iter().flatten() {
            let path = directory.path().to_owned();
            if let Err(error) = directory.close()
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!(
                    "Cannot remove update temporary directory {}: {error}",
                    path.display()
                );
            }
        }
    }
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
    #[cfg(windows)]
    if scoop::is_scoop(&owner) && scoop::Installation::inspect(target).is_ok() {
        return None;
    }
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
    #[cfg(windows)]
    let scoop = if install_blocker.is_none()
        && scoop::is_scoop(&ownership(
            &target,
            &OwnershipHints::new().query_package_database(false),
        )) {
        Some(scoop::Installation::inspect(&target)?)
    } else {
        None
    };
    let parent = if install_blocker.is_none() {
        let parent = target
            .parent()
            .context("Install target has no parent directory")?
            .to_owned();
        #[cfg(windows)]
        let parent = scoop
            .as_ref()
            .map_or(parent, |install| install.root.clone());
        parent
    } else {
        crate::platform::cache_directory()?.join("updates")
    };
    fs::create_dir_all(&parent)?;
    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .rand_bytes(6)
        .tempdir_in(parent)?;
    // Keep the staging root nonempty throughout extraction, even before the
    // first archive entry is available.
    let contents = staging.path().join("payload");
    fs::create_dir(&contents)?;
    extract(artifact, &contents, cancel)?;
    let payload = payload(&contents, &release.asset.name)?;
    validate_payload(&payload, &release.version)?;
    #[cfg(windows)]
    if let Some(install) = &scoop {
        install.write_metadata(&contents, release)?;
    }
    cancelled(cancel)?;
    Ok(PreparedUpdate {
        download: None,
        staging,
        payload,
        target,
        artifact: artifact.to_owned(),
        install_blocker,
        version: release.version.clone(),
        #[cfg(windows)]
        scoop,
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
        let info = app_info(payload)?;
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

#[cfg(target_os = "macos")]
fn app_info(app: &Path) -> Result<plist_parser::Dictionary> {
    let path = app.join("Contents/Info.plist");
    ensure!(
        fs::metadata(&path)?.len() <= 1024 * 1024,
        "Update Info.plist exceeds 1 MiB"
    );
    plist_parser::Value::from_file(path)?
        .into_dictionary()
        .context("Invalid app Info.plist")
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
    #[cfg(windows)]
    if let Some(install) = &update.scoop {
        validate_payload(&update.payload, &update.version)?;
        return install.commit(update.staging.path(), &update.version);
    }
    ensure!(
        update.staging.path().parent() == update.target.parent(),
        "Update staging must be on the target volume"
    );
    validate_payload(&update.payload, &update.version)?;
    swap(&update.payload, &update.target, &update.version)
}

fn swap(payload: &Path, target: &Path, version: &Version) -> Result<Installed> {
    let parent = target.parent().context("Install target has no parent")?;
    ensure!(
        !fs::symlink_metadata(target)?.file_type().is_symlink(),
        "Refusing to replace a symbolic-link install target"
    );
    let name = target.file_name().context("Invalid install target")?;
    let backup = tempfile::Builder::new()
        .prefix(BACKUP_PREFIX)
        .rand_bytes(6)
        .tempdir_in(parent)?;
    let mut receipt = format!("{version}\n").into_bytes();
    receipt.extend_from_slice(name.as_encoded_bytes());
    fs::write(backup.path().join(BACKUP_RECEIPT), receipt)?;
    let previous = backup.path().join(name);
    fs::rename(target, &previous).context("Cannot move the running application aside")?;
    if let Err(error) = fs::rename(payload, target) {
        return match fs::rename(&previous, target) {
            Ok(()) => Err(error)
                .context("Cannot install the update; the original application was restored"),
            Err(rollback) => {
                let _ = backup.keep();
                Err(anyhow::anyhow!(
                    "Install failed: {error}; rollback failed: {rollback}. Original application is preserved at {}",
                    previous.display()
                ))
            }
        };
    }
    let _ = backup.keep();
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

pub(super) fn cleanup_running_backups() -> Result<()> {
    let target = running_root()?;
    // A backup can itself be launched manually. Only the installed copy may
    // acknowledge a replacement.
    if target
        .ancestors()
        .any(|path| is_backup_dir(path) || is_update_dir(path, STAGING_PREFIX))
        || blocker(&target, true).is_some()
    {
        return Ok(());
    }
    let version = Version::parse(env!("CARGO_PKG_VERSION"))?;
    cleanup_after_startup(&target, &version, SystemTime::now())
}

fn is_backup_dir(path: &Path) -> bool {
    is_update_dir(path, BACKUP_PREFIX)
}

fn is_update_dir(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(prefix))
        .is_some_and(|suffix| {
            suffix.len() == 6 && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT includes directory junctions.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn cleanup_after_startup(target: &Path, running_version: &Version, now: SystemTime) -> Result<()> {
    validate_payload(target, running_version)?;
    let parent = target
        .parent()
        .context("Install target has no parent directory")?;
    let name = target.file_name().context("Invalid install target")?;
    let mut failure = None;
    for entry in fs::read_dir(parent)? {
        let directory = entry?.path();
        let cleanup = if is_backup_dir(&directory) {
            cleanup_backup(&directory, name, running_version)
        } else if is_update_dir(&directory, STAGING_PREFIX) {
            cleanup_staging(&directory, now)
        } else {
            continue;
        };
        if let Err(error) = cleanup {
            failure.get_or_insert_with(|| {
                error.context(format!("Clean update directory {}", directory.display()))
            });
        }
    }
    failure.map_or(Ok(()), Err)
}

fn cleanup_staging(directory: &Path, now: SystemTime) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Ok(());
    }
    // Legacy updaters can be creating a staging directory next to this copy.
    // Give them time to populate it before treating an empty shell as stale.
    let old = metadata
        .modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= STAGING_MIN_AGE);
    if !old {
        return Ok(());
    }
    let mut finder_metadata = None;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name() != ".DS_Store" || !entry.file_type()?.is_file() {
            return Ok(());
        }
        finder_metadata = Some(entry.path());
    }
    if let Some(path) = finder_metadata {
        match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    remove_empty_directory(directory)
}

fn cleanup_backup(
    directory: &Path,
    name: &std::ffi::OsStr,
    running_version: &Version,
) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Ok(());
    }
    let receipt = directory.join(BACKUP_RECEIPT);
    let previous = directory.join(name);
    match fs::symlink_metadata(&receipt) {
        Ok(metadata) if metadata.is_file() && !is_link(&metadata) && metadata.len() <= 4096 => {
            let data = fs::read(&receipt)?;
            let Some(split) = data.iter().position(|byte| *byte == b'\n') else {
                return Ok(());
            };
            let version = std::str::from_utf8(&data[..split])
                .ok()
                .and_then(|text| Version::parse(text).ok());
            if &data[split + 1..] != name.as_encoded_bytes()
                || !version.is_some_and(|version| version <= *running_version)
            {
                return Ok(());
            }
        }
        #[cfg(target_os = "macos")]
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if legacy_backup(&previous, running_version) {
                remove_backup_payload(&previous)?;
                remove_empty_directory(directory)?;
            }
            return Ok(());
        }
        _ => return Ok(()),
    }
    remove_backup_payload(&previous)?;
    // Keep the receipt if the old process still holds the executable open.
    match fs::remove_file(receipt) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    remove_empty_directory(directory)
}

#[cfg(target_os = "macos")]
fn legacy_backup(previous: &Path, running_version: &Version) -> bool {
    let Ok(metadata) = fs::symlink_metadata(previous) else {
        return false;
    };
    if !metadata.is_dir() || is_link(&metadata) {
        return false;
    }
    app_info(previous)
        .ok()
        .and_then(|info| Version::parse(info.get("CFBundleShortVersionString")?.as_string()?).ok())
        .is_some_and(|version| {
            version < *running_version && validate_payload(previous, &version).is_ok()
        })
}

fn remove_backup_payload(previous: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(previous) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(!is_link(&metadata), "Update backup is a link");
    if cfg!(target_os = "macos") && metadata.is_dir() {
        fs::remove_dir_all(previous)?;
    } else if metadata.is_file() {
        fs::remove_file(previous)?;
    } else {
        anyhow::bail!("Unexpected update backup type");
    }
    Ok(())
}

fn remove_empty_directory(directory: &Path) -> Result<()> {
    match fs::remove_dir(directory) {
        Err(error)
            if !matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Err(error.into())
        }
        _ => Ok(()),
    }
}
