//! Conservative first-pass document projection. Original text stays editable.
use std::path::Path;
pub fn supports(path: &Path) -> bool {
    superfile_core::formats::FormatSet::ConvertedMarkup.matches(path)
}
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}
pub fn renders_html(path: &Path) -> bool {
    matches!(extension(path).as_str(), "org" | "adoc")
}

pub fn project(path: &Path, text: &str) -> String {
    match extension(path).as_str() {
        "adoc" => asciidoc_html5::convert_with(
            text,
            &asciidoc_html5::Options::new()
                .safe_mode(asciidoc_html5::SafeMode::Secure)
                .set("showtitle"),
        ),
        "org" => {
            let mut html = Vec::new();
            let doc = orgize::Org::parse(text);
            if doc.write_html(&mut html).is_ok()
                && let Ok(html) = String::from_utf8(html)
            {
                return html;
            }
            format!("<pre>{}</pre>", quick_xml::escape::escape(text))
        }
        _ => markdown(path, text),
    }
}

fn fenced(text: &str, language: &str) -> String {
    let width = text
        .lines()
        .map(|l| l.trim_start().bytes().take_while(|b| *b == b'`').count())
        .max()
        .unwrap_or(0)
        .max(2)
        + 1;
    let fence = "`".repeat(width);
    format!("{fence}{language}\n{text}\n{fence}\n\n")
}

