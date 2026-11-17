mod palette;
use gpui_kit::{
    component::{Colorize, Theme},
    *,
};
use std::sync::{Arc, OnceLock};
use superfile_services::{
    preferences::{AccentColor, ThemePreferences},
    themes::{Mode, Scheme},
};

pub(crate) struct AccentPreference {
    preferences: ThemePreferences,
    scheme: Arc<Scheme>,
    error: Option<String>,
    write: Option<Task<()>>,
}
impl Global for AccentPreference {}

pub(crate) fn init(
    preferences: ThemePreferences,
    loaded: anyhow::Result<Option<Scheme>>,
    cx: &mut App,
) {
    let (scheme, error) = match loaded {
        Ok(scheme) => (scheme.map(Arc::new).unwrap_or_else(default_scheme), None),
        Err(error) => (default_scheme(), Some(format!("{error:#}"))),
    };
    cx.set_global(AccentPreference {
        preferences,
        scheme,
        error,
        write: None,
    });
}

fn default_scheme() -> Arc<Scheme> {
    static DEFAULT: OnceLock<Arc<Scheme>> = OnceLock::new();
    DEFAULT
        .get_or_init(|| Arc::new(superfile_services::themes::default_scheme()))
        .clone()
}

pub(crate) fn scheme(cx: &App) -> Arc<Scheme> {
    cx.try_global::<AccentPreference>()
        .map_or_else(default_scheme, |preference| preference.scheme.clone())
}

pub(crate) fn selected_id(cx: &App) -> Option<&str> {
    cx.try_global::<AccentPreference>()
        .and_then(|preference| preference.scheme.id.as_deref())
}

pub(crate) fn load_error(cx: &App) -> Option<&str> {
    cx.try_global::<AccentPreference>()
        .and_then(|preference| preference.error.as_deref())
}

pub(crate) fn has_custom_accent(cx: &App) -> bool {
    cx.try_global::<AccentPreference>()
        .is_some_and(|preference| preference.preferences.accent.is_some())
}

fn mode(cx: &App) -> Mode {
    if Theme::global(cx).is_dark() {
        Mode::Dark
    } else {
        Mode::Light
    }
}

pub(crate) fn color(cx: &App) -> AccentColor {
    if let Some(color) = cx
        .try_global::<AccentPreference>()
        .and_then(|preference| preference.preferences.accent)
    {
        return color;
    }
    let scheme = scheme(cx);
    if scheme.id.is_none() {
        return AccentColor::default();
    }
    let rgba = scheme.palette(mode(cx)).get("--interactive-accent").0;
    let channel = |index: usize| (rgba[index] * 255.).round() as u32;
    AccentColor::from_rgb(channel(0) << 16 | channel(1) << 8 | channel(2))
}

pub(crate) fn accent(cx: &App) -> Hsla {
    let mut accent: Hsla = rgb(color(cx).rgb()).into();
    let scheme = scheme(cx);
    let colors = scheme.palette(mode(cx));
    if scheme.id.is_some() && !has_custom_accent(cx) {
        return palette::color(colors.get("--text-accent"));
    }
    let background = palette::color(colors.get("--background-primary"));
    // Adapt only the displayed ink; the picker and saved RGB retain the choice.
    let step = if luminance(background) < 0.5 {
        0.02
    } else {
        -0.02
    };
    for _ in 0..50 {
        if contrast(accent, background) >= 4.5 {
            break;
        }
        accent.l = (accent.l + step).clamp(0., 1.);
    }
    accent
}

fn luminance(color: Hsla) -> f32 {
    let rgb = color.to_rgb();
    let linear = |value: f32| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
}

