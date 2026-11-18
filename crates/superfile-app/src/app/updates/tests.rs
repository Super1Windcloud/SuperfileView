use super::{
    CancelToken, Phase, State, Updates, busy, interrupt_for_files, prepare_quit, queue_files,
};
use gpui_kit::{
    AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, component::WindowExt, px,
    size, test::TestWindowExt,
};

#[gpui_kit::test]
fn startup_check_starts_a_download_while_a_manual_check_waits_for_confirmation(
    cx: &mut TestAppContext,
) {
    use superfile_services::updates::{Asset, Release, Version};

    let release = Release::new(
        Version::parse("99.0.0").unwrap(),
        Asset::new("test.zip", "https://example.invalid/test.zip", 123),
    );
    cx.update(|cx| {
        for (automatic, enabled, expected) in [
            (true, true, Phase::Downloading),
            (true, false, Phase::Available),
            (false, true, Phase::Available),
        ] {
            let model = cx.new(|_| State {
                phase: Phase::Checking,
                startup: enabled,
                ..Default::default()
            });
            model.update(cx, |state, cx| {
                // Merge a startup request into a check already running.
                state.start_check(automatic, cx);
                state.finish_check(Ok(Some(release.clone())), cx);
                assert_eq!(state.phase, expected);
                // Stop before the simulated result would perform any network I/O.
                state.task.take();
                state.progress_task.take();
            });
        }
    });
}

#[gpui_kit::test]
fn downloaded_update_waits_for_existing_dialog_and_can_be_deferred(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let model = cx.new(|_| State {
        phase: Phase::Ready,
        ..Default::default()
    });
    let handle = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(650.), px(650.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| gpui_kit::Empty),
        )
        .unwrap()
        .0
    });
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.open_dialog(cx, |dialog, _, _| dialog.title("Existing dialog"));
    })
    .unwrap();
    let prompt = model.update(cx, |_, cx| cx.spawn(super::show_download_ready));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        assert!(model.read(cx).dialog.is_none());
        window.close_dialog(cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        window.render_frame(cx);
        window.click("update-later", cx);
        assert!(!window.has_active_dialog(cx));
        assert_eq!(model.read(cx).phase, Phase::Ready);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!window.has_active_dialog(cx))
    })
    .unwrap();
    drop(prompt);
}

#[gpui_kit::test]
fn prepared_restart_can_be_cancelled_and_unlocks_workspaces_before_quit(cx: &mut TestAppContext) {
    let model = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::app::remote::init(Default::default(), cx);
        cx.set_global(crate::app::OpenWorkspaces::default());
        cx.set_global(crate::app::QuitSession::default());
        cx.set_global(crate::app::workspace_sessions::SavedWorkspaces {
            sessions: Default::default(),
            windows: Default::default(),
            write: None,
            window_write: None,
            restoring: false,
        });
        let state = cx.new(|_| State {
            phase: Phase::Installed,
            // Simulate an already completed swap. No executable is replaced or launched.
            installed: Some(super::Installed {
                executable: "/fixture/superfile".into(),
                backup: "/fixture/backup".into(),
            }),
            ..Default::default()
        });
        cx.set_global(Updates(state.clone()));
        state
    });
    let (handle, workspace) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| crate::views::workspace::Workspace::new(None, window, cx))
        })
        .unwrap()
    });
    cx.update(|cx| {
        cx.global_mut::<crate::app::OpenWorkspaces>()
            .0
            .push((workspace.downgrade(), handle));
        model.update(cx, |state, cx| state.install(cx));
        assert_eq!(model.read(cx).phase, Phase::Preparing);
        assert!(workspace.read(cx).is_saving());
        model.update(cx, |state, cx| state.cancel(cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(model.read(cx).phase, Phase::Installed);
        assert!(!workspace.read(cx).is_saving());
        assert!(!cx.global::<crate::app::QuitSession>().0);
        model.update(cx, |state, cx| state.install(cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(model.read(cx).phase, Phase::Restarting);
        assert!(!workspace.read(cx).is_saving());
        assert!(cx.global::<crate::app::QuitSession>().0);
        crate::app::cancel_quit(cx);
        assert_eq!(model.read(cx).phase, Phase::Installed);
        assert!(model.read(cx).restart.is_none());
    });
}

#[gpui_kit::test]
fn quit_and_incoming_files_cancel_preparation_without_losing_open_requests(
    cx: &mut TestAppContext,
) {
    let model = cx.update(|cx| {
        let model = cx.new(|_| State {
            phase: Phase::Preparing,
            ..Default::default()
        });
        cx.set_global(Updates(model.clone()));
        cx.set_global(super::super::QuitSession::default());
        model
    });
    cx.update(|cx| {
        super::super::request_quit(cx);
        assert!(model.read(cx).cancel_install);
        assert!(model.read(cx).quit_after_cancel);
        assert!(!cx.global::<super::super::QuitSession>().0);
        interrupt_for_files(cx);
        let urls = vec!["file:///new-document.md".to_owned()];
        assert!(queue_files(&urls, cx));
        assert_eq!(model.read(cx).queued_urls, urls);
        model.update(cx, |state, _| state.phase = Phase::Installing);
        assert!(prepare_quit(cx));
        assert!(busy(cx));
        model.update(cx, |state, _| state.phase = Phase::Ready);
        assert!(!busy(cx));
        assert!(!queue_files(&urls, cx));
    });
}

#[gpui_kit::test]
fn update_dialog_cancels_download_and_prevents_install_without_verified_payload(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let token = CancelToken::new();
    let model = cx.new(|_| State {
        phase: Phase::Downloading,
        cancel: Some(token.clone()),
        ..Default::default()
    });
    let (handle, _) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(650.), px(650.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|cx| crate::views::software_update::SoftwareUpdate::new(model.clone(), cx))
            },
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("update-cancel").visible());
        window.click("update-cancel", cx);
        assert_eq!(model.read(cx).phase, Phase::Cancelling);
        assert!(token.is_cancelled());
        model.update(cx, |state, cx| {
            state.phase = Phase::Ready;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("update-install", cx);
        assert_eq!(model.read(cx).phase, Phase::Ready);
        assert!(!model.read(cx).can_install());
    })
    .unwrap();
}
