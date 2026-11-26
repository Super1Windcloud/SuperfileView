mod bindings;

use self::bindings::{Bindings, Overrides, parse_custom_key};
use super::actions::{CloseWindow, CloseWorkspace};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, TitleBar,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use superfile_services::locale::tr;

pub(crate) use bindings::format_shortcut_for_display;
pub(super) use bindings::{install, take_write};

pub(crate) struct Shortcuts {
    focus: FocusHandle,
    recording: Option<&'static str>,
    error: Option<String>,
    standalone: bool,
    _intercept: Subscription,
}

impl Shortcuts {
    #[cfg(test)]
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(true, window, cx)
    }

    pub(crate) fn new_tab(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(false, window, cx)
    }

    fn build(standalone: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let handle = window.window_handle();
        let view = cx.entity().downgrade();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if handle == window.window_handle() {
                let _ = view.update(cx, |view, cx| view.capture(&event.keystroke, cx));
            }
        });
        Self {
            focus,
            recording: None,
            error: None,
            standalone,
            _intercept: intercept,
        }
    }

    pub(crate) fn deactivate(&mut self, cx: &mut Context<Self>) {
        if self.recording.take().is_some() || self.error.take().is_some() {
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn init_test(cx: &mut App) {
        Bindings::install(Overrides::new(), |_| Ok(()), cx);
    }

    fn capture(&mut self, key: &Keystroke, cx: &mut Context<Self>) {
        let Some(name) = self.recording else { return };
        cx.stop_propagation();
        if key.key == "escape" && key.modifiers == Modifiers::none() {
            self.recording = None;
            self.error = None;
            cx.notify();
            return;
        }
        let Some(key) = parse_custom_key(&key.unparse()) else {
            self.error = Some(tr("Use a modifier or a function key."));
            cx.notify();
            return;
        };
        let mut overrides = cx.global::<Bindings>().overrides.clone();
        overrides.insert(name.to_owned(), key.unparse());
        self.save(overrides, cx);
    }

    fn save(&mut self, overrides: Overrides, cx: &mut Context<Self>) {
        match Bindings::save_and_apply(overrides, cx.entity().downgrade(), cx) {
            Ok(()) => {
                self.recording = None;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
}

impl Render for Shortcuts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.try_global::<Bindings>();
        let saving = state.is_some_and(|state| state.saving);
        let has_changes = state.is_some_and(|state| !state.overrides.is_empty());
        let entries = state
            .map(|state| {
                state
                    .commands
                    .iter()
                    .map(|command| {
                        (
                            command.name,
                            command.label,
                            state.display(command.name),
                            state.overrides.contains_key(command.name),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let recording = self.recording;
        let status = if let Some(error) = &self.error {
            Some((error.clone(), cx.theme().danger))
        } else if saving {
            Some((tr("Saving…"), cx.theme().muted_foreground))
        } else if recording.is_some() {
            Some((
                tr("Press a key combination. Press Esc to cancel."),
                cx.theme().muted_foreground,
            ))
        } else {
            None
        };
        v_flex()
            .id("keyboard-shortcuts-page")
            .size_full()
            .track_focus(&self.focus)
            .when(self.standalone, |view| {
                view.on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
                    .on_action(cx.listener(|_, _: &CloseWorkspace, window, _| window.remove_window()))
                    .child(TitleBar::new().child(tr("Keyboard Shortcuts — Superfile")))
            })
            .child(
                h_flex()
                    .w_full()
                    .min_h(px(40.))
                    .gap_2()
                    .px_4()
                    .when(!self.standalone, |bar| bar.pt_2())
                    .child(div().flex_1().min_w_0().text_sm().when_some(status, |item, (label, color)| {
                        item.text_color(color).child(label)
                    }))
                    .when(recording.is_some(), |bar| {
                        bar.child(
                            Button::new("cancel-shortcut-recording")
                                .ghost()
                                .small()
                                .label(tr("Cancel"))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.recording = None;
                                    view.error = None;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("restore-all-shortcuts")
                            .ghost()
                            .small()
                            .disabled(saving || !has_changes)
                            .label(tr("Restore Defaults"))
                            .on_click(cx.listener(|view, _, _, cx| view.save(Overrides::new(), cx))),
                    ),
            )
            .child(
                v_flex()
                    .id("shortcut-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_4()
                    .pb_4()
                    .child(
                        v_flex()
                            .flex_shrink_0()
                            .w_full()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(8.))
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .flex_shrink_0()
                                    .bg(cx.theme().muted)
                                    .px_3()
                                    .py_2()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(div().flex_1().min_w_0().child(tr("Operation")))
                                    .child(div().w(px(250.)).flex_shrink_0().child(tr("Shortcut"))),
                            )
                            .children(entries.into_iter().map(|(name, operation, key, customized)| {
                                let active = recording == Some(name);
                                let change_key = if active {
                                    tr("Recording…")
                                } else {
                                    key
                                };
                                h_flex()
                                    .flex_shrink_0()
                                    .w_full()
                                    .min_h(px(36.))
                                    .gap_2()
                                    .px_3()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .text_sm()
                                    .child(div().flex_1().min_w_0().whitespace_normal().child(tr(operation)))
                                    .child(
                                        h_flex()
                                            .w(px(250.))
                                            .flex_shrink_0()
                                            .gap_2()
                                            .child(
                                                Button::new(SharedString::from(format!("record-shortcut-{name}")))
                                                    .small()
                                                    .outline()
                                                    .disabled(saving)
                                                    .min_w_0()
                                                    .flex_1()
                                                    .label(change_key)
                                                    .accessibility_label(tr("Change shortcut for {operation}").replace("{operation}", &tr(operation)))
                                                    .on_click(cx.listener(move |view, _, window, cx| {
                                                        if !cx.global::<Bindings>().saving {
                                                            view.recording = Some(name);
                                                            view.error = None;
                                                            window.focus(&view.focus, cx);
                                                            cx.notify();
                                                        }
                                                    })),
                                            )
                                            .child(div().w(px(70.)).flex_shrink_0().when(customized, |row| {
                                                row.child(
                                                    Button::new(SharedString::from(format!("restore-shortcut-{name}")))
                                                        .ghost()
                                                        .small()
                                                        .disabled(saving)
                                                        .label(tr("Reset"))
                                                        .accessibility_label(tr("Reset shortcut for {operation}").replace("{operation}", &tr(operation)))
                                                        .on_click(cx.listener(move |view, _, _, cx| {
                                                            let mut overrides = cx.global::<Bindings>().overrides.clone();
                                                            overrides.remove(name);
                                                            view.save(overrides, cx);
                                                        })),
                                                )
                                            })),
                                    )
                            })),
                    ),
            )
    }
}

pub fn open(cx: &mut App) {
    super::open_workspace_tab(cx, |view, window, cx| view.show_shortcuts(window, cx));
}

#[cfg(test)]
mod tests {
    use super::{Bindings, Overrides, Shortcuts, format_shortcut_for_display};
    use crate::app::actions::{NewFile, OpenFile};
    use gpui_kit::{
        Action, AppContext, AsKeystroke, Empty, KeyBinding, SharedString, TestAppContext,
        WindowOptions, component::input::Undo, test::TestWindowExt,
    };
    use std::{cell::Cell, rc::Rc};

    #[gpui_kit::test]
    fn records_conflicts_rebinds_other_windows_and_restores_defaults(cx: &mut TestAppContext) {
        let created = Rc::new(Cell::new(0));
        let opened = Rc::new(Cell::new(0));
        let (handle, view, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys([
                KeyBinding::new("secondary-n", NewFile, None),
                KeyBinding::new("secondary-o", OpenFile, None),
            ]);
            let counts = created.clone();
            cx.on_action(move |_: &NewFile, _| counts.set(counts.get() + 1));
            let counts = opened.clone();
            cx.on_action(move |_: &OpenFile, _| counts.set(counts.get() + 1));
            Bindings::install(Overrides::new(), |_| Ok(()), cx);
            let (workspace, _) =
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Empty))
                    .unwrap();
            let (handle, view) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| Shortcuts::new(window, cx))
                })
                .unwrap();
            (handle, view, workspace)
        });
        let record = SharedString::from(format!("record-shortcut-{}", NewFile.name()));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(record.clone(), cx);
            window.press("secondary-o", cx);
            assert!(view.read(cx).error.is_some());
            assert_eq!(opened.get(), 0);
            window.press("escape", cx);
            assert!(view.read(cx).recording.is_none());
            assert!(cx.global::<Bindings>().overrides.is_empty());
            window.click(record.clone(), cx);
            window.press("f12", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(created.get(), 0, "recording dispatched the shortcut");
        cx.update_window(workspace, |_, window, cx| {
            window.render_frame(cx);
            window.press("secondary-n", cx);
            assert_eq!(created.get(), 0);
            window.press("f12", cx);
            assert_eq!(created.get(), 1);
            window.press("secondary-o", cx);
            assert_eq!(opened.get(), 1);
            assert_eq!(
                format_shortcut_for_display(&NewFile, "secondary-n", cx),
                "F12"
            );
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("restore-all-shortcuts", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(workspace, |_, window, cx| {
            window.render_frame(cx);
            assert!(cx.global::<Bindings>().overrides.is_empty());
            window.press("f12", cx);
            assert_eq!(created.get(), 1);
            window.press("secondary-n", cx);
            assert_eq!(created.get(), 2);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn startup_applies_saved_bindings_with_their_original_context(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("secondary-z", Undo, Some("Input")),
                KeyBinding::new("secondary-n", NewFile, None),
            ]);
            Bindings::install(
                Overrides::from([
                    (Undo.name().to_owned(), "f10".to_owned()),
                    (NewFile.name().to_owned(), "n".to_owned()),
                ]),
                |_| Ok(()),
                cx,
            );
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let bindings = keymap.bindings_for_action(&Undo).collect::<Vec<_>>();
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].keystrokes()[0].as_keystroke().key, "f10");
            assert!(bindings[0].predicate().is_some());
            assert!(
                !cx.global::<Bindings>()
                    .overrides
                    .contains_key(NewFile.name())
            );
            let default = keymap.bindings_for_action(&NewFile).next().unwrap();
            assert_eq!(default.keystrokes()[0].as_keystroke().key, "n");
        });
    }
}
