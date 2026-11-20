//! File-backed readers for archives with a member directory.
use super::*;
use crate::work::Reader;
use std::io::{self, Write};

pub(super) const DECODER_KIB: u32 = (crate::limits::TEXT_FULL_BYTES / 1024) as u32;

pub(super) fn supports(ext: &str) -> bool {
    matches!(
        ext,
        "7z" | "rar" | "cab" | "cpio" | "ar" | "a" | "lha" | "lzh" | "iso" | "xar"
    )
}

struct Members<'a> {
    stage: Option<&'a Path>,
    index: tempfile::NamedTempFile,
    cancel: &'a Cancellation,
}

impl Members<'_> {
    fn record(&mut self, name: &str, size: u64, directory: bool) -> Result<Option<File>> {
        self.cancel.check()?;
        let relative = safe(name)?;
        let Some(stage) = self.stage else {
            serde_json::to_writer(
                self.index.as_file_mut(),
                &Entry {
                    name: name.to_owned(),
                    size,
                    directory,
                },
            )?;
            self.index.write_all(b"\n")?;
            return Ok(None);
        };
        let target = stage.join(relative);
        if directory {
            fs::create_dir_all(target)?;
            return Ok(None);
        }
        fs::create_dir_all(target.parent().context("Archive entry has no parent")?)?;
        Ok(Some(File::create_new(target)?))
    }

    fn member(
        &mut self,
        name: &str,
        size: u64,
        directory: bool,
        input: &mut impl Read,
    ) -> Result<()> {
        if let Some(file) = self.record(name, size, directory)? {
            self.copy(input, file, size)?;
        }
        Ok(())
    }

    fn copy(&self, input: &mut (impl Read + ?Sized), file: File, size: u64) -> Result<()> {
        let count = self.cancel.copy(input.take(size.saturating_add(1)), file)?;
        ensure!(count == size, "Archive member length mismatch");
        Ok(())
    }
}

