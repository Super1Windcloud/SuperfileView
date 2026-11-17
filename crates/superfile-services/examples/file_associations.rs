//! Read the operating system's current file associations without opening a GUI.
fn main() -> anyhow::Result<()> {
    let snapshot = superfile_services::file_associations::scan(&Default::default())?;
    serde_json::to_writer_pretty(std::io::stdout().lock(), &snapshot)?;
    Ok(())
}
