use gpui_kit::*;

pub fn open(cx: &mut App) {
    super::open_workspace_tab(cx, |view, window, cx| view.show_about(window, cx));
}

#[cfg(test)]
mod tests {
    use gpui_kit::{AppContext, TestAppContext, component::WindowExt, test::TestWindowExt};

    #[gpui_kit::test]
    fn about_opens_without_workspace_and_reopens_after_close(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            crate::app::remote::init(Default::default(), cx);
            cx.set_global(super::super::OpenWorkspaces::default());
            super::super::show_about(cx);
        });
        let first = cx.update(|cx| {
            assert_eq!(cx.windows().len(), 1);
            cx.windows()[0]
        });
        cx.update_window(first, |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            assert!(
                window
                    .within("document-tabs")
                    .find(("close-tab", 0usize))
                    .visible()
            );
            assert!(window.within("about-page").find("about-github").visible());
            window
                .within("document-tabs")
                .click(("close-tab", 0usize), cx);
            window.render_frame(cx);
            assert!(window.try_find("about-github").is_none());
        })
        .unwrap();
        cx.update(super::super::show_about);
        cx.update_window(first, |_, window, cx| {
            assert_eq!(cx.windows(), vec![first]);
            window.render_frame(cx);
            assert!(window.within("about-page").find("about-github").visible());
        })
        .unwrap();
        cx.update_window(first, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        cx.update(super::super::show_about);
        cx.update(|cx| {
            assert_eq!(cx.windows().len(), 1);
            assert_ne!(cx.windows()[0], first);
        });
    }
}
