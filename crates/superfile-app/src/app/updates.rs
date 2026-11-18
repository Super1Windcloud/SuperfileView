//! Observable updates, workspace protection, and shell-free restart coordination.
#[cfg(test)]
mod tests;
use gpui_kit::{component::WindowExt, *};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use superfile_services::{
    locale::tr,
    updates::{self as service, CancelToken, Installed, PreparedUpdate, Release},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Cancelling,
    Ready,
    Preparing,
    Installing,
    Installed,
    Restarting,
    Failed,
}
struct Updates(Entity<State>);
impl Global for Updates {}
pub(crate) struct State {
    pub phase: Phase,
    pub release: Option<Release>,
    pub message: Option<String>,
    pub startup: bool,
    staged: Option<Arc<PreparedUpdate>>,
    installed: Option<Installed>,
    progress: Arc<AtomicU64>,
    cancel: Option<CancelToken>,
    task: Option<Task<()>>,
    progress_task: Option<Task<()>>,
    preference_write: Option<Task<()>>,
    cancel_install: bool,
    quit_after_cancel: bool,
    restart: Option<PathBuf>,
    queued_urls: Vec<String>,
    preferences_changed: bool,
    auto_download: bool,
    dialog: Option<WeakEntity<crate::views::software_update::SoftwareUpdate>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            release: None,
            message: None,
            startup: true,
            staged: None,
            installed: None,
            progress: Arc::default(),
            cancel: None,
            task: None,
            progress_task: None,
            preference_write: None,
            cancel_install: false,
            quit_after_cancel: false,
            restart: None,
            queued_urls: vec![],
            preferences_changed: false,
            auto_download: false,
            dialog: None,
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    let state = cx.new(|_| State::default());
    cx.set_global(Updates(state.clone()));
    let task = state.update(cx, |_, cx| {
        cx.spawn(async move |state, cx| {
            let startup = cx
                .background_executor()
                .spawn(async { service::startup_enabled() })
                .await;
            let _ = state.update(cx, |state, cx| {
                if !state.preferences_changed {
                    match startup {
                        Ok(enabled) => state.startup = enabled,
                        Err(error) => {
                            log::warn!("Cannot read startup update preference: {error:#}")
                        }
                    }
                }
                if state.startup && matches!(state.phase, Phase::Idle | Phase::Checking) {
                    state.start_check(true, cx);
                }
                cx.notify();
            });
        })
    });
    task.detach();
}

pub(crate) fn open(cx: &mut App) {
    let Some(model) = cx.try_global::<Updates>().map(|state| state.0.clone()) else {
        return;
    };
    model.update(cx, |state, cx| {
        if !matches!(
            state.phase,
            Phase::Ready | Phase::Installed | Phase::Restarting
        ) {
            state.check(cx);
        }
    });
    cx.defer(move |cx| {
        show_dialog(&model, true, cx);
    });
}

fn show_dialog(model: &Entity<State>, allow_inactive: bool, cx: &mut App) -> bool {
    if model
        .read(cx)
        .dialog
        .as_ref()
        .and_then(WeakEntity::upgrade)
        .is_some()
    {
        return true;
    }
    let handle = cx.active_window().or_else(|| {
        if allow_inactive {
            cx.windows().last().copied()
        } else {
            None
        }
    });
    handle.is_some_and(|handle| {
        handle
            .update(cx, |_, window, cx| {
                if window.has_active_dialog(cx) {
                    return false;
                }
                let view = cx.new(|cx| {
                    crate::views::software_update::SoftwareUpdate::new(model.clone(), cx)
                });
                model.update(cx, |state, _| state.dialog = Some(view.downgrade()));
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog
                        .title(tr("Software Update"))
                        .width(px(520.))
                        .child(view.clone())
                });
                true
            })
            .unwrap_or(false)
    })
}

// Wait for an available foreground window instead of covering another dialog.
async fn show_download_ready(view: WeakEntity<State>, cx: &mut AsyncApp) {
    loop {
        let done = cx.update(|cx| {
            let Some(model) = view.upgrade() else {
                return true;
            };
            model.read(cx).phase != Phase::Ready
                || cx
                    .try_global::<super::QuitSession>()
                    .is_some_and(|quit| quit.0)
                || show_dialog(&model, false, cx)
        });
        if done {
            return;
        }
        cx.background_executor().timer(Duration::from_secs(1)).await;
    }
}

