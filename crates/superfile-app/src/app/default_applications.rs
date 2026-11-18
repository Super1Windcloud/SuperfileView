//! Searchable default applications with native choices and verified updates.
pub(crate) mod offer;
mod picker;
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, WindowExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        popover::Popover,
        tooltip::Tooltip,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use picker::ApplicationPicker;
use superfile_services::{
    file_associations::{self, Application, Association, ChangeResult, Snapshot},
    locale::tr,
    work::Cancellation,
};

type Loader = fn(&Cancellation) -> anyhow::Result<Snapshot>;
type ChoicesLoader = fn(&str, &Cancellation) -> anyhow::Result<Vec<Application>>;
type Setter = fn(&str, &Application) -> anyhow::Result<ChangeResult>;

#[derive(Default)]
struct PendingWrite(Option<Task<()>>);
impl Global for PendingWrite {}

pub(super) fn take_write(cx: &mut App) -> Option<Task<()>> {
    if cx.has_global::<PendingWrite>() {
        cx.global_mut::<PendingWrite>().0.take()
    } else {
        None
    }
}

fn retain_write(task: Task<()>, cx: &mut App) {
    if !cx.has_global::<PendingWrite>() {
        cx.set_global(PendingWrite::default());
    }
    cx.global_mut::<PendingWrite>().0 = Some(task);
}

fn application_name(application: &Application) -> String {
    if application.name.is_empty() {
        tr("Unknown application")
    } else {
        application.name.clone()
    }
}

pub(super) fn open(cx: &mut App) {
    open_with_loader(cx, file_associations::scan);
}

fn open_with_loader(cx: &mut App, loader: Loader) {
    // A button action still holds its window during dispatch.
    cx.defer(move |cx| {
        if let Some(handle) = cx.active_window().or_else(|| cx.windows().last().copied()) {
            let _ = handle.update(cx, |_, window, cx| {
                show(window, cx, loader);
            });
        }
    });
}

fn show(window: &mut Window, cx: &mut App, loader: Loader) -> Option<Entity<Associations>> {
    if window.has_active_dialog(cx) {
        return None;
    }
    let view = cx.new(|cx| Associations::new(window, cx, loader));
    let content = view.clone();
    let close = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let height = (window.viewport_size().height - px(190.)).clamp(px(200.), px(560.));
        let close = close.clone();
        dialog
            .title(tr("Default File Types…"))
            .width(px(980.))
            .child(div().h(height).child(content.clone()))
            .on_close(move |_, _, cx| close.update(cx, |view, _| view.cancel()))
    });
    Some(view)
}

struct Associations {
    search: Entity<InputState>,
    snapshot: Snapshot,
    visible: Vec<usize>,
    loading: bool,
    cancelled: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    loader: Loader,
    choices_loader: ChoicesLoader,
    setter: Setter,
    picker: Option<(String, Entity<ApplicationPicker>)>,
    applying: bool,
    change_error: Option<String>,
    status: Option<(bool, String)>,
    _subscription: Subscription,
}

