//! 应用索引：AppsFolder 枚举 + 桌面快捷方式 + 过滤去重（engine 文档 4.1）。
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::core::{w, GUID, Interface, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
use windows::Win32::UI::Shell::{
    PropertiesSystem::PROPERTYKEY, BHID_EnumItems, FOLDERID_CommonPrograms, FOLDERID_Desktop,
    FOLDERID_Programs, FOLDERID_PublicDesktop, FOLDERID_System, FOLDERID_SystemX86,
    FOLDERID_Windows, IEnumShellItems, IShellItem, IShellItem2, IShellLinkW,
    SHCreateItemFromParsingName, ShellLink, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

use crate::keys;
use crate::model::{AppEntry, AppSource, LaunchTarget, Settings};
use crate::win;

/// Windows 管理/维护工具的内置 exe 名单（小写、不含扩展名）。
pub(crate) const SYSTEM_TOOL_EXES: &[&str] = &[
    "odbcad32",
    "regedit",
    "msconfig",
    "perfmon",
    "eventvwr",
    "compmgmt",
    "services",
    "taskschd",
    "dfrgui",
    "cleanmgr",
    "iscsicpl",
    "msinfo32",
    "resmon",
    "recoverydrive",
    "mdsched",
    "secpol",
    "gpedit",
    "diskmgmt",
    "printmanagement",
    "comexp",
    "wf",
];

const PKEY_LINK_TARGET_PARSING_PATH: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb9b4b3fc_2b51_4a42_b5d8_324146afcf25),
    pid: 2,
};

#[derive(Default)]
struct SystemTools {
    system_root: Option<PathBuf>,
    known_folders: Vec<(String, PathBuf)>,
    known_targets: HashSet<String>,
}

impl SystemTools {
    fn detect() -> Self {
        let system_root = win::known_folder(&FOLDERID_Windows);
        let mut shortcut_roots = Vec::new();
        for programs in [
            win::known_folder(&FOLDERID_Programs),
            win::known_folder(&FOLDERID_CommonPrograms),
        ]
        .into_iter()
        .flatten()
        {
            for folder in ["Administrative Tools", "Windows Tools"] {
                let root = programs.join(folder);
                if root.is_dir() {
                    shortcut_roots.push(root);
                }
            }
        }
        let known_folders = [
            (&FOLDERID_System, "system"),
            (&FOLDERID_SystemX86, "system_x86"),
            (&FOLDERID_Windows, "windows"),
        ]
        .into_iter()
        .filter_map(|(guid, _)| {
            win::known_folder(guid).map(|path| (guid_to_braced(guid), path))
        })
        .collect();
        let mut known_targets = HashSet::new();
        for root in &shortcut_roots {
            let Ok(rd) = std::fs::read_dir(root) else {
                continue;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("lnk"))
                {
                    if let Some(target) = lnk_target(&path) {
                        known_targets.insert(normalized_path(Path::new(&target)));
                    }
                }
            }
        }
        Self {
            system_root,
            known_folders,
            known_targets,
        }
    }

    fn classify(&self, name: &str, shortcut: Option<&Path>, target: Option<&Path>) -> bool {
        if matches!(name.to_lowercase().as_str(), "windows tools" | "windows 工具") {
            return true;
        }
        if shortcut.is_some_and(|path| is_tool_shortcut_path(path, &self.known_targets)) {
            return true;
        }
        let Some(target) = target else {
            return false;
        };
        if self.known_targets.contains(&normalized_path(target)) {
            return true;
        }
        let Some(root) = &self.system_root else {
            return false;
        };
        let exe = target
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        is_system_target(root, target, &exe)
    }

    fn expand_parsing(&self, parsing: &str) -> Option<PathBuf> {
        let direct = PathBuf::from(parsing);
        if direct.is_absolute() {
            return Some(direct);
        }
        let (prefix, rest) = parsing.split_once('\\')?;
        let (_, root) = self
            .known_folders
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(prefix))?;
        Some(root.join(rest))
    }
}