fn multiline(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(lines) => lines.iter().filter_map(|v| v.as_str()).collect(),
        _ => String::new(),
    }
}
fn notebook(text: &str) -> Option<String> {
    use std::fmt::Write;
    let book: serde_json::Value = serde_json::from_str(text).ok()?;
    let cells = book.get("cells")?.as_array()?;
    let language = book
        .pointer("/metadata/language_info/name")
        .and_then(|v| v.as_str())
        .filter(|s| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .unwrap_or("");
    let mut result = String::new();
    for cell in cells {
        let source = multiline(&cell["source"]);
        match cell["cell_type"].as_str() {
            Some("markdown") => {
                result.push_str(&source);
                result.push_str("\n\n");
            }
            Some("code") => {
                result.push_str(&fenced(&source, language));
                for output in cell["outputs"].as_array().into_iter().flatten() {
                    let data = &output["data"];
                    if let Some((mime, img)) = ["image/png", "image/jpeg"]
                        .into_iter()
                        .find_map(|m| data.get(m).map(|v| (m, multiline(v))))
                    {
                        let img = img.split_ascii_whitespace().collect::<String>();
                        if img.len() <= crate::limits::TEXT_EDITABLE_BYTES as usize
                            && img
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
                        {
                            let _ = writeln!(result, "![](data:{mime};base64,{img})\n");
                            continue;
                        }
                    }
                    if let Some(text) = output.get("text").or_else(|| data.get("text/plain")) {
                        result.push_str(&fenced(&multiline(text), ""));
                    } else if let Some(message) = output["evalue"].as_str() {
                        result.push_str(&fenced(message, ""));
                    }
                }
            }
            _ => result.push_str(&fenced(&source, "")),
        }
    }
    Some(result)
}
fn subtitles(ext: &str, text: &str) -> Option<String> {
    use std::fmt::Write;
    let mut output = String::new();
    let mut cue = |start: String, end: String, lines: Vec<String>| {
        let _ = writeln!(output, "### `{start} → {end}`\n");
        for line in lines {
            let _ = writeln!(output, "{}  ", quick_xml::escape::escape(&line));
        }
        output.push('\n');
    };
    let text = text.trim_start_matches('\u{feff}');
    if ext == "srt" {
        for item in subtp::srt::SubRip::parse(text).ok()?.subtitles {
            cue(item.start.to_string(), item.end.to_string(), item.text);
        }
    } else {
        for block in subtp::vtt::WebVtt::parse(text).ok()?.blocks {
            if let subtp::vtt::VttBlock::Que(item) = block {
                cue(
                    item.timings.start.to_string(),
                    item.timings.end.to_string(),
                    item.payload,
                );
            }
        }
    }
    Some(output)
}

pub fn markdown(path: &Path, text: &str) -> String {
    let ext = extension(path);
    if ext == "ipynb" {
        return notebook(text).unwrap_or_else(|| fenced(text, "json"));
    }
    if matches!(ext.as_str(), "srt" | "vtt")
        && let Some(projection) = subtitles(&ext, text)
    {
        return projection;
    }
    if matches!(ext.as_str(), "d2" | "pikchr")
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".vega.json"))
    {
        let kind = if ext == "json" { "vega" } else { &ext };
        let fence = "`".repeat(
            text.lines()
                .map(|line| line.chars().take_while(|c| *c == '`').count())
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        return format!("{fence}{kind}\n{text}\n{fence}\n");
    }
    let mut output = String::new();
    let lines: Vec<_> = text.lines().collect();
    let mut code = false;
    for (index, line) in lines.iter().enumerate() {
        if ext == "adoc" && matches!(*line, "----" | "....") {
            code = !code;
            output.push_str("```\n");
        } else if code {
            output.push_str(line);
            output.push('\n');
        } else if ext == "adoc" && line.starts_with("[source,") {
        } else if ext == "org" && line.starts_with("#+TITLE:") {
            output.push_str(&format!(
                "# {}\n",
                line.trim_start_matches("#+TITLE:").trim()
            ));
        } else if (ext == "adoc" && line.starts_with('='))
            || (ext == "org" && line.starts_with('*'))
        {
            let marker = if ext == "org" { '*' } else { '=' };
            let count = line.chars().take_while(|c| *c == marker).count();
            if line.as_bytes().get(count) == Some(&b' ') {
                output.push_str(&format!(
                    "{} {}\n",
                    "#".repeat(count.min(6)),
                    line[count..].trim()
                ));
            } else {
                output.push_str(line);
                output.push('\n');
            }
        } else if ext == "rst"
            && lines.get(index + 1).is_some_and(|next| {
                next.len() >= line.len()
                    && !line.is_empty()
                    && next.chars().all(|c| c == '=' || c == '-')
            })
        {
            output.push_str(&format!("## {line}\n"));
        } else if ext == "rst" && !line.is_empty() && line.chars().all(|c| c == '=' || c == '-') {
        } else if matches!(ext.as_str(), "srt" | "vtt") && line.contains("-->") {
            output.push_str(&format!("\n### `{line}`\n\n"));
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    if code {
        output.push_str("```\n");
    }
    if matches!(ext.as_str(), "gpx" | "kml") {
        // XML is shown literally, never as active HTML or a remote map.
        output = format!(
            "# {}\n\n{}\n\n{}\n\n```xml\n{text}\n```\n",
            ext.to_uppercase(),
            crate::locale::tr("Geographic data · Offline coordinate preview"),
            coordinates(text)
        );
    }
    output
}

fn coordinates(text: &str) -> String {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut output = format!(
        "| {} | {} | {} |\n| --- | --- | --- |\n",
        crate::locale::tr("Longitude"),
        crate::locale::tr("Latitude"),
        crate::locale::tr("Altitude")
    );
    let mut in_coordinates = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag) | Event::Empty(tag)) => {
                if tag.local_name().as_ref() == "coordinates" {
                    in_coordinates = true;
                }
                if matches!(tag.local_name().as_ref(), "wpt" | "trkpt" | "rtept") {
                    let mut lat = String::new();
                    let mut lon = String::new();
                    for attr in tag.attributes().flatten() {
                        let value = attr.value.into_owned();
                        match attr.key.as_ref() {
                            "lat" => lat = value,
                            "lon" => lon = value,
                            _ => {}
                        }
                    }
                    if let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>())
                        && lat.is_finite()
                        && lon.is_finite()
                    {
                        output.push_str(&format!("| {lon} | {lat} | |\n"));
                    }
                }
            }
            Ok(Event::Text(value)) if in_coordinates => {
                for point in value.split_whitespace() {
                    let values: Vec<_> = point
                        .split(',')
                        .filter_map(|v| v.parse::<f64>().ok())
                        .collect();
                    if values.len() >= 2 && values.iter().all(|v| v.is_finite()) {
                        output.push_str(&format!(
                            "| {} | {} | {} |\n",
                            values[0],
                            values[1],
                            values.get(2).copied().unwrap_or(0.0)
                        ));
                    }
                }
            }
            Ok(Event::End(tag)) if tag.local_name().as_ref() == "coordinates" => {
                in_coordinates = false
            }
            Ok(Event::Eof) => break,
            Err(error) => return format!("{}: {error}", crate::locale::tr("XML parsing failed")),
            _ => {}
        }
    }
    output
}
