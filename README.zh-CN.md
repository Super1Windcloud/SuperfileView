# Superfile

[English](README.md) | **简体中文**

使用 Rust 和 gpui-kit 通用组件构建的文档工作区，目标平台为 Windows、macOS 和 Linux。

Superfile 将文件、源码、文档、图片和结构化数据放在同一个工作区中。项目仍在开发，实施进度和平台验收范围见 [PLAN.md](PLAN.md) 与[当前功能状态](docs/support.md)。

作者：**superwindcloud**。

## 设计

- 各桌面平台共用 GPUI 阅读器和交互组件。
- 使用 crates.io 官方锁定依赖，不保留 `vendor` 或框架源码补丁。
- 文件 I/O 在后台执行，任务可取消，数据按页加载，阅读器状态持续保留。
- 使用独立的 Superfile 应用数据与缓存目录。
- 诊断日志保存在本地，没有远程遥测传输或自动更新器。

应用不使用 Swift、Quick Look 或基于 WebView 的阅读器。Markdown 交互使用公开插件接口，Mermaid 通过 Rust 引擎渲染。HTML 使用 TextView 显示，不执行脚本。

## 实现状态

| 范围 | 当前实现 |
| --- | --- |
| 工作区 | 文件树、多目录、标签、导航、最近工作区与会话状态 |
| 外观 | 可配置强调色（默认紫色）、Obsidian 基础配色，以及[社区主题浏览、下载和颜色变量映射](docs/themes.md) |
| 语言 | English 和简体中文；默认跟随系统，可在「设置 → 语言」切换并记住选择，无需重启 |
| 文本与源码 | 基础编辑、语法高亮、查找替换、自动/显式保存与草稿 |
| 文档与数据 | Markdown/HTML、结构化数据、CSV/TSV 表格、SQLite 快照和 OpenAPI 索引 |
| 图片与书籍 | 位图、SVG、GIF、电子书，以及大纲和图形阅读器 |
| Office | DOCX/PPTX/ODT/ODP 文本阅读，XLSX/XLSM 保存值数据表 |
| PDF | 从不可变私有快照按需生成页面图像，提供翻页和图像详情 |
| 压缩包 | ZIP、TAR 和常见压缩流 |

Windows 构建和基础启动已在本机验证。已配置 Windows、macOS ARM/Intel 和 Linux 构建工作流；macOS、Linux 的原生运行尚未验证。

项目尚未完成全部移植，也没有正式发布版本。PDF 文本搜索/选择、图片型和复杂排版 Office 文档、旧 Office/iWork、字体预览、完整音视频播放、OCR，以及剩余格式和平台差异，仍需实现或验收。格式目录表示候选路由，不保证完整支持。具体范围见[功能状态](docs/support.md)。

## 构建与运行

安装 Rust、Python 3 和平台所需的原生工具：

| 平台 | 构建要求 |
| --- | --- |
| Windows | MSVC Rust 工具链、Visual Studio C++ Build Tools、Windows SDK 和 CMake |
| macOS | Xcode Command Line Tools |
| Linux | [构建工作流](.github/workflows/build.yml)中列出的开发库 |

打开应用需要可用的图形驱动和桌面图形会话。

```sh
cargo run --locked -p superfile
cargo run --locked -p superfile -- /absolute/path/to/file-or-folder
```

Windows 示例：

```powershell
cargo run --locked -p superfile -- 'C:\Users\name\Documents'
```

构建产物存放在项目的 `target/` 目录中。用户数据和缓存使用平台目录下的 `Superfile` 子目录。开发或隔离验证时，可将 `SUPERFILE_DATA_DIR` 与 `SUPERFILE_CACHE_DIR` 设置为绝对路径。

