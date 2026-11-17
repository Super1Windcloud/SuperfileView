# Superfile

**English** | [简体中文](README.zh-CN.md)

A document workspace built with Rust and shared gpui-kit components, targeting Windows, macOS, and Linux.

Superfile brings files, source code, documents, images, and structured data into one workspace. It is under active development; the implementation and platform acceptance are tracked in [PLAN.md](PLAN.md) and the [current feature status](docs/support.md).

Author: **superwindcloud**.

## Design

- Shared GPUI readers and controls across desktop platforms.
- Official, locked crates.io dependencies; no `vendor` directory or framework source patches.
- Background file I/O, cancellable jobs, paged data, and retained reader state.
- Independent Superfile application data and cache directories.
- Local diagnostic logs, with no remote telemetry transport or automatic updater.

The application does not use Swift, Quick Look, or WebView-based readers. Markdown interactions use the public plugin API; Mermaid diagrams are rendered by a Rust engine. HTML is displayed with TextView and does not execute scripts.

## Implementation status

| Area | Current implementation |
| --- | --- |
| Workspace | File tree, multiple folders, tabs, navigation, recent workspaces, and session state |
| Appearance | Configurable accent (purple by default), Obsidian foundation palettes, and [community theme browsing, downloads, and color-variable mapping](docs/themes.md) |
| Language | English and Simplified Chinese; follow the system or switch instantly in Settings → Language, with the selection saved between launches |
| Text and source | Basic editing, syntax highlighting, find/replace, automatic and explicit saving, and drafts |
| Documents and data | Markdown/HTML views, structured data, CSV/TSV tables, SQLite snapshots, and OpenAPI indexes |
| Images and books | Raster images, SVG, GIF, electronic books, and outline/graph readers |
| Office | DOCX/PPTX/ODT/ODP text reading and XLSX/XLSM saved-value tables |
| PDF | On-demand page images from an immutable private snapshot, page navigation, and image detail |
| Archives | ZIP, TAR, and common compressed streams |

Windows builds and basic startup have been verified locally. Build workflows are configured for Windows, macOS ARM/Intel, and Linux; macOS and Linux native execution have not yet been verified.

This is not a completed port or a published release. PDF text search/selection, image-only and complex Office documents, legacy Office/iWork formats, font previews, full audio/video playback, OCR, and remaining format/platform gaps still require implementation or acceptance. The format catalog lists candidate routes, not a guarantee of complete support. See [feature status](docs/support.md) for details.

## Build and run

Install Rust, Python 3, and the native tools for your platform:

| Platform | Build requirements |
| --- | --- |
| Windows | MSVC Rust toolchain, Visual Studio C++ Build Tools, a Windows SDK, and CMake |
| macOS | Xcode Command Line Tools |
| Linux | Native development libraries listed in the [build workflow](.github/workflows/build.yml) |

A working graphics driver and a graphical desktop session are required to open the application.

```sh
cargo run --locked -p superfile
cargo run --locked -p superfile -- /absolute/path/to/file-or-folder
```

Windows example:

```powershell
cargo run --locked -p superfile -- 'C:\Users\name\Documents'
```

Build artifacts are stored in the project's `target/` directory. User data and caches use platform directories under `Superfile`. For isolated development or validation, set `SUPERFILE_DATA_DIR` and `SUPERFILE_CACHE_DIR` to absolute paths.

Downloaded themes use the same home-relative `.superfile/themes/` directory on Windows, macOS, and Linux, shared by Debug and Release builds. The theme browser shows the path and can open the folder. An explicit `SUPERFILE_DATA_DIR` override places themes in its `themes/` subdirectory. See [theme storage and migration](docs/themes.md).

Application artwork comes from `assets/branding/superfile-source.png`, the left document/magnifier icon in the supplied reference. Its [source and extraction prompt](assets/branding/icon-source.json) are recorded alongside it. Run `NODE_PATH=/path/to/node_modules node scripts/render-icons.mjs` with the optional `sharp` package to generate PNG, SVG, and Windows ICO assets; macOS also generates ICNS. Normal builds use the committed resources directly.
On macOS, startup sets the Dock icon from the embedded ICNS, including direct `cargo run` launches without an `.app` bundle.

## Tag releases

Commit and push the workflow and packaging scripts first, then tag that commit:

```sh
git tag v0.0.3
git push origin v0.0.3
```

The [release workflow](.github/workflows/build.yml) accepts tags such as `v1.2.3`, `1.2.3`, and `v1.2.3-rc.1`. Prerelease tags create GitHub Prereleases. CI synchronizes only workspace package versions in its own Cargo manifest and lockfile, so the application's version matches the tag without updating external dependencies or writing changes back to the repository.

| Target | Release assets |
| --- | --- |
| Windows x64 | NSIS `-setup.exe` installer and portable `.zip` |
| Linux x64 | `.deb` installer and portable `.tar.gz` |
| macOS Apple Silicon | ARM64 `.dmg` installer and `.tar.gz` application bundle |
| macOS Intel | x86_64 `.dmg` installer and `.tar.gz` application bundle |

All four native builds run workspace tests, Clippy, and release compilation. Publication requires all builds and all eight packages to succeed. It adds `SHA256SUMS.txt`, generates release notes, uploads the assets to the matching GitHub Release, and verifies uploaded names and sizes. A failed platform blocks publication. Only the publish job has `contents: write`; it uses GitHub's automatic [`GITHUB_TOKEN`](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token), with no additional PAT required.

Linux packages are built on Ubuntu 24.04; `.deb` dependencies are derived from the binary, and portable archives still require compatible system libraries and graphics drivers. Windows code signing and Apple Developer ID signing/notarization are not configured; macOS bundles receive an ad-hoc signature. Confirm the first remote run in GitHub Actions; failed jobs can be rerun there.

Check the packaging helpers locally (Python 3.11 or later):

```sh
python -m unittest discover -s scripts/package -p 'test_*.py' -v
```

Packagers require the exact binary under `target/<target>/release/` and never fall back to debug or another architecture. `SUPERFILE_DIST_DIR` isolates package output; `SUPERFILE_PACKAGE_BINARY` is an explicit input override for local packaging smoke tests and is not set by the workflow.

## Checks

```sh
python scripts/check-locales.py
python scripts/check-architecture.py
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked -p superfile-core --lib search::tests
cargo test --locked -p superfile-services --test portable_documents
```

These checks cover the selected code paths; they do not replace native UI acceptance on each platform. Tag workflows produce release installers and portable packages; signing and notarization limits are described above.

## Project layout

```text
crates/
  superfile-core/       Document, table, graph, search, and session models
  superfile-services/   File I/O, parsing, paging, saving, and background services
  superfile-app/        The superfile executable and gpui-kit interface
assets/                Icons, format catalog, locales, and reader resource policy
scripts/               Build helpers and static checks
```

See [architecture](docs/architecture.md) and [implementation plan](PLAN.md) for module boundaries and remaining work. These detailed development documents are currently maintained in Chinese.

## Origin and license

Parts of the Rust parsing and reading logic originate from [Glim](https://github.com/glimapp/glim), based on commit `c8a135f7964417c6e674b1a6678ce77fe7a11516`. Superfile evolves independently and retains the reused code's [MIT license and copyright notice](LICENSE). Third-party dependencies retain their respective licenses.
