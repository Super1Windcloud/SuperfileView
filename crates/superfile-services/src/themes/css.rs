use super::{
    Mode, Palette,
    values::{self, Resolver},
};
use crate::work::Cancellation;
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

// Obsidian's published foundation palette and semantic variables.
// https://docs.obsidian.md/Reference/CSS+variables/Foundations/Colors
fn defaults(mode: Mode) -> BTreeMap<String, String> {
    let base = match mode {
        Mode::Light => [
            0xffffff, 0xfcfcfc, 0xfafafa, 0xf6f6f6, 0xefefef, 0xe4e4e4, 0xdadada, 0xbdbdbd,
            0xababab, 0x707070, 0x5c5c5c, 0x222222,
        ],
        Mode::Dark => [
            0x1c1c1c, 0x212121, 0x232323, 0x282828, 0x2e2e2e, 0x333333, 0x3f3f3f, 0x555555,
            0x666666, 0x999999, 0xb3b3b3, 0xdadada,
        ],
    };
    let mut vars = BTreeMap::new();
    for (level, value) in [
        "00", "05", "10", "20", "25", "30", "35", "40", "50", "60", "70", "100",
    ]
    .into_iter()
    .zip(base)
    {
        vars.insert(format!("--color-base-{level}"), format!("#{value:06x}"));
    }
    let status = match mode {
        Mode::Light => [
            0xe93147, 0xec7500, 0xe0ac00, 0x08b94e, 0x00bfbc, 0x086ddd, 0x7852ee, 0xd53984,
        ],
        Mode::Dark => [
            0xfb464c, 0xe9973f, 0xe0de71, 0x44cf6e, 0x53dfdd, 0x027aff, 0xa882ff, 0xfa99cd,
        ],
    };
    for (name, value) in [
        "red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink",
    ]
    .into_iter()
    .zip(status)
    {
        vars.insert(format!("--color-{name}"), format!("#{value:06x}"));
    }
    for (name, value) in [
        ("--accent-h", "258"),
        ("--accent-s", "88%"),
        ("--accent-l", "66%"),
        (
            "--color-accent",
            "hsl(var(--accent-h), var(--accent-s), var(--accent-l))",
        ),
        (
            "--color-accent-1",
            "hsl(var(--accent-h), var(--accent-s), calc(var(--accent-l) + 4%))",
        ),
        (
            "--color-accent-2",
            "hsl(var(--accent-h), var(--accent-s), calc(var(--accent-l) + 8%))",
        ),
        (
            "--color-accent-hsl",
            "var(--accent-h), var(--accent-s), var(--accent-l)",
        ),
        ("--background-primary", "var(--color-base-00)"),
        ("--background-primary-alt", "var(--color-base-05)"),
        ("--background-secondary", "var(--color-base-10)"),
        ("--background-secondary-alt", "var(--color-base-20)"),
        ("--background-modifier-border", "var(--color-base-30)"),
        ("--background-modifier-border-hover", "var(--color-base-35)"),
        ("--background-modifier-border-focus", "var(--color-base-40)"),
        (
            "--background-modifier-form-field",
            "var(--background-primary)",
        ),
        (
            "--background-modifier-hover",
            "color-mix(in srgb, var(--text-normal) 7%, transparent)",
        ),
        ("--interactive-normal", "var(--background-primary-alt)"),
        ("--interactive-hover", "var(--background-secondary-alt)"),
        ("--interactive-accent", "var(--color-accent)"),
        ("--interactive-accent-hsl", "var(--color-accent-hsl)"),
        ("--interactive-accent-hover", "var(--color-accent-1)"),
        ("--text-normal", "var(--color-base-100)"),
        ("--text-muted", "var(--color-base-70)"),
        ("--text-faint", "var(--color-base-50)"),
        ("--text-on-accent", "white"),
        ("--text-accent", "var(--color-accent)"),
        ("--text-accent-hover", "var(--color-accent-1)"),
        (
            "--text-selection",
            "color-mix(in srgb, var(--color-accent) 24%, transparent)",
        ),
        ("--caret-color", "var(--text-normal)"),
        ("--link-color", "var(--text-accent)"),
        ("--link-color-hover", "var(--text-accent-hover)"),
        (
            "--nav-item-background-active",
            "var(--background-modifier-hover)",
        ),
        (
            "--nav-item-background-hover",
            "var(--background-modifier-hover)",
        ),
        ("--nav-item-color", "var(--text-muted)"),
        ("--nav-item-color-active", "var(--text-normal)"),
        ("--tab-background-active", "var(--background-primary)"),
        ("--tab-text-color-focused-active", "var(--text-normal)"),
        ("--titlebar-background", "var(--background-secondary)"),
        ("--code-background", "var(--background-secondary)"),
        ("--code-normal", "var(--text-normal)"),
        ("--code-comment", "var(--text-faint)"),
        ("--code-function", "var(--color-yellow)"),
        ("--code-keyword", "var(--color-pink)"),
        ("--code-operator", "var(--color-cyan)"),
        ("--code-property", "var(--color-blue)"),
        ("--code-punctuation", "var(--text-muted)"),
        ("--code-string", "var(--color-green)"),
        ("--code-tag", "var(--color-red)"),
        ("--code-value", "var(--color-purple)"),
    ] {
        vars.insert(name.into(), value.into());
    }
    vars
}

