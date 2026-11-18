use super::*;
use std::io::Write;

enum Encoding {
    Raw,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    Lz4,
}

fn encoding(extension: &str) -> Option<Encoding> {
    Some(match extension {
        "gz" | "tgz" => Encoding::Gzip,
        "bz2" | "tbz" | "tbz2" => Encoding::Bzip2,
        "xz" | "txz" => Encoding::Xz,
        "zst" | "zstd" | "tzst" => Encoding::Zstd,
        "lz4" => Encoding::Lz4,
        "tar" => Encoding::Raw,
        _ => return None,
    })
}

pub(super) fn supports(extension: &str) -> bool {
    encoding(extension).is_some()
}

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
    let mut input: Box<dyn Read> = match encoding(&extension) {
        Some(Encoding::Gzip) => Box::new(flate2::read::MultiGzDecoder::new(input)),
        Some(Encoding::Bzip2) => Box::new(bzip2::read::MultiBzDecoder::new(input)),
        Some(Encoding::Xz) => Box::new(xz2::read::XzDecoder::new_multi_decoder(input)),
        Some(Encoding::Zstd) => Box::new(zstd::stream::read::Decoder::new(input)?),
        Some(Encoding::Lz4) => Box::new(lz4_flex::frame::FrameDecoder::new(input)),
        Some(Encoding::Raw) => Box::new(input),
        None => anyhow::bail!(
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
