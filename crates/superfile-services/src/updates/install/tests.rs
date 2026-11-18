use super::*;
use std::io::Cursor;

fn tar_entry(
    builder: &mut tar::Builder<flate2::write::GzEncoder<fs::File>>,
    name: &str,
    bytes: &[u8],
) {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(&mut header, name, bytes).unwrap();
}

fn archive(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    if cfg!(windows) {
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file("superfile.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"new executable").unwrap();
        zip.finish().unwrap();
    } else {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        ));
        if cfg!(target_os = "macos") {
            tar_entry(
                &mut tar,
                "Superfile.app/Contents/MacOS/superfile",
                b"new executable",
            );
            tar_entry(&mut tar, "Superfile.app/Contents/Info.plist", br#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>app.superfile.desktop</string><key>CFBundleExecutable</key><string>superfile</string><key>CFBundleShortVersionString</key><string>1.2.3</string></dict></plist>"#);
        } else {
            tar_entry(
                &mut tar,
                &format!("{}/superfile", name.strip_suffix(".tar.gz").unwrap()),
                b"new executable",
            );
        }
        tar.into_inner().unwrap().finish().unwrap();
    }
    path
}

fn old_target(root: &Path) -> PathBuf {
    let target = root.join(if cfg!(target_os = "macos") {
        "Superfile.app"
    } else {
        "superfile"
    });
    if cfg!(target_os = "macos") {
        fs::create_dir_all(target.join("Contents/MacOS")).unwrap();
    }
    fs::write(executable(&target), b"original executable").unwrap();
    target
}

fn executable(target: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        target.join("Contents/MacOS/superfile")
    } else {
        target.to_owned()
    }
}

#[test]
fn update_backup_survives_replacement_until_new_version_is_running() {
    let directory = tempfile::tempdir().unwrap();
    let release = super::super::tests::release("v1.2.3");
    let artifact = archive(directory.path(), &release.asset.name);
    let target = old_target(directory.path());
    let update = prepare_at(
        &artifact,
        &release,
        target.clone(),
        true,
        &CancelToken::new(),
    )
    .unwrap();
    assert!(
        update.install_blocker.is_none(),
        "{:?}",
        update.install_blocker
    );
    assert_eq!(update.staging.path().parent(), target.parent());
    assert_eq!(
        fs::read(executable(&target)).unwrap(),
        b"original executable"
    );
    let installed = swap(&update.payload, &target, &release.version).unwrap();
    assert_eq!(fs::read(&installed.executable).unwrap(), b"new executable");
    assert_eq!(
        fs::read(executable(&installed.backup)).unwrap(),
        b"original executable"
    );
    // The public continuation cannot replace a fixture or any other application.
    assert!(commit(&update).is_err());
    let staging = update.staging.path().to_owned();
    fs::write(staging.join(".DS_Store"), "Finder metadata").unwrap();
    update.discard();
    assert!(!staging.exists());

    let backup_root = installed.backup.parent().unwrap();
    let receipt = backup_root.join(BACKUP_RECEIPT);
    let name = target.file_name().unwrap();
    cleanup_backup(backup_root, name, &Version::parse("1.2.2").unwrap()).unwrap();
    assert!(receipt.is_file() && installed.backup.exists());
    cleanup_backup(backup_root, "OtherSuperfile".as_ref(), &release.version).unwrap();
    assert!(receipt.is_file() && installed.backup.exists());

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&installed.backup)
            .unwrap();
        assert!(cleanup_after_startup(&target, &release.version, SystemTime::now()).is_err());
        assert!(receipt.is_file() && installed.backup.exists());
        drop(locked);
    }
    cleanup_after_startup(&target, &release.version, SystemTime::now()).unwrap();
    assert!(!backup_root.exists());
    assert_eq!(fs::read(&installed.executable).unwrap(), b"new executable");
    cleanup_after_startup(&target, &release.version, SystemTime::now()).unwrap();
}