应用图标源文件为 `assets/branding/superfile-source.png`，使用提供的左侧文件/放大镜图标。[来源与提取提示词](assets/branding/icon-source.json)随资源保存。通过 `NODE_PATH=/path/to/node_modules node scripts/render-icons.mjs` 可生成 PNG、SVG 和 Windows ICO；macOS 上还会生成 ICNS。该可选工具需要 `sharp`，正常构建直接使用已提交资源。
macOS 启动时会从内嵌 ICNS 设置 Dock 图标，因此直接 `cargo run` 也能显示图标，无需先打包 `.app`。

## Tag 自动发布

先将工作流和相关脚本提交、推送到 GitHub，再在该提交上建立版本 tag：

```sh
git tag v0.0.3
git push origin v0.0.3
```

[发布工作流](.github/workflows/build.yml)接受 `v1.2.3`、`1.2.3` 或 `v1.2.3-rc.1` 形式的 tag。预发布 tag 自动建立 GitHub Prerelease。CI 仅在自己的检出目录中同步 Cargo 工作区和锁文件里的项目版本，因此应用显示的版本与 tag 一致，不改动外部依赖或回写仓库。

| 构建目标 | Release 附件 |
| --- | --- |
| Windows x64 | NSIS `-setup.exe` 安装包、`.zip` 便携包 |
| Linux x64 | `.deb` 安装包、`.tar.gz` 便携包 |
| macOS Apple Silicon | ARM64 `.dmg` 安装包、`.tar.gz` 应用包 |
| macOS Intel | x86_64 `.dmg` 安装包、`.tar.gz` 应用包 |

四个构建均执行工作区测试、Clippy 和 release 编译，全部通过且 8 个包完整后，统一生成 `SHA256SUMS.txt` 并上传至对应的 GitHub Release，自动生成发布说明；上传后检查附件名称和大小。任一平台失败则不进入发布作业。权限只在发布作业提升至 `contents: write`，使用 GitHub 自动提供的 [`GITHUB_TOKEN`](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token)，不需要额外 PAT。

Linux 包在 Ubuntu 24.04 构建，`.deb` 根据二进制生成共享库依赖，便携包仍需要兼容的系统库和图形驱动。当前未配置 Windows 代码签名或 Apple Developer ID 签名/公证；macOS 包只做 ad-hoc 签名。首次远端运行结果需在 GitHub Actions 页面确认，失败时可直接重跑相应作业。

本地检查发布脚本（Python 3.11 及以上）：

```sh
python -m unittest discover -s scripts/package -p 'test_*.py' -v
```

打包脚本默认只读取 `target/<target>/release/` 下的精确目标二进制，不会回退到 debug 或其他架构。`SUPERFILE_DIST_DIR` 可隔离输出目录；`SUPERFILE_PACKAGE_BINARY` 只用于本地显式指定打包测试输入，工作流不设置此覆盖。

## 检查

```sh
python scripts/check-locales.py
python scripts/check-architecture.py
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked -p superfile-core --lib search::tests
cargo test --locked -p superfile-services --test portable_documents
```

这些检查覆盖指定代码路径，不能替代各平台的原生 UI 验收。Tag 工作流构建 release 安装包和便携包；签名与公证范围见上文。

## 项目结构

```text
crates/
  superfile-core/       文档、表格、图形、查找与会话模型
  superfile-services/   文件 I/O、解析、分页、保存和后台服务
  superfile-app/        superfile 可执行文件与 gpui-kit 界面
assets/                图标、格式目录、语言和阅读资源策略
scripts/               构建辅助工具与静态检查
```

模块边界与后续工作见[架构说明](docs/architecture.md)和[实施计划](PLAN.md)。详细开发文档目前使用中文维护。

## 来源与许可

部分 Rust 解析和阅读逻辑源自 [Glim](https://github.com/glimapp/glim)，参考提交为 `c8a135f7964417c6e674b1a6678ce77fe7a11516`。Superfile 独立演进，保留复用代码的 [MIT 许可与版权声明](LICENSE)。第三方依赖保留各自的许可证。
