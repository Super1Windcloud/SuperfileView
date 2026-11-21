//! Convert a fixed-layout document in the background, then hand the private PDF
//! to the same virtualized, theme-aware reader used for ordinary PDFs.
use super::{Cancellation, PreparedDocument};
use crate::{paged, resources, snapshot::MappedSnapshot};
use anyhow::{Context, Result, ensure};
use std::{
    io::{BufReader, Cursor, Read, Write},
    path::Path,
};
use tempfile::{NamedTempFile, TempPath};

pub(super) fn temp_pdf(bytes: &[u8], cancel: &Cancellation) -> Result<TempPath> {
    ensure!(
        bytes.starts_with(b"%PDF-"),
        "Renderer produced an invalid PDF"
    );
    let mut file = tempfile::Builder::new().suffix(".pdf").tempfile()?;
    cancel.copy(Cursor::new(bytes), file.as_file_mut())?;
    file.flush()?;
    Ok(file.into_temp_path())
}

pub(crate) fn zip_size(bytes: &[u8], cancel: &Cancellation) -> Result<u64> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        cancel.check()?;
        total = total
            .checked_add(archive.by_index_raw(index)?.size())
            .context("Document size overflow")?;
    }
    Ok(total)
}

// The importer shares subtrees between child order and mixed-content order.
// Its DOM cost grows with node/attribute count, while embedded PNGs expand
// into pixel buffers during PDF export. Recursive tree traversal also needs
// an independent nesting limit.
const MAX_LAYOUT_XML_DEPTH: u32 = 128;
pub(super) fn inspect_layout(bytes: &[u8], cancel: &Cancellation) -> Result<(u64, bool)> {
    use quick_xml::events::Event;
    let mut references_images = false;
    let mut budget = zip_size(bytes, cancel)?
        .saturating_add(bytes.len() as u64)
        .saturating_mul(2);
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    for index in 0..archive.len() {
        cancel.check()?;
        let entry = archive.by_index(index)?;
        let expected = entry.size();
        crate::limits::preview_budget(
            expected <= crate::limits::RESOURCES_WORKING_BYTES,
            "Office part exceeds layout memory",
        )?;
        crate::limits::preview_budget(
            budget <= crate::limits::RESOURCES_WORKING_BYTES,
            "Office layout exceeds working memory",
        )?;
        let xml = entry.name_raw().ends_with(b".xml") || entry.name_raw().ends_with(b".rels");
        let body = matches!(entry.name_raw(), b"word/document.xml" | b"content.xml");
        let image = entry.name_raw().starts_with(b"word/media/")
            || entry.name_raw().starts_with(b"ppt/media/")
            || entry.name_raw().starts_with(b"Pictures/");
        let mut bounded = entry.take(expected + 1);
        if xml {
            let mut reader = quick_xml::Reader::from_reader(BufReader::new(&mut bounded));
            let mut event_bytes = Vec::new();
            let mut depth = 0u32;
            loop {
                cancel.check()?;
                let start = reader.buffer_position();
                let event = reader.read_event_into(&mut event_bytes)?;
                if body && let Event::Empty(tag) | Event::Start(tag) = &event {
                    references_images |=
                        matches!(tag.local_name().as_ref(), "blip" | "imagedata" | "image");
                }
                let overhead = match event {
                    Event::Eof => break,
                    Event::Start(ref tag) | Event::Empty(ref tag) => {
                        crate::limits::preview_budget(
                            depth < MAX_LAYOUT_XML_DEPTH,
                            "Office XML nesting exceeds layout limits",
                        )?;
                        if matches!(event, Event::Start(_)) {
                            depth += 1;
                        }
                        // Strings, child handles and the attribute map are separate allocations.
                        384 + 128 * tag.attributes().count() as u64
                    }
                    Event::End(_) => {
                        depth = depth.saturating_sub(1);
                        0
                    }
                    Event::DocType(_) => {
                        anyhow::bail!("External document entities are unsupported")
                    }
                    _ => 64,
                };
                let size = reader.buffer_position() - start;
                budget = budget
                    .saturating_add(size.saturating_mul(2))
                    .saturating_add(overhead);
                crate::limits::preview_budget(
                    budget <= crate::limits::RESOURCES_WORKING_BYTES,
                    "Office layout exceeds working memory",
                )?;
                event_bytes.clear();
            }
        } else {
            if image && expected >= 24 {
                let mut header = [0u8; 24];
                bounded.read_exact(&mut header)?;
                if header.starts_with(b"\x89PNG\r\n\x1a\n") && &header[12..16] == b"IHDR" {
                    let width = u32::from_be_bytes(header[16..20].try_into()?);
                    let height = u32::from_be_bytes(header[20..24].try_into()?);
                    let pixels = u64::from(width) * u64::from(height);
                    budget = budget.saturating_add(pixels.saturating_mul(16));
                    crate::limits::preview_budget(
                        budget <= crate::limits::RESOURCES_WORKING_BYTES,
                        "Office image exceeds layout memory",
                    )?;
                }
            }
            cancel.copy(&mut bounded, std::io::sink())?;
        }
        ensure!(
            bounded.limit() == 1,
            "Office part has an inconsistent decompressed size"
        );
    }
    Ok((budget, references_images))
}

