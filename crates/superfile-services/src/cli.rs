//! Command-line parsing, detached spawning, and user-PATH installation helpers.

use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

/// Environment variable set on the detached GUI child so it does not spawn another copy.
pub const DETACHED_ENV: &str = "SUPERFILE_CLI_DETACHED";

/// Parsed command-line launch flags and path/URL arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    /// Open targets (or an empty workspace) in a new window.
    pub new_window: bool,
    /// Add targets to the active workspace rather than opening a separate workspace.
    pub add: bool,
    /// Keep the CLI process attached to the launched GUI instead of detaching.
    pub wait: bool,
    /// Explicit `--cli` mode (set by Windows batch shims for zero-argument invocations).
    pub cli_mode: bool,
    /// File paths, folder paths, or `file://` / `superfile://` URLs.
    pub targets: Vec<OsString>,
}

/// Top-level command action parsed from process arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    /// Print CLI usage instructions and exit.
    Help,
    /// Print CLI version information and exit.
    Version,
    /// Launch or forward targets to the Superfile desktop application.
    Launch(LaunchOptions),
}

/// Decoded payload carried by a `superfile://open-cli` URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOpenRequest {
    /// Whether `-n` / `--new` was requested.
    pub new_window: bool,
    /// Whether `-a` / `--add` was requested.
    pub add: bool,
    /// Resolved `file://` or `superfile://` target URLs.
    pub urls: Vec<String>,
}

/// Result of installing the `superfile` command into a user bin directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInstallOutcome {
    /// Path of the installed symlink or command shim.
    pub path: PathBuf,
    /// Target Superfile executable referenced by the installed command.
    pub executable: PathBuf,
    /// Whether `path.parent()` is already listed on the current `PATH`.
    pub in_path: bool,
}

/// Parses `std::env::args_os().skip(1)` into a [`CliAction`].
pub fn parse_env_args() -> Result<CliAction> {
    parse_args(std::env::args_os().skip(1))
}

/// Parses command-line arguments (excluding `argv[0]`) into a [`CliAction`].
pub fn parse_args<I, S>(args: I) -> Result<CliAction>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut options = LaunchOptions::default();
    let mut after_double_dash = false;

    for raw in args {
        let arg: OsString = raw.into();
        if after_double_dash {
            options.targets.push(arg);
            continue;
        }
        let Some(text) = arg.to_str() else {
            options.targets.push(arg);
            continue;
        };
        if text == "--" {
            after_double_dash = true;
            continue;
        }
        if text.starts_with("-psn_") {
            // macOS LaunchServices passes `-psn_*` when launching `.app` bundles.
            continue;
        }
        match text {
            "-h" | "--help" => return Ok(CliAction::Help),
            "-V" | "-v" | "--version" => return Ok(CliAction::Version),
            "-n" | "--new" => options.new_window = true,
            "-a" | "--add" => options.add = true,
            "-w" | "--wait" => options.wait = true,
            "--cli" => options.cli_mode = true,
            _ if text.starts_with("--") => {
                bail!("Unknown option: {text}\nTry 'superfile --help' for more information.");
            }
            _ if text.starts_with('-') && text.len() > 1 => {
                for flag in text[1..].chars() {
                    match flag {
                        'h' => return Ok(CliAction::Help),
                        'V' | 'v' => return Ok(CliAction::Version),
                        'n' => options.new_window = true,
                        'a' => options.add = true,
                        'w' => options.wait = true,
                        _ => {
                            bail!(
                                "Unknown option: -{flag}\nTry 'superfile --help' for more information."
                            );
                        }
                    }
                }
            }
            _ => options.targets.push(arg),
        }
    }

    Ok(CliAction::Launch(options))
}

/// Returns human-readable help output for `--help` / `-h`.
pub fn help_text(measurement_build: bool) -> String {
    let mut text = String::from(
        "Superfile - Desktop file viewer and lightweight editor\n\n\
         Usage:\n  \
           superfile [OPTIONS] [PATH ...]\n\n\
         Arguments:\n  \
           [PATH ...]        Files, folders, or superfile:// / file:// URLs to open\n\n\
         Options:\n  \
           -n, --new         Open in a new window\n  \
           -a, --add         Add folders or files to the active workspace\n  \
           -w, --wait        Wait for the launched window instead of detaching immediately\n  \
           -V, --version     Print version information\n  \
           -h, --help        Print help information\n  \
           --                Treat all following arguments as paths",
    );
    if measurement_build {
        text.push_str("\n\nMeasurement build:\n  superfile --measure-source-reader fixture.txt");
    }
    text
}

/// Returns version string for `--version` / `-V` / `-v`.
pub fn version_text() -> String {
    format!("superfile {}", env!("CARGO_PKG_VERSION"))
}

