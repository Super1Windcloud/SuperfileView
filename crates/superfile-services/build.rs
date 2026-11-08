use std::{env, path::PathBuf};
#[path = "../../scripts/build-python.rs"]
mod python;
use python::python;
fn main() {
    println!("cargo:rustc-env=SUPERFILE_TELEMETRY_ENVIRONMENT=disabled");
    diagram_cache_fingerprint();
    println!("cargo:rerun-if-changed=../../assets/reader-limits.json");
    println!("cargo:rerun-if-changed=../../scripts/generate-reader-limits.py");
    let limits_out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let generated = python()
        .arg("../../scripts/generate-reader-limits.py")
        .arg("--out-dir")
        .arg(&limits_out)
        .status()
        .expect("Generate shared reader limits");
    assert!(generated.success(), "Reader limit generation failed");

    println!("cargo:rerun-if-changed=../../assets/locales");
    println!("cargo:rerun-if-changed=../../scripts/check-locales.py");
    println!("cargo:rerun-if-changed=../../crates/superfile-app/src");
    println!("cargo:rerun-if-changed=src");
    let checked = python()
        .arg("../../scripts/check-locales.py")
        .status()
        .expect("Check locale catalogs");
    assert!(checked.success(), "Locale catalog validation failed");
    let mut catalogs = std::fs::read_dir("../../assets/locales")
        .expect("locale directory")
        .map(|entry| entry.expect("locale entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    catalogs.sort();
    let entries = catalogs
        .iter()
        .map(|path| {
            let code = path.file_stem().unwrap().to_str().unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            format!("({code:?}, include_str!({:?}))", path.to_str().unwrap())
        })
        .collect::<Vec<_>>()
        .join(",\n");
    std::fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("locales.rs"),
        format!("const BUNDLED_CATALOGS: &[(&str, &str)] = &[{entries}];"),
    )
    .expect("embed locale catalogs");
}

fn diagram_cache_fingerprint() {
    use sha2::{Digest, Sha256};
    let mut files = vec![
        PathBuf::from("../../Cargo.lock"),
        PathBuf::from("../../assets/reader-limits.json"),
        PathBuf::from("../../crates/superfile-app/src/app/diagram_engine.rs"),
        PathBuf::from("src/markdown_preview.rs"),
        PathBuf::from("src/markdown_preview/disk_cache.rs"),
    ];
    let mut directories: Vec<PathBuf> = Vec::new();
    while let Some(directory) = directories.pop() {
        println!("cargo:rerun-if-changed={}", directory.display());
        for entry in std::fs::read_dir(directory).expect("diagram asset directory") {
            let path = entry.expect("diagram asset entry").path();
            if path.is_dir() {
                directories.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut digest = Sha256::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = std::fs::read(&path).expect("diagram cache fingerprint input");
        digest.update(path.to_string_lossy().as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    println!(
        "cargo:rustc-env=SUPERFILE_DIAGRAM_CACHE_FINGERPRINT={:x}",
        digest.finalize()
    );
}
