# DeskPad 引擎板块开发规范（deskpad-engine）· v2

> 配套文档：`ui-architecture.md`（UI 板块）。**接口契约（第 3 节）只在本文件维护**，UI 文档只引用不复制；改契约必须同时检查 UI 文档里引用到的字段名。
> 产品：按 Alt+Space 呼出的桌面启动台（类似 macOS Launchpad），桌面本身保持原样。技术栈为 Rust + Win32 原生，目标系统 Win10 1809+，主力 Win11。

## 0. 给执行 agent 的规则（先读）

1. **本文是唯一依据**。文中写死的数值、路径、文案、顺序一律照做，不要"优化"。确实做不到时，停下来在 `docs/questions.md` 里追加一条（日期 + 章节号 + 问题 + 你建议的方案），然后跳过该项继续做其他部分。
2. 第 3 节标注为「原样照抄」的代码块，逐字复制，不改名、不调整字段顺序、不加 derive。
3. 文中 MUST / 必须是硬性要求；SHOULD / 建议是时间允许时做；v0.2 字样的内容**不做**。
4. 编码约束：
   - `unwrap()` / `expect()` 只能出现在 `#[cfg(test)]` 和 `bin/probe.rs` 里，其他地方一律用 `?` 或 `match` 处理。
   - 每个 `unsafe` 块上方写一行 `// SAFETY: ...`，说明为什么安全。
   - Win32 句柄和 COM 资源统一用 `src/win.rs` 里的 RAII 包装（见 4.9），**禁止裸 CloseHandle / DeleteObject 散落在业务代码中**。
   - 每完成一个里程碑，跑一遍 `cargo build --release`、`cargo test -p deskpad-engine`、`cargo clippy -p deskpad-engine -- -D warnings`，三项全部通过才算完成。
5. **红线（代码评审项，违反即打回）**：禁止 `SetWindowsHookEx`、`WriteProcessMemory`、`ReadProcessMemory`、`CreateRemoteThread`、任何驱动加载、任何进程隐藏或伪装。引擎只读取自己需要的数据，并通过 ShellExecute 启动程序，和普通办公软件同级。

## 1. 职责边界

**引擎 = 所有"找数据、管数据、执行动作"的代码，不创建任何窗口。**

- 做：应用索引（AppsFolder + 桌面快捷方式）、图标提取与磁盘缓存、最近文件/桌面文件聚合、常用文件夹聚合、拼音首字母、搜索排序与使用记录、启动/打开动作、设置持久化、开机自启、单实例互斥、日志、后台扫描与变更监听线程。
- 不做：窗口、消息循环、渲染、全局热键（`RegisterHotKey` 需要 HWND，归 UI）、托盘（归 UI）、任何面向用户的文案（错误以 `EngineError` 枚举返回，由 UI 翻译成文字）。

引擎必须**可无头验证**：提供 `probe` 命令行工具，不开任何窗口就能测试全部功能（4.10）。

## 2. 工程布局

> **项目根目录：`D:\MyWork\DeskPad\`**。它和旧的 C# 工具 `D:\MyWork\DeskTidy\` 是两个独立项目，互不引用，也不读取对方的任何文件。

```
D:\MyWork\DeskPad\
├─ Cargo.toml                 # workspace 根（内容见下）
├─ crates\
│  ├─ engine\                 # 本板块：包名 deskpad-engine，lib 名 deskpad_engine
│  │  ├─ Cargo.toml
│  │  └─ src\
│  │     ├─ lib.rs            # 公开 API（3.3）+ Inner + 线程调度（4.8）
│  │     ├─ model.rs          # 契约数据类型（3.1，原样照抄）
│  │     ├─ search.rs         # 搜索排序（3.2，原样照抄）
│  │     ├─ keys.rs           # SearchKeys 计算：拼音首字母、alias、sort_key（4.1.4）
│  │     ├─ apps.rs           # AppsFolder 枚举 + 桌面快捷方式 + 过滤去重（4.1）
│  │     ├─ icons.rs          # 图标提取 → 96px PNG 缓存（4.2）
│  │     ├─ recent.rs         # 最近文件 + 桌面文件（4.3）
│  │     ├─ dirs.rs           # 常用文件夹（4.4）
│  │     ├─ filekind.rs       # 扩展名 → FileKind 映射表（4.3.4）
│  │     ├─ launch.rs         # ShellExecuteExW 启动/打开（4.5）
│  │     ├─ store.rs          # config.json / usage.json / index.json 原子读写（4.6）
│  │     ├─ autostart.rs      # HKCU Run 键 + StartupApproved（4.7）
│  │     ├─ single.rs         # 单实例互斥（4.7）
│  │     ├─ log.rs            # 文件日志（4.9）
│  │     ├─ win.rs            # RAII 包装与 Win32 小工具（4.9）
│  │     ├─ update.rs         # 在线更新：子进程检查、版本比较、update.json（4.11）
│  │     └─ bin\probe.rs      # 无头验证工具（4.10）
│  └─ app\                    # UI 板块（另一个 agent 负责）
└─ docs\
```

**根 Cargo.toml（原样照抄）：**

```toml
[workspace]
resolver = "2"
members = ["crates/engine", "crates/app"]

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1
panic = "abort"
```

**谁建什么（避免两个 agent 互相覆盖）：**

- 先开工的 agent 负责创建根 `Cargo.toml`，同时给**对方**的 crate 建一个占位：
  - 引擎占位 = 「E0 契约桩」（见第 6 节 E0）；
  - UI 占位 = `crates/app/Cargo.toml`（照抄 UI 文档第 6 节）加上 `src/main.rs`，main.rs 内容为 `// DESKPAD-PLACEHOLDER` 和 `fn main() {}` 两行。
- 只有带 `// DESKPAD-PLACEHOLDER` 标记的文件可以被对方覆盖，其他文件一律不碰。

## 3. 接口契约

### 3.1 model.rs（原样照抄）