impl Associations {
    fn new(window: &mut Window, cx: &mut Context<Self>, loader: Loader) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("Search file types or applications…"))
        });
        let subscription = cx.subscribe(&search, |view, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                view.filter(cx);
            }
        });
        let mut view = Self {
            search,
            snapshot: Snapshot::default(),
            visible: Vec::new(),
            loading: false,
            cancelled: false,
            error: None,
            task: None,
            scroll: UniformListScrollHandle::new(),
            loader,
            choices_loader: file_associations::applications,
            setter: file_associations::set_default,
            picker: None,
            applying: false,
            change_error: None,
            status: None,
            _subscription: subscription,
        };
        view.reload(cx);
        view
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        let query = self.search.read(cx).value();
        self.visible = self
            .snapshot
            .associations
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.matches(&query).then_some(index))
            .collect();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn cancel(&mut self) {
        self.picker = None;
        self.task = None;
        self.loading = false;
        self.cancelled = true;
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.applying {
            return;
        }
        self.picker = None;
        self.status = None;
        self.change_error = None;
        self.task = None;
        self.loading = true;
        self.cancelled = false;
        self.error = None;
        let loader = self.loader;
        let (cancel, guard) = Cancellation::task();
        self.task = Some(cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = cx
                .background_executor()
                .spawn(async move { loader(&cancel) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(snapshot) => view.snapshot = snapshot,
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                view.filter(cx);
            });
        }));
        cx.notify();
    }

    fn apply_default(
        &mut self,
        identifier: String,
        application: Application,
        cx: &mut Context<Self>,
    ) {
        if self.applying {
            return;
        }
        self.applying = true;
        self.picker = None;
        self.change_error = None;
        self.status = None;
        let previous = take_write(cx);
        let setter = self.setter;
        let task = cx.spawn(async move |view, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let query = identifier.clone();
            let name = application_name(&application);
            let result = cx
                .background_executor()
                .spawn(async move { setter(&query, &application) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.applying = false;
                match result {
                    Ok(ChangeResult::Applied(application)) => {
                        if let Some(entry) = view
                            .snapshot
                            .associations
                            .iter_mut()
                            .find(|entry| entry.identifier == identifier)
                        {
                            entry.application = Some(application);
                            entry.issue = None;
                        }
                        view.status = Some((false, format!("{identifier} → {name}")));
                        view.filter(cx);
                    }
                    Ok(ChangeResult::SystemSettingsRequired) => {
                        view.status = Some((true, format!("{identifier} → {name}")));
                        cx.open_url("ms-settings:defaultapps");
                    }
                    Err(error) => view.change_error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
        retain_write(task, cx);
        cx.notify();
    }

    fn row(&mut self, entry: Association, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let label = entry.label();
        let file_tooltip = format!("{}\n{}", label, entry.identifier);
        let (application, details) = if let Some(issue) = &entry.issue {
            (tr("Read failed"), issue.clone())
        } else if let Some(app) = &entry.application {
            let name = application_name(app);
            let details = format!(
                "{}\n{}",
                app.identifier.as_deref().unwrap_or_default(),
                app.path
                    .as_ref()
                    .map(|path| path.to_string_lossy())
                    .unwrap_or_default()
            );
            (name, details)
        } else {
            (
                tr("No default application found"),
                tr("No default application found"),
            )
        };
        let identifier = entry.identifier.clone();
        let current = entry.application.clone();
        let picker = self
            .picker
            .as_ref()
            .filter(|(id, _)| id == &identifier)
            .map(|(_, picker)| picker.clone());
        let is_open = picker.is_some();
        let dropdown = Popover::new(("default-application-picker", index))
            .open(is_open)
            .trigger(
                Button::new(("system-file-type-select", index))
                    .ghost()
                    .small()
                    .w_full()
                    .min_w_0()
                    .disabled(self.loading || self.applying)
                    .accessibility_label(format!(
                        "{}: {}",
                        tr("Choose default application"),
                        entry.label()
                    ))
                    .tooltip(details)
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_left()
                                    .child(application),
                            )
                            .child(
                                gpui_kit::component::Icon::new(
                                    gpui_kit::assets::IconName::ChevronDown,
                                )
                                .size(px(14.)),
                            ),
                    ),
            )
            .on_open_change(cx.listener(move |view, open, _, cx| {
                if *open && !view.applying {
                    let loader = view.choices_loader;
                    let identifier = identifier.clone();
                    let current = current.clone();
                    let owner = cx.entity().downgrade();
                    let picker_id = identifier.clone();
                    let picker =
                        cx.new(|cx| ApplicationPicker::new(identifier, current, loader, owner, cx));
                    view.picker = Some((picker_id, picker));
                } else {
                    view.picker = None;
                }
                cx.notify();
            }))
            .content(move |_, _, _| {
                div().when_some(picker.clone(), |view, picker| view.child(picker))
            });
        h_flex()
            .id(("system-file-type", index))
            .w_full()
            .h(px(54.))
            .gap_4()
            .px_2()
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.5))
            .child(
                div()
                    .id(("system-file-type-label", index))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(label)
                    .tooltip(move |window, cx| {
                        Tooltip::new(file_tooltip.clone()).build(window, cx)
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(entry.description.clone()),
            )
            .child(
                div()
                    .id(("system-file-type-app", index))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(if entry.issue.is_some() {
                        cx.theme().danger
                    } else {
                        cx.theme().foreground
                    })
                    .child(dropdown),
            )
            .into_any_element()
    }
}

impl Render for Associations {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::views::sync_input_placeholder(
            &self.search,
            tr("Search file types or applications…"),
            window,
            cx,
        );
        let loading = self.loading;
        let count = self.visible.len();
        let error = self.error.clone();
        let warnings = self.snapshot.warnings.join("\n");
        let heading = |label| {
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(tr(label))
        };
        v_flex().id("system-file-associations").size_full().gap_3()
            .child(h_flex().w_full().gap_2()
                .child(Input::new(&self.search).small().flex_1())
                .when(loading, |row| row.child(Button::new("cancel-associations-load").small().label(tr("Cancel"))
                    .on_click(cx.listener(|view, _, _, cx| { view.cancel(); cx.notify(); }))))
                .child(Button::new("refresh-associations").small().icon(gpui_kit::assets::IconName::RotateCw)
                    .tooltip(tr("Refresh")).accessibility_label(tr("Refresh")).disabled(loading || self.applying)
                    .on_click(cx.listener(|view, _, _, cx| view.reload(cx)))))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(tr("Registered system file types and their current default applications.")))
            .when(loading, |column| column.child(div().text_sm().child(tr("Reading system file types…"))))
            .when(self.cancelled, |column| column.child(div().text_sm().child(tr("Loading cancelled"))))
            .when(self.applying, |column| column.child(div().text_sm().child(tr("Updating default application…"))))
            .when_some(self.change_error.clone(), |column, error| column.child(v_flex().text_sm().text_color(cx.theme().danger)
                .child(tr("Could not change the default application.")).child(error)))
            .when_some(self.status.clone(), |column, (system, detail)| column.child(v_flex().text_xs().text_color(cx.theme().muted_foreground)
                .child(tr(if system { "Confirm the file type and application in Windows Settings, then refresh." } else { "Default application updated." })).child(detail)))
            .when_some(error, |column, error| column.child(v_flex().gap_1().text_sm().text_color(cx.theme().danger)
                .child(tr("Could not read system file associations.")).child(error)))
            .when(!warnings.is_empty(), |column| column.child(div().id("association-warnings").text_xs().text_color(cx.theme().warning)
                .child(tr("Some system information could not be read; the list may be incomplete."))
                .tooltip(move |window, cx| Tooltip::new(warnings.clone()).build(window, cx))))
            .child(h_flex().w_full().gap_4().px_2().pb_2().border_b_1().border_color(cx.theme().border)
                .child(heading("File format")).child(heading("Type description")).child(heading("Default application")))
            .when(count == 0 && !loading && self.error.is_none() && !self.cancelled, |column| column.child(div().py_4().text_sm().child(tr("No matching file types."))))
            .child(uniform_list("system-file-type-list", count, cx.processor(|view, range: std::ops::Range<usize>, _, cx| {
                range.map(|index| {
                    let entry = view.snapshot.associations[view.visible[index]].clone();
                    view.row(entry, index, cx)
                }).collect()
            })).w_full().flex_1().min_h_0().track_scroll(&self.scroll))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(format!("{} / {}", count, self.snapshot.associations.len())))
    }
}

