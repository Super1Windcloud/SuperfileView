#![allow(unsafe_code)]
use anyhow::Result;
use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Mutex;
use superfile_services::sharing::SharedFile;
use windows::{
    ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager},
    Foundation::TypedEventHandler,
    Storage::{IStorageItem, StorageFile},
    Win32::{Foundation::HWND, UI::Shell::IDataTransferManagerInterop},
    core::{AgileReference, HSTRING, Interface, factory},
};
use windows_collections::IIterable;

pub(crate) struct Session {
    manager: DataTransferManager,
    token: i64,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.manager.RemoveDataRequested(self.token);
    }
}

pub(crate) async fn prepare(file: SharedFile) -> Result<(SharedFile, StorageFile)> {
    let item = StorageFile::GetFileFromPathAsync(&HSTRING::from(file.path().as_os_str()))?.await?;
    Ok((file, item))
}

pub(crate) fn show((file, storage): (SharedFile, StorageFile), window: &Window) -> Result<Session> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("The native window is unavailable: {error}"))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        anyhow::bail!("The window has no Win32 handle");
    };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    let interop: IDataTransferManagerInterop = factory::<DataTransferManager, _>()?;
    // The HWND is borrowed from the live GPUI window on its UI thread.
    let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd) }?;
    let title = storage.Name()?;
    let item: IStorageItem = storage.cast()?;
    let item = Mutex::new(Some(AgileReference::new(&item)?));
    let handler =
        TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(move |_, args| {
            let Some(item) = item.lock().ok().and_then(|mut item| item.take()) else {
                return Ok(());
            };
            let items: IIterable<IStorageItem> = vec![Some(item.resolve()?)].into();
            let data = args.ok()?.Request()?.Data()?;
            data.Properties()?.SetTitle(&title)?;
            data.SetStorageItemsReadOnly(&items)
        });
    let token = manager.DataRequested(&handler)?;
    let session = Session { manager, token };
    // Share UI is associated with the same HWND used to register the handler.
    unsafe { interop.ShowShareUIForWindow(hwnd) }?;
    file.keep();
    Ok(session)
}
