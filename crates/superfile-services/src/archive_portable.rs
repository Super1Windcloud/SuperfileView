use super::*;
use std::io::Write;

pub(super) fn native(
    path: &Path,
    destination: Option<&Path>,
    cancel: &Cancellation,
) -> Result<tempfile::NamedTempFile> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let input = File::open(path)?;
    let mut input: Box<dyn Read> = match extension.as_str() {
        "gz" | "tgz" => Box::new(flate2::read::MultiGzDecoder::new(input)),
        "bz2" | "tbz" | "tbz2" => Box::new(bzip2::read::MultiBzDecoder::new(input)),
        "xz" | "txz" => Box::new(xz2::read::XzDecoder::new_multi_decoder(input)),
        "zst" | "zstd" | "tzst" => Box::new(zstd::stream::read::Decoder::new(input)?),
        "lz4" => Box::new(lz4_flex::frame::FrameDecoder::new(input)),
        "tar" => Box::new(input),
        _ => anyhow::bail!(
            "This archive format requires an external archive application on this platform"
        ),
    };
    let mut index = tempfile::NamedTempFile::new()?;
    let tar = extension == "tar"
        || name.contains(".tar.")
        || matches!(extension.as_str(), "tgz" | "tbz" | "tbz2" | "txz" | "tzst");
    if tar {
        let reader = crate::work::Reader {
            inner: input,
            cancel: cancel.clone(),
        };
        let mut archive = tar::Archive::new(reader);
        for entry in archive.entries()? {
            cancel.check()?;
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            let relative = safe(&path.to_string_lossy())?;
            let kind = entry.header().entry_type();
            ensure!(
                kind.is_file() || kind.is_dir(),
                "Archive links and special entries are unsupported on this platform"
            );
            serde_json::to_writer(
                index.as_file_mut(),
                &serde_json::json!({"name":relative.to_string_lossy(),"size":entry.size(),"directory":kind.is_dir()}),
            )?;
            index.write_all(b"\n")?;
            if let Some(destination) = destination {
                ensure!(entry.unpack_in(destination)?, "Unsafe archive entry");
            }
        }
    } else {
        let entry_name = stem(path);
        let relative = safe(&entry_name)?;
        let size = if let Some(destination) = destination {
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination.join(relative))?;
            cancel.copy(&mut input, &mut output)?
        } else {
            cancel.copy(&mut input, std::io::sink())?
        };
        serde_json::to_writer(
            index.as_file_mut(),
            &serde_json::json!({"name":entry_name,"size":size,"directory":false}),
        )?;
        index.write_all(b"\n")?;
    }
    Ok(index)
}