```rust
//! 接口契约：引擎与 UI 共享的数据类型。本文件即契约原文，修改须同步两份文档。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// 应用稳定 id：FNV-1a 64 位哈希（见 engine 文档 4.1「id 规则」）。
pub type AppId = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchTarget {
    /// shell:AppsFolder 项的解析名（UWP 的 AUMID，或桌面程序的 "{KNOWNFOLDERID}\\path.exe" 等）。
    AppsFolder(String),
    /// 桌面上的 .lnk 文件本体完整路径。
    Shortcut(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppSource {
    StartMenu,
    Desktop,
    ExtraScanDir,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppEntry {
    pub id: AppId,
    /// 显示名（AppsFolder 的 SIGDN_NORMALDISPLAY，或 .lnk 文件名去扩展名）。
    pub name: String,
    pub launch: LaunchTarget,
    /// 96×96 非预乘 RGBA PNG。None = 尚未提取或提取失败，UI 画占位图标。
    pub icon_png: Option<PathBuf>,
    pub source: AppSource,
    /// 引擎内部用于搜索与排序的派生键，UI 不得解析。
    pub keys: SearchKeys,
    /// 是否为 Windows 管理/维护工具；日常应用不算。
    #[serde(default)]
    pub is_system: bool,
    /// 是否在 settings.hidden_apps 中；只用于 all_apps 的管理视图。
    #[serde(default)]
    pub hidden: bool,
    /// hidden_apps 使用的小写稳定键；扫描时预先计算。
    #[serde(default)]
    pub setting_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchKeys {
    /// 名称小写。
    pub name_lower: String,
    /// 词首/拼音首字母，例："微信"→"wx"，"Visual Studio Code"→"vsc"，"PowerShell"→"ps"。
    pub initials: String,
    /// 可执行文件名小写去扩展名（取不到为空），例："photoshop"。
    pub alias: String,
    /// 排序键：每个汉字替换为其拼音首字母后的小写名称，用于空查询时的字母序。
    pub sort_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
    Image,
    Video,
    Audio,
    Archive,
    Installer,
    Code,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    /// 文件名（含扩展名）。
    pub name: String,
    /// 扩展名小写、不带点，无扩展名为空串。
    pub ext: String,
    pub kind: FileKind,
    /// Unix 秒。来自 Recent 的条目取 .lnk 的修改时间（≈最近打开时间），桌面文件取文件修改时间。
    pub mtime: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirEntry {
    pub path: PathBuf,
    /// 显示名（已知文件夹用中文固定名，见 engine 文档 4.4）。
    pub name: String,
    /// 顶层条目数（跳过隐藏/系统），封顶 9999。
    pub count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotkeyPreset {
    AltSpace,
    CtrlAltSpace,
    AltBacktick,
}

impl HotkeyPreset {
    pub const ALL: [HotkeyPreset; 3] = [Self::AltSpace, Self::CtrlAltSpace, Self::AltBacktick];
    /// (fsModifiers, vk)，直接传给 RegisterHotKey；MOD_NOREPEAT 由 UI 另行 OR 上。
    pub fn mods_vk(self) -> (u32, u32) {
        const MOD_ALT: u32 = 0x0001;
        const MOD_CONTROL: u32 = 0x0002;
        match self {
            Self::AltSpace => (MOD_ALT, 0x20),
            Self::CtrlAltSpace => (MOD_ALT | MOD_CONTROL, 0x20),
            Self::AltBacktick => (MOD_ALT, 0xC0), // VK_OEM_3
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::AltSpace => "Alt + Space",
            Self::CtrlAltSpace => "Ctrl + Alt + Space",
            Self::AltBacktick => "Alt + `",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    Dark,
}

/// 启动台主体视觉风格。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VisualStyle {
    #[default]
    Flat,
    Glass,
}

/// 启动台尺寸。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PadSize {
    #[default]
    Window,
    Fullscreen,
}

/// 应用图标呈现方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IconStyle {
    #[default]
    Native,
    Unified,
}

/// 启动台应用排序方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortMode {
    #[default]
    Name,
    /// 按 30 天半衰期的启动频率分排序。
    Frequency,
}

