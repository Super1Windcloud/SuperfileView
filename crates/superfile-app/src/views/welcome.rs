use crate::app::actions::{NewFile, OpenFile, OpenFolder};
use crate::app::recent_paths::{Kind, RecentPaths};
use crate::app::shortcuts::format_shortcut_for_display;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon, Sizable,
        button::{Button, ButtonCustomVariant, ButtonVariants},
        h_flex,
        menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

fn quick_action(
    id: &'static str,
    label: &'static str,
    action: &dyn Action,
    shortcut: &'static str,
    icon: IconName,
    cx: &App,
) -> Button {
    Button::new(id)
        .ghost()
        .w_full()
        .min_w_0()
        .h(px(40.))
        .px_3()
        .rounded(px(6.))
        .accessibility_label(superfile_services::locale::tr(label))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_3()
                .child(
                    Icon::new(icon)
                        .size(px(17.))
                        .text_color(cx.theme().muted_foreground)
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_left()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(cx.theme().foreground)
                        .child(superfile_services::locale::tr(label)),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format_shortcut_for_display(action, shortcut, cx)),
                ),
        )
}

fn section_heading(id: &'static str, label: &'static str, cx: &App) -> impl IntoElement {
    h_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap_3()
        .mb_2()
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().muted_foreground)
                .child(superfile_services::locale::tr(label).to_uppercase()),
        )
        .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
}

fn recent_paths(
    current: &super::workspace::Workspace,
    kind: Kind,
    cx: &mut Context<super::workspace::Workspace>,
) -> impl IntoElement {
    let (id, title, empty, remove, icon) = match kind {
        Kind::File => (
            "recent-files",
            "Recent Files",
            "No recent files yet.",
            "Remove from Recent Files",
            IconName::FileText,
        ),
        Kind::Directory => (
            "recent-directories",
            "Recent Folders",
            "No recent folders yet.",
            "Remove from Recent Folders",
            IconName::Folder,
        ),
    };
    let paths = cx
        .try_global::<RecentPaths>()
        .map(|history| history.0.paths(kind).to_vec())
        .unwrap_or_default();
    let limit = current.recent_paths_visible(kind);
    let more = paths.len() > limit;
    let empty_list = paths.is_empty();
    v_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .gap_1()
        .child(section_heading(
            match kind {
                Kind::File => "recent-files-heading",
                Kind::Directory => "recent-directories-heading",
            },
            title,
            cx,
        ))
        .when(empty_list, |section| {
            section.child(
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(superfile_services::locale::tr(empty)),
            )
        })
        .children(
            paths
                .into_iter()
                .take(limit)
                .enumerate()
                .map(|(index, path)| {
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned();
                    let full_path = path.display().to_string();
                    let parent = path
                        .parent()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default();
                    let open = path.clone();
                    let remove_path = path.clone();
                    let open_window = path.clone();
                    let owner = cx.entity().downgrade();
                    let context_owner = owner.clone();
                    Button::new((id, index))
                        .ghost()
                        .w_full()
                        .min_w_0()
                        .h(px(34.))
                        .px_3()
                        .rounded(px(6.))
                        .accessibility_label(format!("{name}: {full_path}"))
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap_3()
                                .items_center()
                                .child(
                                    Icon::new(icon)
                                        .size(px(17.))
                                        .flex_shrink_0()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(
                                    div()
                                        .max_w(relative(0.55))
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .font_weight(FontWeight::NORMAL)
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_left()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(parent),
                                ),
                        )
                        .tooltip(full_path)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.open_recent_path(kind, open.clone(), window, cx)
                        }))
                        .context_menu(move |menu, _, _| {
                            let owner = context_owner.clone();
                            let path = path.clone();
                            let open_window = open_window.clone();
                            let remove_path = remove_path.clone();
                            menu.item(
                                PopupMenuItem::new(superfile_services::locale::tr("Open"))
                                    .on_click(move |_, window, cx| {
                                        let _ = owner.update(cx, |view, cx| {
                                            view.open_recent_path(kind, path.clone(), window, cx)
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(superfile_services::locale::tr(
                                    "Open in New Window",
                                ))
                                .on_click(move |_, _, cx| {
                                    crate::app::recent_paths::open_in_new_window(
                                        open_window.clone(),
                                        cx,
                                    )
                                }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new(superfile_services::locale::tr(remove))
                                    .on_click(move |_, _, cx| {
                                        crate::app::recent_paths::remove(
                                            kind,
                                            remove_path.clone(),
                                            cx,
                                        )
                                    }),
                            )
                        })
                }),
        )
        .when(more, |section| {
            section.child(
                Button::new(match kind {
                    Kind::File => "recent-files-more",
                    Kind::Directory => "recent-directories-more",
                })
                .ghost()
                .small()
                .icon(IconName::ChevronDown)
                .label(superfile_services::locale::tr("Show more"))
                .on_click(cx.listener(move |view, _, _, cx| view.show_more_recent_paths(kind, cx))),
            )
        })
}

fn quick_actions(
    current: &super::workspace::Workspace,
    cx: &mut Context<super::workspace::Workspace>,
) -> impl IntoElement {
    let locations = current.remote_locations(cx);
    let location_owner = cx.entity().downgrade();
    v_flex()
        .w_full()
        .gap_1()
        .child(
            quick_action(
                "welcome-new-file",
                "New File",
                &NewFile,
                "secondary-n",
                IconName::FilePlus,
                cx,
            )
            .on_click(|_, window, cx| window.dispatch_action(Box::new(NewFile), cx)),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .min_w_0()
                .child(
                    quick_action(
                        "welcome-new-workspace",
                        "Open Workspace…",
                        &OpenFolder,
                        "secondary-shift-o",
                        IconName::FolderOpen,
                        cx,
                    )
                    .flex_1()
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFolder), cx)),
                )
                .child(
                    Button::new("welcome-workspace-location")
                        .ghost()
                        .h(px(32.))
                        .w(px(32.))
                        .flex_shrink_0()
                        .rounded(px(6.))
                        .text_color(cx.theme().muted_foreground)
                        .icon(Icon::new(IconName::ChevronDown).size(px(14.)))
                        .tooltip(superfile_services::locale::tr("Choose Workspace Location"))
                        .accessibility_label(superfile_services::locale::tr(
                            "Choose Workspace Location",
                        ))
                        .dropdown_menu(move |mut menu, _, _| {
                            menu = menu.item(
                                PopupMenuItem::new(superfile_services::locale::tr(
                                    "This Computer…",
                                ))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(OpenFolder), cx)
                                }),
                            );
                            if !locations.is_empty() {
                                menu = menu.separator();
                            }
                            for connection in &locations {
                                let connection = connection.clone();
                                let owner = location_owner.clone();
                                menu = menu.item(PopupMenuItem::new(connection.label()).on_click(
                                    move |_, window, cx| {
                                        let _ = owner.update(cx, |workspace, cx| {
                                            workspace.show_remote(
                                                Some(connection.clone()),
                                                false,
                                                false,
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                ));
                            }
                            let owner = location_owner.clone();
                            menu.separator().item(
                                PopupMenuItem::new(superfile_services::locale::tr(
                                    "Add SSH Connection…",
                                ))
                                .on_click(move |_, window, cx| {
                                    let _ = owner.update(cx, |workspace, cx| {
                                        workspace.add_remote_connection(window, cx)
                                    });
                                }),
                            )
                        }),
                ),
        )
        .child(
            quick_action(
                "welcome-open-file",
                "Open File…",
                &OpenFile,
                "secondary-o",
                IconName::FileText,
                cx,
            )
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFile), cx)),
        )
}

