//! Local diagnostic event metadata. No remote configuration, identifiers, worker or transport.
use anyhow::Result;
use std::{path::Path, time::Instant};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Consent {
    pub usage: bool,
    pub diagnostics: bool,
}
pub fn environment() -> &'static str {
    "disabled"
}
pub fn is_debug() -> bool {
    true
}
pub struct Property(serde_json::Value);
pub fn text(value: &'static str) -> Property {
    Property(value.into())
}
pub fn number(value: u64) -> Property {
    Property(value.into())
}
pub fn flag(value: bool) -> Property {
    Property(value.into())
}
pub fn track(name: &'static str, props: impl IntoIterator<Item = (&'static str, Property)>) {
    if log::log_enabled!(log::Level::Trace) {
        let values = props
            .into_iter()
            .map(|(key, value)| (key, value.0))
            .collect::<std::collections::BTreeMap<_, _>>();
        log::trace!("event={name} properties={}", serde_json::json!(values));
    }
}
pub fn operation_failed(operation: &'static str, _path: Option<&Path>, error: &anyhow::Error) {
    crate::diagnostics::event("ERROR", &format!("operation={operation} error={error:#}"));
}
pub fn file_properties(path: &Path) -> Vec<(&'static str, Property)> {
    const EXTENSIONS: &str = "md markdown mdx txt log csv tsv json jsonc json5 jsonl yaml yml toml xml html htm css scss less js jsx mjs cjs ts tsx rs py pyi go java kt kts c cc cpp h hpp cs swift m mm rb php sh bash zsh fish sql ini conf env gitignore make cmake dockerfile vue svelte astro dart lua r jl scala ex exs erl hs clj lisp nim zig nix tf proto graphql ipynb pdf doc docx rtf odt xls xlsx xlsm ods numbers ppt pptx odp key pages epub fb2 cbz png jpg jpeg gif svg webp avif heic tif tiff bmp ico icns raw exr hdr qoi mp3 m4a aac wav aiff flac ogg mp4 mov mkv m4v webm zip tar gz bz2 xz zst 7z rar dmg iso db db3 sqlite sqlite3 opml xmind drawio dot gv mermaid mmd d2 plantuml puml pikchr vega wavedrom tex otf ttf ttc woff woff2 obj stl bdf npy safetensors plist";
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let named = match name.as_str() {
        "dockerfile" | "containerfile" => Some("dockerfile"),
        "makefile" | "gnumakefile" => Some("make"),
        "cmakelists.txt" => Some("cmake"),
        ".gitignore" | ".dockerignore" | ".ignore" => Some("gitignore"),
        ".env" => Some("env"),
        _ if name.starts_with(".env.") => Some("env"),
        _ => None,
    };
    let suffix = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = named.unwrap_or_else(|| {
        EXTENSIONS
            .split_whitespace()
            .find(|candidate| *candidate == suffix)
            .unwrap_or("other")
    });
    let family = match extension {
        "md" | "markdown" | "mdx" => "markdown",
        "pdf" => "pdf",
        "doc" | "docx" | "rtf" | "odt" | "xls" | "xlsx" | "xlsm" | "ods" | "numbers" | "ppt"
        | "pptx" | "odp" | "key" | "pages" => "office",
        "epub" | "fb2" | "cbz" => "ebook",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "avif" | "heic" | "tif" | "tiff"
        | "bmp" | "ico" | "icns" | "raw" | "exr" | "hdr" | "qoi" => "image",
        "mp3" | "m4a" | "aac" | "wav" | "aiff" | "flac" | "ogg" => "audio",
        "mp4" | "mov" | "mkv" | "m4v" | "webm" => "video",
        "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "dmg" | "iso" => "archive",
        "csv" | "tsv" | "json" | "jsonc" | "json5" | "jsonl" | "db" | "db3" | "sqlite"
        | "sqlite3" | "npy" | "safetensors" => "data",
        "opml" | "xmind" | "drawio" | "dot" | "gv" | "mermaid" | "mmd" | "d2" | "plantuml"
        | "puml" | "pikchr" | "vega" | "wavedrom" => "diagram",
        "otf" | "ttf" | "ttc" | "woff" | "woff2" | "bdf" => "font",
        "txt" | "log" => "text",
        "other" => "other",
        _ => "source_or_config",
    };
    vec![
        ("file_extension", text(extension)),
        ("file_family", text(family)),
    ]
}
pub fn file_event(
    name: &'static str,
    path: &Path,
    extra: impl IntoIterator<Item = (&'static str, Property)>,
) {
    let mut props = file_properties(path);
    props.extend(extra);
    track(name, props);
}
pub fn result<T>(event: &'static str, path: Option<&Path>, started: Instant, outcome: &Result<T>) {
    result_with(event, path, started, outcome, []);
}

pub fn result_with<T>(
    event: &'static str,
    path: Option<&Path>,
    started: Instant,
    outcome: &Result<T>,
    extra: impl IntoIterator<Item = (&'static str, Property)>,
) {
    let mut props = vec![
        (
            "result",
            text(if outcome.is_ok() { "success" } else { "failed" }),
        ),
        (
            "duration_ms",
            number(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
        ),
    ];
    if let Some(path) = path {
        props.extend(file_properties(path));
    }
    props.extend(extra);
    track(event, props);
    if let Err(error) = outcome {
        operation_failed(event, path, error);
    }
}
pub fn reader_feature(
    kind: &'static str,
    action: &'static str,
    extra: impl IntoIterator<Item = (&'static str, Property)>,
) {
    let mut props = vec![("reader_kind", text(kind)), ("action", text(action))];
    props.extend(extra);
    track("reader_feature", props);
}
