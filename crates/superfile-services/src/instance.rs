//! Single-owner desktop launch forwarding within the application state directory.
use crate::platform::{PrivateDirExt, PrivateFileExt};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const FORWARD_WAIT: Duration = Duration::from_secs(5);

pub enum Launch {
    Primary(Instance, Option<PathBuf>),
    Forwarded,
}

pub struct Instance {
    directory: PathBuf,
    // Keep this inode in place. Removing it would allow two processes to lock
    // different files, and the OS releases its lock when the owner exits.
    _lock: File,
}

pub fn launch(path: Option<PathBuf>) -> Result<Launch> {
    launch_at(&crate::platform::data_directory()?.join("instance"), path)
        .context("Cannot start or contact the Superfile instance")
}

fn launch_at(directory: &Path, path: Option<PathBuf>) -> Result<Launch> {
    fs::DirBuilder::new()
        .recursive(true)
        .private_mode(0o700)
        .create(directory)?;
    crate::platform::restrict_permissions(directory, 0o700)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .private_mode(0o600)
        .open(directory.join("owner.lock"))?;
    let primary = |lock, path| {
        Launch::Primary(
            Instance {
                directory: directory.to_owned(),
                _lock: lock,
            },
            path,
        )
    };
    if try_lock(&lock)? {
        return Ok(primary(lock, path));
    }

    let marker = enqueue(directory, path.as_deref())?;
    let deadline = Instant::now() + FORWARD_WAIT;
    while marker.try_exists()? {
        if try_lock(&lock)? {
            // The former owner was exiting. Drain the existing request on
            // startup if it was not acknowledged before the lock passed to us.
            return if marker.try_exists()? {
                Ok(primary(lock, None))
            } else {
                Ok(Launch::Forwarded)
            };
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(35));
    }
    Ok(Launch::Forwarded)
}

fn try_lock(file: &File) -> Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

fn enqueue(directory: &Path, path: Option<&Path>) -> Result<PathBuf> {
    let value = path
        .map(|path| {
            url::Url::from_file_path(std::path::absolute(path)?)
                .map_err(|_| anyhow::anyhow!("Cannot forward file path: {}", path.display()))
        })
        .transpose()?;
    let bytes = value
        .as_ref()
        .map_or(&[][..], |url| url.as_str().as_bytes());
    ensure!(
        bytes.len() <= MAX_REQUEST_BYTES,
        "Launch request is too large"
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let mut temporary = tempfile::Builder::new()
        .prefix(&format!("launch-{stamp:020}-"))
        .suffix(".pending")
        .tempfile_in(directory)?;
    temporary.write_all(bytes)?;
    let marker = temporary.path().with_extension("open");
    temporary.into_temp_path().persist_noclobber(&marker)?;
    Ok(marker)
}

impl Instance {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

pub struct Request {
    marker: PathBuf,
    pub file_url: Option<String>,
}

impl Request {
    pub fn acknowledge(self) -> Result<()> {
        fs::remove_file(self.marker)?;
        Ok(())
    }
}

pub fn pending(directory: &Path) -> Result<Vec<Request>> {
    let mut markers = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|suffix| suffix == "open") && entry.file_type()?.is_file() {
            markers.push(path);
        }
    }
    markers.sort_unstable();
    Ok(markers
        .into_iter()
        .filter_map(|marker| match read_request(&marker) {
            Ok(file_url) => Some(Request { marker, file_url }),
            Err(error) => {
                log::warn!("Ignoring invalid Superfile launch request: {error:#}");
                let _ = fs::remove_file(marker);
                None
            }
        })
        .collect())
}

fn read_request(marker: &Path) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    File::open(marker)?
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_REQUEST_BYTES,
        "Launch request is too large"
    );
    if bytes.is_empty() {
        return Ok(None);
    }
    let value = String::from_utf8(bytes)?;
    ensure!(
        url::Url::parse(&value)?.to_file_path().is_ok(),
        "Expected a local file URL"
    );
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_launch_forwards_a_file_or_activation_and_releases_ownership() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let Launch::Primary(instance, _) = launch_at(directory.path(), None)? else {
            anyhow::bail!("First launch did not acquire ownership");
        };
        let path = directory.path().join("文件 #1.md");
        for value in [Some(path.clone()), None] {
            let dir = directory.path().to_owned();
            let expected = value.clone();
            let spawned = std::thread::spawn(move || {
                launch_at(&dir, value).map(|launch| matches!(launch, Launch::Forwarded))
            });
            let deadline = Instant::now() + Duration::from_secs(3);
            let request = loop {
                if let Some(request) = pending(directory.path())?.pop() {
                    break request;
                }
                ensure!(Instant::now() < deadline, "Launch request did not arrive");
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(
                request
                    .file_url
                    .as_deref()
                    .and_then(|value| url::Url::parse(value).ok())
                    .and_then(|url| url.to_file_path().ok()),
                expected,
            );
            request.acknowledge()?;
            assert!(spawned.join().expect("second launch thread")?);
        }
        drop(instance);
        assert!(matches!(
            launch_at(directory.path(), None)?,
            Launch::Primary(..)
        ));
        Ok(())
    }

    #[test]
    fn launch_takes_over_during_shutdown_and_keeps_the_forwarded_request() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let Launch::Primary(instance, _) = launch_at(directory.path(), None)? else {
            anyhow::bail!("First launch did not acquire ownership");
        };
        let path = directory.path().join("folder");
        let dir = directory.path().to_owned();
        let spawned = std::thread::spawn(move || launch_at(&dir, Some(path)));
        let deadline = Instant::now() + Duration::from_secs(3);
        while pending(directory.path())?.is_empty() {
            ensure!(Instant::now() < deadline, "Launch request did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(instance);
        let Launch::Primary(_instance, None) = spawned.join().expect("takeover thread")? else {
            anyhow::bail!("Pending request should transfer to the new owner");
        };
        assert_eq!(pending(directory.path())?.len(), 1);
        Ok(())
    }
}
