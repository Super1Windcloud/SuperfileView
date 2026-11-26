use super::Workspace;
use super::reader_actions::{ToolbarAction, ToolbarItem, document_items};
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
        let mut document = self
            .active_reader()
            .map(|r| document_items(r, self.word_wrap, cx))
            .unwrap_or_default();
        if self.picking && self.active_reader().is_some_and(|r| r.is_archive()) {
            for item in &mut document {
                if matches!(item.action, ToolbarAction::Extract) {
                    *item = ToolbarItem::new(ToolbarAction::CancelExtract);
                }
            }
        }
        if self.active_reader().is_some() {
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
        toolbar
            .children(self.share_file_button(cx))
            .child(self.application_menu(cx))
            .into_any_element()
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
                if self.active_reader().is_some() {
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
    use super::super::{Tab, Workspace};
    use crate::app::actions::{
        About, CheckForUpdates, DefaultApplications, KeyboardShortcuts, TogglePreview,
        ToggleWordWrap,
    };
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
            cx.set_global(crate::app::AppearancePreference(
                superfile_services::preferences::Appearance::System,
            ));
            cx.set_global(crate::app::LanguagePreference("system".into()));
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
                        workspace.file_statistics = cx.new(|_| {
                            crate::views::reader::statistics::FileStatistics::new(
                                superfile_services::work::scheduler::Scheduler::from_policy(),
                            )
                        });
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
                            workspace.tabs.push(Tab::document(reader));
                        }
                        workspace.empty_home = false;
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

    #[gpui_kit::test]
    fn share_menu_is_reachable_in_the_document_and_in_fullscreen(cx: &mut TestAppContext) {
        let (handle, view) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            for fullscreen in [false, true] {
                view.update(cx, |workspace, _| {
                    workspace.file_fullscreen = fullscreen;
                    workspace.sidebar_visible = false;
                });
                window.render_frame(cx);
                assert!(
                    window.find("share-file").bounds().right()
                        <= window.find("application-menu").bounds().origin.x
                );
                window.click("share-file", cx);
                let menu = window.within("popup-menu");
                let share_label = if cfg!(target_os = "linux") {
                    "Share via Email…"
                } else {
                    "System Share…"
                };
                assert_eq!(menu.find(0).label(), Some(tr(share_label).as_str()));
                assert_eq!(menu.find(1).label(), Some(tr("Copy File").as_str()));
                assert_eq!(menu.find(2).label(), Some(tr("Save a Copy…").as_str()));
                window.click("share-file", cx);
            }
            view.update(cx, |workspace, cx| workspace.show_home(cx));
            window.render_frame(cx);
            assert!(window.try_find("share-file").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn share_menu_saves_unsaved_text_to_the_selected_copy(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("copy.txt");
        let (handle, view) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            view.update(cx, |workspace, cx| workspace.activate_tab(1, window, cx));
            window.render_frame(cx);
            window.press("secondary-a", cx);
            window.input("草稿\nUnsaved", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(view.read(cx).active_reader().unwrap().is_dirty());
            window.render_frame(cx);
            window.click("share-file", cx);
            window.within("popup-menu").click(2usize, cx);
        })
        .unwrap();
        cx.simulate_new_path_selection(|_| Some(destination.clone()));
        cx.run_until_parked();
        cx.update(|cx| {
            let workspace = view.read(cx);
            assert!(workspace.error.is_none(), "{:?}", workspace.error);
            assert!(!workspace.picking);
            assert!(workspace.active_reader().unwrap().is_dirty());
        });
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "草稿\nUnsaved"
        );
    }

    #[gpui_kit::test]
    fn document_footer_follows_the_active_tab_and_unsaved_edits(cx: &mut TestAppContext) {
        use std::time::Duration;
        use superfile_services::text_statistics::TextStatistics;

        let (handle, view) = open(cx);
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let counts = view.read(cx).file_statistics.read(cx).counts();
            assert_eq!(counts, Some(TextStatistics { words: 2, lines: 1 }));
            let footer = window.find("file-statistics").bounds();
            assert!(footer.bottom() <= window.viewport_size().height);
            assert!(footer.bottom() >= window.viewport_size().height - px(1.));
            view.update(cx, |workspace, cx| workspace.activate_tab(1, window, cx));
            window.render_frame(cx);
            window.press("secondary-a", cx);
            window.input("中文 hello\nlast line", cx);
        })
        .unwrap();
        cx.executor().advance_clock(Duration::from_millis(140));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let workspace = view.read(cx);
            assert_eq!(
                workspace.file_statistics.read(cx).counts(),
                Some(TextStatistics { words: 5, lines: 2 })
            );
            assert!(workspace.active_reader().unwrap().is_dirty());
            view.update(cx, |workspace, cx| workspace.activate_tab(0, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                view.read(cx).file_statistics.read(cx).counts(),
                Some(TextStatistics { words: 2, lines: 1 })
            );
            view.update(cx, |workspace, cx| workspace.show_home(cx));
            window.render_frame(cx);
            assert!(window.try_find("file-statistics").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn document_footer_switches_between_non_text_readers(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        std::fs::write(
            &path,
            include_bytes!("../../../../../assets/file-icons/document.png"),
        )
        .unwrap();
        let bytes = path.metadata().unwrap().len();
        let picture = superfile_services::documents::read_content(&path).unwrap();
        let (handle, view) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                workspace
                    .tabs
                    .push(Tab::document(Reader::from_content(picture, window, cx)));
                workspace.activate_tab(workspace.tabs.len() - 1, window, cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("file-statistics").label(),
                Some(format!("PNG · 48 × 48 px · {bytes} B").as_str())
            );
            view.update(cx, |workspace, cx| {
                let missing = directory.path().join("missing.pdf");
                workspace.tabs.push(Tab::document(Reader::unavailable(
                    missing,
                    "missing".to_owned().into(),
                    window,
                    cx,
                )));
                workspace.activate_tab(workspace.tabs.len() - 1, window, cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("file-statistics").label(), Some("PDF"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn document_footer_counts_past_the_end_of_the_editable_page(cx: &mut TestAppContext) {
        use std::time::Duration;
        use superfile_services::{paged, text_statistics::TextStatistics};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("paged.txt");
        let text = format!("{} 中\nend\n", "a".repeat(paged::PAGE_BYTES as usize + 4));
        std::fs::write(&path, text).unwrap();
        let first = paged::read_page(&path, 0, false).unwrap();
        assert!(first.more);
        let (handle, view) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                workspace
                    .tabs
                    .push(Tab::document(Reader::from_page(first, window, cx)));
                workspace.activate_tab(workspace.tabs.len() - 1, window, cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                view.read(cx).file_statistics.read(cx).counts(),
                Some(TextStatistics { words: 3, lines: 2 })
            );
            let editor = view.read(cx).active_reader().unwrap().editor().unwrap();
            window.render_frame(cx);
            window.click(("input", editor.entity_id()), cx);
            window.press("secondary-a", cx);
            window.input("word\n更", cx);
            assert_eq!(editor.read(cx).text().to_string(), "word\n更");
        })
        .unwrap();
        cx.executor().advance_clock(Duration::from_millis(140));
        cx.run_until_parked();
        cx.update(|cx| {
            let workspace = view.read(cx);
            assert_eq!(
                workspace.file_statistics.read(cx).counts(),
                Some(TextStatistics { words: 5, lines: 3 })
            );
            assert!(workspace.active_reader().unwrap().is_dirty());
        });
    }

    fn click_application_entry(label: &str, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
        window.click("application-menu", cx);
        let bounds = window.find("popup-menu").bounds();
        assert!(bounds.origin.x >= px(0.) && bounds.right() <= window.viewport_size().width);
        assert!(bounds.origin.y >= px(0.) && bounds.bottom() <= window.viewport_size().height);
        let index = (0usize..24)
            .find(|index| {
                window
                    .within("popup-menu")
                    .try_find(*index)
                    .is_some_and(|item| item.label() == Some(tr(label).as_str()))
            })
            .unwrap_or_else(|| panic!("Missing application entry: {label}"));
        window.within("popup-menu").click(index, cx);
    }

    #[gpui_kit::test]
    fn application_menu_opens_preferences_with_sidebar_hidden_and_preserves_document(
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::component::WindowExt as _;

        let (handle, view) = open(cx);
        let editor = cx.update(|cx| view.read(cx).active_reader().unwrap().editor().unwrap());
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, _| workspace.sidebar_visible = false);
            window.render_frame(cx);
            click_application_entry("Settings…", window, cx);
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            assert!(matches!(view.read(cx).active_tab(), Some(Tab::Settings)));
            assert!(
                window
                    .within("document-tabs")
                    .find(("close-tab", 2usize))
                    .visible()
            );
            assert!(window.find("browse-community-themes").visible());
            assert!(!view.read(cx).sidebar_visible);
            window.click("settings-code-theme", cx);
            assert!(window.has_active_dialog(cx));
            window.close_dialog(cx);
            window.render_frame(cx);
            click_application_entry("Browse themes…", window, cx);
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            assert!(matches!(view.read(cx).active_tab(), Some(Tab::Themes)));
            assert!(
                window
                    .within("document-tabs")
                    .find(("close-tab", 3usize))
                    .visible()
            );
            let doc_index = view
                .read(cx)
                .tabs
                .iter()
                .position(|tab| {
                    tab.as_reader()
                        .is_some_and(|r| r.editor() == Some(editor.clone()))
                })
                .unwrap();
            view.update(cx, |workspace, cx| {
                workspace.activate_tab(doc_index, window, cx)
            });
            assert_eq!(
                view.read(cx).active_reader().unwrap().editor(),
                Some(editor.clone())
            );
            assert_eq!(editor.read(cx).value().as_ref(), "{\"saved\":true}");
            window.render_frame(cx);
            window.click("application-menu", cx);
            window.within("popup-menu").hover(1usize, cx);
            window
                .within("submenu")
                .within("popup-menu")
                .click(0usize, cx);
        })
        .unwrap();
        cx.update(|cx| assert!(view.read(cx).sidebar_visible));
    }

    #[gpui_kit::test]
    fn non_dialog_app_views_open_and_switch_as_workspace_tabs(cx: &mut TestAppContext) {
        use gpui_kit::component::WindowExt as _;

        let (handle, view) = open(cx);
        cx.update(crate::app::shortcuts::Shortcuts::init_test);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).tabs.len(), 2);

            view.update(cx, |workspace, cx| workspace.show_settings(window, cx));
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            assert!(matches!(view.read(cx).active_tab(), Some(Tab::Settings)));
            assert!(window.find("browse-community-themes").visible());

            view.update(cx, |workspace, cx| workspace.show_shortcuts(window, cx));
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            assert!(matches!(
                view.read(cx).active_tab(),
                Some(Tab::Shortcuts(_))
            ));
            assert!(window.find("restore-all-shortcuts").visible());

            view.update(cx, |workspace, cx| workspace.show_about(window, cx));
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            assert!(matches!(view.read(cx).active_tab(), Some(Tab::About(_))));
            assert!(window.within("about-page").find("about-github").visible());

            view.update(cx, |workspace, cx| workspace.show_themes(window, cx));
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            assert!(matches!(view.read(cx).active_tab(), Some(Tab::Themes)));

            view.update(cx, |workspace, cx| workspace.show_home(cx));
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            assert!(matches!(view.read(cx).active_tab(), Some(Tab::Start)));
            assert!(window.find("welcome-new-file").visible());

            // All non-dialog views and the 2 initial documents coexist in the tab bar.
            assert_eq!(view.read(cx).tabs.len(), 7);
            for index in 0..7usize {
                assert!(
                    window
                        .within("document-tabs")
                        .try_find(("close-tab", index))
                        .is_some()
                );
            }
            assert!(
                window
                    .within("document-tabs")
                    .find(("close-tab", 0usize))
                    .visible()
            );

            // Re-opening a singleton non-dialog view focuses its existing tab without duplicating.
            view.update(cx, |workspace, cx| workspace.show_settings(window, cx));
            window.render_frame(cx);
            assert_eq!(view.read(cx).tabs.len(), 7);
            assert!(matches!(view.read(cx).active_tab(), Some(Tab::Settings)));

            // Closing the active Settings tab leaves the remaining tabs intact.
            let active = view.read(cx).active.unwrap();
            assert!(
                window
                    .within("document-tabs")
                    .find(("close-tab", active))
                    .visible()
            );
            window
                .within("document-tabs")
                .click(("close-tab", active), cx);
            assert_eq!(view.read(cx).tabs.len(), 6);
            assert!(
                !view
                    .read(cx)
                    .tabs
                    .iter()
                    .any(|tab| matches!(tab, Tab::Settings))
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn application_menu_dispatches_utilities_on_start_and_in_file_fullscreen(
        cx: &mut TestAppContext,
    ) {
        let (handle, view) = open(cx);
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            crate::app::text_zoom::init_test(0, cx);
            let log = calls.clone();
            cx.on_action(move |_: &About, _| log.borrow_mut().push("About Superfile"));
            let log = calls.clone();
            cx.on_action(move |_: &CheckForUpdates, _| log.borrow_mut().push("Check for Updates…"));
            let log = calls.clone();
            cx.on_action(move |_: &DefaultApplications, _| {
                log.borrow_mut().push("Default File Types…")
            });
            let log = calls.clone();
            cx.on_action(move |_: &KeyboardShortcuts, _| {
                log.borrow_mut().push("Keyboard Shortcuts…")
            });
        });
        for (home, fullscreen) in [(true, false), (false, true)] {
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |workspace, cx| {
                    if home {
                        workspace.show_home(cx);
                    } else {
                        let doc_index = workspace
                            .tabs
                            .iter()
                            .position(|tab| matches!(tab, Tab::Document(_)))
                            .unwrap();
                        workspace.activate_tab(doc_index, window, cx);
                    }
                    workspace.file_fullscreen = fullscreen;
                    workspace.sidebar_visible = false;
                });
                window.render_frame(cx);
                let trigger = window.find("application-menu").bounds();
                assert!(trigger.origin.x >= px(0.) && trigger.right() <= px(800.));
            })
            .unwrap();
            for label in [
                "About Superfile",
                "Check for Updates…",
                "Default File Types…",
                "Keyboard Shortcuts…",
            ] {
                cx.update_window(handle, |_, window, cx| {
                    click_application_entry(label, window, cx);
                })
                .unwrap();
                assert_eq!(calls.borrow().last(), Some(&label));
            }
            for (label, size) in [
                ("Increase text size", px(14.)),
                ("Reset text size", px(13.)),
            ] {
                cx.update_window(handle, |_, window, cx| {
                    window.click("application-menu", cx);
                    window.within("popup-menu").hover(1usize, cx);
                    let bounds = window.within("submenu").find("popup-menu").bounds();
                    assert!(
                        bounds.origin.x >= px(0.) && bounds.right() <= window.viewport_size().width
                    );
                    let mut submenu = window.within("submenu");
                    let mut menu = submenu.within("popup-menu");
                    let index = (0usize..10)
                        .find(|index| {
                            menu.try_find(*index)
                                .is_some_and(|item| item.label() == Some(tr(label).as_str()))
                        })
                        .unwrap_or_else(|| panic!("Missing view entry: {label}"));
                    menu.click(index, cx);
                })
                .unwrap();
                cx.update(|cx| assert_eq!(crate::app::text_zoom::font_size(px(13.), cx), size));
            }
        }
        assert_eq!(calls.borrow().len(), 8);
    }
}
