//! Downloads real community packages into the configured Superfile data directory.
//! Example: SUPERFILE_DATA_DIR=target/theme-audit-data cargo run -p superfile-services --example audit_themes -- Minimal Things
use anyhow::{Context, Result};
use superfile_services::{themes, work::Cancellation};

fn main() -> Result<()> {
    anyhow::ensure!(
        std::env::var_os("SUPERFILE_DATA_DIR").is_some(),
        "Set SUPERFILE_DATA_DIR to an audit directory first"
    );
    let cancel = Cancellation::default();
    let catalog = themes::catalog(&cancel)?;
    println!(
        "Directory: {} entries, cached={}",
        catalog.themes.len(),
        catalog.cached
    );
    for name in std::env::args().skip(1) {
        let entry = catalog
            .themes
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(&name))
            .with_context(|| format!("Unknown theme: {name}"))?;
        let package = themes::install(entry, &cancel)?;
        let reopened = themes::load(&entry.id(), &cancel)?;
        println!("{} ({})", package.scheme.name, package.entry.repo);
        for mode in [themes::Mode::Light, themes::Mode::Dark] {
            let palette = reopened.scheme.palette(mode);
            println!(
                "  {mode:?}: mapped={} fallback={:?}, background={:?}, text={:?}, accent={:?}",
                palette.applied,
                palette.unresolved,
                palette.get("--background-primary"),
                palette.get("--text-normal"),
                palette.get("--interactive-accent")
            );
        }
        match themes::preview(entry, &cancel) {
            Ok(path) => println!("  preview={}", path.display()),
            Err(error) => println!("  preview unavailable: {error:#}"),
        }
    }
    Ok(())
}
