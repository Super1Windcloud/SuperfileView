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
        let rendered = if request["kind"].as_str() == Some("mermaid") {
            mermaid_rs_renderer::render(source)
        } else {
            Err(anyhow::anyhow!(
                "This diagram engine is not implemented yet"
            ))
        };
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
