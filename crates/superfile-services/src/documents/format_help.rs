//! User-facing capabilities for the current reader routes, without probing files.
use super::presentation::{MarkupFormat, TextPresentation};
use std::path::Path;
use superfile_core::formats::{CATALOG_SUFFIXES, FormatSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preview {
    Text,
    Markdown,
    Html,
    ConvertedMarkup,
    Structured,
    Table,
    Image,
    Pdf,
    OfficeText,
    Workbook,
    Book,
    MindMap,
    Outline,
    Diagram,
    Vector,
    Archive,
    Container,
    Sqlite,
    BinaryPlist,
    BinaryReport,
    Executable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editing {
    Source,
    ReadOnly,
    TextOnly,
}

pub struct FormatHelp {
    pub name: String,
    pub preview: Preview,
    pub editing: Editing,
}

pub fn formats() -> Vec<FormatHelp> {
    let compound_archives = [
        "tar.gz", "tar.bz2", "tar.xz", "tar.zst", "tar.zstd", "tar.lz4",
    ];
    let mut suffixes = CATALOG_SUFFIXES.to_vec();
    suffixes.extend(compound_archives);
    suffixes.sort_unstable();
    suffixes.dedup();
    let mut entries = suffixes
        .into_iter()
        .filter_map(|suffix| {
            let sample = format!("sample.{suffix}");
            let (preview, editing) = capability(Path::new(&sample))?;
            Some(FormatHelp {
                name: format!(".{suffix}"),
                preview,
                editing,
            })
        })
        .collect::<Vec<_>>();
    for name in [
        "Dockerfile",
        "Containerfile",
        "Makefile",
        "CMakeLists.txt",
        "Gemfile",
        ".gitignore",
    ] {
        entries.push(FormatHelp {
            name: name.into(),
            preview: Preview::Text,
            editing: Editing::Source,
        });
    }
    for name in [".exe", ".dll", ".so", ".dylib", "PE / ELF / Mach-O"] {
        entries.push(FormatHelp {
            name: name.into(),
            preview: Preview::Executable,
            editing: Editing::ReadOnly,
        });
    }
    entries.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());
    entries
}

fn capability(path: &Path) -> Option<(Preview, Editing)> {
    let extension = path.extension()?.to_str()?;
    let read_only = |preview| Some((preview, Editing::ReadOnly));
    // Match the order of overlapping reader routes, especially FB2.ZIP,
    // raw archives and OpenDocument containers.
    if FormatSet::BinaryPlist.matches(path) {
        Some((Preview::BinaryPlist, Editing::TextOnly))
    } else if FormatSet::Opml.matches(path) {
        read_only(Preview::Outline)
    } else if FormatSet::Drawio.matches(path) {
        read_only(Preview::Diagram)
    } else if FormatSet::Book.matches(path) {
        read_only(Preview::Book)
    } else if FormatSet::Xmind.matches(path) {
        read_only(Preview::MindMap)
    } else if FormatSet::Vector.matches(path) {
        read_only(Preview::Vector)
    } else if FormatSet::Archive.matches(path) {
        crate::archive::can_preview_extension(extension)
            .then_some((Preview::Archive, Editing::ReadOnly))
    } else if FormatSet::ContainerReport.matches(path) {
        read_only(if extension == "npy" {
            Preview::BinaryReport
        } else {
            Preview::Container
        })
    } else if FormatSet::Sqlite.matches(path) {
        read_only(Preview::Sqlite)
    } else if FormatSet::Safetensors.matches(path) {
        read_only(Preview::BinaryReport)
    } else if FormatSet::Image.matches(path) {
        read_only(Preview::Image)
    } else if FormatSet::Pdf.matches(path) {
        read_only(Preview::Pdf)
    } else if crate::rich_documents::is_workbook_extension(extension) {
        read_only(Preview::Workbook)
    } else if crate::rich_documents::is_text_document_extension(extension) {
        read_only(Preview::OfficeText)
    } else if crate::preview::supports(path) {
        // Recognition alone does not mean the portable reader can open it.
        None
    } else {
        let presentation = TextPresentation::for_path(path);
        let preview = if presentation.table.is_some() {
            Preview::Table
        } else {
            match presentation.markup {
                Some(MarkupFormat::Markdown) => Preview::Markdown,
                Some(MarkupFormat::Html) => Preview::Html,
                Some(MarkupFormat::Converted) => Preview::ConvertedMarkup,
                None if presentation.structured => Preview::Structured,
                None => Preview::Text,
            }
        };
        Some((preview, Editing::Source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_distinguishes_live_readers_from_recognition_only_routes() {
        let entries = formats();
        for (name, preview, editing) in [
            (".md", Preview::Markdown, Editing::Source),
            (".csv", Preview::Table, Editing::Source),
            (".rs", Preview::Text, Editing::Source),
            (".pdf", Preview::Pdf, Editing::ReadOnly),
            (".svg", Preview::Image, Editing::ReadOnly),
            (".fb2.zip", Preview::Book, Editing::ReadOnly),
            (".tar.gz", Preview::Archive, Editing::ReadOnly),
            (".docx", Preview::OfficeText, Editing::ReadOnly),
            (".ods", Preview::Container, Editing::ReadOnly),
            (".xlsx", Preview::Workbook, Editing::ReadOnly),
            (".plist", Preview::BinaryPlist, Editing::TextOnly),
        ] {
            let entry = entries.iter().find(|entry| entry.name == name).unwrap();
            assert_eq!((entry.preview, entry.editing), (preview, editing), "{name}");
        }
        for name in [".doc", ".7z", ".mp4", ".ttf"] {
            assert!(!entries.iter().any(|entry| entry.name == name), "{name}");
        }
    }
}
