# Superfile 桌面文件渲染选型与迁移顺序

2026-11-20，基于 Superfile `33f16b3`。建议先做 DOCX 和 PPTX 的静态原版式预览，分别试用 WordCraft 与 DeckCraft 的库接口。PDF 继续用 PdfCraft；三者共享窗口内的连续页浏览体验，各自负责解释文件、排版并画出页面。

[逐格式 CSV（300 行，可按后缀或优先级筛选）](renderer-selection.zh-CN.csv) 覆盖目录的全部 **232 个后缀**，另列出 51 个文件名或模式、6 个复合归档别名、5 个可执行文件示例或签名、4 个 SQLite 模式以及 OpenAPI 探测和通用回退。文件关联、图标和遥测中出现的名称不单独算作渲染能力。基线来自[格式目录](format-catalog.zh-CN.md)、[服务路由](../crates/superfile-services/src/documents/registry.rs#L15-L82)、[文本投影](../crates/superfile-services/src/documents/presentation.rs#L46-L88)和[文件名规则](../crates/superfile-core/src/document.rs#L28-L103)。

## 此次迁移后的打开行为

上文的统计和逐格式 CSV 记录选型时的基线。当前工作区已经接入以下显示路径。

| 文件 | 打开方式与主题行为 |
| --- | --- |
| PDF、DOCX、ODT、PPTX、ODP | PDF 用 PdfCraft 连续分页。Office 通过 Rust 库 lo_writer、lo_impress 生成临时 PDF，复用页缓存、滚动和阅读配色。可切回原稿颜色检查照片或品牌色。 |
| DOC、PPT、RTF、Pages、Numbers、Keynote | 旧二进制 Office 提取结构化内容，RTF 转富文本。iWork 呈现可提取的正文、表格或幻灯片。均使用编辑器的阅读底色。 |
| XLS、ODS、XLSX、XLSM | 前两种经 calamine 读取，四种进入可分页的工作表。字体文件以实际字形轮廓生成样张，并使用文档页配色。 |
| 归档 | 7z、RAR、CAB、CPIO、AR、XAR、ISO、LHA/LZH 以及 LZMA、LZIP、Unix compress 和对应的 TAR 别名经过进程内解码，使用原有的归档目录和提取入口。 |
| 音视频 | 音频由 Rodio 输出，视频由 DeckCraft Media 解码。控件采用主题色，底层视频像素保留颜色。能否播放取决于实际编码、音频设备和解码预算。 |
| 标记文本、科学数据和图片 | AsciiDoc、Org 使用专用解析器。Notebook 展示保存的 Markdown、代码、文本和静态图片，字幕按时间段展示。NPY 读取 dtype 和字节序，ICNS 可读取旧式图像和透明蒙版。 |
| 图表 | Mermaid、D2、DOT、Pikchr、PlantUML、WaveDrom、数学公式在本地生成图形。Vega 静态预览支持内联数据、线性或分段类别比例尺、左轴和底轴、矩形、圆点、线段与文字。使用外部数据、变换或交互表达式时，读者可查看原始规范。 |

归档实现按输入选择。AR 用 object 读取 GNU/BSD 长文件名，ISO 使用 hadris-iso，Unix compress 使用 newtua-lzw-z，XAR 通过 flate2 和 roxmltree 有界读取目录。ISO 已验证 ISO9660、Joliet 与 Rock Ridge，CPIO 已验证 newc；仅含 UDF 的镜像、旧式 CPIO、加密或跨卷 RAR 尚未覆盖。播放回归样本中的压缩音频和视频来自 [DeckCraft 的 CC0 合成样本](https://github.com/storytold/deckcraft/tree/5b422e8dc5fb24558f90175475d68cdc03a29e19/crates/media/tests/fixtures)。

主题切换的验证按内容区分。纸张和文墨检查实际页面像素，图表检查换色后的 SVG 或栅格，照片和视频检查原色。完整 WPS 版面对照、实际声卡输出以及三平台原生窗口验收需要各自的运行证据。

## 迁移顺序

| 顺序 | 目录后缀数 | 交付目标 | 验收依据 |
| --- | ---: | --- | --- |
| P0 | 2 | 先 DOCX，再 PPTX。前者交付分页、表格与图片，后者交付带母版和层叠顺序的静态幻灯片 | 相同字体下对照 WPS 导出的参考 PDF，同时记录首屏时间、连续翻页和峰值内存 |
| P1 | 19 | 先用 calamine 补 XLS/ODS 网格，再复用 P0 引擎补 DOC、PPT、ODT；随后补 7z/RAR、OpenType 字体样张和音频播放 | 从现有打开入口验证对应文件；旧 Office、加密归档、字体集合及音频设备分别验收 |
| P2 | 28 | iWork、RTF、ODP、其余缺失归档、DFONT、旧 ICNS、NPY、Notebook、字幕和部分标记语言 | 各后缀的目标和输入变体见 CSV；按真实样本和依赖成本逐项排期 |
| 保留 | 178 | PDF、XLSX/XLSM、常用图片、图书、文本与代码、结构化数据等沿用当前路径 | 维持现有大文件分页、源文编辑、取消、尺寸预算及离线访问 |
| 暂缓 | 5 | MP4、M4V、MOV、MKV 的通用视频播放；完整 Vega 规范渲染 | 先测试真实 codec、帧输出和同步，Vega 另验规范到场景图的完整性 |

## Office 和 PDF

| 格式 | 当前打开时实际看到什么 | 推荐与备选 |
| --- | --- | --- |
| PDF | 上一修订直接调用 Hayro 渲染选定页。当前锁定的 PdfCraft 0.21.1 按页输出像素，内存不足以保留输入副本时走同修订的 Hayro 映射路径。GPUI 列表负责上下连续滚动 | 保留 PdfCraft 与虚拟页列表。WPS 式滚动的验收在视图层进行，另测页边距、混合尺寸、快速跨页与滚动回看。[服务端渲染](../crates/superfile-services/src/pdf.rs#L30-L130) · [页列表](../crates/superfile-app/src/views/reader/pdf.rs#L183-L224) · [上游](https://github.com/storytold/pdfcraft) |
| DOCX | 解包并提取正文 XML 的文字，用 Markdown 视图显示。原分页、页眉和浮动图片会丢失，纯图片文档可能提示没有可读文字 | 首选 WordCraft 的 DOCX 导入、分页和 `render_page`，将像素交给 GPUI。它还有区域渲染接口，适合以后测高倍率缩放。[现行提取](../crates/superfile-services/src/rich_documents.rs#L42-L138) · [现行视图](../crates/superfile-app/src/views/reader/document.rs#L85-L95) · [WordCraft 页面接口](https://github.com/storytold/wordcraft/blob/main/crates/render/src/lib.rs) |
| PPTX | 读取 slide XML 中的文字，再用 Markdown 展示。全图幻灯片和文字遮挡关系尤其容易失真 | 首选 `deckcraft-pptx` 与 `deckcraft-render`。逐张呈现固定画布，先覆盖母版、图片裁切、组合形状与文本框。演示动画、视频和放映控制单列。[DeckCraft](https://github.com/storytold/deckcraft) · [渲染库](https://github.com/storytold/deckcraft/tree/main/crates/render) |
| DOC、PPT、RTF、ODT、ODP | DOC/PPT/RTF 进入文档窗口但缺少便携读取分支；ODT/ODP 先命中 ZIP 容器报告，其中可含提取的正文 | DOC、RTF、ODT 优先复用 WordCraft，PPT 复用 DeckCraft。ODP 单独验证 libreoffice-rs 的 ODF/Impress 库；ODT 可用 OpenDoc 比对。[容器报告](../crates/superfile-services/src/formats.rs#L47-L81) · [WordCraft](https://github.com/storytold/wordcraft) · [OpenDoc](https://casualoffice.org/opendoc/) · [libreoffice-rs](https://github.com/clark-labs-inc/libreoffice-rs) |
| XLS、ODS、XLSX、XLSM | XLS 缺工作表入口，ODS 是容器报告；XLSX/XLSM 已把工作表和共享字符串建为磁盘索引，可分页浏览单元格和已存公式结果 | XLS/ODS 用 calamine 读取后接现有表格视图，限制整表展开；XLSX/XLSM 保留流式索引。若以后需要展示图表、打印区和分页，再将 GridCraft 与原网格比较。[现有工作簿](../crates/superfile-services/src/spreadsheet.rs#L110-L120) · [calamine](https://github.com/tafia/calamine) · [GridCraft](https://github.com/storytold/gridcraft) |
| Pages、Numbers、Keynote | 能识别包，缺少正文、表格或幻灯片视图；普通文本也可能使用 .key | 首选 iwork 解析 IWA 包，分别投影为段落、表格、幻灯片摘要。有内嵌预览时可加封面。当前表示的是结构阅读目标，原版式仍需另一组样本验证。[包探测](../crates/superfile-services/src/preview.rs#L45-L68) · [iwork](https://docs.rs/iwork/latest/iwork/) |

**共同备选是 office2pdf → PdfCraft。** 它可把 DOCX/PPTX 转成 PDF，通过同一个阅读器交付固定页面，适合用作两项 P0 的对照。如果专用引擎必须引入编辑器 UI、与项目锁定的 PdfCraft 补丁发生冲突，或三平台字体处理成本过高，就优先评估这条路径。需要实测整文转换带来的首屏延迟、临时文件和 Typst 依赖体积。文字提取继续作为布局失败时的可读入口。office_oxide 适合旧文档的语义提取；docx-rs 的主要用途是读写 OOXML。这两个方向单独都不能交付可对照 WPS 的页面。[office2pdf](https://github.com/developer0hye/office2pdf) · [office_oxide](https://office.oxide.fyi/) · [docx-rs](https://crates.io/crates/docx-rs)

## 其余阅读器

| 文件家族 | 推荐的显示路径 | 选择依据 |
| --- | --- | --- |
| 代码、配置、纯文本与日志 | 继续使用分页文本、GPUI Kit 编辑器和现有语言映射。CSV 为每个代码后缀写明当前使用的语言，包括以 HTML 规则显示的 Vue 等别名 | 能保留编辑、定位和大文件阅读；同一语法可服务多个后缀。JSON 方言、Notebook、OpenAPI 仍按入口区别处理。[语言映射](../crates/superfile-core/src/document.rs#L28-L103) · [GPUI Kit](https://docs.rs/gpui-kit) |
| Markdown、HTML、书籍 | Markdown 保留带源位置的 AST；HTML 保留离线富文本；EPUB/FB2 保留章节、导航及资源解析；CBZ 走漫画页 | pulldown-cmark 下载量高，但迁移需重建任务勾选、数学节点及源码范围。书籍另有阅读位置和插图语义。[Markdown 处理](../crates/superfile-services/src/markdown_preview.rs#L17-L55) · [书籍结构](../crates/superfile-services/src/books/mod.rs#L22-L73) |
| 表格、树、数据库与科学数据 | CSV/TSV 用 csv 和网格；JSON/YAML/XML 用现有方言解析和有界结构视图；SQLite 用 rusqlite 快照；NPY 候选 npyz；Safetensors 保留元数据读取 | 限制展开量，保持排序、分页及 WAL 主库归属。大型权重文件以形状、类型和偏移为有效预览。[SQLite 快照](../crates/superfile-services/src/sqlite.rs#L54-L88) · [NPY 数值报告](../crates/superfile-services/src/formats.rs#L131-L224) |
| 图片、字体与 3D | 常规位图保留 image，GIF 保留逐帧合成，SVG 保留 resvg。用 icns 补旧图块，用 skrifa 画实际字体样张；OBJ/STL/BDF 保留静态投影 | 普通图片缩略图、动画合成、字形轮廓与网格投影的输出各异。DFONT 先读资源容器再取字体。[图像分派](../crates/superfile-services/src/inline_images.rs#L26-L90) · [skrifa](https://docs.rs/skrifa) |
| 音频与视频 | 音频用 Symphonia 解码、CPAL 输出，GPUI 持有播放控制状态。视频先试 DeckCraft 使用的 craft-media/craft-video，逐项测试 H.264、HEVC、VP9、AV1 | M4A、CAF、WAV 需要匹配内部编码。MP4/MOV/MKV 还涉及帧呈现、同步和 seek。CPAL 需要系统音频 API；FFmpeg 和 GStreamer 的 Rust 包依赖原生库。仅凭可解封装无法承诺播放。[Symphonia 支持表](https://github.com/pdeljanov/Symphonia) · [CPAL 平台说明](https://github.com/RustAudio/cpal) · [DeckCraft 实际 codec 特性](https://github.com/storytold/deckcraft/blob/main/Cargo.toml) |
| 归档 | 保留现有 ZIP、TAR 和压缩流。优先以 sevenz-rust2 补 7z、rars 补 RAR。CAB、CPIO、ar、XAR、ISO、LHA、LZIP 和 Unix compress 在 CSV 中逐个映射 | 当前 7z/RAR 等已声明候选，但缺少便携解码分支。新解码器复用现有目录视图、路径检查和取消逻辑，按压缩算法验证。[当前支持判定](../crates/superfile-services/src/archive.rs#L25-L32) · [sevenz-rust2](https://docs.rs/sevenz-rust2) · [rars](https://docs.rs/rars) |
| 图表、地理、字幕及其他标记 | Draw.io、XMind、OPML 继续现有专用视图。D2 可试 d2-render 的 native 子集，Pikchr 可试 pikru。SRT/VTT 用 subtp，GPX/KML 保留离线坐标和表格 | 图表子进程当前仅实现 Mermaid，识别 DOT、PlantUML、ZenUML、flowchart、WaveDrom 和数学围栏不等于已有画面。DOT 可评估 layout-rs；其余在有可验证库与样本前保留源码。Vega 的 vl-convert-rs 需要内嵌 V8，Avenger 接收场景图，完整规范渲染暂缓。[图表分派](../crates/superfile-app/src/app/diagram_engine.rs#L31-L39) · [D2](https://docs.rs/d2-render/latest/d2_render/native/) · [Pikru](https://docs.rs/pikru) · [Vega 转换器](https://github.com/vega/vl-convert) · [Avenger](https://github.com/jonmmease/avenger) |
| 可执行文件与未知格式 | PE/ELF/Mach-O 用 object 生成只读报告；其他通过文件内容进入文本或字节分页 | 后缀缺失也尝试签名，保留跨平台和未知文件的可读入口。[签名探测](../crates/superfile-services/src/executables.rs#L125-L179) · [文本与字节回退](../crates/superfile-services/src/documents/mod.rs#L135-L172) |

## 流行度与适配依据

下表记录 2026-11-20 读取的 crates.io `recent_downloads`（近 90 天、全版本，含传递依赖与 CI）。列出的发行版本与日期对应查询时查到的非撤回稳定发行记录，供选型追溯，项目的锁定版本以 Cargo.lock 为准。下载量适合比较同一职责的候选；格式覆盖、页面保真、内存模型和可嵌入的库接口决定最终取舍。

以 Git 依赖为主的项目另看同日仓库关注度。PdfCraft **21,288** stars，WordCraft **18,309**，DeckCraft **4,890**，GridCraft **7,697**，office2pdf **152**。这些数字统计整个项目。WordCraft 和 DeckCraft 有独立的排版或栅格化库，才使它们具备接入现有 GPUI 阅读器的条件。[PdfCraft](https://github.com/storytold/pdfcraft) · [WordCraft](https://github.com/storytold/wordcraft) · [DeckCraft](https://github.com/storytold/deckcraft) · [GridCraft](https://github.com/storytold/gridcraft) · [office2pdf](https://github.com/developer0hye/office2pdf)

| 用途 | Rust 包 | 稳定发行记录 | 发行日期 | 近 90 天下载 |
| --- | --- | --- | --- | ---: |
| PDF 后端对照 | [hayro](https://crates.io/crates/hayro) | 0.9.0 | 2026-11-02 | 4,172,838 |
| PDF 后端对照 | [pdfium-render](https://crates.io/crates/pdfium-render) | 0.9.5 | 2026-11-01 | 2,094,524 |
| Office 格式转换与语义读取 | [office2pdf](https://crates.io/crates/office2pdf) | 0.11.0 | 2026-11-14 | 6,943 |
| Office 格式转换与语义读取 | [libreoffice-pure](https://crates.io/crates/libreoffice-pure) | 0.5.8 | 2026-08-26 | 2,351 |
| Office 格式转换与语义读取 | [office_oxide](https://crates.io/crates/office_oxide) | 0.1.17 | 2026-11-11 | 916,597 |
| Office 格式转换与语义读取 | [docx-rs](https://crates.io/crates/docx-rs) | 0.4.22 | 2026-07-21 | 788,395 |
| Office 格式转换与语义读取 | [calamine](https://crates.io/crates/calamine) | 0.38.0 | 2026-11-02 | 7,700,229 |
| Office 格式转换与语义读取 | [iwork](https://crates.io/crates/iwork) | 0.4.0 | 2026-10-08 | 781 |
| 文本、表格与标记 | [gpui-kit](https://crates.io/crates/gpui-kit) | 0.10.0 | 2026-11-16 | 448,541 |
| 文本、表格与标记 | [tree-sitter](https://crates.io/crates/tree-sitter) | 0.27.2 | 2026-11-07 | 24,043,769 |
| 文本、表格与标记 | [markdown](https://crates.io/crates/markdown) | 1.0.0 | 2025-04-23 | 6,258,453 |
| 文本、表格与标记 | [pulldown-cmark](https://crates.io/crates/pulldown-cmark) | 0.13.6 | 2026-10-30 | 76,304,302 |
| 文本、表格与标记 | [html5ever](https://crates.io/crates/html5ever) | 0.40.1 | 2026-09-14 | 61,975,208 |
| 文本、表格与标记 | [csv](https://crates.io/crates/csv) | 1.4.0 | 2025-10-17 | 67,692,405 |
| 文本、表格与标记 | [quick-xml](https://crates.io/crates/quick-xml) | 0.42.0 | 2026-08-22 | 185,281,689 |
| 文本、表格与标记 | [serde_json](https://crates.io/crates/serde_json) | 1.0.154 | 2026-10-11 | 451,296,240 |
| 文本、表格与标记 | [serde_yaml_ng](https://crates.io/crates/serde_yaml_ng) | 0.10.0 | 2024-05-26 | 17,284,278 |
| 图片、字体与绘图 | [image](https://crates.io/crates/image) | 0.25.10 | 2026-03-10 | 86,370,614 |
| 图片、字体与绘图 | [gif](https://crates.io/crates/gif) | 0.14.2 | 2026-04-09 | 61,532,283 |
| 图片、字体与绘图 | [resvg](https://crates.io/crates/resvg) | 0.50.0 | 2026-11-11 | 17,607,620 |
| 图片、字体与绘图 | [icns](https://crates.io/crates/icns) | 0.5.0 | 2026-09-14 | 223,416 |
| 图片、字体与绘图 | [skrifa](https://crates.io/crates/skrifa) | 0.50.0 | 2026-11-10 | 32,142,898 |
| 图片、字体与绘图 | [mermaid-rs-renderer](https://crates.io/crates/mermaid-rs-renderer) | 0.3.1 | 2026-07-06 | 218,111 |
| 图片、字体与绘图 | [d2-render](https://crates.io/crates/d2-render) | 0.1.0 | 2026-10-18 | 696 |
| 音视频 | [symphonia](https://crates.io/crates/symphonia) | 0.6.1 | 2026-08-13 | 11,862,233 |
| 音视频 | [cpal](https://crates.io/crates/cpal) | 0.18.2 | 2026-08-16 | 11,349,110 |
| 音视频 | [rodio](https://crates.io/crates/rodio) | 0.23.0 | 2026-10-11 | 4,115,915 |
| 音视频 | [ffmpeg-next](https://crates.io/crates/ffmpeg-next) | 9.1.0 | 2026-11-11 | 2,665,587 |
| 音视频 | [gstreamer](https://crates.io/crates/gstreamer) | 0.25.4 | 2026-09-21 | 3,719,556 |
| 归档与压缩 | [zip](https://crates.io/crates/zip) | 9.8.1 | 2026-11-20 | 109,428,365 |
| 归档与压缩 | [tar](https://crates.io/crates/tar) | 0.4.46 | 2026-05-18 | 83,062,647 |
| 归档与压缩 | [sevenz-rust2](https://crates.io/crates/sevenz-rust2) | 0.25.0 | 2026-11-06 | 1,592,282 |
| 归档与压缩 | [rars](https://crates.io/crates/rars) | 0.13.0 | 2026-11-17 | 106,916 |
| 归档与压缩 | [cab](https://crates.io/crates/cab) | 0.6.0 | 2024-04-05 | 2,254,412 |
| 归档与压缩 | [cpio](https://crates.io/crates/cpio) | 0.4.2 | 2026-11-01 | 273,389 |
| 归档与压缩 | [ar](https://crates.io/crates/ar) | 0.9.0 | 2021-08-16 | 2,213,518 |
| 归档与压缩 | [apple-xar](https://crates.io/crates/apple-xar) | 0.20.0 | 2024-11-29 | 230,528 |
| 归档与压缩 | [oxiarc-archive](https://crates.io/crates/oxiarc-archive) | 0.4.2 | 2026-09-12 | 7,272 |
| 归档与压缩 | [delharc](https://crates.io/crates/delharc) | 0.8.0 | 2026-07-22 | 83,179 |
| 归档与压缩 | [lzma-rust2](https://crates.io/crates/lzma-rust2) | 0.25.0 | 2026-11-01 | 16,746,383 |
| 归档与压缩 | [unarc-rs](https://crates.io/crates/unarc-rs) | 0.8.1 | 2026-10-29 | 4,841 |
| 数据与二进制 | [rusqlite](https://crates.io/crates/rusqlite) | 0.43.0 | 2026-11-15 | 65,299,067 |
| 数据与二进制 | [object](https://crates.io/crates/object) | 0.41.0 | 2026-11-10 | 152,343,891 |
| 数据与二进制 | [safetensors](https://crates.io/crates/safetensors) | 0.8.0 | 2026-06-09 | 15,557,829 |
| 数据与二进制 | [npyz](https://crates.io/crates/npyz) | 0.9.1 | 2026-05-02 | 1,657,309 |
| 数据与二进制 | [plist](https://crates.io/crates/plist) | 1.10.4 | 2026-11-16 | 45,333,042 |

## 实施验收

1. **先做一个 DOCX 原型。** 固定 WordCraft 修订，只取导入、排版、字体和渲染库，由 services 在后台产生页尺寸和像素，app 复用连续页列表。用包含中文字体、页眉页脚、跨页表格、浮动图片及纯图片页的文档，以相同字体的 WPS 导出结果逐页对照。记录页数、漏字、错位、冷启动首屏、峰值内存和安装包增量。真机验收分别覆盖 Windows、macOS 和 Linux。
2. **再接 PPTX 和旧格式。** 为同一个页列表接入 DeckCraft 的静态画布。比较母版、文本框与图片裁切。检查两个上游各自固定的 craft-libs、字体资源及根目录补丁，确认库级依赖可复现构建。P0 通过后分别复用其布局能力验证 DOC、PPT、ODT 和 RTF。
3. **分别验证大文件与平台后端。** 电子表格用大稀疏表测读取峰值、取消与跨页访问。音频在 Windows 音频设备、macOS CoreAudio 和 Linux 音频后端实播。归档用超大字典、加密、分卷和坏头测试。已有 rusqlite 链接内置 SQLite，xz2、zstd 经 Rust 接口调用原生实现；新增依赖要核对解析后的实际特性及分发文件。[当前依赖](../crates/superfile-services/Cargo.toml#L24-L81)
4. **验收粒度到具体格式及行为。** 在同一输入上比较文件打开入口、首屏、滚动或定位、缩放、取消和关闭后的内存回收。对照结果通过后再更新用户可见的格式帮助。此次交付记录了源码行为、候选上游能力和流行度，候选库的三平台构建、视觉对照和性能测量属于迁移阶段。
