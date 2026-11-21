use anyhow::Result;
use std::{fs, io::Write, path::Path};
use superfile_services::{
    archive, files, inline_images, platform,
    rich_documents::{self, PreparedDocument},
    work::Cancellation,
};

fn zip(path: &Path, members: &[(&str, &[u8])]) -> Result<()> {
    let mut archive = zip::ZipWriter::new(fs::File::create(path)?);
    for (name, data) in members {
        archive.start_file(*name, zip::write::SimpleFileOptions::default())?;
        archive.write_all(data)?;
    }
    archive.finish()?;
    Ok(())
}

#[test]
fn docx_and_pptx_use_portable_text_reading() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let docx = directory.path().join("文档.docx");
    zip(&docx, &[("word/document.xml", br#"<w:document xmlns:w="word"><w:p><w:r><w:t>Hello &amp; world</w:t></w:r></w:p><w:p><w:r><w:t>Second paragraph</w:t></w:r></w:p></w:document>"#)])?;
    let PreparedDocument::Text { page, .. } =
        rich_documents::read_document_text(&docx, &Cancellation::default())?
    else {
        panic!("expected text")
    };
    assert!(page.text.contains("Hello & world"));
    assert!(page.text.contains("Second paragraph"));
    let pptx = directory.path().join("slides.pptx");
    zip(
        &pptx,
        &[
            (
                "ppt/slides/slide10.xml",
                b"<slide><p><t>Tenth</t></p></slide>",
            ),
            (
                "ppt/slides/slide2.xml",
                b"<slide><p><t>Second</t></p></slide>",
            ),
        ],
    )?;
    let PreparedDocument::FallbackText { page, .. } =
        rich_documents::prepare(&pptx, &Cancellation::default())?
    else {
        panic!("expected text")
    };
    assert!(page.text.find("Second").unwrap() < page.text.find("Tenth").unwrap());
    Ok(())
}

#[test]
fn cancelled_preparation_never_publishes_a_document() -> Result<()> {
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    assert!(
        rich_documents::prepare(
            Path::new("missing.docx"),
            &Cancellation::from_flag(cancelled)
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn save_conflicts_preserve_external_edits_and_file_ids_detect_replacement() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("笔记.txt");
    fs::write(&path, "original")?;
    let original = fs::File::open(&path)?;
    let identity = platform::file_identity(&original)?;
    drop(original);
    files::save_document(&path, "original", "saved")?;
    assert_eq!(fs::read_to_string(&path)?, "saved");
    fs::write(&path, "external")?;
    assert!(files::save_document(&path, "saved", "overwrite").is_err());
    assert_eq!(fs::read_to_string(&path)?, "external");
    let new_file = fs::File::open(&path)?;
    assert_ne!(identity, platform::file_identity(&new_file)?);
    Ok(())
}

#[test]
fn archive_traversal_is_rejected_without_touching_siblings() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let archive_path = directory.path().join("unsafe.zip");
    zip(&archive_path, &[("../escape.txt", b"invalid")])?;
    assert!(archive::extract(&archive_path, directory.path(), &Cancellation::default()).is_err());
    assert!(!directory.path().join("escape.txt").exists());
    Ok(())
}

#[test]
fn archive_extraction_publishes_new_directories_without_replacing_previous_results() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let archive_path = directory.path().join("example.zip");
    zip(&archive_path, &[("nested/file.txt", b"content")])?;
    let first = archive::extract(&archive_path, directory.path(), &Cancellation::default())?;
    let second = archive::extract(&archive_path, directory.path(), &Cancellation::default())?;
    assert_ne!(first, second);
    assert_eq!(fs::read(first.join("nested/file.txt"))?, b"content");
    assert_eq!(fs::read(second.join("nested/file.txt"))?, b"content");
    Ok(())
}

#[test]
fn inline_image_decoding_uses_rust_on_each_platform() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("image.png");
    image::RgbaImage::from_pixel(80, 40, image::Rgba([10, 30, 80, 255])).save(&path)?;
    let frame = inline_images::from_file_cancellable(&path, &Cancellation::default())?;
    assert_eq!(frame.buffer().width(), 80);
    assert_eq!(frame.buffer().height(), 40);
    Ok(())
}

#[test]
fn pdf_pages_use_an_immutable_snapshot_and_render_without_a_system_viewer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("page.pdf");
    let content = "q 1 0 0 rg 0 0 72 72 re f Q";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> /Contents 4 0 R >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    fs::write(&path, pdf)?;
    let document = superfile_services::pdf::PdfDocument::open(&path, &Cancellation::default())?;
    fs::write(&path, "source changed after opening")?;
    let page = document.render(0, &Cancellation::default())?;
    assert_eq!(page.pages, 1);
    let image = image::load_from_memory(&page.png)?.to_rgba8();
    assert!(image.width() > 0 && image.height() > 0);
    assert!(image.get_pixel(image.width() / 2, image.height() / 2)[0] > 200);
    assert!(document.render(1, &Cancellation::default()).is_err());
    Ok(())
}

