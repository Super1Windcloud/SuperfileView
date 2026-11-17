use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Sizable,
        button::Button,
        h_flex,
        input::{Input, InputEvent, InputState},
        switch::Switch,
        tooltip::Tooltip,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::{collections::HashMap, path::PathBuf};
use superfile_services::{
    locale::tr,
    themes::{self, CommunityTheme, InstalledTheme, Scheme},
    work::Cancellation,
};

enum Preview {
    Pending,
    Ready(PathBuf),
    Unavailable,
}

pub(crate) struct CommunityThemes {
    search: Entity<InputState>,
    _search_subscription: Subscription,
    entries: Vec<CommunityTheme>,
    installed: HashMap<String, InstalledTheme>,
    only_installed: bool,
    loading: bool,
    cached: bool,
    error: Option<String>,
    load_task: Option<Task<()>>,
    installing: Option<String>,
    install_task: Option<Task<()>>,
    previews: HashMap<String, Preview>,
    preview_tasks: HashMap<String, Task<()>>,
    scroll: UniformListScrollHandle,
    columns: usize,
}

impl CommunityThemes {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search themes…")));
        let subscription = cx.subscribe(&search, |view, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                view.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
        });
        Self {
            search,
            _search_subscription: subscription,
            entries: Vec::new(),
            installed: HashMap::new(),
            only_installed: false,
            loading: false,
            cached: false,
            error: None,
            load_task: None,
            installing: None,
            install_task: None,
            previews: HashMap::new(),
            preview_tasks: HashMap::new(),
            scroll: UniformListScrollHandle::new(),
            columns: 1,
        }
    }

    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        let (cancel, guard) = Cancellation::task();
        self.load_task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let installed_cancel = cancel.clone();
            let installed = cx
                .background_executor()
                .spawn(async move { themes::installed(&installed_cancel) })
                .await;
            let _ = view.update(cx, |view, cx| {
                match installed {
                    Ok(installed) => {
                        view.installed = installed
                            .into_iter()
                            .map(|theme| (theme.entry.id(), theme))
                            .collect();
                        view.merge_installed();
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
            let catalog = cx
                .background_executor()
                .spawn(async move { themes::catalog(&cancel) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.loading = false;
                match catalog {
                    Ok(catalog) => {
                        view.entries = catalog.themes;
                        view.cached = catalog.cached;
                        view.merge_installed();
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn merge_installed(&mut self) {
        for package in self.installed.values() {
            if !self
                .entries
                .iter()
                .any(|entry| entry.id() == package.entry.id())
            {
                self.entries.push(package.entry.clone());
            }
        }
        self.entries
            .sort_by_cached_key(|entry| entry.name.to_lowercase());
    }

    fn filtered(&self, cx: &App) -> Vec<CommunityTheme> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                (!self.only_installed || self.installed.contains_key(&entry.id()))
                    && (query.is_empty()
                        || entry.name.to_lowercase().contains(&query)
                        || entry.author.to_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }

    fn request_preview(&mut self, entry: &CommunityTheme, cx: &mut Context<Self>) {
        let id = entry.id();
        if entry.screenshot.is_empty()
            || self.previews.contains_key(&id)
            || self.preview_tasks.len() >= 4
        {
            return;
        }
        self.previews.insert(id.clone(), Preview::Pending);
        let entry = entry.clone();
        let (cancel, guard) = Cancellation::task();
        let key = id.clone();
        let task = cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = cx
                .background_executor()
                .spawn(async move { themes::preview(&entry, &cancel) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.previews.insert(
                    key.clone(),
                    match result {
                        Ok(path) => Preview::Ready(path),
                        Err(error) => {
                            log::debug!("Theme screenshot unavailable: {error:#}");
                            Preview::Unavailable
                        }
                    },
                );
                view.preview_tasks.remove(&key);
                cx.notify();
            });
        });
        self.preview_tasks.insert(id, task);
    }

    fn activate(scheme: Scheme, entry: Option<&CommunityTheme>, cx: &mut App) {
        if let Some(entry) = entry {
            // Single-mode themes should be visible immediately when selected.
            if entry.modes == ["dark"] {
                crate::app::set_appearance(superfile_services::preferences::Appearance::Dark, cx);
            }
            if entry.modes == ["light"] {
                crate::app::set_appearance(superfile_services::preferences::Appearance::Light, cx);
            }
        }
        crate::app::theme::use_scheme(scheme, cx);
    }

    fn use_theme(&mut self, entry: CommunityTheme, cx: &mut Context<Self>) {
        if self.installing.is_some() || crate::app::updates::busy(cx) {
            return;
        }
        let id = entry.id();
        if let Some(package) = self.installed.get(&id) {
            Self::activate(package.scheme.clone(), Some(&package.entry), cx);
            cx.notify();
            return;
        }
        self.installing = Some(id);
        self.error = None;
        let (cancel, guard) = Cancellation::task();
        self.install_task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = cx
                .background_executor()
                .spawn(async move { themes::install(&entry, &cancel) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.installing = None;
                match result {
                    Ok(package) => {
                        Self::activate(package.scheme.clone(), Some(&package.entry), cx);
                        view.installed.insert(package.entry.id(), package);
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn palette_preview(scheme: &Scheme, cx: &App) -> AnyElement {
        let palette = scheme.palette(if cx.theme().is_dark() {
            themes::Mode::Dark
        } else {
            themes::Mode::Light
        });
        let color = |name| {
            let [r, g, b, a] = palette.get(name).0;
            Hsla::from(Rgba { r, g, b, a })
        };
        h_flex()
            .size_full()
            .overflow_hidden()
            .rounded(px(4.))
            .bg(color("--background-primary"))
            .child(
                v_flex()
                    .w_1_3()
                    .h_full()
                    .p_3()
                    .gap_2()
                    .bg(color("--background-secondary"))
                    .child(
                        div()
                            .w_full()
                            .h(px(5.))
                            .rounded_full()
                            .bg(color("--interactive-accent")),
                    )
                    .children((0..4).map(|_| {
                        div()
                            .w_full()
                            .h(px(4.))
                            .rounded_full()
                            .bg(color("--text-faint").opacity(0.4))
                    })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .p_4()
                    .gap_3()
                    .child(
                        div()
                            .w_2_3()
                            .h(px(8.))
                            .rounded_full()
                            .bg(color("--text-normal")),
                    )
                    .children((0..3).map(|_| {
                        div()
                            .w_full()
                            .h(px(4.))
                            .rounded_full()
                            .bg(color("--text-muted").opacity(0.5))
                    })),
            )
            .into_any_element()
    }

    fn card(&mut self, entry: Option<CommunityTheme>, cx: &mut Context<Self>) -> AnyElement {
        let id = entry.as_ref().map(CommunityTheme::id);
        let active = crate::app::theme::selected_id(cx) == id.as_deref();
        let installed = id.as_ref().and_then(|id| self.installed.get(id));
        let scheme = installed
            .map(|package| package.scheme.clone())
            .unwrap_or_else(themes::default_scheme);
        let name = entry
            .as_ref()
            .map_or_else(|| tr("Default theme"), |entry| entry.name.clone());
        let author = entry
            .as_ref()
            .map_or("Obsidian", |entry| entry.author.as_str())
            .to_owned();
        let modes = entry.as_ref().map_or_else(
            || format!("{} · {}", tr("Light"), tr("Dark")),
            |entry| {
                entry
                    .modes
                    .iter()
                    .filter_map(|mode| match mode.as_str() {
                        "light" => Some(tr("Light")),
                        "dark" => Some(tr("Dark")),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            },
        );
        let is_installed = entry.is_none() || installed.is_some();
        let pending = id
            .as_ref()
            .is_some_and(|id| self.installing.as_ref() == Some(id));
        let diagnostics = installed.map(|package| {
            let palette = package.scheme.palette(if cx.theme().is_dark() {
                themes::Mode::Dark
            } else {
                themes::Mode::Light
            });
            format!(
                "{}: {} · {}: {}",
                tr("Mapped colors"),
                palette.applied,
                tr("Fallback colors"),
                palette.unresolved.len()
            )
        });
        if let Some(entry) = entry.as_ref() {
            self.request_preview(entry, cx);
        }
        let preview = match id.as_ref().and_then(|id| self.previews.get(id)) {
            Some(Preview::Ready(path)) => img(path.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            _ if is_installed => Self::palette_preview(&scheme, cx),
            _ => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(cx.theme().muted)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr("Preview unavailable"))
                .into_any_element(),
        };
        let button = Button::new("theme-use")
            .small()
            .label(tr(if active {
                "Current theme"
            } else if is_installed {
                "Use"
            } else {
                "Download and use"
            }))
            .accessibility_label(format!(
                "{}: {name}",
                tr(if is_installed {
                    "Use"
                } else {
                    "Download and use"
                })
            ))
            .loading(pending)
            .disabled(active || self.installing.is_some() || crate::app::updates::busy(cx))
            .on_click(cx.listener(move |view, _, _, cx| {
                if let Some(entry) = entry.clone() {
                    view.use_theme(entry, cx);
                } else {
                    Self::activate(themes::default_scheme(), None, cx);
                    cx.notify();
                }
            }));
        v_flex()
            .id(SharedString::from(
                id.unwrap_or_else(|| "default-theme".into()),
            ))
            .flex_1()
            .min_w_0()
            .h(px(268.))
            .p_3()
            .gap_2()
            .border_1()
            .border_color(if active {
                super::accent(cx)
            } else {
                cx.theme().border
            })
            .rounded(px(6.))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(name),
            )
            .child(
                h_flex()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .gap_2()
                    .child(div().flex_1().truncate().child(author))
                    .child(modes),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .rounded(px(4.))
                    .child(preview),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_between()
                    .child(button)
                    .when_some(diagnostics, |row, detail| {
                        row.child(
                            div()
                                .id("theme-compatibility")
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("Color variables"))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(detail.clone()).build(window, cx)
                                }),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for CommunityThemes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.filtered(cx);
        let count = entries.len();
        let columns = self.columns;
        let rows = (count + 1).div_ceil(columns);
        let view = cx.entity().downgrade();
        let error = self
            .error
            .clone()
            .or_else(|| crate::app::theme::load_error(cx).map(str::to_owned));
        v_flex().size_full().min_w_0().min_h_0().p_6().gap_4()
            .child(h_flex().gap_2().child(super::icon_button("themes-back", IconName::ArrowLeft, tr("Settings"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::app::actions::OpenSettings), cx)))
                .child(div().text_lg().child(tr("Community themes"))))
            .child(h_flex().gap_4().w_full().flex_wrap()
                .child(div().w(px(300.)).max_w_full().child(Input::new(&self.search).small()))
                .child(Switch::new("themes-installed").checked(self.only_installed).label(tr("Installed only"))
                    .on_click(cx.listener(|view, checked, _, cx| { view.only_installed = *checked; view.scroll.scroll_to_item(0, ScrollStrategy::Top); cx.notify(); })))
                .child(div().flex_1())
                .child(super::icon_button("themes-refresh", IconName::RotateCw, tr("Refresh")).loading(self.loading).disabled(self.loading)
                    .on_click(cx.listener(|view, _, _, cx| view.reload(cx)))))
            .child(h_flex().w_full().gap_3().text_xs().text_color(cx.theme().muted_foreground)
                .child(format!("{}: {}", tr("Themes"), count + 1))
                .when(self.loading, |row| row.child(tr("Loading…")))
                .when(self.cached, |row| row.child(tr("Offline · cached directory")))
                .when(self.installing.is_some(), |row| row.child(Button::new("cancel-theme-download").small().label(tr("Cancel download"))
                    .on_click(cx.listener(|view, _, _, cx| { view.install_task = None; view.installing = None; cx.notify(); })))))
            .when_some(error, |column, error| column.child(div().id("themes-error").text_xs().text_color(cx.theme().danger).child(error)))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(tr("Applies Obsidian color variables. Layout and plugin styles are not applied; unsupported colors use the default palette.")))
            .child(canvas(move |bounds, window, cx| {
                let columns = ((f32::from(bounds.size.width) + 12.) / 240.).floor().clamp(1., 4.) as usize;
                let view = view.clone();
                window.defer(cx, move |_, cx| { let _ = view.update(cx, |view, cx| {
                    if view.columns != columns { view.columns = columns; cx.notify(); }
                }); });
            }, |_, _, _, _| {}).w_full().h(px(0.)))
            .child(uniform_list("community-theme-grid", rows, cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                range.map(|row| {
                    let mut element = h_flex().w_full().h(px(280.)).gap_3().pb_3();
                    for column in 0..columns {
                        let index = row * columns + column;
                        if index == 0 { element = element.child(view.card(None, cx)); }
                        else if let Some(entry) = entries.get(index - 1) { element = element.child(view.card(Some(entry.clone()), cx)); }
                        else { element = element.child(div().flex_1().min_w_0()); }
                    }
                    element
                }).collect()
            })).w_full().flex_1().min_h_0().track_scroll(&self.scroll))
    }
}

#[cfg(test)]
mod tests {
    use super::{CommunityTheme, CommunityThemes, InstalledTheme, themes};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };

    #[gpui_kit::test]
    fn search_and_installed_filter_preserve_available_packages_without_network(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(560.), px(600.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| CommunityThemes::new(window, cx)),
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                let entry = |name: &str, author: &str, repo: &str| CommunityTheme {
                    name: name.into(),
                    author: author.into(),
                    repo: repo.into(),
                    screenshot: String::new(),
                    modes: vec![],
                    branch: None,
                    legacy: false,
                };
                let minimal = entry("Minimal", "kepano", "kepano/minimal");
                let things = entry("Things", "colin", "colin/things");
                view.entries = vec![minimal.clone(), things.clone()];
                view.installed.insert(
                    minimal.id(),
                    InstalledTheme {
                        entry: minimal.clone(),
                        scheme: themes::default_scheme(),
                    },
                );
                assert_eq!(view.filtered(cx).len(), 2);
                view.search
                    .update(cx, |search, cx| search.set_value("  KEPANO  ", window, cx));
                assert_eq!(view.filtered(cx), vec![minimal.clone()]);
                view.search
                    .update(cx, |search, cx| search.set_value("Things", window, cx));
                assert_eq!(view.filtered(cx), vec![things]);
                view.only_installed = true;
                assert!(view.filtered(cx).is_empty());
                view.search
                    .update(cx, |search, cx| search.set_value("", window, cx));
                assert_eq!(view.filtered(cx), vec![minimal]);
            });
            window.render_frame(cx);
            let button = window.within("default-theme").find("theme-use").bounds();
            assert!(button.origin.x >= px(0.) && button.right() <= px(560.));
            view.update(cx, |view, _| assert!(view.preview_tasks.is_empty()));
        })
        .unwrap();
    }
}
