//! Decode Apple binary property lists to private, read-only XML preview pages.
use crate::{paged::TextPage, work::Cancellation};
use anyhow::Result;
#[cfg(target_os = "macos")]
use anyhow::ensure;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
use std::{io::Read, path::Path};

pub fn is_binary(path: &Path) -> bool {
    if !superfile_core::formats::FormatSet::BinaryPlist.matches(path) {
        return false;
    }
    let mut header = [0; 8];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header == b"bplist00"
}
pub fn read(path: &Path, cancel: &Cancellation) -> Result<TextPage> {
    cancel.check()?;
    #[cfg(target_os = "macos")]
    {
        let output = tempfile::NamedTempFile::new()?;
        let status = crate::work::run(
            Command::new("/usr/bin/plutil")
                .args(["-convert", "xml1", "-o"])
                .arg(output.path())
                .arg("--")
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
            cancel,
        )?;
        ensure!(status.success(), "Cannot decode binary property list");
        cancel.check()?;
        crate::paged::generated(path, output)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        anyhow::bail!("Binary property list preview requires macOS plutil")
    }
}