#[test]
fn office_pages_use_the_pdf_viewport_and_editor_palette() -> Result<()> {
    use superfile_core::reader_palette::ReaderPalette;
    use superfile_services::{
        documents::{Content, read_content},
        pdf::PdfDocument,
    };
    let directory = tempfile::tempdir()?;
    let mut word = lo_writer::WriterEditor::new("");
    word.push_heading(1, "Sample layout")
        .push_paragraph("Body and page layout");
    let mut slides = lo_impress::ImpressBuilder::new("");
    slides
        .add_title_slide("First", "Opening")
        .add_diagram_slide("Second");
    let cancel = Cancellation::default();
    for (ext, bytes, count) in [
        ("odt", lo_writer::save_as(&word.document, "odt")?, 1),
        (
            "pptx",
            lo_impress::save_as(&slides.presentation, "pptx")?,
            2,
        ),
        ("odp", lo_impress::save_as(&slides.presentation, "odp")?, 2),
    ] {
        let source = directory.path().join(format!("sample.{ext}"));
        fs::write(&source, bytes)?;
        assert!(
            matches!(read_content(&source)?, Content::Native(..)),
            "{ext}"
        );
        let PreparedDocument::Pages(pdf) = rich_documents::prepare(&source, &cancel)? else {
            panic!("expected fixed pages for {ext}")
        };
        let doc = PdfDocument::open(&pdf, &cancel)?;
        assert_eq!(doc.page_count(), count, "{ext}");
        let palette = ReaderPalette {
            paper: [15, 22, 37],
            ink: [215, 218, 225],
            surface: [40; 3],
            accent: [100; 3],
        };
        let image = doc.render_with_palette(count - 1, Some(palette), &cancel)?;
        let image = image::load_from_memory(&image.png)?.to_rgb8();
        assert!(
            image.pixels().any(|pixel| pixel
                .0
                .iter()
                .zip(palette.ink)
                .all(|(a, b)| a.abs_diff(b) <= 3)),
            "missing ink in {ext}"
        );
        assert!(
            image.pixels().any(|pixel| pixel
                .0
                .iter()
                .zip(palette.paper)
                .all(|(a, b)| a.abs_diff(b) <= 3)),
            "missing paper in {ext}"
        );
    }
    Ok(())
}

