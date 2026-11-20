//! Read the object streams needed for semantic viewing; full-size package
//! media stays on disk. A directory bundle and its ZIP form share this path.
use super::PreparedDocument;
use crate::{paged, work::Cancellation};
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

fn needed(name: &str) -> bool {
    (name.starts_with("Index/") && name.ends_with(".iwa"))
        || name.starts_with("Metadata/")
        || name == ".iwph"
        || name == ".iwpv2"
}

fn append(
    entries: &mut Vec<(String, Vec<u8>)>,
    total: &mut u64,
    name: String,
    source: impl Read,
    cancel: &Cancellation,
) -> Result<()> {
    let remaining = crate::limits::ARCHIVE_BYTES.saturating_sub(*total);
    let mut bytes = Vec::new();
    cancel.copy(source.take(remaining + 1), &mut bytes)?;
    ensure!(
        bytes.len() as u64 <= remaining,
        "iWork object streams exceed the preview memory budget"
    );
    *total += bytes.len() as u64;
    entries.push((name, bytes));
    Ok(())
}

fn read_package(path: &Path, cancel: &Cancellation) -> Result<iwork::Package> {
    let mut package = iwork::Package::default();
    let mut total = 0;
    if path.is_dir() {
        package.form = iwork::Form::Directory;
        for entry in ignore::WalkBuilder::new(path)
            .standard_filters(false)
            .follow_links(false)
            .max_depth(Some(8))
            .build()
        {
            cancel.check()?;
            let entry = entry?;
            if !entry.file_type().is_some_and(|f| f.is_file()) {
                continue;
            }
            let name = entry
                .path()
                .strip_prefix(path)?
                .to_string_lossy()
                .replace('\\', "/");
            if needed(&name) {
                append(
                    &mut package.entries,
                    &mut total,
                    name,
                    File::open(entry.path())?,
                    cancel,
                )?;
            }
        }
    } else {
        let mut archive = zip::ZipArchive::new(File::open(path)?)?;
        for i in 0..archive.len() {
            cancel.check()?;
            let entry = archive.by_index(i)?;
            let name = String::from_utf8_lossy(entry.name_raw()).into_owned();
            if needed(&name) {
                append(&mut package.entries, &mut total, name, entry, cancel)?;
            }
        }
    }
    ensure!(
        package
            .entries
            .iter()
            .any(|(name, _)| name.ends_with(".iwa")),
        "No iWork object streams found"
    );
    Ok(package)
}

// IWA is a sequence of length-prefixed Snappy blocks. The protobuf object
// graph is built from the expanded streams, even when those compress very well.
fn object_bytes(name: &str, mut data: &[u8]) -> Result<u64> {
    if !name.ends_with(".iwa") {
        return Ok(data.len() as u64);
    }
    let mut total = 0u64;
    while !data.is_empty() {
        let (header, rest) = data.split_at_checked(4).context("Truncated iWork stream")?;
        ensure!(header[0] == 0, "Invalid iWork block type");
        let length = u32::from_le_bytes([header[1], header[2], header[3], 0]) as usize;
        let (block, next) = rest
            .split_at_checked(length)
            .context("Truncated iWork block")?;
        let mut size = 0u32;
        let mut completed = false;
        for (i, b) in block.iter().copied().take(3).enumerate() {
            size |= u32::from(b & 0x7f) << (i * 7);
            if b & 0x80 == 0 {
                completed = true;
                break;
            }
        }
        ensure!(completed && size <= 65_536, "Invalid iWork block size");
        total = total.saturating_add(u64::from(size));
        data = next;
    }
    Ok(total)
}

fn semantic(
    path: &Path,
    extension: &str,
    cancel: &Cancellation,
) -> Result<(PreparedDocument, bool)> {
    let package = read_package(path, cancel)?;
    let required = package
        .entries
        .iter()
        .try_fold(0u64, |sum, (name, data)| {
            Ok::<_, anyhow::Error>(sum.saturating_add(object_bytes(name, data)?))
        })?
        .saturating_mul(8);
    let _memory = crate::resources::pool().reserve_working(required, required)?;
    let doc = iwork::Document::from_package(package).context("Could not read iWork package")?;
    let mut file = tempfile::NamedTempFile::new()?;
    let mut has_content = false;
    let escaped = |s: &str| quick_xml::escape::escape(s).replace('\n', "<br>");
    match extension {
        "key" => {
            for slide in doc.slides() {
                cancel.check()?;
                writeln!(file, "<h2>{}</h2>", slide.index + 1)?;
                for text in slide.texts {
                    has_content |= !text.text.trim().is_empty();
                    writeln!(file, "<p>{}</p>", escaped(&text.text))?;
                }
            }
        }
        "pages" => {
            for storage in doc.text_storages() {
                cancel.check()?;
                has_content |= !storage.text.trim().is_empty();
                writeln!(file, "<p>{}</p>", escaped(&storage.text))?;
            }
        }
        _ => {}
    }
    for table in doc.tables() {
        has_content = true;
        cancel.check()?;
        if let Some(ref sheet) = table.sheet {
            writeln!(file, "<h2>{}</h2>", escaped(sheet))?;
        }
        writeln!(file, "<h3>{}</h3><table>", escaped(&table.name))?;
        for cell in table.cells() {
            cancel.check()?;
            writeln!(
                file,
                "<tr><td>{}</td><td>{}</td></tr>",
                crate::spreadsheet::cell_address(
                    "",
                    cell.row as u64 + 1,
                    &mut (cell.column as u64)
                ),
                escaped(&cell.value.to_text())
            )?;
        }
        writeln!(file, "</table>")?;
    }
    ensure!(
        file.as_file().metadata()?.len() > 0,
        "iWork document contains no readable text or tables"
    );
    file.flush()?;
    Ok((
        PreparedDocument::Text {
            page: paged::generated(path, file)?,
            html: true,
        },
        has_content,
    ))
}

