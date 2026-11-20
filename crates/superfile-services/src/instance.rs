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
    Primary(Instance, Vec<String>),
    Forwarded,
}

pub struct Instance {
    directory: PathBuf,
    // Keep this inode in place. Removing it would allow two processes to lock
    // different files, and the OS releases its lock when the owner exits.
    _lock: File,
}

fn validate_url(value: &str) -> Result<url::Url> {
    let url = url::Url::parse(value)?;
    ensure!(
        (url.scheme() == "file" && url.to_file_path().is_ok())
            || (url.scheme() == "superfile"
                && matches!(url.host_str(), Some("open-workspace" | "open-remote"))),
        "Unsupported launch URL"
    );
    Ok(url)
}

pub fn launch_url(argument: &Path) -> Result<String> {
    if let Some(value) = argument.to_str()
        && value.split_once("://").is_some_and(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("file") || scheme.eq_ignore_ascii_case("superfile")
        })
    {
        return Ok(validate_url(value)?.to_string());
    }
    url::Url::from_file_path(std::path::absolute(argument)?)
        .map(String::from)
        .map_err(|_| anyhow::anyhow!("Cannot open file path: {}", argument.display()))
}

pub fn launch(urls: Vec<String>) -> Result<Launch> {
    launch_at(&crate::platform::data_directory()?.join("instance"), urls)
        .context("Cannot start or contact the Superfile instance")
}

fn launch_at(directory: &Path, urls: Vec<String>) -> Result<Launch> {
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
    let primary = |lock, urls| {
        Launch::Primary(
            Instance {
                directory: directory.to_owned(),
                _lock: lock,
            },
            urls,
        )
    };
    if try_lock(&lock)? {
        return Ok(primary(lock, urls));
    }

    let marker = enqueue(directory, &urls)?;
    let deadline = Instant::now() + FORWARD_WAIT;
    while marker.try_exists()? {
        if try_lock(&lock)? {
            // The former owner was exiting. Drain the existing request on
            // startup if it was not acknowledged before the lock passed to us.
            return if marker.try_exists()? {
                Ok(primary(lock, Vec::new()))
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

fn enqueue(directory: &Path, urls: &[String]) -> Result<PathBuf> {
    // Keep the empty activation and single-URL formats used by older versions.
    let bytes = urls.join("\n").into_bytes();
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
    temporary.write_all(&bytes)?;
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
    pub urls: Vec<String>,
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
            Ok(urls) => Some(Request { marker, urls }),
            Err(error) => {
                log::warn!("Ignoring invalid Superfile launch request: {error:#}");
                let _ = fs::remove_file(marker);
                None
            }
        })
        .collect())
}

fn read_request(marker: &Path) -> Result<Vec<String>> {
    let mut bytes = Vec::new();
    File::open(marker)?
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_REQUEST_BYTES,
        "Launch request is too large"
    );
    std::str::from_utf8(&bytes)?
        .lines()
        .map(|value| validate_url(value).map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_and_protocol_arguments_survive_cold_start_and_queue_decoding() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("two words #1.txt");
        let url = launch_url(&file)?;
        let remote = "superfile://open-remote?request=%7B%22root%22%3A%22%2Fsrv%22%7D";
        let targets = vec![launch_url(Path::new(&url))?, launch_url(Path::new(remote))?];
        let Launch::Primary(_owner, urls) = launch_at(dir.path(), targets.clone())? else {
            anyhow::bail!("Expected a new primary instance");
        };
        assert_eq!(urls, [url, remote.to_owned()]);
        assert_eq!(read_request(&enqueue(dir.path(), &targets)?)?, targets);
        let invalid = dir.path().join("invalid.open");
        fs::write(&invalid, "https://example.org")?;
        assert!(read_request(&invalid).is_err());
        Ok(())
    }

    #[test]
    fn second_launch_forwards_a_file_or_activation_and_releases_ownership() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let Launch::Primary(instance, _) = launch_at(directory.path(), vec![])? else {
            anyhow::bail!("First launch did not acquire ownership");
        };
        let path = directory.path().join("文件 #1.md");
        let file = launch_url(&path)?;
        let remote = launch_url(Path::new("superfile://open-remote?request=%7B%7D"))?;
        for value in [vec![file.clone()], vec![], vec![file, remote]] {
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
            assert_eq!(request.urls, expected);
            request.acknowledge()?;
            assert!(spawned.join().expect("second launch thread")?);
        }
        drop(instance);
        assert!(matches!(
            launch_at(directory.path(), vec![])?,
            Launch::Primary(..)
        ));
        Ok(())
    }

    #[test]
    fn launch_takes_over_during_shutdown_and_keeps_the_forwarded_request() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let Launch::Primary(instance, _) = launch_at(directory.path(), vec![])? else {
            anyhow::bail!("First launch did not acquire ownership");
        };
        let path = directory.path().join("folder");
        let dir = directory.path().to_owned();
        let urls = vec![launch_url(&path)?];
        let spawned = std::thread::spawn(move || launch_at(&dir, urls));
        let deadline = Instant::now() + Duration::from_secs(3);
        while pending(directory.path())?.is_empty() {
            ensure!(Instant::now() < deadline, "Launch request did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(instance);
        let Launch::Primary(_instance, urls) = spawned.join().expect("takeover thread")? else {
            anyhow::bail!("Pending request should transfer to the new owner");
        };
        assert!(urls.is_empty());
        assert_eq!(pending(directory.path())?.len(), 1);
        Ok(())
    }
}