pub const SETTINGS_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub hotkey: HotkeyPreset,
    /// 以注册表真实状态为准：init() 时引擎会用注册表覆盖此值。
    pub autostart: bool,
    /// 1..=20，越界由引擎在加载/保存时钳制。
    pub max_recent: u32,
    /// v0.1 只实现 Light；Dark 存在但 UI 按 Light 绘制。
    pub theme: Theme,
    /// 角标按钮 40×40 主体（不含阴影边距）左上角，虚拟屏幕物理像素。None = 默认位置。
    // v0.2 起删除 launcher_pos / grid_snap：角标按钮位置固定（贴近任务栏左端），
    // 不再支持拖动，也不需要网格吸附（questions.md #33）。
    pub first_run_done: bool,
    /// 打开启动台时是否自动检查更新（距上次 ≥24 小时才真正发起，questions.md #35）。
    /// serde default = true，旧 config.json 缺这个字段也能读。
    pub auto_check_update: bool,
    pub visual_style: VisualStyle,
    pub pad_size: PadSize,
    pub icon_style: IconStyle,
    pub show_system_tools: bool,
    /// AppsFolder parsing name 或快捷方式目标路径，统一小写。
    pub hidden_apps: Vec<String>,
    pub extra_scan_dirs: Vec<PathBuf>,
    pub manual_apps: Vec<PathBuf>,
    pub sort_mode: SortMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            hotkey: HotkeyPreset::AltSpace,
            autostart: false,
            max_recent: 8,
            theme: Theme::Light,

            first_run_done: false,
            auto_check_update: true,
            visual_style: VisualStyle::Flat,
            pad_size: PadSize::Window,
            icon_style: IconStyle::Native,
            show_system_tools: false,
            hidden_apps: Vec::new(),
            extra_scan_dirs: Vec::new(),
            manual_apps: Vec::new(),
            sort_mode: SortMode::Name,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// 每次发布 +1；UI 用它判断是否需要重建缓存。
    pub generation: u64,
    /// false = 首次扫描尚未完成（UI 显示「正在读取应用…」）。
    pub ready: bool,
    /// 未过滤的全量应用，隐藏项带 hidden=true，供设置页管理。
    pub all_apps: Arc<Vec<AppEntry>>,
    /// 已按 settings.sort_mode 排序的可见应用。
    pub apps: Arc<Vec<AppEntry>>,
    /// 已排序：mtime 降序，截断到 settings.max_recent。
    pub recent: Arc<Vec<FileEntry>>,
    /// 顺序固定：见 engine 文档 4.4。
    pub dirs: Arc<Vec<DirEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// 另一个 DeskPad 实例正在运行（single::acquire 返回）。
    AlreadyRunning,
    /// launch_app 传入的 id 不在当前快照中。
    UnknownApp(AppId),
    /// ShellExecuteExW 失败；code = GetLastError()。
    LaunchFailed { name: String, code: u32 },
    /// open_path 的目标不存在。
    PathNotFound(PathBuf),
    /// 文件/注册表读写失败，附英文技术细节（只进日志，不直接展示）。
    Io(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EngineError {}
```

### 3.2 search.rs（原样照抄）

搜索是纯函数，不依赖 Win32，可以直接 `cargo test`。打分规则就是这段代码本身，**不允许改动权重**。如果对排序结果有意见，写进 `questions.md`。

```rust
//! 搜索排序（纯函数，无 Win32 依赖；拼音首字母由 keys.rs 预先算好放进 SearchKeys）。
use crate::model::{AppEntry, AppId};
use std::collections::HashMap;

/// 使用记录（usage.json），由引擎维护，search 只读。
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Usage {
    /// 每个应用经 DeskPad 启动的次数。
    pub launch_counts: HashMap<AppId, u32>,
    /// 每个应用最后一次成功启动的 Unix 秒时间。
    pub last_launched: HashMap<AppId, i64>,
    /// 规范化查询串 → 用户上一次从该查询启动的应用。
    pub query_picks: HashMap<String, AppId>,
}

/// 查询规范化：trim、小写、去掉所有空白。
pub fn normalize_query(q: &str) -> String {
    q.trim().chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect()
}

/// 词首位置（char 下标）：0、非字母数字之后、小写→大写的驼峰处、字母↔数字切换处、每个 CJK 字符。
pub fn word_starts(name: &str) -> Vec<usize> {
    let cs: Vec<char> = name.chars().collect();
    let mut v = Vec::new();
    for i in 0..cs.len() {
        let c = cs[i];
        if !c.is_alphanumeric() { continue; }
        let start = if i == 0 { true } else {
            let p = cs[i - 1];
            !p.is_alphanumeric()
                || (p.is_lowercase() && c.is_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit())
                || is_cjk(c) || is_cjk(p)
        };
        if start { v.push(i); }
    }
    v
}

pub fn is_cjk(c: char) -> bool { ('\u{4E00}'..='\u{9FFF}').contains(&c) }

fn is_subsequence(q: &str, s: &str) -> Option<usize> {
    let mut it = s.chars().enumerate();
    let mut first = None;
    for qc in q.chars() {
        loop {
            match it.next() {
                Some((i, sc)) if sc == qc => { if first.is_none() { first = Some(i); } break; }
                Some(_) => continue,
                None => return None,
            }
        }
    }
    first
}

/// 单个应用对查询的得分；0 = 不匹配。q 必须已经过 normalize_query。
pub fn score(app: &AppEntry, q: &str) -> u32 {
    let k = &app.keys;
    let name_nospace: String = k.name_lower.chars().filter(|c| !c.is_whitespace()).collect();
    if k.name_lower == q || name_nospace == q { return 120; }
    if k.name_lower.starts_with(q) || name_nospace.starts_with(q) { return 100; }
    let lower: Vec<char> = k.name_lower.chars().collect();
    let starts = word_starts(&app.name);
    for &i in &starts {
        let tail: String = lower[i.min(lower.len())..].iter().collect();
        if tail.starts_with(q) { return 85; }
    }
    if k.initials.starts_with(q) { return 75; }
    if k.name_lower.contains(q) { return 60; }
    if !k.alias.is_empty() && k.alias.starts_with(q) { return 55; }
    if k.initials.contains(q) { return 45; }
    if !k.alias.is_empty() && k.alias.contains(q) { return 40; }
    if let Some(first) = is_subsequence(q, &k.name_lower) {
        return if starts.contains(&first) { 30 } else { 20 };
    }
    0
}

/// 返回 apps 的下标，按相关度降序。空查询返回 0..len（apps 已按 sort_key 排好）。
pub fn search(apps: &[AppEntry], usage: &Usage, raw_q: &str) -> Vec<usize> {
    let q = normalize_query(raw_q);
    if q.is_empty() { return (0..apps.len()).collect(); }
    let pick = usage.query_picks.get(&q).copied();
    let mut hits: Vec<(u32, u32, usize, usize)> = Vec::new(); // (score, count, name_len, idx)
    for (idx, a) in apps.iter().enumerate() {
        let mut s = score(a, &q);
        if s == 0 { continue; }
        if pick == Some(a.id) { s += 1000; }
        let cnt = usage.launch_counts.get(&a.id).copied().unwrap_or(0);
        hits.push((s, cnt, a.name.chars().count(), idx));
    }
    hits.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)).then(x.2.cmp(&y.2))
        .then(apps[x.3].keys.sort_key.cmp(&apps[y.3].keys.sort_key)));
    hits.into_iter().map(|h| h.3).collect()
}
```

E0 必须带上下面两个测试，放在 `lib.rs` 末尾（原样照抄；`app()` 是只在测试里用的构造函数）：

```rust
#[cfg(test)]
mod t {
    use super::model::*; use super::search::*;
    fn app(id: u64, name: &str, initials: &str, alias: &str) -> AppEntry {
        AppEntry { id, name: name.into(), launch: LaunchTarget::AppsFolder(name.into()), icon_png: None,
          source: AppSource::StartMenu, keys: SearchKeys { name_lower: name.to_lowercase(), initials: initials.into(), alias: alias.into(), sort_key: name.to_lowercase() } }
    }
    #[test] fn settings_roundtrip() {
        let s = Settings { max_recent: 12, ..Default::default() };
        let j = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&j).unwrap(), s);
        // 缺字段、多出未知字段的旧配置也必须能读，且已有字段保留
        let old: Settings = serde_json::from_str(r#"{"autostart":true,"unknown_future":1}"#).unwrap();
        assert!(old.autostart && old.max_recent == 8 && old.hotkey == HotkeyPreset::AltSpace);
    }
    #[test] fn ranking() {
        let apps = vec![
            app(1, "Adobe Photoshop 2025", "ap2", "photoshop"),
            app(2, "Windows PowerShell", "wps", "powershell"),
            app(3, "微信", "wx", "weixin"),
            app(4, "企业微信", "qywx", "wxwork"),
            app(5, "Visual Studio Code", "vsc", "code"),
            app(6, "WPS Office", "wo", "wps"),
        ];
        let mut u = Usage::default();
        let names = |v: Vec<usize>| v.iter().map(|&i| apps[i].name.clone()).collect::<Vec<_>>();
        println!("ps  {:?}", names(search(&apps, &u, "ps")));
        println!("wx  {:?}", names(search(&apps, &u, "wx")));
        println!("vsc {:?}", names(search(&apps, &u, "vsc")));
        println!("code {:?}", names(search(&apps, &u, "code")));
        println!("photo {:?}", names(search(&apps, &u, "photo")));
        println!("微 {:?}", names(search(&apps, &u, "微")));
        assert_eq!(search(&apps, &u, "wx")[0], 2);
        u.query_picks.insert("ps".into(), 1);
        assert_eq!(apps[search(&apps, &u, "ps")[0]].id, 1);
        assert_eq!(apps[search(&apps, &u, " P S ")[0]].id, 1);
        assert_eq!(word_starts("PowerShell"), vec![0,5]);
        assert_eq!(word_starts("Photoshop2025"), vec![0,9]);
    }
}
```

打分速查（与代码一致，方便人读）：

| 条件（q 已规范化） | 分 |
|---|---|
| 名称全等（含去空格后全等） | 120 |
| 名称前缀 | 100 |
| 某个词首开始的前缀（驼峰、空格、数字边界、每个汉字都算词首） | 85 |
| 首字母串前缀（"wx" → 微信，"vsc" → Visual Studio Code） | 75 |
| 名称包含 | 60 |
| alias（exe 名）前缀 | 55 |
| 首字母串包含 | 45 |
| alias 包含 | 40 |
| 子序列，且起点在词首 / 不在词首 | 30 / 20 |
| **该查询上次被用来启动这个应用（query_picks）** | +1000 |

同分时依次比较：启动次数多者优先 → 名称短者优先 → sort_key 字母序。

### 3.3 lib.rs 公开 API（签名原样照抄，函数体自己实现）

```rust
pub mod log;
pub mod model;
pub mod search;
pub mod single;
// 以下为私有模块
mod apps; mod autostart; mod dirs; mod filekind; mod icons; mod keys;
mod launch; mod recent; mod store; mod win;

pub use model::*;

/// 保证：Clone + Send + Sync。内部为 Arc<Inner>（4.8）。
#[derive(Clone)]
pub struct EngineHandle { /* inner: std::sync::Arc<Inner> */ }

impl EngineHandle {
    /// 不要求调用线程已初始化 COM。流程见 4.8.1。
    /// 有 index.json 缓存时 ≤ 50ms 返回；首次运行最多阻塞 3 秒，等待应用列表就绪。
    pub fn init() -> Result<EngineHandle, EngineError>;

    /// 永不阻塞（只克隆几个 Arc）。
    pub fn snapshot(&self) -> Snapshot;

