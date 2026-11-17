pub fn python() -> std::process::Command {
    println!("cargo:rerun-if-env-changed=PYTHON");
    let mut command = std::process::Command::new(std::env::var_os("PYTHON").unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    }));
    // Keep diagnostics and any implicit stream encoding independent of the
    // Windows ANSI code page. Repository file I/O also pins UTF-8 explicitly.
    command.args(["-X", "utf8"]);
    command
}
