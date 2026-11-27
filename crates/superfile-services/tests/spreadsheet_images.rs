use anyhow::Result;
use std::{fs::File, io::Write, path::Path};
use superfile_services::{
    locale,
    spreadsheet::{cell_images_for_table, prepare},
    work::Cancellation,
};

// This integration binary owns its locale so switching it cannot race other tests.
#[test]
fn workbook_columns_and_images_are_language_independent() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("with_images.xlsx");
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 120, 220, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;

    let mut zip = zip::ZipWriter::new(File::create(&path)?);
    let opts = zip::write::SimpleFileOptions::default();
    let mut add = |name: &str, bytes: &[u8]| -> Result<()> {
        zip.start_file(name, opts)?;
        zip.write_all(bytes)?;
        Ok(())
    };
    add(
        "[Content_Types].xml",
        br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/></Types>"#,
    )?;
    add(
        "_rels/.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="wb" Target="xl/workbook.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"/></Relationships>"#,
    )?;
    add(
        "xl/workbook.xml",
        br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="s1"/></sheets></workbook>"#,
    )?;
    add(
        "xl/_rels/workbook.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s1" Target="worksheets/sheet1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"/></Relationships>"#,
    )?;
    add(
        "xl/worksheets/sheet1.xml",
        br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Text</t></is></c><c r="B1"><f>DISPIMG("ID_IMG1",1)</f></c></row></sheetData><drawing r:id="d1"/></worksheet>"#,
    )?;
    add(
        "xl/worksheets/_rels/sheet1.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="d1" Target="../drawings/drawing1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing"/></Relationships>"#,
    )?;
    add(
        "xl/drawings/drawing1.xml",
        br#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:row>1</xdr:row></xdr:from><xdr:pic><xdr:blipFill><a:blip r:embed="rImg2"/></xdr:blipFill></xdr:pic></xdr:twoCellAnchor></xdr:wsDr>"#,
    )?;
    add(
        "xl/drawings/_rels/drawing1.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rImg2" Target="../media/image2.png" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"/></Relationships>"#,
    )?;
    add(
        "xl/cellimages.xml",
        br#"<etc:cellImages xmlns:etc="http://www.wps.cn/officeDocument/2017/etCustomData" xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><etc:cellImage><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="1" name="ID_IMG1"/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rImg1"/></xdr:blipFill></xdr:pic></etc:cellImage></etc:cellImages>"#,
    )?;
    add(
        "xl/_rels/cellimages.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rImg1" Target="media/image1.png" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"/></Relationships>"#,
    )?;
    add("xl/media/image1.png", &png)?;
    add("xl/media/image2.png", &png)?;
    zip.finish()?;

    let xlsm = dir.path().join("with_images.xlsm");
    std::fs::copy(&path, &xlsm)?;
    for language in ["zh-CN", "en"] {
        locale::set(language);
        for path in [&xlsm, &path] {
            check_workbook(path)?;
        }
    }
    Ok(())
}

fn check_workbook(path: &Path) -> Result<()> {
    let book = prepare(path, &Cancellation::default())?.expect("workbook prepared");
    let db = rusqlite::Connection::open(&book.database)?;
    let anchored = db.query_row(
        "SELECT Cell, Value, Formula FROM \"1: Sheet1\" WHERE Cell = 'B2'",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    assert_eq!(anchored, ("B2".into(), String::new(), String::new()));
    let images_db = book.images_database.as_ref().expect("images db created");
    let resolved = cell_images_for_table(
        images_db,
        "1: Sheet1",
        &["A1", "B1", "B2"],
        &Cancellation::default(),
    )?;
    assert!(!resolved.contains_key("A1"));
    assert!(resolved["B1"].to_file_path().unwrap().is_file());
    assert!(resolved["B2"].to_file_path().unwrap().is_file());
    Ok(())
}
