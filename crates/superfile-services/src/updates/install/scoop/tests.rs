use super::*;

fn installation(root: &Path) -> PathBuf {
    let previous = root.join("scoop/apps/superfile/1.2.2");
    fs::create_dir_all(&previous).unwrap();
    fs::write(previous.join("superfile.exe"), b"old executable").unwrap();
    fs::write(
        previous.join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": "1.2.2", "bin": "superfile.exe", "description": "Superfile",
            "architecture": {"64bit": {"url": "old.zip", "hash": "old hash"}},
            "shortcuts": [["superfile.exe", "Superfile"]],
            "checkver": {"jsonpath": "$.version"}, "autoupdate": {"url": "$version.zip"}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        previous.join("install.json"),
        br#"{"architecture":"64bit","bucket":"hp","hold":true}"#,
    )
    .unwrap();
    junction::create(&previous, previous.parent().unwrap().join("current")).unwrap();
    previous.join("superfile.exe").canonicalize().unwrap()
}

fn prepared(root: &Path, target: PathBuf) -> PreparedUpdate {
    let release = super::super::super::tests::release("v1.2.3");
    let artifact = super::super::tests::archive(root, &release.asset.name);
    prepare_at(&artifact, &release, target, true, &CancelToken::new()).unwrap()
}

#[test]
fn scoop_update_switches_current_and_metadata_without_touching_locked_old_version() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = tempfile::tempdir().unwrap();
    let target = installation(directory.path());
    // No sharing, including DELETE. Updating must not rename the running image.
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&target)
        .unwrap();
    let update = prepared(directory.path(), target.clone());
    assert!(
        update.install_blocker.is_none(),
        "{:?}",
        update.install_blocker
    );
    assert!(blocker(&target, false).is_some());
    let scoop = update.scoop.as_ref().unwrap();
    let root = scoop.root.clone();
    let old_manifest = fs::read(target.parent().unwrap().join("manifest.json")).unwrap();
    assert_eq!(update.staging.path().parent(), Some(root.as_path()));
    let installed = scoop
        .commit(update.staging.path(), &update.version)
        .unwrap();
    assert_eq!(installed.backup, target);
    assert_eq!(fs::read(&installed.executable).unwrap(), b"new executable");
    let current = root.join("current");
    assert_eq!(
        current.canonicalize().unwrap(),
        root.join("1.2.3").canonicalize().unwrap()
    );
    let manifest: Value =
        serde_json::from_slice(&fs::read(current.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["version"], "1.2.3");
    assert_eq!(
        manifest["architecture"]["64bit"]["url"],
        super::super::super::tests::release("v1.2.3").asset.url
    );
    assert_eq!(manifest["architecture"]["64bit"]["hash"], "a".repeat(64));
    assert_eq!(manifest["bin"], "superfile.exe");
    assert_eq!(manifest["shortcuts"][0][0], "superfile.exe");
    assert_eq!(
        fs::read(current.join("install.json")).unwrap(),
        scoop.receipt
    );
    assert_eq!(
        fs::read(target.parent().unwrap().join("manifest.json")).unwrap(),
        old_manifest
    );
    update.discard();
    drop(locked);
    assert_eq!(fs::read(target).unwrap(), b"old executable");
    assert_eq!(
        fs::read(current.join("superfile.exe")).unwrap(),
        b"new executable"
    );
}