/// Resolves parsed [`LaunchOptions`] into normalized `file://` or `superfile://` launch URLs.
pub fn resolve_urls(options: &LaunchOptions) -> Result<Vec<String>> {
    let resolved = options
        .targets
        .iter()
        .map(|target| crate::instance::launch_url(Path::new(target)))
        .collect::<Result<Vec<_>>>()?;

    if options.new_window || options.add {
        return Ok(vec![encode_open_cli_url(&CliOpenRequest {
            new_window: options.new_window,
            add: options.add,
            urls: resolved,
        })?]);
    }

    Ok(resolved)
}

/// Encodes CLI flags and target URLs into a `superfile://open-cli?...` request URL.
pub fn encode_open_cli_url(request: &CliOpenRequest) -> Result<String> {
    let mut url = url::Url::parse("superfile://open-cli")?;
    {
        let mut query = url.query_pairs_mut();
        if request.new_window {
            query.append_pair("new", "1");
        }
        if request.add {
            query.append_pair("add", "1");
        }
        for target in &request.urls {
            let normalized = crate::instance::launch_url(Path::new(target))?;
            ensure!(
                !normalized.starts_with("superfile://open-cli"),
                "Nested open-cli URLs are not allowed"
            );
            query.append_pair("target", &normalized);
        }
    }
    Ok(url.into())
}

/// Validates a `superfile://open-cli` URL without applying any action.
pub(crate) fn validate_open_cli_url(url: &url::Url) -> Result<()> {
    ensure!(
        url.scheme() == "superfile" && url.host_str() == Some("open-cli"),
        "Not an open-cli URL"
    );
    let _ = parse_open_cli_query(url)?;
    Ok(())
}

/// Decodes a `superfile://open-cli` URL, returning `None` for other URLs.
pub fn decode_open_cli_url(url: &url::Url) -> Result<Option<CliOpenRequest>> {
    if url.scheme() != "superfile" || url.host_str() != Some("open-cli") {
        return Ok(None);
    }
    Ok(Some(parse_open_cli_query(url)?))
}

fn parse_open_cli_query(url: &url::Url) -> Result<CliOpenRequest> {
    let mut request = CliOpenRequest::default();
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "new" => request.new_window = value == "1" || value.eq_ignore_ascii_case("true"),
            "add" => request.add = value == "1" || value.eq_ignore_ascii_case("true"),
            "target" => {
                let target_url = crate::instance::launch_url(Path::new(value.as_ref()))?;
                ensure!(
                    !target_url.starts_with("superfile://open-cli"),
                    "Nested open-cli URLs are not allowed"
                );
                request.urls.push(target_url);
            }
            _ => {}
        }
    }
    Ok(request)
}

/// Returns whether this process invocation came from a terminal CLI call that should detach.
///
/// Direct GUI launches (`cargo run` without flags, Finder, Explorer without targets) run in-process.
pub fn should_detach(options: &LaunchOptions, argv0: Option<&OsStr>) -> bool {
    if options.wait || std::env::var_os(DETACHED_ENV).is_some() {
        return false;
    }
    if options.cli_mode || options.new_window || options.add || !options.targets.is_empty() {
        return true;
    }
    argv0.is_some_and(invoked_via_symlink)
}