    /// 每次发布新快照后，在**引擎扫描线程**上调用 f。f 必须很快返回，UI 在里面只做 PostMessageW。
    /// 重复调用会替换旧的回调。
    pub fn set_on_change(&self, f: Box<dyn Fn() + Send + Sync + 'static>);

    /// 请求后台重扫（APPS + FILES 两类，见 4.8.3），非阻塞，自带去抖。UI 每次打开启动台都调用一次。
    pub fn refresh(&self);

    /// 返回 snap.apps 的下标，按相关度降序。内部调用 search::search(&snap.apps, &usage, query)。
    pub fn search(&self, snap: &Snapshot, query: &str) -> Vec<usize>;

    /// 调用线程必须已经 CoInitializeEx(STA)（UI 主线程和 probe 的 main 都满足）。
    /// from_query：用户是通过哪个查询启动的；没有查询时传 ""。成功后更新 usage.json。
    pub fn launch_app(&self, id: AppId, from_query: &str) -> Result<(), EngineError>;

    /// 打开文件或文件夹。调用线程要求同 launch_app。
    pub fn open_path(&self, path: &std::path::Path) -> Result<(), EngineError>;

    pub fn settings(&self) -> Settings;

    /// 钳制字段 → 原子写 config.json → autostart 变化时写注册表 →
    /// 应用视图变化立即发布；扫描目录/手动应用变化触发 APPS 重扫。
    pub fn save_settings(&self, s: &Settings) -> Result<(), EngineError>;

    /// 以稳定 setting_id 隐藏/取消隐藏；触发 APPS 重扫并发布新 Snapshot。
    pub fn set_hidden(&self, id: &str, hidden: bool) -> Result<(), EngineError>;
    /// 添加/移除手动 exe、lnk、url；成功保存后触发 APPS 重扫。
    pub fn add_manual_app(&self, path: &std::path::Path) -> Result<(), EngineError>;
    pub fn remove_manual_app(&self, path: &std::path::Path) -> Result<(), EngineError>;
    /// 添加/移除额外目录；网络盘拒绝添加；成功保存后触发 APPS 重扫。
    pub fn add_scan_dir(&self, path: &std::path::Path) -> Result<(), EngineError>;
    pub fn remove_scan_dir(&self, path: &std::path::Path) -> Result<(), EngineError>;

    /// 通知后台线程退出并 join，最多等 1 秒。UI 在退出前调用。
    pub fn shutdown(&self);
}
```

```rust
// single.rs 公开部分
pub struct InstanceGuard { /* 持有 mutex 句柄；Drop 时 CloseHandle */ }
/// 返回的 guard 必须一直持有到进程结束（UI 把它放进 main 的局部变量）。
pub fn acquire() -> Result<InstanceGuard, crate::EngineError>; // 已有实例 → Err(AlreadyRunning)

// log.rs 公开部分（UI 也用它写日志）
pub fn info(msg: &str);
pub fn warn(msg: &str);
pub fn error(msg: &str);
```

**数据流**：

1. UI 启动后先调用 `single::acquire()`，再调用 `EngineHandle::init()`，然后 `set_on_change(PostMessage)`，拿到 `snapshot()` 做首次绘制。
2. 后台扫描完成后发布新快照，回调触发 PostMessage，UI 在消息里重新取 `snapshot()` 并重绘。
3. **UI 不直接访问文件系统、注册表或 COM（WIC/D2D 渲染除外），所有数据都经由引擎 API。**

## 4. 模块明细

### 4.1 apps.rs — 应用索引

#### 4.1.1 来源 A：`shell:AppsFolder`（主来源，对应 AppSource::StartMenu）

它就是开始菜单「所有应用」的数据源，同时覆盖传统 .lnk 应用和 UWP/Store 应用（设置、计算器、照片、终端等），由系统自动去重。

```
item  = SHCreateItemFromParsingName(w!("shell:AppsFolder"), None) -> IShellItem
enum  = item.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems)
loop  enum.Next(&mut [None; 1], Some(&mut fetched))，fetched == 0 时结束：
    name    = child.GetDisplayName(SIGDN_NORMALDISPLAY)          // PWSTR，读完后 CoTaskMemFree
    parsing = child.GetDisplayName(SIGDN_PARENTRELATIVEPARSING)  // 例 "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
                                                                 //    或 "{6D809377-...}\\Adobe\\Photoshop.exe"
    launch  = LaunchTarget::AppsFolder(parsing)
```

系统工具判定在扫描时完成，结果写入 `AppEntry.is_system`：

- 快捷方式位于开始菜单 `Administrative Tools` / `Windows Tools` 目录，或 shell 链接属性 `System.Link.TargetParsingPath` 指向这些目录中的快捷方式目标；
- 或目标 exe 位于 `%SystemRoot%`（含 System32 / SysWOW64）且文件名命中内置名单：`odbcad32`、`regedit`、`msconfig`、`perfmon`、`eventvwr`、`compmgmt`、`services`、`taskschd`、`dfrgui`、`cleanmgr`、`iscsicpl`、`msinfo32`、`resmon`、`recoverydrive`、`mdsched`、`secpol`、`gpedit`、`diskmgmt`、`printmanagement`、`comexp`、`wf` 等。
- 记事本、画图、计算器、终端、设置等日常应用不算系统工具。

#### 4.1.2 来源 B：桌面快捷方式（AppSource::Desktop）

- 扫描目录：`FOLDERID_Desktop` 和 `FOLDERID_PublicDesktop`，**只看顶层，不递归**。
- `*.lnk`：用 `IShellLinkW` + `IPersistFile::Load(path, STGM_READ)` 打开，再 `GetPath(&mut [u16; 260], null, 0)` 取目标。
  - 目标为空（MSI 广告快捷方式，例如 Office）→ 视为应用；
  - 目标扩展名属于 `exe / msc / bat / cmd / appref-ms` → 视为应用；
  - 其他情况（指向文件或文件夹的快捷方式）→ **不是应用**，跳过。
- `*.url`：按文本读取文件，找到以 `URL=` 开头的那一行。scheme 不是 `http` / `https` / `file` 时视为应用（例如 Steam 游戏的 `steam://rungameid/...`），否则跳过。
- name = 文件名去掉扩展名，再去掉末尾的 ` - 快捷方式` 或 ` - Shortcut`（不区分大小写）。
- launch = `LaunchTarget::Shortcut(完整路径)`。

**来源 C：额外扫描目录（AppSource::ExtraScanDir）**

- 对 `settings.extra_scan_dirs` 的每个本地目录扫描根目录与一级子目录，只收 `.exe` / `.lnk` / `.url`。
- 不递归更深层；名称含 `卸载` / `uninstall` 的子目录跳过；UNC 与 `DRIVE_REMOTE` 网络盘跳过。
- 额外目录及其一级子目录用 `FindFirstChangeNotificationW` 监听，设置变化后重建监听集合，不轮询。

**来源 D：手动应用（AppSource::Manual）**

- 只接受存在的 `.exe` / `.lnk` / `.url`；`.url` 的 `URL=` scheme 不是 `http` / `https` / `file` 时视为应用，否则不进入启动台；设置保存后走同一条 APPS 重扫路径。

#### 4.1.3 过滤、去重、id

- **过滤**（name 转小写后判断）：
  - 含 `卸载`、`帮助`、`说明`、`官网` 之一 → 丢弃；
  - 按空格和标点切词后，出现 `uninstall`、`readme`、`help`、`documentation`、`manual`、`website` 之一 → 丢弃；
  - alias 以 `unins` 开头 → 丢弃。
