use anyhow::Result;
use gpui_kit::{App, Global};
use superfile_services::sharing::SharedFile;

#[cfg(target_os = "macos")]
#[path = "sharing/macos.rs"]
mod native;
#[cfg(windows)]
#[path = "sharing/windows.rs"]
mod native;
#[cfg(any(target_os = "macos", windows))]
pub(crate) use native::{Session, prepare, show};

#[cfg(target_os = "linux")]
pub(crate) async fn compose_email(file: SharedFile) -> Result<()> {
    use ashpd::desktop::{ResponseError, email::EmailRequest};
    let attachment = std::fs::File::open(file.path())?;
    let result = EmailRequest::default()
        .attach(attachment.into())
        .send()
        .await
        .and_then(|request| request.response());
    match result {
        Ok(()) => file.keep(),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[derive(Default)]
struct FileClipboard(Option<arboard::Clipboard>);
impl Global for FileClipboard {}

pub(crate) fn copy_file(file: SharedFile, cx: &mut App) -> Result<()> {
    let clipboard = &mut cx.default_global::<FileClipboard>().0;
    if clipboard.is_none() {
        *clipboard = Some(arboard::Clipboard::new()?);
    }
    clipboard
        .as_mut()
        .expect("initialized clipboard")
        .set()
        .file_list(&[file.path()])?;
    file.keep();
    Ok(())
}

pub(super) fn close_clipboard(cx: &mut App) {
    if cx.has_global::<FileClipboard>() {
        drop(cx.remove_global::<FileClipboard>());
    }
}
