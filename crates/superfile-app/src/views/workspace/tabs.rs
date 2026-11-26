use super::{Tab, Workspace};
use crate::views::reader::Reader;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
    },
    prelude::FluentBuilder,
    *,
};
use superfile_services::locale::tr;

#[derive(Clone)]
struct DraggedTab {
    workspace: EntityId,
    source: WeakEntity<Workspace>,
    window: AnyWindowHandle,
    target: super::header::Target,
    label: String,
}
impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let source = self.source.clone();
        let source_window = self.window;
        let target = self.target.clone();
        h_flex()
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let source = source.clone();
                        let target = target.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase != DispatchPhase::Capture
                                || event.button != MouseButton::Left
                                || window.window_handle() != source_window
                                || !cx.has_active_drag()
                                || Bounds::new(Point::default(), window.viewport_size())
                                    .contains(&event.position)
                            {
                                return;
                            }
                            cx.stop_active_drag(window);
                            let _ =
                                source.update(cx, |view, cx| view.detach_tab(&target, window, cx));
                            cx.stop_propagation();
                        });
                    },
                )
                .absolute()
                .size_0(),
            )
            .px_3()
            .py_1()
            .rounded(px(7.))
            .bg(cx.theme().secondary)
            .text_xs()
            .child(self.label.clone())
    }
}

enum TabMenuAction {
    Close(Vec<super::header::Target>),
    Trash(std::path::PathBuf),
    Detach(super::header::Target),
    CopyPath(std::path::PathBuf),
    Reveal(std::path::PathBuf),
}

