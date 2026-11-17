# 应用更新

Superfile 使用 [AprilNEA/gpui-updater](https://github.com/AprilNEA/gpui-updater) 的 `gpui-updater-core = 0.4.4`。`StaticManifestSource` 读取固定 HTTPS 地址：

```text
https://github.com/Super1Windcloud/SuperfileView/releases/latest/download/latest.json
```

客户端没有 GitHub Token，也不调用 `api.github.com`。检查元数据和下载安装包均通过 Releases 下载链接，避开 GitHub REST API 的匿名配额；普通下载仍可能受网络、CDN 可用性和服务端策略影响。

## 使用

应用菜单 → 检查更新，或设置 → 应用 → 软件更新，可以打开更新对话框。启动检查默认开启，每 24 小时最多自动尝试一次；可以在对话框关闭。手动检查不受此间隔限制。网络、404、无匹配平台和校验失败会显示错误并允许重试，不会当成“已是最新”。只接受比当前版本更高的稳定版本，不降级，也不自动安装预发布版本。

发现新版本后由用户点击下载，显示进度并允许取消。下载完成后可查看归档；可更新的正式构建提供“重启并更新”。安装前必须保存全部修改，并等待文件打开、保存和其他文件操作结束。应用锁定编辑，等待会话与写入完成后才替换程序；准备期间取消或收到新的文件打开请求会中止安装。替换期间的新打开请求会保留，处理后再次确认重启。退出写入全部结束后，直接启动新程序。

Debug 和未标记为正式发布的本地构建可以检查和下载，但不能替换开发可执行文件。正式构建要求编译时设置 `SUPERFILE_BUILD_CHANNEL=release`；标签工作流已设置。macOS 需要可写位置中的 `Superfile.app`；磁盘映像/App Translocation、包管理器安装和 Linux 系统目录不执行应用内替换，应使用原安装方式更新。下载目录仍可用于手动安装。

检查开关和最后尝试时间使用共享应用数据库。下载和临时解包使用现有缓存目录，Debug/Release 无专属目录或隐式路径覆盖。安装暂存目录位于目标程序旁以保证同卷重命名；下载及暂存文件随取消或应用退出清理。成功安装的旧版本保留在目标旁 `.superfile-previous-*` 目录中，不自动删除用户安装目录中的其他内容。

## 发布契约

推送版本标签后，工作流完成 Windows x64、Linux x64、macOS ARM64/Intel 的测试和构建，校验 8 个安装包/便携归档，再生成 `SHA256SUMS.txt` 和 `latest.json`，共上传 10 个附件。Manifest 含版本、标签、发布时间、发布页面，以及四个更新归档的 OS、架构、格式、字节数和 SHA-256。Windows 更新选择 `.zip`，Linux/macOS 选择 `.tar.gz`；原有 NSIS、DEB、DMG 安装包仍正常发布。

```sh
python3 scripts/package/release.py manifest v1.2.3 --directory package-dist --repository Super1Windcloud/SuperfileView
python3 scripts/package/release.py verify v1.2.3 --directory package-dist --checksums --manifest --repository Super1Windcloud/SuperfileView
```

预发布标签生成对应 Manifest，但不会移动 GitHub 的 latest 稳定版入口。客户端默认地址是上面的主仓库；工作流通过编译时 `SUPERFILE_UPDATE_MANIFEST_URL` 自动指向当前仓库，Fork 不会检查主仓库的版本。此参数仅配置公开静态地址，不能用于存放凭据。

下载由上游引擎强制 HTTPS、字节数上限和 SHA-256 校验。客户端额外验证仓库、版本、标签、平台、架构与附件名称一致；解包拒绝越界路径、符号/硬链接、特殊文件、重复写入同一目标和超过 4 GiB 的展开内容。macOS 校验 bundle ID、可执行文件名和版本。安装使用纯 Rust 解包与同卷重命名；替换失败会尝试恢复旧版本，并保留备份及错误详情。运行时不依赖 `tar`、`hdiutil`、`ditto`、PowerShell 或 shell 重启脚本。

当前校验基于 HTTPS 和发布附件中的 SHA-256，未配置独立签名密钥、Windows Authenticode 或 Apple Developer ID/公证；SHA-256 不等同于发布者签名。CI 的既有 macOS ad-hoc 签名保留。客户端无需 Token，CI 发布仍使用 GitHub 自动提供的 `GITHUB_TOKEN`。

本地验证覆盖归档、路径防护、同卷暂存、备份、失败回滚、取消、更新界面及 Manifest 生成；真实远程发布、各平台安装替换和重启仍需在发布产物上验收。首次发布包含 `latest.json` 的标签后，线上更新入口才具备本次协议。
