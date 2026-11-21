use anyhow::Result;
use std::io::{Cursor, Write};

fn document(body: &str) -> Result<Vec<u8>> {
    let mut file = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    file.start_file("word/document.xml", options)?;
    write!(
        file,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}</w:body></w:document>"#
    )?;
    file.start_file("word/_rels/document.xml.rels", options)?;
    file.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="photo" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/sample.png"/></Relationships>"#)?;
    file.start_file("word/media/sample.png", options)?;
    let mut png = Cursor::new(Vec::new());
    image::RgbImage::from_pixel(12, 12, image::Rgb([190, 15, 65]))
        .write_to(&mut png, image::ImageFormat::Png)?;
    file.write_all(png.get_ref())?;
    Ok(file.finish()?.into_inner())
}

fn picture(width: u32, height: u32) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="{}" cy="{}"/><a:graphic><a:blip r:embed="photo"/></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        width * 12700,
        height * 12700
    )
}

const LANDSCAPE: &str = r#"<w:sectPr><w:pgSz w:w="23820" w:h="16905" w:orient="landscape"/><w:pgMar w:left="1080" w:right="1440" w:top="720" w:bottom="1800"/></w:sectPr>"#;

#[test]
fn word_keeps_section_geometry_and_fits_figures_proportionally() -> Result<()> {
    let source = document(&format!(
        "{}{}{}{LANDSCAPE}",
        picture(1800, 60),
        picture(10, 7),
        picture(10, 1800)
    ))?;
    let word = lo_writer::load_bytes("", &source, "docx")?;
    let copy = lo_writer::save_as(&word, "docx")?;
    assert_eq!(
        lo_writer::load_bytes("", &copy, "docx")?.page_style.section,
        word.page_style.section
    );
    let pdf = lo_writer::save_as(&word, "pdf")?;
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/MediaBox [0 0 1191.00 845.25]"));
    let commands: Vec<_> = text.lines().collect();
    let transforms: Vec<Vec<f32>> = commands
        .windows(2)
        .filter(|lines| lines[0].ends_with(" cm") && lines[1].ends_with(" Do"))
        .map(|lines| {
            lines[0]
                .split_whitespace()
                .take(6)
                .map(|s| s.parse::<f32>().unwrap())
                .collect()
        })
        .collect();
    assert_eq!(transforms.len(), 3);
    let near = |value: f32, expected: f32| {
        assert!(
            (value - expected).abs() < 0.02,
            "{value} expected {expected}"
        )
    };
    let [wide, small, tall] = transforms.as_slice() else {
        unreachable!()
    };
    near(wide[0], 1065.);
    near(wide[3], 35.5);
    near(wide[4], 54.);
    near(wide[5], 773.75);
    near(small[0], 10.);
    near(small[3], 7.);
    near(tall[0], 701.25 / 180.);
    near(tall[3], 701.25);
    near(tall[5], 108.);
    Ok(())
}

#[test]
fn malformed_and_multiple_word_sections_leave_a_readable_page() -> Result<()> {
    let invalid = r#"<w:sectPr><w:pgSz w:w="-1" w:h="999999999999999999"/><w:pgMar w:left="65000" w:right="65000" w:top="-720" w:bottom="0"/></w:sectPr>"#;
    let doc = lo_writer::load_bytes("", &document(invalid)?, "docx")?;
    let section = doc.page_style.section.unwrap();
    assert_eq!((section.width, section.height), (11906, 16838));
    assert_eq!((section.left, section.right), (1134, 1134));
    assert_eq!((section.top, section.bottom), (720, 0));
    let sections = format!(
        r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr></w:pPr></w:p>{LANDSCAPE}"#
    );
    let mixed = lo_writer::load_bytes("", &document(&sections)?, "docx")?;
    assert!(mixed.page_style.section.is_none());
    Ok(())
}