- **去重**：按来源优先级 A → B → C → D；后来源中 name 小写与前面任一项相同 → 丢弃。来源 A 内部不需要去重。
- **id**：`fnv1a64(key.as_bytes())`。来源 A 的 key = `"af:" + parsing.to_lowercase()`，来源 B 的 key = `"sc:" + 完整路径.to_lowercase()`。FNV-1a 64 位（offset basis `0xcbf29ce484222325`，prime `0x100000001b3`）自己写，放在 `keys.rs`。**禁止使用 `DefaultHasher`**，它的算法不保证跨 Rust 版本稳定。
- **setting_id**：AppsFolder 用 parsing name 小写；快捷方式用目标路径小写（解析失败时退回快捷方式路径）；额外目录/手动应用用完整文件路径小写。
- **排序**：`SortMode::Name` 按 `keys.sort_key` 升序，再按 name 升序。`SortMode::Frequency` 按启动次数乘 30 天半衰期衰减系数降序：`count × 0.5^(age_days / 30)`，同分再按最后启动时间、启动次数与名称稳定排序。
- 耗时参考：300 个应用约 100～300ms，在扫描线程上执行。

**Snapshot 投影**：每次发布前按当前设置重新生成两份列表。`all_apps` 是全量列表，按 `hidden_apps` 设置 `hidden`；`apps` 去掉 `hidden=true` 的项，并在 `show_system_tools=false` 时去掉 `is_system=true` 的项。两份列表使用同一排序规则。

#### 4.1.4 keys.rs — SearchKeys 计算

- `name_lower` = `name.to_lowercase()`。
- `alias`：
  1. parsing 含 `!`（UWP 的 AUMID）：取 `!` 之前的部分，再取第一个 `_` 之前的部分，最后取最后一个 `.` 之后的部分。例：`Microsoft.WindowsCalculator_8wekyb3d8bbwe!App` → `windowscalculator`。
  2. 否则含 `\` 或 `/`：取最后一段的文件名，去掉扩展名。
  3. 否则按 `.` 切分，丢掉纯数字段和 `exe`、`microsoft`，取剩下的最后一段。例：`Microsoft.Office.WINWORD.EXE.15` → `winword`。
  4. 来源 B 的 .lnk 用解析出的目标 exe 文件名去扩展名；目标为空或是 .url 时 alias 为空。
  - 所有结果转小写。
- `initials`：遍历 `search::word_starts(name)` 给出的每个词首字符 c：
  - c 是 ASCII 字母或数字 → 取 `c.to_ascii_lowercase()`；
  - c 是汉字 → 取 `hanzi_initial(c)`，返回 None 就跳过该字；
  - 其他字符 → 跳过。
  - 例：「微信」→ `wx`；「Visual Studio Code」→ `vsc`；「PowerShell」→ `ps`；「QQ音乐」→ `qyl`。
- `sort_key`：把 name 中每个汉字替换为 `hanzi_initial(c)`（返回 None 时保留原字），其余字符转小写。这样中英文混排时，「微信」会排在 w 区。

**拼音首字母（不引入任何 crate、不带大词表）：**

```rust
/// GB2312 一级汉字按拼音排序，用区位码边界就能得到首字母。
const BOUNDS: [(u16, char); 23] = [
    (0xB0A1,'a'),(0xB0C5,'b'),(0xB2C1,'c'),(0xB4EE,'d'),(0xB6EA,'e'),(0xB7A2,'f'),
    (0xB8C1,'g'),(0xB9FE,'h'),(0xBBF7,'j'),(0xBFA6,'k'),(0xC0AC,'l'),(0xC2E8,'m'),
    (0xC4C3,'n'),(0xC5B6,'o'),(0xC5BE,'p'),(0xC6DA,'q'),(0xC8BB,'r'),(0xC8F6,'s'),
    (0xCBFA,'t'),(0xCDDA,'w'),(0xCEF4,'x'),(0xD1B9,'y'),(0xD4D1,'z'),
];
/// 一级字表之外、但常见于软件名的字。发现新的可以往这里追加（需在 questions.md 记一笔）。
const SUPPLEMENT: [(char, char); 13] = [
    ('哔','b'),('咔','k'),('咚','d'),('咪','m'),('喵','m'),('嗨','h'),('昕','x'),
    ('浏','l'),('穹','q'),('蜓','t'),('蜻','q'),('逍','x'),('呱','g'),
];

pub fn hanzi_initial(c: char) -> Option<char> {
    if let Some(&(_, l)) = SUPPLEMENT.iter().find(|(h, _)| *h == c) { return Some(l); }
    let mut w = [0u16; 2];
    let w = c.encode_utf16(&mut w);
    let mut buf = [0u8; 4];
    // SAFETY: 输入输出缓冲区都由本函数持有，长度正确。
    let n = unsafe { WideCharToMultiByte(936, 0, w, Some(&mut buf), PCSTR::null(), None) };
    if n != 2 { return None; }
    let code = (buf[0] as u16) << 8 | buf[1] as u16;
    if !(0xB0A1..=0xD7F9).contains(&code) { return None; }   // 只覆盖一级汉字
    BOUNDS.iter().rev().find(|(start, _)| code >= *start).map(|&(_, l)| l)
}
```

边界表已用 GB2312 校验：每个边界对应的字依次是 啊 芭 擦 搭 蛾 发 噶 哈 击 喀 垃 妈 拿 哦 啪 期 然 撒 塌 挖 昔 压 匝。多音字不处理（例如「银行」会得到 yx）。

keys.rs 必须包含以下单元测试（仅 `#[cfg(windows)]` 下运行）：「微信」→ `wx`；「网易云音乐」→ `wyyyl`；「哔哩哔哩」→ `blbl`；「腾讯会议」→ `txhy`；「Visual Studio Code」→ `vsc`；FNV-1a 对空串的结果等于 `0xcbf29ce484222325`。

### 4.2 icons.rs — 图标缓存

- **统一路径**：给定一个 IShellItem，取 `IShellItemImageFactory` 调用 `GetImage(SIZE{cx:96, cy:96}, SIIGBF_ICONONLY)`，得到 HBITMAP。IShellItem 的获取方式：
  - 来源 A：`SHCreateItemFromParsingName("shell:AppsFolder\\" + parsing)`；
  - 来源 B：`SHCreateItemFromParsingName(快捷方式完整路径)`。这样能自动带上 .lnk/.url 里自定义的 IconLocation。
- **HBITMAP 转 RGBA**：
  1. 用 `GetObjectW` 取 BITMAP，得到宽高。
  2. `GetDIBits(BITMAPINFOHEADER{biBitCount:32, biCompression:BI_RGB, biHeight: -h})` 读出 top-down 的 BGRA。
  3. 用 `HBitmapGuard` 删除位图。
- **alpha 规范化**（PNG 必须存**非预乘** alpha）：
  1. 所有像素的 alpha 都是 0 → 老式无 alpha 图标，全部 alpha 置 255；
  2. 否则，只要有任意像素满足 `max(r,g,b) > a`，就说明数据本来是非预乘的，原样保留；
  3. 否则视为预乘数据，逐像素反预乘：`c = min(255, c*255/a)`（a=0 的像素颜色置 0）。
