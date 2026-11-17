use super::*;
use crate::platform::PrivateDirExt;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

const DIRECTORY: &str = "https://raw.githubusercontent.com/obsidianmd/obsidian-releases/master/community-css-themes.json";
const CSS_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Package {
    entry: CommunityTheme,
    css: String,
}

fn fetch(url: &url::Url, limit: u64, cancel: &Cancellation) -> Result<Vec<u8>> {
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("raw.githubusercontent.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "Unsupported theme download URL"
    );
    cancel.check()?;
    static CLIENT: OnceLock<ureq::Agent> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(20)))
                .timeout_per_read(Some(Duration::from_secs(3)))
                .build(),
        )
    });
    let mut response = client
        .get(url.as_str())
        .header("User-Agent", "Superfile-Community-Themes")
        .call()?;
    let mut reader = crate::work::Reader {
        inner: response.body_mut().as_reader().take(limit + 1),
        cancel: cancel.clone(),
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Theme download exceeds its size limit"
    );
    cancel.check()?;
    Ok(bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Theme storage has no parent")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .private_mode(0o700)
        .create(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    // Readers use owned bytes and have closed their descriptors before replacement.
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn parse_catalog(bytes: &[u8]) -> Result<Vec<CommunityTheme>> {
    let entries: Vec<CommunityTheme> = serde_json::from_slice(bytes)?;
    let mut repos = std::collections::BTreeSet::new();
    let mut themes = Vec::new();
    for entry in entries {
        if entry.validate().is_ok() && repos.insert(entry.repo.to_ascii_lowercase()) {
            themes.push(entry);
        }
    }
    ensure!(!themes.is_empty(), "The community theme directory is empty");
    themes.sort_by_cached_key(|entry| entry.name.to_lowercase());
    Ok(themes)
}

pub fn catalog(cancel: &Cancellation) -> Result<Catalog> {
    let cache = crate::platform::cache_directory()?.join("themes/catalog.json");
    let remote = fetch(&url::Url::parse(DIRECTORY)?, 4 * 1024 * 1024, cancel)
        .and_then(|bytes| parse_catalog(&bytes).map(|themes| (bytes, themes)));
    match remote {
        Ok((bytes, themes)) => {
            if let Err(error) = write_atomic(&cache, &bytes) {
                log::warn!("Cannot cache theme directory: {error:#}");
            }
            Ok(Catalog {
                themes,
                cached: false,
            })
        }
        Err(error) => {
            cancel.check()?;
            let bytes = read_limited(&cache, 4 * 1024 * 1024)
                .with_context(|| format!("Cannot load community themes: {error:#}"))?;
            Ok(Catalog {
                themes: parse_catalog(&bytes)?,
                cached: true,
            })
        }
    }
}

fn package_path(root: &Path, id: &str) -> Result<PathBuf> {
    ensure!(
        id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid installed theme ID"
    );
    Ok(root.join(format!("{id}.json")))
}

fn load_at(root: &Path, id: &str, cancel: &Cancellation) -> Result<InstalledTheme> {
    cancel.check()?;
    let package: Package =
        serde_json::from_slice(&read_limited(&package_path(root, id)?, 24 * 1024 * 1024)?)?;
    ensure!(
        package.entry.id() == id,
        "Installed theme identity does not match its package"
    );
    let scheme = compile(&package.entry, &package.css, cancel)?;
    Ok(InstalledTheme {
        entry: package.entry,
        scheme,
    })
}

pub fn load(id: &str, cancel: &Cancellation) -> Result<InstalledTheme> {
    load_at(&package_directory()?, id, cancel)
}

pub fn installed(cancel: &Cancellation) -> Result<Vec<InstalledTheme>> {
    let root = package_directory()?;
    let files = match std::fs::read_dir(&root) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut result = Vec::new();
    for file in files {
        cancel.check()?;
        let file = file?;
        if file
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let id = file
            .path()
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        match load_at(&root, &id, cancel) {
            Ok(package) => result.push(package),
            Err(error) => {
                cancel.check()?;
                log::warn!("Cannot load installed theme {id}: {error:#}");
            }
        }
    }
    result.sort_by_cached_key(|package| package.entry.name.to_lowercase());
    Ok(result)
}

pub fn install(entry: &CommunityTheme, cancel: &Cancellation) -> Result<InstalledTheme> {
    let bytes = match fetch(&entry.resource_url("theme.css")?, CSS_LIMIT, cancel) {
        Ok(bytes) => bytes,
        Err(error)
            if entry.legacy
                || matches!(
                    error.downcast_ref::<ureq::Error>(),
                    Some(ureq::Error::StatusCode(404))
                ) =>
        {
            fetch(&entry.resource_url("obsidian.css")?, CSS_LIMIT, cancel)?
        }
        Err(error) => return Err(error),
    };
    let css = String::from_utf8(bytes).context("Theme stylesheet is not UTF-8")?;
    let scheme = compile(entry, &css, cancel)?;
    cancel.check()?;
    let package = Package {
        entry: entry.clone(),
        css,
    };
    write_atomic(
        &package_path(&package_directory()?, &entry.id())?,
        &serde_json::to_vec(&package)?,
    )?;
    Ok(InstalledTheme {
        entry: entry.clone(),
        scheme,
    })
}

pub fn preview(entry: &CommunityTheme, cancel: &Cancellation) -> Result<PathBuf> {
    let url = entry.resource_url(&entry.screenshot)?;
    use sha2::{Digest, Sha256};
    let cache = crate::platform::cache_directory()?.join(format!(
        "themes/previews/{:x}.png",
        Sha256::digest(url.as_str().as_bytes())
    ));
    if cache.is_file() {
        return Ok(cache);
    }
    let bytes = fetch(&url, 8 * 1024 * 1024, cancel)?;
    let extension = Path::new(&entry.screenshot)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    ensure!(
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
        ),
        "Unsupported theme screenshot format"
    );
    let mut input = tempfile::Builder::new()
        .suffix(&format!(".{extension}"))
        .tempfile()?;
    input.write_all(&bytes)?;
    input.flush()?;
    let png = crate::inline_images::thumbnail_png_cancellable(input.path(), 640, cancel)?;
    cancel.check()?;
    write_atomic(&cache, &png)?;
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> CommunityTheme {
        CommunityTheme {
            name: "Test".into(),
            author: "Author".into(),
            repo: "owner/repo".into(),
            screenshot: "preview wide.png".into(),
            modes: vec!["dark".into(), "light".into()],
            branch: None,
            legacy: false,
        }
    }
    #[test]
    fn repository_and_paths_cannot_escape_download_or_storage_roots() {
        let mut item = entry();
        assert_eq!(
            item.resource_url("preview wide.png").unwrap().as_str(),
            "https://raw.githubusercontent.com/owner/repo/HEAD/preview%20wide.png"
        );
        for repo in [
            "../repo",
            "owner/../repo",
            "https://evil.example/repo",
            "owner/repo?query",
        ] {
            item.repo = repo.into();
            assert!(item.validate().is_err());
        }
        for path in [
            "../theme.css",
            "/theme.css",
            "https://evil.example/theme.css",
            "theme.css?token=x",
        ] {
            assert!(entry().resource_url(path).is_err());
        }
        assert!(package_path(Path::new("themes"), "../../settings").is_err());
    }
    #[test]
    fn package_is_atomic_reopenable_and_rejects_identity_substitution() {
        let root = tempfile::tempdir().unwrap();
        let entry = entry();
        let mut package = Package {
            entry: entry.clone(),
            css: ".theme-dark { --background-primary: #123456; }".into(),
        };
        let path = package_path(root.path(), &entry.id()).unwrap();
        write_atomic(&path, &serde_json::to_vec(&package).unwrap()).unwrap();
        let opened = load_at(root.path(), &entry.id(), &Cancellation::default()).unwrap();
        super::super::assert_rgb(opened.scheme.dark.get("--background-primary"), 0x123456);
        drop(opened);
        package.css = ".theme-dark { --background-primary: #654321; }".into();
        write_atomic(&path, &serde_json::to_vec(&package).unwrap()).unwrap();
        super::super::assert_rgb(
            load_at(root.path(), &entry.id(), &Cancellation::default())
                .unwrap()
                .scheme
                .dark
                .get("--background-primary"),
            0x654321,
        );
        package.entry.repo = "other/theme".into();
        write_atomic(&path, &serde_json::to_vec(&package).unwrap()).unwrap();
        assert!(load_at(root.path(), &entry.id(), &Cancellation::default()).is_err());
    }
}
