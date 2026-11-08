use std::{env, path::PathBuf, process::Command};
#[path = "../../scripts/build-python.rs"]
mod python;
fn main() {
    // GPUI's nested layout traversal exceeds the MSVC 1 MiB main-stack default
    // in debug builds. Reserve 8 MiB, matching a typical desktop main thread;
    // pages are committed on demand. This is an executable setting, not a GPUI patch.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-arg-bin=superfile=/STACK:8388608");
    }
    embed_build_identity();
    println!("cargo:rustc-env=SUPERFILE_BUILD_NUMBER=1");
    println!("cargo:rerun-if-changed=../../assets/reader-limits.json");
    println!("cargo:rerun-if-changed=../../scripts/generate-reader-limits.py");
    assert!(
        python::python()
            .arg("../../scripts/generate-reader-limits.py")
            .arg("--out-dir")
            .arg(PathBuf::from(env::var_os("OUT_DIR").unwrap()))
            .status()
            .expect("Generate reader policy")
            .success()
    );
}

fn embed_build_identity() {
    for name in ["SUPERFILE_BUILD_CHANNEL", "SUPERFILE_BUILD_TIMESTAMP"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    // Refresh direct Cargo builds when sources change, not just packaged builds.
    for path in ["crates", "assets", "scripts", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed=../../{path}");
    }
    let output = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    for name in ["HEAD", "index"] {
        if let Some(path) = output("git", &["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = output("git", &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = output("git", &["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    let channel = env::var("SUPERFILE_BUILD_CHANNEL").unwrap_or_else(|_| "local".into());
    assert!(
        matches!(channel.as_str(), "local" | "release"),
        "Invalid build channel"
    );
    let timestamp = env::var("SUPERFILE_BUILD_TIMESTAMP")
        .ok()
        .or_else(|| output("date", &["+%Y-%m-%d %H:%M:%S %z"]))
        .unwrap_or_else(|| {
            format!(
                "{} UTC epoch seconds",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock")
                    .as_secs()
            )
        });
    let hash = output("git", &["rev-parse", "--short=12", "HEAD"]).unwrap_or_default();
    let dirty = output(
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    )
    .is_some_and(|status| !status.is_empty());
    println!("cargo:rustc-env=SUPERFILE_BUILD_CHANNEL={channel}");
    println!("cargo:rustc-env=SUPERFILE_BUILD_TIMESTAMP={timestamp}");
    println!("cargo:rustc-env=SUPERFILE_BUILD_HASH={hash}");
    println!("cargo:rustc-env=SUPERFILE_BUILD_DIRTY={dirty}");
}