#[test]
fn failed_swap_restores_original_and_cancel_never_stages() {
    let directory = tempfile::tempdir().unwrap();
    let target = old_target(directory.path());
    let error = swap(
        &directory.path().join("missing-payload"),
        &target,
        &Version::parse("1.2.3").unwrap(),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("restored"));
    assert_eq!(
        fs::read(executable(&target)).unwrap(),
        b"original executable"
    );
    assert!(
        !fs::read_dir(directory.path())
            .unwrap()
            .any(|entry| is_backup_dir(&entry.unwrap().path()))
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(
        prepare_at(
            Path::new("missing.zip"),
            &super::super::tests::release("v1.2.3"),
            target,
            true,
            &cancel
        )
        .is_err()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn legacy_cleanup_removes_only_matching_older_superfile_bundles() {
    use std::os::unix::fs::PermissionsExt;

    fn bundle(parent: &Path, version: &str, id: &str) -> PathBuf {
        let target = old_target(parent);
        let mut info = plist_parser::Dictionary::new();
        for (key, value) in [
            ("CFBundleIdentifier", id),
            ("CFBundleExecutable", "superfile"),
            ("CFBundleShortVersionString", version),
        ] {
            info.insert(key.into(), plist_parser::Value::String(value.into()));
        }
        plist_parser::Value::Dictionary(info)
            .to_file_xml(target.join("Contents/Info.plist"))
            .unwrap();
        fs::set_permissions(executable(&target), fs::Permissions::from_mode(0o755)).unwrap();
        target
    }

    let directory = tempfile::tempdir().unwrap();
    let id = "app.superfile.desktop";
    let target = bundle(directory.path(), "1.2.3", id);
    let old_dir = directory.path().join(".superfile-previous-Old123");
    let old = bundle(&old_dir, "1.2.2", id);
    let other = bundle(
        &directory.path().join(".superfile-previous-Bad123"),
        "1.2.2",
        "app.superfile.icon-preview",
    );
    let newer = bundle(
        &directory.path().join(".superfile-previous-New123"),
        "1.2.4",
        id,
    );
    let outside = tempfile::tempdir().unwrap();
    let outside_app = bundle(outside.path(), "1.2.2", id);
    let link = directory.path().join(".superfile-previous-Lnk123");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    let unrelated = old_dir.join("keep.txt");
    fs::write(&unrelated, "unrelated file").unwrap();

    cleanup_after_startup(
        &target,
        &Version::parse("1.2.3").unwrap(),
        SystemTime::now(),
    )
    .unwrap();
    assert!(!old.exists());
    for preserved in [target, other, newer, outside_app, link, unrelated] {
        assert!(preserved.exists());
    }
}

#[test]
fn startup_removes_stale_staging_shells_and_preserves_active_or_nonempty_stages() {
    let directory = tempfile::tempdir().unwrap();
    let release = super::super::tests::release("v1.2.3");
    let artifact = archive(directory.path(), &release.asset.name);
    let target = old_target(directory.path());
    let prepared = prepare_at(
        &artifact,
        &release,
        target.clone(),
        true,
        &CancelToken::new(),
    )
    .unwrap();
    let installed = swap(&prepared.payload, &target, &release.version).unwrap();
    prepared.discard();
    let root = directory.path();
    let empty = root.join(".superfile-update-Abc123");
    let finder = root.join(".superfile-update-Def456");
    let active = root.join(".superfile-update-Ghi789");
    let extracting = root.join(".superfile-update-Act123");
    let unrelated = root.join(".superfile-update-unrelated");
    for path in [&empty, &finder, &active, &unrelated] {
        fs::create_dir(path).unwrap();
    }
    fs::create_dir_all(extracting.join("payload")).unwrap();
    fs::write(finder.join(".DS_Store"), "Finder metadata").unwrap();
    fs::write(active.join("update.bin"), "pending payload").unwrap();
    fs::write(active.join(".DS_Store"), "Finder metadata").unwrap();
    #[cfg(unix)]
    {
        let symlink = root.join(".superfile-update-Lnk123");
        std::os::unix::fs::symlink(&unrelated, symlink).unwrap();
        let metadata_link = root.join(".superfile-update-Lnk456");
        fs::create_dir(&metadata_link).unwrap();
        std::os::unix::fs::symlink(active.join(".DS_Store"), metadata_link.join(".DS_Store"))
            .unwrap();
    }
    let now = SystemTime::now();
    cleanup_after_startup(&target, &release.version, now).unwrap();
    assert!(empty.is_dir() && finder.is_dir());

    let later = now + STAGING_MIN_AGE + Duration::from_secs(1);
    cleanup_after_startup(&target, &release.version, later).unwrap();
    cleanup_after_startup(&target, &release.version, later).unwrap();
    assert!(!empty.exists() && !finder.exists());
    assert!(active.join("update.bin").is_file() && active.join(".DS_Store").is_file());
    assert!(extracting.join("payload").is_dir());
    assert!(unrelated.is_dir());
    #[cfg(unix)]
    for path in [
        root.join(".superfile-update-Lnk123"),
        root.join(".superfile-update-Lnk456/.DS_Store"),
    ] {
        assert!(fs::symlink_metadata(path).unwrap().is_symlink());
    }
    assert!(installed.executable.is_file());
}

#[test]
fn extraction_rejects_zip_traversal_symlinks_and_duplicate_entries() {
    for (name, link, duplicate) in [
        ("../escape", false, false),
        ("link", true, false),
        ("same", false, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unsafe.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        if link {
            zip.add_symlink(name, "outside", options).unwrap();
        } else {
            zip.start_file(name, options).unwrap();
            zip.write_all(b"entry").unwrap();
            if duplicate {
                zip.start_file("./same", options).unwrap();
                zip.write_all(b"duplicate").unwrap();
            }
        }
        zip.finish().unwrap();
        let dest = directory.path().join("extracted");
        fs::create_dir(&dest).unwrap();
        assert!(extract(&path, &dest, &CancelToken::new()).is_err());
        assert!(!directory.path().join("escape").exists());
    }
}

#[test]
fn extraction_rejects_tar_links_and_unsafe_paths() {
    for kind in [
        tar::EntryType::Symlink,
        tar::EntryType::Link,
        tar::EntryType::Fifo,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unsafe.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_link_name("outside").unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "unsafe", std::io::empty())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        assert!(extract(&path, directory.path(), &CancelToken::new()).is_err());
    }
    for path in ["../escape", "/absolute", "foo/../../escape", "foo\\bar"] {
        assert!(safe_relative(Path::new(path)).is_err());
    }
}

#[test]
fn extraction_enforces_expanded_size_truncation_and_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("entry");
    assert!(
        copy_entry(
            &mut Cursor::new(b"x"),
            &path,
            MAX_EXPANDED + 1,
            &mut 0,
            &CancelToken::new()
        )
        .is_err()
    );
    assert!(!path.exists());
    assert!(
        copy_entry(
            &mut Cursor::new(b"x"),
            &path,
            2,
            &mut 0,
            &CancelToken::new()
        )
        .is_err()
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(
        copy_entry(
            &mut Cursor::new(b"x"),
            &directory.path().join("cancelled"),
            1,
            &mut 0,
            &cancel
        )
        .is_err()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn bundle_version_must_match_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let release = super::super::tests::release("v1.2.3");
    let artifact = archive(directory.path(), &release.asset.name);
    extract(&artifact, directory.path(), &CancelToken::new()).unwrap();
    let payload = payload(directory.path(), &release.asset.name).unwrap();
    validate_payload(&payload, &release.version).unwrap();
    assert!(validate_payload(&payload, &Version::parse("2.0.0").unwrap()).is_err());
    fs::write(payload.join("Contents/Info.plist"), b"invalid plist").unwrap();
    assert!(validate_payload(&payload, &release.version).is_err());
}
