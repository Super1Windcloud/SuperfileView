//! Portable document preparation. UI consumers receive pages or a retained workbook snapshot.
mod fixed_layout;
pub(crate) use fixed_layout::zip_size as package_size;
mod iwork_reader;
mod specimens;
use crate::{paged, work::Cancellation};
use anyhow::{Result, bail, ensure};
use std::{
    fs::File,
    io::{BufReader, Write},
    path::Path,
};

pub use tempfile::TempPath as DocumentPages;

pub enum PreparedDocument {
    Text { page: paged::TextPage, html: bool },
    Workbook(crate::spreadsheet::Spreadsheet),
    Pages(DocumentPages),
}

pub(crate) fn is_workbook_extension(extension: &str) -> bool {
    matches!(extension, "xls" | "xlsx" | "xlsm" | "ods")
}

pub(crate) fn is_fixed_document_extension(extension: &str) -> bool {
    matches!(extension, "docx" | "pptx" | "odt" | "odp")
}

pub fn has_text_view(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| is_fixed_document_extension(&ext.to_ascii_lowercase()))
}

pub fn read_document_text(path: &Path, cancel: &Cancellation) -> Result<PreparedDocument> {
    ensure!(has_text_view(path), "Unsupported document text view");
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    extract_text(path, &extension, cancel)
}

pub fn prepare(path: &Path, cancel: &Cancellation) -> Result<PreparedDocument> {
    cancel.check()?;
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if is_workbook_extension(&extension) {
        if let Some(workbook) = crate::spreadsheet::prepare(path, cancel)? {
            return Ok(PreparedDocument::Workbook(workbook));
        }
        if extension == "ods" {
            return Ok(PreparedDocument::Text {
                page: crate::formats::read_preview_cancellable(path, cancel)?,
                html: false,
            });
        }
        bail!("Workbook has no readable worksheets");
    }
    if is_fixed_document_extension(&extension) {
        match fixed_layout::convert(path, &extension, cancel) {
            Ok(file) => return Ok(PreparedDocument::Pages(file)),
            Err(error) => {
                cancel.check()?;
                log::warn!("Office page layout unavailable, reading document text: {error:#}");
            }
        }
    }
    if matches!(extension.as_str(), "doc" | "ppt" | "rtf") {
        return fixed_layout::legacy(path, &extension, cancel);
    }
    if crate::preview::is_font(path) {
        return specimens::prepare(path, cancel).map(PreparedDocument::Pages);
    }
    if crate::preview::is_iwork(path) {
        return iwork_reader::prepare(path, &extension, cancel);
    }
    if matches!(extension.as_str(), "html" | "htm" | "xhtml") {
        return Ok(PreparedDocument::Text {
            page: paged::read_page_cancellable(path, 0, false, cancel)?,
            html: true,
        });
    }
    if is_fixed_document_extension(&extension) {
        return match extract_text(path, &extension, cancel) {
            Err(error) if matches!(extension.as_str(), "odt" | "odp") => {
                cancel.check()?;
                log::warn!("OpenDocument text unavailable, reading container report: {error:#}");
                Ok(PreparedDocument::Text {
                    page: crate::formats::read_preview_cancellable(path, cancel)?,
                    html: false,
                })
            }
            result => result,
        };
    }
    bail!("This format does not yet have a portable reader: {extension}")
}

fn extract_text(path: &Path, extension: &str, cancel: &Cancellation) -> Result<PreparedDocument> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let parts = if extension == "docx" {
        vec!["word/document.xml".to_owned()]
    } else if extension == "pptx" {
        let mut parts = archive
            .file_names()
            .flatten()
            .filter(|name| {
                name.starts_with("ppt/slides/slide")
                    && name.ends_with(".xml")
                    && !name.contains("/_rels/")
            })
            .map(|name| name.into_owned())
            .collect::<Vec<_>>();
        parts.sort_by_key(|name| {
            name.trim_start_matches("ppt/slides/slide")
                .trim_end_matches(".xml")
                .parse::<u64>()
                .unwrap_or(u64::MAX)
        });
        parts
    } else {
        vec!["content.xml".to_owned()]
    };
    let mut output = tempfile::NamedTempFile::new()?;
    let mut has_text = false;
    for (index, part) in parts.iter().enumerate() {
        cancel.check()?;
        if extension == "pptx" {
            writeln!(output, "\n# {}\n", index + 1)?;
        }
        let entry = archive.by_name(part)?;
        let input = crate::work::RecordReader::new(entry, crate::limits::IO_RECORD_BYTES, cancel);
        let mut reader = quick_xml::Reader::from_reader(BufReader::with_capacity(
            crate::limits::IO_BUFFER_BYTES,
            input,
        ));
        let mut buffer = Vec::new();
        let mut text = false;
        loop {
            cancel.check()?;
            buffer.clear();
            reader
                .get_mut()
                .get_mut()
                .reset(crate::limits::IO_RECORD_BYTES);
            use quick_xml::events::Event;
            let event = reader.read_event_into(&mut buffer)?;
            match event {
                Event::Eof => break,
                Event::DocType(_) => bail!("Document external entities are unsupported"),
                Event::Start(tag) => {
                    if tag.local_name().as_ref() == "t"
                        || (matches!(extension, "odt" | "odp")
                            && matches!(tag.local_name().as_ref(), "p" | "h"))
                    {
                        text = true;
                    }
                }
                Event::End(tag) => match tag.local_name().as_ref() {
                    "t" => text = false,
                    "p" | "h" | "tr" => {
                        output.write_all(b"\n\n")?;
                        text = false;
                    }
                    _ => {}
                },
                Event::Empty(tag) if matches!(tag.local_name().as_ref(), "br" | "line-break") => {
                    output.write_all(b"\n")?
                }
                Event::Empty(tag) if tag.local_name().as_ref() == "tab" => {
                    output.write_all(b"\t")?
                }
                event if text => {
                    if let Some(text) = crate::xml::event_text(&event)? {
                        has_text |= !text.trim().is_empty();
                        output.write_all(text.as_bytes())?;
                    }
                }
                _ => {}
            }
        }
    }
    ensure!(has_text, "This document contains no readable text");
    output.flush()?;
    Ok(PreparedDocument::Text {
        page: paged::generated(path, output)?,
        html: false,
    })
}
