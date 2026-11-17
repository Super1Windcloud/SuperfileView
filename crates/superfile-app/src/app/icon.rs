//! Set the Dock icon even when Cargo starts an executable outside an app bundle.
#[allow(unsafe_code)]
pub(super) fn set() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(main_thread) = MainThreadMarker::new() else {
        log::warn!("Application icon must be set on the main thread");
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../../../../assets/macos/Superfile.icns"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        log::warn!("Cannot decode the embedded application icon");
        return;
    };
    // SAFETY: AppKit runs on the main thread; Some supplies the non-null image
    // required by this setter, and NSApplication retains it after this call.
    unsafe {
        NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image));
    }
}