pub(super) fn read(
    path: &Path,
    ext: &str,
    stage: Option<&Path>,
    cancel: &Cancellation,
) -> Result<tempfile::NamedTempFile> {
    let mut entries = Members {
        stage,
        index: tempfile::NamedTempFile::new()?,
        cancel,
    };
    let input = Reader {
        inner: File::open(path)?,
        cancel: cancel.clone(),
    };
    match ext {
        "ar" | "a" => {
            let snapshot = crate::snapshot::MappedSnapshot::open(path, cancel)?;
            let data = snapshot.as_ref();
            let archive = object::read::archive::ArchiveFile::parse(data)?;
            ensure!(
                !archive.is_thin(),
                "Thin ar archives reference external files"
            );
            for entry in archive.members() {
                let entry = entry?;
                let name = std::str::from_utf8(entry.name())?;
                let mut bytes = entry.data(data)?;
                entries.member(name, entry.size(), false, &mut bytes)?;
            }
        }
        "cpio" => {
            let mut input = BufReader::new(input);
            loop {
                // newc names have a 32-bit declared length. Bound that one
                // allocation before passing the stream to the parser.
                let mut header = [0; 110];
                input.read_exact(&mut header)?;
                let name_size = u32::from_str_radix(std::str::from_utf8(&header[94..102])?, 16)?;
                ensure!(
                    u64::from(name_size) <= crate::limits::ARCHIVE_BYTES,
                    "CPIO name exceeds record budget"
                );
                input.seek_relative(-110)?;
                let mut reader = cpio::NewcReader::new(input)?;
                let entry = reader.entry();
                if entry.is_trailer() {
                    break;
                }
                let kind = entry.mode() & 0o170000;
                ensure!(
                    matches!(kind, 0 | 0o100000 | 0o040000),
                    "Unsupported CPIO special member"
                );
                ensure!(
                    kind == 0o040000 || entry.nlink() <= 1,
                    "CPIO hard links are unsupported"
                );
                let name = entry.name().to_owned();
                entries.member(
                    &name,
                    entry.file_size().into(),
                    kind == 0o040000,
                    &mut reader,
                )?;
                input = reader.skip()?;
            }
        }
        "cab" => {
            let mut cabinet = cab::Cabinet::new(input)?;
            let files: Vec<_> = cabinet
                .folder_entries()
                .flat_map(|folder| folder.file_entries())
                .map(|e| (e.name().to_owned(), u64::from(e.uncompressed_size())))
                .collect();
            for (name, size) in files {
                // CAB stores paths with Windows separators, including on Unix.
                if let Some(out) = entries.record(&name.replace('\\', "/"), size, false)? {
                    entries.copy(&mut cabinet.read_file(&name)?, out, size)?;
                }
            }
        }
        "lha" | "lzh" => {
            let mut reader = delharc::LhaDecodeReader::new(BufReader::new(input))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            while reader.is_present() {
                let header = reader.header();
                let name = header.parse_pathname_to_str();
                let mode = header.parse_unix_permissions().map_or(0, |m| m.bits()) & 0o170000;
                ensure!(
                    matches!(mode, 0 | 0o100000 | 0o040000),
                    "Unsupported LHA special member"
                );
                if let Some(file) =
                    entries.record(&name, header.original_size, header.is_directory())?
                {
                    ensure!(
                        reader.is_decoder_supported(),
                        "Unsupported LHA compression method"
                    );
                    let size = reader.header().original_size;
                    entries.copy(&mut reader, file, size)?;
                    reader.crc_check().map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                if !reader
                    .seek_next_file()
                    .map_err(|e| anyhow::anyhow!("{e}"))?
                {
                    break;
                }
            }
        }
        "iso" => iso(input, &mut entries)?,
        "7z" => {
            let mut seven = sevenz_rust2::ArchiveReader::new_with_memory_limit(
                input,
                sevenz_rust2::Password::empty(),
                DECODER_KIB as usize,
            )?;
            if stage.is_some() {
                seven.for_each_entries(|entry, input| {
                    let kind = (entry.windows_attributes >> 16) & 0o170000;
                    if entry.is_anti_item
                        || entry.windows_attributes & 0x400 != 0
                        || !matches!(kind, 0 | 0o100000 | 0o040000)
                    {
                        return Err(sevenz_rust2::Error::Unsupported("7z special member".into()));
                    }
                    let file = entries
                        .record(&entry.name, entry.size, entry.is_directory)
                        .map_err(io::Error::other)?;
                    if let Some(file) = file {
                        entries
                            .copy(input, file, entry.size)
                            .map_err(io::Error::other)?;
                    }
                    Ok(true)
                })?;
            } else {
                for e in &seven.archive().files {
                    entries.record(&e.name, e.size, e.is_directory)?;
                }
            }
        }
        "rar" => rar(input, &mut entries)?,
        "xar" => xar::read(input, &mut entries)?,
        _ => anyhow::bail!("Unsupported archive format {ext}"),
    }
    Ok(entries.index)
}

/// IsoImage retains descriptors and the path table while opening a volume.
/// Limit those reads independently of file content streamed after opening.
struct IsoInput<'a> {
    file: Reader<File>,
    catalog_bytes: &'a std::cell::Cell<Option<usize>>,
}

impl Read for IsoInput<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Some(remaining) = self.catalog_bytes.get() else {
            return self.file.read(out);
        };
        if remaining == 0 && !out.is_empty() {
            return Err(io::Error::other(
                "ISO volume catalog exceeds the working budget",
            ));
        }
        let count = out.len().min(remaining);
        let read = self.file.read(&mut out[..count])?;
        self.catalog_bytes.set(Some(remaining - read));
        Ok(read)
    }
}

impl io::Seek for IsoInput<'_> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

