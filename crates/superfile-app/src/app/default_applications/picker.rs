use super::{Application, Associations, ChoicesLoader, application_name};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use superfile_services::{locale::tr, work::Cancellation};

pub(super) struct ApplicationPicker {
    identifier: String,
    current: Option<Application>,
    applications: Vec<Application>,
    loading: bool,
    error: Option<String>,
    _task: Task<()>,
    owner: WeakEntity<Associations>,
}

impl ApplicationPicker {
    pub(super) fn new(
        identifier: String,
        current: Option<Application>,
        loader: ChoicesLoader,
        owner: WeakEntity<Associations>,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = identifier.clone();
        let (cancel, guard) = Cancellation::task();
        let task = cx.spawn(async move |view, cx| {
            let _guard = guard;
            let result = cx
                .background_executor()
                .spawn(async move { loader(&query, &cancel) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(applications) => view.applications = applications,
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
        Self {
            identifier,
            current,
            applications: vec![],
            loading: true,
            error: None,
            _task: task,
            owner,
        }
    }
}

impl Render for ApplicationPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("default-application-options")
            .w(px(320.))
            .max_h(px(360.))
            .overflow_y_scroll()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .mb_1()
                    .child(tr("Choose the default application")),
            )
            .when(self.loading, |view| {
                view.child(
                    div()
                        .text_sm()
                        .child(tr("Reading compatible applications…")),
                )
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .when(
                !self.loading && self.error.is_none() && self.applications.is_empty(),
                |view| {
                    view.child(
                        div()
                            .text_sm()
                            .child(tr("No compatible applications found.")),
                    )
                },
            )
            .children(
                self.applications
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(index, application)| {
                        let selected = self
                            .current
                            .as_ref()
                            .is_some_and(|current| current.same_handler(&application));
                        let name = application_name(&application);
                        let details = format!(
                            "{}\n{}",
                            application.identifier.as_deref().unwrap_or_default(),
                            application
                                .path
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_default()
                        );
                        let identifier = self.identifier.clone();
                        let owner = self.owner.clone();
                        Button::new(("default-application-option", index))
                            .ghost()
                            .small()
                            .w_full()
                            .justify_start()
                            .accessibility_label(format!("{}: {name}", tr("Default application")))
                            .tooltip(details)
                            .disabled(self.loading)
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        div().flex_1().min_w_0().truncate().text_left().child(name),
                                    )
                                    .when(selected, |row| {
                                        row.child(Icon::new(IconName::Check).size(px(14.)))
                                    }),
                            )
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.apply_default(identifier.clone(), application.clone(), cx)
                                });
                            })
                    }),
            )
    }
}
