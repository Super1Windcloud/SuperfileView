//! Materialize a document for desktop sharing without modifying its source.
use crate::{files, paged::PageEdit, platform, work::Cancellation};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
    time::Duration,
};

pub enum Contents {
    Local(PathBuf),
    Cached(PathBuf),
    Text(String),
    EditedPage(PageEdit, String),
}

pub struct SharedFile {
    path: PathBuf,
    snapshot: Option<tempfile::TempDir>,
}

impl SharedFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// File clipboard items and share targets can open the path after we return.
    pub fn keep(self) {
        if let Some(snapshot) = self.snapshot {
            let _ = snapshot.keep();
        }
    }
}

impl Contents {
    pub fn prepare(self, name: &OsStr, cancel: &Cancellation) -> Result<SharedFile> {
        cancel.check()?;
        if let Self::Local(path) = self {
            let path = std::path::absolute(path)?;
            ensure!(
                path.metadata()?.is_file(),
                "The source is not a regular file"
            );
            return Ok(SharedFile {
                path,
                snapshot: None,
            });
        }
        self.prepare_in(
            &platform::cache_directory()?.join("shared-files"),
            name,
            cancel,
        )
    }

    fn prepare_in(
        self,
        directory: &Path,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<SharedFile> {
        use platform::PrivateDirExt;
        let mut components = Path::new(name).components();
        ensure!(
            matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
            "Invalid shared filename"
        );
        std::fs::DirBuilder::new()
            .recursive(true)
            .private_mode(0o700)
            .create(directory)?;
        ensure!(
            std::fs::symlink_metadata(directory)?.file_type().is_dir(),
            "Invalid sharing cache directory"
        );
        prune_snapshots(directory);
        let snapshot = tempfile::Builder::new()
            .prefix("snapshot-")
            .tempdir_in(directory)?;
        let path = snapshot.path().join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let _guard = files::save_lock(cancel)?;
        self.write(&mut file, cancel)?;
        drop(file);
        Ok(SharedFile {
            path,
            snapshot: Some(snapshot),
        })
    }

    pub fn save_copy(
        self,
        selected_path: &Path,
        open_files: &[PathBuf],
        cancel: &Cancellation,
    ) -> Result<PathBuf> {
        let mut protected = open_files.to_vec();
        match &self {
            Self::Local(path) | Self::Cached(path) => protected.push(path.clone()),
            Self::EditedPage(page, _) => protected.push(page.path.clone()),
            Self::Text(_) => {}
        }
        let destination = files::prepare_file_destination(selected_path, &protected, cancel)?;
        files::publish_file(&destination, cancel, |destination| {
            self.write(destination, cancel)
        })
    }

    fn write(&self, output: &mut File, cancel: &Cancellation) -> Result<()> {
        match self {
            Self::Local(path) | Self::Cached(path) => {
                let mut source = File::open(path).context("Cannot open the file to share")?;
                let before = source.metadata()?;
                ensure!(before.is_file(), "The source is not a regular file");
                let identity = platform::file_identity(&source)?;
                let bytes = cancel.copy((&mut source).take(before.len()), output)?;
                let after = File::open(path)?;
                let info = after.metadata()?;
                ensure!(
                    bytes == before.len()
                        && info.len() == before.len()
                        && info.modified().ok() == before.modified().ok()
                        && platform::file_identity(&after)? == identity,
                    "The source file changed while copying"
                );
            }
            Self::Text(text) => {
                cancel.copy(Cursor::new(text.as_bytes()), output)?;
            }
            Self::EditedPage(page, text) => {
                crate::paged::copy_page_edit(page, &page.path, text, output, cancel)?;
            }
        }
        Ok(())
    }
}

fn prune_snapshots(directory: &Path) {
    const LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("snapshot-") {
            continue;
        }
        if std::fs::symlink_metadata(entry.path()).is_ok_and(|metadata| {
            metadata.file_type().is_dir()
                && metadata
                    .modified()
                    .is_ok_and(|time| time.elapsed().is_ok_and(|age| age > LIFETIME))
        }) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_preserves_original_and_other_open_files() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.bin");
        let other = directory.path().join("other.bin");
        let copy = directory.path().join("copy.bin");
        let bytes = [0, 128, 255];
        std::fs::write(&original, bytes).unwrap();
        std::fs::write(&other, b"other").unwrap();
        let content = || Contents::Local(original.clone());
        let cancel = Cancellation::default();

        assert!(content().save_copy(&original, &[], &cancel).is_err());
        assert!(
            content()
                .save_copy(&other, std::slice::from_ref(&other), &cancel)
                .is_err()
        );
        content().save_copy(&copy, &[], &cancel).unwrap();
        assert_eq!(std::fs::read(copy).unwrap(), bytes);
        assert_eq!(std::fs::read(original).unwrap(), bytes);
        assert_eq!(std::fs::read(other).unwrap(), b"other");
    }

    #[test]
    fn snapshots_survive_only_successful_handoffs() {
        let directory = tempfile::tempdir().unwrap();
        let build = || {
            Contents::Text("草稿\nDraft".into())
                .prepare_in(
                    directory.path(),
                    OsStr::new("draft.txt"),
                    &Cancellation::default(),
                )
                .unwrap()
        };
        let pending = build();
        let failed_path = pending.path().to_owned();
        drop(pending);
        assert!(!failed_path.exists());

        let handed_off = build();
        let shared_path = handed_off.path().to_owned();
        handed_off.keep();
        assert_eq!(std::fs::read_to_string(shared_path).unwrap(), "草稿\nDraft");
    }

    #[test]
    fn paged_copy_includes_unedited_content() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.txt");
        let copy = directory.path().join("copy.txt");
        let prefix = format!("{}\n", "a".repeat(crate::paged::PAGE_BYTES as usize));
        let text = format!("{prefix}old\nlast\n");
        std::fs::write(&original, &text).unwrap();
        let page = crate::paged::read_page(&original, prefix.len() as u64, false).unwrap();
        assert_eq!(page.text, "old\nlast\n");
        let edit = PageEdit {
            path: page.path,
            start: page.start,
            end: page.end,
            total: page.total,
            modified: page.modified,
            identity: page.identity,
            expected: page.text,
        };
        Contents::EditedPage(edit, "new\nlast\n".into())
            .save_copy(&copy, &[], &Cancellation::default())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(copy).unwrap(),
            format!("{prefix}new\nlast\n")
        );
        assert_eq!(std::fs::read_to_string(original).unwrap(), text);
    }
}
