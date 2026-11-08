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
        rich_documents::prepare(&docx, &Cancellation::default())?
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
    let PreparedDocument::Text { page, .. } =
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