fn invoked_via_symlink(argv0: &OsStr) -> bool {
    let candidate = Path::new(argv0);
    if candidate.components().count() > 1 {
        return is_symlink(candidate);
    }
    let Some(path_env) = std::env::var_os("PATH") else {
        return false;
    };
    for directory in std::env::split_paths(&path_env) {
        let entry = directory.join(candidate);
        if is_symlink(&entry) {
            return true;
        }
    }
    false
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// Spawns `executable` detached from the current terminal with [`DETACHED_ENV`] set.
pub fn spawn_detached(executable: &Path, urls: &[String]) -> Result<()> {
    let mut command = std::process::Command::new(executable);
    command
        .args(urls)
        .env(DETACHED_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    command
        .spawn()
        .with_context(|| format!("Cannot start Superfile from {}", executable.display()))?;
    Ok(())
}

/// Returns the installed CLI path if one currently exists in the default user CLI directory.
pub fn installed_cli_path() -> Option<PathBuf> {
    let home = crate::platform::home().ok()?;
    let candidates = candidate_install_dirs(&home, std::env::var_os("PATH").as_deref());
    for dir in candidates {
        let path = dir.join(cli_binary_name());
        if fs::symlink_metadata(&path).is_ok() {
            return Some(path);
        }
    }
    None
}

/// Installs the `superfile` CLI symlink (Unix) or `.cmd` shim (Windows) into a user bin directory.
pub fn install_cli() -> Result<CliInstallOutcome> {
    let executable = std::env::current_exe().context("Cannot locate the Superfile executable")?;
    let executable = fs::canonicalize(&executable).unwrap_or(executable);
    let home = crate::platform::home()?;
    let path_env = std::env::var_os("PATH");
    let target_dir = select_install_dir(&home, path_env.as_deref());
    install_cli_in_dir(&target_dir, &executable, path_env.as_deref())
}

fn cli_binary_name() -> &'static str {
    if cfg!(windows) {
        "superfile.cmd"
    } else {
        "superfile"
    }
}

fn local_bin_dir(home: &Path) -> PathBuf {
    home.join(".local").join("bin")
}

fn candidate_install_dirs(home: &Path, path_env: Option<&OsStr>) -> Vec<PathBuf> {
    let mut dirs = vec![local_bin_dir(home)];
    if cfg!(unix) {
        let usr_local = PathBuf::from("/usr/local/bin");
        if path_contains_dir(path_env, &usr_local) && !dirs.contains(&usr_local) {
            dirs.push(usr_local);
        }
    }
    dirs
}

fn select_install_dir(home: &Path, path_env: Option<&OsStr>) -> PathBuf {
    let local_bin = local_bin_dir(home);
    if path_contains_dir(path_env, &local_bin) {
        return local_bin;
    }
    #[cfg(unix)]
    {
        let usr_local = PathBuf::from("/usr/local/bin");
        if path_contains_dir(path_env, &usr_local) && dir_is_writable(&usr_local) {
            return usr_local;
        }
    }
    local_bin
}

#[cfg(unix)]
fn dir_is_writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    tempfile::Builder::new()
        .prefix(".superfile-cli-probe-")
        .tempfile_in(dir)
        .is_ok()
}

fn path_contains_dir(path_env: Option<&OsStr>, target: &Path) -> bool {
    let Some(path_env) = path_env else {
        return false;
    };
    let canonical_target = fs::canonicalize(target).ok();
    std::env::split_paths(path_env).any(|entry| {
        if entry == target {
            return true;
        }
        canonical_target
            .as_ref()
            .is_some_and(|canon| fs::canonicalize(&entry).ok().as_ref() == Some(canon))
    })
}

/// Installs the CLI command into an explicit directory (used directly by tests).
pub fn install_cli_in_dir(
    bin_dir: &Path,
    executable: &Path,
    path_env: Option<&OsStr>,
) -> Result<CliInstallOutcome> {
    fs::create_dir_all(bin_dir)
        .with_context(|| format!("Cannot create CLI directory {}", bin_dir.display()))?;
    let destination = bin_dir.join(cli_binary_name());

    #[cfg(unix)]
    install_unix_symlink(&destination, executable)?;
    #[cfg(windows)]
    install_windows_shim(&destination, executable)?;

    Ok(CliInstallOutcome {
        in_path: path_contains_dir(path_env, bin_dir),
        path: destination,
        executable: executable.to_path_buf(),
    })
}