- 尺寸不是 96×96 时（有的图标只给 48px），用双线性缩放到 96 以内并居中放到 96×96 透明画布上（自己写约 20 行，或用 `image::imageops::resize` 配 `Triangle` 滤镜）。
- **编码**：`image` crate（`default-features = false, features = ["png"]`）写 RGBA8 PNG。
- **路径**：`%LocalAppData%\DeskPad\icons\{id:016x}.png`。
- **缓存策略**：
  - 文件存在且修改时间在 7 天内 → 命中，直接复用；
  - 超过 7 天 → 先复用旧图，排进后台队列重新提取；
  - 提取失败 → **不写文件**，icon_png = None，本进程内同一个 id 最多重试 3 次（下次扫描再试）；
  - 写盘失败只记 warn，不算致命错误。
- 每批提取结束后调用一次 `CoFreeUnusedLibrariesEx(0, 0)`，卸载 shell 扩展 DLL，压住常驻内存。

### 4.3 recent.rs — 最近文件和桌面文件

1. **来源 R（最近打开）**：`SHGetKnownFolderPath(FOLDERID_Recent)` 下的 `*.lnk`，先按 .lnk 自身的修改时间降序排序，然后逐个解析（同 4.1.2 的 IShellLinkW 方式，不调用 `Resolve()`）：
   - 目标里还有 `%` → 先用 `ExpandEnvironmentStringsW` 展开；
   - 目标以 `\\` 开头（UNC 路径），或 `GetDriveTypeW(盘符根)` 等于 `DRIVE_REMOTE` → 跳过，**不做存在性检查**，避免离线网络盘卡住扫描；
   - `GetFileAttributesW` 失败（文件不存在）或带 `FILE_ATTRIBUTE_DIRECTORY` → 跳过；
   - 收集到 `max_recent * 2` 条后停止，后面的 .lnk 不再解析；
   - mtime = .lnk 的修改时间。
2. **来源 D（桌面文件）**：`FOLDERID_Desktop` 顶层的文件：
   - 跳过隐藏和系统文件；
   - 跳过 `.lnk`、`.url`、`desktop.ini`，以及所有目录；
   - mtime = 文件的修改时间。
3. 合并两个来源 → 按路径小写去重（保留 mtime 较大的那条）→ 按 mtime 降序 → 截断到 `max_recent` 条。
4. **filekind.rs 映射表**（扩展名小写；不在表里的归 Other）：

| FileKind | 扩展名 |
|---|---|
| Document | doc docx odt rtf txt md wps |
| Spreadsheet | xls xlsx xlsm csv ods et |
| Presentation | ppt pptx odp key dps |
| Pdf | pdf |
| Image | png jpg jpeg gif bmp webp svg ico tif tiff heic psd ai |
| Video | mp4 mkv mov avi wmv flv webm m4v |
| Audio | mp3 wav flac aac m4a ogg wma |
| Archive | zip rar 7z tar gz bz2 xz iso |
| Installer | exe msi msix appx appxbundle |
| Code | rs c cpp h hpp cs js ts jsx tsx py java go kt swift json toml yaml yml xml html css sql sh ps1 bat |

### 4.4 dirs.rs — 常用文件夹

固定顺序：

1. 桌面（`FOLDERID_Desktop`）
2. 下载（`FOLDERID_Downloads`）
3. 文档（`FOLDERID_Documents`）
4. 图片（`FOLDERID_Pictures`）
5. 视频（`FOLDERID_Videos`）

然后追加**桌面上实际存在的**以下同名子目录（按这个顺序，路径小写去重）：`文档`、`表格`、`演示`、`PDF`、`图片`、`视频`、`音频`、`压缩包`、`安装包`、`代码`、`快捷方式`、`其他`。

> ⚠ 这份名单是按 FileKind 推出来的。DeskTidy 实际用的分类目录名如果不一样，由用户在这里改，agent 不要去读 DeskTidy 的代码。

- name：固定项用上面列出的中文名；追加项用目录名本身。
- count：顶层条目数（跳过隐藏和系统，不递归），数到 9999 就停。

### 4.5 launch.rs — 启动与打开

统一使用 `ShellExecuteExW`，参数如下：

- `fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI`
- `lpVerb = "open"`
- `nShow = SW_SHOWNORMAL`
- `lpFile` 按目标类型取值：
  - `AppsFolder(p)` → `"shell:AppsFolder\\" + p`
  - `Shortcut(path)` → path（工作目录和参数由快捷方式自己决定）
  - `open_path(p)` → p。调用前先检查路径是否存在，不存在返回 `PathNotFound`。

返回 FALSE 时：`Err(LaunchFailed{ name, code: GetLastError().0 })`，同时记一条 warn 日志。

`launch_app` 成功后：

1. `usage.launch_counts[id] += 1`；
2. `usage.last_launched[id] = 当前 Unix 秒`；
3. `normalize_query(from_query)` 非空时，`usage.query_picks[q] = id`；
4. 原子写入 `%LOCALAPPDATA%\DeskPad\usage.json`，失败只记 warn；Frequency 模式下立即重投影并发布 Snapshot。

`launch_app` 传入的 id 在当前快照里找不到 → `Err(UnknownApp(id))`。

### 4.6 store.rs — 持久化

数据目录：`SHGetKnownFolderPath(FOLDERID_LocalAppData)` + `\DeskPad\`，`init` 时创建（包括 `icons\` 子目录）。

| 文件 | 内容 | 读失败时 |
|---|---|---|
| `config.json` | `Settings`（serde_json 美化输出） | 回退到 `Settings::default()`，把坏文件改名为 `config.bad.json`，记 warn |
| `usage.json` | `search::Usage` | 回退到默认值 |
| `index.json` | `{ "version": 2, "apps": Vec<AppEntry> }` | 旧版本或解析失败均忽略，视为首次运行 |
| `log.txt` / `log.old.txt` | 日志 | — |

- **原子写**：先写带进程号后缀的 `xxx.<pid>.tmp`，flush 后用 `std::fs::rename` 覆盖正式文件（Windows 上底层是 MoveFileEx + REPLACE_EXISTING），避免并发实例/线程抢同一个临时文件。
- **加载 Settings 后钳制**：`max_recent` 限制在 1..=20；`version` 小于 `SETTINGS_VERSION` 时按迁移表升级（v0.1 没有迁移，直接把 version 改成 1）。
- `#[serde(default)]` 保证旧配置缺字段或多出未知字段都能读，**不允许因为解析失败清空用户设置**。model.rs 里有对应的测试，必须保留。

### 4.7 autostart.rs / single.rs

**autostart：**

- 写入位置：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `DeskPad`，REG_SZ，内容为 `"<当前 exe 完整路径>" --autostart`。exe 路径用 `GetModuleFileNameW(None)` 取得。
- **真实状态** = Run 值存在，并且 `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run` 下的 `DeskPad` 值不存在，或者它的第一个字节是偶数（奇数表示用户在任务管理器里禁用了）。
- 启用：写 Run 值，同时删除 StartupApproved 里的 `DeskPad` 值（清掉任务管理器的禁用标记）。
- 禁用：删除 Run 值。
- `init` 时用真实状态覆盖 `settings.autostart`；如果真实状态为启用但 Run 里的路径和当前 exe 不一致（exe 被移动过）→ 重写为当前路径。
- **只有 exe 文件名是 `DeskPad.exe` 时才允许写注册表**，probe 调用时一律跳过并记 info。这样可以防止 probe 把自己注册成开机启动项。

