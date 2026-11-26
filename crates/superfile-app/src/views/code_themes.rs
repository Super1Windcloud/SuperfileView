use crate::app::code_theme::{self, Palette};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, WindowExt,
        button::{Button, ButtonVariants},
        color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
        h_flex,
        input::{Editor, EditorState, Input, InputState},
        menu::{DropdownMenu, PopupMenuItem},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::{collections::BTreeMap, sync::Arc};
use superfile_services::{
    code_themes::{self, Appearance, Color, Library, Theme},
    locale::tr,
};

const SAMPLE: &str = r#"// Read a named setting.
fn read_setting(name: &str) -> Option<String> {
    let limit = 64;
    if name.len() > limit {
        return None;
    }
    Some(format!("setting: {}", name))
}
"#;

const FIELDS: &[(&str, &str)] = &[
    ("editor.background", "Editor background"),
    ("editor.foreground", "Editor text"),
    ("editor.selection", "Text selection"),
    ("editor.caret", "Caret"),
    ("editor.active_line.background", "Current line"),
    ("editor.gutter.background", "Gutter background"),
    ("editor.line_number", "Line numbers"),
    ("syntax.comment", "Comments"),
    ("syntax.keyword", "Keywords"),
    ("syntax.string", "Strings"),
    ("syntax.number", "Numbers"),
    ("syntax.function", "Functions"),
    ("syntax.type", "Types"),
    ("syntax.variable", "Variables"),
    ("syntax.constant", "Constants"),
    ("syntax.property", "Properties"),
    ("syntax.operator", "Operators"),
    ("syntax.punctuation", "Punctuation"),
];

pub(crate) fn open(window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let panel = cx.new(|cx| CodeThemePanel::new(window, cx));
    super::dialog_drag::open(window, cx, move |dialog, window, cx, drag| {
        let height = (window.viewport_size().height - px(160.)).clamp(px(230.), px(530.));
        dialog
            .title(super::dialog_drag::title(drag, tr("Code Theme"), cx))
            .width(px(680.))
            .child(div().h(height).child(panel.clone()))
    });
}

fn label(library: &Library, id: Option<&str>) -> String {
    id.and_then(|id| library.theme(id)).map_or_else(
        || tr("Follow application theme"),
        |theme| theme.name.clone(),
    )
}

struct CodeThemePanel {
    selected: Option<String>,
    field: usize,
    overrides: BTreeMap<String, Color>,
    name: Entity<InputState>,
    color: Entity<ColorPickerState>,
    preview: Entity<EditorState>,
    status: Option<&'static str>,
    error: Option<String>,
    task: Option<Task<()>>,
    _color_observer: Subscription,
    _color_subscription: Subscription,
}