#[cfg(test)]
mod tests {
    use super::{Association, Associations, ChangeResult, Snapshot, open_with_loader, show};
    use gpui_kit::{
        AppContext, TestAppContext, WindowOptions,
        component::{WindowExt, input::InputEvent},
        test::TestWindowExt,
    };
    use superfile_core::file_associations::Application;

    fn fixture(_: &superfile_services::work::Cancellation) -> anyhow::Result<Snapshot> {
        Ok(Snapshot {
            associations: (0..5000)
                .map(|index| Association {
                    identifier: format!("example/{index}"),
                    extensions: vec![format!("type{index}")],
                    description: "Test document".into(),
                    application: (index % 2 == 0).then(|| Application {
                        name: "Test Viewer".into(),
                        identifier: Some("test.viewer".into()),
                        path: None,
                    }),
                    issue: None,
                })
                .collect(),
            warnings: vec![],
        })
    }

    fn choices(
        _: &str,
        _: &superfile_services::work::Cancellation,
    ) -> anyhow::Result<Vec<Application>> {
        Ok(vec![
            Application {
                name: "Test Viewer".into(),
                identifier: Some("test.viewer".into()),
                path: None,
            },
            Application {
                name: "Alternate Viewer".into(),
                identifier: Some("test.alternate".into()),
                path: None,
            },
        ])
    }