// iWork '09 packages can carry a full Quick Look PDF. Newer IWA packages
// commonly provide only a first-page JPEG, so prefer their searchable content.
pub(super) fn prepare(
    path: &Path,
    extension: &str,
    cancel: &Cancellation,
) -> Result<PreparedDocument> {
    if let Ok(Some(pdf)) = member(path, "QuickLook/Preview.pdf", cancel)
        && pdf.starts_with(b"%PDF-")
        && let Ok(file) = super::fixed_layout::temp_pdf(&pdf, cancel)
        && crate::pdf::PdfDocument::open(&file, cancel).is_ok()
    {
        return Ok(PreparedDocument::Pages(file));
    }
    cancel.check()?;
    let semantic = semantic(path, extension, cancel);
    if let Ok((doc, true)) = semantic {
        return Ok(doc);
    }
    for name in ["preview.jpg", "QuickLook/Thumbnail.jpg", "preview-web.jpg"] {
        cancel.check()?;
        if let Ok(Some(preview)) = member(path, name, cancel)
            && let Ok(file) = jpeg_preview(&preview, cancel)
        {
            return Ok(PreparedDocument::Pages(file));
        }
    }
    cancel.check()?;
    semantic.map(|(doc, _)| doc)
}

fn member(path: &Path, name: &str, cancel: &Cancellation) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let limit = crate::limits::ARCHIVE_BYTES;
    if path.is_dir() {
        let path = path.join(name);
        if path.parent().is_some_and(|p| p.is_symlink())
            || !path.symlink_metadata().is_ok_and(|m| m.is_file())
        {
            return Ok(None);
        }
        let file = File::open(path)?;
        ensure!(
            file.metadata()?.len() <= limit,
            "iWork preview exceeds memory budget"
        );
        cancel.copy(file.take(limit + 1), &mut bytes)?;
    } else {
        let mut zip = zip::ZipArchive::new(File::open(path)?)?;
        let Ok(entry) = zip.by_name(name) else {
            return Ok(None);
        };
        ensure!(entry.size() <= limit, "iWork preview exceeds memory budget");
        cancel.copy(entry.take(limit + 1), &mut bytes)?;
    }
    ensure!(
        bytes.len() as u64 <= limit,
        "iWork preview exceeds memory budget"
    );
    Ok(Some(bytes))
}

fn jpeg_preview(bytes: &[u8], cancel: &Cancellation) -> Result<tempfile::TempPath> {
    use image::{ColorType, ImageDecoder};
    use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};
    let jpeg = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(bytes))?;
    let (width, height) = jpeg.dimensions();
    ensure!(
        u64::from(width)
            .saturating_mul(u64::from(height))
            .saturating_mul(4)
            <= crate::limits::RESOURCES_WORKING_BYTES,
        "iWork preview exceeds image budget"
    );
    let mut pdf = Pdf::new();
    pdf.catalog(Ref::new(1)).pages(Ref::new(2));
    pdf.pages(Ref::new(2)).kids([Ref::new(3)]).count(1);
    let mut page = pdf.page(Ref::new(3));
    let scale = 800. / width.max(height).max(1) as f32;
    let (w, h) = (width as f32 * scale, height as f32 * scale);
    let name = Name(b"Cover");
    page.parent(Ref::new(2))
        .media_box(Rect::new(0., 0., w, h))
        .contents(Ref::new(5));
    page.resources().x_objects().pair(name, Ref::new(4));
    page.finish();
    let mut image = pdf.image_xobject(Ref::new(4), bytes);
    image
        .width(i32::try_from(width)?)
        .height(i32::try_from(height)?)
        .bits_per_component(8)
        .filter(Filter::DctDecode);
    match jpeg.color_type() {
        ColorType::L8 => {
            image.color_space().device_gray();
        }
        ColorType::Rgb8 => {
            image.color_space().device_rgb();
        }
        _ => anyhow::bail!("Unsupported iWork thumbnail color space"),
    }
    image.finish();
    let mut content = Content::new();
    content.transform([w, 0., 0., h, 0., 0.]).x_object(name);
    pdf.stream(Ref::new(5), &content.finish());
    super::fixed_layout::temp_pdf(&pdf.finish(), cancel)
}
