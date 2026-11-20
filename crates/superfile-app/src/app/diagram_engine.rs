//! Cancellable subprocess boundary for the pure Rust diagram engine.
use anyhow::{Context, Result, ensure};
pub fn run() -> Result<()> {
    let input = std::path::PathBuf::from(
        std::env::args_os()
            .nth(2)
            .context("Missing diagram input")?,
    );
    let output = std::path::PathBuf::from(
        std::env::args_os()
            .nth(3)
            .context("Missing diagram output")?,
    );
    ensure!(
        std::fs::metadata(&input)?.len() <= superfile_services::limits::DIAGRAM_REQUEST_BYTES,
        "Diagram request exceeds working budget"
    );
    let requests: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(input)?)?;
    ensure!(
        requests.len() <= superfile_services::markdown_preview::MAX_DIAGRAMS,
        "Too many active diagrams"
    );
    let mut results = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        let source = request["source"]
            .as_str()
            .context("Missing diagram source")?;
        ensure!(
            source.len() <= superfile_services::markdown_preview::MAX_SOURCE,
            "Diagram source exceeds working budget"
        );
        let palette = serde_json::from_value(request["palette"].clone())?;
        let rendered = render_diagram(
            request["kind"].as_str().unwrap_or_default(),
            source,
            palette,
        );
        let result = match rendered {
            Ok(svg) => serde_json::json!({"index":index,"svg":svg}),
            Err(error) => serde_json::json!({"index":index,"error":error.to_string()}),
        };
        let bytes = serde_json::to_vec(&result)?;
        ensure!(
            bytes.len() <= superfile_services::limits::DIAGRAM_IPC_BYTES as usize,
            "Diagram output exceeds working budget"
        );
        let destination = output.with_extension(format!("{index}.json"));
        let temporary = destination.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, destination)?;
        results.push(result);
    }
    std::fs::write(output, serde_json::to_vec(&results)?)?;
    Ok(())
}

fn render_diagram(
    kind: &str,
    source: &str,
    palette: Option<superfile_core::reader_palette::ReaderPalette>,
) -> Result<String> {
    match kind {
        "mermaid" => super::diagram_palette::mermaid(source, palette),
        "vega" => superfile_services::vega::render(source, palette),
        "math" | "inline-math" => {
            let options = mathra::Options {
                display: kind == "math",
                ..Default::default()
            };
            let rendered =
                mathra::convert(source, &options).map_err(|error| anyhow::anyhow!("{error}"))?;
            Ok(rendered.svg)
        }
        "plantuml" => Ok(plantuml_core::render(source)?),
        "wavedrom" => {
            let mut svg = Vec::new();
            wavedrom::render_json5(source, &mut svg)
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            Ok(String::from_utf8(svg)?)
        }
        "d2" => {
            let options = if palette.is_some_and(|p| p.is_dark()) {
                d2_render::RenderOptions::new().theme(200)
            } else {
                d2_render::RenderOptions::new()
            };
            Ok(d2_render::native::render_svg(source, &options)?.svg)
        }
        "pikchr" => pikru::pikchr(source).map_err(|e| anyhow::anyhow!("{e}")),
        "dot" => Ok(graphviz_pure::render_dot(source, "dot", "svg")?.into_output()),
        _ => anyhow::bail!("No diagram renderer for {kind}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_engines_produce_svg() {
        for (kind, source) in [
            ("d2", "reader -> page"),
            ("dot", "digraph { reader -> page }"),
            ("pikchr", "box \"reader\"; arrow; box \"page\""),
            ("plantuml", "@startuml\nAlice -> Bob: hello\n@enduml"),
            ("wavedrom", "{signal:[{name:'clock',wave:'p...'}]}"),
            ("math", r"\frac{a}{b}"),
            ("inline-math", "x^2"),
        ] {
            let svg = render_diagram(kind, source, None).unwrap_or_else(|e| panic!("{kind}: {e}"));
            assert!(svg.contains("<svg") && svg.contains("</svg>"), "{kind}");
        }
    }
}
