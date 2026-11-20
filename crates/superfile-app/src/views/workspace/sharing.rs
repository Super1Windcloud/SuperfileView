use super::{Page, Workspace, header::Target};
use crate::app::sharing;
use anyhow::{Context as _, Result};
use gpui_kit::{
    component::{
        Disableable, Icon, Sizable, WindowExt,
        button::{Button, ButtonVariants},
        menu::{DropdownMenu, PopupMenuItem},
        notification::Notification,
    },
    *,
};
use std::path::Path;
use superfile_services::{locale::tr, work::Cancellation};

#[derive(Default)]
pub(super) struct FileSharing {
    task: Option<Task<()>>,
    #[cfg(any(target_os = "macos", windows))]
    session: Option<sharing::Session>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShareAction {
    System,
    Copy,
    SaveCopy,
}

fn menu_entries() -> &'static [(ShareAction, &'static str)] {
    &[
        #[cfg(any(target_os = "macos", windows))]
        (ShareAction::System, "System Share…"),
        #[cfg(target_os = "linux")]
        (ShareAction::System, "Share via Email…"),
        (ShareAction::Copy, "Copy File"),
        (ShareAction::SaveCopy, "Save a Copy…"),
    ]
}

fn copy_name(path: &Path) -> String {
    let mut name = path
        .file_stem()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    name.push(' ');
    name.push_str(&tr("copy"));
    if let Some(extension) = path.extension() {
        name.push('.');
        name.push_str(&extension.to_string_lossy());
    }
    name
}

impl Workspace {
    pub(super) fn share_file_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.page != Page::Document {
            return None;
        }
        let reader = self
            .active_reader()
            .filter(|reader| reader.can_share_file(cx))?;
        let target = Target::from_reader(reader);
        let view = cx.entity().downgrade();
        Some(
            Button::new("share-file")
                .ghost()
                .small()
                .w(px(28.))
                .h(px(28.))
                .flex_shrink_0()
                .icon(Icon::new(gpui_kit::assets::IconName::Share).size(px(15.)))
                .tooltip(tr("Share File"))
                .accessibility_label(tr("Share File"))
                .disabled(
                    self.picking
                        || self.file_sharing.task.is_some()
                        || crate::app::updates::busy(cx),
                )
                .dropdown_menu(move |mut menu, _, _| {
                    for &(action, label) in menu_entries() {
                        let view = view.clone();
                        let target = target.clone();
                        menu = menu.item(PopupMenuItem::new(tr(label)).on_click(
                            move |_, window, cx| {
                                let _ = view.update(cx, |workspace, cx| {
                                    workspace.share_file(&target, action, window, cx)
                                });
                            },
                        ));
                    }
                    menu
                })
                .into_any_element(),
        )
    }

    fn share_file(
        &mut self,
        target: &Target,
        action: ShareAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.picking || self.file_sharing.task.is_some() || crate::app::updates::busy(cx) {
            return;
        }
        let Some(reader) = self
            .tabs
            .iter()
            .find(|reader| Target::from_reader(reader) == *target && reader.can_share_file(cx))
        else {
            return;
        };
        let source = match reader.share_source(cx) {
            Ok(source) => source,
            Err(error) => {
                self.error = Some(format!("{}\n{error:#}", tr("Could not share the file")).into());
                cx.notify();
                return;
            }
        };
        let picker = if action == ShareAction::SaveCopy {
            let directory = source
                .directory
                .clone()
                .or_else(|| superfile_services::platform::home().ok())
                .unwrap_or_default();
            self.picking = true;
            Some(cx.prompt_for_new_path(&directory, Some(&copy_name(Path::new(&source.name)))))
        } else {
            None
        };
        let (cancel, guard) = Cancellation::task();
        self.file_sharing.task = Some(cx.spawn_in(window, async move |view, cx| {
            let _guard = guard;
            let result: Result<Option<&str>> = async {
                if let Some(picker) = picker {
                    let path = picker.await.context("Could not open the save dialog")??;
                    let Some(path) = path else { return Ok(None) };
                    let open = cx.update(|_, cx| crate::app::open_document_paths(cx))?;
                    cx.background_executor()
                        .spawn(async move { source.save_copy(&path, open, &cancel) })
                        .await?;
                    return Ok(Some("File copy saved"));
                }

                let file = cx
                    .background_executor()
                    .spawn(async move { source.prepare(&cancel) })
                    .await?;
                match action {
                    ShareAction::Copy => {
                        view.update_in(cx, |_, _, cx| sharing::copy_file(file, cx))??;
                        Ok(Some("File copied"))
                    }
                    #[cfg(any(target_os = "macos", windows))]
                    ShareAction::System => {
                        let file = sharing::prepare(file).await?;
                        view.update_in(cx, |view, window, _| -> Result<()> {
                            view.file_sharing.session = None;
                            view.file_sharing.session = Some(sharing::show(file, window)?);
                            Ok(())
                        })??;
                        Ok(None)
                    }
                    #[cfg(target_os = "linux")]
                    ShareAction::System => {
                        cx.background_executor()
                            .spawn(sharing::compose_email(file))
                            .await?;
                        Ok(None)
                    }
                    ShareAction::SaveCopy => unreachable!("save copy uses the file picker"),
                }
            }
            .await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.file_sharing.task = None;
                if action == ShareAction::SaveCopy {
                    view.picking = false;
                }
                match result {
                    Ok(Some(message)) => {
                        window.push_notification(Notification::success(tr(message)), cx)
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let title = if action == ShareAction::SaveCopy {
                            "Could not save the file copy"
                        } else {
                            "Could not share the file"
                        };
                        view.error = Some(format!("{}\n{error:#}", tr(title)).into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
