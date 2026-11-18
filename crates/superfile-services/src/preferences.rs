//! Application preferences and workspace state backed by the shared SQLite store.
use crate::storage::{self, set_setting, setting};
use anyhow::Result;
use std::{collections::BTreeMap, path::PathBuf};

pub(crate) fn directory() -> Result<PathBuf> {
    crate::platform::data_directory()
}
pub fn workspace_folders() -> Result<Vec<PathBuf>> {
    storage::paths(&storage::open()?, "workspace", "")
}
pub fn save_workspace_folders(folders: &[PathBuf]) -> Result<()> {
    let mut db = storage::open()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    storage::save_paths(&tx, "workspace", "", folders)?;
    tx.commit()?;
    Ok(())
}
pub type ExplorerExpansion = BTreeMap<PathBuf, Vec<PathBuf>>;
pub fn explorer_expansion(roots: &[PathBuf]) -> Result<ExplorerExpansion> {
    let db = storage::open()?;
    roots
        .iter()
        .map(|root| {
            Ok((
                root.clone(),
                storage::paths(&db, "explorer", storage::path_text(root)?)?,
            ))
        })
        .collect()
}
pub fn save_explorer_expansion(roots: &[PathBuf], expanded: &ExplorerExpansion) -> Result<()> {
    let mut db = storage::open()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for root in roots {
        let paths = expanded
            .get(root)
            .into_iter()
            .flatten()
            .filter(|p| p.starts_with(root))
            .cloned()
            .collect::<Vec<_>>();
        storage::save_paths(&tx, "explorer", storage::path_text(root)?, &paths)?;
    }
    tx.commit()?;
    Ok(())
}
/// A workspace's ordered folder roots, including repeated paths.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RecentWorkspace {
    pub id: String,
    pub folders: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl RecentWorkspace {
    pub fn title(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.folders
                .iter()
                .map(|path| {
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
                .join(" · ")
        })
    }
}

pub(crate) fn recent_at(db: &rusqlite::Connection) -> Result<Vec<RecentWorkspace>> {
    let values = db
        .prepare("SELECT id,name FROM recent_workspaces ORDER BY position")?
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    values
        .into_iter()
        .map(|(id, name)| {
            Ok(RecentWorkspace {
                folders: storage::paths(db, "recent", &id)?,
                id,
                name,
            })
        })
        .collect()
}
pub fn recent_workspaces() -> Result<Vec<RecentWorkspace>> {
    let mut db = storage::open()?;
    let tx = db.transaction()?;
    let recent = recent_at(&tx)?;
    tx.commit()?;
    Ok(recent)
}
pub(crate) fn save_recent_at(db: &rusqlite::Connection, recent: &[RecentWorkspace]) -> Result<()> {
    db.execute("DELETE FROM recent_workspaces", [])?;
    db.execute("DELETE FROM paths WHERE scope='recent'", [])?;
    for (position, entry) in recent.iter().enumerate() {
        db.execute(
            "INSERT INTO recent_workspaces VALUES(?1,?2,?3)",
            rusqlite::params![entry.id, entry.name, position as i64],
        )?;
        storage::save_paths(db, "recent", &entry.id, &entry.folders)?;
    }
    Ok(())
}
pub fn remember_recent_workspace(mut workspace: RecentWorkspace) -> Result<Vec<RecentWorkspace>> {
    let mut db = storage::open()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut recent = recent_at(&tx)?;
    if workspace.name.is_none() {
        workspace.name = recent
            .iter()
            .find(|e| {
                e.id == workspace.id
                    || (!crate::remote::is_remote_id(&workspace.id)
                        && !crate::remote::is_remote_id(&e.id)
                        && e.folders == workspace.folders)
            })
            .and_then(|e| e.name.clone());
    }
    recent.retain(|e| {
        e.id != workspace.id
            && (crate::remote::is_remote_id(&workspace.id)
                || crate::remote::is_remote_id(&e.id)
                || workspace.folders.is_empty()
                || e.folders != workspace.folders)
    });
    if !workspace.folders.is_empty() {
        recent.insert(0, workspace);
    }
    save_recent_at(&tx, &recent)?;
    tx.commit()?;
    Ok(recent)
}
pub fn remove_recent_workspace(id: &str) -> Result<Vec<RecentWorkspace>> {
    let mut db = storage::open()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut recent = recent_at(&tx)?;
    recent.retain(|e| e.id != id);
    save_recent_at(&tx, &recent)?;
    tx.commit()?;
    Ok(recent)
}
fn get(key: &str) -> Result<Option<String>> {
    setting(&storage::open()?, key)
}
fn set(key: &str, value: &str) -> Result<()> {
    set_setting(&storage::open()?, key, value)
}
pub fn word_wrap() -> bool {
    get("word-wrap")
        .ok()
        .flatten()
        .is_some_and(|v| v.trim() == "true")
}
pub fn save_word_wrap(enabled: bool) -> Result<()> {
    set("word-wrap", &enabled.to_string())
}

