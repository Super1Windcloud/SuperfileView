//! File imports and persistent preferences for source editor palettes.
use crate::storage;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::OnceLock,
};

const LIMIT: u64 = 4 * 1024 * 1024;
const SETTINGS: &str = "code-themes";
const EDITOR_FIELDS: &[&str] = &[
    "editor.background",
    "editor.foreground",
    "editor.selection",
    "editor.caret",
    "editor.active_line.background",
    "editor.gutter.background",
    "editor.line_number",
    "editor.invisible",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color(pub [u8; 4]);

impl TryFrom<String> for Color {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        let hex = value
            .strip_prefix('#')
            .context("Expected a hexadecimal color")?;
        ensure!(
            matches!(hex.len(), 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "Expected #RRGGBB or #RRGGBBAA"
        );
        let mut rgba = [0, 0, 0, 255];
        for (index, channel) in rgba.iter_mut().take(hex.len() / 2).enumerate() {
            *channel = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)?;
        }
        Ok(Self(rgba))
    }
}

impl From<Color> for String {
    fn from(color: Color) -> Self {
        let [red, green, blue, alpha] = color.0;
        if alpha == 255 {
            format!("#{red:02X}{green:02X}{blue:02X}")
        } else {
            format!("#{red:02X}{green:02X}{blue:02X}{alpha:02X}")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    fn default_id(self) -> String {
        match self {
            Self::Light => "xcode-light",
            Self::Dark => "xcode-dark",
        }
        .to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
    pub colors: BTreeMap<String, Color>,
}

impl Theme {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.chars().count() <= 120
                && !self.name.chars().any(char::is_control),
            "Invalid code theme name"
        );
        ensure!(
            !self.colors.is_empty() && self.colors.len() <= 256,
            "Expected a code theme with up to 256 colors"
        );
        ensure!(
            self.colors.keys().all(|field| {
                EDITOR_FIELDS.contains(&field.as_str())
                    || field.strip_prefix("syntax.").is_some_and(syntax_name)
            }),
            "Invalid code theme color field"
        );
        Ok(())
    }
}

fn syntax_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 72
        && name.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-')
                })
        })
}

#[derive(Serialize, Deserialize)]
struct ThemeFile {
    superfile_code_theme: u8,
    theme: Theme,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub light: Option<String>,
    pub dark: Option<String>,
    custom: BTreeMap<String, Theme>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            light: Some(Appearance::Light.default_id()),
            dark: Some(Appearance::Dark.default_id()),
            custom: BTreeMap::new(),
        }
    }
}

