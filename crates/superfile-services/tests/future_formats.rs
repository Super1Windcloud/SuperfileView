use superfile_services::{formats, text_formats, vector_formats};
#[test]
fn container_previews_reject_damage_and_do_not_extract() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paths.zip");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    archive
        .start_file("../outside.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"untrusted").unwrap();
    archive.finish().unwrap();
    let before = std::fs::read(&path).unwrap();
    let preview = formats::read_preview(&path).unwrap();
    assert!(
        preview
            .text
            .contains(&superfile_services::locale::tr(" · Unsafe extraction path"))
    );
    assert_eq!(before, std::fs::read(&path).unwrap());
    assert!(!dir.path().parent().unwrap().join("outside.txt").exists());
    std::fs::write(&path, b"PK\x03\x04broken").unwrap();
    assert!(formats::read_preview(&path).is_err());
}
#[test]
fn documents_and_structured_data_keep_source_separate() {
    let source = "= 标题\n\n[source,python]\n----\nprint('test')\n----\n";
    let preview = text_formats::markdown(std::path::Path::new("test.adoc"), source);
    assert!(preview.starts_with("# 标题"));
    assert!(preview.contains("```\nprint('test')\n```"));
    assert!(superfile_services::json::parse_format("{name:'demo',hex:0x10,}", "json5").is_ok());
    let nodes = superfile_services::json::parse_format("{\"a\":1}\n{\"a\":2}\n", "ndjson").unwrap();
    assert_eq!(nodes[0].children.len(), 2);
    assert!(superfile_services::json::parse_format("{}\ninvalid", "ndjson").is_err());
    assert_eq!(
        superfile_services::csv::read_page(
            "a\tb\n1\t\"two\tthree\"".as_bytes(),
            b'\t',
            0,
            0,
            &superfile_services::work::Cancellation::default()
        )
        .unwrap()
        .rows[1][1],
        "two\tthree"
    );
    let coordinates = text_formats::markdown(
        std::path::Path::new("test.gpx"),
        "<gpx><wpt lat=\"39.9\" lon=\"116.4\"/></gpx>",
    );
    assert!(coordinates.contains("| 116.4 | 39.9 |"));
}
#[test]
fn malformed_mesh_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.obj");
    std::fs::write(&path, "v 0 0 0\nf 1 2 3").unwrap();
    assert!(vector_formats::read_preview(&path).is_err());
}
#[test]
fn numpy_values_match_the_declared_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("array.npy");
    let header = "{'descr': '<f8', 'fortran_order': False, 'shape': (1, 2), }\n";
    let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
    bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(&1.5_f64.to_le_bytes());
    bytes.extend_from_slice(&2.5_f64.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    assert!(
        formats::read_preview(&path)
            .unwrap()
            .text
            .contains("[1] 2.5")
    );
    bytes.truncate(bytes.len() - 8);
    std::fs::write(&path, bytes).unwrap();
    assert!(formats::read_preview(&path).is_err());
}
#[test]
fn optional_exploration_corpus_is_read_without_mutation() {
    let Some(root) = std::env::var_os("SUPERFILE_FORMAT_FIXTURES") else {
        return;
    };
    fn visit(path: &std::path::Path, count: &mut usize) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, count);
                continue;
            }
            if path.file_name().unwrap() == "README.md" {
                continue;
            }
            let before = std::fs::read(&path).unwrap();
            if path.file_name().unwrap() == "截断ZIP.zip" {
                assert!(formats::read_preview(&path).is_err());
            } else if formats::supports(&path) {
                assert!(!formats::read_preview(&path).unwrap().text.is_empty());
            } else if vector_formats::supports(&path) {
                assert!(!vector_formats::read_preview(&path).unwrap().png.is_empty());
            } else if superfile_services::media::supports(&path) {
                assert!(
                    !superfile_services::media::read_image(&path)
                        .unwrap()
                        .png
                        .is_empty()
                );
            } else {
                let document = superfile_services::files::read_document(&path).unwrap();
                if text_formats::supports(&path) {
                    assert!(!text_formats::markdown(&path, &document.text).is_empty());
                }
                if path
                    .extension()
                    .is_some_and(|e| e == "json5" || e == "ndjson")
                {
                    superfile_services::json::parse_format(
                        &document.text,
                        path.extension().unwrap().to_str().unwrap(),
                    )
                    .unwrap();
                }
            }
            assert_eq!(before, std::fs::read(&path).unwrap(), "{}", path.display());
            *count += 1;
        }
    }
    let mut count = 0;
    visit(std::path::Path::new(&root), &mut count);
    assert_eq!(count, 35);
}

#[test]
fn document_projection_retains_content_and_keeps_static_notebook_outputs() {
    use std::path::Path;
    let adoc = text_formats::project(
        Path::new("demo.adoc"),
        "= Example\n\n*strong* and _emphasis_",
    );
    assert!(adoc.contains("<strong>strong</strong>") && adoc.contains("<em>emphasis</em>"));
    let org = text_formats::project(Path::new("demo.org"), "* Heading\n\n*bold*");
    assert!(
        org.contains("Heading")
            && (org.contains("<b>bold</b>") || org.contains("<strong>bold</strong>"))
    );
    let cells = serde_json::json!({"metadata":{"language_info":{"name":"python"}},"cells":[{"cell_type":"markdown","source":["# Heading"]},{"cell_type":"code","source":["print(1)"],"outputs":[{"text":["1\n"]},{"data":{"text/html":"<script>unexpected()</script>","text/plain":"plain"}},{"data":{"image/png":"AA=="}}]}]});
    let notebook = text_formats::project(Path::new("demo.ipynb"), &cells.to_string());
    for part in [
        "# Heading",
        "\x60\x60\x60python",
        "print(1)",
        "plain",
        "data:image/png;base64,AA==",
    ] {
        assert!(notebook.contains(part), "{part}");
    }
    assert!(!notebook.contains("<script>"));
    for (path, source) in [
        ("demo.srt", "1\n00:00:01,000 --> 00:00:02,000\ncaption\n\n"),
        (
            "demo.vtt",
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\ncaption\n\n",
        ),
    ] {
        let projected = text_formats::project(Path::new(path), source);
        assert!(projected.contains("caption") && projected.contains("00:00:02"));
    }
}

#[test]
fn numpy_big_endian_signed_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("big.npy");
    let header = "{'descr': '>i2', 'fortran_order': False, 'shape': (2,), }\n";
    let mut file = b"\x93NUMPY\x01\x00".to_vec();
    file.extend_from_slice(&(header.len() as u16).to_le_bytes());
    file.extend_from_slice(header.as_bytes());
    file.extend_from_slice(&(-235_i16).to_be_bytes());
    file.extend_from_slice(&372_i16.to_be_bytes());
    std::fs::write(&path, file).unwrap();
    let result = formats::read_preview(&path).unwrap().text;
    assert!(
        result.contains("-235") && result.contains("372"),
        "{result}"
    );
}
