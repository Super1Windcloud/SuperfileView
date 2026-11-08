# Cross-Platform Safety Rules

## Operating System Compatibility
The codebase targets **Windows**, **macOS**, and **Linux** as first-class citizens.

1. **No External System CLI Dependencies**:
   - Do not call Unix-only utilities (e.g. `/usr/bin/zip`, `/usr/bin/sqlite3`, `plutil`) via `std::process::Command` in production code or tests.
   - Use in-tree pure Rust crates (`zip`, `rusqlite`, etc.).

2. **File Sharing and Handle Management on Windows**:
   - Close all open file descriptors before calling `tempfile::NamedTempFile::persist` or attempting to rename/delete active files.
   - For `File::set_times`, ensure files are opened with write permission (`read(true).write(true)`).

3. **Keyboard Shortcuts**:
   - Platform modifier keys must be dynamic: use `Ctrl` / `Alt` / `Shift` on Windows and Linux, and `⌘` / `⌥` / `⇧` on macOS.

4. **Line Endings**:
   - Repository uses `.gitattributes` with `* text=auto eol=lf` to prevent line-ending discrepancies between Windows CRLF and Unix LF.
