use super::reader_actions::{ToolbarAction, ToolbarItem, document_items};
use super::{Page, Workspace};
use crate::app::actions::{TogglePreview, ToggleWordWrap};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Selectable, Sizable,
        button::{Button, ButtonCustomVariant, ButtonVariants},
        h_flex,
        menu::{DropdownMenu, PopupMenuItem},
    },
    *,
};

impl Workspace {
    pub(super) fn set_file_fullscreen(&mut self, enabled: bool, window: &Window) {
        if enabled == self.file_fullscreen {
            return;
        }
        let mut props = self.usage_context();
        props.push(("enabled", superfile_services::telemetry::flag(enabled)));
        superfile_services::telemetry::track("reader_fullscreen_changed", props);
        if enabled {
            self.file_fullscreen_seen_window = window.is_fullscreen();
            self.file_fullscreen_entered_window = !window.is_fullscreen();
            if self.file_fullscreen_entered_window {
                window.toggle_fullscreen();
            }
        } else {
            if self.file_fullscreen_entered_window && window.is_fullscreen() {
                window.toggle_fullscreen();
            }
            self.file_fullscreen_entered_window = false;
            self.file_fullscreen_seen_window = false;
        }
        self.file_fullscreen = enabled;
    }

    fn toolbar_button(&self, item: ToolbarItem, cx: &mut Context<Self>) -> Button {
        let action = item.action;
        let label = superfile_services::locale::tr(item.label);
        let icon = match item.icon {
            Some(icon) => Icon::new(icon),
            None if matches!(action, ToolbarAction::FileFullscreen) => {
                Icon::empty().path(if self.file_fullscreen {
                    "navigation/file-fullscreen-exit.svg"
                } else {
                    "navigation/file-fullscreen.svg"
                })
            }
            None => Icon::empty().path("navigation/wrap.svg"),
        };
        let button = Button::new(action.id())
            .custom(
                ButtonCustomVariant::new(cx)
                    .hover(cx.theme().foreground.opacity(0.05))
                    .active(cx.theme().foreground.opacity(0.09)),
            )
            .small()
            .w(px(28.))
            .h(px(28.))
            .flex_shrink_0()
            .rounded(px(7.))
            .icon(icon.size(px(16.)))
            .tooltip(label.clone())
            .accessibility_label(label.clone())
            .selected(item.selected)
            .disabled(!item.enabled)
            .on_click(cx.listener(move |view, _, window, cx| {
                view.run_toolbar_action(action, window, cx);
            }));
        if matches!(
            action,
            ToolbarAction::Extract | ToolbarAction::CancelExtract
        ) {
            button.w_auto().label(label)
        } else {
            button
        }
    }

