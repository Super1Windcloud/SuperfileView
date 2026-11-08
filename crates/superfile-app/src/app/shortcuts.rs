use super::actions::{CloseWindow, CloseWorkspace};
use gpui_kit::{
    component::{ActiveTheme, TitleBar, h_flex, v_flex},
    *,
};

struct Shortcuts {
    focus: FocusHandle,
}
impl Render for Shortcuts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = platform_shortcuts();
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &CloseWorkspace, window, _| window.remove_window()))
            .child(TitleBar::new().child(superfile_services::locale::tr(
                "Keyboard Shortcuts — Superfile",
            )))
            .child(
                v_flex()
                    .id("shortcut-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
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
                                    .w_full()
                                    .bg(cx.theme().muted)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .px_3()
                                            .py_2()
                                            .child(superfile_services::locale::tr("Operation")),
                                    )
                                    .child(
                                        div()
                                            .w(px(200.))
                                            .flex_shrink_0()
                                            .px_3()
                                            .py_2()
                                            .border_l_1()
                                            .border_color(cx.theme().border)
                                            .child(superfile_services::locale::tr("Shortcut")),
                                    ),
                            )
                            .children(entries.into_iter().map(|(description, mac)| {
                                h_flex()
                                    .flex_shrink_0()
                                    .w_full()
                                    .items_stretch()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .text_sm()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .px_3()
                                            .py_2()
                                            .whitespace_normal()
                                            .child(superfile_services::locale::tr(description)),
                                    )
                                    .child(
                                        div()
                                            .w(px(200.))
                                            .flex_shrink_0()
                                            .px_3()
                                            .py_2()
                                            .border_l_1()
                                            .border_color(cx.theme().border)
                                            .child(if cfg!(target_os = "macos") {
                                                mac.to_owned()
                                            } else {
                                                mac.replace("⌘", "Ctrl+")
                                                    .replace("⇧", "Shift+")
                                                    .replace("⌥", "Alt+")
                                            }),
                                    )
                            })),
                    ),
            )
    }
}
pub fn open(cx: &mut App) {
    superfile_services::telemetry::track("shortcuts_opened", []);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(680.), px(760.)),
            cx,
        ))),
        ..TitleBar::window_options()
    };
    if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Shortcuts { focus }
        })
    }) {
        log::error!("Cannot open keyboard shortcuts: {error:#}");
    }
}

fn platform_shortcuts() -> Vec<(&'static str, &'static str)> {
    if cfg!(target_os = "macos") {
        vec![
            ("Open Settings", "⌘,"),
            ("New window", "⌘⇧N"),
            ("New file", "⌘N"),
            ("Open file", "⌘O"),
            ("Open folder", "⌘⇧O"),
            ("Add workspace folder", "⌘⌥O"),
            ("Save file", "⌘S"),
            ("Refresh", "⌘R"),
            ("Close current tab; close window when empty", "⌘W"),
            ("Close workspace window", "⌘⇧W"),
            ("Quit Superfile", "⌘Q"),
            ("Search workspace files", "⌘P"),
            ("Toggle sidebar", "⌘B"),
            ("Toggle preview / source", "⌘⇧V"),
            ("Toggle word wrap", "⌥Z"),
            ("Next tab", "Ctrl+Tab / ⌘⇧]"),
            ("Previous tab", "Ctrl+Shift+Tab / ⌘⇧["),
            ("Navigate back / forward", "⌥← / ⌥→"),
            ("Toggle theme", "⌘⌥T"),
            ("Keyboard shortcuts", "F1"),
            ("Copy / cut / paste / select all (editor)", "⌘C / X / V / A"),
            ("Copy selected text / image / table cell", "⌘C / Ctrl+C"),
            ("Undo (editor)", "⌘Z"),
            ("Redo (editor)", "⌘⇧Z"),
            ("Find in file", "⌘F"),
            ("Commit staged contents (commit input)", "⌘Enter"),
            ("Select / expand / open in file tree", "↑↓ / ←→ / Enter"),
        ]
    } else {
        vec![
            ("Open Settings", "Ctrl+,"),
            ("New window", "Ctrl+Shift+N"),
            ("New file", "Ctrl+N"),
            ("Open file", "Ctrl+O"),
            ("Open folder", "Ctrl+Shift+O"),
            ("Add workspace folder", "Ctrl+Alt+O"),
            ("Save file", "Ctrl+S"),
            ("Refresh", "Ctrl+R"),
            ("Close current tab; close window when empty", "Ctrl+W"),
            ("Close workspace window", "Ctrl+Shift+W"),
            ("Quit Superfile", "Ctrl+Q"),
            ("Search workspace files", "Ctrl+P"),
            ("Toggle sidebar", "Ctrl+B"),
            ("Toggle preview / source", "Ctrl+Shift+V"),
            ("Toggle word wrap", "Alt+Z"),
            ("Next tab", "Ctrl+Tab"),
            ("Previous tab", "Ctrl+Shift+Tab"),
            ("Navigate back / forward", "Alt+← / Alt+→"),
            ("Toggle theme", "Ctrl+Alt+T"),
            ("Keyboard shortcuts", "F1"),
            (
                "Copy / cut / paste / select all (editor)",
                "Ctrl+C / X / V / A",
            ),
            ("Copy selected text / image / table cell", "Ctrl+C"),
            ("Undo (editor)", "Ctrl+Z"),
            ("Redo (editor)", "Ctrl+Shift+Z / Ctrl+Y"),
            ("Find in file", "Ctrl+F / F3"),
            ("Commit staged contents (commit input)", "Ctrl+Enter"),
            ("Select / expand / open in file tree", "↑↓ / ←→ / Enter"),
        ]
    }
}