#[test]
fn reads_iwork_packages_and_spreadsheet_cells() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let cancel = Cancellation::default();
    let mut doc = iwork::Document::new(iwork::Kind::Pages)?;
    doc.body_mut()?.append("iWork reader sample")?;
    let path = directory.path().join("sample.pages");
    doc.save(&path)?;
    let PreparedDocument::Text { page, .. } = rich_documents::prepare(&path, &cancel)? else {
        panic!("expected Pages text")
    };
    assert!(page.text.contains("iWork reader sample"));
    let path = directory.path().join("sample.numbers");
    let mut numbers = iwork::Document::new_spreadsheet("Demo", "Sales", 4, 3)?;
    numbers.table_mut("Sales")?.set("B2", "North")?;
    numbers.save(&path)?;
    let PreparedDocument::Text { page, .. } = rich_documents::prepare(&path, &cancel)? else {
        panic!("expected Numbers table")
    };
    assert!(page.text.contains("B2") && page.text.contains("North"));
    let path = directory.path().join("sample.key");
    let mut keynote = iwork::Document::new(iwork::Kind::Keynote)?;
    keynote.add_slide(None)?;
    keynote.save(&path)?;
    let PreparedDocument::Text { page, .. } = rich_documents::prepare(&path, &cancel)? else {
        panic!("expected Keynote slides")
    };
    assert!(page.text.contains("<h2>2</h2>"));
    let ods = directory.path().join("legacy.ods");
    zip(&ods,&[
        ("mimetype",b"application/vnd.oasis.opendocument.spreadsheet"),
        ("META-INF/manifest.xml",br#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"/>"#),
        ("content.xml",br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Sales"><table:table-row><table:table-cell office:value-type="float" office:value="42"><text:p>42</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#)
    ])?;
    let PreparedDocument::Workbook(book) = rich_documents::prepare(&ods, &cancel)? else {
        panic!("expected ODS worksheet")
    };
    let db = rusqlite::Connection::open(book.database)?;
    let values = db.query_row("SELECT * FROM \"1: Sales\"", [], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    assert_eq!(values, ("A1".into(), "42".into()));
    Ok(())
}

#[test]
fn continuous_word_preserves_embedded_image_pixels() -> Result<()> {
    use superfile_services::rich_documents::docx::model::Block;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("photo.docx");
    let pixel = image::Rgb([220, 22, 44]);
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(32, 32, pixel).write_to(&mut png, image::ImageFormat::Png)?;
    zip(&path, &[
        ("word/document.xml", br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="914400"/><a:graphic><a:blip r:embed="rId1"/></a:graphic></wp:inline></w:drawing></w:r></w:p></w:body></w:document>"#),
        ("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/photo.png"/></Relationships>"#),
        ("word/media/photo.png", png.get_ref()),
    ])?;
    let cancel = Cancellation::default();
    let PreparedDocument::Word(doc) = rich_documents::prepare(&path, &cancel)? else {
        panic!("expected continuous Word content")
    };
    let [Block::Image(photo)] = doc.blocks.as_slice() else {
        panic!("expected the original embedded image")
    };
    assert_eq!(photo.data, *png.get_ref());
    assert_eq!(
        image::load_from_memory(&photo.data)?
            .to_rgb8()
            .get_pixel(0, 0),
        &pixel
    );
    Ok(())
}

#[test]
fn font_specimens_draw_the_file_glyphs_from_sfnt_and_resource_fork() -> Result<()> {
    // Original test font containing one closed triangle mapped to 'A'.
    let font = include_bytes!("../../../assets/reader-contracts/triangle.ttf");
    let mut data = Vec::new();
    let length = font.len() as u32 + 4;
    for field in [16u32, 16 + length, length, 50] {
        data.extend(field.to_be_bytes());
    }
    data.extend((font.len() as u32).to_be_bytes());
    data.extend(font);
    let mut map = vec![0u8; 50];
    map[..16].copy_from_slice(&data[..16]);
    map[24..26].copy_from_slice(&28u16.to_be_bytes());
    map[26..28].copy_from_slice(&50u16.to_be_bytes());
    map[30..34].copy_from_slice(b"sfnt");
    map[36..38].copy_from_slice(&10u16.to_be_bytes());
    map[40..42].copy_from_slice(&u16::MAX.to_be_bytes());
    data.extend(map);
    let directory = tempfile::tempdir()?;
    let cancel = Cancellation::default();
    for (ext, bytes) in [("ttf", font.as_slice()), ("dfont", data.as_slice())] {
        let path = directory.path().join(format!("triangle.{ext}"));
        fs::write(&path, bytes)?;
        let PreparedDocument::Pages(pdf) = rich_documents::prepare(&path, &cancel)? else {
            panic!("expected specimen for {ext}")
        };
        let doc = superfile_services::pdf::PdfDocument::open(&pdf, &cancel)?;
        let frame = doc.render(0, &cancel)?;
        let image = image::load_from_memory(&frame.png)?.to_luma8();
        assert!(image.iter().filter(|v| **v < 32).count() > 100);
    }
    Ok(())
}

#[test]
fn biff_workbook_and_rtf_read_their_native_content() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("quarter.xls");
    fs::write(
        &path,
        include_bytes!("../../../assets/reader-contracts/quarter.xls"),
    )?;
    let cancel = Cancellation::default();
    let PreparedDocument::Workbook(book) = rich_documents::prepare(&path, &cancel)? else {
        panic!("expected BIFF worksheet")
    };
    let db = rusqlite::Connection::open(book.database)?;
    let (cell, value) =
        db.query_row("SELECT * FROM \"1: Quarter\" WHERE Cell='B2'", [], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
    assert_eq!((cell.as_str(), value.as_str()), ("B2", "42"));
    let formula: String = db.query_row(
        "SELECT Formula FROM \"1: Quarter\" WHERE Cell='B3'",
        [],
        |row| row.get(0),
    )?;
    assert!(formula.contains("B2"));
    let path = directory.path().join("styled.rtf");
    fs::write(&path, br"{\rtf1\ansi hello {\b bold} content}")?;
    let PreparedDocument::Text { page, html } = rich_documents::prepare(&path, &cancel)? else {
        panic!("expected RTF spans")
    };
    assert!(html && page.text.contains("<b>bold</b>"));
    Ok(())
}

#[test]
fn nested_word_table_keeps_its_continuous_layout() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("tables.docx");
    let cell = r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:tcBorders><w:bottom w:val="single"/></w:tcBorders></w:tcPr><w:p><w:r><w:rPr><w:b/><w:color w:val="012345"/><w:sz w:val="20"/></w:rPr><w:t>Cell text</w:t></w:r></w:p></w:tc>"#;
    let row = format!("<w:tr>{}</w:tr>", cell.repeat(4));
    let nested_table = format!("<w:tbl>{row}</w:tbl>");
    let body = format!(
        "<w:tbl>{}</w:tbl>",
        format!("<w:tr><w:tc>{nested_table}<w:p/></w:tc></w:tr>").repeat(180)
    );
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    zip(&path, &[("word/document.xml", xml.as_bytes())])?;
    let cancel = Cancellation::default();
    let PreparedDocument::Word(word) = rich_documents::prepare(&path, &cancel)? else {
        panic!("ordinary nested tables should open in the formatted preview");
    };
    let [rich_documents::docx::model::Block::Table(table)] = word.blocks.as_slice() else {
        panic!("expected the Word table");
    };
    assert_eq!(table.rows.len(), 180);
    let nested = &table.rows[0].cells[0].tables[0].table;
    assert_eq!(nested.rows[0].cells.len(), 4);
    Ok(())
}

#[test]
fn oversized_office_layout_reuses_searchable_text() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("large.docx");
    let nested = format!(
        "<document><body>{}<p><t>Readable</t></p>{}</body></document>",
        "<span>".repeat(256),
        "</span>".repeat(256)
    );
    zip(&path, &[("word/document.xml", nested.as_bytes())])?;
    let PreparedDocument::FallbackText { page, reason } =
        rich_documents::prepare(&path, &Cancellation::default())?
    else {
        panic!("expected bounded fallback")
    };
    assert_eq!(reason, rich_documents::LayoutFallback::PreviewLimit);
    assert!(page.text.contains("Readable"));
    Ok(())
}

#[test]
fn oversized_embedded_png_reserves_pixel_memory_before_layout() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("large-image.docx");
    let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    header.extend(12_000u32.to_be_bytes());
    header.extend(16_000u32.to_be_bytes());
    zip(
        &path,
        &[
            (
                "word/document.xml",
                b"<document><body><p><t>Caption</t></p></body></document>",
            ),
            ("word/media/picture.png", &header),
        ],
    )?;
    let PreparedDocument::FallbackText { page, reason } =
        rich_documents::prepare(&path, &Cancellation::default())?
    else {
        panic!("expected a bounded preview")
    };
    assert_eq!(reason, rich_documents::LayoutFallback::PreviewLimit);
    assert!(page.text.contains("Caption"));
    Ok(())
}

#[test]
fn legacy_iwork_packages_show_the_embedded_preview() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cancel = Cancellation::default();
    let file = dir.path().join("old.pages");
    let mut word = lo_writer::WriterEditor::new("");
    word.push_paragraph("Embedded layout");
    let pdf = lo_writer::save_as(&word.document, "pdf")?;
    zip(&file, &[("QuickLook/Preview.pdf", pdf.as_slice())])?;
    let PreparedDocument::Pages(page) = rich_documents::prepare(&file, &cancel)? else {
        panic!("expected native preview")
    };
    assert_eq!(fs::read(page)?, pdf);
    let bundle = dir.path().join("old.key");
    fs::create_dir_all(bundle.join("QuickLook"))?;
    image::RgbImage::from_pixel(12, 8, image::Rgb([17, 130, 210]))
        .save(bundle.join("QuickLook/Thumbnail.jpg"))?;
    let PreparedDocument::Pages(page) = rich_documents::prepare(&bundle, &cancel)? else {
        panic!("expected cover")
    };
    let pdf = superfile_services::pdf::PdfDocument::open(&page, &cancel)?;
    assert_eq!(pdf.page_count(), 1);
    assert!((pdf.page_ratio(0).unwrap() - 1.5).abs() < 0.001);
    Ok(())
}
