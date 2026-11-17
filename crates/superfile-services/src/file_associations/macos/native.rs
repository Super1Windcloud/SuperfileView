//! Public CoreServices queries. Copy/Create references are released in this module;
//! borrowed array elements and Spotlight results never escape their owning reference.
#![allow(unsafe_code)]
use crate::work::Cancellation;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{c_char, c_void},
    path::PathBuf,
    ptr,
};

type Ref = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFStringCreateWithBytes(
        allocator: Ref,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> Ref;
    fn CFStringGetLength(value: Ref) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(value: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFArrayGetCount(value: Ref) -> isize;
    fn CFArrayGetValueAtIndex(value: Ref, index: isize) -> Ref;
    fn CFURLCopyFileSystemPath(url: Ref, style: isize) -> Ref;
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn UTTypeCreateAllIdentifiersForTag(class: Ref, tag: Ref, conforming: Ref) -> Ref;
    fn UTTypeCopyAllTagsWithClass(identifier: Ref, class: Ref) -> Ref;
    fn UTTypeCopyDescription(identifier: Ref) -> Ref;
    fn LSCopyDefaultRoleHandlerForContentType(identifier: Ref, roles: u32) -> Ref;
    fn LSCopyApplicationURLsForBundleIdentifier(identifier: Ref, error: *mut Ref) -> Ref;
    fn MDQueryCreate(allocator: Ref, query: Ref, attributes: Ref, sorting: Ref) -> Ref;
    fn MDQueryExecute(query: Ref, flags: usize) -> u8;
    fn MDQueryGetResultCount(query: Ref) -> isize;
    fn MDQueryGetResultAtIndex(query: Ref, index: isize) -> Ref;
    fn MDItemCopyAttribute(item: Ref, name: Ref) -> Ref;
}

struct Owned(Ref);
impl Owned {
    fn new(value: Ref) -> Option<Self> {
        if value.is_null() {
            None
        } else {
            Some(Self(value))
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}

fn cf_string(text: &str) -> Option<Owned> {
    // UTF-8 bytes remain valid for the duration of this copying constructor.
    Owned::new(unsafe {
        CFStringCreateWithBytes(
            ptr::null(),
            text.as_ptr(),
            text.len() as isize,
            0x0800_0100,
            0,
        )
    })
}

fn string(value: Ref) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) != CFStringGetTypeID() } {
        return None;
    }
    let length =
        unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), 0x0800_0100) };
    let size = usize::try_from(length).ok()?.checked_add(1)?;
    if size > 1024 * 1024 {
        return None;
    }
    let mut bytes = vec![0u8; size];
    if unsafe { CFStringGetCString(value, bytes.as_mut_ptr().cast(), size as isize, 0x0800_0100) }
        == 0
    {
        return None;
    }
    let end = bytes.iter().position(|byte| *byte == 0)?;
    String::from_utf8(bytes[..end].to_vec()).ok()
}

fn strings(array: &Owned) -> Vec<String> {
    if unsafe { CFGetTypeID(array.0) != CFArrayGetTypeID() } {
        return Vec::new();
    }
    (0..unsafe { CFArrayGetCount(array.0) })
        .filter_map(|index| string(unsafe { CFArrayGetValueAtIndex(array.0, index) }))
        .collect()
}

pub(super) fn identifiers(extension: &str) -> Vec<String> {
    let Some(class) = cf_string("public.filename-extension") else {
        return Vec::new();
    };
    let Some(tag) = cf_string(extension) else {
        return Vec::new();
    };
    Owned::new(unsafe { UTTypeCreateAllIdentifiersForTag(class.0, tag.0, ptr::null()) })
        .map(|array| strings(&array))
        .unwrap_or_default()
}

pub(super) fn details(identifier: &str) -> (Option<String>, Vec<String>) {
    let Some(identifier) = cf_string(identifier) else {
        return (None, Vec::new());
    };
    let description = Owned::new(unsafe { UTTypeCopyDescription(identifier.0) })
        .and_then(|value| string(value.0));
    let extensions = cf_string("public.filename-extension")
        .and_then(|class| Owned::new(unsafe { UTTypeCopyAllTagsWithClass(identifier.0, class.0) }))
        .map(|array| strings(&array))
        .unwrap_or_default();
    (description, extensions)
}

pub(super) fn handler(identifier: &str) -> Option<String> {
    let identifier = cf_string(identifier)?;
    Owned::new(unsafe { LSCopyDefaultRoleHandlerForContentType(identifier.0, u32::MAX) })
        .and_then(|value| string(value.0))
}

pub(super) fn application_path(identifier: &str) -> Option<PathBuf> {
    let identifier = cf_string(identifier)?;
    let urls = Owned::new(unsafe {
        LSCopyApplicationURLsForBundleIdentifier(identifier.0, ptr::null_mut())
    })?;
    if unsafe { CFGetTypeID(urls.0) != CFArrayGetTypeID() || CFArrayGetCount(urls.0) == 0 } {
        return None;
    }
    let url = unsafe { CFArrayGetValueAtIndex(urls.0, 0) };
    Owned::new(unsafe { CFURLCopyFileSystemPath(url, 0) })
        .and_then(|path| string(path.0))
        .map(PathBuf::from)
}

pub(super) fn indexed_applications(cancel: &Cancellation) -> Result<Vec<PathBuf>> {
    let text = cf_string("kMDItemContentType == 'com.apple.application-bundle'")
        .context("Cannot create application index query")?;
    let query = Owned::new(unsafe { MDQueryCreate(ptr::null(), text.0, ptr::null(), ptr::null()) })
        .context("Cannot create application index query")?;
    cancel.check()?;
    ensure!(
        unsafe { MDQueryExecute(query.0, 1) } != 0,
        "The system application index is unavailable; discovery may be incomplete"
    );
    let path = cf_string("kMDItemPath").context("Cannot query application paths")?;
    let mut paths = Vec::new();
    for index in 0..unsafe { MDQueryGetResultCount(query.0) } {
        cancel.check()?;
        let item = unsafe { MDQueryGetResultAtIndex(query.0, index) };
        if item.is_null() {
            continue;
        }
        if let Some(value) = Owned::new(unsafe { MDItemCopyAttribute(item, path.0) })
            .and_then(|value| string(value.0))
        {
            paths.push(PathBuf::from(value));
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_native_values_never_construct_an_owned_null_reference() {
        assert!(Owned::new(ptr::null()).is_none());
        assert_eq!(string(ptr::null()), None);
        let value = cf_string("not an array").unwrap();
        assert!(strings(&value).is_empty());
        let (description, extensions) = details("app.superfile.unregistered-test-type");
        assert!(description.is_none());
        assert!(extensions.is_empty());
    }
}
