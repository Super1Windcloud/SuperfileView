//! Exercise both CIDFont types through the Office converter and the PDF reader.
use anyhow::Result;
use std::{fs, path::Path, process::Command};
use superfile_services::{pdf::PdfDocument, work::Cancellation};

const FONT_OVERRIDE: &str = "LIBREOFFICE_PURE_UNICODE_FONT";
const CHILD: &str = "SUPERFILE_TEST_OFFICE_FONT_SCENARIO";

#[test]
fn shared_font_outlines_preserve_each_unicode_scalar() -> Result<()> {
    // fontdb reads its override from the process environment. Run each case in
    // a separate test process so other readers can use their own system fonts.
    if let Some(scenario) = std::env::var_os(CHILD) {
        return verify(scenario == "last-resort");
    }
    let dir = tempfile::tempdir()?;
    let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/lo_core/tests/fonts");
    let flagged = dir.path().join("unrelated-family-name.ttf");
    let mut data = fs::read(fonts.join("shared-outline.ttf"))?;
    let count = u16::from_be_bytes(data[4..6].try_into()?) as usize;
    let head = (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&i| &data[i..i + 4] == b"head")
        .unwrap();
    let offset = u32::from_be_bytes(data[head + 8..head + 12].try_into()?) as usize;
    data[offset + 16] |= 0x40; // OpenType head.flags bit 14, independently of the face name.
    fs::write(&flagged, data)?;
    for (name, path) in [
        ("ttf", fonts.join("shared-outline.ttf")),
        ("cff", fonts.join("shared-outline.otf")),
        ("last-resort", flagged),
    ] {
        let output = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "shared_font_outlines_preserve_each_unicode_scalar",
                "--nocapture",
            ])
            .env(CHILD, name)
            .env(FONT_OVERRIDE, path)
            .output()?;
        assert!(
            output.status.success(),
            "{name}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn verify(last_resort: bool) -> Result<()> {
    // The independently authored TTF and CFF fixtures share the triangular
    // outline for A and Greek alpha; black square has a different outline.
    const TEXT: &str = "AΑ■\u{fe0f}\u{10fffd}";
    let mut doc = lo_writer::WriterEditor::new("");
    doc.push_paragraph(TEXT);
    let bytes = lo_writer::save_as(&doc.document, "pdf")?;
    let extracted = lo_writer::from_pdf_bytes("", &bytes)?.plain_text();
    assert!(extracted.contains(TEXT), "extracted {extracted:?}");
    let names = String::from_utf8_lossy(&bytes);
    assert!(names.contains("SuperfileMissingGlyph"));
    assert_eq!(names.contains("SuperfileAlias"), !last_resort);
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("sample.pdf");
    fs::write(&path, bytes)?;
    let cancel = Cancellation::default();
    let rendered = PdfDocument::open(&path, &cancel)?.render(0, &cancel)?;
    let image = image::load_from_memory(&rendered.png)?.to_luma8();
    let mut runs = Vec::new();
    let mut ink = 0;
    for x in 0..image.width() {
        let count = (0..image.height())
            .filter(|&y| image.get_pixel(x, y)[0] < 128)
            .count();
        if count == 0 && ink > 0 {
            runs.push(ink);
            ink = 0;
        } else {
            ink += count;
        }
    }
    assert_eq!(runs.len(), 4, "unexpected rendered glyphs {runs:?}");
    if !last_resort {
        assert!(
            runs[0].abs_diff(runs[1]) <= runs[0] / 5,
            "aliased triangles changed shape: {runs:?}"
        );
        assert!(
            runs[2] > runs[0] * 11 / 10,
            "missing distinctive square glyph: {runs:?}"
        );
    }
    Ok(())
}