impl Workspace {
    fn detach_tab(
        &mut self,
        target: &super::header::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_saving()
            || self.loading
            || self.confirm_task.is_some()
            || crate::app::updates::busy(cx)
        {
            self.error =
                Some("A file operation is in progress. Please retry after it finishes.".into());
            cx.notify();
            return;
        }
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| super::header::Target::from_tab(tab) == *target)
        else {
            return;
        };
        if !matches!(self.tabs[index], Tab::Document(_)) {
            return;
        }
        if !self.tabs[index].can_move_to_window(cx) {
            self.error =
                Some("A file operation is in progress. Please retry after it finishes.".into());
            cx.notify();
            return;
        }
        // Create the destination before taking ownership of the live reader.
        let (destination, handle) = match crate::app::open_detached_window(cx) {
            Ok(destination) => destination,
            Err(error) => {
                self.error = Some(format!("{error:#}").into());
                cx.notify();
                return;
            }
        };
        let Tab::Document(reader) = self.tabs.remove(index) else {
            return;
        };
        let reader = *reader;
        reader.deactivate(cx);
        if let Some(editor) = reader.editor() {
            let id = editor.entity_id();
            self.autosave.remove(&id);
            // The destination takes over the same recovery record.
            self.recovery_keys.remove(&id);
            self.save_queue.retain(|queued| *queued != id);
        }
        self.active = selection_after_close(self.active, index, self.tabs.len() + 1);
        if self.tabs.is_empty() {
            self.empty_home = true;
        }
        self.observe_autosave(window, cx);
        if let Some(active) = self.active {
            self.activate_tab(active, window, cx);
            if let Some(reader) = self.tabs[active].as_reader() {
                reader.activate(cx);
            }
        }
        if let Some(session) = self.window_session(cx) {
            crate::app::workspace_sessions::remember(session, cx);
        }
        self.sync_workspace(cx);
        cx.notify();
        // The just-created window cannot close between these synchronous updates.
        handle
            .update(cx, |_, window, cx| {
                destination.update(cx, |view, cx| view.receive_detached_tab(reader, window, cx));
            })
            .expect("Newly created tab window");
    }

    fn receive_detached_tab(
        &mut self,
        reader: Reader,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(remote) = reader.document.remote.clone() {
            self.remote = Some(remote);
            self.recent_id = format!("ssh-workspace-detached-{}", self.recent_id);
            self.roots = reader
                .document
                .path
                .parent()
                .map(|path| vec![path.to_path_buf()])
                .unwrap_or_default();
            self.root = self.roots.first().cloned();
            self.remember_remote(cx);
        }
        self.workspace_name = Some(
            reader
                .document
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| reader.title.to_string()),
        );
        reader.bind_window(window, cx);
        let editor = reader.editor();
        let autosave = reader.document.draft_directory.is_none() && reader.is_dirty();
        self.empty_home = false;
        self.tabs.push(Tab::document(reader));
        self.observe_autosave(window, cx);
        self.activate_tab(0, window, cx);
        if let Some(reader) = self.tabs[0].as_reader() {
            reader.activate(cx);
        }
        self.watch_file_parent(window, cx);
        if let Some(editor) = editor {
            self.checkpoint_editor(editor.entity_id(), cx);
            if autosave {
                self.save_editor(editor.entity_id(), window, cx);
            }
        }
        self.sync_workspace(cx);
        self.remember_session(cx);
    }

    pub(super) fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    pub(super) fn active_reader(&self) -> Option<&Reader> {
        self.active_tab().and_then(Tab::as_reader)
    }

    pub(super) fn reveal_active_file(&self, cx: &mut Context<Self>) {
        if let (Some(explorer), Some(reader)) = (&self.explorer, self.active_reader())
            && reader.document.draft_directory.is_none()
        {
            explorer.update(cx, |explorer, cx| {
                explorer.reveal_path(reader.document.path.clone(), cx)
            });
        }
    }

    pub(super) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.tabs.get(index) {
            self.empty_home = false;
            if let Some(reader) = tab.as_reader() {
                superfile_services::telemetry::file_event(
                    "tab_activated",
                    &reader.document.path,
                    [],
                );
                log::info!("tab activate file={}", reader.document.path.display());
            }
            if self.loading_path.is_some() {
                self.load_task = None;
                self.loading = false;
                self.loading_path = None;
            }
            self.load_incoming_file(window, cx);
            self.active = Some(index);
            self.reveal_active_file(cx);
            window.focus(&self.focus, cx);
            if let Some(reader) = self.tabs[index].as_reader()
                && reader.editor().is_some()
                && reader.mode() == superfile_core::ReaderMode::Source
            {
                reader.focus_source(window, cx);
            }
            self.tab_scroll.scroll_to_item(index);
            window.set_window_title("Superfile");
            self.record_history(cx);
            cx.notify();
        }
    }

    fn close_tabs(
        &mut self,
        targets: Vec<super::header::Target>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }
        let closes_document = targets
            .iter()
            .any(|target| matches!(target, super::header::Target::File(_)));
        if (self.is_saving() && closes_document)
            || self
                .tabs
                .iter()
                .any(|t| t.is_dirty() && targets.contains(&super::header::Target::from_tab(t)))
        {
            self.confirm_discard(
                super::editing::DiscardAction::CloseTabs(targets),
                window,
                cx,
            );
        } else {
            self.close_tabs_unchecked(targets, window, cx);
        }
    }
    pub(super) fn close_tabs_unchecked(
        &mut self,
        targets: Vec<super::header::Target>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closes_document = targets
            .iter()
            .any(|target| matches!(target, super::header::Target::File(_)));
        if self.is_saving() && closes_document {
            self.error =
                Some("A file operation is in progress. Please retry after it finishes.".into());
            cx.notify();
            return;
        }
        let active = self.active_tab().map(super::header::Target::from_tab);
        let previous = self.active.unwrap_or(0);
        self.tabs.retain(|tab| {
            let keep = !targets.contains(&super::header::Target::from_tab(tab));
            if !keep && let Some(r) = tab.as_reader() {
                superfile_services::telemetry::file_event(
                    "tab_closed",
                    &r.document.path,
                    [("unsaved", superfile_services::telemetry::flag(r.is_dirty()))],
                );
                r.deactivate(cx);
            }
            keep
        });
        if self.loading_path.is_some() {
            self.load_task = None;
            self.loading = false;
            self.loading_path = None;
        }
        self.load_incoming_file(window, cx);
        self.active = active
            .and_then(|target| {
                self.tabs
                    .iter()
                    .position(|tab| super::header::Target::from_tab(tab) == target)
            })
            .or_else(|| (!self.tabs.is_empty()).then(|| previous.min(self.tabs.len() - 1)));
        if self.tabs.is_empty() {
            self.empty_home = true;
        }
        self.observe_autosave(window, cx);
        if let Some(index) = self.active {
            self.activate_tab(index, window, cx);
        } else {
            window.set_window_title("Superfile");
            window.focus(&self.focus, cx);
        }
        self.remember_session(cx);
        cx.notify();
    }
    fn show_tab_menu(
        &mut self,
        target: &super::header::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| super::header::Target::from_tab(tab) == *target)
        else {
            return;
        };
        let mut items = Vec::new();
        let mut actions = Vec::new();
        if let Some(reader) = self.tabs[index].as_reader() {
            items.push((
                Some(superfile_services::locale::tr("Move Tab to New Window")),
                !self.is_saving()
                    && !self.loading
                    && reader.can_move_to_window(cx)
                    && self.confirm_task.is_none()
                    && !crate::app::updates::busy(cx),
            ));
            actions.push(Some(TabMenuAction::Detach(target.clone())));
            if reader.document.draft_directory.is_none() {
                let path = reader.document.path.clone();
                if reader.document.remote.is_none() {
                    items.push((
                        Some(superfile_services::locale::tr("Reveal in File Manager")),
                        true,
                    ));
                    actions.push(Some(TabMenuAction::Reveal(path.clone())));
                }
                items.push((
                    Some(superfile_services::locale::tr("Copy Absolute Path")),
                    true,
                ));
                let relative = self
                    .roots
                    .iter()
                    .filter_map(|root| path.strip_prefix(root).ok())
                    .min_by_key(|relative| relative.components().count())
                    .map(ToOwned::to_owned);
                actions.push(Some(TabMenuAction::CopyPath(path)));
                if let Some(relative) = relative {
                    items.push((
                        Some(superfile_services::locale::tr("Copy Relative Path")),
                        true,
                    ));
                    actions.push(Some(TabMenuAction::CopyPath(relative)));
                }
            }
            items.push((None, true));
            actions.push(None);
        }
        for (label, mode) in [
            ("Close", 0),
            ("Close Tabs to the Right", 1),
            ("Close Other Tabs", 2),
            ("Close All Tabs", 3),
        ] {
            let targets: Vec<_> = self
                .tabs
                .iter()
                .enumerate()
                .filter(|(i, _)| match mode {
                    0 => *i == index,
                    1 => *i > index,
                    2 => *i != index,
                    _ => true,
                })
                .map(|(_, tab)| super::header::Target::from_tab(tab))
                .collect();
            items.push((
                Some(superfile_services::locale::tr(label)),
                !targets.is_empty(),
            ));
            actions.push(Some(TabMenuAction::Close(targets)));
        }
        if let Some(reader) = self.tabs[index].as_reader()
            && reader.document.draft_directory.is_none()
            && reader.document.remote.is_none()
        {
            items.push((None, true));
            actions.push(None);
            items.push((Some(superfile_services::locale::tr("Move to Trash…")), true));
            actions.push(Some(TabMenuAction::Trash(reader.document.path.clone())));
        }
        self.context_menu_task = None;
        let Some(menu) = crate::views::context_menu::ContextMenu::new(&items, window, cx) else {
            return;
        };
        self.context_menu_task = Some(cx.spawn_in(window, async move |view, cx| {
            let selected = menu.choose(cx).await;
            let _ = view.update_in(cx, |view, window, cx| {
                if let Some(action) = selected
                    .and_then(|index| actions.get_mut(index))
                    .and_then(Option::take)
                {
                    match action {
                        TabMenuAction::Detach(target) => view.detach_tab(&target, window, cx),
                        TabMenuAction::CopyPath(path) => cx.write_to_clipboard(
                            ClipboardItem::new_string(path.to_string_lossy().into_owned()),
                        ),
                        TabMenuAction::Reveal(path) => cx.reveal_path(&path),
                        TabMenuAction::Close(targets) => view.close_tabs(targets, window, cx),
                        TabMenuAction::Trash(path) => view.confirm_trash(path, window, cx),
                    }
                }
            });
        }));
    }

    pub(super) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let is_document = self
            .tabs
            .get(index)
            .is_some_and(|tab| matches!(tab, Tab::Document(_)));
        if self.tabs.get(index).is_some_and(Tab::is_dirty) || (is_document && self.is_saving()) {
            self.confirm_discard(super::editing::DiscardAction::CloseTab(index), window, cx);
        } else {
            self.close_tab_unchecked(index, window, cx);
        }
    }

    pub(super) fn close_tab_unchecked(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() {
            return;
        }
        if self.loading_path.is_some() {
            self.load_task = None;
            self.loading = false;
            self.loading_path = None;
        }
        self.load_incoming_file(window, cx);
        self.active = selection_after_close(self.active, index, self.tabs.len());
        if let Some(reader) = self.tabs[index].as_reader() {
            superfile_services::telemetry::file_event(
                "tab_closed",
                &reader.document.path,
                [(
                    "unsaved",
                    superfile_services::telemetry::flag(reader.is_dirty()),
                )],
            );
            reader.deactivate(cx);
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.empty_home = true;
        }
        self.observe_autosave(window, cx);
        if let Some(active) = self.active {
            self.activate_tab(active, window, cx);
        } else {
            window.set_window_title("Superfile");
            window.focus(&self.focus, cx);
        }
        self.remember_session(cx);
        cx.notify();
    }
}

