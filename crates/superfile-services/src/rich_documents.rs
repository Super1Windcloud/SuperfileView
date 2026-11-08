//! Portable document preparation. UI consumers receive pages or a retained workbook snapshot.
use crate::{paged, work::Cancellation};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::File,
    io::{BufReader, Write},
    path::Path,
};

pub enum PreparedDocument {
    Text { page: paged::TextPage, html: bool },
    Workbook(crate::spreadsheet::Spreadsheet),
}

pub fn prepare(path: &Path, cancel: &Cancellation) -> Result<PreparedDocument> {
    cancel.check()?;
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "xlsx" | "xlsm") {
        return Ok(PreparedDocument::Workbook(
            crate::spreadsheet::prepare(path, cancel)?
                .context("Workbook has no readable worksheets")?,
        ));
    }
    if matches!(extension.as_str(), "html" | "htm" | "xhtml") {
        return Ok(PreparedDocument::Text {
            page: paged::read_page_cancellable(path, 0, false, cancel)?,
            html: true,
        });
    }
    if matches!(extension.as_str(), "docx" | "pptx" | "odt" | "odp") {
        let mut archive = zip::ZipArchive::new(File::open(path)?)?;
        let parts = if extension == "docx" {
            vec!["word/document.xml".to_owned()]
        } else if extension == "pptx" {
            let mut parts = archive
                .file_names()
                .filter(|name| {
                    name.starts_with("ppt/slides/slide")
                        && name.ends_with(".xml")
                        && !name.contains("/_rels/")
                })
                .map(str::to_owned)
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
            let input =
                crate::work::RecordReader::new(entry, crate::limits::IO_RECORD_BYTES, cancel);
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
                        if tag.local_name().as_ref() == b"t"
                            || (matches!(extension.as_str(), "odt" | "odp")
                                && matches!(tag.local_name().as_ref(), b"p" | b"h"))
                        {
                            text = true;
                        }
                    }
                    Event::End(tag) => match tag.local_name().as_ref() {
                        b"t" => text = false,
                        b"p" | b"h" | b"tr" => {
                            output.write_all(b"\n\n")?;
                            text = false;
                        }
                        _ => {}
                    },
                    Event::Empty(tag)
                        if matches!(tag.local_name().as_ref(), b"br" | b"line-break") =>
                    {
                        output.write_all(b"\n")?
                    }
                    Event::Empty(tag) if tag.local_name().as_ref() == b"tab" => {
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
        ensure!(
            has_text,
            "This document contains no readable text; image-only Office previews are not implemented yet"
        );
        output.flush()?;
        return Ok(PreparedDocument::Text {
            page: paged::generated(path, output)?,
            html: false,
        });
    }
    bail!("This format does not yet have a portable reader: {extension}")
}
