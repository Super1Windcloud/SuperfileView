//! Application-owned SQLite state. All entry points are blocking services.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod migration;
pub(crate) mod sessions;

const DATABASE_NAME: &str = "superfile.sqlite3";
const SCHEMA_VERSION: i64 = 3;

pub fn initialize() -> Result<()> {
    let directory = crate::preferences::directory()?;
    initialize_at(&directory)
}

// Called only by the desktop instance owner, before opening any live SQLite
// connections. Other storage entry points may already have active connections.
fn initialize_at(directory: &Path) -> Result<()> {
    preflight(directory)?;
    let mut connection = open_at(directory)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    migration::archive_sources(&tx, directory)?;
    tx.commit()?;
    Ok(())
}

fn preflight(directory: &Path) -> Result<()> {
    let mut has_sidecars = false;
    for suffix in ["-wal", "-shm", "-journal"] {
        has_sidecars |= directory
            .join(format!("{DATABASE_NAME}{suffix}"))
            .try_exists()?;
    }
    let path = directory.join(DATABASE_NAME);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() == 0 && !has_sidecars => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !has_sidecars => {
            return Ok(());
        }
        Err(error) => return Err(error).context("Cannot open Superfile application database"),
        Ok(metadata) => ensure!(
            metadata.is_file() && crate::sqlite::has_header(&path),
            "Invalid Superfile application database header"
        ),
    }
    if has_sidecars {
        // Opening a damaged database can checkpoint or remove its sidecars,
        // even on error. Let SQLite recover and check the private copy first.
        let snapshot = crate::sqlite::prepare(&path)?;
        let db = Connection::open(
            snapshot
                .database
                .as_ref()
                .context("Missing database snapshot")?,
        )?;
        let result: String = db.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        ensure!(
            result == "ok",
            "Superfile application database is damaged: {result}"
        );
        schema_version(&db)?;
    }
    Ok(())
}
pub(crate) fn open() -> Result<Connection> {
    open_at(&crate::preferences::directory()?)
}
pub(crate) fn open_at(directory: &Path) -> Result<Connection> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(DATABASE_NAME);
    if !path.try_exists()? {
        // Close the private temporary file before publishing it. Opening and
        // closing the live file outside SQLite can release its POSIX locks.
        let empty = tempfile::NamedTempFile::new_in(directory)?.into_temp_path();
        if let Err(error) = empty.persist_noclobber(&path)
            && error.error.kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(error).context("Cannot create private Superfile application database");
        }
    }
    let mut db = Connection::open(&path).context("Cannot open Superfile application database")?;
    db.busy_timeout(std::time::Duration::from_secs(10))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    if schema_version(&db)? < SCHEMA_VERSION {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            tx.execute_batch(include_str!("storage/schema.sql"))?;
            migration::import(&tx, directory).context(
                "Cannot migrate existing Superfile data; original files have been preserved",
            )?;
        } else if version < SCHEMA_VERSION {
            if version == 1 {
                tx.execute_batch(
                    "INSERT INTO records(kind,id,payload)
                SELECT 'migration-source',path,'' FROM imported_files;
                DROP TABLE imported_files;",
                )?;
            }
            tx.execute_batch(
                "ALTER TABLE sessions ADD COLUMN window_active INTEGER NOT NULL DEFAULT 0 CHECK(window_active IN (0,1));
                ALTER TABLE sessions ADD COLUMN show_home INTEGER NOT NULL DEFAULT 0 CHECK(show_home IN (0,1));",
            )?;
        }
        tx.execute_batch("PRAGMA user_version=3")?;
        tx.commit()?;
    }
    let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        db.pragma_update(None, "journal_mode", "WAL")?;
    }
    Ok(db)
}

