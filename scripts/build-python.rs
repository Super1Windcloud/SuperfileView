pub fn python() -> std::process::Command {
    println!("cargo:rerun-if-env-changed=PYTHON");
    std::process::Command::new(std::env::var_os("PYTHON").unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    }))
}