pub const MIN_READER_FONT_SIZE: i16 = 6;
pub const MAX_READER_FONT_SIZE: i16 = 100;
// The default source editor uses 13 px. Other readers keep their own base sizes.
const READER_FONT_REFERENCE: i16 = 13;
const READER_FONT_OFFSET_KEY: &str = "reader-font-offset";

pub fn clamp_reader_font_offset(offset: i16) -> i16 {
    offset.clamp(
        MIN_READER_FONT_SIZE - READER_FONT_REFERENCE,
        MAX_READER_FONT_SIZE - READER_FONT_REFERENCE,
    )
}

pub fn reader_font_offset() -> i16 {
    storage::open()
        .and_then(|db| reader_font_offset_in(&db))
        .unwrap_or_default()
}

fn reader_font_offset_in(db: &rusqlite::Connection) -> Result<i16> {
    Ok(setting(db, READER_FONT_OFFSET_KEY)?
        .and_then(|value| value.trim().parse().ok())
        .map_or(0, clamp_reader_font_offset))
}

pub fn save_reader_font_offset(offset: i16) -> Result<()> {
    save_reader_font_offset_in(&storage::open()?, offset)
}

fn save_reader_font_offset_in(db: &rusqlite::Connection, offset: i16) -> Result<()> {
    set_setting(
        db,
        READER_FONT_OFFSET_KEY,
        &clamp_reader_font_offset(offset).to_string(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
pub fn appearance() -> Appearance {
    match get("appearance").ok().flatten().as_deref().map(str::trim) {
        Some("light") => Appearance::Light,
        Some("dark") => Appearance::Dark,
        _ => Appearance::System,
    }
}
pub fn save_appearance(value: Appearance) -> Result<()> {
    set(
        "appearance",
        match value {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AccentColor(u32);

impl AccentColor {
    pub const PURPLE: Self = Self(0x8b5cf6);

    pub fn from_rgb(rgb: u32) -> Self {
        Self(rgb & 0xffffff)
    }

    pub fn rgb(self) -> u32 {
        self.0
    }

    pub fn hex(self) -> String {
        format!("#{:06X}", self.0)
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(value, 16).ok().map(Self)
    }
}

impl Default for AccentColor {
    fn default() -> Self {
        Self::PURPLE
    }
}

impl TryFrom<String> for AccentColor {
    type Error = &'static str;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::parse(&value).ok_or("Invalid accent color")
    }
}
impl From<AccentColor> for String {
    fn from(value: AccentColor) -> Self {
        value.hex()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ThemePreferences {
    pub scheme: Option<String>,
    pub accent: Option<AccentColor>,
}

pub fn theme_preferences() -> ThemePreferences {
    storage::open()
        .and_then(|db| theme_preferences_in(&db))
        .unwrap_or_default()
}

fn theme_preferences_in(db: &rusqlite::Connection) -> Result<ThemePreferences> {
    if let Some(value) = setting(db, "theme-preferences")? {
        return serde_json::from_str(&value).map_err(Into::into);
    }
    Ok(ThemePreferences {
        scheme: None,
        accent: setting(db, "accent-color")?
            .as_deref()
            .and_then(AccentColor::parse),
    })
}

pub fn save_theme_preferences(preferences: &ThemePreferences) -> Result<()> {
    save_theme_preferences_in(&storage::open()?, preferences)
}

fn save_theme_preferences_in(
    db: &rusqlite::Connection,
    preferences: &ThemePreferences,
) -> Result<()> {
    anyhow::ensure!(
        preferences
            .scheme
            .as_ref()
            .is_none_or(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())),
        "Invalid theme package ID"
    );
    set_setting(
        db,
        "theme-preferences",
        &serde_json::to_string(preferences)?,
    )
}

pub fn accent_color() -> AccentColor {
    storage::open()
        .and_then(|db| accent_color_in(&db))
        .unwrap_or_default()
}

fn accent_color_in(db: &rusqlite::Connection) -> Result<AccentColor> {
    Ok(setting(db, "accent-color")?
        .as_deref()
        .and_then(AccentColor::parse)
        .unwrap_or_default())
}

pub fn save_accent_color(color: AccentColor) -> Result<()> {
    save_accent_color_in(&storage::open()?, color)
}

fn save_accent_color_in(db: &rusqlite::Connection, color: AccentColor) -> Result<()> {
    set_setting(db, "accent-color", &color.hex())
}

fn preview_key(kind: &str) -> Result<String> {
    anyhow::ensure!(
        matches!(kind, "html" | "markdown" | "csv"),
        "Unknown preview preference"
    );
    Ok(format!("{kind}-view"))
}
pub fn preview_mode(kind: &str) -> bool {
    preview_key(kind)
        .ok()
        .and_then(|key| get(&key).ok().flatten())
        .map_or(kind != "csv", |v| v.trim() != "source")
}
pub fn save_preview_mode(kind: &str, preview: bool) -> Result<()> {
    set(
        &preview_key(kind)?,
        if preview { "preview" } else { "source" },
    )
}
pub fn language() -> Option<String> {
    language_in(&storage::open().ok()?).ok().flatten()
}
fn language_in(db: &rusqlite::Connection) -> Result<Option<String>> {
    Ok(setting(db, "language")?
        .filter(|v| crate::locale::supported(v.trim()))
        .map(|v| v.trim().to_owned()))
}
pub fn save_language(language: &str) -> Result<()> {
    save_language_in(&storage::open()?, language)
}
fn save_language_in(db: &rusqlite::Connection, language: &str) -> Result<()> {
    anyhow::ensure!(crate::locale::supported(language), "Unknown language");
    set_setting(db, "language", language)
}
/// A missing choice lets the app apply its defaults; unreadable data fails closed.
pub fn telemetry_consent() -> Result<Option<crate::telemetry::Consent>> {
    telemetry_consent_in(&storage::open()?)
}
fn telemetry_consent_in(db: &rusqlite::Connection) -> Result<Option<crate::telemetry::Consent>> {
    if let Some(value) = setting(db, "telemetry-consent")? {
        return Ok(Some(serde_json::from_str(&value)?));
    }
    Ok(setting(db, "telemetry")?.map(|value| {
        let enabled = value.trim() == "true";
        crate::telemetry::Consent {
            usage: enabled,
            diagnostics: enabled,
        }
    }))
}
pub fn save_telemetry_consent(consent: crate::telemetry::Consent) -> Result<()> {
    let mut db = storage::open()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    set_setting(&tx, "telemetry-consent", &serde_json::to_string(&consent)?)?;
    // Older builds have one switch. Downgrading must not re-enable a channel
    // that was explicitly disabled with the split controls.
    set_setting(
        &tx,
        "telemetry",
        &(consent.usage && consent.diagnostics).to_string(),
    )?;
    tx.commit()?;
    Ok(())
}
pub fn workspace_sessions() -> Result<BTreeMap<String, superfile_core::session::WorkspaceSession>> {
    Ok(storage::sessions::load(&storage::open()?, "workspace")?
        .into_iter()
        .filter_map(|(id, sessions)| sessions.into_iter().next().map(|s| (id, s)))
        .collect())
}
pub fn save_workspace_sessions(
    sessions: &BTreeMap<String, superfile_core::session::WorkspaceSession>,
) -> Result<()> {
    save_groups(
        "workspace",
        &sessions
            .iter()
            .map(|(id, s)| (id.clone(), vec![s.clone()]))
            .collect(),
    )
}
pub fn window_sessions() -> Result<BTreeMap<String, Vec<superfile_core::session::WorkspaceSession>>>
{
    storage::sessions::load(&storage::open()?, "window")
}
pub fn save_window_sessions(
    windows: &BTreeMap<String, Vec<superfile_core::session::WorkspaceSession>>,
) -> Result<()> {
    save_groups("window", windows)
}
fn save_groups(
    scope: &str,
    groups: &BTreeMap<String, Vec<superfile_core::session::WorkspaceSession>>,
) -> Result<()> {
    let mut db = storage::open()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    storage::sessions::save(&tx, scope, groups)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod reader_font_tests {
    use super::*;

    #[test]
    fn reader_font_offset_and_reset_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let mut db = storage::open_at(directory.path()).unwrap();
        assert_eq!(reader_font_offset_in(&db).unwrap(), 0);
        for offset in [3, -2, 0] {
            save_reader_font_offset_in(&db, offset).unwrap();
            drop(db);
            db = storage::open_at(directory.path()).unwrap();
            assert_eq!(reader_font_offset_in(&db).unwrap(), offset);
        }
        set_setting(&db, READER_FONT_OFFSET_KEY, "999").unwrap();
        assert_eq!(reader_font_offset_in(&db).unwrap(), 87);
        set_setting(&db, READER_FONT_OFFSET_KEY, "invalid").unwrap();
        assert_eq!(reader_font_offset_in(&db).unwrap(), 0);
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn language_survives_reopening_and_invalid_choices_preserve_the_saved_value() {
        let directory = tempfile::tempdir().unwrap();
        let mut db = storage::open_at(directory.path()).unwrap();
        assert_eq!(language_in(&db).unwrap(), None);
        for choice in ["zh-CN", "en", "system"] {
            save_language_in(&db, choice).unwrap();
            drop(db);
            db = storage::open_at(directory.path()).unwrap();
            assert_eq!(language_in(&db).unwrap().as_deref(), Some(choice));
        }
        assert!(save_language_in(&db, "fr").is_err());
        assert_eq!(language_in(&db).unwrap().as_deref(), Some("system"));
        set_setting(&db, "language", " zh-CN ").unwrap();
        assert_eq!(language_in(&db).unwrap().as_deref(), Some("zh-CN"));
        set_setting(&db, "language", "unknown").unwrap();
        assert_eq!(language_in(&db).unwrap(), None);
    }
}

#[cfg(test)]
mod accent_tests {
    use super::*;

    #[test]
    fn theme_selection_migrates_legacy_accent_and_persists_reset() {
        let directory = tempfile::tempdir().unwrap();
        let db = storage::open_at(directory.path()).unwrap();
        assert_eq!(
            theme_preferences_in(&db).unwrap(),
            ThemePreferences::default()
        );
        save_accent_color_in(&db, AccentColor::from_rgb(0x3b82f6)).unwrap();
        let mut preferences = theme_preferences_in(&db).unwrap();
        assert_eq!(preferences.accent.unwrap().hex(), "#3B82F6");
        preferences.scheme = Some("a".repeat(64));
        preferences.accent = None;
        save_theme_preferences_in(&db, &preferences).unwrap();
        drop(db);
        let db = storage::open_at(directory.path()).unwrap();
        assert_eq!(theme_preferences_in(&db).unwrap(), preferences);
        save_theme_preferences_in(&db, &ThemePreferences::default()).unwrap();
        assert_eq!(
            theme_preferences_in(&db).unwrap(),
            ThemePreferences::default()
        );
        preferences.scheme = Some("../theme".into());
        assert!(save_theme_preferences_in(&db, &preferences).is_err());
        assert_eq!(
            theme_preferences_in(&db).unwrap(),
            ThemePreferences::default()
        );
        set_setting(&db, "theme-preferences", "{broken").unwrap();
        assert!(theme_preferences_in(&db).is_err());
    }

    #[test]
    fn accent_defaults_to_purple_and_survives_reopening_the_store() {
        let directory = tempfile::tempdir().unwrap();
        let db = storage::open_at(directory.path()).unwrap();
        assert_eq!(accent_color_in(&db).unwrap(), AccentColor::PURPLE);
        let color = AccentColor::parse("#0aBC12").unwrap();
        save_accent_color_in(&db, color).unwrap();
        drop(db);
        let db = storage::open_at(directory.path()).unwrap();
        assert_eq!(accent_color_in(&db).unwrap(), color);
        assert_eq!(
            setting(&db, "accent-color").unwrap().as_deref(),
            Some("#0ABC12")
        );
        set_setting(&db, "accent-color", "#not-a-color").unwrap();
        assert_eq!(accent_color_in(&db).unwrap(), AccentColor::PURPLE);
    }

    #[test]
    fn accent_rejects_invalid_or_transparent_values() {
        for value in ["", "#123", "#12345678", "#12GGFF", "+12345", "#紫色"] {
            assert_eq!(AccentColor::parse(value), None, "{value}");
        }
        assert_eq!(AccentColor::parse("  #8B5CF6 "), Some(AccentColor::PURPLE));
    }
}