    pub(super) fn navigation_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut back = ToolbarItem::new(ToolbarAction::Back);
        back.enabled = self.history_cursor.is_some_and(|i| i > 0);
        let mut forward = ToolbarItem::new(ToolbarAction::Forward);
        forward.enabled = self
            .history_cursor
            .is_some_and(|i| i + 1 < self.history.len());
        h_flex()
            .h(px(28.))
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .child(self.toolbar_button(ToolbarItem::new(ToolbarAction::Sidebar), cx))
            .child(self.toolbar_button(back, cx))
            .child(self.toolbar_button(forward, cx))
            .into_any_element()
    }

    pub(super) fn reader_actions(&self, cx: &App) -> Vec<ToolbarItem> {
        let mut document = if self.page == Page::Document {
            self.active_reader()
                .map(|r| document_items(r, self.word_wrap, cx))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if self.picking && self.active_reader().is_some_and(|r| r.is_archive()) {
            for item in &mut document {
                if matches!(item.action, ToolbarAction::Extract) {
                    *item = ToolbarItem::new(ToolbarAction::CancelExtract);
                }
            }
        }
        if self.page == Page::Document && self.active_reader().is_some() {
            let mut fullscreen = ToolbarItem::new(ToolbarAction::FileFullscreen);
            fullscreen.selected = self.file_fullscreen;
            if self.file_fullscreen {
                fullscreen.label = "Exit File Fullscreen";
            }
            document.push(fullscreen);
        }
        document
    }

    pub(super) fn workspace_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let document = self.reader_actions(cx);
        let mut toolbar = h_flex().gap_1().flex_shrink_0();
        let mut view_menu_added = false;
        for item in document {
            if matches!(
                item.action,
                ToolbarAction::Preview | ToolbarAction::Source | ToolbarAction::WordWrap
            ) {
                if !view_menu_added {
                    toolbar = toolbar.child(self.reader_view_menu(cx));
                    view_menu_added = true;
                }
            } else {
                toolbar = toolbar.child(self.toolbar_button(item, cx));
            }
        }
        toolbar.into_any_element()
    }

    fn reader_view_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let preview = self
            .reader_actions(cx)
            .into_iter()
            .find(|item| matches!(item.action, ToolbarAction::Preview) && item.selected);
        let label = format!(
            "{} · {}",
            superfile_services::locale::tr("View options"),
            superfile_services::locale::tr(preview.as_ref().map_or("Source", |item| item.label)),
        );
        let icon = preview.and_then(|item| item.icon).unwrap_or(IconName::Code);
        let owner = cx.entity().downgrade();
        // Dropdown menus retain their content until dismissed. Give each live
        // document/mode/layout its own scope so cached menus cannot cross tabs.
        let scope = SharedString::from(format!(
            "reader-view-{:?}-{:?}-{}",
            self.active_reader().map(|reader| reader.document.version()),
            self.active_reader().map(|reader| reader.mode()),
            self.word_wrap,
        ));
        div()
            .id(scope)
            .child(
                Button::new("reader-view-menu")
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .hover(cx.theme().foreground.opacity(0.05))
                            .active(cx.theme().foreground.opacity(0.09)),
                    )
                    .small()
                    .h(px(28.))
                    .px_2()
                    .flex_shrink_0()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .icon(Icon::new(icon).size(px(16.)))
                    .child(Icon::new(IconName::ChevronDown).size(px(12.)))
                    .tooltip(label.clone())
                    .accessibility_label(label)
                    .dropdown_menu(move |menu, _, cx| {
                        let Some(workspace) = owner.upgrade() else {
                            return menu;
                        };
                        let workspace = workspace.read(cx);
                        let items = workspace.reader_actions(cx);
                        let mut menu = menu
                            .action_context(workspace.focus.clone())
                            .check_side(gpui_kit::component::Side::Right)
                            .min_w(px(240.));
                        if items
                            .iter()
                            .any(|item| matches!(item.action, ToolbarAction::Preview))
                        {
                            menu = menu
                                .item(reader_menu_item(
                                    ToolbarItem::new(ToolbarAction::TogglePreview),
                                    owner.clone(),
                                ))
                                .separator();
                            for item in items.iter().filter(|item| {
                                matches!(
                                    item.action,
                                    ToolbarAction::Preview | ToolbarAction::Source
                                )
                            }) {
                                menu = menu.item(reader_menu_item(*item, owner.clone()));
                            }
                            if items
                                .iter()
                                .any(|item| matches!(item.action, ToolbarAction::WordWrap))
                            {
                                menu = menu.separator();
                            }
                        }
                        for item in items
                            .into_iter()
                            .filter(|item| matches!(item.action, ToolbarAction::WordWrap))
                        {
                            menu = menu.item(reader_menu_item(item, owner.clone()));
                        }
                        menu
                    }),
            )
            .into_any_element()
    }

    pub(super) fn run_toolbar_action(
        &mut self,
        action: ToolbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut props = self.usage_context();
        props.push(("action", superfile_services::telemetry::text(action.id())));
        superfile_services::telemetry::track("reader_action", props);
        match action {
            ToolbarAction::Extract => self.extract_archive(window, cx),
            ToolbarAction::Find => {
                if let Some(reader) = self.active_reader() {
                    reader.find(window, cx);
                }
            }
            ToolbarAction::CancelExtract => {
                self.picker_task = None;
                self.picking = false;
                cx.notify();
            }
            ToolbarAction::Sidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                self.sidebar_window_width = 0.;
                cx.notify();
            }
            ToolbarAction::Back => self.navigate(false, window, cx),
            ToolbarAction::Forward => self.navigate(true, window, cx),
            ToolbarAction::TogglePreview | ToolbarAction::Preview | ToolbarAction::Source => {
                if self.active_reader().is_some_and(|r| r.can_preview()) {
                    let preview = match action {
                        ToolbarAction::TogglePreview => self
                            .active_reader()
                            .is_some_and(|r| r.mode() != superfile_core::ReaderMode::Preview),
                        _ => matches!(action, ToolbarAction::Preview),
                    };
                    self.set_reader_preview(preview, window, cx);
                }
            }
            ToolbarAction::WordWrap => {
                if self.active_reader().is_some_and(|r| r.can_word_wrap()) {
                    self.toggle_word_wrap(window, cx);
                }
            }
            ToolbarAction::FileFullscreen => {
                if self.page == Page::Document && self.active_reader().is_some() {
                    self.set_file_fullscreen(!self.file_fullscreen, window);
                    self.results.clear();
                    self.search_task = None;
                    self.history_swipe = None;
                    // Keep the existing reader and its editing/preview state alive.
                    window.focus(&self.focus, cx);
                    cx.notify();
                }
            }
        }
    }
}

fn reader_menu_item(item: ToolbarItem, owner: WeakEntity<Workspace>) -> PopupMenuItem {
    let mut menu_item = PopupMenuItem::new(superfile_services::locale::tr(item.label))
        .checked(item.selected)
        .disabled(!item.enabled);
    if let Some(icon) = item.icon {
        menu_item = menu_item.icon(icon);
    }
    menu_item = match item.action {
        ToolbarAction::TogglePreview => menu_item.action(Box::new(TogglePreview)),
        ToolbarAction::WordWrap => menu_item.action(Box::new(ToggleWordWrap)),
        _ => menu_item,
    };
    menu_item.on_click(move |_, window, cx| {
        let _ = owner.update(cx, |workspace, cx| {
            workspace.run_toolbar_action(item.action, window, cx)
        });
    })
}

