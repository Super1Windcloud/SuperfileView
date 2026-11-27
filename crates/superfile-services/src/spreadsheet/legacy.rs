//! Import BIFF and OpenDocument workbooks into the same worksheet browser.
use super::{Spreadsheet, cell_address, connection, worksheet_insert};
use crate::work::Cancellation;
use anyhow::{Context, Result, ensure};
use calamine::{Data, Reader};
use rusqlite::params;
use std::{io::Cursor, path::Path};

pub(super) fn prepare(path: &Path, cancel: &Cancellation) -> Result<Spreadsheet> {
    let input = crate::snapshot::MappedSnapshot::open(path, cancel)?;
    let bytes = input.as_ref();
    let ods = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ods"));
    let input_size = if ods {
        crate::rich_documents::package_size(bytes, cancel)?
    } else {
        bytes.len() as u64
    };
    let input_budget = input_size.saturating_mul(8);
    let _input = crate::resources::pool().reserve_working(input_budget, input_budget)?;
    // Calamine materializes rectangular ranges and their formulas before the
    // disk-backed importer sees the first cell. Size this separately from I/O.
    let required = if ods {
        ods_budget(bytes, cancel)?
    } else {
        biff_cells(bytes, cancel)?.saturating_mul(128)
    };
    let _memory = crate::resources::pool().reserve_working(required, required)?;
    let cursor = std::io::Cursor::new(bytes);
    if ods {
        index(calamine::Ods::new(cursor)?, cancel)
    } else {
        index(calamine::Xls::new(cursor)?, cancel)
    }
}

fn ods_budget(bytes: &[u8], cancel: &Cancellation) -> Result<u64> {
    use quick_xml::events::Event;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let part = archive.by_name("content.xml")?;
    let (mut total, mut row, mut column) = (0u64, 0u64, 0u64);
    let (mut used_row, mut used_column) = (0u64, 0u64);
    let (mut row_repeat, mut cell_repeat) = (1u64, 0u64);
    let mut depth = 0usize;
    super::scan_reader(part, cancel, |event| {
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
                let attrs = super::attributes(tag)?;
                let repeat = |name: &str| -> Result<u64> {
                    let value = super::attr(&attrs, name);
                    if value.is_empty() {
                        Ok(1)
                    } else {
                        Ok(value.parse()?)
                    }
                };
                match tag.local_name().as_ref() {
                    "table-row" => {
                        row_repeat = repeat("number-rows-repeated")?;
                        row = row.saturating_add(row_repeat);
                        column = 0;
                    }
                    "table-cell" | "covered-table-cell" => {
                        let repeated = repeat("number-columns-repeated")?;
                        // A subsequent cell forces preceding empty positions into
                        // the temporary row, even when it too is empty.
                        total = total
                            .saturating_add(column.saturating_sub(used_column).saturating_mul(64));
                        column = column.saturating_add(repeated);
                        // Empty trailing runs are common in LibreOffice documents.
                        if matches!(event, Event::Start(_))
                            || attrs.iter().any(|(key, _)| {
                                key == "formula" || key == "value-type" || key.ends_with("value")
                            })
                        {
                            used_row = row;
                            used_column = used_column.max(column);
                            total = total.saturating_add(
                                (tag.len() as u64)
                                    .saturating_mul(row_repeat)
                                    .saturating_mul(repeated),
                            );
                        }
                        cell_repeat = if matches!(event, Event::Start(_)) {
                            repeated
                        } else {
                            0
                        };
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                depth = depth.checked_sub(1).context("Unbalanced worksheet XML")?;
                match tag.local_name().as_ref() {
                    "table" => {
                        total = total.saturating_add(
                            used_row.saturating_mul(used_column).saturating_mul(128),
                        );
                        (row, column, used_row, used_column) = (0, 0, 0, 0);
                    }
                    "table-cell" | "covered-table-cell" => cell_repeat = 0,
                    _ => {}
                }
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if cell_repeat > 0 => {
                total = total.saturating_add(
                    (event.len() as u64)
                        .saturating_mul(cell_repeat)
                        .saturating_mul(row_repeat)
                        .saturating_mul(4),
                );
            }
            _ => {}
        }
        Ok(())
    })?;
    ensure!(depth == 0, "Incomplete worksheet XML");
    Ok(total)
}

fn biff_cells(bytes: &[u8], cancel: &Cancellation) -> Result<u64> {
    let mut cfb = office_oxide::cfb::CfbReader::new(Cursor::new(bytes))?;
    let entry = cfb
        .entries()
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case("Workbook") || e.name.eq_ignore_ascii_case("Book"))
        .context("Missing BIFF workbook")?;
    ensure!(
        entry.stream_size <= bytes.len() as u64,
        "Invalid BIFF workbook size"
    );
    let stream = cfb.open_stream(&entry.name.clone())?;
    let (mut total, mut rows, mut columns) = (0u64, 0u64, 0u64);
    let (mut first_row, mut first_column) = (u64::MAX, u64::MAX);
    let area = |r: u64, c: u64, top: u64, left: u64| {
        r.saturating_sub(top).saturating_mul(c.saturating_sub(left))
    };
    let mut remaining = stream.as_slice();
    let u16_at =
        |bytes: &[u8], pos| -> u64 { u64::from(u16::from_le_bytes([bytes[pos], bytes[pos + 1]])) };
    while remaining.len() >= 4 {
        cancel.check()?;
        let tag = u16_at(remaining, 0);
        let size = u16_at(remaining, 2) as usize;
        let (data, next) = remaining[4..]
            .split_at_checked(size)
            .context("Incomplete BIFF record")?;
        remaining = next;
        match tag {
            0x0200 if data.len() == 10 => {
                ensure!(
                    u16_at(data, 2) == 0 || u16_at(data, 2) > u16_at(data, 0),
                    "Invalid BIFF row range"
                );
                rows = rows.max(u16_at(data, 2));
                columns = columns.max(u16_at(data, 6));
                first_row = first_row.min(u16_at(data, 0));
                let col = u16_at(data, 4);
                first_column = first_column.min(if col <= 255 && col < u16_at(data, 6) {
                    col
                } else {
                    0
                });
            }
            0x0200 if matches!(data.len(), 14 | 16) => {
                let first = u32::from_le_bytes(data[..4].try_into()?);
                let end = u32::from_le_bytes(data[4..8].try_into()?);
                ensure!(end == 0 || end > first, "Invalid BIFF row range");
                rows = rows.max(u64::from(end));
                columns = columns.max(u16_at(data, 10));
                first_row = first_row.min(u64::from(first));
                let col = u16_at(data, 8);
                first_column = first_column.min(if col <= 255 && col < u16_at(data, 10) {
                    col
                } else {
                    0
                });
            }
            0x0203 | 0x0204 | 0x00d6 | 0x0205 | 0x027e | 0x00fd | 0x0006 | 0x00bd
                if data.len() >= 4 =>
            {
                rows = rows.max(u16_at(data, 0) + 1);
                first_row = first_row.min(u16_at(data, 0));
                first_column = first_column.min(u16_at(data, 2));
                let last = if tag == 0x00bd { data.len() - 2 } else { 2 };
                columns = columns.max(u16_at(data, last) + 1);
            }
            // BOF marks workbook or worksheet substreams. Conservatively keep
            // the maxima through nested chart streams until the next sheet.
            0x0809 | 0x0409 | 0x0209 | 0x0009
                if data.len() >= 4 && matches!(u16_at(data, 2), 0x10 | 0x05) =>
            {
                total = total.saturating_add(area(rows, columns, first_row, first_column));
                (rows, columns) = (0, 0);
                (first_row, first_column) = (u64::MAX, u64::MAX);
            }
            _ => {}
        }
    }
    Ok(total.saturating_add(area(rows, columns, first_row, first_column)))
}