**single：**

- `CreateMutexW(None, false, w!("Local\\HexNook.DeskPad.SingleInstance"))` 之后**立刻**调用 `GetLastError()`；结果等于 `ERROR_ALREADY_EXISTS` → 关闭句柄并返回 `Err(AlreadyRunning)`。
- 成功时返回 `InstanceGuard`。**`init()` 不调用 acquire**，所以 probe 在 DeskPad 运行时也能正常使用。

### 4.8 lib.rs — Inner 与线程

```rust
struct Inner {
    snap: RwLock<Snapshot>,
    settings: Mutex<Settings>,
    usage: Mutex<search::Usage>,
    on_change: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    refresh_event: win::OwnedHandle,   // 自动复位事件
    shutdown_event: win::OwnedHandle,  // 手动复位事件
    worker: Mutex<Option<JoinHandle<()>>>,
    data_dir: PathBuf,
}
```

**锁顺序**：任何时候最多同时持有一把锁。调用 on_change 回调前必须先释放所有锁。

#### 4.8.1 init 流程

1. 创建数据目录，初始化日志，记一条 info：`init v{版本} exe={路径}`。
2. 加载 config.json → 钳制 → 读取 autostart 真实状态并覆盖。
3. 加载 usage.json。
4. 加载 index.json：
   - 版本 2 且存在 → 用 `all_apps` 和当前设置的投影生成 generation=1、ready=true 的初始快照（recent/dirs 先为空）；
   - 不存在或旧版本 → 空快照，ready=false。
5. 启动扫描线程（4.8.2）。
6. 如果第 4 步没有可用的 v2 index.json：在 Condvar 上最多等 3 秒，直到第一次 APPS 阶段完成发布。超时也照常返回（ready=false，UI 显示「正在读取应用…」）。
7. 返回 handle。

#### 4.8.2 扫描线程（只有这一个后台线程）

```
线程开头：win::ComInit::sta()（CoInitializeEx(COINIT_APARTMENTTHREADED)，Drop 时 CoUninitialize）
         SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN)   // 降低 CPU 和磁盘 IO 优先级，扫描时不和前台程序抢资源
         注册变更监听（4.8.3）
pending = APPS | FILES
loop {
    if pending 含 APPS  { apps::scan() → 发布(apps 替换, ready=true) → 写 index.json }
    if pending 含 FILES { recent::scan() + dirs::scan() → 发布 }
    icons::fill_missing(): 逐个提取，每 40 个发布一次，全部完成后再发布一次 → 写 index.json
    pending = 等待(shutdown, refresh, 各监听句柄)       // 无超时
    if shutdown 被触发 { break }
    去抖：继续等，直到连续 1000ms 没有新信号，期间收到的信号按 4.8.3 的规则累加到 pending
    距上次扫描结束不足 3 秒 → 补睡到满 3 秒
}
```

- **发布** = 获取写锁 → generation += 1 → 替换字段 → 释放锁 → 调用 on_change。
- **icons::fill_missing 期间也要响应 shutdown**：每提取一个图标就用 `WaitForSingleObject(shutdown, 0)` 检查一次。

#### 4.8.3 变更监听（取代 v1 的 10 分钟轮询）

`FindFirstChangeNotificationW` 监听以下目录，信号到达后调用 `FindNextChangeNotification` 继续监听：

| 目录 | 递归 | 过滤 | 触发 |
|---|---|---|---|
| `FOLDERID_Programs` | 是 | FILE_NAME \| DIR_NAME \| LAST_WRITE | APPS |
| `FOLDERID_CommonPrograms` | 是 | 同上 | APPS |
| `FOLDERID_Desktop` | 否 | FILE_NAME \| DIR_NAME \| LAST_WRITE | APPS \| FILES |
| `FOLDERID_PublicDesktop` | 否 | 同上 | APPS |
| `FOLDERID_Recent` | 否 | FILE_NAME \| LAST_WRITE | FILES |
| `settings.extra_scan_dirs` 及各自一级子目录 | 否 | FILE_NAME \| DIR_NAME \| LAST_WRITE | APPS |

- `refresh()` → SetEvent(refresh_event)，触发 APPS | FILES。UWP 应用安装不会改动上面这些目录，靠 UI 每次打开启动台时调用 refresh 来覆盖。
- `save_settings` 里 max_recent 变化 → 也走 refresh_event，但只设置 FILES 位（用一个 AtomicU8 存放待处理的扫描范围）。
- 额外扫描目录变化或 add/remove API 保存设置 → 设置 APPS 位并重建目录监听；不轮询。
- 某个目录监听失败 → 记 warn，跳过该目录，其他照常。

### 4.9 log.rs / win.rs

**log：**

- 文件：`%LocalAppData%\DeskPad\log.txt`；超过 1 MB 时改名为 `log.old.txt`（覆盖旧的）再新建。
- 格式：`2026-10-05 01:57:03.123 [INFO] [tid] 消息`。时间用本地时间，`GetLocalTime` 取得。
- 实现：`OnceLock<Mutex<Option<File>>>`。写入失败时静默忽略。debug 构建同时输出到 stderr。
- 不引入 log 或 tracing crate。

**win.rs 必备的 RAII 包装：**

- `OwnedHandle(HANDLE)`：Drop 时 CloseHandle。
- `ChangeHandle`：Drop 时 FindCloseChangeNotification。
- `HBitmapGuard(HBITMAP)`：Drop 时 DeleteObject。
- `CoTaskStr(PWSTR)`：提供 `to_string()`，Drop 时 CoTaskMemFree。
- `ComInit`：`ComInit::sta()` 调用 CoInitializeEx，Drop 时 CoUninitialize。
- 小工具：`known_folder(&GUID) -> Option<PathBuf>`、`to_wide(&str) -> Vec<u16>`（以 0 结尾）、`filetime_to_unix(FILETIME) -> i64`。

### 4.11 update.rs — 在线更新（v0.2 新增，questions.md #35）

**红线：WinINet 不进主进程。** 检查更新放在一次性子进程里做（照 4.x 的 `--thumb-pdf` 写法），
本模块被 app 的 main 识别 `--check-update` 后调用；它不建窗口、不占单实例锁、退出时不做清理。
为了让主进程的导入表里不出现 `wininet.dll`，本模块**动态加载** WinINet：
`LoadLibraryW(w!("wininet.dll"))` + `GetProcAddress` 取 6 个导出，签名手写、`transmute` 成函数指针，
常量（`INTERNET_OPEN_TYPE_PRECONFIG` 等）按 wininet.h 照抄。安装程序（crates/setup）复用同一份实现。

**公开 API：**

