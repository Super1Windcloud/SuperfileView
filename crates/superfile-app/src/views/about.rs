use gpui_kit::{
    component::{
        ActiveTheme, Icon, Sizable,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    *,
};

const GITHUB_URL: &str = "https://github.com/Super1Windcloud/SuperfileView";

pub(crate) struct About {
    focus: FocusHandle,
}

impl About {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.on_release(|_, _| superfile_services::telemetry::track("about_closed", []))
            .detach();
        Self { focus }
    }
}

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("about-page")
            .size_full()
            .track_focus(&self.focus)
            .items_center()
            .justify_center()
            .p_6()
            .gap_3()
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(img("branding/superfile.png").size(px(64.)))
                    .child(
                        div()
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Superfile"),
                    ),
            )
            .child(env!("CARGO_PKG_VERSION"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{}: {}",
                        superfile_services::locale::tr("Author"),
                        env!("CARGO_PKG_AUTHORS")
                    )),
            )
            .child(
                Button::new("about-github")
                    .ghost()
                    .small()
                    .size(px(28.))
                    .icon(Icon::empty().path("navigation/github.svg").size(px(20.)))
                    .tooltip(format!(
                        "{} · {GITHUB_URL}",
                        superfile_services::locale::tr("View project on GitHub")
                    ))
                    .accessibility_label(superfile_services::locale::tr("View project on GitHub"))
                    .on_click(|_, _, cx| cx.open_url(GITHUB_URL)),
            )
    }
}
