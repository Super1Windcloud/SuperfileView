use anyhow::Result;
use std::io::{Cursor, Write};

fn layout(body: &str) -> Result<Vec<u8>> {
    let mut file = zip::ZipWriter::new(Cursor::new(Vec::new()));
    file.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )?;
    write!(
        file,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    )?;
    let doc = lo_writer::from_docx_bytes("", &file.finish()?.into_inner())?;
    Ok(lo_writer::try_to_pdf(&doc)?)
}

fn rectangles(pdf: &str) -> Vec<Vec<f32>> {
    pdf.lines()
        .filter_map(|line| line.strip_suffix(" re S"))
        .map(|line| {
            line.split_ascii_whitespace()
                .map(|part| part.parse().unwrap())
                .collect()
        })
        .collect()
}

#[test]
fn docx_uses_the_authored_grid_merge_text_style_and_fill() -> Result<()> {
    let pdf = layout(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="700"/><w:gridCol w:w="1400"/><w:gridCol w:w="2100"/></w:tblGrid>
        <w:tr><w:tc><w:tcPr><w:gridSpan w:val="3"/></w:tcPr><w:p><w:pPr><w:jc w:val="center"/></w:pPr>
        <w:r><w:rPr><w:b/><w:shd w:fill="FFFF00"/><w:sz w:val="28"/><w:color w:val="336699"/></w:rPr><w:t>Title</w:t></w:r></w:p></w:tc></w:tr>
        <w:tr><w:tc><w:p/></w:tc><w:tc><w:tcPr><w:shd w:fill="FF0000"/></w:tcPr><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr>
        <w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
    )?;
    let text = String::from_utf8_lossy(&pdf);
    let rects = rectangles(&text);
    assert_eq!(
        rects.iter().map(|r| r[2]).collect::<Vec<_>>(),
        [210., 35., 70., 105., 35., 70., 105.]
    );
    assert!((rects[2][0] - rects[0][0] - 35.).abs() < 0.01);
    assert!((rects[3][0] - rects[0][0] - 105.).abs() < 0.01);
    assert_eq!(
        text.lines().filter(|line| line.ends_with(" re f")).count(),
        2
    );
    assert!(text.contains("1.000 0.000 0.000 rg"));
    assert!(text.contains("1.000 1.000 0.000 rg"));
    assert!(text.contains("/Helvetica-Bold"));
    assert!(text.contains("0.200 0.400 0.600 rg"));
    let title = text.split_once("(Title) Tj").unwrap().0;
    assert!(
        title
            .lines()
            .rev()
            .take(2)
            .any(|line| line.contains("14 Tf"))
    );
    let title_x: f32 = title
        .lines()
        .last()
        .unwrap()
        .split_ascii_whitespace()
        .nth(4)
        .unwrap()
        .parse()?;
    assert!((title_x - rects[0][0] - 90.).abs() < 8.);
    Ok(())
}

#[test]
fn body_paragraphs_keep_the_same_authored_text_styles_as_cells() -> Result<()> {
    let pdf = layout(
        r#"<w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="38"/><w:color w:val="336699"/><w:shd w:fill="FFF5B8"/></w:rPr><w:t>Body</w:t></w:r></w:p>"#,
    )?;
    let pdf = String::from_utf8_lossy(&pdf);
    let before_text = pdf.split_once("(Body) Tj").unwrap().0;
    assert!(before_text.contains("1.000 0.961 0.722 rg"));
    assert!(before_text.contains("0.200 0.400 0.600 rg"));
    assert!(before_text.contains("/F2 19 Tf"));
    let position = before_text.lines().last().unwrap();
    let x: f32 = position.split_whitespace().nth(4).unwrap().parse()?;
    assert!(x > 400.0, "expected a right-aligned paragraph, got {x}");
    Ok(())
}

#[test]
fn excessive_grid_span_is_rejected_and_narrow_content_area_is_supported() -> Result<()> {
    let bad = "<w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val=\"4294967295\"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>";
    assert!(
        layout(bad)
            .unwrap_err()
            .to_string()
            .contains("grid exceeds")
    );
    let narrow = layout(
        r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Tiny</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="4000" w:h="6400"/><w:pgMar w:left="1520" w:right="1520" w:top="300" w:bottom="300"/></w:sectPr>"#,
    )?;
    let rects = rectangles(&String::from_utf8_lossy(&narrow));
    assert_eq!(rects.len(), 1);
    assert_eq!(rects[0][2], 48.);
    Ok(())
}
