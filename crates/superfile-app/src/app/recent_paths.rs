//! Serialize background history reads/writes, including the final shutdown flush.
use gpui_kit::*;
use std::path::PathBuf;
pub(crate) use superfile_services::recent_paths::{History, Kind};

#[derive(Default)]
pub(crate) struct RecentPaths(pub History);
impl Global for RecentPaths {}
#[derive(Default)]
struct Pending(Option<Task<()>>);
impl Global for Pending {}

pub(super) fn init(cx: &mut App) {
    cx.set_global(RecentPaths::default());
    cx.set_global(Pending::default());
    enqueue(None, cx);
}
pub(super) fn take_write(cx: &mut App) -> Option<Task<()>> {
    cx.global_mut::<Pending>().0.take()
}
pub(crate) fn remember(kind: Kind, path: PathBuf, cx: &mut App) {
    if cx.has_global::<Pending>() && path.is_absolute() {
        enqueue(Some((kind, path, true)), cx);
    }
}
pub(crate) fn remove(kind: Kind, path: PathBuf, cx: &mut App) {
    if cx.has_global::<Pending>() {
        enqueue(Some((kind, path, false)), cx);
    }
}
fn enqueue(change: Option<(Kind, PathBuf, bool)>, cx: &mut App) {
    let previous = cx.global_mut::<Pending>().0.take();
    let task = cx.spawn(async move |cx| {
        if let Some(previous) = previous {
            previous.await;
        }
        let result = cx
            .background_executor()
            .spawn(async move {
                use superfile_services::recent_paths;
                match change {
                    Some((kind, path, true)) => recent_paths::remember(kind, &path),
                    Some((kind, path, false)) => recent_paths::remove(kind, &path),
                    None => recent_paths::load(),
                }
            })
            .await;
        match result {
            Ok(history) => cx.update(|cx| {
                cx.global_mut::<RecentPaths>().0 = history;
                cx.refresh_windows();
            }),
            Err(error) => log::error!("Cannot update recent paths: {error:#}"),
        }
    });
    cx.global_mut::<Pending>().0 = Some(task);
}

pub(crate) fn open_in_new_window(path: PathBuf, cx: &mut App) {
    cx.defer(move |cx| super::open_workspace(Some(path), cx));
}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
        test::TestWindowExt,
    };

    #[gpui_kit::test]
    fn recent_file_and_folder_rows_open_and_preserve_the_previous_workspace(
        cx: &mut TestAppContext,
    ) {
        use crate::app::recent_paths::{History, RecentPaths};
        use crate::views::workspace_window::WorkspaceWindow;
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("中文 note.txt");
        std::fs::write(&file, "recent document").unwrap();
        let folder = directory.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            crate::app::remote::init(Default::default(), cx);
            cx.set_global(crate::app::OpenWorkspaces::default());
            // Rendering fixtures do not initialize the production persistence queue.
            cx.set_global(RecentPaths(History {
                files: vec![file.clone(); 6],
                directories: vec![folder.clone()],
            }));
        });
        let (handle, host) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1100.), px(900.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let host = cx.new(WorkspaceWindow::new);
                    host.update(cx, |host, cx| {
                        host.append(None, None, window, cx);
                    });
                    host
                },
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let row = window.find(("recent-files", 0_usize)).bounds();
            assert!(row.size.width > px(300.));
            assert!(row.right() <= px(1100.));
            window.click("recent-files-more", cx);
            assert!(window.find(("recent-files", 5_usize)).visible());
            window.click(("recent-files", 0_usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert!(crate::app::open_document_paths(cx).contains(&file)));
        cx.update_window(handle, |_, window, cx| {
            let original = host.read(cx).slots[0].view.clone();
            original.update(cx, |view, cx| view.show_home(cx));
            window.render_frame(cx);
            window.click(("recent-directories", 0_usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(host.read(cx).slots.len(), 2);
            let active = host.read(cx).active;
            assert_eq!(
                host.read(cx).slots[active]
                    .view
                    .read(cx)
                    .workspace_descriptor()
                    .folders,
                [folder]
            );
            assert!(crate::app::open_document_paths(cx).contains(&file));
        });
        assert_eq!(std::fs::read_to_string(file).unwrap(), "recent document");
    }
}
