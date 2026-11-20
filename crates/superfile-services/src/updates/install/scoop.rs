//! Update standard Scoop portable installs without changing shims or old versions.
use super::*;
use gpui_updater_core::install::{Ownership, PackageManager};
use serde_json::Value;

#[cfg(test)]
mod tests;

pub(super) fn is_scoop(owner: &Ownership) -> bool {
    matches!(
        owner,
        Ownership::Managed {
            manager: PackageManager::Scoop,
            ..
        }
    )
}

pub(super) struct Installation {
    pub root: PathBuf,
    previous: PathBuf,
    manifest: Vec<u8>,
    receipt: Vec<u8>,
}

impl Installation {
    pub fn inspect(target: &Path) -> Result<Self> {
        ensure!(
            target
                .file_name()
                .is_some_and(|name| name == "superfile.exe"),
            "Unexpected Scoop executable"
        );
        let previous = target.parent().context("Missing Scoop version directory")?;
        let root = previous
            .parent()
            .context("Missing Scoop application directory")?;
        ensure!(
            root.file_name().is_some_and(|name| name == "superfile")
                && root
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name == "apps"),
            "Unexpected Scoop install layout"
        );
        ensure!(
            !is_link(&fs::symlink_metadata(previous)?),
            "Scoop version directory must not be a link"
        );
        let current = root.join("current");
        ensure!(
            junction::exists(&current)? && current.canonicalize()? == previous.canonicalize()?,
            "Scoop current junction does not point to the running version"
        );
        let manifest = read_metadata(&previous.join("manifest.json"))?;
        let receipt = read_metadata(&previous.join("install.json"))?;
        let package: Value = serde_json::from_slice(&manifest)?;
        let install: Value = serde_json::from_slice(&receipt)?;
        let version = package["version"]
            .as_str()
            .context("Missing Scoop version")?;
        Version::parse(version)?;
        ensure!(
            previous.file_name().is_some_and(|name| name == version),
            "Scoop version metadata does not match its directory"
        );
        ensure!(
            install["architecture"] == "64bit" && cfg!(target_arch = "x86_64"),
            "Unsupported Scoop architecture"
        );
        ensure!(
            package["bin"] == "superfile.exe",
            "Unsupported Scoop command shim"
        );
        ensure!(
            package["architecture"]
                .as_object()
                .is_some_and(|entries| entries.len() == 1 && entries.contains_key("64bit")),
            "Unsupported Scoop architecture manifest"
        );
        ensure!(
            package["architecture"]["64bit"]
                .as_object()
                .is_some_and(|fields| fields
                    .keys()
                    .all(|key| matches!(key.as_str(), "url" | "hash"))),
            "Invalid Scoop architecture payload"
        );
        // Scripts, persistence and custom layouts require Scoop's own installer.
        for key in [
            "installer",
            "uninstaller",
            "pre_install",
            "post_install",
            "pre_uninstall",
            "post_uninstall",
            "pre_update",
            "post_update",
            "persist",
            "extract_dir",
            "extract_to",
            "env_add_path",
            "env_set",
            "psmodule",
        ] {
            ensure!(
                package.get(key).is_none() && package["architecture"]["64bit"].get(key).is_none(),
                "Scoop manifest requires package-manager installation: {key}"
            );
        }
        if let Some(shortcuts) = package.get("shortcuts") {
            ensure!(
                shortcuts
                    .as_array()
                    .is_some_and(|entries| entries.iter().all(|entry| entry
                        .as_array()
                        .is_some_and(|parts| parts.len() == 2
                            && parts[0] == "superfile.exe"
                            && parts[1].is_string()))),
                "Unsupported Scoop shortcuts"
            );
        }
        Ok(Self {
            root: root.to_owned(),
            previous: previous.to_owned(),
            manifest,
            receipt,
        })
    }

    pub fn write_metadata(&self, contents: &Path, release: &Release) -> Result<()> {
        let mut manifest: Value = serde_json::from_slice(&self.manifest)?;
        manifest["version"] = release.version.to_string().into();
        manifest["architecture"]["64bit"]["url"] = release.asset.url.clone().into();
        manifest["architecture"]["64bit"]["hash"] = release
            .sha256
            .clone()
            .context("Missing Scoop update checksum")?
            .into();
        for (name, bytes) in [
            ("manifest.json", serde_json::to_vec_pretty(&manifest)?),
            ("install.json", self.receipt.clone()),
        ] {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(contents.join(name))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Ok(())
    }

    fn unchanged(&self) -> Result<()> {
        let active = Self::inspect(&self.previous.join("superfile.exe"))?;
        ensure!(
            active.manifest == self.manifest && active.receipt == self.receipt,
            "Scoop install metadata changed while downloading; check for updates again"
        );
        Ok(())
    }

    pub fn commit(&self, staging: &Path, version: &Version) -> Result<Installed> {
        ensure!(
            staging.parent() == Some(self.root.as_path()),
            "Scoop staging must be in the application directory"
        );
        // Serialize application updates. Scoop itself does not use this lock, so
        // also revalidate its active junction and metadata just before switching.
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".superfile-update.lock"))?;
        lock.try_lock()
            .context("Another Superfile update is installing")?;
        self.unchanged()?;
        let previous: Value = serde_json::from_slice(&self.manifest)?;
        ensure!(
            version
                > &Version::parse(
                    previous["version"]
                        .as_str()
                        .context("Missing Scoop version")?
                )?,
            "Scoop update must be newer than the installed version"
        );
        let destination = self.root.join(version.to_string());
        ensure!(
            !destination.try_exists()? && fs::symlink_metadata(&destination).is_err(),
            "Scoop update version already exists"
        );
        let contents = staging.join("payload");
        let next = staging.join("next-current");
        let old = staging.join("previous-current");
        let current = self.root.join("current");
        // Build the replacement junction before touching current. This needs no
        // elevation or shell, and remains on the same volume as the version.
        if let Err(error) = junction::create(&destination, &next) {
            // Junction creation can leave an empty directory on failure.
            let _ = fs::remove_dir(&next);
            return Err(error).context("Cannot prepare Scoop current junction");
        }
        let result = self.publish(&contents, &destination, &current, &next, &old);
        // Remove junctions only, never the directories they point at.
        for link in [&next, &old] {
            if let Err(error) = fs::remove_dir(link)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("Cannot remove update junction {}: {error}", link.display());
            }
        }
        result?;
        Ok(Installed {
            executable: destination.join("superfile.exe"),
            backup: self.previous.join("superfile.exe"),
        })
    }

    fn publish(
        &self,
        contents: &Path,
        destination: &Path,
        current: &Path,
        next: &Path,
        old: &Path,
    ) -> Result<()> {
        self.unchanged()?;
        // Windows directory rename refuses an existing destination. Do not
        // merge with or overwrite any existing Scoop version.
        fs::rename(contents, destination).context("Cannot publish Scoop update version")?;
        let activate = (|| -> Result<()> {
            self.unchanged()?;
            fs::rename(current, old).context("Cannot move Scoop current junction aside")?;
            if let Err(error) = fs::rename(next, current) {
                fs::rename(old, current).with_context(|| format!("Cannot restore Scoop current junction after {error}; original version preserved at {}. Run scoop reset superfile to restore its links", self.previous.display()))?;
                return Err(error)
                    .context("Cannot activate Scoop update; original version restored");
            }
            Ok(())
        })();
        if let Err(error) = activate {
            // Leave both versions in place if restoring current failed.
            if current
                .canonicalize()
                .ok()
                .is_some_and(|path| Some(path) == self.previous.canonicalize().ok())
            {
                fs::rename(destination, contents).with_context(|| {
                    format!(
                        "{error:#}; update payload retained at {}",
                        destination.display()
                    )
                })?;
            }
            return Err(error);
        }
        Ok(())
    }
}

fn read_metadata(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !is_link(&metadata) && metadata.len() <= 1024 * 1024,
        "Invalid Scoop metadata: {}",
        path.display()
    );
    Ok(fs::read(path)?)
}
