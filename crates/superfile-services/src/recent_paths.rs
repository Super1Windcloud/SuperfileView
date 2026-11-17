//! Local file/folder MRU history in the shared application database.
use crate::storage;
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

const CAPACITY: usize = 200;
const SEEDED: &str = "recent-paths-seeded";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
}
impl Kind {
    fn scope(self) -> &'static str {
        match self {
            Self::File => "recent-files",
            Self::Directory => "recent-directories",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
}
impl History {
    pub fn paths(&self, kind: Kind) -> &[PathBuf] {
        match kind {
            Kind::File => &self.files,
            Kind::Directory => &self.directories,
        }
    }
    pub fn remove(&mut self, kind: Kind, path: &Path) {
        self.paths_mut(kind).retain(|entry| entry != path);
    }
    fn paths_mut(&mut self, kind: Kind) -> &mut Vec<PathBuf> {
        match kind {
            Kind::File => &mut self.files,
            Kind::Directory => &mut self.directories,
        }
    }
}

enum Change<'a> {
    Remember(Kind, &'a Path),
    Remove(Kind, &'a Path),
}

pub fn load() -> Result<History> {
    update_at(&mut storage::open()?, None)
}
pub fn remember(kind: Kind, path: &Path) -> Result<History> {
    ensure!(path.is_absolute(), "Recent paths must be absolute");
    storage::path_text(path)?;
    update_at(&mut storage::open()?, Some(Change::Remember(kind, path)))
}
pub fn remove(kind: Kind, path: &Path) -> Result<History> {
    update_at(&mut storage::open()?, Some(Change::Remove(kind, path)))
}

fn update_at(db: &mut rusqlite::Connection, change: Option<Change<'_>>) -> Result<History> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut history = History {
        files: storage::paths(&tx, Kind::File.scope(), "")?,
        directories: storage::paths(&tx, Kind::Directory.scope(), "")?,
    };
    let seed = storage::setting(&tx, SEEDED)?.is_none();
    if seed {
        // Existing workspace order is known; old session tabs provide a one-time fallback
        // for files opened before dedicated MRU tracking existed.
        for workspace in crate::preferences::recent_at(&tx)? {
            if !crate::remote::is_remote_id(&workspace.id) {
                for path in workspace.folders {
                    append(&mut history.directories, path);
                }
            }
        }
        let tabs = tx
            .prepare(
                "SELECT t.path,s.workspace_id FROM session_tabs t JOIN sessions s ON s.id=t.session
             WHERE t.kind='file' ORDER BY s.window_active DESC,s.id DESC,
             CASE WHEN t.position=s.active THEN 0 ELSE 1 END,t.position",
            )?
            .query_map([], |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    row.get::<_, Option<String>>(1)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (path, workspace) in tabs {
            if !workspace
                .as_deref()
                .is_some_and(crate::remote::is_remote_id)
            {
                append(&mut history.files, path);
            }
        }
        storage::set_setting(&tx, SEEDED, "true")?;
    }
    let changed = change.is_some();
    if let Some(change) = change {
        match change {
            Change::Remember(kind, path) => {
                let paths = history.paths_mut(kind);
                paths.retain(|entry| entry != path);
                paths.insert(0, path.to_owned());
                paths.truncate(CAPACITY);
            }
            Change::Remove(kind, path) => history.remove(kind, path),
        }
    }
    if seed || changed {
        for kind in [Kind::File, Kind::Directory] {
            storage::save_paths(&tx, kind.scope(), "", history.paths(kind))?;
        }
    }
    tx.commit()?;
    Ok(history)
}

fn append(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.is_absolute() && paths.len() < CAPACITY && !paths.contains(&path) {
        paths.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mru_is_persistent_separate_bounded_and_transactional() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut db = storage::open_at(directory.path())?;
        let first = directory.path().join("中文 file.txt");
        let second = directory.path().join("second.txt");
        for path in [&first, &second, &first] {
            update_at(&mut db, Some(Change::Remember(Kind::File, path)))?;
        }
        update_at(
            &mut db,
            Some(Change::Remember(Kind::Directory, directory.path())),
        )?;
        drop(db);
        let mut db = storage::open_at(directory.path())?;
        let history = update_at(&mut db, None)?;
        assert_eq!(history.files, [first.clone(), second.clone()]);
        assert_eq!(history.directories, [directory.path()]);
        update_at(&mut db, Some(Change::Remove(Kind::File, &first)))?;
        assert_eq!(update_at(&mut db, None)?.files, [second]);
        for index in 0..CAPACITY + 5 {
            update_at(
                &mut db,
                Some(Change::Remember(
                    Kind::File,
                    &directory.path().join(index.to_string()),
                )),
            )?;
        }
        assert_eq!(update_at(&mut db, None)?.files.len(), CAPACITY);
        assert_eq!(
            db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?,
            "ok"
        );
        Ok(())
    }

    #[test]
    fn existing_local_workspaces_and_tabs_seed_once_without_remote_paths() -> Result<()> {
        use superfile_core::session::{SessionTab, WorkspaceSession};
        let directory = tempfile::tempdir()?;
        let mut db = storage::open_at(directory.path())?;
        let folder = directory.path().join("project");
        let file = folder.join("readme.md");
        let remote_id = "ssh-workspace-test";
        assert!(crate::remote::is_remote_id(remote_id));
        crate::preferences::save_recent_at(
            &db,
            &[
                crate::preferences::RecentWorkspace {
                    id: "local".into(),
                    name: None,
                    folders: vec![folder.clone(), folder.clone()],
                },
                crate::preferences::RecentWorkspace {
                    id: remote_id.into(),
                    name: None,
                    folders: vec![directory.path().join("remote-folder")],
                },
            ],
        )?;
        storage::sessions::save(
            &db,
            "window",
            &std::collections::BTreeMap::from([(
                "test".into(),
                vec![
                    WorkspaceSession {
                        tabs: vec![SessionTab::File {
                            path: file.clone(),
                            preview: true,
                        }],
                        ..Default::default()
                    },
                    WorkspaceSession {
                        workspace_id: Some(remote_id.into()),
                        tabs: vec![SessionTab::File {
                            path: directory.path().join("remote-file"),
                            preview: true,
                        }],
                        ..Default::default()
                    },
                ],
            )]),
        )?;
        let history = update_at(&mut db, None)?;
        assert_eq!(history.files, [file]);
        assert_eq!(
            history.directories.as_slice(),
            std::slice::from_ref(&folder)
        );
        update_at(&mut db, Some(Change::Remove(Kind::Directory, &folder)))?;
        assert!(update_at(&mut db, None)?.directories.is_empty());
        assert_eq!(crate::preferences::recent_at(&db)?.len(), 2);
        Ok(())
    }

    #[test]
    fn concurrent_writers_merge_and_failed_writes_preserve_existing_order() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        let mut db = storage::open_at(directory.path())?;
        update_at(&mut db, None)?;
        let root = directory.path();
        std::thread::scope(|scope| {
            let writers = [&first, &second].map(|path| {
                scope.spawn(move || -> Result<()> {
                    let mut db = storage::open_at(root)?;
                    update_at(&mut db, Some(Change::Remember(Kind::File, path)))?;
                    Ok(())
                })
            });
            for writer in writers {
                writer.join().unwrap()?;
            }
            Ok::<_, anyhow::Error>(())
        })?;
        let before = update_at(&mut db, None)?;
        assert_eq!(before.files.len(), 2);
        assert!(before.files.contains(&first) && before.files.contains(&second));
        db.execute_batch(
            "CREATE TRIGGER reject_history BEFORE INSERT ON paths
            WHEN NEW.scope='recent-files' BEGIN SELECT RAISE(ABORT,'injected failure'); END;",
        )?;
        assert!(update_at(&mut db, Some(Change::Remove(Kind::File, &first))).is_err());
        assert_eq!(update_at(&mut db, None)?, before);
        Ok(())
    }
}