fn contrast(left: Hsla, right: Hsla) -> f32 {
    let left = luminance(left);
    let right = luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

pub(crate) fn apply(cx: &mut App) {
    let accent = accent(cx);
    let scheme = scheme(cx);
    let colors = scheme.palette(mode(cx));
    let override_accent = scheme.id.is_none() || has_custom_accent(cx);
    Theme::update(cx, |theme| {
        palette::apply(theme, colors);
        if !override_accent {
            return;
        }
        let surface = theme.background.blend(accent.opacity(0.14));
        let foreground = theme.foreground;
        let primary_foreground = if contrast(accent, black()) > contrast(accent, white()) {
            black()
        } else {
            white()
        };
        theme.primary = accent;
        theme.primary_hover = if primary_foreground == black() {
            accent.lighten(0.04)
        } else {
            accent.darken(0.04)
        };
        theme.primary_active = if primary_foreground == black() {
            accent.lighten(0.08)
        } else {
            accent.darken(0.08)
        };
        theme.primary_foreground = primary_foreground;
        theme.button_primary = accent;
        theme.button_primary_hover = theme.primary_hover;
        theme.button_primary_active = theme.primary_active;
        theme.button_primary_foreground = primary_foreground;
        theme.accent = surface;
        theme.accent_foreground = foreground;
        theme.ring = accent;
        theme.caret = accent;
        theme.link = accent;
        theme.link_hover = accent;
        theme.link_active = accent;
        theme.selection = accent.opacity(0.24);
        theme.list_active_border = accent;
        theme.table_active = surface;
        theme.table_active_border = accent;
        theme.sidebar_primary = accent;
        theme.sidebar_primary_foreground = primary_foreground;
        theme.sidebar_accent_foreground = foreground;
        theme.tab_active_foreground = accent;
        theme.drag_border = accent;
        theme.progress_bar = accent;
        theme.slider_bar = accent;
    });
}

pub(crate) fn set_color(color: AccentColor, cx: &mut App) {
    if super::updates::busy(cx) || cx.global::<AccentPreference>().preferences.accent == Some(color)
    {
        return;
    }
    cx.global_mut::<AccentPreference>().preferences.accent = Some(color);
    apply(cx);
    save(cx);
}

pub(crate) fn reset_accent(cx: &mut App) {
    if super::updates::busy(cx) || !has_custom_accent(cx) {
        return;
    }
    cx.global_mut::<AccentPreference>().preferences.accent = None;
    apply(cx);
    save(cx);
}

pub(crate) fn use_scheme(scheme: Scheme, cx: &mut App) {
    if super::updates::busy(cx) {
        return;
    }
    let preference = cx.global_mut::<AccentPreference>();
    preference.preferences.scheme = scheme.id.clone();
    preference.preferences.accent = None;
    preference.scheme = Arc::new(scheme);
    preference.error = None;
    apply(cx);
    save(cx);
}

fn save(cx: &mut App) {
    let preference = cx.global_mut::<AccentPreference>();
    let preferences = preference.preferences.clone();
    let previous = preference.write.take();
    let write = cx.spawn(async move |cx| {
        if let Some(previous) = previous {
            previous.await;
        }
        let result =
            cx.background_executor()
                .spawn(async move {
                    superfile_services::preferences::save_theme_preferences(&preferences)
                })
                .await;
        if let Err(error) = result {
            log::error!("Cannot remember theme settings: {error:#}");
            cx.update(|cx| {
                if let Some((_, handle)) = cx
                    .try_global::<super::OpenWorkspaces>()
                    .and_then(|workspaces| workspaces.0.last().cloned())
                {
                    let _ = handle.update(cx, |_, window, cx| {
                        drop(window.prompt(
                            PromptLevel::Warning,
                            &superfile_services::locale::tr("Cannot remember theme settings"),
                            Some(&format!("{error:#}")),
                            &[superfile_services::locale::tr("OK").as_str()],
                            cx,
                        ));
                    });
                }
            });
        }
    });
    cx.global_mut::<AccentPreference>().write = Some(write);
}

pub(crate) fn take_write(cx: &mut App) -> Option<Task<()>> {
    cx.global_mut::<AccentPreference>().write.take()
}

#[cfg(test)]
mod tests {
    use super::{AccentColor, accent, apply, color, contrast, init};
    use gpui_kit::{TestAppContext, component::Theme};
    use superfile_services::preferences::Appearance;

    #[gpui_kit::test]
    fn community_palettes_apply_to_controls_editor_and_both_appearances(cx: &mut TestAppContext) {
        use superfile_services::{preferences::ThemePreferences, themes, work::Cancellation};
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            let entry = themes::CommunityTheme {
                name: "Fixture".into(), author: "Test".into(), repo: "test/palette".into(),
                screenshot: String::new(), modes: vec![], branch: None, legacy: false,
            };
            let scheme = themes::compile(&entry, "body { --code-keyword: #e040a0; --interactive-accent: #20a080; } .theme-dark { --background-primary: #182030; --text-normal: #d0e0f0; } .theme-light { --background-primary: #f4f0e8; --text-normal: #203040; }", &Cancellation::default()).unwrap();
            let preferences = ThemePreferences { scheme: Some(entry.id()), accent: None };
            init(preferences.clone(), Ok(Some(scheme.clone())), cx);
            for (appearance, background) in [(Appearance::Dark, 0x182030), (Appearance::Light, 0xf4f0e8)] {
                super::super::apply_appearance(appearance, cx);
                let theme = Theme::global(cx);
                let actual = theme.background.to_rgb();
                let expected = gpui_kit::rgb(background);
                assert!((actual.r - expected.r).abs() < 0.00001);
                assert!((actual.g - expected.g).abs() < 0.00001);
                assert!((actual.b - expected.b).abs() < 0.00001);
                assert_eq!(theme.highlight_theme.style.editor_background, Some(theme.background));
                assert_eq!(theme.button_primary, theme.primary);
                assert_eq!(theme.tokens.primary.color, theme.primary);
                let syntax = serde_json::to_value(&theme.highlight_theme.style.syntax).unwrap();
                assert_eq!(syntax["keyword"]["color"], serde_json::json!(super::palette::color(themes::CssColor::rgb(0xe040a0))));
                assert_eq!(super::selected_id(cx), Some(entry.id().as_str()));
                assert!(!super::has_custom_accent(cx));
            }
            init(ThemePreferences { accent: Some(AccentColor::PURPLE), ..preferences }, Ok(Some(scheme)), cx);
            apply(cx);
            assert_eq!(color(cx), AccentColor::PURPLE);
            assert_eq!(Theme::global(cx).primary, accent(cx));
            init(ThemePreferences::default(), Err(anyhow::anyhow!("Missing package")), cx);
            apply(cx);
            assert_eq!(super::selected_id(cx), None);
            assert!(super::load_error(cx).unwrap().contains("Missing package"));
        });
    }

    #[gpui_kit::test]
    fn custom_accent_survives_appearance_changes_and_syncs_component_tokens(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            let custom = AccentColor::from_rgb(0x06b6d4);
            init(
                superfile_services::preferences::ThemePreferences {
                    scheme: None,
                    accent: Some(custom),
                },
                Ok(None),
                cx,
            );
            for appearance in [Appearance::Light, Appearance::Dark, Appearance::System] {
                super::super::apply_appearance(appearance, cx);
                let theme = Theme::global(cx);
                assert_eq!(color(cx), custom);
                assert_eq!(theme.primary, accent(cx));
                assert_eq!(theme.ring, theme.primary);
                assert_eq!(theme.button_primary, theme.primary);
                assert_eq!(theme.tokens.primary.color, theme.primary);
                assert_eq!(
                    gpui_kit::base::Theme::global(cx).tokens.colors.primary,
                    theme.primary
                );
            }
        });
    }

    #[gpui_kit::test]
    fn accent_text_and_primary_labels_remain_readable_for_extreme_colors(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            for appearance in [Appearance::Light, Appearance::Dark] {
                super::super::apply_appearance(appearance, cx);
                for rgb in [0x8b5cf6, 0x000000, 0xffffff, 0xffff00, 0x00ffff, 0xff0000] {
                    let original = AccentColor::from_rgb(rgb);
                    init(
                        superfile_services::preferences::ThemePreferences {
                            scheme: None,
                            accent: Some(original),
                        },
                        Ok(None),
                        cx,
                    );
                    apply(cx);
                    let theme = Theme::global(cx);
                    assert!(
                        contrast(theme.link, theme.background) >= 4.5,
                        "{appearance:?}: {rgb:06X}"
                    );
                    assert!(contrast(theme.button_primary, theme.button_primary_foreground) >= 4.5);
                    assert!(
                        contrast(theme.button_primary_hover, theme.button_primary_foreground)
                            >= 4.5
                    );
                    assert!(
                        contrast(theme.button_primary_active, theme.button_primary_foreground)
                            >= 4.5
                    );
                    assert_eq!(color(cx), original);
                }
            }
        });
    }
}