impl CodeThemePanel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = if cx.theme().is_dark() {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        let selected = code_theme::library(cx)
            .selection(appearance)
            .map(str::to_owned);
        let preview = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .language("rust")
                .searchable(false)
                .line_number(true)
                .soft_wrap(false);
            editor.set_value(SAMPLE, window, cx);
            editor.set_readonly(true, cx);
            editor
        });
        let color = cx.new(|cx| ColorPickerState::new(window, cx));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Custom theme name…")));
        let subscription = cx.subscribe(&color, |panel: &mut Self, _, change, cx| {
            let ColorPickerEvent::Change(Some(color)) = change else {
                return;
            };
            if let Some(base) = panel.base(cx) {
                let field = FIELDS[panel.field].0;
                let base_color = code_theme::from_hsla(Palette::new(base).color(field));
                let color = code_theme::from_hsla(*color);
                if base_color == color {
                    panel.overrides.remove(field);
                } else {
                    panel.overrides.insert(field.to_owned(), color);
                }
                panel.status = None;
                cx.notify();
            }
        });
        let observer = cx.observe(&color, |_, _, cx| cx.notify());
        Self {
            selected,
            field: 0,
            overrides: BTreeMap::new(),
            name,
            color,
            preview,
            status: None,
            error: None,
            task: None,
            _color_observer: observer,
            _color_subscription: subscription,
        }
    }

    fn base<'a>(&self, cx: &'a App) -> Option<&'a Theme> {
        code_theme::library(cx).theme(self.selected.as_deref()?)
    }

    fn draft(&self, cx: &App) -> Option<Theme> {
        let mut theme = self.base(cx)?.clone();
        theme.colors.extend(self.overrides.clone());
        Some(theme)
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = id;
        self.overrides.clear();
        self.status = None;
        self.error = None;
        self.name
            .update(cx, |name, cx| name.set_value("", window, cx));
        cx.notify();
    }

    fn apply(&mut self, appearance: Option<Appearance>, cx: &mut Context<Self>) {
        if !self.overrides.is_empty() || crate::app::updates::busy(cx) {
            return;
        }
        let mut library = code_theme::library(cx).clone();
        let selected = self
            .selected
            .clone()
            .filter(|id| library.theme(id).is_some());
        if appearance != Some(Appearance::Dark) {
            library.light = selected.clone();
        }
        if appearance != Some(Appearance::Light) {
            library.dark = selected;
        }
        code_theme::update(cx, library);
        self.status = Some("Code theme applied.");
        self.error = None;
        cx.notify();
    }

    fn add(
        &mut self,
        themes: Vec<Theme>,
        status: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut library = code_theme::library(cx).clone();
        match library.add(themes) {
            Ok(ids) => {
                code_theme::update(cx, library);
                self.select(ids.into_iter().next(), window, cx);
                self.status = Some(status);
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut theme) = self.draft(cx) else {
            return;
        };
        let name = self.name.read(cx).value();
        theme.name = if name.trim().is_empty() {
            tr("{name} (Custom)").replace("{name}", &theme.name)
        } else {
            name.trim().to_owned()
        };
        self.add(vec![theme], "Custom code theme saved.", window, cx);
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Import Code Theme…").into()),
        });
        self.error = None;
        self.status = None;
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = match selection.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        cx.background_executor()
                            .spawn(async move { code_themes::import(&path) })
                            .await
                            .map(Some)
                    } else {
                        Ok(None)
                    }
                }
                Ok(Ok(None)) => Ok(None),
                other => Err(anyhow::anyhow!("Cannot choose code theme: {other:?}")),
            };
            let _ = view.update_in(cx, |panel, window, cx| {
                panel.task = None;
                match result {
                    Ok(Some(themes)) => panel.add(themes, "Code theme imported.", window, cx),
                    Ok(None) => {}
                    Err(error) => panel.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(theme) = self.draft(cx) else { return };
        let directory = match superfile_services::platform::home() {
            Ok(directory) => directory,
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                cx.notify();
                return;
            }
        };
        let selection = cx.prompt_for_new_path(&directory, Some("superfile-code-theme.json"));
        self.error = None;
        self.status = None;
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = match selection.await {
                Ok(Ok(Some(path))) => cx
                    .background_executor()
                    .spawn(async move { code_themes::export(&path, theme) })
                    .await
                    .map(|()| true),
                Ok(Ok(None)) => Ok(false),
                other => Err(anyhow::anyhow!(
                    "Cannot choose export destination: {other:?}"
                )),
            };
            let _ = view.update(cx, |panel, cx| {
                panel.task = None;
                match result {
                    Ok(true) => panel.status = Some("Code theme exported."),
                    Ok(false) => {}
                    Err(error) => panel.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone().filter(|id| id.starts_with("local:")) else {
            return;
        };
        let buttons = [
            PromptButton::ok(tr("Remove")),
            PromptButton::cancel(tr("Cancel")),
        ];
        let confirm = window.prompt(
            PromptLevel::Warning,
            &tr("Remove this code theme?"),
            None,
            &buttons,
            cx,
        );
        cx.spawn_in(window, async move |view, cx| {
            if !matches!(confirm.await, Ok(0)) {
                return;
            }
            let _ = view.update_in(cx, |panel, window, cx| {
                let mut library = code_theme::library(cx).clone();
                if library.remove(&id) {
                    code_theme::update(cx, library);
                    if panel.selected.as_deref() == Some(&id) {
                        panel.select(None, window, cx);
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for CodeThemePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::sync_input_placeholder(&self.name, tr("Custom theme name…"), window, cx);
        let library = code_theme::library(cx);
        let entries: Vec<_> = library
            .entries()
            .map(|(id, theme)| (id.to_owned(), theme.name.clone()))
            .collect();
        let selection = self.selected.clone();
        let current = label(library, selection.as_deref());
        let light = label(library, library.light.as_deref());
        let dark = label(library, library.dark.as_deref());
        let mut palette = self.draft(cx).map(|theme| Arc::new(Palette::new(&theme)));
        let editable = palette.is_some();
        let busy = self.task.is_some() || crate::app::updates::busy(cx);
        let field = self.field;
        if let Some(base_palette) = &palette {
            let value = base_palette.color(FIELDS[field].0);
            self.color.update(cx, |color, cx| {
                if color.value() != Some(value) {
                    color.set_value(value, window, cx);
                }
            });
            let picker = self.color.read(cx);
            if let Some(color) = picker.displayed_color()
                && picker.value() != Some(color)
                && let Some(mut draft) = self.draft(cx)
            {
                draft
                    .colors
                    .insert(FIELDS[field].0.into(), code_theme::from_hsla(color));
                palette = Some(Arc::new(Palette::new(&draft)));
            }
        }
        let panel = cx.entity().downgrade();
        let picker = Button::new("code-theme-picker")
            .small()
            .label(current)
            .icon(gpui_kit::assets::IconName::ChevronDown)
            .dropdown_menu(move |mut menu, _, _| {
                let owner = panel.clone();
                menu = menu.item(
                    PopupMenuItem::new(tr("Follow application theme"))
                        .checked(selection.is_none())
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |view, cx| view.select(None, window, cx));
                        }),
                );
                for (id, title) in &entries {
                    let owner = panel.clone();
                    let selected = selection.as_deref() == Some(id.as_str());
                    let id = id.clone();
                    menu = menu.item(
                        PopupMenuItem::new(title.clone())
                            .checked(selected)
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.select(Some(id.clone()), window, cx)
                                });
                            }),
                    );
                }
                menu
            });
        let panel = cx.entity().downgrade();
        let apply = Button::new("apply-code-theme")
            .small()
            .primary()
            .label(tr("Apply to…"))
            .disabled(busy || !self.overrides.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for (mode, text) in [
                    (Some(Appearance::Light), "Light appearance"),
                    (Some(Appearance::Dark), "Dark appearance"),
                    (None, "Both appearances"),
                ] {
                    let owner = panel.clone();
                    menu = menu.item(PopupMenuItem::new(tr(text)).on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |view, cx| view.apply(mode, cx));
                    }));
                }
                menu
            });
        let panel = cx.entity().downgrade();
        let edit_field = Button::new("code-theme-color-field")
            .small()
            .label(tr(FIELDS[field].1))
            .icon(gpui_kit::assets::IconName::ChevronDown)
            .dropdown_menu(move |mut menu, _, _| {
                for (index, (_, name)) in FIELDS.iter().enumerate() {
                    let owner = panel.clone();
                    menu = menu.item(
                        PopupMenuItem::new(tr(name))
                            .checked(index == field)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.field = index;
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            });
        let preview = code_theme::editor(
            Editor::new(&self.preview)
                .readonly(true)
                .appearance(false)
                .bordered(false)
                .size_full(),
            &self.preview,
            palette,
            cx,
        );
        let local = self
            .selected
            .as_deref()
            .is_some_and(|id| id.starts_with("local:"));
        div()
            .id("code-themes-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(picker)
                            .child(apply)
                            .child(
                                Button::new("import-code-theme")
                                    .small()
                                    .label(tr("Import Code Theme…"))
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|view, _, window, cx| view.import(window, cx)),
                                    ),
                            )
                            .when(local, |row| {
                                row.child(
                                    Button::new("remove-code-theme")
                                        .small()
                                        .ghost()
                                        .label(tr("Remove"))
                                        .disabled(busy)
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.remove(window, cx)
                                        })),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .flex_wrap()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("Light code theme: {name}").replace("{name}", &light))
                            .child(tr("Dark code theme: {name}").replace("{name}", &dark)),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(210.))
                            .overflow_hidden()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(5.))
                            .bg(cx.theme().background)
                            .child(preview),
                    )
                    .when(editable, |column| {
                        column
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(edit_field)
                                    .child(
                                        ColorPicker::new(&self.color)
                                            .small()
                                            .accessibility_label(tr("Code theme color")),
                                    )
                                    .child(
                                        Button::new("reset-code-theme-color")
                                            .small()
                                            .label(tr("Reset color"))
                                            .disabled(!self.overrides.contains_key(FIELDS[field].0))
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.overrides.remove(FIELDS[field].0);
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .w_full()
                                    .child(Input::new(&self.name).small().flex_1().min_w_0())
                                    .child(
                                        Button::new("save-custom-code-theme")
                                            .small()
                                            .label(tr("Save a copy"))
                                            .disabled(busy)
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.save_as(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("export-code-theme")
                                            .small()
                                            .label(tr("Export…"))
                                            .disabled(busy)
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.export(window, cx)
                                            })),
                                    ),
                            )
                    })
                    .when(!self.overrides.is_empty(), |column| {
                        column.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("Save a copy to apply custom colors.")),
                        )
                    })
                    .when(self.task.is_some(), |column| {
                        column.child(div().text_sm().child(tr("Working…")))
                    })
                    .when_some(self.status, |column, status| {
                        column.child(div().text_sm().child(tr(status)))
                    })
                    .when_some(self.error.clone(), |column, error| {
                        column.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Appearance, CodeThemePanel, Color, SAMPLE, code_theme};
    use gpui_kit::{AppContext, TestAppContext, WindowOptions, rgb, test::TestWindowExt};

    #[gpui_kit::test]
    fn theme_picker_changes_preview_and_color_reset_restores_preset(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, panel) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| CodeThemePanel::new(window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(panel.read(cx).selected.as_deref(), Some("xcode-light"));
            window.render_frame(cx);
            window.click("code-theme-picker", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").click(3usize, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(panel.read(cx).selected.as_deref(), Some("catppuccin-latte"));
            let color = panel.read(cx).color.clone();
            color.update(cx, |color, cx| {
                color.select_color(rgb(0x102030).into(), window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let view = panel.read(cx);
            assert_eq!(
                view.draft(cx).unwrap().colors["editor.background"],
                Color([16, 32, 48, 255])
            );
            assert_eq!(view.preview.read(cx).text().to_string(), SAMPLE);
            assert_eq!(
                code_theme::library(cx).selection(Appearance::Light),
                Some("xcode-light")
            );
            window.render_frame(cx);
            window.click("reset-code-theme-color", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let view = panel.read(cx);
            assert!(view.overrides.is_empty());
            assert_eq!(
                view.draft(cx).unwrap().colors["editor.background"],
                Color([239, 241, 245, 255])
            );
        });
    }
}