pub fn welcome(
    current: &super::workspace::Workspace,
    _current_id: EntityId,
    cx: &mut Context<super::workspace::Workspace>,
) -> impl IntoElement {
    let has_workspace = current
        .workspace_window
        .as_ref()
        .and_then(|host| host.upgrade())
        .is_some_and(|host| {
            host.read(cx)
                .slots
                .iter()
                .any(|slot| !slot.workspace.folders.is_empty() || slot.workspace.name.is_some())
        });
    let descriptor = current.workspace_descriptor();
    let center_content =
        !has_workspace && descriptor.folders.is_empty() && descriptor.name.is_none();
    let recent = cx
        .try_global::<crate::app::RecentWorkspaces>()
        .map(|history| history.0.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|workspace| {
            workspace.folders.len() > 1
                || workspace.name.is_some()
                || superfile_services::remote::is_remote_id(&workspace.id)
        })
        .collect::<Vec<_>>();
    let more = recent.len() > current.recent_visible();
    let visible = recent
        .into_iter()
        .take(current.recent_visible())
        .collect::<Vec<_>>();
    let content = v_flex()
        .w_full()
        .max_w(px(660.))
        .min_w_0()
        .flex_shrink_0()
        .gap_6()
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(
                    img("branding/superfile-welcome.png")
                        .size(px(52.))
                        .object_fit(ObjectFit::Contain)
                        .flex_shrink_0(),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(24.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(superfile_services::locale::tr("Welcome to Superfile")),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(superfile_services::locale::tr("Your files, within reach.")),
                        ),
                ),
        )
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .child(section_heading("welcome-get-started", "Get Started", cx))
                .child(quick_actions(current, cx)),
        )
        .child(recent_paths(current, Kind::File, cx))
        .child(recent_paths(current, Kind::Directory, cx))
        .when(!visible.is_empty(), |view| {
            view.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .child(h_flex().gap_2().items_center().child(section_heading(
                        "recent-workspaces-heading",
                        "Recent Workspaces",
                        cx,
                    )))
                    .child(v_flex().w_full().min_w_0().gap_1().children(
                        visible.into_iter().enumerate().map(|(index, workspace)| {
                            let title = workspace.title();
                            let mut paths = workspace
                                .folders
                                .iter()
                                .map(|path| path.display().to_string())
                                .collect::<Vec<_>>()
                                .join(" · ");
                            let remote = cx
                                .global::<crate::app::remote::Connections>()
                                .catalog
                                .workspaces
                                .get(&workspace.id)
                                .and_then(|descriptor| {
                                    cx.global::<crate::app::remote::Connections>()
                                        .catalog
                                        .connections
                                        .iter()
                                        .find(|connection| connection.id == descriptor.connection)
                                });
                            if let Some(connection) = remote {
                                paths = format!("{} · {paths}", connection.label());
                            }
                            let remote_icon =
                                superfile_services::remote::is_remote_id(&workspace.id);
                            let is_current = current.matches_recent_workspace(&workspace);
                            let open = workspace.clone();
                            let new_window = workspace.clone();
                            let remove_id = workspace.id.clone();
                            let context_workspace = workspace.clone();
                            let weak = cx.entity().downgrade();
                            let context_weak = weak;
                            let accent = super::accent(cx);
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .h(px(52.))
                                .items_center()
                                .rounded(px(6.))
                                .when(is_current, |row| {
                                    row.bg(cx.theme().foreground.opacity(0.035))
                                })
                                .hover(|row| row.bg(cx.theme().muted.opacity(0.55)))
                                .child(
                                    Button::new(("recent-workspace", index))
                                        .ghost()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        .px_3()
                                        .justify_start()
                                        .accessibility_label(format!("{title}: {paths}"))
                                        .child(
                                            h_flex()
                                                .w_full()
                                                .min_w_0()
                                                .items_center()
                                                .gap_3()
                                                .child(if remote_icon {
                                                    super::connection_icon(
                                                        ("recent-connection-status", index),
                                                        crate::app::remote::workspace_is_connected(
                                                            &workspace.id,
                                                            cx,
                                                        ),
                                                        px(17.),
                                                        cx,
                                                    )
                                                } else {
                                                    Icon::new(IconName::Folder)
                                                        .size(px(17.))
                                                        .text_color(if is_current {
                                                            accent
                                                        } else {
                                                            cx.theme().muted_foreground
                                                        })
                                                        .flex_shrink_0()
                                                        .into_any_element()
                                                })
                                                .child(
                                                    v_flex()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .gap(px(2.))
                                                        .child(
                                                            div().truncate().text_sm().child(title),
                                                        )
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .text_xs()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .child(paths),
                                                        ),
                                                )
                                                .when(is_current, |row| {
                                                    row.child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .text_xs()
                                                            .text_color(accent)
                                                            .child(superfile_services::locale::tr(
                                                                "Current",
                                                            )),
                                                    )
                                                }),
                                        )
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.open_recent_here(open.clone(), window, cx)
                                        })),
                                )
                                .child(
                                    Button::new(("recent-new-window", index))
                                        .custom(
                                            ButtonCustomVariant::new(cx)
                                                .foreground(
                                                    cx.theme().muted_foreground.opacity(0.70),
                                                )
                                                .hover(cx.theme().foreground.opacity(0.04))
                                                .active(cx.theme().foreground.opacity(0.08)),
                                        )
                                        .small()
                                        .icon(Icon::new(IconName::ArrowUpRight).size(px(14.)))
                                        .tooltip(superfile_services::locale::tr(
                                            "Open in New Window",
                                        ))
                                        .accessibility_label(superfile_services::locale::tr(
                                            "Open in New Window",
                                        ))
                                        .on_click(move |_, _, cx| {
                                            crate::app::open_recent_workspace(
                                                new_window.clone(),
                                                cx,
                                            )
                                        }),
                                )
                                .child(
                                    Button::new(("recent-remove", index))
                                        .custom(
                                            ButtonCustomVariant::new(cx)
                                                .foreground(
                                                    cx.theme().muted_foreground.opacity(0.70),
                                                )
                                                .hover(cx.theme().foreground.opacity(0.04))
                                                .active(cx.theme().foreground.opacity(0.08)),
                                        )
                                        .small()
                                        .mr_2()
                                        .icon(Icon::new(IconName::X).size(px(14.)))
                                        .tooltip(superfile_services::locale::tr(
                                            "Remove from Recent Workspaces",
                                        ))
                                        .accessibility_label(superfile_services::locale::tr(
                                            "Remove from Recent Workspaces",
                                        ))
                                        .on_click(move |_, _, cx| {
                                            crate::app::remove_recent_workspace(
                                                remove_id.clone(),
                                                cx,
                                            )
                                        }),
                                )
                                .context_menu(move |menu, _, _| {
                                    let open = context_workspace.clone();
                                    let weak = context_weak.clone();
                                    let new_window = context_workspace.clone();
                                    let remove = context_workspace.id.clone();
                                    menu.item(
                                        PopupMenuItem::new(superfile_services::locale::tr(
                                            "Open in Current Window",
                                        ))
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = weak.update(cx, |view, cx| {
                                                    view.open_recent_here(open.clone(), window, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        PopupMenuItem::new(superfile_services::locale::tr(
                                            "Open in New Window",
                                        ))
                                        .on_click(
                                            move |_, _, cx| {
                                                crate::app::open_recent_workspace(
                                                    new_window.clone(),
                                                    cx,
                                                )
                                            },
                                        ),
                                    )
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(superfile_services::locale::tr(
                                            "Remove from Recent Workspaces",
                                        ))
                                        .on_click(
                                            move |_, _, cx| {
                                                crate::app::remove_recent_workspace(
                                                    remove.clone(),
                                                    cx,
                                                )
                                            },
                                        ),
                                    )
                                })
                        }),
                    ))
                    .when(more, |view| {
                        view.child(
                            Button::new("recent-more")
                                .ghost()
                                .icon(IconName::ChevronDown)
                                .label(superfile_services::locale::tr("Show more"))
                                .on_click(cx.listener(|view, _, _, cx| view.show_more_recent(cx))),
                        )
                    }),
            )
        });
    v_flex()
        .id("welcome-page")
        .size_full()
        .min_h_0()
        .overflow_y_scroll()
        .items_center()
        .px_6()
        .pt(px(24.))
        .pb_8()
        .when(center_content, |view| {
            view.pt_8().child(div().flex_1().flex_grow(0.85).min_h_0())
        })
        .child(content)
        .when(center_content, |view| {
            view.child(div().flex_1().flex_grow(1.15).min_h_0())
        })
}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        AppContext, Bounds, Pixels, Point, TestAppContext, WindowBounds, WindowOptions,
        component::{Theme, ThemeMode},
        px, size,
        test::TestWindowExt,
    };

    #[gpui_kit::test]
    fn welcome_actions_fit_both_themes_and_sidebar_layouts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(|cx| crate::app::remote::init(Default::default(), cx));
        for (mode, width, height) in [
            (ThemeMode::Light, 800., 480.),
            (ThemeMode::Light, 1100., 760.),
            (ThemeMode::Light, 1470., 855.),
            (ThemeMode::Dark, 800., 480.),
            (ThemeMode::Dark, 1100., 760.),
            (ThemeMode::Dark, 1470., 855.),
        ] {
            cx.update(|cx| Theme::change(mode, None, cx));
            let (handle, _) = cx.update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Point::default(),
                            size: size(px(width), px(height)),
                        })),
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        cx.new(|cx| super::super::workspace::Workspace::new(None, window, cx))
                    },
                )
                .unwrap()
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let mut previous: Option<Bounds<Pixels>> = None;
                for id in [
                    "welcome-new-file",
                    "welcome-new-workspace",
                    "welcome-open-file",
                ] {
                    let bounds = window.find(id).bounds();
                    assert!(bounds.size.width >= px(105.), "{id} too narrow at {width}");
                    assert!(bounds.origin.x >= px(0.));
                    assert!(bounds.origin.x + bounds.size.width <= px(width));
                    assert!(bounds.origin.y >= px(40.));
                    assert!(bounds.bottom() <= px(height));
                    if let Some(previous) = previous {
                        assert_eq!(bounds.origin.x, previous.origin.x);
                        assert!(
                            bounds.origin.y >= previous.bottom(),
                            "quick actions overlap at {width}"
                        );
                    }
                    previous = Some(bounds);
                }
                let workspace = window.find("welcome-new-workspace").bounds();
                let location = window.find("welcome-workspace-location").bounds();
                assert!(location.origin.x >= workspace.right());
                assert!(location.right() <= px(width));

                let expanded = window.find("welcome-open-file").bounds();
                window.click("toggle-sidebar", cx);
                let collapsed = window.find("welcome-open-file").bounds();
                assert!(collapsed.origin.x < expanded.origin.x);
                assert!(collapsed.size.width >= expanded.size.width);
                window.click("toggle-sidebar", cx);
                assert_eq!(window.find("welcome-open-file").bounds(), expanded);

                window.click("workspace-actions", cx);
                assert!(window.find("popup-menu").visible());
            })
            .unwrap();
        }
    }
}