impl Library {
    pub fn theme(&self, id: &str) -> Option<&Theme> {
        self.custom.get(id).or_else(|| {
            bundled()
                .iter()
                .find(|(key, _)| key == id)
                .map(|(_, theme)| theme)
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &Theme)> {
        bundled()
            .iter()
            .map(|(id, theme)| (id.as_str(), theme))
            .chain(self.custom.iter().map(|(id, theme)| (id.as_str(), theme)))
    }

    pub fn selection(&self, appearance: Appearance) -> Option<&str> {
        match appearance {
            Appearance::Light => self.light.as_deref(),
            Appearance::Dark => self.dark.as_deref(),
        }
    }

    pub fn add(&mut self, themes: Vec<Theme>) -> Result<Vec<String>> {
        let incoming = themes
            .into_iter()
            .map(|theme| {
                theme.validate()?;
                let id = format!("local:{:x}", Sha256::digest(serde_json::to_vec(&theme)?));
                Ok((id, theme))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut custom = self.custom.clone();
        let ids = incoming.iter().map(|(id, _)| id.clone()).collect();
        custom.extend(incoming);
        ensure!(custom.len() <= 128, "The code theme library is full");
        self.custom = custom;
        Ok(ids)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        if self.custom.remove(id).is_none() {
            return false;
        }
        for (selected, mode) in [
            (&mut self.light, Appearance::Light),
            (&mut self.dark, Appearance::Dark),
        ] {
            if selected.as_deref() == Some(id) {
                *selected = Some(mode.default_id());
            }
        }
        true
    }
}

fn bundled() -> &'static [(String, Theme)] {
    static BUNDLED: OnceLock<Vec<(String, Theme)>> = OnceLock::new();
    BUNDLED.get_or_init(|| {
        let bytes = include_bytes!("../../../assets/code-themes/presets.json");
        let entries = read_json(bytes).expect("valid bundled code themes");
        let ids = [
            "xcode-light",
            "xcode-dark",
            "catppuccin-latte",
            "catppuccin-mocha",
            "rose-pine-dawn",
            "rose-pine-moon",
        ];
        assert_eq!(
            ids.len(),
            entries.len(),
            "every bundled code theme has an ID"
        );
        ids.into_iter()
            .zip(entries)
            .map(|(id, theme)| (id.to_owned(), theme))
            .collect()
    })
}

pub fn load() -> Result<Library> {
    load_from(&storage::open()?)
}

fn load_from(db: &rusqlite::Connection) -> Result<Library> {
    let Some(value) = storage::setting(db, SETTINGS)? else {
        return Ok(Library::default());
    };
    let mut library: Library = serde_json::from_str(&value)?;
    ensure!(
        library.custom.len() <= 128,
        "The code theme library is full"
    );
    for theme in library.custom.values() {
        theme.validate()?;
    }
    for mode in [Appearance::Light, Appearance::Dark] {
        if library
            .selection(mode)
            .is_some_and(|id| library.theme(id).is_none())
        {
            match mode {
                Appearance::Light => library.light = Some(mode.default_id()),
                Appearance::Dark => library.dark = Some(mode.default_id()),
            }
        }
    }
    Ok(library)
}

pub fn save(library: &Library) -> Result<()> {
    save_to(&storage::open()?, library)
}

fn save_to(db: &rusqlite::Connection, library: &Library) -> Result<()> {
    storage::set_setting(db, SETTINGS, &serde_json::to_string(library)?)
}

pub fn import(path: &Path) -> Result<Vec<Theme>> {
    ensure!(path.is_file(), "Select a regular theme file");
    let mut bytes = Vec::new();
    File::open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "Code theme file exceeds 4 MiB");
    let extension = path
        .extension()
        .and_then(|suffix| suffix.to_str())
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "json" => read_json(&bytes),
        "xccolortheme" | "dvtcolortheme" => {
            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("Xcode");
            Ok(vec![read_xcode(&bytes, name)?])
        }
        _ => anyhow::bail!("Expected a Zed or Superfile JSON theme or an Xcode .xccolortheme file"),
    }
}

pub fn export(path: &Path, theme: Theme) -> Result<()> {
    theme.validate()?;
    let directory = path.parent().context("Select a destination directory")?;
    let mut output = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer_pretty(
        &mut output,
        &ThemeFile {
            superfile_code_theme: 1,
            theme,
        },
    )?;
    output.write_all(b"\n")?;
    output.as_file().sync_all()?;
    output.persist(path)?;
    Ok(())
}

fn read_json(bytes: &[u8]) -> Result<Vec<Theme>> {
    let value: Value =
        serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))?;
    if value.get("superfile_code_theme").is_some() {
        let file: ThemeFile = serde_json::from_value(value)?;
        ensure!(
            file.superfile_code_theme == 1,
            "Unsupported Superfile code theme version"
        );
        file.theme.validate()?;
        return Ok(vec![file.theme]);
    }
    let variants = value
        .get("themes")
        .and_then(Value::as_array)
        .context("Expected a Zed JSON theme family")?;
    ensure!(
        !variants.is_empty() && variants.len() <= 128,
        "Invalid Zed theme family size"
    );
    variants.iter().map(read_zed).collect()
}

fn put(colors: &mut BTreeMap<String, Color>, key: &str, value: Option<Color>) {
    if let Some(color) = value {
        colors.insert(key.to_owned(), color);
    }
}

fn read_zed(value: &Value) -> Result<Theme> {
    let style = value
        .get("style")
        .and_then(Value::as_object)
        .context("Missing Zed theme style")?;
    let mut colors = BTreeMap::new();
    for &field in EDITOR_FIELDS {
        put(&mut colors, field, zed_color(style.get(field)));
    }
    put(
        &mut colors,
        "editor.selection",
        zed_color(
            style
                .get("players")
                .and_then(|players| players.get(0))
                .and_then(|player| player.get("selection")),
        ),
    );
    put(
        &mut colors,
        "editor.caret",
        zed_color(
            style
                .get("players")
                .and_then(|players| players.get(0))
                .and_then(|player| player.get("cursor")),
        ),
    );
    if let Some(syntax) = style.get("syntax").and_then(Value::as_object) {
        for (token, entry) in syntax {
            if syntax_name(token) {
                put(
                    &mut colors,
                    &format!("syntax.{token}"),
                    zed_color(entry.get("color")),
                );
            }
        }
    }
    let theme = Theme {
        name: value
            .get("name")
            .and_then(Value::as_str)
            .context("Missing Zed theme name")?
            .to_owned(),
        appearance: serde_json::from_value(
            value
                .get("appearance")
                .cloned()
                .context("Missing Zed theme appearance")?,
        )?,
        colors,
    };
    theme.validate()?;
    Ok(theme)
}

fn zed_color(value: Option<&Value>) -> Option<Color> {
    Color::try_from(value?.as_str()?.to_owned()).ok()
}

