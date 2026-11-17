use gpui_kit::{component::Theme, *};
use superfile_services::themes::{CssColor, Palette};

pub(super) fn color(color: CssColor) -> Hsla {
    let [r, g, b, a] = color.0;
    Rgba { r, g, b, a }.into()
}

pub(super) fn apply(theme: &mut Theme, palette: &Palette) {
    let get = |name| color(palette.get(name));
    let fallback = if theme.is_dark() {
        rgb(0x1c1c1c)
    } else {
        rgb(0xffffff)
    };
    let background = Hsla::from(fallback).blend(get("--background-primary"));
    let foreground = get("--text-normal");
    let sidebar = background.blend(get("--background-secondary"));
    let border = get("--background-modifier-border");
    let hover = background.blend(get("--background-modifier-hover"));
    theme.background = background;
    theme.foreground = foreground;
    theme.border = border;
    theme.muted = sidebar;
    theme.muted_foreground = get("--text-muted");
    theme.secondary = sidebar;
    theme.secondary_foreground = foreground;
    theme.secondary_hover = hover;
    theme.secondary_active = hover;
    theme.button = get("--interactive-normal");
    theme.button_hover = get("--interactive-hover");
    theme.button_active = get("--interactive-hover");
    theme.button_foreground = foreground;
    theme.button_secondary = theme.button;
    theme.button_secondary_hover = theme.button_hover;
    theme.button_secondary_active = theme.button_active;
    theme.button_secondary_foreground = foreground;
    theme.input = border;
    theme.accordion = sidebar;
    theme.group_box = sidebar;
    theme.group_box_foreground = foreground;
    theme.description_list_label = sidebar;
    theme.description_list_label_foreground = foreground;
    theme.popover = background.blend(get("--background-primary-alt"));
    theme.popover_foreground = foreground;
    theme.title_bar = background.blend(get("--titlebar-background"));
    theme.title_bar_border = border;
    theme.window_border = border;
    theme.status_bar = sidebar;
    theme.status_bar_border = border;
    theme.sidebar = sidebar;
    theme.sidebar_foreground = get("--nav-item-color");
    theme.sidebar_border = border;
    theme.colors.list = background;
    theme.list_even = background;
    theme.list_head = sidebar;
    theme.list_active = sidebar.blend(get("--nav-item-background-active"));
    theme.list_hover = sidebar.blend(get("--nav-item-background-hover"));
    theme.tab = sidebar;
    theme.tab_bar = sidebar;
    theme.tab_bar_segmented = sidebar;
    theme.tab_foreground = get("--text-muted");
    theme.tab_active = background.blend(get("--tab-background-active"));
    theme.tab_active_foreground = get("--tab-text-color-focused-active");
    theme.primary = get("--interactive-accent");
    theme.primary_hover = get("--interactive-accent-hover");
    theme.primary_active = get("--color-accent-2");
    theme.primary_foreground = get("--text-on-accent");
    theme.button_primary = theme.primary;
    theme.button_primary_hover = theme.primary_hover;
    theme.button_primary_active = theme.primary_active;
    theme.button_primary_foreground = theme.primary_foreground;
    theme.accent = hover;
    theme.accent_foreground = foreground;
    theme.ring = get("--background-modifier-border-focus");
    theme.caret = get("--caret-color");
    theme.link = get("--link-color");
    theme.link_hover = get("--link-color-hover");
    theme.link_active = theme.link_hover;
    theme.selection = get("--text-selection");
    theme.list_active_border = theme.primary;
    theme.table_active = background.blend(theme.selection);
    theme.table_active_border = theme.primary;
    theme.table = background;
    theme.table_even = background;
    theme.table_head = sidebar;
    theme.table_head_foreground = foreground;
    theme.table_foot = sidebar;
    theme.table_foot_foreground = foreground;
    theme.table_hover = hover;
    theme.table_row_border = border;
    theme.sidebar_primary = theme.primary;
    theme.sidebar_primary_foreground = theme.primary_foreground;
    theme.sidebar_accent = theme.list_active;
    theme.sidebar_accent_foreground = get("--nav-item-color-active");
    theme.drag_border = theme.primary;
    theme.progress_bar = theme.primary;
    theme.slider_bar = theme.primary;
    theme.switch = border;
    theme.switch_thumb = foreground;
    theme.slider_thumb = foreground;
    theme.scrollbar = background.opacity(0.);
    theme.scrollbar_thumb = get("--text-faint").opacity(0.4);
    theme.scrollbar_thumb_hover = get("--text-muted").opacity(0.6);
    theme.skeleton = border;
    theme.drop_target = background.blend(theme.selection);
    theme.red = get("--color-red");
    theme.green = get("--color-green");
    theme.yellow = get("--color-yellow");
    theme.blue = get("--color-blue");
    theme.cyan = get("--color-cyan");
    theme.magenta = get("--color-pink");
    theme.red_light = background.blend(theme.red.opacity(0.15));
    theme.green_light = background.blend(theme.green.opacity(0.15));
    theme.yellow_light = background.blend(theme.yellow.opacity(0.15));
    theme.blue_light = background.blend(theme.blue.opacity(0.15));
    theme.cyan_light = background.blend(theme.cyan.opacity(0.15));
    theme.magenta_light = background.blend(theme.magenta.opacity(0.15));
    theme.danger = theme.red;
    theme.success = theme.green;
    theme.warning = get("--color-orange");
    theme.info = theme.blue;
    let ink = |color| {
        if super::contrast(color, black()) > super::contrast(color, white()) {
            black()
        } else {
            white()
        }
    };
    theme.danger_foreground = ink(theme.danger);
    theme.success_foreground = ink(theme.success);
    theme.warning_foreground = ink(theme.warning);
    theme.info_foreground = ink(theme.info);
    theme.danger_hover = background.blend(theme.danger.opacity(0.85));
    theme.danger_active = background.blend(theme.danger.opacity(0.7));
    theme.success_hover = background.blend(theme.success.opacity(0.85));
    theme.success_active = background.blend(theme.success.opacity(0.7));
    theme.warning_hover = background.blend(theme.warning.opacity(0.85));
    theme.warning_active = background.blend(theme.warning.opacity(0.7));
    theme.info_hover = background.blend(theme.info.opacity(0.85));
    theme.info_active = background.blend(theme.info.opacity(0.7));
    theme.button_danger = theme.danger;
    theme.button_danger_foreground = theme.danger_foreground;
    theme.button_danger_hover = theme.danger_hover;
    theme.button_danger_active = theme.danger_active;
    theme.button_success = theme.success;
    theme.button_success_foreground = theme.success_foreground;
    theme.button_success_hover = theme.success_hover;
    theme.button_success_active = theme.success_active;
    theme.button_warning = theme.warning;
    theme.button_warning_foreground = theme.warning_foreground;
    theme.button_warning_hover = theme.warning_hover;
    theme.button_warning_active = theme.warning_active;
    theme.button_info = theme.info;
    theme.button_info_foreground = theme.info_foreground;
    theme.button_info_hover = theme.info_hover;
    theme.button_info_active = theme.info_active;

    let highlight = std::sync::Arc::make_mut(&mut theme.highlight_theme);
    highlight.style.editor_background = Some(background);
    highlight.style.editor_foreground = Some(get("--code-normal"));
    highlight.style.editor_gutter_background = Some(background);
    highlight.style.editor_line_number = Some(get("--text-faint"));
    highlight.style.editor_active_line_number = Some(foreground);
    if let Ok(serde_json::Value::Object(mut syntax)) = serde_json::to_value(&highlight.style.syntax)
    {
        for (token, variable) in [
            ("comment", "--code-comment"),
            ("comment_doc", "--code-comment"),
            ("function", "--code-function"),
            ("keyword", "--code-keyword"),
            ("operator", "--code-operator"),
            ("property", "--code-property"),
            ("punctuation", "--code-punctuation"),
            ("punctuation.bracket", "--code-punctuation"),
            ("punctuation.delimiter", "--code-punctuation"),
            ("string", "--code-string"),
            ("tag", "--code-tag"),
            ("number", "--code-value"),
            ("constant", "--code-value"),
            ("boolean", "--code-value"),
        ] {
            let entry = syntax
                .entry(token.to_owned())
                .or_insert_with(|| serde_json::json!({}));
            if entry.is_null() {
                *entry = serde_json::json!({});
            }
            if let Some(entry) = entry.as_object_mut() {
                entry.insert("color".into(), serde_json::json!(get(variable)));
            }
        }
        if let Ok(syntax) = serde_json::from_value(serde_json::Value::Object(syntax)) {
            highlight.style.syntax = syntax;
        }
    }
}
