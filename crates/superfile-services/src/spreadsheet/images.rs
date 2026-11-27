//! Extract embedded cell and anchored worksheet images from OOXML workbooks.
use super::{attr, attributes, cell_address, connection, scan_reader};
use crate::{limits, work::Cancellation};
use anyhow::Result;
use quick_xml::events::Event;
use rusqlite::{Connection, params};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) struct SheetMeta {
    pub id: i64,
    pub name: String,
    pub member: String,
    pub dispimg_cells: Vec<(String, String)>,
    pub rich_cells: Vec<(String, usize)>,
}

pub(super) fn parse_dispimg_id(text: &str) -> Option<&str> {
    let trimmed = text.trim().trim_start_matches('=').trim();
    let prefix = b"DISPIMG(";
    if !trimmed
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    {
        return None;
    }
    let after = trimmed.get(prefix.len()..)?.trim_start();
    let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let rest = after.get(quote.len_utf8()..)?;
    let end = rest.find(quote)?;
    let id = &rest[..end];
    (!id.is_empty()).then_some(id)
}

pub(super) fn extract(
    archive: &mut zip::ZipArchive<File>,
    sheets: &[SheetMeta],
    directory: &Path,
    worksheets: &mut Connection,
    cancel: &Cancellation,
) -> Result<Option<PathBuf>> {
    let mut cell_images: Vec<(String, String, String)> = Vec::new();
    let dispimg_targets = load_cellimages(archive, cancel)?;
    let rich_targets = load_rich_images(archive, cancel)?;

    for sheet in sheets {
        cancel.check()?;
        let table = format!("{}: {}", sheet.id, sheet.name);
        let mut anchored_missing = Vec::new();
        for (address, media_member) in load_sheet_drawings(archive, &sheet.member, cancel)? {
            cell_images.push((table.clone(), address.clone(), media_member));
            anchored_missing.push(address);
        }
        for (address, id) in &sheet.dispimg_cells {
            if let Some(media_member) = dispimg_targets.get(id) {
                cell_images.push((table.clone(), address.clone(), media_member.clone()));
            }
        }
        for (address, vm_index) in &sheet.rich_cells {
            if let Some(media_member) = rich_targets.get(vm_index) {
                cell_images.push((table.clone(), address.clone(), media_member.clone()));
            }
        }
        ensure_anchored_cells(worksheets, &table, &anchored_missing)?;
    }

    if cell_images.is_empty() {
        return Ok(None);
    }

    let media_dir = directory.join("media");
    std::fs::create_dir_all(&media_dir)?;
    let mut extracted: HashMap<String, PathBuf> = HashMap::new();
    let mut extracted_bytes = 0_u64;
    let mut index = 0_usize;

    let images_db_path = directory.join("images.sqlite");
    let images_db = connection(&images_db_path)?;
    images_db.execute_batch(
        "CREATE TABLE cell_images (table_name TEXT NOT NULL, address TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY (table_name, address)); BEGIN;",
    )?;

    let mut insert = images_db.prepare("INSERT OR IGNORE INTO cell_images VALUES (?1, ?2, ?3)")?;
    let mut recorded = 0_usize;
    for (table, address, member) in cell_images {
        cancel.check()?;
        let path = if let Some(existing) = extracted.get(&member) {
            existing.clone()
        } else {
            let Some(saved) = extract_media_member(
                archive,
                &member,
                &media_dir,
                &mut index,
                &mut extracted_bytes,
                cancel,
            )?
            else {
                continue;
            };
            extracted.insert(member, saved.clone());
            saved
        };
        insert.execute(params![table, address, path.to_string_lossy().as_ref()])?;
        recorded += 1;
    }
    drop(insert);
    images_db.execute_batch("COMMIT;")?;

    if recorded == 0 {
        return Ok(None);
    }
    Ok(Some(images_db_path))
}