#[test]
fn scoop_refuses_collisions_concurrent_installs_and_changed_receipts() {
    let directory = tempfile::tempdir().unwrap();
    let target = installation(directory.path());
    let update = prepared(directory.path(), target.clone());
    let scoop = update.scoop.as_ref().unwrap();
    let destination = scoop.root.join("1.2.3");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep"), "existing version").unwrap();
    assert!(
        scoop
            .commit(update.staging.path(), &update.version)
            .is_err()
    );
    assert_eq!(
        fs::read(destination.join("keep")).unwrap(),
        b"existing version"
    );
    fs::remove_file(destination.join("keep")).unwrap();
    fs::remove_dir(&destination).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(scoop.root.join(".superfile-update.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert!(
        scoop
            .commit(update.staging.path(), &update.version)
            .is_err()
    );
    drop(lock);
    fs::write(
        target.parent().unwrap().join("install.json"),
        br#"{"architecture":"64bit","bucket":"other"}"#,
    )
    .unwrap();
    assert!(
        scoop
            .commit(update.staging.path(), &update.version)
            .is_err()
    );
    assert!(!destination.exists());
    assert_eq!(
        scoop.root.join("current").canonicalize().unwrap(),
        target.parent().unwrap()
    );
    assert_eq!(fs::read(target).unwrap(), b"old executable");
}

#[test]
fn scoop_activation_failure_restores_current_and_keeps_payload_for_retry() {
    let directory = tempfile::tempdir().unwrap();
    let target = installation(directory.path());
    let update = prepared(directory.path(), target.clone());
    let scoop = update.scoop.as_ref().unwrap();
    let staging = update.staging.path();
    let destination = scoop.root.join("1.2.3");
    let error = scoop
        .publish(
            &staging.join("payload"),
            &destination,
            &scoop.root.join("current"),
            &staging.join("missing-next-current"),
            &staging.join("previous-current"),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("original version restored"));
    assert_eq!(
        scoop.root.join("current").canonicalize().unwrap(),
        target.parent().unwrap()
    );
    assert!(!destination.exists());
    assert!(update.payload.is_file());
    let installed = scoop.commit(staging, &update.version).unwrap();
    assert_eq!(fs::read(installed.executable).unwrap(), b"new executable");
}

#[test]
fn scoop_rejects_custom_installers_and_inactive_or_unlinked_versions() {
    let directory = tempfile::tempdir().unwrap();
    let target = installation(directory.path());
    let manifest = target.parent().unwrap().join("manifest.json");
    let original = fs::read(&manifest).unwrap();
    let mut custom: Value = serde_json::from_slice(&original).unwrap();
    custom["post_install"] = "custom script".into();
    fs::write(&manifest, serde_json::to_vec(&custom).unwrap()).unwrap();
    assert!(blocker(&target, true).unwrap().contains("scoop update"));
    fs::write(manifest, original).unwrap();
    let update = prepared(directory.path(), target.clone());
    fs::remove_dir(target.parent().unwrap().parent().unwrap().join("current")).unwrap();
    assert!(blocker(&target, true).is_some());
    assert!(
        update
            .scoop
            .as_ref()
            .unwrap()
            .commit(update.staging.path(), &update.version)
            .is_err()
    );
    assert_eq!(fs::read(target).unwrap(), b"old executable");
}

#[test]
fn scoop_junction_sharing_failure_keeps_current_and_allows_retry() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = tempfile::tempdir().unwrap();
    let target = installation(directory.path());
    let update = prepared(directory.path(), target.clone());
    let scoop = update.scoop.as_ref().unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        // Open the directory junction itself, without following its target.
        .custom_flags(0x02000000 | 0x00200000)
        .open(scoop.root.join("current"))
        .unwrap();
    assert!(
        scoop
            .commit(update.staging.path(), &update.version)
            .is_err()
    );
    assert!(target.is_file() && update.payload.is_file());
    assert!(!scoop.root.join("1.2.3").exists());
    drop(locked);
    assert_eq!(
        scoop.root.join("current").canonicalize().unwrap(),
        target.parent().unwrap()
    );
    assert!(scoop.commit(update.staging.path(), &update.version).is_ok());
}

#[test]
fn cancelling_or_discarding_a_scoop_update_leaves_installation_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let target = installation(directory.path());
    let update = prepared(directory.path(), target.clone());
    let root = update.scoop.as_ref().unwrap().root.clone();
    let staging = update.staging.path().to_owned();
    update.discard();
    assert!(!staging.exists());
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(
        prepare_at(
            Path::new("missing.zip"),
            &super::super::super::tests::release("v1.2.3"),
            target.clone(),
            true,
            &cancel
        )
        .is_err()
    );
    assert_eq!(
        root.join("current").canonicalize().unwrap(),
        target.parent().unwrap()
    );
    assert!(!root.join("1.2.3").exists());
    assert_eq!(fs::read(target).unwrap(), b"old executable");
}