| 名字 | 说明 |
|---|---|
| `MANIFEST_URL` / `RELEASES_URL` | 发布仓库的 latest.json / 首页 |
| `is_newer(current, latest) -> bool` | 按 a.b.c 逐段**数字**比；任一边不是三段式 → false |
| `UpdateInfo` | `update.json` 的内容：`{checked_at, latest, url, sha256, notes, error}`，全部字段 `#[serde(default)]` |
| `load()` / `save(&UpdateInfo)` | 数据目录下 `update.json` 的读 / 原子写（先 .tmp 再 rename，同 4.6） |
| `install_dir() -> PathBuf` | `DeskPad.exe` 所在目录（有 `uninstall.exe` 就是安装目录，否则是便携 zip 的解压目录） |
| `get(url, progress) -> Result<Vec<u8>, String>` | GET 全部字节；WinINet 超时 20 秒，`INTERNET_OPEN_TYPE_PRECONFIG` 跟随系统代理 |
| `latest() -> Result<Release, String>` | 读 latest.json（version / url / sha256 / notes） |
| `check_update_worker() -> i32` | 子进程入口：读清单 → 写 update.json → `FindWindowW("DeskPadMain")` 投 `WM_APP_UPDATE`（WM_APP+6）→ 退出 |
| `UpdateGuard::spawn() -> bool` | 主进程侧：互斥 + 启动 `DeskPad.exe --check-update`（create_new 的 `update.lock`，>5 分钟的陈旧锁先清掉） |
| `AUTO_CHECK_INTERVAL` | 24×60×60：启动台每次打开时的最短检查间隔 |

**什么时候检查**（不加定时器、UI 不开线程，隐藏时 CPU 为 0）：启动台每次打开时看
`settings.auto_check_update` 与 `update.json` 的 `checked_at`，满足 24 小时间隔就 `UpdateGuard::spawn()`；
设置窗口的「检查更新」按钮立即 `spawn()`。子进程写完文件后投消息，主进程只在那一刻读一次、重绘一次。

### 4.10 bin/probe.rs — 无头验证（必须实现）

`main` 开头先 `ComInit::sta()`。所有命令都输出 UTF-8 文本；出错时退出码为 1，并把错误打到 stderr。

```
probe apps [--ids] [--all] # 默认只列 Snapshot.apps；--all 列 all_apps，并用 [system]/[hidden] 标记
                           # 先打印 "count=N ready=true"，再逐行打印：id | 来源 | 名称+标记 | alias | initials | 图标:有/无
probe icons             # 等待图标补全完成（最多 60 秒），打印命中率；把缺失项逐个列出
probe recent            # 每行：mtime(本地时间) | kind | 路径
probe dirs              # 每行：count | 名称 | 路径
probe search <q>        # 前 10 条：分数 | 名称（分数用 search::score 加上 pick 加成重新算一遍）
probe pick <q> <id>     # 模拟一次「从 q 启动 id」，只写 usage，不真正启动
probe launch <id>       # 真正启动
probe open <path>
probe settings          # 打印 config.json 当前内容，以及 autostart 真实状态
probe watch [秒数=60]   # 订阅 on_change，每次触发打印 generation 和三个列表的长度
probe refresh-loop <n>  # 连续 refresh n 次，每次等快照更新；每 10 次打印一次 PrivateUsage(MB)
```

## 5. 依赖（crates/engine/Cargo.toml，原样照抄）

```toml
[package]
name = "deskpad-engine"
version = "0.1.0"
edition = "2021"

[lib]
name = "deskpad_engine"
path = "src/lib.rs"

[[bin]]
name = "probe"
path = "src/bin/probe.rs"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
image = { version = "0.25", default-features = false, features = ["png"] }
windows = { version = "=0.58.0", features = [
  "Win32_Foundation",
  "Win32_Security",
  "Win32_Globalization",
  "Win32_Graphics_Gdi",
  "Win32_Storage_FileSystem",
  "Win32_System_Com",
  "Win32_System_Environment",
  "Win32_System_LibraryLoader",
  "Win32_System_ProcessStatus",
  "Win32_System_Registry",
  "Win32_System_Threading",
  "Win32_UI_Shell",
  "Win32_UI_WindowsAndMessaging",
] }
```

> 以上 feature 都已对照 windows 0.58.0 源码核对过。依赖关系中容易漏的几项：
> - `CreateMutexW` / `CreateEventW` 要求同时开启 `Win32_Security`；
> - `IShellLinkW::GetPath` 要求 `Win32_Storage_FileSystem`；
> - `IShellItemImageFactory::GetImage` 要求 `Win32_Graphics_Gdi`；
> - `IShellItem::BindToHandler` 和 `SHCreateItemFromParsingName` 要求 `Win32_System_Com`；
> - `ShellExecuteExW` 要求 `Win32_System_Registry`；
> - `WideCharToMultiByte` 在 `Win32_Globalization`；`K32GetProcessMemoryInfo` 在 `Win32_System_ProcessStatus`。
>
> 版本**锁死在 `=0.58.0`**，两个板块必须一致。不允许单方升级；要升级时整体升，并同时修改两份文档。不引入 anyhow 和 log 等其他 crate，错误类型统一用 `EngineError`。

## 6. 里程碑与验收（全部可以用 probe 独立验证）

| 里程碑 | 内容 | 验收 |
|---|---|---|
| **E0 契约桩** | 根 Cargo.toml、engine 的 Cargo.toml、model.rs、search.rs（照抄），lib.rs 写全部签名：`init` 返回空快照（ready=true），其余方法返回 `Ok(())`、空结果或默认值；log/single 做最简单的实现 | `cargo build` 整个 workspace 通过；`cargo test -p deskpad-engine` 通过（settings_roundtrip、ranking 两个测试） |
| E1 | store / log / single / autostart / win.rs + `probe settings` | 删掉 config.json 后运行 probe settings，打印出默认值；手写一个缺字段的 config.json，能读出来并且已有字段保持不变；`probe settings` 能打印 autostart 的真实状态 |
| E2 | apps.rs + keys.rs（暂不做图标） | `probe apps` 的 count ≥ 30，包含「计算器」「设置」（UWP）和「Microsoft Edge」；没有任何含「卸载」或 uninstall 的项；连续运行两次 `probe apps --ids`，输出逐字一致 |
| E3 | icons.rs | `probe icons` 命中率 ≥ 95%；PNG 都是 96×96，能用「照片」打开；至少 80% 的图标左上角像素 alpha = 0；第二次运行时新提取数为 0 |
| E4 | recent / dirs / launch | `probe recent` 条数 ≤ 8，按时间降序，不含目录；`probe launch <计算器 id>` 和 `probe launch <某个桌面快捷方式 id>` 都能真正启动；`probe open <某文件夹>` 能打开资源管理器 |
| E5 | 扫描线程 + 变更监听 + search 接入 | `probe search wx` 首条是微信；`probe pick ps <Photoshop id>` 之后，`probe search ps` 首条是 Photoshop；`probe watch` 运行期间往桌面放一个 .lnk，5 秒内打印出新的 generation；`probe refresh-loop 100` 第 10 次到第 100 次之间，PrivateUsage 增长 < 1 MB |

## 7. 与 UI 板块联调

1. UI 从 U2 开始接真实引擎（U1 用 E0 桩就能编译）。
2. 集成日检查单（两个板块都完成后一起过）：
   - `cargo build --release` 整个 workspace 通过，产物为 `target\release\DeskPad.exe` 和 `probe.exe`；
   - DeskPad 运行时 `probe apps` 照样能用（单实例互斥不影响 probe）；
   - 内存指标统一用任务管理器「详细信息」页的「内存(专用工作集)」：启动台隐藏 30 秒后测量，**目标 < 15 MB，红线 < 20 MB**；
   - exe 体积**目标 ≤ 3 MB，红线 < 5 MB**；
   - 红线 grep 自查：`rg "SetWindowsHookEx|WriteProcessMemory|ReadProcessMemory|CreateRemoteThread" crates` 无结果。