#[cfg(test)]
mod tests {
    use super::{Page, Workspace};
    use crate::app::actions::{TogglePreview, ToggleWordWrap};
    use crate::views::reader::Reader;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Entity, Focusable, KeyBinding, Point, TestAppContext,
        WindowBounds, WindowOptions, px, size, test::TestWindowExt,
    };
    use superfile_core::{Document, DocumentKind, ReaderMode};
    use superfile_services::locale::tr;

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::app::remote::init(Default::default(), cx);
            cx.bind_keys([
                KeyBinding::new("secondary-shift-v", TogglePreview, None),
                KeyBinding::new("alt-z", ToggleWordWrap, None),
            ]);
        });
        cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(800.), px(480.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut workspace = Workspace::new(None, window, cx);
                        for (name, text) in
                            [("menu.json", "{\"saved\":true}"), ("menu.txt", "Text")]
                        {
                            let path = std::env::temp_dir().join(name);
                            let mut reader = Reader::new(
                                Document {
                                    kind: DocumentKind::from_path(&path),
                                    path,
                                    text: text.into(),
                                },
                                window,
                                cx,
                            );
                            reader.set_preview(true, cx);
                            workspace.tabs.push(reader);
                        }
                        workspace.page = Page::Document;
                        workspace.active = Some(0);
                        workspace
                    })
                },
            )
            .unwrap()
        })
    }

    #[gpui_kit::test]
    fn menu_switches_views_without_replacing_edits_or_leaving_focus_in_hidden_source(
        cx: &mut TestAppContext,
    ) {
        let (handle, view) = open(cx);
        let editor = cx.update(|cx| view.read(cx).active_reader().unwrap().editor().unwrap());
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for old_button in ["reader-preview", "reader-source", "word-wrap"] {
                assert!(window.try_find(old_button).is_none());
            }
            window.click("reader-view-menu", cx);
            assert_eq!(
                window.within("popup-menu").find(0usize).label(),
                Some(tr("Toggle Preview").as_str())
            );
            assert_eq!(
                window.within("popup-menu").find(2usize).label(),
                Some(tr("JSON tree").as_str())
            );
            assert_eq!(
                window.within("popup-menu").find(5usize).label(),
                Some(tr("Word wrap · Source view").as_str())
            );
            let wrap = view.read(cx).word_wrap;
            window.within("popup-menu").click(5usize, cx);
            assert_eq!(view.read(cx).word_wrap, wrap);
            assert!(window.find("popup-menu").visible());

            window.within("popup-menu").click(3usize, cx);
            assert_eq!(
                view.read(cx).active_reader().unwrap().mode(),
                ReaderMode::Source
            );
            assert!(editor.read(cx).focus_handle(cx).is_focused(window));
            window.press("secondary-a", cx);
            window.input("{\"edited\":true}", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(view.read(cx).active_reader().unwrap().is_dirty());

            window.click("reader-view-menu", cx);
            assert_eq!(
                window.within("popup-menu").find(5usize).label(),
                Some(tr("Word wrap").as_str())
            );
            window.within("popup-menu").click(0usize, cx);
            let reader = view.read(cx).active_reader().unwrap();
            assert_eq!(reader.mode(), ReaderMode::Preview);
            assert_eq!(reader.editor().unwrap(), editor);
            assert_eq!(editor.read(cx).value().as_ref(), "{\"edited\":true}");
            assert!(view.read(cx).focus.is_focused(window));
            assert!(!editor.read(cx).focus_handle(cx).is_focused(window));

            window.press("secondary-shift-v", cx);
            assert_eq!(
                view.read(cx).active_reader().unwrap().mode(),
                ReaderMode::Source
            );
            assert!(editor.read(cx).focus_handle(cx).is_focused(window));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn menu_rebuilds_for_the_active_file_and_keeps_plain_text_source_only(cx: &mut TestAppContext) {
        let (handle, view) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| workspace.activate_tab(1, window, cx));
            window.render_frame(cx);
            window.click("reader-view-menu", cx);
            assert_eq!(
                window.within("popup-menu").find(0usize).label(),
                Some(tr("Word wrap").as_str())
            );
            assert!(window.within("popup-menu").try_find(1usize).is_none());
            window.click("reader-view-menu", cx);
            view.update(cx, |workspace, cx| workspace.activate_tab(0, window, cx));
            window.render_frame(cx);
            window.click("reader-view-menu", cx);
            assert_eq!(
                window.within("popup-menu").find(2usize).label(),
                Some(tr("JSON tree").as_str())
            );
            let trigger = window.find("reader-view-menu").bounds();
            assert!(trigger.origin.x >= px(0.) && trigger.right() <= px(800.));
            let menu = window.find("popup-menu").bounds();
            assert!(menu.origin.x >= px(0.) && menu.right() <= px(800.));
        })
        .unwrap();
    }
}
