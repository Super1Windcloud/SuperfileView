//! Read the operating system's current file associations without opening a GUI.
fn main() -> anyhow::Result<()> {
    use superfile_services::file_associations;
    if let Some(identifier) = std::env::args().nth(1) {
        let applications = file_associations::applications(&identifier, &Default::default())?;
        serde_json::to_writer_pretty(std::io::stdout().lock(), &applications)?;
    } else {
        let snapshot = file_associations::scan(&Default::default())?;
        serde_json::to_writer_pretty(std::io::stdout().lock(), &snapshot)?;
    }
    Ok(())
}
