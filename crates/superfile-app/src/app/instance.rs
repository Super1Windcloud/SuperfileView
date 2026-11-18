use gpui_kit::{App, Global, Task};
use std::time::Duration;
use superfile_services::instance::{self, Instance};

struct SingleInstance {
    owner: Option<Instance>,
    listener: Option<Task<()>>,
}

impl Global for SingleInstance {}

pub(super) fn init(owner: Instance, cx: &mut App) {
    let directory = owner.directory().to_owned();
    let listener = cx.spawn(async move |cx| {
        loop {
            let directory = directory.clone();
            match cx
                .background_executor()
                .spawn(async move { instance::pending(&directory) })
                .await
            {
                Ok(mut requests) if !requests.is_empty() => {
                    cx.update(|cx| {
                        let urls = requests
                            .iter_mut()
                            .filter_map(|request| request.file_url.take())
                            .collect::<Vec<_>>();
                        if urls.len() < requests.len() {
                            activate(cx);
                        }
                        if !urls.is_empty() {
                            super::open_system_files(urls, cx);
                        }
                    });
                    cx.background_executor()
                        .spawn(async move {
                            for request in requests {
                                if let Err(error) = request.acknowledge() {
                                    log::warn!("Cannot acknowledge Superfile launch: {error:#}");
                                }
                            }
                        })
                        .await;
                }
                Ok(_) => {}
                Err(error) => log::warn!("Cannot read pending Superfile launches: {error:#}"),
            }
            // Check only the private launch directory, including while the app
            // is hidden or a filesystem coalesces rename notifications.
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
        }
    });
    cx.set_global(SingleInstance {
        owner: Some(owner),
        listener: Some(listener),
    });
}

pub(super) fn stop(cx: &mut App) -> Option<Instance> {
    let session = cx.global_mut::<SingleInstance>();
    drop(session.listener.take());
    session.owner.take()
}

pub(super) fn activate(cx: &mut App) {
    if let Some(window) = cx.active_window().or_else(|| cx.windows().last().copied()) {
        cx.activate(true);
        let _ = window.update(cx, |_, window, _| window.activate_window());
    } else if !super::updates::busy(cx) && !super::workspace_sessions::restore_windows(cx) {
        super::open_workspace(None, cx);
    }
}
