use super::Workspace;
use crate::app::actions::{CheckForUpdates, DefaultApplications, InstallCli, SupportedFormats};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Icon, Sizable,
        button::Button,
        color_picker::ColorPicker,
        h_flex,
        menu::{DropdownMenu, PopupMenuItem},
        v_flex,
    },
    *,
};
use superfile_services::preferences::Appearance;

#[derive(Clone, Copy)]
enum PreferenceChoice {
    Theme(Appearance),
    Language(&'static str),
}

fn preference_picker(
    id: &'static str,
    label: &'static str,
    value: &'static str,
    options: Vec<(PreferenceChoice, &'static str)>,
    cx: &App,
) -> AnyElement {
    Button::new(id)
        .small()
        .min_w(px(110.))
        .h(px(30.))
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(5.))
        .disabled(crate::app::updates::busy(cx))
        .accessibility_label(format!(
            "{}: {}",
            superfile_services::locale::tr(label),
            superfile_services::locale::tr(value)
        ))
        .child(
            h_flex()
                .gap_2()
                .child(superfile_services::locale::tr(value))
                .child(Icon::new(gpui_kit::assets::IconName::ChevronDown).size(px(14.))),
        )
        .dropdown_menu(move |mut menu, _, _| {
            for (choice, label) in &options {
                let choice = *choice;
                menu = menu.item(
                    PopupMenuItem::new(superfile_services::locale::tr(label)).on_click(
                        move |_, _, cx| match choice {
                            PreferenceChoice::Theme(mode) => crate::app::set_appearance(mode, cx),
                            PreferenceChoice::Language(language) => {
                                crate::app::change_language(language, cx)
                            }
                        },
                    ),
                );
            }
            menu
        })
        .into_any_element()
}

impl Workspace {
    pub(super) fn settings_page(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let color = crate::app::theme::color(cx);
        let value = rgb(color.rgb()).into();
        self.accent_color.update(cx, |state, cx| {
            if state.value() != Some(value) {
                state.set_value(value, window, cx);
            }
        });
        let accent_color = h_flex()
            .gap_2()
            .child(
                ColorPicker::new(&self.accent_color)
                    .small()
                    .label(color.hex())
                    .accessibility_label(superfile_services::locale::tr("Accent color"))
                    .featured_colors(
                        [0x8b5cf6, 0x3b82f6, 0x06b6d4, 0x22c55e, 0xf97316, 0xec4899]
                            .into_iter()
                            .map(|color| rgb(color).into())
                            .collect(),
                    ),
            )
            .child(
                Button::new("reset-accent-color")
                    .small()
                    .icon(gpui_kit::assets::IconName::RotateCcw)
                    .tooltip(superfile_services::locale::tr("Use theme accent"))
                    .accessibility_label(superfile_services::locale::tr("Use theme accent"))
                    .disabled(!crate::app::theme::has_custom_accent(cx))
                    .on_click(|_, _, cx| crate::app::theme::reset_accent(cx)),
            )
            .into_any_element();
        let appearance = cx.global::<crate::app::AppearancePreference>().0;
        let theme = preference_picker(
            "settings-theme",
            "Theme",
            match appearance {
                Appearance::System => "Follow System",
                Appearance::Light => "Light",
                Appearance::Dark => "Dark",
            },
            vec![
                (PreferenceChoice::Theme(Appearance::System), "Follow System"),
                (PreferenceChoice::Theme(Appearance::Light), "Light"),
                (PreferenceChoice::Theme(Appearance::Dark), "Dark"),
            ],
            cx,
        );
        let languages = superfile_services::locale::available();
        let current = cx.global::<crate::app::LanguagePreference>().0.as_str();
        let label = languages
            .iter()
            .find(|(code, _)| *code == current)
            .map_or("Follow System", |(_, name)| *name);
        let mut options = vec![(PreferenceChoice::Language("system"), "Follow System")];
        options.extend(
            languages
                .into_iter()
                .map(|(code, name)| (PreferenceChoice::Language(code), name)),
        );
        let language = preference_picker("settings-language", "Language", label, options, cx);
        let color_scheme = h_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::app::theme::scheme(cx).name.clone()),
            )
            .child(
                Button::new("browse-community-themes")
                    .small()
                    .label(superfile_services::locale::tr("Browse themes…"))
                    .on_click(cx.listener(|view, _, window, cx| view.show_themes(window, cx))),
            )
            .into_any_element();
        let action_button = |id: &'static str, label: &'static str| {
            Button::new(id)
                .small()
                .min_w(px(110.))
                .h(px(30.))
                .border_1()
                .border_color(cx.theme().border)
                .rounded(px(5.))
                .disabled(crate::app::updates::busy(cx))
                .label(superfile_services::locale::tr(label))
        };
        let associations = action_button("settings-file-types", "View…")
            .accessibility_label(superfile_services::locale::tr("Default File Types…"))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(DefaultApplications), cx))
            .into_any_element();
        let code_theme = action_button("settings-code-theme", "Manage…")
            .accessibility_label(superfile_services::locale::tr("Code Theme"))
            .on_click(|_, window, cx| crate::views::code_themes::open(window, cx))
            .into_any_element();
        let connections = action_button("settings-ssh", "Manage…")
            .accessibility_label(superfile_services::locale::tr("SSH Connections"))
            .on_click(
                cx.listener(|view, _, window, cx| view.show_remote(None, true, false, window, cx)),
            )
            .into_any_element();
        let shortcuts = action_button("settings-shortcuts", "Edit…")
            .accessibility_label(superfile_services::locale::tr("Keyboard Shortcuts…"))
            .on_click(cx.listener(|view, _, window, cx| view.show_shortcuts(window, cx)))
            .into_any_element();
        let row = |label: &'static str, detail: &'static str, control: AnyElement| {
            let mut text = v_flex()
                .flex_1()
                .min_w(px(0.))
                .gap_1()
                .child(div().text_sm().child(superfile_services::locale::tr(label)));
            if !detail.is_empty() {
                text = text.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(superfile_services::locale::tr(detail)),
                );
            }
            h_flex()
                .w_full()
                .min_h(px(58.))
                .py_3()
                .gap_4()
                .child(text)
                .child(div().flex_shrink_0().child(control))
        };
        let group = || v_flex().w_full();
        let divider = || div().w_full().h(px(1.)).bg(cx.theme().border.opacity(0.6));
        let heading = |label: &'static str| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(superfile_services::locale::tr(label))
        };
        let information = |label: &'static str, value: String| {
            h_flex()
                .w_full()
                .gap_4()
                .text_sm()
                .child(
                    div()
                        .flex_1()
                        .text_color(cx.theme().muted_foreground)
                        .child(superfile_services::locale::tr(label)),
                )
                .child(div().child(value))
        };
        let local_build = env!("SUPERFILE_BUILD_CHANNEL") == "local";
        let version = if local_build {
            format!(
                "{} · {}",
                env!("CARGO_PKG_VERSION"),
                superfile_services::locale::tr("Local build")
            )
        } else {
            env!("CARGO_PKG_VERSION").to_owned()
        };
        let mut about = group().py_2().child(information("Version", version));
        if local_build {
            about = about.child(information(
                "Built",
                env!("SUPERFILE_BUILD_TIMESTAMP").to_owned(),
            ));
            if !env!("SUPERFILE_BUILD_HASH").is_empty() {
                let mut revision = env!("SUPERFILE_BUILD_HASH").to_owned();
                if env!("SUPERFILE_BUILD_DIRTY") == "true" {
                    revision.push_str(" · ");
                    revision.push_str(&superfile_services::locale::tr("Modified"));
                }
                about = about.child(information("Commit", revision));
            }
        }
        div()
            .id("workspace-settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex().w_full().items_center().px_6().py_6().child(
                    v_flex()
                        .w_full()
                        .max_w(px(560.))
                        .gap_3()
                        .child(heading("General"))
                        .child(
                            group()
                                .child(row("Theme", "", theme))
                                .child(divider())
                                .child(row("Color scheme", "", color_scheme))
                                .child(divider())
                                .child(row("Code Theme", "", code_theme))
                                .child(divider())
                                .child(row("Accent color", "", accent_color))
                                .child(divider())
                                .child(row("Language", "", language)),
                        )
                        .child(heading("Files").mt_3())
                        .child(
                            group()
                                .child(row(
                                    "Default File Types…",
                                    "View the default application for each registered file type.",
                                    associations,
                                ))
                                .child(divider())
                                .child(row(
                                    "SSH Connections",
                                    "Browse remote files in a read-only workspace.",
                                    connections,
                                )),
                        )
                        .child(heading("Application").mt_3())
                        .child(
                            group()
                                .child(row("Keyboard Shortcuts…", "", shortcuts))
                                .child(divider())
                                .child(row(
                                    "Supported File Formats",
                                    "",
                                    action_button("settings-supported-formats", "View…")
                                        .accessibility_label(superfile_services::locale::tr(
                                            "Supported File Formats",
                                        ))
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(Box::new(SupportedFormats), cx)
                                        })
                                        .into_any_element(),
                                ))
                                .child(divider())
                                .child(row(
                                    "Command Line",
                                    "",
                                    action_button("settings-install-cli", "Install…")
                                        .accessibility_label(superfile_services::locale::tr(
                                            "Install 'superfile' Command…",
                                        ))
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(Box::new(InstallCli), cx)
                                        })
                                        .into_any_element(),
                                ))
                                .child(divider())
                                .child(row(
                                    "Software Update",
                                    "",
                                    action_button("settings-updates", "Check for Updates…")
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(Box::new(CheckForUpdates), cx)
                                        })
                                        .into_any_element(),
                                )),
                        )
                        .child(self.integrations.clone())
                        .child(heading("About").mt_3())
                        .child(about),
                ),
            )
            .into_any_element()
    }
}
