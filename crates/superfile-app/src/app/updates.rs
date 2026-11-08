//! Startup/session coordination. Superfile currently ships without an automatic updater.
use gpui_kit::*;
use std::path::PathBuf;
pub(crate) fn init(_cx: &mut App) {}
pub(crate) fn busy(_cx: &App) -> bool {
    false
}
pub(crate) fn interrupt_for_files(_cx: &mut App) {}
pub(crate) fn intercept_last_window_close(_cx: &mut App) -> bool {
    false
}
pub(crate) fn prepare_quit(_cx: &mut App) -> bool {
    false
}
pub(crate) fn start_application(path: Option<PathBuf>, urls: Vec<String>, cx: &mut App) {
    cx.global_mut::<super::workspace_sessions::SavedWorkspaces>()
        .restoring = true;
    let restored = super::workspace_sessions::restore_windows(cx);
    cx.global_mut::<super::workspace_sessions::SavedWorkspaces>()
        .restoring = false;
    if !urls.is_empty() {
        super::open_system_files(urls, cx);
    } else if let Some(path) = path {
        super::open_workspace(Some(path), cx);
    } else if !restored && cx.windows().is_empty() {
        super::open_workspace(None, cx);
    }
}
