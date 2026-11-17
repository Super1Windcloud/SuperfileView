# Obsidian 社区主题配色

在「设置 → 主题配色 → 浏览主题」中搜索、下载并使用社区主题，也可以只查看已安装的主题。默认配色采用 Obsidian 的浅色/深色基础色板，默认强调色为 `#8B5CF6`。设置中的强调色选择器可覆盖当前主题的强调色，恢复按钮会返回当前主题的强调色。切换主题时清除强调色覆盖；单模式主题会切换到其支持的明暗模式。

主题目录来自 [Obsidian 官方社区目录](https://github.com/obsidianmd/obsidian-releases/blob/master/community-css-themes.json)。下载前不修改当前配色，安装成功后才切换；可以取消下载。目录和截图缓存于应用缓存目录的 `themes/`。断网时可使用缓存目录和已经安装的主题。应用会记住主题选择、强调色覆盖和明暗模式，重启后恢复；损坏或缺失的主题包回退到默认配色，并在主题页显示错误。

## 本地主题路径

下载的主题包统一保存到用户主目录的 `.superfile/themes/`，与安装位置、当前工作目录、Debug/Release 构建模式无关；开发运行和打包安装共用同一目录。

| 平台 | 默认目录 |
| --- | --- |
| Windows | `%USERPROFILE%\.superfile\themes\` |
| macOS | `$HOME/.superfile/themes/` |
| Linux | `$HOME/.superfile/themes/` |

主题页显示实际目录，并提供「打开主题文件夹」按钮。每个 `<主题仓库 SHA-256>.json` 包含完整 CSS 和主题元数据，离线读取无需重新下载；截图和目录列表属于可重新下载的缓存。

旧版本应用数据目录中的 `themes/*.json` 会在读取已安装主题、安装主题或打开主题文件夹时自动复制到新目录，保持主题 ID 和已保存选择不变。迁移保留原文件，不覆盖新目录中的同 ID 文件；损坏或身份不符的旧包会跳过并记录日志。

显式设置 `SUPERFILE_DATA_DIR` 时，主题包改存于该绝对路径下的 `themes/`，便于隔离开发与测试；Debug 和 Release 对同一环境变量使用相同路径。`SUPERFILE_CACHE_DIR` 只控制缓存，不改变已安装主题的位置。

## 兼容范围

Superfile 映射 [Obsidian 颜色变量](https://docs.obsidian.md/Reference/CSS+variables/Foundations/Colors)，并非 Obsidian 的 HTML/CSS 运行时。兼容背景、文字、边框、按钮、选中态、导航、标签和基础代码高亮颜色。支持 `body`、`:root`、`.theme-light`、`.theme-dark`、桌面和当前平台根选择器、常见 `:not/:is/:where` 条件、颜色模式媒体查询、变量引用与回退、十六进制/RGB/HSL、`calc` 算术，以及 CSS Color 4 和 `color-mix` 表达式。

布局、字体、组件专用选择器、CSS 嵌套、插件和 Style Settings 扩展不执行，主题的可选变体不自动启用。无法解析的语义颜色使用默认值；已安装主题卡片的「颜色变量」提示可查看当前模式的映射数和回退数。因此社区截图用于展示原主题，Superfile 的实际布局仍保持简约界面。不同平台可能因主题中的平台条件而得到不同颜色。

下载使用进程内 Rust HTTPS 客户端，只访问 GitHub 原始内容域名，不跟随重定向，也不加载 CSS 的 `@import` 或远程资源。目录、CSS 和截图分别限制为 4、16、8 MiB；截图在后台解码为受资源预算约束的 PNG 缩略图，画廊只为可见卡片请求截图，最多同时处理四张。下载包以临时文件原子替换，解析和安装均在后台执行。

## 验证真实主题

以下命令只写指定的审计数据和缓存目录，下载后会重新打开安装包并打印两种模式的映射及回退情况：

```sh
SUPERFILE_DATA_DIR="$PWD/target/community-audit-data" \
SUPERFILE_CACHE_DIR="$PWD/target/community-audit-cache" \
cargo run -p superfile-services --locked --example audit_themes -- \
  Minimal Things AnuPpuccin 'Blue Topaz'
```