fn read_xcode(bytes: &[u8], name: &str) -> Result<Theme> {
    let value = plist_parser::Value::from_reader(std::io::Cursor::new(bytes))?;
    let info = value.as_dictionary().context("Invalid Xcode theme")?;
    let background = xcode_color(info.get("DVTSourceTextBackground"))
        .context("Missing Xcode editor background")?;
    let mut colors = BTreeMap::from([("editor.background".to_owned(), background)]);
    for (from, to) in [
        ("DVTSourceTextSelectionColor", "editor.selection"),
        ("DVTSourceTextInsertionPointColor", "editor.caret"),
        (
            "DVTSourceTextCurrentLineHighlightColor",
            "editor.active_line.background",
        ),
        ("DVTSourceTextInvisiblesColor", "editor.invisible"),
    ] {
        put(&mut colors, to, xcode_color(info.get(from)));
    }
    if let Some(syntax) = info
        .get("DVTSourceTextSyntaxColors")
        .and_then(plist_parser::Value::as_dictionary)
    {
        for (from, to) in [
            ("plain", "editor.foreground"),
            ("comment", "syntax.comment"),
            ("comment.doc", "syntax.comment.doc"),
            ("comment.doc.keyword", "syntax.comment.doc.keyword"),
            ("keyword", "syntax.keyword"),
            ("string", "syntax.string"),
            ("character", "syntax.string.special"),
            ("number", "syntax.number"),
            ("identifier.constant", "syntax.constant"),
            ("identifier.type", "syntax.type"),
            ("identifier.class", "syntax.type.builtin"),
            ("identifier.function", "syntax.function"),
            ("identifier.variable", "syntax.variable"),
            ("identifier.macro", "syntax.function.macro"),
            ("preprocessor", "syntax.preproc"),
            ("attribute", "syntax.attribute"),
            ("url", "syntax.link_uri"),
        ] {
            let key = format!("xcode.syntax.{from}");
            put(&mut colors, to, xcode_color(syntax.get(&key)));
        }
    }
    let [red, green, blue, _] = background.0;
    let lightness = u32::from(red) * 2126 + u32::from(green) * 7152 + u32::from(blue) * 722;
    let theme = Theme {
        name: name.to_owned(),
        appearance: if lightness >= 128 * 10000 {
            Appearance::Light
        } else {
            Appearance::Dark
        },
        colors,
    };
    theme.validate()?;
    Ok(theme)
}

fn xcode_color(value: Option<&plist_parser::Value>) -> Option<Color> {
    let mut channels = value?.as_string()?.split_whitespace();
    let mut rgba = [0; 4];
    for channel in &mut rgba {
        let value: f32 = channels.next()?.parse().ok()?;
        if !value.is_finite() {
            return None;
        }
        *channel = (value.clamp(0., 1.) * 255.).round() as u8;
    }
    channels.next().is_none().then_some(Color(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xcode_defaults_and_independent_selections_survive_reopening() -> Result<()> {
        let profile = tempfile::tempdir()?;
        let db = storage::open_at(profile.path())?;
        let mut library = load_from(&db)?;
        assert_eq!(library.entries().count(), 6);
        assert_eq!(library.selection(Appearance::Light), Some("xcode-light"));
        assert_eq!(library.selection(Appearance::Dark), Some("xcode-dark"));
        assert_eq!(
            library.theme("xcode-dark").unwrap().colors["editor.background"],
            Color([0x29, 0x2a, 0x30, 255])
        );
        library.light = None;
        library.dark = Some("rose-pine-moon".into());
        save_to(&db, &library)?;
        drop(db);
        let db = storage::open_at(profile.path())?;
        let library = load_from(&db)?;
        assert_eq!(library.selection(Appearance::Light), None);
        assert_eq!(library.selection(Appearance::Dark), Some("rose-pine-moon"));
        Ok(())
    }

    #[test]
    fn xcode_import_and_custom_json_round_trip() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let input = directory.path().join("Xcode Test.xccolortheme");
        std::fs::write(
            &input,
            br#"<?xml version="1.0"?><plist version="1.0"><dict>
                <key>DVTSourceTextBackground</key><string>0 0.5 0.25 1</string>
                <key>DVTSourceTextSelectionColor</key><string>1 0 0 0.25</string>
                <key>DVTSourceTextSyntaxColors</key><dict>
                    <key>xcode.syntax.keyword</key><string>1 0 0.5 1</string>
                    <key>xcode.syntax.string</key><string>NaN 0 1 1</string>
                </dict>
            </dict></plist>"#,
        )?;
        let mut themes = import(&input)?;
        let mut theme = themes.pop().unwrap();
        assert_eq!(theme.appearance, Appearance::Dark);
        assert_eq!(theme.colors["syntax.keyword"], Color([255, 0, 128, 255]));
        assert_eq!(theme.colors["editor.selection"], Color([255, 0, 0, 64]));
        assert!(!theme.colors.contains_key("syntax.string"));
        theme.name = "Custom".into();
        theme
            .colors
            .insert("syntax.string".into(), Color([1, 2, 3, 220]));
        let path = directory.path().join("export.json");
        export(&path, theme.clone())?;
        assert_eq!(import(&path)?, vec![theme.clone()]);

        let mut library = Library::default();
        let id = library.add(vec![theme.clone()])?.remove(0);
        let same = library.add(vec![theme])?.remove(0);
        assert_eq!(id, same);
        library.dark = Some(id.clone());
        assert!(library.remove(&id));
        assert_eq!(library.selection(Appearance::Dark), Some("xcode-dark"));
        assert_eq!(library.selection(Appearance::Light), Some("xcode-light"));
        Ok(())
    }
}
