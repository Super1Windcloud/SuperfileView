# Superfile Development Guidelines & System Agents

## 1. Project Overview & Scope
`superfile` is a high-performance, cross-platform desktop file viewer and editor for documents, code, images, books, audio/video, and structured data, built in Rust using `gpui-kit`.
- **Supported Platforms**: Windows, macOS, and Linux.
- **Architectural Principle**: Single unified codebase across all platforms. No platform-exclusive host shell/binary dependencies.

---

## 2. Architecture & Layering Rules
Maintain strict one-way dependency flow:
```
superfile-app -> superfile-services -> superfile-core
```
- **`superfile-core`**:
  - Contains pure data structures, format detection, and MIME resolution.
  - **Forbidden**: Never perform filesystem I/O, network calls, or import GPUI.
- **`superfile-services`**:
  - Handles paging, streaming, background caching, file parsing, and async tasks.
  - **Forbidden**: Never import GPUI or UI view types. Keep completely headless.
- **`superfile-app`**:
  - Keep `src/main.rs` thin. Place application lifecycle in `src/app/` and view components in `src/views/`.
  - **Forbidden**: Never perform blocking I/O or directory scans inside `render()` methods. Offload work to `cx.background_executor()` or service workers.

---

## 3. Cross-Platform Invariants
Every implementation must run natively and reliably on Windows, macOS, and Linux:
1. **Windows File Locking (Sharing Violations)**:
   - On Windows, open file descriptors prevent file renaming/replacement (`ERROR_SHARING_VIOLATION` / `os error 5`).
   - Always drop/close file handles and reader instances before calling `NamedTempFile::persist` or replacing target files atomically.
2. **File Timestamps & Permissions**:
   - On Windows, `File::set_times` requires write access permissions (`FILE_WRITE_ATTRIBUTES`).
   - Always open cache files with `File::options().read(true).write(true).open(...)` before setting timestamps.
3. **Pure Rust Implementations (No Host Binary Dependencies)**:
   - Never invoke host-installed CLI utilities (such as `/usr/bin/zip`, `/usr/bin/sqlite3`, `plutil`).
   - Use in-process pure Rust libraries (`zip`, `rusqlite`, pure parsers) for both production logic and test fixtures.
4. **Platform-Adaptive Keybindings**:
   - Use `format_shortcut_for_display` and `Shortcut::modifier_symbol` to show `Ctrl` / `Alt` / `Shift` on Windows & Linux, and `⌘` / `⌥` / `⇧` on macOS.
5. **Line Endings & Text Fixtures**:
   - Enforce `.gitattributes` (`* text=auto eol=lf`) to ensure multi-line test fixtures (CSV, JSON, Markdown) behave identically across Windows CRLF and Unix LF.

---

## 4. System Subagents
This project defines three specialized subagents to coordinate engineering and review:

### `rust-architect`
- **Focus**: `superfile-core` and `superfile-services`.
- **Responsibilities**:
  - Enforce crate layering invariants and pure core boundaries.
  - Manage streaming, paging, resource admission, and memory safety for arbitrarily large files.
  - Implement pure Rust decoders and format parsers.
  - Ensure lock contention safety and thread pool efficiency.

### `gpui-expert`
- **Focus**: `superfile-app` UI presentation and view lifecycle.
- **Responsibilities**:
  - Design and refine GPUI views, controls, and layout with `gpui-kit`.
  - Maintain anti-slop UI standards: strict adherence to theme tokens, consistent paddings, and semantic typography.
  - Manage platform-aware keyboard shortcut routing and display.
  - Optimize high-DPI scaling and font database resolution.

### `cross-platform-qa`
- **Focus**: Verification, test automation, and multi-OS matrix compliance.
- **Responsibilities**:
  - Execute full test matrix: `cargo test --workspace --locked`.
  - Enforce clean lints: `cargo clippy --workspace --all-targets --locked -- -D warnings`.
  - Enforce formatting: `cargo fmt --all --check`.
  - Validate architecture and locale scripts: `python scripts/check-locales.py` and `python scripts/check-architecture.py`.
  - Ensure zero regression on all target platforms.

---

## 5. Verification Commands
Run the following verification suite before submitting any changes:
```powershell
# Format check
cargo fmt --all --check

# Clippy linter
cargo clippy --workspace --all-targets --locked -- -D warnings

# Test suite (all 188+ unit & integration tests)
cargo test --workspace --locked

# Script validations
python scripts/check-locales.py
python scripts/check-architecture.py
```

---

## 6. Commit Message & Delivery Guidelines
- **All commit messages MUST be written in English.**
- Strictly adhere to the **Conventional Commits** specification:
  - Format: `<type>(<scope>): <concise summary in English>`
  - Allowed types: `feat`, `fix`, `refactor`, `perf`, `test`, `build`, `ci`, `docs`, `chore`.
- Provide clear, bulleted details covering:
  - Motivation and implementation background
  - Defensive design and cross-platform handling
  - Automated verification evidence and test results
