#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod views;

fn main() -> anyhow::Result<()> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--open-from-codex")) {
        return superfile_services::integrations::codex::run();
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--mcp")) {
        return superfile_services::integrations::mcp::run();
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--build-info")) {
        app::print_build_info();
        return Ok(());
    }
    #[cfg(feature = "reader-measurements")]
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--measure-source-reader"))
    {
        return app::reader_measurement::run();
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--render-markdown")) {
        return app::diagram_engine::run();
    }

    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--prepare-matroska")) {
        return app::media_prepare::run();
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--prepare-spreadsheet-page"))
    {
        return app::spreadsheet_prepare::run();
    }
    app::run()
}
