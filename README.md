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
- Local diagnostic logs, with no remote telemetry transport; [static-manifest application updates](docs/updates.md) without a client token.

The application does not use Swift, Quick Look, or WebView-based readers. Markdown interactions use the public plugin API; Mermaid diagrams are rendered by a Rust engine. HTML is displayed with TextView and does not execute scripts.

## Implementation status

Open **Supported File Formats** from the application menu or Settings to search supported extensions, previews, and editing capabilities.

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

Windows builds and basic startup have been verified locally. macOS startup and the update dialog have also been inspected. Build workflows cover Windows x64, macOS ARM64, and Linux x64; Linux native execution and installation/update acceptance across platforms remain pending.

Settings → Default File Types opens a searchable dialog showing system file types, descriptions, and their current default applications. Each application cell opens a dropdown of registered compatible apps. macOS/Linux apply the selection and read back the system default; Windows opens system Settings for user confirmation and requires a refresh afterward. Discovery runs in the background, supports refresh/cancellation, and reports incomplete reads. This also works during development. See [system file associations](docs/file-associations.md).

The top-right application menu provides File and View actions, Settings, community themes, default file types, keyboard shortcuts, updates, and About on all three platforms. It remains accessible on the start page, with the sidebar collapsed, and in file fullscreen. Native macOS menus share the same action definitions. Menu navigation has automated coverage and macOS smoke testing; Windows/Linux visual acceptance remains pending.

The workspace home groups quick actions, Recent Files, and Recent Folders into compact lists. Local opens are remembered across launches; reopening moves an item to the top. Each list initially shows five entries with Show more, and keeps up to 200 entries. Right-click an entry to open it in another window or remove only its history record. Existing workspace roots and saved local file tabs are imported once; multi-folder and SSH workspaces remain available separately. History uses the shared application database in both Debug and Release.

The published 0.x releases remain under active development. PDF text search/selection, image-only and complex Office documents, legacy Office/iWork formats, font previews, full audio/video playback, OCR, and remaining format/platform gaps still require implementation or acceptance. The format catalog lists candidate routes, not a guarantee of complete support. See [feature status](docs/support.md) for details.

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

Debug uses incremental compilation and skips debug symbols. For a debugger session, run `cargo --config 'profile.dev.debug=2' run --locked -p superfile`. Release keeps level-3 optimization and ThinLTO, with parallel code generation for the application crate. Run `cargo build --release --locked -p superfile --timings` to measure a release build.

Windows example:

```powershell
cargo run --locked -p superfile -- 'C:\Users\name\Documents'
```

Build artifacts are stored in the project's `target/` directory. User data and caches use platform directories under `Superfile`. For isolated development or validation, set `SUPERFILE_DATA_DIR` and `SUPERFILE_CACHE_DIR` to absolute paths.

Downloaded themes use the same home-relative `.superfile/themes/` directory on Windows, macOS, and Linux, shared by Debug and Release builds. The theme browser shows the path and can open the folder. An explicit `SUPERFILE_DATA_DIR` override places themes in its `themes/` subdirectory. See [theme storage and migration](docs/themes.md).

Application artwork comes from `assets/branding/superfile-source.png`, the left document/magnifier icon in the supplied reference. Its [source and extraction prompt](assets/branding/icon-source.json) are recorded alongside it. Run `NODE_PATH=/path/to/node_modules node scripts/render-icons.mjs` with the optional `sharp` package to generate PNG, SVG, and Windows ICO assets; macOS also generates ICNS. Normal builds use the committed resources directly.
On macOS, startup sets the Dock icon from the embedded ICNS, including direct `cargo run` launches without an `.app` bundle.

For macOS file-association testing, build the separately identified **Superfile Debug.app** with `python3 scripts/package/macos/bundle.py --debug` and open the resulting bundle once. The picker reads the applications registered with Launch Services.

## Tag releases

From the Superfile checkout, use `just` with Git, Rust, and `uv` (Python 3.11+):

```sh
just publish-plan           # Preview the next patch release
just publish                # Publish the next stable patch
just publish minor          # Increment minor and reset patch to zero
just publish major          # Increment major and reset minor and patch to zero
just publish v1.2.3         # Or publish an explicit stable version
just publish-resume v1.2.3  # Continue an existing tag after an interruption
```

Use `just publish-plan minor` or `just publish-plan major` to check the target version. `just publish` runs the workspace checks, commits the version and lockfile, pushes `main` and the tag atomically, waits for GitHub Actions, verifies all release assets, and synchronizes Hyperscoop. The default Hyperscoop checkout is `~/demo/hyperscoop`; set `SUPERFILE_HYPERSCOOP` to use another location with its bucket submodule initialized. Git uses the repositories' configured credentials. The public GitHub reads can also use an existing `GH_TOKEN` to avoid anonymous rate limits. If Actions fails, rerun the failed workflow before resuming the same tag.

To trigger the packaging workflow manually, push the workflow and packaging scripts first, then tag that commit:

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

Changes to Cargo manifests, the lockfile, toolchain configuration, or the release workflow on `main` warm the three Release build caches. Tag builds restore these caches without uploading tag-scoped copies. The tag workflow validates the version, compiles Release binaries, packages and validates artifacts, and uploads the release. Workspace tests, Clippy, formatting and repository audits are development checks outside this publishing workflow. Publication requires all builds and all six packages to succeed. It adds `SHA256SUMS.txt` and `latest.json` (8 assets in total), generates release notes, uploads the assets to the matching GitHub Release, and verifies uploaded names and sizes. A failed platform blocks publication. Only the publish job has `contents: write`; it uses GitHub's automatic [`GITHUB_TOKEN`](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token), with no additional PAT required.

Automated releases publish macOS Apple Silicon packages; Intel macOS can still be built and packaged manually from source. This matrix applies to tags created after the workflow change; v0.0.4 keeps its original four-target workflow.

Linux packages are built on Ubuntu 24.04; `.deb` dependencies are derived from the binary, and portable archives still require compatible system libraries and graphics drivers. Windows code signing and Apple Developer ID signing/notarization are not configured; macOS bundles receive an ad-hoc signature. Confirm the first remote run in GitHub Actions; failed jobs can be rerun there.

Check the packaging helpers locally (Python 3.11 or later):

```sh
python -m unittest discover -s scripts/package -p 'test_*.py' -v
```

Packagers require the exact binary under `target/<target>/release/` and never fall back to debug or another architecture. `SUPERFILE_DIST_DIR` isolates package output; `SUPERFILE_PACKAGE_BINARY` is an explicit input override for local packaging smoke tests and is not set by the workflow.

## Application updates

The updater uses [`gpui-updater-core`](https://github.com/AprilNEA/gpui-updater) with a static [latest.json](https://github.com/Super1Windcloud/SuperfileView/releases/latest/download/latest.json), without GitHub API calls or an embedded token. By default, every launch checks for a newer stable version, downloads and verifies it in the background, then prompts the user to restart and update or defer. The application menu and Settings → Application → Software Update let users reopen the dialog, check manually, and disable startup updates. Installation requires confirmation and saved workspace changes. Development builds and package-manager installations can download the archive for manual use. See [update behavior and release protocol](docs/updates.md).

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
