use super::*;
use std::io::Write;

enum Encoding {
    Raw,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    Lz4,
    Lzma,
    Lzip,
    Compress,
}

/// Stream each LZIP member with a bounded dictionary, even when the expanded
/// payload is larger than the decoder budget.
struct LzipInput {
    input: BufReader<crate::work::Reader<File>>,
    decoder: lzma_rust2::LzipStream,
}

impl Read for LzipInput {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while !self.decoder.is_finished() {
            let input = self.input.fill_buf()?;
            let action = if input.is_empty() {
                lzma_rust2::Action::Finish
            } else {
                lzma_rust2::Action::Run
            };
            let result = self.decoder.process(input, out, action)?;
            self.input.consume(result.bytes_consumed);
            if result.bytes_produced > 0 {
                return Ok(result.bytes_produced);
            }
            if result.bytes_consumed == 0 && !self.decoder.is_finished() {
                return Err(std::io::Error::other("LZIP decoder made no progress"));
            }
        }
        Ok(0)
    }
}

fn encoding(extension: &str) -> Option<Encoding> {
    Some(match extension {
        "gz" | "tgz" => Encoding::Gzip,
        "bz2" | "tbz" | "tbz2" => Encoding::Bzip2,
        "xz" | "txz" => Encoding::Xz,
        "zst" | "zstd" | "tzst" => Encoding::Zstd,
        "lz4" => Encoding::Lz4,
        "lzma" | "tlz" => Encoding::Lzma,
        "lz" => Encoding::Lzip,
        "z" | "taz" => Encoding::Compress,
        "tar" => Encoding::Raw,
        _ => return None,
    })
}

pub(super) fn supports(extension: &str) -> bool {
    encoding(extension).is_some() || members::supports(extension)
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
    if members::supports(&extension) {
        return members::read(path, &extension, destination, cancel);
    }
    let input = File::open(path)?;
    let mut input: Box<dyn Read> = match encoding(&extension) {
        Some(Encoding::Gzip) => Box::new(flate2::read::MultiGzDecoder::new(input)),
        Some(Encoding::Bzip2) => Box::new(bzip2::read::MultiBzDecoder::new(input)),
        Some(Encoding::Xz) => Box::new(xz2::read::XzDecoder::new_multi_decoder(input)),
        Some(Encoding::Zstd) => Box::new(zstd::stream::read::Decoder::new(input)?),
        Some(Encoding::Lz4) => Box::new(lz4_flex::frame::FrameDecoder::new(input)),
        Some(Encoding::Lzma) => Box::new(lzma_rust2::LzmaReader::new_mem_limit(
            input,
            members::DECODER_KIB,
            None,
        )?),
        Some(Encoding::Lzip) => Box::new(LzipInput {
            input: BufReader::new(crate::work::Reader {
                inner: input,
                cancel: cancel.clone(),
            }),
            decoder: lzma_rust2::LzipStream::new_mem_limit(true, members::DECODER_KIB),
        }),
        Some(Encoding::Compress) => Box::new(lzw_z::Decoder::new(input)),
        Some(Encoding::Raw) => Box::new(input),
        None => anyhow::bail!("Unsupported archive format {extension}"),
    };
    let mut index = tempfile::NamedTempFile::new()?;
    let tar = extension == "tar"
        || name
            .rsplit_once('.')
            .is_some_and(|(base, _)| base.ends_with(".tar"))
        || matches!(
            extension.as_str(),
            "tgz" | "tbz" | "tbz2" | "txz" | "tzst" | "tlz" | "taz"
        );
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
