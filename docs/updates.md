# 应用更新

Superfile 使用 [AprilNEA/gpui-updater](https://github.com/AprilNEA/gpui-updater) 的 `gpui-updater-core = 0.4.4`。`StaticManifestSource` 读取固定 HTTPS 地址：

```text
https://github.com/Super1Windcloud/SuperfileView/releases/latest/download/latest.json
```

客户端没有 GitHub Token，也不调用 `api.github.com`。检查元数据和下载安装包均通过 Releases 下载链接，避开 GitHub REST API 的匿名配额；普通下载仍可能受网络、CDN 可用性和服务端策略影响。

## 使用

默认每次启动都会在后台检查更新，发现更高的稳定版本就自动下载并验证。下载完成后，应用在可交互窗口弹出更新对话框。选择“重启并更新”确认安装，或选择“稍后”继续工作，之后可从应用菜单 → 检查更新，或设置 → 应用 → 软件更新重新打开。

可以在对话框关闭启动时检查并下载更新。手动检查发现新版后仍可自行选择下载。对话框显示下载进度，支持取消和失败后重试，也能打开已下载归档。安装前必须保存全部修改，并等待文件打开、保存和其他文件操作结束。应用锁定编辑，等待会话与写入完成后才替换程序；准备期间取消或收到新的文件打开请求会中止安装。替换期间的新打开请求会保留，处理后再次确认重启。退出写入全部结束后，直接启动新程序。

Debug 和未标记为正式发布的本地构建可以检查和下载，但不能替换开发可执行文件。正式构建要求编译时设置 `SUPERFILE_BUILD_CHANNEL=release`；标签工作流已设置。macOS 需要可写位置中的 `Superfile.app`；磁盘映像/App Translocation、包管理器安装和 Linux 系统目录不执行应用内替换，应使用原安装方式更新。下载目录仍可用于手动安装。

启动更新开关使用共享应用数据库。下载和临时解包使用现有缓存目录，Debug/Release 无专属目录或隐式路径覆盖。安装暂存目录位于目标程序旁以保证同卷重命名。替换时保留旧版本用于回滚。新版本打开窗口后，后台按目标和版本标记清理旧应用包或可执行文件。Windows 旧进程仍占用备份时短暂重试，必要时留待下次启动。macOS 还会检查旧版更新器留下的备份，确认应用身份、文件名与版本后再清理，避免旧应用出现在「打开方式」。

## 发布契约

`main` 分支负责预热三个平台的 Release 构建缓存。标签构建从 `main` 恢复可用的 Rust 缓存，不上传标签作用域的副本。标签发布不运行工作区测试、Clippy、格式和仓库审查任务；这些检查由开发验证执行。自动发布和更新清单采用三个目标；macOS Intel 仍可手动构建、打包。v0.0.4 已启动的流程保留其原有四目标配置，新的矩阵从后续标签生效。

推送版本标签后，工作流校验标签后，完成 Windows x64、Linux x64、macOS ARM64 的 Release 编译和打包，校验 6 个安装包/便携归档，再生成 `SHA256SUMS.txt` 和 `latest.json`，共上传 8 个附件。Manifest 含版本、标签、发布时间、发布页面，以及三个更新归档的 OS、架构、格式、字节数和 SHA-256。Windows 更新选择 `.zip`，Linux/macOS 选择 `.tar.gz`；原有 NSIS、DEB、DMG 安装包仍正常发布。

```sh
python3 scripts/package/release.py manifest v1.2.3 --directory package-dist --repository Super1Windcloud/SuperfileView
python3 scripts/package/release.py verify v1.2.3 --directory package-dist --checksums --manifest --repository Super1Windcloud/SuperfileView
```

预发布标签生成对应 Manifest，但不会移动 GitHub 的 latest 稳定版入口。客户端默认地址是上面的主仓库；工作流通过编译时 `SUPERFILE_UPDATE_MANIFEST_URL` 自动指向当前仓库，Fork 不会检查主仓库的版本。此参数仅配置公开静态地址，不能用于存放凭据。

下载由上游引擎强制 HTTPS、字节数上限和 SHA-256 校验。客户端额外验证仓库、版本、标签、平台、架构与附件名称一致；解包拒绝越界路径、符号/硬链接、特殊文件、重复写入同一目标和超过 4 GiB 的展开内容。macOS 校验 bundle ID、可执行文件名和版本。安装使用纯 Rust 解包与同卷重命名；替换失败会尝试恢复旧版本，并保留备份及错误详情。运行时不依赖 `tar`、`hdiutil`、`ditto`、PowerShell 或 shell 重启脚本。

当前校验基于 HTTPS 和发布附件中的 SHA-256，未配置独立签名密钥、Windows Authenticode 或 Apple Developer ID/公证；SHA-256 不等同于发布者签名。CI 的既有 macOS ad-hoc 签名保留。客户端无需 Token，CI 发布仍使用 GitHub 自动提供的 `GITHUB_TOKEN`。

本地验证覆盖归档、路径防护、同卷暂存、备份、失败回滚、取消、更新界面及 Manifest 生成；真实远程发布、各平台安装替换和重启仍需在发布产物上验收。首次发布包含 `latest.json` 的标签后，线上更新入口才具备本次协议。