impl Workspace {
    fn reorder_tab(
        &mut self,
        source: &super::header::Target,
        destination: &super::header::Target,
        after: bool,
        cx: &mut Context<Self>,
    ) {
        // Resolve identities at drop time: a tab may have closed during the drag.
        let Some(from) = self
            .tabs
            .iter()
            .position(|tab| super::header::Target::from_tab(tab) == *source)
        else {
            return;
        };
        let Some(target) = self
            .tabs
            .iter()
            .position(|tab| super::header::Target::from_tab(tab) == *destination)
        else {
            return;
        };
        let gap = target + usize::from(after);
        let to = if from < gap { gap - 1 } else { gap };
        if from == to {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self.active.map(|active| {
            if active == from {
                to
            } else if from < active && active <= to {
                active - 1
            } else if to <= active && active < from {
                active + 1
            } else {
                active
            }
        });
        self.tab_scroll.scroll_to_item(to);
        self.remember_session(cx);
        cx.notify();
    }

    pub(super) fn tab_bar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let active = self.active == Some(index);
                let target = super::header::Target::from_tab(tab);
                let label = tab_label(tab);
                let hint_path = tab_hint(tab);
                let dirty = tab.is_dirty();
                let icon = match tab {
                    Tab::Document(reader) => {
                        if self.pending_open_path() == Some(&reader.document.path) {
                            crate::views::loading_icon(
                                ("file-loading", index),
                                px(16.),
                                tr("Loading…"),
                                cx,
                            )
                        } else {
                            crate::views::file_icons::file_icon(&reader.document.path, cx)
                                .into_any_element()
                        }
                    }
                    Tab::Start => Icon::new(IconName::House)
                        .size(px(15.))
                        .flex_shrink_0()
                        .into_any_element(),
                    Tab::Settings => Icon::new(IconName::Settings)
                        .size(px(15.))
                        .flex_shrink_0()
                        .into_any_element(),
                    Tab::Themes => Icon::new(IconName::Palette)
                        .size(px(15.))
                        .flex_shrink_0()
                        .into_any_element(),
                    Tab::Shortcuts(_) => Icon::new(IconName::Keyboard)
                        .size(px(15.))
                        .flex_shrink_0()
                        .into_any_element(),
                    Tab::About(_) => Icon::new(IconName::Info)
                        .size(px(15.))
                        .flex_shrink_0()
                        .into_any_element(),
                };
                let hint_bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
                let painted_hint_bounds = hint_bounds.clone();
                let drop_bounds = hint_bounds.clone();
                let drop_target = target.clone();
                let dragged = DraggedTab {
                    workspace: cx.entity_id(),
                    source: cx.entity().downgrade(),
                    window: window.window_handle(),
                    target: target.clone(),
                    label: label.clone(),
                };
                let workspace_id = cx.entity_id();
                let drop_color = cx.theme().secondary;
                let close_label = label.clone();

                h_flex()
                    .id(("document-tab", index))
                    .role(Role::Tab)
                    .aria_label(label.clone())
                    .aria_description(hint_path.clone())
                    .aria_selected(active)
                    .tab_index(0)
                    .focus_visible(|style| style.border_color(crate::views::accent(cx)))
                    .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            view.activate_tab(index, window, cx);
                            if let Some(reader) = view.tabs[index].as_reader() {
                                reader.activate(cx);
                            }
                            cx.stop_propagation();
                        }
                    }))
                    .relative()
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                painted_hint_bounds.set(bounds);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .h_full()
                    .border_b_2()
                    .border_color(if active {
                        crate::views::accent(cx)
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .font_weight(if active {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .flex_shrink_0()
                    .gap_1()
                    .pl_3()
                    .pr_2()
                    .text_color(if active {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .hover(|tab| tab.bg(cx.theme().foreground.opacity(0.035)))
                    .cursor_pointer()
                    .on_drag(dragged, |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                    .drag_over::<DraggedTab>(move |style, drag, _, _| {
                        if drag.workspace == workspace_id {
                            style.bg(drop_color)
                        } else {
                            style
                        }
                    })
                    .on_drop(cx.listener(move |view, drag: &DraggedTab, window, cx| {
                        if drag.workspace != cx.entity_id() {
                            return;
                        }
                        let bounds = drop_bounds.get();
                        let after =
                            window.mouse_position().x >= bounds.origin.x + bounds.size.width / 2.;
                        view.reorder_tab(&drag.target, &drop_target, after, cx);
                        cx.stop_propagation();
                    }))
                    .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        view.activate_tab(index, window, cx);
                        if let Some(reader) = view.tabs[index].as_reader() {
                            reader.activate(cx);
                        }
                    }))
                    .child(
                        h_flex()
                            .id(("document-tab-label", index))
                            .h_full()
                            .gap(px(5.))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(hint_path.clone())
                                    .build(window, cx)
                            })
                            .child(icon)
                            .child(div().max_w(px(210.)).truncate().text_xs().child(label)),
                    )
                    .child(
                        Button::new(("close-tab", index))
                            .ghost()
                            .xsmall()
                            .w(px(22.))
                            .h(px(22.))
                            .when(dirty, |button| {
                                button.child(div().size(px(9.)).rounded_full().bg(cx.theme().blue))
                            })
                            .when(!dirty, |button| {
                                button.icon(Icon::new(IconName::X).size(px(12.)))
                            })
                            .text_color(cx.theme().muted_foreground)
                            .tooltip(tr(if dirty {
                                "Unsaved changes · Close"
                            } else {
                                "Close"
                            }))
                            .accessibility_label(format!(
                                "{} {}",
                                tr(if dirty {
                                    "Unsaved changes · Close"
                                } else {
                                    "Close"
                                }),
                                close_label
                            ))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                cx.stop_propagation();
                                view.close_tab(index, window, cx);
                            })),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |view, _, window, cx| {
                            cx.stop_propagation();
                            view.show_tab_menu(&target, window, cx);
                        }),
                    )
            })
            .collect::<Vec<_>>();
        h_flex()
            .h(px(36.))
            .w_full()
            .flex_shrink_0()
            .bg(cx.theme().background)
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.6))
            .child(
                h_flex()
                    .id("document-tabs")
                    .role(Role::TabList)
                    .track_scroll(&self.tab_scroll)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .children(self.pending_open_tab(cx))
                    .children(tabs),
            )
            .into_any_element()
    }

    fn pending_open_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let path = self.pending_open_path()?;
        if self.readers().any(|reader| reader.document.path == *path) {
            return None;
        }
        let label = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let status = format!(
            "{} · {}",
            superfile_services::locale::tr("Loading…"),
            path.display()
        );
        Some(
            h_flex()
                .id("pending-document-tab")
                .role(Role::Tab)
                .aria_label(label.clone())
                .aria_description(status.clone())
                .aria_selected(false)
                .h_full()
                .flex_shrink_0()
                .gap_1()
                .pl_3()
                .pr_2()
                .border_b_2()
                .border_color(gpui_kit::transparent_black())
                .bg(cx.theme().background)
                .child(
                    h_flex()
                        .gap(px(5.))
                        .child(crate::views::loading_icon(
                            "pending-file-loading",
                            px(16.),
                            status,
                            cx,
                        ))
                        .child(div().max_w(px(210.)).truncate().text_xs().child(label)),
                )
                .child(
                    Button::new("cancel-pending-file")
                        .ghost()
                        .xsmall()
                        .w(px(22.))
                        .h(px(22.))
                        .icon(
                            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::X)
                                .size(px(12.)),
                        )
                        .text_color(cx.theme().muted_foreground)
                        .tooltip(superfile_services::locale::tr("Cancel"))
                        .accessibility_label(superfile_services::locale::tr("Cancel"))
                        .on_click(cx.listener(|view, _, window, cx| {
                            cx.stop_propagation();
                            view.cancel_pending_open(window, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}

fn tab_label(tab: &Tab) -> String {
    match tab {
        Tab::Document(reader) => {
            if reader.document.draft_directory.is_some() {
                reader.title.to_string()
            } else {
                reader
                    .document
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }
        }
        Tab::Start => tr("Start"),
        Tab::Settings => tr("Settings"),
        Tab::Themes => tr("Community themes"),
        Tab::Shortcuts(_) => tr("Keyboard shortcuts"),
        Tab::About(_) => tr("About Superfile"),
    }
}

fn tab_hint(tab: &Tab) -> String {
    match tab {
        Tab::Document(reader) => {
            if reader.document.draft_directory.is_some() {
                tr("Not saved to disk")
            } else if let Some(remote) = &reader.document.remote {
                format!(
                    "{} · {}\n{}",
                    tr("Remote · Read-only"),
                    remote.connection.label(),
                    reader.document.path.display()
                )
            } else {
                reader.document.path.display().to_string()
            }
        }
        Tab::Start => tr("Start"),
        Tab::Settings => tr("Settings"),
        Tab::Themes => tr("Community themes"),
        Tab::Shortcuts(_) => tr("Keyboard shortcuts"),
        Tab::About(_) => tr("About Superfile"),
    }
}

fn selection_after_close(active: Option<usize>, closed: usize, count: usize) -> Option<usize> {
    active.and_then(|active| {
        if count <= 1 {
            None
        } else if closed < active {
            Some(active - 1)
        } else {
            Some(active.min(count - 2))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::selection_after_close;

    #[test]
    fn closing_another_tab_preserves_the_selected_document() {
        for count in 2..10 {
            let tabs = (0..count).collect::<Vec<_>>();
            for active in 0..count {
                for closed in 0..count {
                    if active == closed {
                        continue;
                    }
                    let next = selection_after_close(Some(active), closed, count).unwrap();
                    let mut remaining = tabs.clone();
                    remaining.remove(closed);
                    assert_eq!(remaining[next], tabs[active]);
                }
            }
        }
    }

    #[test]
    fn closing_active_selects_a_neighbor_and_last_close_clears_selection() {
        assert_eq!(selection_after_close(Some(1), 1, 3), Some(1));
        assert_eq!(selection_after_close(Some(2), 2, 3), Some(1));
        assert_eq!(selection_after_close(Some(0), 0, 1), None);
    }
}
