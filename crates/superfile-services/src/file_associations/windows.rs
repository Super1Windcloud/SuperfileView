//! Read registry type names, then ask the Windows Shell for effective user defaults.
#![allow(unsafe_code)]
use super::*;
use ::windows::{
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{ASSOC_FILTER_NONE, SHAssocEnumHandlers},
    },
    core::{PCWSTR, PWSTR},
};
use anyhow::{Context, ensure};
use std::{collections::BTreeSet, path::PathBuf, ptr};
use windows_registry::{CLASSES_ROOT, CURRENT_USER};

fn owned_string(value: PWSTR) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let result = unsafe { value.to_string() }
        .ok()
        .filter(|value| !value.is_empty());
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    result
}

pub(super) fn applications(identifier: &str, cancel: &Cancellation) -> Result<Vec<Application>> {
    let _com = Com::new()?;
    let identifier = identifier.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let handlers = unsafe { SHAssocEnumHandlers(PCWSTR(identifier.as_ptr()), ASSOC_FILTER_NONE) }
        .map_err(|error| {
        anyhow::anyhow!("Cannot enumerate compatible applications: {error}")
    })?;
    let mut result = Vec::new();
    loop {
        cancel.check()?;
        let mut next = [None];
        let mut fetched = 0;
        unsafe { handlers.Next(&mut next, Some(&mut fetched)) }.map_err(|error| {
            anyhow::anyhow!("Cannot read the next compatible application: {error}")
        })?;
        let Some(handler) = next[0].take().filter(|_| fetched != 0) else {
            break;
        };
        let Some(id) = unsafe { handler.GetName() }.ok().and_then(owned_string) else {
            continue;
        };
        let name = unsafe { handler.GetUIName() }
            .ok()
            .and_then(owned_string)
            .unwrap_or_else(|| id.clone());
        let path = PathBuf::from(&id);
        result.push(Application {
            name,
            identifier: Some(id),
            path: path.is_absolute().then_some(path),
        });
    }
    Ok(result)
}

pub(super) fn set_default(_: &str, _: &Application) -> Result<ChangeResult> {
    // Windows 8+ reserves default changes for its user-facing system UI.
    Ok(ChangeResult::SystemSettingsRequired)
}

#[link(name = "Shlwapi")]
unsafe extern "system" {
    fn AssocQueryStringW(
        flags: u32,
        kind: u32,
        association: *const u16,
        extra: *const u16,
        output: *mut u16,
        length: *mut u32,
    ) -> i32;
}
#[link(name = "Ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut std::ffi::c_void, mode: u32) -> i32;
    fn CoUninitialize();
}

struct Com(bool);
impl Com {
    fn new() -> Result<Self> {
        let result = unsafe { CoInitializeEx(ptr::null_mut(), 0) };
        ensure!(
            result >= 0 || result as u32 == 0x8001_0106,
            "Cannot initialize Windows association queries: 0x{result:08X}"
        );
        Ok(Self(result >= 0))
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

fn query(extension: &str, kind: u32) -> std::result::Result<Option<String>, i32> {
    let extension = extension.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut length = 0;
    // The Shell returns S_FALSE and the required UTF-16 length for this size query.
    let result = unsafe {
        AssocQueryStringW(
            0,
            kind,
            extension.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            &mut length,
        )
    };
    if result as u32 == 0x8007_0483 || result as u32 == 0x8007_0002 {
        return Ok(None);
    }
    if result < 0 || length == 0 || length > 1024 * 1024 {
        return Err(result);
    }
    let mut buffer = vec![0u16; length as usize];
    let result = unsafe {
        AssocQueryStringW(
            0,
            kind,
            extension.as_ptr(),
            ptr::null(),
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if result < 0 {
        return Err(result);
    }
    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..end]);
    Ok((!value.is_empty()).then_some(value))
}

pub(super) fn scan(cancel: &Cancellation) -> Result<Snapshot> {
    let _com = Com::new()?;
    let mut snapshot = Snapshot::default();
    let mut extensions = BTreeSet::new();
    for key in CLASSES_ROOT
        .keys()
        .context("Cannot enumerate registered Windows file types")?
    {
        cancel.check()?;
        if key.starts_with('.') && key.len() > 1 {
            extensions.insert(key.to_lowercase());
        }
    }
    // Include per-user extensions even if no merged Classes entry currently exists.
    match CURRENT_USER.open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts") {
        Ok(key) => {
            for name in key.keys()? {
                cancel.check()?;
                if name.starts_with('.') && name.len() > 1 {
                    extensions.insert(name.to_lowercase());
                }
            }
        }
        Err(error) if error.code().0 as u32 == 0x8007_0002 => {}
        Err(error) => super::warning(
            &mut snapshot,
            format!("Cannot read user file types: {error}"),
        ),
    }
    for extension in extensions {
        cancel.check()?;
        let name = query(&extension, 4);
        let executable = query(&extension, 2);
        let identifier = query(&extension, 20).ok().flatten();
        let mut issue = None;
        let path = executable
            .as_ref()
            .ok()
            .and_then(|value| value.as_ref())
            .map(PathBuf::from);
        let application = match (name, path) {
            (Ok(Some(name)), path) => Some(Application {
                name,
                identifier,
                path,
            }),
            (_, Some(path)) => Some(Application {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                identifier,
                path: Some(path),
            }),
            (Err(error), None) => {
                issue = Some(format!("Windows association query failed: 0x{error:08X}"));
                None
            }
            (Ok(None), None) => {
                if let Err(error) = executable {
                    issue = Some(format!("Windows association query failed: 0x{error:08X}"));
                }
                None
            }
        };
        snapshot.associations.push(Association {
            identifier: extension.clone(),
            extensions: vec![extension.trim_start_matches('.').to_owned()],
            description: query(&extension, 3).ok().flatten().unwrap_or_default(),
            application,
            issue,
        });
    }
    Ok(snapshot)
}