fn schema_version(db: &Connection) -> Result<i64> {
    let version = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    ensure!(
        version <= SCHEMA_VERSION,
        "Superfile database was created by a newer application version"
    );
    Ok(version)
}
pub(crate) fn setting(db: &Connection, key: &str) -> Result<Option<String>> {
    Ok(db
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()?)
}
pub(crate) fn set_setting(db: &Connection, key: &str, value: &str) -> Result<()> {
    db.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (key,value))?;
    Ok(())
}
pub(crate) fn record(db: &Connection, kind: &str, id: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT payload FROM records WHERE kind=?1 AND id=?2",
            (kind, id),
            |r| r.get(0),
        )
        .optional()?)
}
pub(crate) fn set_record(db: &Connection, kind: &str, id: &str, payload: &str) -> Result<()> {
    db.execute("INSERT INTO records(kind,id,payload) VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload", (kind,id,payload))?;
    Ok(())
}
pub(crate) fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .context("Application state contains a non-UTF-8 path")
}
pub(crate) fn paths(db: &Connection, scope: &str, owner: &str) -> Result<Vec<PathBuf>> {
    Ok(db
        .prepare("SELECT path FROM paths WHERE scope=?1 AND owner=?2 ORDER BY position")?
        .query_map((scope, owner), |r| {
            Ok(PathBuf::from(r.get::<_, String>(0)?))
        })?
        .collect::<rusqlite::Result<_>>()?)
}
pub(crate) fn save_paths(
    db: &Connection,
    scope: &str,
    owner: &str,
    paths: &[PathBuf],
) -> Result<()> {
    db.execute(
        "DELETE FROM paths WHERE scope=?1 AND owner=?2",
        (scope, owner),
    )?;
    let mut insert =
        db.prepare("INSERT INTO paths(scope,owner,position,path) VALUES(?1,?2,?3,?4)")?;
    for (index, path) in paths.iter().enumerate() {
        insert.execute((scope, owner, index as i64, path_text(path)?))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use superfile_core::session::{SessionTab, WorkspaceSession};

    #[test]
    fn failed_initialization_keeps_corrupt_database_and_sidecars() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let files = [
            ("superfile.sqlite3", b"not a SQLite database".as_slice()),
            ("superfile.sqlite3-wal", b"fixture WAL"),
            ("superfile.sqlite3-shm", b"fixture SHM"),
        ];
        for (name, data) in files {
            fs::write(directory.path().join(name), data)?;
        }
        assert!(initialize_at(directory.path()).is_err());
        for (name, data) in files {
            assert_eq!(fs::read(directory.path().join(name))?, data);
        }
        Ok(())
    }

    #[test]
    fn startup_preflight_checks_committed_wal_without_modifying_it() -> Result<()> {
        let writer_directory = tempfile::tempdir()?;
        let fixture = tempfile::tempdir()?;
        let writer = open_at(writer_directory.path())?;
        writer.execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA user_version=4;")?;
        let names = [
            DATABASE_NAME,
            "superfile.sqlite3-wal",
            "superfile.sqlite3-shm",
        ];
        let copies = names.map(|name| fixture.path().join(name));
        for (name, copy) in names.into_iter().zip(&copies) {
            fs::copy(writer_directory.path().join(name), copy)?;
        }
        drop(writer);
        let before = copies
            .iter()
            .map(fs::read)
            .collect::<std::io::Result<Vec<_>>>()?;
        let error = initialize_at(fixture.path()).unwrap_err();
        assert!(error.to_string().contains("newer application version"));
        for (copy, before) in copies.iter().zip(before) {
            assert_eq!(fs::read(copy)?, before);
        }
        Ok(())
    }

    #[test]
    fn opening_another_wal_connection_preserves_the_active_transaction() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut writer = open_at(directory.path())?;
        assert!(rusqlite::version_number() >= 3_051_003);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = directory
                .path()
                .join("superfile.sqlite3")
                .metadata()?
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let transaction = writer.transaction_with_behavior(TransactionBehavior::Immediate)?;
        set_setting(&transaction, "appearance", "dark")?;
        let reader = open_at(directory.path())?;
        assert_eq!(setting(&reader, "appearance")?, None);
        transaction.commit()?;
        assert_eq!(setting(&reader, "appearance")?.as_deref(), Some("dark"));
        Ok(())
    }

    #[test]
    fn version_two_sessions_gain_selection_without_changing_tabs_or_recent_order() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut db = open_at(directory.path())?;
        let sessions = BTreeMap::from([(
            "window".into(),
            vec![WorkspaceSession {
                roots: vec!["/dev".into()],
                workspace_id: Some("dev".into()),
                ..Default::default()
            }],
        )]);
        let tx = db.transaction()?;
        sessions::save(&tx, "window", &sessions)?;
        tx.execute("INSERT INTO recent_workspaces VALUES('dev',NULL,0)", [])?;
        tx.execute_batch("ALTER TABLE sessions DROP COLUMN window_active; ALTER TABLE sessions DROP COLUMN show_home; PRAGMA user_version=2;")?;
        tx.commit()?;
        drop(db);
        let mut db = open_at(directory.path())?;
        assert_eq!(sessions::load(&db, "window")?, sessions);
        let mut selected = sessions.clone();
        selected.get_mut("window").unwrap()[0].window_active = true;
        selected.get_mut("window").unwrap()[0].show_home = true;
        let tx = db.transaction()?;
        sessions::save(&tx, "window", &selected)?;
        tx.commit()?;
        assert_eq!(sessions::load(&db, "window")?, selected);
        assert_eq!(
            db.query_row(
                "SELECT id FROM recent_workspaces ORDER BY position",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "dev"
        );
        assert_eq!(
            db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?,
            "ok"
        );
        Ok(())
    }

    #[test]
    fn legacy_migration_preserves_window_order_duplicate_roots_and_is_not_reimported() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let first = WorkspaceSession {
            roots: vec!["/项目".into(), "/项目".into(), "/项目/子目录".into()],
            tabs: vec![
                SessionTab::File {
                    path: "/项目/a.md".into(),
                    preview: true,
                },
                SessionTab::Playlist {
                    path: "/音乐".into(),
                    shuffle: true,
                },
            ],
            active: Some(1),
            workspace_id: Some("one".into()),
            name: Some("研究".into()),
            window_group: Some("window".into()),
            window_active: true,
            show_home: true,
        };
        let windows = BTreeMap::from([
            (
                "window".to_owned(),
                vec![first.clone(), WorkspaceSession::default()],
            ),
            ("empty".to_owned(), vec![]),
        ]);
        std::fs::write(
            directory.path().join("window-sessions.json"),
            serde_json::to_vec(&windows)?,
        )?;
        std::fs::write(directory.path().join("word-wrap"), "true\n")?;
        let db = open_at(directory.path())?;
        assert_eq!(sessions::load(&db, "window")?, windows);
        assert_eq!(setting(&db, "word-wrap")?.as_deref(), Some("true"));
        set_setting(&db, "word-wrap", "false")?;
        drop(db);
        let db = open_at(directory.path())?;
        assert_eq!(setting(&db, "word-wrap")?.as_deref(), Some("false"));
        assert!(directory.path().join("window-sessions.json").is_file());
        migration::archive_sources(&db, directory.path())?;
        assert!(
            directory
                .path()
                .join("LegacyMigration/window-sessions.json")
                .is_file()
        );
        assert!(db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok");
        Ok(())
    }
    #[test]
    fn malformed_legacy_state_rolls_back_and_can_be_retried_without_losing_sources() -> Result<()> {
        let directory = tempfile::tempdir()?;
        std::fs::write(directory.path().join("word-wrap"), "true")?;
        let source = directory.path().join("workspace-sessions.json");
        std::fs::write(&source, b"broken json")?;
        assert!(open_at(directory.path()).is_err());
        let db = Connection::open(directory.path().join("superfile.sqlite3"))?;
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            0
        );
        assert_eq!(std::fs::read(&source)?, b"broken json");
        drop(db);
        std::fs::write(source, b"{}")?;
        assert_eq!(
            setting(&open_at(directory.path())?, "word-wrap")?.as_deref(),
            Some("true")
        );
        Ok(())
    }
    #[test]
    fn invalid_session_replacement_keeps_the_previous_committed_snapshot() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut db = open_at(directory.path())?;
        let original = BTreeMap::from([("window".into(), vec![WorkspaceSession::default()])]);
        {
            let tx = db.transaction()?;
            sessions::save(&tx, "window", &original)?;
            tx.commit()?;
        }
        {
            let tx = db.transaction()?;
            let invalid = BTreeMap::from([(
                "window".into(),
                vec![WorkspaceSession {
                    active: Some(0),
                    ..Default::default()
                }],
            )]);
            assert!(sessions::save(&tx, "window", &invalid).is_err());
        }
        assert_eq!(sessions::load(&db, "window")?, original);
        Ok(())
    }
}