fn guid_to_braced(guid: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7],
    )
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

#[cfg(test)]
pub(crate) fn stable_setting_id(launch: &LaunchTarget) -> String {
    match launch {
        LaunchTarget::AppsFolder(parsing) => parsing.to_lowercase(),
        LaunchTarget::Shortcut(path) => {
            let target = if path
                .extension()
                .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("lnk"))
            {
                lnk_target(path).map(PathBuf::from).unwrap_or_else(|| path.clone())
            } else {
                path.clone()
            };
            normalized_path(&target)
        }
    }
}

pub(crate) fn stable_setting_id_plain(launch: &LaunchTarget) -> String {
    match launch {
        LaunchTarget::AppsFolder(parsing) => parsing.to_lowercase(),
        LaunchTarget::Shortcut(path) => normalized_path(path),
    }
}

/// 目标是 Windows 目录下的内置管理工具 exe 时返回 true。
pub(crate) fn is_system_target(system_root: &Path, target: &Path, exe_name: &str) -> bool {
    let root = normalized_path(system_root);
    let target = normalized_path(target);
    let root_with_sep = format!("{}\\", root.trim_end_matches('\\'));
    if !target.starts_with(&root_with_sep) {
        return false;
    }
    let stem = Path::new(exe_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    SYSTEM_TOOL_EXES.contains(&stem.as_str())
}

/// 位于开始菜单 Administrative Tools / Windows Tools 目录的快捷方式视为系统工具。
pub(crate) fn is_tool_shortcut_path(path: &Path, _known_targets: &HashSet<String>) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_lowercase();
        matches!(name.as_str(), "administrative tools" | "windows tools")
    })
}

/// UNC 与映射网络盘都拒绝；只扫描本地目录。
pub(crate) fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if text.starts_with(r"\\") {
        return true;
    }
    let Some(drive) = text.get(..2).filter(|s| s.as_bytes().get(1) == Some(&b':')) else {
        return false;
    };
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    const DRIVE_REMOTE: u32 = 4;
    let root = format!("{drive}\\");
    let wide = win::to_wide(&root);
    // SAFETY: wide 以 0 结尾，且 root 只包含盘符与反斜杠。
    unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) == DRIVE_REMOTE }
}

/// 只扫描根目录与一级子目录中的 exe/lnk/url。
#[cfg(test)]
pub(crate) fn scan_extra_dir(root: &Path) -> Vec<AppEntry> {
    scan_extra_dir_with(root, &SystemTools::default())
}

fn scan_extra_dir_with(root: &Path, tools: &SystemTools) -> Vec<AppEntry> {
    if is_network_path(root) {
        return Vec::new();
    }
    let mut out = scan_extra_files(root, tools);
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() || is_uninstall_name(&path) || is_network_path(&path) {
            continue;
        }
        out.extend(scan_extra_files(&path, tools));
    }
    out
}

fn scan_extra_files(dir: &Path, tools: &SystemTools) -> Vec<AppEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().map(|x| x.to_string_lossy().to_lowercase()) else {
            continue;
        };
        let app = match ext.as_str() {
            "exe" => path_entry(&path, AppSource::ExtraScanDir, tools),
            "lnk" => lnk_entry_from(&path, AppSource::ExtraScanDir, tools),
            "url" => url_entry_from(&path, AppSource::ExtraScanDir, tools),
            _ => None,
        };
        if let Some(app) = app {
            out.push(app);
        }
    }
    out
}

fn is_uninstall_name(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name.contains("卸载") || name.contains("uninstall")
}

fn scan_extra_dirs(dirs: &[PathBuf], tools: &SystemTools) -> Vec<AppEntry> {
    let mut out = Vec::new();
    for dir in dirs {
        out.extend(scan_extra_dir_with(dir, tools));
    }
    out
}