pub(crate) fn busy(cx: &App) -> bool {
    cx.try_global::<Updates>()
        .is_some_and(|state| matches!(state.0.read(cx).phase, Phase::Preparing | Phase::Installing))
}
fn activity(cx: &App) -> superfile_core::update::UpdateActivity {
    let mut combined = superfile_core::update::UpdateActivity::default();
    for (view, _) in super::OpenWorkspaces::windows(cx) {
        if let Some(view) = view.upgrade() {
            let value = view.read(cx).update_activity();
            combined.dirty |= value.dirty;
            combined.saving |= value.saving;
            combined.loading |= value.loading;
        }
    }
    combined
}
fn lock_editors(locked: bool, cx: &mut App) {
    for (view, _) in super::OpenWorkspaces::windows(cx) {
        let _ = view.update(cx, |view, cx| view.set_editing_locked(locked, cx));
    }
    cx.refresh_windows();
}

impl State {
    pub fn label(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "Software Update",
            Phase::Checking => "Checking for updates…",
            Phase::Current => "Update check complete",
            Phase::Available => "An update is available",
            Phase::Downloading | Phase::Cancelling => "Downloading update…",
            Phase::Ready => "Update ready to install",
            Phase::Preparing => "Preparing update…",
            Phase::Installing => "Installing update…",
            Phase::Installed | Phase::Restarting => "Restart and Update",
            Phase::Failed => "Update could not complete",
        }
    }
    pub fn working(&self) -> bool {
        matches!(
            self.phase,
            Phase::Checking
                | Phase::Downloading
                | Phase::Cancelling
                | Phase::Preparing
                | Phase::Installing
                | Phase::Restarting
        )
    }
    pub fn received(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }
    pub fn can_install(&self) -> bool {
        matches!(self.phase, Phase::Ready | Phase::Installed)
            && (self.installed.is_some()
                || self
                    .staged
                    .as_ref()
                    .is_some_and(|update| update.install_blocker.is_none()))
    }
    pub fn download_path(&self) -> Option<&std::path::Path> {
        self.staged.as_ref().map(|update| update.artifact.as_path())
    }
    pub fn check(&mut self, cx: &mut Context<Self>) {
        self.start_check(false, cx);
    }
    fn start_check(&mut self, automatic: bool, cx: &mut Context<Self>) {
        if self.phase == Phase::Checking {
            self.auto_download |= automatic;
            return;
        }
        if self.working() || self.installed.is_some() {
            return;
        }
        if let Some(staged) = self.staged.take() {
            cx.background_executor()
                .spawn(async move {
                    drop(staged);
                })
                .detach();
        }
        self.phase = Phase::Checking;
        self.auto_download = automatic;
        self.release = None;
        self.message = None;
        self.task = Some(cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async { service::check() })
                .await;
            let _ = view.update(cx, |view, cx| view.finish_check(result, cx));
        }));
        cx.notify();
    }
    fn finish_check(&mut self, result: anyhow::Result<Option<Release>>, cx: &mut Context<Self>) {
        let automatic = std::mem::take(&mut self.auto_download) && self.startup;
        match result {
            Ok(Some(release)) => {
                self.release = Some(release);
                self.phase = Phase::Available;
                if automatic {
                    self.start_download(true, cx);
                }
            }
            Ok(None) => {
                self.phase = Phase::Current;
                self.message = Some("You’re up to date.".into());
            }
            Err(error) => {
                self.phase = Phase::Failed;
                self.message = Some(format!("{error:#}"));
            }
        }
        cx.notify();
    }
    pub fn download(&mut self, cx: &mut Context<Self>) {
        self.start_download(false, cx);
    }
    fn start_download(&mut self, automatic: bool, cx: &mut Context<Self>) {
        if self.phase != Phase::Available {
            return;
        }
        let Some(release) = self.release.clone() else {
            return;
        };
        self.phase = Phase::Downloading;
        self.message = None;
        self.progress.store(0, Ordering::Relaxed);
        let progress = self.progress.clone();
        let cancel = CancelToken::new();
        self.cancel = Some(cancel.clone());
        let allow_install = !cfg!(debug_assertions) && env!("SUPERFILE_BUILD_CHANNEL") == "release";
        self.task = Some(cx.spawn(async move |view, cx| {
            let cancelled = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    service::download(&release, allow_install, &cancel, move |received, _| {
                        progress.store(received, Ordering::Relaxed)
                    })
                })
                .await;
            let ready = view.update(cx, |view, cx| {
                view.progress_task = None;
                view.cancel = None;
                if cancelled.is_cancelled() {
                    if let Ok(staged) = result {
                        cx.background_executor()
                            .spawn(async move {
                                drop(staged);
                            })
                            .detach();
                    }
                    view.phase = Phase::Available;
                    view.message = Some("Update cancelled".into());
                } else {
                    match result {
                        Ok(staged) => {
                            view.message = staged.install_blocker.clone();
                            view.staged = Some(Arc::new(staged));
                            view.phase = Phase::Ready;
                        }
                        Err(error) => {
                            view.phase = Phase::Available;
                            view.message = Some(format!("{error:#}"));
                        }
                    }
                }
                cx.notify();
                view.phase == Phase::Ready
            });
            if automatic && matches!(ready, Ok(true)) {
                show_download_ready(view, cx).await;
            }
        }));
        self.progress_task = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if view
                    .update(cx, |view, cx| {
                        cx.notify();
                        matches!(view.phase, Phase::Downloading | Phase::Cancelling)
                    })
                    .ok()
                    != Some(true)
                {
                    break;
                }
            }
        }));
        cx.notify();
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
            self.phase = Phase::Cancelling;
        }
        if self.phase == Phase::Preparing {
            self.cancel_install = true;
        }
        cx.notify();
    }
    pub fn toggle_startup(&mut self, cx: &mut Context<Self>) {
        self.preferences_changed = true;
        self.startup = !self.startup;
        let value = self.startup;
        let previous = self.preference_write.take();
        self.preference_write = Some(cx.spawn(async move |view, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = cx
                .background_executor()
                .spawn(async move { service::save_startup(value) })
                .await;
            let _ = view.update(cx, |view, cx| {
                if let Err(error) = result {
                    view.message = Some(format!("{error:#}"));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Ready | Phase::Installed) {
            return;
        }
        if let Some(reason) = activity(cx).blocker() {
            self.message = Some(reason.into());
            cx.notify();
            return;
        }
        if let Some(reason) = self
            .staged
            .as_ref()
            .and_then(|staged| staged.install_blocker.clone())
        {
            self.message = Some(reason);
            cx.notify();
            return;
        }
        let staged = self.staged.clone();
        let installed = self.installed.clone();
        if staged.is_none() && installed.is_none() {
            return;
        }
        self.phase = Phase::Preparing;
        self.cancel_install = false;
        self.message = None;
        lock_editors(true, cx);
        let mut writes = Vec::new();
        for (view, _) in super::OpenWorkspaces::windows(cx) {
            let _ = view.update(cx, |view, cx| {
                view.flush_usage();
                view.remember_session(cx);
                writes.extend(view.take_reader_writes(cx));
                writes.extend(view.take_preference_write());
            });
        }
        super::workspace_sessions::checkpoint_windows(cx);
        {
            let sessions = cx.global_mut::<super::workspace_sessions::SavedWorkspaces>();
            writes.extend(sessions.write.take());
            writes.extend(sessions.window_write.take());
        }
        self.task = Some(cx.spawn(async move |view, cx| {
            for write in writes {
                write.await;
            }
            let permitted = view
                .update(cx, |view, cx| {
                    if view.cancel_install {
                        view.phase = if view.installed.is_some() {
                            Phase::Installed
                        } else {
                            Phase::Ready
                        };
                        lock_editors(false, cx);
                        view.finish_interruption(cx);
                        cx.notify();
                        false
                    } else {
                        view.phase = Phase::Installing;
                        cx.notify();
                        true
                    }
                })
                .unwrap_or(false);
            if !permitted {
                return;
            }
            let result = match installed {
                Some(installed) => Ok(installed),
                None => {
                    cx.background_executor()
                        .spawn(async move { service::install(&staged.unwrap()) })
                        .await
                }
            };
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(installed) => {
                        view.restart = Some(installed.executable.clone());
                        view.installed = Some(installed);
                        view.phase = Phase::Restarting;
                        // Workspace::is_saving includes the editing lock. Release
                        // it before normal quit approval, which still protects
                        // any draft created while the deferred quit is pending.
                        lock_editors(false, cx);
                        if !view.queued_urls.is_empty() {
                            view.restart = None;
                            view.phase = Phase::Installed;
                            view.finish_interruption(cx);
                        } else {
                            cx.defer(|cx| {
                                super::request_quit(cx);
                                if !cx.global::<super::QuitSession>().0 {
                                    quit_cancelled(cx);
                                }
                            });
                        }
                    }
                    Err(error) => {
                        view.phase = Phase::Ready;
                        view.message = Some(format!("{error:#}"));
                        lock_editors(false, cx);
                        view.finish_interruption(cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn finish_interruption(&mut self, cx: &mut Context<Self>) {
        let urls = std::mem::take(&mut self.queued_urls);
        if !urls.is_empty() {
            self.message =
                Some("New files were opened. Review them before restarting to update.".into());
            cx.defer(move |cx| super::open_system_files(urls, cx));
        }
        if self.quit_after_cancel {
            self.quit_after_cancel = false;
            cx.defer(super::request_quit);
        }
    }
}
pub(crate) fn interrupt_for_files(cx: &mut App) {
    if let Some(state) = cx.try_global::<Updates>().map(|state| state.0.clone()) {
        state.update(cx, |state, cx| {
            if state.phase == Phase::Preparing {
                state.cancel_install = true;
            }
            if state.phase == Phase::Restarting {
                state.phase = Phase::Installed;
                state.restart = None;
                lock_editors(false, cx);
            }
            cx.notify();
        });
    }
}
pub(crate) fn queue_files(urls: &[String], cx: &mut App) -> bool {
    if !busy(cx) {
        return false;
    }
    let state = cx.global::<Updates>().0.clone();
    state.update(cx, |state, _| state.queued_urls.extend_from_slice(urls));
    true
}
pub(crate) fn quit_cancelled(cx: &mut App) {
    if let Some(state) = cx.try_global::<Updates>().map(|state| state.0.clone()) {
        state.update(cx, |state, cx| {
            if state.phase == Phase::Restarting {
                state.phase = Phase::Installed;
                state.restart = None;
                cx.notify();
            }
        });
    }
}
pub(crate) fn intercept_last_window_close(cx: &mut App) -> bool {
    busy(cx)
}
pub(crate) fn prepare_quit(cx: &mut App) -> bool {
    if !busy(cx) {
        return false;
    }
    let state = cx.global::<Updates>().0.clone();
    state.update(cx, |state, cx| {
        if state.phase == Phase::Preparing {
            state.cancel_install = true;
            state.quit_after_cancel = true;
        }
        cx.notify();
    });
    true
}
pub(crate) fn shutdown(cx: &mut App) -> (Option<Task<()>>, Option<Task<()>>, Option<PathBuf>) {
    let Some(state) = cx.try_global::<Updates>().map(|state| state.0.clone()) else {
        return (None, None, None);
    };
    state.update(cx, |state, _| {
        if let Some(cancel) = &state.cancel {
            cancel.cancel();
        }
        let pending = if matches!(state.phase, Phase::Preparing | Phase::Installing) {
            state.task.take()
        } else {
            None
        };
        (pending, state.preference_write.take(), state.restart.take())
    })
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
    if !cfg!(debug_assertions)
        && env!("SUPERFILE_BUILD_CHANNEL") == "release"
        && let Some(handle) = cx.windows().first().copied()
    {
        let _ = handle.update(cx, |_, window, _| {
            // GPUI runs next-frame callbacks before drawing. Give the window
            // one frame to open before acknowledging this installation.
            window.on_next_frame(|window, _| {
                window.on_next_frame(|_, cx| {
                    cx.spawn(async |cx| {
                        for attempt in 0..6 {
                            let result = cx
                                .background_executor()
                                .spawn(async { service::cleanup_backups_after_startup() })
                                .await;
                            match result {
                                Ok(()) => return,
                                // The Windows process that launched us may
                                // still be exiting and holding the old image.
                                Err(_) if cfg!(windows) && attempt < 5 => {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(500))
                                        .await;
                                }
                                Err(error) => {
                                    log::warn!("Cannot clean up update backups: {error:#}");
                                    return;
                                }
                            }
                        }
                    })
                    .detach();
                });
            });
        });
    }
}
