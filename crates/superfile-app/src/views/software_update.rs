use crate::app::updates::{Phase, State};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable,
        button::{Button, ButtonVariants},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use superfile_services::locale::tr;

pub(crate) struct SoftwareUpdate {
    state: Entity<State>,
    _subscription: Subscription,
}
impl SoftwareUpdate {
    pub(crate) fn new(state: Entity<State>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _subscription: subscription,
        }
    }
}
impl Render for SoftwareUpdate {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let phase = state.phase;
        let title = state.label();
        let current = format!("Superfile {}", env!("CARGO_PKG_VERSION"));
        let release = state
            .release
            .as_ref()
            .map(|release| release.version.to_string());
        let notes = state
            .release
            .as_ref()
            .and_then(|release| release.notes.clone());
        let message = state.message.clone();
        let received = state.received();
        let expected = state
            .release
            .as_ref()
            .map_or(0, |release| release.asset.size);
        let startup = state.startup;
        let can_install = state.can_install();
        let artifact = state.download_path().map(std::path::Path::to_owned);
        let model = self.state.clone();
        let download = self.state.clone();
        let install = self.state.clone();
        let cancel = self.state.clone();
        let preferences = self.state.clone();
        v_flex()
            .id("software-update")
            .w_full()
            .gap_3()
            .child(div().text_lg().child(tr(title)))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(current),
            )
            .when_some(release, |view, version| {
                view.child(
                    div()
                        .text_sm()
                        .child(format!("{} {version}", tr("Available version"))),
                )
            })
            .when_some(notes.filter(|notes| !notes.is_empty()), |view, notes| {
                view.child(
                    div()
                        .id("update-release-notes")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .text_sm()
                        .child(notes),
                )
            })
            .when_some(message, |view, message| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(&message)),
                )
            })
            .when(
                matches!(phase, Phase::Downloading | Phase::Cancelling),
                |view| {
                    view.child(div().text_xs().child(format!(
                        "{:.1} / {:.1} MiB",
                        received as f64 / 1048576.,
                        expected as f64 / 1048576.
                    )))
                    .child(
                        div()
                            .w_full()
                            .h(px(4.))
                            .bg(cx.theme().muted)
                            .rounded_full()
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(if expected == 0 {
                                        0.
                                    } else {
                                        (received as f32 / expected as f32).clamp(0., 1.)
                                    }))
                                    .bg(cx.theme().primary),
                            ),
                    )
                },
            )
            .child(
                Button::new("update-check")
                    .label(tr("Check for Updates…"))
                    .disabled(state.working())
                    .on_click(move |_, _, cx| model.update(cx, |state, cx| state.check(cx))),
            )
            .when(phase == Phase::Available, |view| {
                view.child(
                    Button::new("update-download")
                        .primary()
                        .label(tr("Download Update"))
                        .on_click(move |_, _, cx| {
                            download.update(cx, |state, cx| state.download(cx))
                        }),
                )
            })
            .when(matches!(phase, Phase::Ready | Phase::Installed), |view| {
                view.child(
                    Button::new("update-install")
                        .primary()
                        .disabled(!can_install)
                        .label(tr("Restart and Update"))
                        .on_click(move |_, _, cx| {
                            install.update(cx, |state, cx| state.install(cx))
                        }),
                )
            })
            .when_some(artifact, |view, artifact| {
                view.child(
                    Button::new("update-show-download")
                        .label(tr("Show Downloaded Update"))
                        .on_click(move |_, _, cx| cx.reveal_path(&artifact)),
                )
            })
            .when(
                matches!(phase, Phase::Downloading | Phase::Preparing),
                |view| {
                    view.child(
                        Button::new("update-cancel")
                            .label(tr("Cancel This Update"))
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |state, cx| state.cancel(cx))
                            }),
                    )
                },
            )
            .child(
                Button::new("update-release-page")
                    .ghost()
                    .label(tr("View releases on GitHub"))
                    .on_click(|_, _, cx| cx.open_url(&superfile_services::updates::release_page())),
            )
            .child(
                Button::new("update-startup")
                    .ghost()
                    .label(tr("Check for updates on startup"))
                    .icon(if startup {
                        gpui_kit::assets::IconName::Check
                    } else {
                        gpui_kit::assets::IconName::Minus
                    })
                    .on_click(move |_, _, cx| {
                        preferences.update(cx, |state, cx| state.toggle_startup(cx))
                    }),
            )
    }
}