const COLORS: &[&str] = &[
    "--color-accent",
    "--color-accent-1",
    "--color-accent-2",
    "--background-primary",
    "--background-primary-alt",
    "--background-secondary",
    "--background-secondary-alt",
    "--background-modifier-border",
    "--background-modifier-border-hover",
    "--background-modifier-border-focus",
    "--background-modifier-form-field",
    "--background-modifier-hover",
    "--interactive-normal",
    "--interactive-hover",
    "--interactive-accent",
    "--interactive-accent-hover",
    "--text-normal",
    "--text-muted",
    "--text-faint",
    "--text-on-accent",
    "--text-accent",
    "--text-accent-hover",
    "--text-selection",
    "--caret-color",
    "--link-color",
    "--link-color-hover",
    "--nav-item-background-active",
    "--nav-item-background-hover",
    "--nav-item-color",
    "--nav-item-color-active",
    "--tab-background-active",
    "--tab-text-color-focused-active",
    "--titlebar-background",
    "--code-background",
    "--code-normal",
    "--code-comment",
    "--code-function",
    "--code-keyword",
    "--code-operator",
    "--code-property",
    "--code-punctuation",
    "--code-string",
    "--code-tag",
    "--code-value",
    "--color-red",
    "--color-orange",
    "--color-yellow",
    "--color-green",
    "--color-cyan",
    "--color-blue",
    "--color-purple",
    "--color-pink",
];