fn index<R: std::io::Read + std::io::Seek, W: Reader<R>>(
    mut book: W,
    cancel: &Cancellation,
) -> Result<Spreadsheet>
where
    W::Error: std::error::Error + Send + Sync + 'static,
{
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("worksheets.sqlite");
    let mut output = connection(&database)?;
    let transaction = output.transaction()?;
    let names = book.sheet_names();
    ensure!(!names.is_empty(), "Workbook contains no sheets");
    for (index, name) in names.iter().enumerate() {
        cancel.check()?;
        let range = book
            .worksheet_range(name)
            .with_context(|| format!("Could not read worksheet {name}"))?;
        let formulas = book.worksheet_formula(name)?;
        let mut insert = worksheet_insert(&transaction, (index + 1) as i64, name)?;
        let (row0, col0) = range.start().unwrap_or_default();
        for (row, col, data) in range.used_cells() {
            cancel.check()?;
            let r = row0 + row as u32;
            let c = col0 + col as u32;
            let address = cell_address("", u64::from(r) + 1, &mut u64::from(c));
            let formatted = match data {
                Data::DateTime(value) if value.is_datetime() => value
                    .as_datetime()
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| data.to_string()),
                _ => data.to_string(),
            };
            insert.execute(params![
                address,
                formatted,
                formulas.get_value((r, c)).map(String::as_str).unwrap_or("")
            ])?;
        }
        // A formula may have no cached result and consequently no data cell.
        let (fr, fc) = formulas.start().unwrap_or_default();
        for (r, c, formula) in formulas.used_cells() {
            cancel.check()?;
            let position = (fr + r as u32, fc + c as u32);
            if range.get_value(position).is_none_or(|v| *v == Data::Empty) {
                let address =
                    cell_address("", u64::from(position.0) + 1, &mut u64::from(position.1));
                insert.execute(params![address, "", formula])?;
            }
        }
    }
    transaction.commit()?;
    Ok(Spreadsheet {
        cache: None,
        images_database: None,
        database,
        information: crate::locale::tr(
            "Read-only worksheet cells · Saved formula results · Select a worksheet, page through cells, or double-click a value to read it in full.",
        ),
        worksheets: names.len(),
        _directory: directory,
    })
}