fn iso(input: Reader<File>, entries: &mut Members<'_>) -> Result<()> {
    let catalog_bytes = std::cell::Cell::new(Some(crate::limits::ARCHIVE_BYTES as usize));
    let image = hadris_iso::read::IsoImage::open(IsoInput {
        file: input,
        catalog_bytes: &catalog_bytes,
    })?;
    catalog_bytes.set(None);
    let root = image
        .root_dirs()
        .try_best_choice()
        .context("Missing ISO root")?;
    let joliet = matches!(
        root.entry_type(),
        hadris_iso::file::EntryType::Joliet { .. }
    );
    let mut pending = vec![(String::new(), root.dir_ref())];
    let mut visited = std::collections::HashSet::new();
    while let Some((prefix, dir)) = pending.pop() {
        ensure!(visited.insert(dir.extent.0), "Cyclic ISO directory");
        for entry in image.open_dir(dir).entries() {
            entries.cancel.check()?;
            let entry = entry?;
            if entry.is_special() {
                continue;
            }
            let rrip = entry.rrip.as_ref();
            ensure!(
                !rrip.is_some_and(|r| r.symlink_target.is_some()),
                "ISO symlinks are unsupported"
            );
            let name = if let Some(name) = rrip.and_then(|r| r.alternate_name.as_ref()) {
                name.clone()
            } else {
                let raw = if joliet {
                    ensure!(
                        entry.name().len().is_multiple_of(2),
                        "Invalid Joliet filename"
                    );
                    let units = entry
                        .name()
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|u| u16::from_be_bytes([u[0], u[1]]))
                        .collect::<Vec<_>>();
                    String::from_utf16(&units)?
                } else {
                    std::str::from_utf8(entry.name())?.to_owned()
                };
                match raw.rsplit_once(';') {
                    Some((base, version))
                        if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()) =>
                    {
                        base.to_owned()
                    }
                    _ => raw,
                }
            };
            ensure!(
                !name.contains('/') && !matches!(name.as_str(), "" | "." | ".."),
                "Invalid ISO member name"
            );
            let name = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let directory = entry.is_directory();
            let size = if directory { 0 } else { entry.total_size() };
            let output = entries.record(&name, size, directory)?;
            if directory {
                pending.push((name, entry.as_dir_ref(&image)?));
            } else if let Some(mut output) = output {
                let mut chunks =
                    image.read_file_chunked::<{ crate::limits::IO_BUFFER_BYTES }>(&entry)?;
                let mut copied = 0u64;
                while let Some(bytes) = chunks.next_chunk()? {
                    entries.cancel.check()?;
                    output.write_all(&bytes)?;
                    copied += bytes.len() as u64;
                }
                ensure!(copied == size, "Truncated ISO member");
            }
        }
    }
    Ok(())
}

struct CancelWriter(File, Cancellation);
impl Write for CancelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.1.check().map_err(io::Error::other)?;
        self.0.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn rar(input: Reader<File>, entries: &mut Members<'_>) -> Result<()> {
    let budget = u64::from(DECODER_KIB) * 1024;
    let options = rars::ArchiveReadOptions::new()
        .with_threads(1)
        .with_max_header_bytes(budget)
        .with_max_reader_workspace_bytes(budget)
        .with_rar50_dictionary_size_limit(budget)
        .with_rar50_buffered_decode_limit(crate::limits::ARCHIVE_BYTES);
    let archive = rars::ArchiveReader::read_reader_with_options(input, options)?;
    for member in archive.members() {
        entries.cancel.check()?;
        let meta = &member.meta;
        let name = String::from_utf8_lossy(&meta.name).replace('\\', "/");
        if entries.stage.is_some() {
            let kind = if meta.attr_source() == rars::AttrSource::Unix {
                meta.file_attr & 0o170000
            } else {
                0
            };
            ensure!(
                !meta.is_redirection && matches!(kind, 0 | 0o100000 | 0o040000),
                "Unsupported RAR special member"
            );
            ensure!(
                !meta.is_split_before && !meta.is_split_after,
                "RAR member spans multiple volumes"
            );
        }
        // The catalog pass preserves empty directories. Output handles for files
        // are opened by the sequential decoder, including for solid archives.
        if entries.stage.is_some() && !meta.is_directory {
            safe(&name)?;
        } else {
            entries.record(&name, meta.unpacked_size, meta.is_directory)?;
        }
    }
    if entries.stage.is_some() {
        let cancel = entries.cancel.clone();
        archive.extract_to_with_options(options, |meta| {
            if meta.is_directory {
                return Ok(Box::new(io::sink()));
            }
            let name = meta.name_lossy().replace('\\', "/");
            let file = entries
                .record(&name, 0, false)
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("Missing archive destination"))?;
            Ok(Box::new(CancelWriter(file, cancel.clone())))
        })?;
    }
    Ok(())
}