fn strip_comments(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut at = 0;
    let mut quote = None;
    let mut escape = false;
    while at < bytes.len() {
        let byte = bytes[at];
        if escape {
            result.push(byte);
            escape = false;
            at += 1;
            continue;
        }
        if byte == b'\\' {
            result.push(byte);
            escape = true;
            at += 1;
            continue;
        }
        if let Some(current) = quote {
            result.push(byte);
            if byte == current {
                quote = None;
            }
            at += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            result.push(byte);
            at += 1;
            continue;
        }
        if bytes.get(at..at + 2) == Some(b"/*") {
            at += 2;
            while at < bytes.len() && bytes.get(at..at + 2) != Some(b"*/") {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
            result.push(b' ');
        } else {
            result.push(byte);
            at += 1;
        }
    }
    String::from_utf8(result).expect("comments are removed on UTF-8 boundaries")
}

fn root_matches(selector: &str, mode: Mode, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    let mut selector = selector.trim().to_owned();
    for function in [":not(", ":is(", ":where("] {
        while let Some(index) = selector.find(function) {
            let Some(end) = values::closing(&selector, index + function.len() - 1, b'(', b')')
            else {
                return false;
            };
            let matched = values::split_top(&selector[index + function.len()..end], ',')
                .into_iter()
                .any(|argument| root_matches(argument, mode, depth + 1));
            if matched == (function == ":not(") {
                return false;
            }
            selector.replace_range(index..=end, "");
        }
    }
    if selector.is_empty() {
        return true;
    }
    if selector
        .bytes()
        .any(|ch| ch.is_ascii_whitespace() || b">+~[]#".contains(&ch))
    {
        return false;
    }
    let selector = selector
        .strip_prefix("body")
        .or_else(|| selector.strip_prefix("html"))
        .unwrap_or(&selector);
    let selector = selector.strip_prefix(":root").unwrap_or(selector);
    if selector.is_empty() || selector == "*" {
        return true;
    }
    selector.split('.').enumerate().all(|(index, class)| {
        if index == 0 {
            return class.is_empty();
        }
        match class {
            "theme-light" => mode == Mode::Light,
            "theme-dark" => mode == Mode::Dark,
            "is-desktop" => true,
            "mod-macos" => cfg!(target_os = "macos"),
            "mod-windows" => cfg!(windows),
            "mod-linux" => cfg!(target_os = "linux"),
            _ => false,
        }
    })
}

#[derive(Default)]
struct Rules {
    values: BTreeMap<String, (bool, usize, usize, String)>,
    order: usize,
}
impl Rules {
    fn read(&mut self, text: &str, mode: Mode, cancel: &Cancellation, depth: usize) -> Result<()> {
        ensure!(depth < 64, "Theme rule nesting exceeds 64");
        let mut at = 0;
        while at < text.len() {
            cancel.check()?;
            let Some(open) = text[at..].find('{').map(|index| at + index) else {
                break;
            };
            let Some(end) = values::closing(text, open, b'{', b'}') else {
                break;
            };
            let prefix = values::split_top(&text[at..open], ';');
            let header = prefix.last().copied().unwrap_or_default().trim();
            let body = &text[open + 1..end];
            if header.starts_with("@media") {
                let normalized = header.to_ascii_lowercase().replace(' ', "");
                let scheme = if normalized.contains("prefers-color-scheme:dark") {
                    Some(Mode::Dark)
                } else if normalized.contains("prefers-color-scheme:light") {
                    Some(Mode::Light)
                } else {
                    None
                };
                if scheme == Some(mode)
                    || matches!(normalized.as_str(), "@mediaall" | "@mediascreen")
                {
                    self.read(body, mode, cancel, depth + 1)?;
                }
            } else if header.starts_with("@layer") {
                self.read(body, mode, cancel, depth + 1)?;
            } else if !header.starts_with('@') {
                let specificity = values::split_top(header, ',')
                    .into_iter()
                    .filter(|selector| root_matches(selector, mode, 0))
                    .map(|selector| {
                        selector.matches('.').count() * 10
                            + selector.matches(':').count() * 10
                            + usize::from(selector.trim().starts_with("body"))
                    })
                    .max();
                if let Some(specificity) = specificity {
                    for declaration in values::split_top(body, ';') {
                        let Some((name, raw)) = declaration.split_once(':') else {
                            continue;
                        };
                        let name = name.trim();
                        if !name.starts_with("--")
                            || !name
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
                        {
                            continue;
                        }
                        let raw = raw.trim();
                        let important = raw.to_ascii_lowercase().ends_with("!important");
                        let raw = if important {
                            raw[..raw.len() - 10].trim()
                        } else {
                            raw
                        };
                        self.order += 1;
                        let rank = (important, specificity, self.order);
                        if self
                            .values
                            .get(name)
                            .is_none_or(|old| rank >= (old.0, old.1, old.2))
                        {
                            self.values.insert(
                                name.into(),
                                (important, specificity, self.order, raw.into()),
                            );
                        }
                    }
                }
            }
            at = end + 1;
        }
        Ok(())
    }
}

pub(super) fn compile(css: &str, mode: Mode, cancel: &Cancellation) -> Result<Palette> {
    let mut rules = Rules::default();
    rules.read(&strip_comments(css), mode, cancel, 0)?;
    let original = defaults(mode);
    let mut vars = original.clone();
    let overrides = rules.values.keys().cloned().collect::<BTreeSet<_>>();
    for (name, (_, _, _, value)) in rules.values {
        vars.insert(name, value);
    }
    let empty = BTreeSet::new();
    let mut default = Resolver::new(&original, &empty);
    let mut resolver = Resolver::new(&vars, &overrides);
    let mut palette = Palette {
        colors: BTreeMap::new(),
        applied: 0,
        unresolved: Vec::new(),
    };
    for name in COLORS {
        cancel.check()?;
        match resolver
            .resolve(name)
            .and_then(|(text, used)| values::color(&text).ok().map(|value| (value, used)))
        {
            Some((value, used)) => {
                palette.colors.insert((*name).into(), value);
                palette.applied += usize::from(used);
            }
            None => {
                palette.unresolved.push((*name).into());
                let (value, _) = default.resolve(name).expect("built-in variable");
                palette.colors.insert(
                    (*name).into(),
                    values::color(&value).expect("built-in color"),
                );
            }
        }
    }
    Ok(palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_palettes_match_published_obsidian_surfaces() {
        let light = compile("", Mode::Light, &Cancellation::default()).unwrap();
        let dark = compile("", Mode::Dark, &Cancellation::default()).unwrap();
        super::super::assert_rgb(light.get("--background-primary"), 0xffffff);
        super::super::assert_rgb(dark.get("--background-primary"), 0x1c1c1c);
        super::super::assert_rgb(dark.get("--background-secondary"), 0x232323);
        assert!(light.unresolved.is_empty() && dark.unresolved.is_empty());
    }
    #[test]
    fn mode_specificity_important_and_descendant_isolation() {
        let css = ".theme-dark { --background-primary: #123456 !important; --base-l: 15%; --bg: hsl(0, 0%, calc(var(--base-l) + 5%)); --text-muted: var(--bg); } body { --background-primary: #eeeeee; } .theme-light { --background-primary: #fafafa; } .theme-dark .note { --background-primary: red !important; } @media (prefers-color-scheme: dark) { .theme-dark { --text-normal: #abcdef; } }";
        let dark = compile(css, Mode::Dark, &Cancellation::default()).unwrap();
        super::super::assert_rgb(dark.get("--background-primary"), 0x123456);
        super::super::assert_rgb(dark.get("--text-normal"), 0xabcdef);
        assert!((dark.get("--text-muted").0[0] - 0.2).abs() < 0.001);
        super::super::assert_rgb(
            compile(css, Mode::Light, &Cancellation::default())
                .unwrap()
                .get("--background-primary"),
            0xfafafa,
        );
        let fallback = compile(
            ".theme-dark { --text-normal: var(--missing); }",
            Mode::Dark,
            &Cancellation::default(),
        )
        .unwrap();
        assert!(fallback.unresolved.contains(&"--text-normal".into()));
        super::super::assert_rgb(fallback.get("--text-normal"), 0xdadada);
    }
}
