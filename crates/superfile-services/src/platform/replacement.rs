//! Metadata-preserving replacement of an existing local file.
#![allow(unsafe_code)]

use anyhow::{Context, Result, ensure};
use std::{fs::File, path::Path};

pub(crate) fn single_link(file: &File) -> Result<()> {
    #[cfg(unix)]
    let count = {
        use std::os::unix::fs::MetadataExt;
        file.metadata()?.nlink()
    };
    #[cfg(windows)]
    let count = winapi_util::file::information(file)?.number_of_links();
    ensure!(
        count <= 1,
        "{}",
        crate::locale::tr("This file has hard links. Save a copy to keep the linked files intact.")
    );
    Ok(())
}

pub(crate) fn copy_metadata(source: &File, output: &File) -> Result<()> {
    let permissions = source.metadata()?.permissions();
    ensure!(!permissions.readonly(), "File is read-only");
    single_link(source)?;
    output.set_permissions(permissions)?;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: both descriptors are live and a null state asks copyfile to manage its own state.
        let result = unsafe {
            libc::fcopyfile(
                source.as_raw_fd(),
                output.as_raw_fd(),
                std::ptr::null_mut(),
                libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error()).context("Cannot preserve file metadata");
        }
    }
    #[cfg(target_os = "linux")]
    {
        use xattr::FileExt;
        for name in source.list_xattr()? {
            let value = source
                .get_xattr(&name)?
                .context("File metadata changed while saving")?;
            output
                .set_xattr(&name, &value)
                .context("Cannot preserve file metadata")?;
        }
    }
    Ok(())
}

pub(crate) fn publish(temporary: tempfile::NamedTempFile, target: &Path) -> Result<()> {
    #[cfg(unix)]
    temporary
        .into_temp_path()
        .persist(target)
        .map_err(|e| e.error)?;
    #[cfg(windows)]
    {
        use std::{fs, os::windows::ffi::OsStrExt};
        use windows::{
            Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
            core::PCWSTR,
        };
        let path = temporary.into_temp_path();
        let parent = target.parent().context("File has no parent folder")?;
        let backup = tempfile::Builder::new()
            .prefix(".superfile-save-")
            .tempdir_in(parent)?;
        let original = backup
            .path()
            .join(target.file_name().context("File has no name")?);
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain([0])
                .collect::<Vec<_>>()
        };
        let (target_name, replacement_name, backup_name) =
            (wide(target), wide(&path), wide(&original));
        // SAFETY: the three paths are terminated wide strings retained for the call.
        let result = unsafe {
            ReplaceFileW(
                PCWSTR(target_name.as_ptr()),
                PCWSTR(replacement_name.as_ptr()),
                PCWSTR(backup_name.as_ptr()),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        };
        if let Err(error) = result {
            // Some ReplaceFileW errors leave the original under the backup name.
            // Bring it back without overwriting another process's new file.
            if original.exists() && fs::hard_link(&original, target).is_err() {
                let retained = backup.keep();
                anyhow::bail!(
                    "Cannot replace the file: {error}. Original file retained in {}",
                    retained.display()
                );
            }
            anyhow::bail!("Cannot replace the file: {error}");
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn exchange(left: &Path, right: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let left = CString::new(left.as_os_str().as_bytes())?;
    let right = CString::new(right.as_os_str().as_bytes())?;
    // SAFETY: the two terminated paths remain valid for the duration of the call.
    if unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_SWAP,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[test]
fn saved_text_preserves_extended_attributes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("note.txt");
    let value = b"preserve this metadata";
    std::fs::write(&path, "original")?;
    {
        let file = File::open(&path)?;
        #[cfg(target_os = "linux")]
        xattr::FileExt::set_xattr(&file, "user.superfile.test", value)?;
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: the file, attribute name and input bytes remain live for the call.
            let result = unsafe {
                libc::fsetxattr(
                    file.as_raw_fd(),
                    c"com.superfile.test".as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    crate::files::save_document(&path, "original", "edited")?;
    assert_eq!(std::fs::read_to_string(&path)?, "edited");
    let file = File::open(&path)?;
    #[cfg(target_os = "linux")]
    assert_eq!(
        xattr::FileExt::get_xattr(&file, "user.superfile.test")?.as_deref(),
        Some(value.as_slice())
    );
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        let mut actual = [0; 64];
        // SAFETY: the output points to an array of the passed size for the entire call.
        let count = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                c"com.superfile.test".as_ptr(),
                actual.as_mut_ptr().cast(),
                actual.len(),
                0,
                0,
            )
        };
        assert_eq!(count, value.len() as isize);
        assert_eq!(&actual[..value.len()], value);
    }
    Ok(())
}
