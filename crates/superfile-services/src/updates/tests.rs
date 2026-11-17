use super::*;
use gpui_updater_core::Asset;

#[test]
fn static_manifest_check_selects_native_archive_without_api_or_authorization() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };
    let release = release("v1.2.3");
    let asset = serde_json::json!({
        "name": release.asset.name, "url": release.asset.url,
        "size": release.asset.size, "sha256": release.sha256,
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "format": if cfg!(windows) { "zip" } else { "tar.gz" },
    });
    let mut wrong = asset.clone();
    wrong["arch"] = "unsupported".into();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1, "app_id": "app.superfile.desktop", "version": "1.2.3",
        "assets": [wrong, asset],
    }))
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            if let Ok((socket, _)) = listener.accept() {
                break socket;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Manifest server timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).unwrap();
            assert!(count > 0 && request.len() < 16384);
            request.extend_from_slice(&buffer[..count]);
        }
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
        socket.write_all(&bytes).unwrap();
        String::from_utf8(request).unwrap()
    });
    // Upstream permits loopback HTTP for deterministic tests; production pins HTTPS.
    let source = TrustedManifest(source(&format!("http://{address}/latest.json")));
    let result = UpdateEngine::new(
        source,
        EngineConfig::new(Version::parse("0.0.2").unwrap()).verification(Verification::Checksum),
    )
    .check();
    let request = server.join().unwrap();
    assert!(request.starts_with("GET /latest.json HTTP/1.1"));
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
    assert_eq!(result.unwrap().unwrap().asset.name, release.asset.name);
}

pub(super) fn release(tag: &str) -> Release {
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    let name = format!(
        "superfile-{tag}-{}-{}.{extension}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let prefix = manifest_url()
        .strip_suffix("/latest/download/latest.json")
        .unwrap();
    Release::new(
        Version::parse(tag.trim_start_matches('v')).unwrap(),
        Asset::new(&name, format!("{prefix}/download/{tag}/{name}"), 10),
    )
    .with_sha256("a".repeat(64))
}

#[test]
fn manifest_binds_exact_repository_version_platform_and_checksum() {
    for tag in ["v1.2.3", "1.2.3"] {
        let valid = release(tag);
        validate_release(&valid).unwrap();
        let mut invalid = valid.clone();
        invalid.sha256 = None;
        assert!(validate_release(&invalid).is_err());
        invalid.sha256 = Some("a".repeat(63));
        assert!(validate_release(&invalid).is_err());
        invalid = valid.clone();
        invalid.version = Version::parse("9.9.9").unwrap();
        assert!(validate_release(&invalid).is_err());
        invalid = valid.clone();
        invalid.asset.name = invalid
            .asset
            .name
            .replace(std::env::consts::ARCH, "wrongarch");
        assert!(validate_release(&invalid).is_err());
        for url in [
            valid.asset.url.replacen("https:", "http:", 1),
            valid.asset.url.replacen("github.com", "evil.example", 1),
            valid.asset.url.replace("/download/", "/download/other/"),
            format!("{}?token=secret", valid.asset.url),
            format!("{}#fragment", valid.asset.url),
        ] {
            invalid = valid.clone();
            invalid.asset.url = url;
            assert!(validate_release(&invalid).is_err());
        }
    }
    assert!(validate_release(&release("v1.2.3-rc.1")).is_err());
}

#[test]
fn verified_cached_download_uses_static_source_without_network_or_host_tools() {
    let directory = tempfile::tempdir().unwrap();
    let mut release = release("v1.2.3");
    let path = directory.path().join(&release.asset.name);
    let bytes = b"verified update archive";
    std::fs::write(&path, bytes).unwrap();
    release.asset.size = bytes.len() as u64;
    release.sha256 = Some(gpui_updater_core::verify::sha256_hex(&path).unwrap());
    let engine = engine(Some(directory.path().to_owned())).unwrap();
    let artifact = engine
        .download_cancellable(&release, &CancelToken::new(), |_, _| {})
        .unwrap();
    assert_eq!(artifact.path(), path);
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(
        engine
            .download_cancellable(&release, &cancel, |_, _| {})
            .is_err()
    );
}
