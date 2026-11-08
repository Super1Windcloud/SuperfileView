//! Host filesystem and desktop operations. SFTP paths use separate protocol encoding.
use anyhow::{Context, Result};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

pub fn home() -> Result<PathBuf> {
    dirs::home_dir().context("Home directory is unavailable")
}
pub fn data_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SUPERFILE_DATA_DIR") {
        let path = PathBuf::from(path);
        anyhow::ensure!(path.is_absolute(), "SUPERFILE_DATA_DIR must be absolute");
        return Ok(path);
    }
    Ok(dirs::data_local_dir()
        .context("Application data directory is unavailable")?
        .join("Superfile"))
}
pub fn cache_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SUPERFILE_CACHE_DIR") {
        let path = PathBuf::from(path);
        anyhow::ensure!(path.is_absolute(), "SUPERFILE_CACHE_DIR must be absolute");
        return Ok(path);
    }
    Ok(dirs::cache_dir()
        .context("Cache directory is unavailable")?
        .join("Superfile"))
}
pub fn file_identity(file: &File) -> std::io::Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        let information = winapi_util::file::information(file)?;
        Ok((information.volume_serial_number(), information.file_index()))
    }
}
pub fn restrict_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    // Windows inherits the user's application-data ACL, rather than Unix mode bits.
    #[cfg(windows)]
    let _ = (path, mode);
    Ok(())
}
pub trait PrivateDirExt {
    fn private_mode(&mut self, mode: u32) -> &mut Self;
}
impl PrivateDirExt for std::fs::DirBuilder {
    fn private_mode(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            self.mode(mode);
        }
        #[cfg(windows)]
        let _ = mode;
        self
    }
}
pub trait PrivateFileExt {
    fn private_mode(&mut self, mode: u32) -> &mut Self;
}
impl PrivateFileExt for std::fs::OpenOptions {
    fn private_mode(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            self.mode(mode);
        }
        #[cfg(windows)]
        let _ = mode;
        self
    }
}
pub fn open(path: impl AsRef<std::ffi::OsStr>) -> Result<()> {
    open::that(path)?;
    Ok(())
}