pub(super) fn convert(path: &Path, extension: &str, cancel: &Cancellation) -> Result<TempPath> {
    let snapshot = MappedSnapshot::open(path, cancel)?;
    let bytes = snapshot.as_ref();
    let (required, references_images) = inspect_layout(bytes, cancel)?;
    let _memory = resources::pool().reserve_working(required, required)?;
    let output = if extension == "odt" {
        let document = lo_writer::load_bytes("", bytes, extension)?;
        // Writer leaves an empty paragraph when it drops every source drawing.
        // Its text projection includes descriptions of imported image blocks.
        ensure!(
            !references_images || !document.plain_text().trim().is_empty(),
            "The document's embedded drawings could not be read"
        );
        cancel.check()?;
        lo_writer::save_as(&document, "pdf")?
    } else {
        let slides = lo_impress::load_bytes("", bytes, extension)?;
        cancel.check()?;
        lo_impress::save_as(&slides, "pdf")?
    };
    temp_pdf(&output, cancel)
}

pub(super) fn legacy(
    path: &Path,
    extension: &str,
    cancel: &Cancellation,
) -> Result<PreparedDocument> {
    let mut output = NamedTempFile::new()?;
    let html = extension == "rtf";
    let input = MappedSnapshot::open(path, cancel)?;
    let input_size = input.as_ref().len() as u64;
    let required = input_size.saturating_mul(8).max(1024 * 1024);
    let _memory = resources::pool().reserve_working(required, required)?;
    if html {
        let raw = input.as_ref();
        let text = if let Ok(utf8) = std::str::from_utf8(raw) {
            std::borrow::Cow::Borrowed(utf8)
        } else {
            let encoding = raw
                .windows(8)
                .position(|v| v == b"\\ansicpg")
                .and_then(|pos| {
                    let code: Vec<_> = raw[pos + 8..]
                        .iter()
                        .copied()
                        .take_while(u8::is_ascii_digit)
                        .collect();
                    let code = std::str::from_utf8(&code).ok()?;
                    let label = match code {
                        "936" => "gbk".into(),
                        "932" => "shift_jis".into(),
                        "949" => "euc-kr".into(),
                        "950" => "big5".into(),
                        _ => format!("windows-{code}"),
                    };
                    encoding_rs::Encoding::for_label(label.as_bytes())
                })
                .unwrap_or(encoding_rs::WINDOWS_1252);
            encoding.decode(raw).0
        };
        let parsed = rtf_parser::RtfDocument::try_from(text.as_ref())
            .map_err(|error| anyhow::anyhow!("Invalid RTF: {error}"))?;
        for block in parsed.body {
            cancel.check()?;
            let mut text = quick_xml::escape::escape(&block.text).replace('\n', "<br>");
            for (active, tag) in [
                (block.painter.bold, "b"),
                (block.painter.italic, "i"),
                (block.painter.underline, "u"),
            ] {
                if active {
                    text = format!("<{tag}>{text}</{tag}>");
                }
            }
            output.write_all(text.as_bytes())?;
        }
    } else {
        let format = if extension == "doc" {
            office_oxide::DocumentFormat::Doc
        } else {
            office_oxide::DocumentFormat::Ppt
        };
        let mut options = office_oxide::options::ParseOptions::default();
        options.lazy_pictures = true;
        options.limits.memory_bytes = required;
        let document =
            office_oxide::Document::from_reader_with(Cursor::new(input), format, &options)?;
        let text = document.to_markdown();
        ensure!(!text.trim().is_empty(), "Document has no readable text");
        cancel.copy(Cursor::new(text.as_bytes()), &mut output)?;
    }
    cancel.check()?;
    output.flush()?;
    Ok(PreparedDocument::Text {
        page: paged::generated(path, output)?,
        html,
    })
}
