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
fn preparation_leaves_original_intact_and_swap_preserves_backup() {
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
    let installed = swap(&update.payload, &target).unwrap();
    assert_eq!(fs::read(&installed.executable).unwrap(), b"new executable");
    assert_eq!(
        fs::read(executable(&installed.backup)).unwrap(),
        b"original executable"
    );
    // The public continuation cannot replace a fixture or any other application.
    assert!(commit(&update).is_err());
}

#[test]
fn failed_swap_restores_original_and_cancel_never_stages() {
    let directory = tempfile::tempdir().unwrap();
    let target = old_target(directory.path());
    let error = swap(&directory.path().join("missing-payload"), &target).unwrap_err();
    assert!(format!("{error:#}").contains("restored"));
    assert_eq!(
        fs::read(executable(&target)).unwrap(),
        b"original executable"
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
