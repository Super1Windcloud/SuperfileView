use anyhow::Result;
use image::{Rgb, RgbImage};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
use superfile_services::{
    pdf::PdfDocument,
    rich_documents::{self, PreparedDocument},
    work::Cancellation,
};

fn drawing(id: &str, width: u32, height: u32) -> String {
    format!(
        r#"<w:drawing><wp:inline><wp:extent cx="{}" cy="{}"/><wp:docPr descr="{id}"/><a:graphic><a:blip r:embed="{id}"/></a:graphic></wp:inline></w:drawing>"#,
        width * 12700,
        height * 12700
    )
}

fn write_word(path: &Path, cells: &[String]) -> Result<()> {
    let mut zip = zip::ZipWriter::new(fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("word/document.xml", options)?;
    let grid = if cells.len() == 1 {
        r#"<w:gridCol w:w="7200"/>"#
    } else {
        r#"<w:gridCol w:w="2400"/><w:gridCol w:w="4800"/>"#
    };
    let cells: String = cells
        .iter()
        .map(|content| format!("<w:tc>{content}</w:tc>"))
        .collect();
    write!(
        zip,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body><w:tbl><w:tblGrid>{grid}</w:tblGrid><w:tr>{cells}</w:tr></w:tbl><w:sectPr><w:pgSz w:w="8000" w:h="6400"/><w:pgMar w:top="400" w:bottom="400" w:left="400" w:right="400"/></w:sectPr></w:body></w:document>"#
    )?;
    zip.start_file("word/_rels/document.xml.rels", options)?;
    zip.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="red" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/red.png"/><Relationship Id="blue" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/blue.png"/></Relationships>"#)?;
    for (name, color) in [("red", [218, 21, 47]), ("blue", [30, 90, 204])] {
        zip.start_file(format!("word/media/{name}.png"), options)?;
        let mut png = Cursor::new(Vec::new());
        RgbImage::from_pixel(16, 16, Rgb(color)).write_to(&mut png, image::ImageFormat::Png)?;
        zip.write_all(png.get_ref())?;
    }
    zip.finish()?;
    Ok(())
}

fn render(path: &Path) -> Result<(Vec<u8>, RgbImage)> {
    let cancel = Cancellation::default();
    let PreparedDocument::Word(word) = rich_documents::prepare(path, &cancel)? else {
        anyhow::bail!("expected continuous Word content")
    };
    assert!(matches!(
        word.blocks.as_slice(),
        [rich_documents::docx::model::Block::Table(_)]
    ));
    let source = lo_writer::from_docx_bytes("", &fs::read(path)?)?;
    let bytes = lo_writer::save_as(&source, "pdf")?;
    let mut pdf = tempfile::NamedTempFile::new()?;
    pdf.write_all(&bytes)?;
    pdf.flush()?;
    let doc = PdfDocument::open(pdf.path(), &cancel)?;
    assert_eq!(doc.page_count(), 1);
    let page = doc.render(0, &cancel)?;
    Ok((bytes, image::load_from_memory(&page.png)?.to_rgb8()))
}

// Measure each solid photo's bounds in the Writer PDF export.
fn bounds(frame: &RgbImage, color: [u8; 3]) -> [f32; 4] {
    let mut min_x = frame.width();
    let mut min_y = frame.height();
    let mut max_x = 0;
    let mut max_y = 0;
    for (x, y, pixel) in frame.enumerate_pixels() {
        if pixel.0.iter().zip(color).all(|(a, b)| a.abs_diff(b) <= 2) {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    assert!(max_x > min_x && max_y > min_y, "missing picture {color:?}");
    let x_scale = 400. / frame.width() as f32;
    let y_scale = 320. / frame.height() as f32;
    [
        min_x as f32 * x_scale,
        min_y as f32 * y_scale,
        (max_x + 1) as f32 * x_scale,
        (max_y + 1) as f32 * y_scale,
    ]
}

#[test]
fn mixed_word_runs_keep_each_photo_with_its_cell_and_text() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("mixed.docx");
    let red = drawing("red", 40, 80);
    let blue = drawing("blue", 480, 120);
    // Both branches depict the same right-hand image, with different ids in real packages.
    let blue = format!(
        "<mc:AlternateContent><mc:Choice>{blue}</mc:Choice><mc:Fallback>{blue}</mc:Fallback></mc:AlternateContent>"
    );
    write_word(
        &path,
        &[
            format!("<w:p><w:r><w:t>Before</w:t>{red}<w:t>After</w:t></w:r></w:p>"),
            format!("<w:p><w:r>{blue}</w:r></w:p>"),
        ],
    )?;
    let word = lo_writer::load_bytes("", &fs::read(&path)?, "docx")?;
    assert_eq!(word.embedded_images().len(), 2);
    let plain = word.plain_text();
    assert!(plain.find("Before").unwrap() < plain.find("[image: red]").unwrap());
    assert!(plain.find("[image: red]").unwrap() < plain.find("After").unwrap());
    let (pdf, frame) = render(&path)?;
    let commands = String::from_utf8_lossy(&pdf);
    let photos: Vec<_> = commands
        .match_indices(" Do\n")
        .map(|(index, _)| index)
        .collect();
    assert_eq!(photos.len(), 2);
    assert!(commands.find("(Before)").unwrap() < photos[0]);
    assert!(photos[0] < commands.find("(After)").unwrap());
    let [x1, y1, x2, y2] = bounds(&frame, [218, 21, 47]);
    assert!(
        x1 >= 20. && x2 <= 140. && (x2 - x1 - 40.).abs() < 2. && (y2 - y1 - 80.).abs() < 2.,
        "{x1} {y1} {x2} {y2}"
    );
    let [x1, y1, x2, y2] = bounds(&frame, [30, 90, 204]);
    assert!(
        x1 >= 140. && x2 <= 380. && (x2 - x1 - 228.).abs() < 2. && (y2 - y1 - 57.).abs() < 2.,
        "{x1} {y1} {x2} {y2}"
    );
    Ok(())
}

#[test]
fn word_picture_only_cell_survives_admission_and_missing_relationship() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("photo.docx");
    let red = drawing("red", 30, 900);
    let missing = drawing("missing", 80, 48);
    write_word(&path, &[format!("<w:p><w:r>{missing}{red}</w:r></w:p>")])?;
    let word = lo_writer::load_bytes("", &fs::read(&path)?, "docx")?;
    assert_eq!(word.embedded_images().len(), 1);
    assert!(word.plain_text().contains("[image: missing]"));
    assert!(word.plain_text().contains("[image: red]"));
    let (pdf, frame) = render(&path)?;
    assert!(String::from_utf8_lossy(&pdf).contains("[image: missing]"));
    let [x1, y1, x2, y2] = bounds(&frame, [218, 21, 47]);
    assert!(y1 > 30. && y2 <= 300. && y2 - y1 > 220.);
    assert!(((y2 - y1) / (x2 - x1) - 30.).abs() < 3.);
    Ok(())
}
