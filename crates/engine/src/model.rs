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
    /// 内容缩略图 PNG（图片 / 视频首帧 / 演示文稿封面 / PDF 首页，边长 ≤160，保持比例）。
    /// None = 该类型不取缩略图或系统没有，UI 画扁平类型图标。
    #[serde(default)]
    pub thumb_png: Option<PathBuf>,
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

/// 右侧「最近文件 / 常用文件夹」的显示方式，用户在卡片右上角切换，各自独立保存。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    /// 图标视图（默认）
    #[default]
    Grid,
    /// 列表视图
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    Dark,
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
    pub first_run_done: bool,
    /// 最近文件的显示方式。
    pub files_view: ViewMode,
    /// 常用文件夹的显示方式。
    pub dirs_view: ViewMode,
    /// 打开启动台时自动检查更新（距上次 ≥24 小时才真正发起；questions.md #35）。
    /// serde default = true，旧 config.json 缺这个字段也能读。
    pub auto_check_update: bool,
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
            files_view: ViewMode::Grid,
            dirs_view: ViewMode::Grid,
            auto_check_update: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// 每次发布 +1；UI 用它判断是否需要重建缓存。
    pub generation: u64,
    /// false = 首次扫描尚未完成（UI 显示「正在读取应用…」）。
    pub ready: bool,
    /// 已按 keys.sort_key 升序排列（空查询时的显示顺序）。
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