    fn accept(_: &str, application: &Application) -> anyhow::Result<ChangeResult> {
        Ok(ChangeResult::Applied(application.clone()))
    }
    fn reject(_: &str, _: &Application) -> anyhow::Result<ChangeResult> {
        anyhow::bail!("Permission denied")
    }
    fn system(_: &str, _: &Application) -> anyhow::Result<ChangeResult> {
        Ok(ChangeResult::SystemSettingsRequired)
    }

    #[gpui_kit::test]
    fn dropdown_updates_only_after_confirmation_and_preserves_defaults_on_failure(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            // Hit testing must use settled dialog/popover geometry, regardless
            // of how far the entrance animation moves between pointer events.
            cx.set_reduce_motion(true);
        });
        let handle = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| gpui_kit::Empty)
            })
            .unwrap()
            .0
        });
        let view = cx
            .update_window(handle, |_, window, cx| show(window, cx, fixture).unwrap())
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.choices_loader = choices;
                view.setter = accept;
                view.search.update(cx, |input, cx| {
                    input.set_value("type0", window, cx);
                    cx.emit(InputEvent::Change);
                });
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find(("system-file-type-select", 0_usize))
                    .bounds()
                    .size
                    .width
                    > gpui_kit::px(100.)
            );
            window.click(("system-file-type-select", 0_usize), cx);
            assert!(
                view.read(cx).picker.is_some(),
                "Picker did not open; trigger={:?}",
                window.find(("system-file-type-select", 0_usize)).bounds()
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                view.read(cx).picker.is_some(),
                "Clicking the application must open its picker"
            );
            assert!(
                window
                    .find(("default-application-option", 0_usize))
                    .visible()
            );
            window.click(("default-application-option", 1_usize), cx);
            view.update(cx, |view, _| {
                assert!(view.applying);
                assert_eq!(
                    view.snapshot.associations[0]
                        .application
                        .as_ref()
                        .unwrap()
                        .name,
                    "Test Viewer"
                );
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(!view.applying);
                assert_eq!(
                    view.snapshot.associations[0]
                        .application
                        .as_ref()
                        .unwrap()
                        .name,
                    "Alternate Viewer"
                );
                view.setter = reject;
                view.apply_default(
                    "example/0".into(),
                    choices("", &Default::default()).unwrap()[0].clone(),
                    cx,
                );
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(
                    view.change_error
                        .as_deref()
                        .unwrap()
                        .contains("Permission denied")
                );
                assert_eq!(
                    view.snapshot.associations[0]
                        .application
                        .as_ref()
                        .unwrap()
                        .name,
                    "Alternate Viewer"
                );
                view.setter = system;
                view.apply_default(
                    "example/0".into(),
                    choices("", &Default::default()).unwrap()[0].clone(),
                    cx,
                );
            })
        });
        cx.run_until_parked();
        assert_eq!(cx.opened_url().as_deref(), Some("ms-settings:defaultapps"));
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                assert!(view.status.as_ref().unwrap().0);
                assert_eq!(
                    view.snapshot.associations[0]
                        .application
                        .as_ref()
                        .unwrap()
                        .name,
                    "Alternate Viewer"
                );
                view.reload(cx);
                assert!(view.status.is_none());
            })
        });
    }

    #[gpui_kit::test]
    fn window_action_opens_a_searchable_dialog_without_opening_system_settings(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| gpui_kit::Empty)
            })
            .unwrap()
            .0
        });
        cx.update(|cx| {
            cx.on_action(|_: &crate::app::actions::DefaultApplications, cx| {
                open_with_loader(cx, fixture)
            });
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(Box::new(crate::app::actions::DefaultApplications), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.opened_url(), None);
        cx.update_window(handle, |_, window, cx| {
            assert!(window.has_active_dialog(cx));
            assert!(show(window, cx, fixture).is_none());
            window.close_dialog(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let view = cx
            .update_window(handle, |_, window, cx| show(window, cx, fixture).unwrap())
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view: &mut Associations, cx| {
                assert_eq!(view.snapshot.associations.len(), 5000);
                view.search.update(cx, |input, cx| {
                    input.set_value("type42 viewer", window, cx);
                    cx.emit(InputEvent::Change);
                });
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                assert!(!view.visible.is_empty());
                assert!(
                    view.visible
                        .iter()
                        .all(|index| view.snapshot.associations[*index].matches("type42 viewer"))
                );
                view.reload(cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                assert_eq!(view.search.read(cx).value().as_str(), "type42 viewer");
                view.cancel();
                assert!(view.task.is_none());
            });
            window.close_dialog(cx);
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }
}