fn ensure_anchored_cells(
    worksheets: &mut Connection,
    table: &str,
    addresses: &[String],
) -> Result<()> {
    if addresses.is_empty() {
        return Ok(());
    }
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let mut existing = HashSet::new();
    {
        let mut stmt = worksheets.prepare(&format!("SELECT Cell FROM {quoted}"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            existing.insert(row.get::<_, String>(0)?);
        }
    }
    let tx = worksheets.transaction()?;
    {
        let mut insert = tx.prepare(&format!("INSERT INTO {quoted} VALUES (?1, '', '')"))?;
        for address in addresses {
            if existing.insert(address.clone()) {
                insert.execute([address])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn extract_media_member(
    archive: &mut zip::ZipArchive<File>,
    member: &str,
    media_dir: &Path,
    index: &mut usize,
    total_bytes: &mut u64,
    cancel: &Cancellation,
) -> Result<Option<PathBuf>> {
    let Ok(entry) = archive.by_name(member) else {
        return Ok(None);
    };
    let size = entry.size();
    if size == 0
        || size > limits::IO_RECORD_BYTES as u64
        || total_bytes.saturating_add(size) > limits::IO_CACHE_BYTES
    {
        return Ok(None);
    }
    let ext = Path::new(member)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    if !crate::media::supports(Path::new(&format!("image.{ext}"))) {
        return Ok(None);
    }
    *index += 1;
    let destination = media_dir.join(format!("img-{}.{ext}", *index));
    let mut bounded = entry.take(size + 1);
    let copied = cancel.copy(&mut bounded, &mut File::create(&destination)?)?;
    if bounded.limit() == 0 {
        let _ = std::fs::remove_file(&destination);
        return Ok(None);
    }
    *total_bytes = total_bytes.saturating_add(copied);
    Ok(Some(destination))
}

fn rels_path_for(member: &str) -> String {
    match member.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{member}.rels"),
    }
}

fn read_relationships(
    archive: &mut zip::ZipArchive<File>,
    part_member: &str,
    cancel: &Cancellation,
) -> Result<HashMap<String, String>> {
    let rels_member = rels_path_for(part_member);
    let Ok(entry) = archive.by_name(&rels_member) else {
        return Ok(HashMap::new());
    };
    let mut map = HashMap::new();
    scan_reader(entry, cancel, |event| {
        if let Event::Start(tag) | Event::Empty(tag) = event
            && tag.local_name().as_ref() == "Relationship"
        {
            let a = attributes(tag)?;
            if attr(&a, "TargetMode") != "External"
                && let Some(resolved) = crate::books::resolve(part_member, attr(&a, "Target"))
            {
                map.insert(
                    attr(&a, "Id").to_owned(),
                    resolved.trim_start_matches('/').to_owned(),
                );
            }
        }
        Ok(())
    })?;
    Ok(map)
}

fn load_sheet_drawings(
    archive: &mut zip::ZipArchive<File>,
    sheet_member: &str,
    cancel: &Cancellation,
) -> Result<Vec<(String, String)>> {
    let sheet_rels = read_relationships(archive, sheet_member, cancel)?;
    if sheet_rels.is_empty() {
        return Ok(Vec::new());
    }
    let mut drawing_ids = Vec::new();
    if let Ok(entry) = archive.by_name(sheet_member) {
        scan_reader(entry, cancel, |event| {
            if let Event::Start(tag) | Event::Empty(tag) = event
                && tag.local_name().as_ref() == "drawing"
            {
                let a = attributes(tag)?;
                let id = attr(&a, "id");
                if !id.is_empty() {
                    drawing_ids.push(id.to_owned());
                }
            }
            Ok(())
        })?;
    }
    let mut results = Vec::new();
    for drawing_id in drawing_ids {
        let Some(drawing_member) = sheet_rels.get(&drawing_id) else {
            continue;
        };
        let drawing_rels = read_relationships(archive, drawing_member, cancel)?;
        let Ok(entry) = archive.by_name(drawing_member) else {
            continue;
        };
        let mut in_anchor = false;
        let mut in_from = false;
        let mut field = String::new();
        let mut col: Option<u64> = None;
        let mut row: Option<u64> = None;
        let mut embed_id: Option<String> = None;
        let mut buf = String::new();
        scan_reader(entry, cancel, |event| {
            match event {
                Event::Start(tag)
                    if matches!(tag.local_name().as_ref(), "twoCellAnchor" | "oneCellAnchor") =>
                {
                    in_anchor = true;
                    in_from = false;
                    col = None;
                    row = None;
                    embed_id = None;
                }
                Event::End(tag)
                    if matches!(tag.local_name().as_ref(), "twoCellAnchor" | "oneCellAnchor") =>
                {
                    if let (Some(r), Some(mut c), Some(id)) = (row, col, embed_id.take())
                        && let Some(target) = drawing_rels.get(&id)
                    {
                        let addr = cell_address("", r + 1, &mut c);
                        results.push((addr, target.clone()));
                    }
                    in_anchor = false;
                    in_from = false;
                }
                Event::Start(tag) if in_anchor && tag.local_name().as_ref() == "from" => {
                    in_from = true;
                }
                Event::End(tag) if in_anchor && tag.local_name().as_ref() == "from" => {
                    in_from = false;
                }
                Event::Start(tag)
                    if in_from && matches!(tag.local_name().as_ref(), "col" | "row") =>
                {
                    field = tag.local_name().as_ref().to_owned();
                    buf.clear();
                }
                Event::End(tag)
                    if in_from && matches!(tag.local_name().as_ref(), "col" | "row") =>
                {
                    if let Ok(parsed) = buf.trim().parse::<u64>() {
                        if field == "col" {
                            col = Some(parsed);
                        } else {
                            row = Some(parsed);
                        }
                    }
                    field.clear();
                }
                event @ (Event::Text(_) | Event::CData(_) | Event::GeneralRef(_))
                    if !field.is_empty() =>
                {
                    if let Some(text) = crate::xml::event_text(event)? {
                        buf.push_str(&text);
                    }
                }
                Event::Start(tag) | Event::Empty(tag)
                    if in_anchor && tag.local_name().as_ref() == "blip" =>
                {
                    let a = attributes(tag)?;
                    let embed = attr(&a, "embed");
                    if !embed.is_empty() {
                        embed_id = Some(embed.to_owned());
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(results)
}

fn load_cellimages(
    archive: &mut zip::ZipArchive<File>,
    cancel: &Cancellation,
) -> Result<HashMap<String, String>> {
    let rels = read_relationships(archive, "xl/cellimages.xml", cancel)?;
    if rels.is_empty() {
        return Ok(HashMap::new());
    }
    let Ok(entry) = archive.by_name("xl/cellimages.xml") else {
        return Ok(HashMap::new());
    };
    let mut map = HashMap::new();
    let mut in_cell_image = false;
    let mut name: Option<String> = None;
    let mut embed: Option<String> = None;
    scan_reader(entry, cancel, |event| {
        match event {
            Event::Start(tag) if tag.local_name().as_ref() == "cellImage" => {
                in_cell_image = true;
                name = None;
                embed = None;
            }
            Event::End(tag) if tag.local_name().as_ref() == "cellImage" => {
                if let (Some(n), Some(e)) = (name.take(), embed.take())
                    && let Some(target) = rels.get(&e)
                {
                    map.insert(n, target.clone());
                }
                in_cell_image = false;
            }
            Event::Start(tag) | Event::Empty(tag)
                if in_cell_image && tag.local_name().as_ref() == "cNvPr" =>
            {
                let a = attributes(tag)?;
                let value = attr(&a, "name");
                if !value.is_empty() {
                    name = Some(value.to_owned());
                }
            }
            Event::Start(tag) | Event::Empty(tag)
                if in_cell_image && tag.local_name().as_ref() == "blip" =>
            {
                let a = attributes(tag)?;
                let value = attr(&a, "embed");
                if !value.is_empty() {
                    embed = Some(value.to_owned());
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(map)
}

fn load_rich_images(
    archive: &mut zip::ZipArchive<File>,
    cancel: &Cancellation,
) -> Result<BTreeMap<usize, String>> {
    let rels = read_relationships(archive, "xl/richData/rdrichvaluerel.xml", cancel)?;
    if rels.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut rel_ids = Vec::new();
    if let Ok(entry) = archive.by_name("xl/richData/rdrichvaluerel.xml") {
        scan_reader(entry, cancel, |event| {
            if let Event::Start(tag) | Event::Empty(tag) = event
                && tag.local_name().as_ref() == "rel"
            {
                let a = attributes(tag)?;
                rel_ids.push(attr(&a, "id").to_owned());
            }
            Ok(())
        })?;
    }
    if rel_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let local_image_field = find_local_image_field(archive, cancel)?;
    let mut rv_rel_indices: Vec<Option<usize>> = Vec::new();
    if let Ok(entry) = archive.by_name("xl/richData/rdrichvalue.xml") {
        let mut in_rv = false;
        let mut structure_idx = 0_usize;
        let mut field_idx = 0_usize;
        let mut chosen: Option<usize> = None;
        let mut in_v = false;
        let mut buf = String::new();
        scan_reader(entry, cancel, |event| {
            match event {
                Event::Start(tag) if tag.local_name().as_ref() == "rv" => {
                    let a = attributes(tag)?;
                    structure_idx = attr(&a, "s").parse().unwrap_or(0);
                    in_rv = true;
                    field_idx = 0;
                    chosen = None;
                }
                Event::End(tag) if tag.local_name().as_ref() == "rv" => {
                    rv_rel_indices.push(chosen.take());
                    in_rv = false;
                }
                Event::Start(tag) if in_rv && tag.local_name().as_ref() == "v" => {
                    in_v = true;
                    buf.clear();
                }
                Event::End(tag) if in_rv && tag.local_name().as_ref() == "v" => {
                    let target_field = local_image_field.get(&structure_idx).copied().unwrap_or(0);
                    if field_idx == target_field
                        && let Ok(parsed) = buf.trim().parse::<usize>()
                    {
                        chosen = Some(parsed);
                    }
                    field_idx += 1;
                    in_v = false;
                }
                event @ (Event::Text(_) | Event::CData(_) | Event::GeneralRef(_)) if in_v => {
                    if let Some(text) = crate::xml::event_text(event)? {
                        buf.push_str(&text);
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    let mut vm_to_rv = Vec::new();
    if let Ok(entry) = archive.by_name("xl/metadata.xml") {
        let mut in_value_metadata = false;
        let mut in_bk = false;
        scan_reader(entry, cancel, |event| {
            match event {
                Event::Start(tag) if tag.local_name().as_ref() == "valueMetadata" => {
                    in_value_metadata = true;
                }
                Event::End(tag) if tag.local_name().as_ref() == "valueMetadata" => {
                    in_value_metadata = false;
                }
                Event::Start(tag) if in_value_metadata && tag.local_name().as_ref() == "bk" => {
                    in_bk = true;
                }
                Event::End(tag) if in_value_metadata && tag.local_name().as_ref() == "bk" => {
                    in_bk = false;
                }
                Event::Start(tag) | Event::Empty(tag)
                    if in_bk && tag.local_name().as_ref() == "rc" =>
                {
                    let a = attributes(tag)?;
                    vm_to_rv.push(attr(&a, "v").parse::<usize>().ok());
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    let mut out = BTreeMap::new();
    for (idx, rv_opt) in vm_to_rv.into_iter().enumerate() {
        let vm = idx + 1;
        if let Some(rv) = rv_opt
            && let Some(Some(rel_idx)) = rv_rel_indices.get(rv)
            && let Some(rid) = rel_ids.get(*rel_idx)
            && let Some(target) = rels.get(rid)
        {
            out.insert(vm, target.clone());
        }
    }
    Ok(out)
}

fn find_local_image_field(
    archive: &mut zip::ZipArchive<File>,
    cancel: &Cancellation,
) -> Result<HashMap<usize, usize>> {
    let Ok(entry) = archive.by_name("xl/richData/rdrichvaluestructure.xml") else {
        return Ok(HashMap::new());
    };
    let mut out = HashMap::new();
    let mut struct_idx = 0_usize;
    let mut key_idx = 0_usize;
    let mut in_s = false;
    scan_reader(entry, cancel, |event| {
        match event {
            Event::Start(tag) if tag.local_name().as_ref() == "s" => {
                in_s = true;
                key_idx = 0;
            }
            Event::End(tag) if tag.local_name().as_ref() == "s" => {
                in_s = false;
                struct_idx += 1;
            }
            Event::Start(tag) | Event::Empty(tag) if in_s && tag.local_name().as_ref() == "k" => {
                let a = attributes(tag)?;
                if attr(&a, "n") == "_rvRel:LocalImageIdentifier" {
                    out.insert(struct_idx, key_idx);
                }
                key_idx += 1;
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(out)
}

/// Resolve extracted worksheet/cell image paths for a batch of cell addresses.
pub fn cell_images_for_table(
    images_db: &Path,
    table: &str,
    addresses: &[&str],
    cancel: &Cancellation,
) -> Result<HashMap<String, url::Url>> {
    cancel.check()?;
    if addresses.is_empty() {
        return Ok(HashMap::new());
    }
    let db = Connection::open_with_flags(images_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt =
        db.prepare("SELECT path FROM cell_images WHERE table_name = ?1 AND address = ?2")?;
    let mut out = HashMap::new();
    for address in addresses {
        cancel.check()?;
        let mut rows = stmt.query(params![table, address])?;
        if let Some(row) = rows.next()? {
            let path: String = row.get(0)?;
            if let Ok(url) = url::Url::from_file_path(Path::new(&path)) {
                out.insert((*address).to_owned(), url);
            }
        }
    }
    Ok(out)
}
