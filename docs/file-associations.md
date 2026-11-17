# 系统文件类型与默认应用

在「设置 → 默认文件类型 → 查看」中打开对话框。三列分别显示文件格式、类型说明和当前默认应用；悬停格式或应用可查看类型标识、应用标识及路径。搜索同时匹配扩展名、类型、应用名称和路径，多个关键词同时生效。

查询范围是系统已知的类型，不限于 Superfile 能读取的格式。未找到默认处理程序时显示「未找到默认应用」；查询失败会显示错误，部分来源不可读时提示列表可能不完整。结果是查询时的快照，外部更改默认应用后可点击刷新。刷新保留搜索内容；取消或关闭对话框会取消后台查询。表格使用虚拟滚动，不按数量截断类型列表。

对话框只读取系统配置，不注册 Superfile，也不修改默认应用。因此直接 `cargo run` 和安装版均可查看当前系统的关联；无需先构建 Release。

## 平台来源与边界

- **Windows**：枚举合并后的 Classes 注册表与用户 FileExts，使用 [AssocQueryStringW](https://learn.microsoft.com/en-us/windows/win32/api/shlwapi/nf-shlwapi-assocquerystringw) 查询有效的默认应用、可执行文件和类型说明，不根据某个候选应用的声明推断默认值。
- **macOS**：汇总 CoreTypes 声明、标准应用目录、Spotlight 应用索引以及用户 LaunchServices 类型记录，使用公开 LaunchServices 接口查询每个 UTI 的默认处理程序和应用位置。应用可访问的声明与索引不等同于一份完整的 LaunchServices 数据库导出；未被索引且位于非标准目录的应用，以及不可读取的声明，可能使列表不完整。未覆盖的类型可以在系统索引更新后刷新。
- **Linux**：使用 [GIO 的系统类型枚举](https://docs.gtk.org/gio/func.content_types_get_registered.html)与[默认处理程序查询](https://docs.gtk.org/gio/type_func.AppInfo.get_default_for_type.html)，从 XDG MIME 的 `globs2` 补充扩展名。需要系统 GLib/GIO；`.deb` 声明运行时依赖，便携包也需系统提供这些库。缺少库或所需符号时显示读取错误，应用仍可继续使用。

读取和查询均在服务层后台执行；渲染过程不扫描注册表、目录或应用元数据。不调用 `lsregister`、`defaults`、`reg`、`xdg-mime` 等外部命令。macOS 声明文件与 Linux 后缀元数据分别限制为 16 MiB；超限会报告读取失败，不将截断数据当作完整结果。

## 本机只读核对

```sh
cargo run -p superfile-services --locked --example file_associations > target/file-associations.json
```

示例输出完整 JSON 快照和来源警告，不会修改系统关联。平台 API 的行为及桌面环境差异仍需在相应系统进行运行验证。
