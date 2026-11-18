#![allow(unsafe_code)]
use anyhow::{Context, Result};
use gpui_kit::Window;
use objc2::{AnyThread, MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSSharingServicePicker, NSView};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{ffi::CString, os::unix::ffi::OsStrExt, ptr::NonNull};
use superfile_services::sharing::SharedFile;

pub(crate) struct Session {
    _picker: Retained<NSSharingServicePicker>,
}

pub(crate) async fn prepare(file: SharedFile) -> Result<SharedFile> {
    Ok(file)
}

pub(crate) fn show(file: SharedFile, window: &Window) -> Result<Session> {
    let _ = MainThreadMarker::new().context("System sharing requires the AppKit thread")?;
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("The native window is unavailable: {error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        anyhow::bail!("The window has no AppKit view");
    };
    let path = CString::new(file.path().as_os_str().as_bytes())?;
    let bytes = NonNull::from(&path.as_bytes_with_nul()[0]).cast();
    // The borrowed window handle owns a live NSView, and the file-system
    // representation remains null-terminated for the full NSURL call.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let url = unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(bytes, false, None)
    };
    let items = NSArray::<AnyObject>::from_slice(&[&url]);
    // NSURL implements AppKit's pasteboard-writing protocol.
    let picker =
        unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items) };
    let bounds = view.bounds();
    let x = bounds.origin.x + (bounds.size.width - 64.).max(0.);
    let y = if view.isFlipped() {
        8.
    } else {
        (bounds.size.height - 32.).max(0.)
    };
    let anchor = NSRect::new(NSPoint::new(x, bounds.origin.y + y), NSSize::new(24., 24.));
    picker.showRelativeToRect_ofView_preferredEdge(anchor, view, NSRectEdge::MinY);
    file.keep();
    Ok(Session { _picker: picker })
}
