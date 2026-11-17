//! Obsidian color-variable compatibility and community packages. All I/O is background-only.
mod css;
mod store;
mod values;

use crate::work::Cancellation;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub use store::{catalog, install, installed, load, preview};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct CssColor(pub [f32; 4]);
impl CssColor {
    pub fn rgb(value: u32) -> Self {
        Self([
            ((value >> 16) & 255) as f32 / 255.,
            ((value >> 8) & 255) as f32 / 255.,
            (value & 255) as f32 / 255.,
            1.,
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub colors: BTreeMap<String, CssColor>,
    pub applied: usize,
    pub unresolved: Vec<String>,
}
impl Palette {
    pub fn get(&self, name: &str) -> CssColor {
        self.colors
            .get(name)
            .copied()
            .unwrap_or(CssColor::rgb(0x8b5cf6))
    }
}

#[derive(Clone, Debug)]
pub struct Scheme {
    pub id: Option<String>,
    pub name: String,
    pub author: String,
    pub light: Palette,
    pub dark: Palette,
}
impl Scheme {
    pub fn palette(&self, mode: Mode) -> &Palette {
        match mode {
            Mode::Light => &self.light,
            Mode::Dark => &self.dark,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommunityTheme {
    pub name: String,
    pub author: String,
    pub repo: String,
    #[serde(default)]
    pub screenshot: String,
    #[serde(default)]
    pub modes: Vec<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub legacy: bool,
}
impl CommunityTheme {
    pub fn id(&self) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(self.repo.as_bytes()))
    }
    pub fn validate(&self) -> Result<()> {
        let parts = self.repo.split('/').collect::<Vec<_>>();
        ensure!(
            parts.len() == 2
                && parts.iter().all(|part| !part.is_empty()
                    && *part != "."
                    && *part != ".."
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))),
            "Invalid community theme repository"
        );
        ensure!(
            !self.name.trim().is_empty() && self.name.len() < 256 && self.author.len() < 256,
            "Invalid theme metadata"
        );
        Ok(())
    }
    pub fn resource_url(&self, resource: &str) -> Result<url::Url> {
        self.validate()?;
        let branch = self.branch.as_deref().unwrap_or("HEAD");
        ensure!(
            !branch.is_empty()
                && !branch.contains(['\\', '?', '#'])
                && !branch.split('/').any(|p| matches!(p, "." | "..")),
            "Invalid theme branch"
        );
        ensure!(
            !resource.is_empty()
                && !resource.starts_with('/')
                && !resource.contains(['\\', ':', '?', '#'])
                && !resource.split('/').any(|p| matches!(p, "." | "..")),
            "Invalid theme resource path"
        );
        let mut url = url::Url::parse("https://raw.githubusercontent.com/")?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid theme URL"))?;
            for component in self
                .repo
                .split('/')
                .chain(std::iter::once(branch))
                .chain(resource.split('/'))
            {
                path.push(component);
            }
        }
        Ok(url)
    }
}

#[derive(Clone, Debug)]
pub struct InstalledTheme {
    pub entry: CommunityTheme,
    pub scheme: Scheme,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub themes: Vec<CommunityTheme>,
    pub cached: bool,
}

pub fn compile(entry: &CommunityTheme, css: &str, cancel: &Cancellation) -> Result<Scheme> {
    entry.validate()?;
    ensure!(
        css.len() <= 16 * 1024 * 1024,
        "Theme stylesheet exceeds 16 MiB"
    );
    let light = css::compile(css, Mode::Light, cancel)?;
    let dark = css::compile(css, Mode::Dark, cancel)?;
    ensure!(
        light.applied + dark.applied > 0,
        "This theme does not define supported root color variables"
    );
    Ok(Scheme {
        id: Some(entry.id()),
        name: entry.name.clone(),
        author: entry.author.clone(),
        light,
        dark,
    })
}

pub fn default_scheme() -> Scheme {
    Scheme {
        id: None,
        name: "Obsidian".into(),
        author: "Obsidian".into(),
        light: css::compile("", Mode::Light, &Cancellation::default()).expect("built-in palette"),
        dark: css::compile("", Mode::Dark, &Cancellation::default()).expect("built-in palette"),
    }
}

pub fn package_directory() -> Result<PathBuf> {
    Ok(crate::platform::data_directory()?.join("themes"))
}

pub(super) fn read_limited(path: &std::path::Path, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let file =
        std::fs::File::open(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= max,
        "Theme file exceeds its size limit"
    );
    Ok(bytes)
}

#[cfg(test)]
fn assert_rgb(actual: CssColor, expected: u32) {
    let expected = CssColor::rgb(expected);
    for (actual, expected) in actual.0.into_iter().zip(expected.0) {
        assert!(
            (actual - expected).abs() < 0.00001,
            "{actual} != {expected}"
        );
    }
}
