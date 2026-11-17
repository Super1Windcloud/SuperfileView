//! Shared catalogs for the application UI. English message IDs
//! are the canonical keys; locale files are discovered and embedded at build time.
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
include!(concat!(env!("OUT_DIR"), "/locales.rs"));
static LANGUAGE: AtomicUsize = AtomicUsize::new(0);
static CATALOGS: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();
fn catalogs() -> &'static Vec<HashMap<String, String>> {
    CATALOGS.get_or_init(|| {
        BUNDLED_CATALOGS
            .iter()
            .map(|(_, json)| serde_json::from_str(json).expect("bundled locale catalog"))
            .collect()
    })
}
fn english_index() -> usize {
    BUNDLED_CATALOGS
        .iter()
        .position(|(code, _)| *code == "en")
        .expect("English catalog")
}
pub fn supported(choice: &str) -> bool {
    choice == "system" || BUNDLED_CATALOGS.iter().any(|(code, _)| *code == choice)
}
pub fn available() -> Vec<(&'static str, &'static str)> {
    BUNDLED_CATALOGS
        .iter()
        .enumerate()
        .map(|(i, (code, _))| {
            (
                *code,
                catalogs()[i]
                    .get("__language_name")
                    .expect("locale language name")
                    .as_str(),
            )
        })
        .collect()
}
pub fn initialize() {
    set(&crate::preferences::language().unwrap_or_else(|| "system".into()));
}
pub fn set(choice: &str) {
    let system;
    let requested = if choice == "system" {
        system = sys_locale::get_locale().unwrap_or_else(|| "en".into());
        &system
    } else {
        choice
    };
    LANGUAGE.store(resolve(requested), Ordering::Relaxed);
}
fn resolve(requested: &str) -> usize {
    let normalized = requested.replace('_', "-").to_ascii_lowercase();
    BUNDLED_CATALOGS
        .iter()
        .position(|(code, _)| code.to_ascii_lowercase() == normalized)
        .or_else(|| {
            BUNDLED_CATALOGS.iter().position(|(code, _)| {
                code.split('-')
                    .next()
                    .unwrap()
                    .eq_ignore_ascii_case(normalized.split('-').next().unwrap())
            })
        })
        .unwrap_or_else(english_index)
}
pub fn code() -> &'static str {
    BUNDLED_CATALOGS[LANGUAGE.load(Ordering::Relaxed)].0
}
pub fn tr(text: &str) -> String {
    catalogs()[LANGUAGE.load(Ordering::Relaxed)]
        .get(text)
        .or_else(|| catalogs()[english_index()].get(text))
        .cloned()
        .unwrap_or_else(|| text.to_owned())
}
pub fn translations() -> serde_json::Value {
    let mut merged = catalogs()[english_index()].clone();
    merged.extend(catalogs()[LANGUAGE.load(Ordering::Relaxed)].clone());
    serde_json::to_value(merged).expect("locale catalog")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_locales_resolve_to_chinese_or_english_with_english_fallback() {
        for requested in ["zh-CN", "zh_CN", "ZH-cn", "zh-Hans-CN", "zh-TW", "zh"] {
            let index = resolve(requested);
            assert_eq!(BUNDLED_CATALOGS[index].0, "zh-CN", "{requested}");
            assert_eq!(catalogs()[index]["Settings"], "设置");
            assert_eq!(catalogs()[index]["Author"], "作者");
        }
        for requested in ["en", "en-US", "en_GB", "EN-us", "fr-FR", ""] {
            let index = resolve(requested);
            assert_eq!(BUNDLED_CATALOGS[index].0, "en", "{requested}");
            assert_eq!(catalogs()[index]["Settings"], "Settings");
            assert_eq!(catalogs()[index]["Author"], "Author");
        }
        assert_eq!(available(), vec![("en", "English"), ("zh-CN", "简体中文")]);
    }
}