#[cfg(unix)]
fn install_unix_symlink(destination: &Path, executable: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {}
        Ok(_) => {
            bail!(
                "Refusing to overwrite existing non-symlink file at {}",
                destination.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let parent = destination
        .parent()
        .context("CLI destination has no parent directory")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_link = parent.join(format!(".superfile-cli-{}-{stamp}", std::process::id()));
    std::os::unix::fs::symlink(executable, &temp_link).with_context(|| {
        format!(
            "Cannot create symlink from {} to {}",
            temp_link.display(),
            executable.display()
        )
    })?;
    if let Err(error) = fs::rename(&temp_link, destination) {
        let _ = fs::remove_file(&temp_link);
        return Err(error)
            .with_context(|| format!("Cannot install symlink at {}", destination.display()));
    }
    Ok(())
}

#[cfg(windows)]
const WINDOWS_SHIM_MARKER: &str = "REM Superfile CLI shim";

#[cfg(windows)]
fn install_windows_shim(destination: &Path, executable: &Path) -> Result<()> {
    use std::io::Write;
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let existing = fs::read_to_string(destination).unwrap_or_default();
            ensure!(
                existing.contains(WINDOWS_SHIM_MARKER),
                "Refusing to overwrite existing non-Superfile script at {}",
                destination.display()
            );
        }
        Ok(_) => {
            bail!(
                "Refusing to overwrite non-file entry at {}",
                destination.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let exe_str = executable
        .to_str()
        .context("Superfile executable path is not valid UTF-8")?;
    ensure!(
        !exe_str.contains('"') && !exe_str.contains('\n') && !exe_str.contains('\r'),
        "Superfile executable path contains unsupported characters"
    );
    let content = format!("@echo off\r\n{WINDOWS_SHIM_MARKER}\r\n\"{exe_str}\" --cli %*\r\n");
    let parent = destination
        .parent()
        .context("CLI destination has no parent directory")?;
    let mut temp = tempfile::Builder::new()
        .prefix(".superfile-cli-")
        .suffix(".cmd.tmp")
        .tempfile_in(parent)?;
    temp.write_all(content.as_bytes())?;
    temp.persist(destination).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_targets_double_dash_and_ignores_macos_psn() -> Result<()> {
        assert_eq!(parse_args(["-h"])?, CliAction::Help);
        assert_eq!(parse_args(["--help"])?, CliAction::Help);
        assert_eq!(parse_args(["-V"])?, CliAction::Version);
        assert_eq!(parse_args(["-v"])?, CliAction::Version);
        assert_eq!(parse_args(["--version"])?, CliAction::Version);

        let action = parse_args([
            "-psn_0_12345",
            "-nw",
            "--add",
            "--",
            "-literal-file.txt",
            ".",
        ])?;
        assert_eq!(
            action,
            CliAction::Launch(LaunchOptions {
                new_window: true,
                add: true,
                wait: true,
                cli_mode: false,
                targets: vec![OsString::from("-literal-file.txt"), OsString::from(".")],
            })
        );

        assert!(parse_args(["--bogus"]).is_err());
        assert!(parse_args(["-z"]).is_err());
        Ok(())
    }

    #[test]
    fn resolves_cli_open_urls_and_canonicalizes_relative_dot() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let canonical_dir = fs::canonicalize(dir.path())?;
        let file = canonical_dir.join("note #1.md");
        fs::write(&file, "hello")?;

        let opts = LaunchOptions {
            new_window: true,
            add: true,
            wait: false,
            cli_mode: false,
            targets: vec![canonical_dir.clone().into(), file.clone().into()],
        };
        let urls = resolve_urls(&opts)?;
        assert_eq!(urls.len(), 1);
        let parsed = url::Url::parse(&urls[0])?;
        let decoded = decode_open_cli_url(&parsed)?.expect("open-cli request");
        assert!(decoded.new_window);
        assert!(decoded.add);
        assert_eq!(
            decoded.urls,
            vec![
                url::Url::from_file_path(&canonical_dir)
                    .unwrap()
                    .to_string(),
                url::Url::from_file_path(&file).unwrap().to_string(),
            ]
        );

        // Plain empty --new still encodes a superfile://open-cli?new=1 request.
        let empty_new = resolve_urls(&LaunchOptions {
            new_window: true,
            ..Default::default()
        })?;
        let decoded_empty = decode_open_cli_url(&url::Url::parse(&empty_new[0])?)?.unwrap();
        assert!(decoded_empty.new_window);
        assert!(!decoded_empty.add);
        assert!(decoded_empty.urls.is_empty());
        Ok(())
    }

    #[test]
    fn direct_gui_launch_without_flags_does_not_detach() {
        let gui = LaunchOptions::default();
        assert!(!should_detach(
            &gui,
            Some(OsStr::new("target/debug/superfile"))
        ));

        let with_target = LaunchOptions {
            targets: vec![OsString::from(".")],
            ..Default::default()
        };
        assert!(should_detach(&with_target, None));

        let with_wait = LaunchOptions {
            wait: true,
            cli_mode: true,
            targets: vec![OsString::from(".")],
            ..Default::default()
        };
        assert!(!should_detach(&with_wait, None));
    }

    #[test]
    fn installs_cli_into_target_directory_and_protects_unrelated_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let bin_dir = temp.path().join("bin");
        let fake_exe = temp.path().join("superfile-bin");
        fs::write(&fake_exe, b"binary")?;

        let outcome = install_cli_in_dir(&bin_dir, &fake_exe, Some(bin_dir.as_os_str()))?;
        assert!(outcome.in_path);
        assert!(fs::symlink_metadata(&outcome.path).is_ok());

        // Reinstalling updates cleanly.
        let fake_exe_v2 = temp.path().join("superfile-bin-v2");
        fs::write(&fake_exe_v2, b"binary-v2")?;
        let updated = install_cli_in_dir(&bin_dir, &fake_exe_v2, None)?;
        assert!(!updated.in_path);
        #[cfg(unix)]
        assert_eq!(fs::read_link(&updated.path)?, fake_exe_v2);

        // Unrelated non-symlink/non-shim file must not be overwritten.
        let blocked_dir = temp.path().join("blocked");
        fs::create_dir_all(&blocked_dir)?;
        let foreign = blocked_dir.join(cli_binary_name());
        fs::write(&foreign, b"do not clobber")?;
        assert!(install_cli_in_dir(&blocked_dir, &fake_exe, None).is_err());
        assert_eq!(fs::read(&foreign)?, b"do not clobber");

        Ok(())
    }
}