#[path = "archive_xar.rs"]
mod xar;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn ar_preserves_extended_gnu_and_bsd_member_names() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cancel = Cancellation::default();
        let name = "extended member name.txt";
        let contents = b"ar file";
        let header = ar::Header::new(name.as_bytes().to_vec(), contents.len() as u64);
        for (filename, gnu) in [("gnu.a", true), ("bsd.ar", false)] {
            let path = root.path().join(filename);
            let file = File::create(&path)?;
            if gnu {
                ar::GnuBuilder::new(file, vec![name.as_bytes().to_vec()])
                    .append(&header, &contents[..])?;
            } else {
                ar::Builder::new(file).append(&header, &contents[..])?;
            }
            let archive = super::super::read(&path, &cancel)?;
            assert_eq!(archive.entries[0].name, name);
            let output = super::super::extract(&path, root.path(), &cancel)?;
            assert_eq!(fs::read(output.join(name))?, contents);
        }
        Ok(())
    }

    #[test]
    fn extraction_failure_removes_the_unpublished_directory() -> Result<()> {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("sample.cpio");
        let mut out = File::create(&archive)?;
        for name in ["first.txt", "first.txt"] {
            let mut writer = cpio::newc::Builder::new(name).write(&mut out, 3);
            writer.write_all(b"one")?;
            writer.finish()?;
        }
        cpio::newc::trailer(&mut out)?;
        drop(out);
        assert!(super::super::extract(&archive, root.path(), &Cancellation::default()).is_err());
        assert_eq!(fs::read_dir(root.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn opens_and_extracts_portable_archive_variants() -> Result<()> {
        // Independently encoded examples of each container, all with the same
        // payload. The outer ZIP only keeps the mostly-empty ISO fixture small.
        let fixtures = include_bytes!("../../../assets/reader-contracts/portable-archives.zip");
        let mut fixtures = zip::ZipArchive::new(Cursor::new(fixtures))?;
        let root = tempfile::tempdir()?;
        let cancel = Cancellation::default();
        let payload = b"Hello Superfile!\n";
        for i in 0..fixtures.len() {
            let mut fixture = fixtures.by_index(i)?;
            let filename = fixture.name()?.into_owned();
            let path = root.path().join(&filename);
            io::copy(&mut fixture, &mut File::create(&path)?)?;
            let archive = super::super::read(&path, &cancel).with_context(|| filename.clone())?;
            assert_eq!(archive.entries.len(), 1, "{filename}");
            let file = &archive.entries[0];
            assert!(
                file.name.eq_ignore_ascii_case("hello.txt"),
                "{filename}: {}",
                file.name
            );
            assert_eq!(file.size, payload.len() as u64, "{filename}");
            let stage = super::super::extract(&path, root.path(), &cancel)
                .with_context(|| filename.clone())?;
            assert_eq!(fs::read(stage.join(&file.name))?, payload, "{filename}");
        }
        Ok(())
    }
}
