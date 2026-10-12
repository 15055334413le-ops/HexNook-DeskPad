# HN DeskPad

Windows 启动台（Rust + Win32）：Alt + Space 打开，集中显示全部应用、最近文件和常用文件夹。

- `crates/engine` — 数据引擎（应用扫描、最近文件、图标与缩略图、设置）
- `crates/app` — 界面（DeskPad.exe）
- `crates/setup` — 在线安装程序（HN-DeskPad-Setup.exe）
- `docs/` — 开发规范（engine-architecture.md、ui-architecture.md）与决策记录（questions.md）

每次推送到 `main`，GitHub Actions 会在 Windows 上编译、测试、clippy，并上传构建产物（发布包 zip、安装程序、SHA256SUMS）。
公开下载在 [HexNook-DeskPad-Releases](https://github.com/15055334413le-ops/HexNook-DeskPad-Releases)。
