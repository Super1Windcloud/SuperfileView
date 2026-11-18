//! Static-manifest updates. Blocking work stays off the UI thread.
mod install;
#[cfg(test)]
mod tests;
pub use gpui_updater_core::{Asset, CancelToken, Release, Version};
pub use install::{Installed, PreparedUpdate};

use anyhow::{Context, Result, ensure};
use gpui_updater_core::{
    BundleVersion, CodeSigning, EngineConfig, StaticManifestSource, UpdateEngine, UpdateSource,
    Verification,
};
use std::path::PathBuf;

pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/Super1Windcloud/SuperfileView/releases/latest/download/latest.json";

pub fn manifest_url() -> &'static str {
    option_env!("SUPERFILE_UPDATE_MANIFEST_URL").unwrap_or(DEFAULT_MANIFEST_URL)
}

pub fn release_page() -> String {
    manifest_url()
        .strip_suffix("/latest/download/latest.json")
        .unwrap_or("https://github.com/Super1Windcloud/SuperfileView/releases")
        .to_owned()
}

struct TrustedManifest(StaticManifestSource);
impl UpdateSource for TrustedManifest {
    fn fetch_latest(&self) -> gpui_updater_core::Result<Release> {
        let release = self.0.fetch_latest()?;
        validate_release(&release)
            .map_err(|error| gpui_updater_core::Error::Payload(error.to_string()))?;
        Ok(release)
    }
}

fn validate_release(release: &Release) -> Result<()> {
    let manifest = url::Url::parse(manifest_url())?;
    let asset = url::Url::parse(&release.asset.url)?;
    ensure!(
        manifest.scheme() == "https" && asset.scheme() == "https",
        "Update URLs must use HTTPS"
    );
    let prefix = manifest
        .path()
        .strip_suffix("/releases/latest/download/latest.json")
        .context("Invalid update manifest URL")?;
    ensure!(
        manifest.host_str() == Some("github.com")
            && manifest.username().is_empty()
            && manifest.password().is_none()
            && manifest.query().is_none()
            && manifest.fragment().is_none()
            && manifest.port().is_none()
            && asset.host_str() == manifest.host_str()
            && asset.username().is_empty()
            && asset.password().is_none()
            && asset.query().is_none()
            && asset.fragment().is_none()
            && asset.port().is_none(),
        "Update asset is outside the configured GitHub repository"
    );
    ensure!(
        release.version.pre.is_empty(),
        "The stable update feed must not publish a prerelease"
    );
    ensure!(release.asset.size > 0, "Update asset size is missing");
    ensure!(
        release.sha256.as_ref().is_some_and(
            |hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        ),
        "Update SHA-256 is missing or invalid"
    );
    let suffix = if cfg!(windows) { ".zip" } else { ".tar.gz" };
    let matched = [release.version.to_string(), format!("v{}", release.version)]
        .into_iter()
        .any(|tag| {
            let name = format!(
                "superfile-{tag}-{}-{}{suffix}",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            release.asset.name == name
                && asset.path() == format!("{prefix}/releases/download/{tag}/{name}")
        });
    ensure!(
        matched,
        "Update asset does not match the repository, version, platform or architecture"
    );
    Ok(())
}

fn source(url: &str) -> StaticManifestSource {
    StaticManifestSource::new(url)
        .os(std::env::consts::OS)
        .arch(std::env::consts::ARCH)
        .format(if cfg!(windows) { "zip" } else { "tar.gz" })
}

fn engine(download_dir: Option<PathBuf>) -> Result<UpdateEngine<TrustedManifest>> {
    // Download-only users (e.g. package-manager installs) need no write access
    // to the running executable. Our installer checks its real target separately.
    let root = match &download_dir {
        Some(directory) => directory.join("download-only"),
        None => install::running_root()?,
    };
    let mut config = EngineConfig::new(Version::parse(env!("CARGO_PKG_VERSION"))?)
        .verification(Verification::Checksum)
        // Native archive validation below replaces upstream command-line payload checks.
        .code_signing(CodeSigning::Off)
        .bundle_version(BundleVersion::Ignore)
        .install_root(root);
    config.download_dir = download_dir;
    Ok(UpdateEngine::new(
        TrustedManifest(source(manifest_url())),
        config,
    ))
}

pub fn check() -> Result<Option<Release>> {
    engine(None)?
        .check()
        .context("Cannot check the update manifest")
}

pub fn download(
    release: &Release,
    allow_install: bool,
    cancel: &CancelToken,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PreparedUpdate> {
    validate_release(release)?;
    let directory = crate::platform::cache_directory()?.join("updates/downloads");
    std::fs::create_dir_all(&directory)?;
    let download = tempfile::Builder::new()
        .prefix("update-")
        .tempdir_in(directory)?;
    let artifact = engine(Some(download.path().to_owned()))?
        .download_cancellable(release, cancel, progress)?;
    let mut prepared = install::prepare(artifact.path(), release, allow_install, cancel)?;
    prepared.download = Some(download);
    Ok(prepared)
}

pub fn install(update: &PreparedUpdate) -> Result<Installed> {
    install::commit(update)
}

pub fn relaunch(executable: &std::path::Path) -> Result<()> {
    // Spawn only our installed executable, after every application shutdown write.
    // GPUI's built-in restart uses shell scripts; this path needs no host utilities.
    std::process::Command::new(executable)
        .current_dir(crate::platform::home()?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Cannot relaunch the updated application")?;
    Ok(())
}

pub fn startup_enabled() -> Result<bool> {
    let db = crate::storage::open()?;
    Ok(crate::storage::setting(&db, "update-startup")?.as_deref() != Some("false"))
}
pub fn save_startup(enabled: bool) -> Result<()> {
    crate::storage::set_setting(
        &crate::storage::open()?,
        "update-startup",
        &enabled.to_string(),
    )
}
fn cancelled(cancel: &CancelToken) -> Result<()> {
    ensure!(!cancel.is_cancelled(), "Update cancelled");
    Ok(())
}