fn scan_manual_apps(paths: &[PathBuf], tools: &SystemTools) -> Vec<AppEntry> {
    let mut out = Vec::new();
    for path in paths {
        let Some(ext) = path.extension().map(|x| x.to_string_lossy().to_lowercase()) else {
            continue;
        };
        let app = match ext.as_str() {
            "exe" => path_entry(path, AppSource::Manual, tools),
            "lnk" => lnk_entry_from(path, AppSource::Manual, tools),
            "url" => url_entry_from(path, AppSource::Manual, tools),
            _ => None,
        };
        if let Some(app) = app {
            out.push(app);
        }
    }
    out
}

/// 全量扫描（主线程为扫描线程，COM STA 已初始化）。
/// 排序：keys.sort_key 升序，相同则 name 升序（4.1.3）。
pub(crate) fn scan(settings: &Settings) -> Vec<AppEntry> {
    let t0 = std::time::Instant::now();
    let tools = SystemTools::detect();
    let groups = vec![
        scan_apps_folder(&tools),
        scan_desktop_shortcuts(&tools),
        scan_extra_dirs(&settings.extra_scan_dirs, &tools),
        scan_manual_apps(&settings.manual_apps, &tools),
    ];
    let group_counts: Vec<_> = groups.iter().map(Vec::len).collect();
    let mut seen_names = HashSet::new();
    let mut apps = Vec::new();
    for mut group in groups {
        group.retain(|app| !seen_names.contains(&app.name.to_lowercase()));
        for app in &group {
            seen_names.insert(app.name.to_lowercase());
        }
        apps.append(&mut group);
    }
    apps.sort_by(|a, b| {
        a.keys
            .sort_key
            .cmp(&b.keys.sort_key)
            .then(a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    });
    crate::log::info(&format!(
        "apps::scan af={} sc={} extra={} manual={} total={} in {:?}",
        group_counts[0],
        group_counts[1],
        group_counts[2],
        group_counts[3],
        apps.len(),
        t0.elapsed()
    ));
    apps
}

/// 来源 A：shell:AppsFolder（开始菜单「所有应用」，覆盖 .lnk 与 UWP）。
fn scan_apps_folder(tools: &SystemTools) -> Vec<AppEntry> {
    let mut out = Vec::new();
    // SAFETY: 扫描线程 COM STA 已初始化；COM 对象由 Rust 封装自动释放。
    unsafe {
        let root: IShellItem = match SHCreateItemFromParsingName(w!("shell:AppsFolder"), None) {
            Ok(i) => i,
            Err(e) => {
                crate::log::warn(&format!("SHCreateItemFromParsingName(shell:AppsFolder): {e}"));
                return out;
            }
        };
        let items: IEnumShellItems = match root.BindToHandler(None, &BHID_EnumItems) {
            Ok(e) => e,
            Err(e) => {
                crate::log::warn(&format!("BindToHandler(BHID_EnumItems): {e}"));
                return out;
            }
        };
        let mut buf: [Option<IShellItem>; 1] = [None];
        let mut fetched = 0u32;
        loop {
            let hr = items.Next(&mut buf, Some(&mut fetched));
            if hr.is_err() || fetched == 0 {
                break;
            }
            for slot in buf.iter_mut().take(fetched as usize) {
                if let Some(child) = slot.take() {
                    if let Some(entry) = apps_folder_entry(&child, tools) {
                        out.push(entry);
                    }
                }
            }
            buf = [None];
        }
    }
    out
}

/// 单个 AppsFolder 条目 → AppEntry；失败/被过滤返回 None。
fn apps_folder_entry(child: &IShellItem, tools: &SystemTools) -> Option<AppEntry> {
    // SAFETY: GetDisplayName 返回的 PWSTR 由 CoTaskStr 托管释放。
    unsafe {
        let name_pw = child.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?;
        let name = win::CoTaskStr::new(name_pw).to_string();
        let parsing_pw = child.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()?;
        let parsing = win::CoTaskStr::new(parsing_pw).to_string();
        if name.is_empty() || parsing.is_empty() {
            return None;
        }
        let alias = keys::alias_from_parsing(&parsing);
        if filtered(&name, &alias) {
            return None;
        }
        let id = keys::fnv1a64(format!("af:{}", parsing.to_lowercase()).as_bytes());
        let sk = keys::compute(&name, &alias);
        let target = shell_item_target(child).or_else(|| tools.expand_parsing(&parsing));
        let is_system = tools.classify(&name, None, target.as_deref());
        let setting_id = parsing.to_lowercase();
        Some(AppEntry {
            id,
            name,
            launch: LaunchTarget::AppsFolder(parsing),
            icon_png: None,
            source: AppSource::StartMenu,
            keys: sk,
            is_system,
            hidden: false,
            setting_id,
        })
    }
}

/// shell:AppsFolder 中的 .lnk/.url 项通过属性取目标；失败的项由 parsing name 兜底。
fn shell_item_target(child: &IShellItem) -> Option<PathBuf> {
    let item: IShellItem2 = child.cast().ok()?;
    // SAFETY: GetString 返回的 PWSTR 由 CoTaskStr 托管释放；属性键常量在静态存储中。
    let target = unsafe { item.GetString(&PKEY_LINK_TARGET_PARSING_PATH) }.ok()?;
    let target = win::CoTaskStr::new(target).to_string();
    (!target.is_empty()).then(|| PathBuf::from(target))
}

/// 来源 B：桌面与公共桌面的顶层 .lnk / .url（不递归）。
fn scan_desktop_shortcuts(tools: &SystemTools) -> Vec<AppEntry> {
    let mut out = Vec::new();
    let dirs = [
        win::known_folder(&FOLDERID_Desktop),
        win::known_folder(&FOLDERID_PublicDesktop),
    ];
    for dir in dirs.into_iter().flatten() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Some(ext) = path.extension().map(|x| x.to_string_lossy().to_lowercase()) else {
                continue;
            };
            match ext.as_str() {
                "lnk" => {
                    if let Some(a) = lnk_entry_from(&path, AppSource::Desktop, tools) {
                        out.push(a);
                    }
                }
                "url" => {
                    if let Some(a) = url_entry_from(&path, AppSource::Desktop, tools) {
                        out.push(a);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// 读取 .lnk 的目标路径（IShellLinkW 方式，不调用 Resolve）。MSI 广告快捷方式返回空串。
pub(crate) fn lnk_target(path: &Path) -> Option<String> {
    // SAFETY: COM STA 已初始化；COM 对象由 Rust 封装自动释放；缓冲区由本函数持有。
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        let wide = win::to_wide(&path.to_string_lossy());
        pf.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let mut target_buf = [0u16; 260];
        link.GetPath(&mut target_buf, std::ptr::null_mut(), 0).ok()?;
        let len = target_buf.iter().position(|&c| c == 0).unwrap_or(target_buf.len());
        Some(String::from_utf16_lossy(&target_buf[..len]))
    }
}

/// 解析 .lnk：目标为空（MSI 广告）或目标扩展名属于 exe/msc/bat/cmd/appref-ms → 应用。
fn lnk_entry_from(path: &Path, source: AppSource, tools: &SystemTools) -> Option<AppEntry> {
    let target = lnk_target(path)?;

    let t_ext = std::path::Path::new(&target)
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
        let is_app = target.is_empty()
            || matches!(t_ext.as_str(), "exe" | "msc" | "bat" | "cmd" | "appref-ms");
        if !is_app {
            return None;
        }
        // 目标为空（MSI）或 .url 时 alias 为空；.lnk 用目标 exe 文件名去扩展名。
        let alias = if target.is_empty() {
            String::new()
        } else {
            keys::alias_from_target(&target)
        };
        let name = display_name(path);
        if filtered(&name, &alias) {
            return None;
        }
        let id = keys::fnv1a64(format!("sc:{}", path.to_string_lossy().to_lowercase()).as_bytes());
        let sk = keys::compute(&name, &alias);
        let is_system = tools.classify(
            &name,
            Some(path),
            if target.is_empty() { None } else { Some(Path::new(&target)) },
        );
        let setting_id = if target.is_empty() {
            normalized_path(path)
        } else {
            normalized_path(Path::new(&target))
        };
        Some(AppEntry {
            id,
            name,
            launch: LaunchTarget::Shortcut(path.to_path_buf()),
            icon_png: None,
            source,
            keys: sk,
            is_system,
            hidden: false,
            setting_id,
        })
}

/// 解析 .url：URL= 行的 scheme 不是 http/https/file → 应用。
fn url_entry_from(path: &Path, source: AppSource, tools: &SystemTools) -> Option<AppEntry> {
    let bytes = std::fs::read(path).ok()?;
    let text = decode_text_file(&bytes);
    let mut target = None;
    for line in text.lines() {
        let t = line.trim();
        let b = t.as_bytes();
        if b.len() >= 4 && b[..4].eq_ignore_ascii_case(b"url=") {
            // 前 4 字节是 ASCII，字节 4 必是字符边界。
            target = Some(t[4..].trim().to_string());
            break;
        }
    }
    let target = target?;
    let scheme = match target.split_once("://") {
        Some((s, _)) => s.to_lowercase(),
        None => String::new(),
    };
    if matches!(scheme.as_str(), "http" | "https" | "file") {
        return None;
    }
    let name = display_name(path);
    if filtered(&name, "") {
        return None;
    }
    let id = keys::fnv1a64(format!("sc:{}", path.to_string_lossy().to_lowercase()).as_bytes());
    let sk = keys::compute(&name, "");
    let is_system = tools.classify(&name, Some(path), None);
    let setting_id = normalized_path(path);
    Some(AppEntry {
        id,
        name,
        launch: LaunchTarget::Shortcut(path.to_path_buf()),
        icon_png: None,
        source,
        keys: sk,
        is_system,
        hidden: false,
        setting_id,
    })
}

fn path_entry(path: &Path, source: AppSource, tools: &SystemTools) -> Option<AppEntry> {
    let name = display_name(path);
    let alias = keys::alias_from_target(&path.to_string_lossy());
    if filtered(&name, &alias) {
        return None;
    }
    let path_key = normalized_path(path);
    let id = keys::fnv1a64(format!("{source:?}:{path_key}").as_bytes());
    let is_system = tools.classify(&name, None, Some(path));
    let setting_id = path_key;
    Some(AppEntry {
        id,
        name: name.clone(),
        launch: LaunchTarget::Shortcut(path.to_path_buf()),
        icon_png: None,
        source,
        keys: keys::compute(&name, &alias),
        is_system,
        hidden: false,
        setting_id,
    })
}

/// .url 可能是 ANSI/UTF-8 或 UTF-16（带 BOM）；统一成 String。
fn decode_text_file(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

/// 文件名去扩展名，再去掉末尾「 - 快捷方式」/「 - Shortcut」（不区分大小写）。
fn display_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let lower = stem.to_lowercase();
    for suf in [" - 快捷方式", " - shortcut"] {
        if let Some(pos) = lower.rfind(suf) {
            if pos + suf.len() == lower.len() {
                return stem[..pos].trim_end().to_string();
            }
        }
    }
    stem
}

/// 过滤（name 转小写后判断，4.1.3）：
/// 含 卸载/帮助/说明/官网 → 丢弃；切词后出现 uninstall/readme/help/documentation/manual/website → 丢弃；
/// alias 以 unins 开头 → 丢弃。
fn filtered(name: &str, alias: &str) -> bool {
    let lower = name.to_lowercase();
    if ["卸载", "帮助", "说明", "官网"].iter().any(|w| lower.contains(w)) {
        return true;
    }
    let tokens: HashSet<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    if ["uninstall", "readme", "help", "documentation", "manual", "website"]
        .iter()
        .any(|w| tokens.contains(*w))
    {
        return true;
    }
    alias.starts_with("unins")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use super::{
        is_network_path, is_system_target, is_tool_shortcut_path, scan_extra_dir,
        scan_manual_apps, stable_setting_id, SystemTools,
    };
    use crate::model::{AppSource, LaunchTarget};

    #[test]
    fn classifies_only_known_system_executables_and_tool_folder_shortcuts() {
        let system_root = Path::new(r"C:\Windows");
        assert!(is_system_target(
            system_root,
            Path::new(r"C:\Windows\System32\regedit.exe"),
            "regedit.exe",
        ));
        assert!(is_system_target(
            system_root,
            Path::new(r"C:\Windows\SysWOW64\odbcad32.exe"),
            "odbcad32.exe",
        ));
        assert!(!is_system_target(
            system_root,
            Path::new(r"C:\Windows\System32\notepad.exe"),
            "notepad.exe",
        ));
        assert!(!is_system_target(
            system_root,
            Path::new(r"C:\Program Files\Git\git-bash.exe"),
            "git-bash.exe",
        ));

        let tools = HashSet::from([
            r"c:\windows\system32\perfmon.exe".to_string(),
            r"c:\windows\system32\eventvwr.msc".to_string(),
        ]);
        assert!(is_tool_shortcut_path(
            Path::new(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Administrative Tools\Event Viewer.lnk"),
            &tools,
        ));
        assert!(!is_tool_shortcut_path(
            Path::new(r"C:\Users\me\Desktop\Event Viewer.lnk"),
            &tools,
        ));
    }

    #[test]
    fn stable_setting_id_uses_parsing_name_or_target_path_lowercase() {
        assert_eq!(
            stable_setting_id(&LaunchTarget::AppsFolder("Some.Vendor_App!App".into())),
            "some.vendor_app!app"
        );
        assert_eq!(
            stable_setting_id(&LaunchTarget::Shortcut(Path::new(r"C:\Tools\Editor.EXE").into())),
            r"c:\tools\editor.exe"
        );
    }

    #[test]
    fn extra_dir_scans_only_root_and_first_level_and_skips_uninstall_dirs() {
        let root = std::env::temp_dir().join(format!(
            "deskpad-engine-extra-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let child = root.join("Tools");
        let uninstall = root.join("卸载 Tools");
        let deeper = child.join("deeper");
        std::fs::create_dir_all(&deeper).unwrap();
        std::fs::create_dir_all(&uninstall).unwrap();
        std::fs::write(root.join("Root.exe"), []).unwrap();
        std::fs::write(child.join("Child.exe"), []).unwrap();
        std::fs::write(deeper.join("Deep.exe"), []).unwrap();
        std::fs::write(uninstall.join("Remover.exe"), []).unwrap();
        std::fs::write(root.join("notes.txt"), []).unwrap();

        let apps = scan_extra_dir(&root);
        let names: HashSet<_> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(apps.len(), 2);
        assert!(names.contains("Root"));
        assert!(names.contains("Child"));
        assert!(apps.iter().all(|a| a.source == AppSource::ExtraScanDir));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_unc_network_scan_roots() {
        assert!(is_network_path(Path::new(r"\\server\share\apps")));
        assert!(!is_network_path(Path::new(r"C:\Tools")));
    }

    #[test]
    fn manual_apps_accept_url_shortcuts() {
        let root = std::env::temp_dir().join(format!(
            "deskpad-engine-manual-url-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let url = root.join("Steam Game.url");
        std::fs::write(&url, "[InternetShortcut]\r\nURL=steam://rungameid/570\r\n").unwrap();
        let apps = scan_manual_apps(&[url.clone()], &SystemTools::default());
        assert_eq!(apps.len(), 1);
        assert!(matches!(
            &apps[0].launch,
            LaunchTarget::Shortcut(p) if p == &url
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_system_root_and_expands_known_folder_parsing() {
        let tools = SystemTools::detect();
        let root = tools.system_root.as_ref().unwrap();
        let target = root.join("System32").join("odbcad32.exe");
        assert!(tools.classify("ODBC", None, Some(&target)));
        let expanded = tools
            .expand_parsing(r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\odbcad32.exe")
            .unwrap();
        assert!(expanded.to_string_lossy().to_lowercase().ends_with(r"\system32\odbcad32.exe"));
    }
}
