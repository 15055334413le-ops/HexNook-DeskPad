//! 应用索引：AppsFolder 枚举 + 桌面快捷方式 + 过滤去重（engine 文档 4.1）。
use std::collections::HashSet;
use std::path::Path;

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_Desktop, FOLDERID_PublicDesktop, IEnumShellItems, IShellItem,
    IShellLinkW, SHCreateItemFromParsingName, ShellLink, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};

use crate::keys;
use crate::model::{AppEntry, AppSource, LaunchTarget};
use crate::win;

/// 全量扫描（主线程为扫描线程，COM STA 已初始化）。
/// 排序：keys.sort_key 升序，相同则 name 升序（4.1.3）。
pub(crate) fn scan() -> Vec<AppEntry> {
    let t0 = std::time::Instant::now();
    let mut from_af = scan_apps_folder();
    let mut from_sc = scan_desktop_shortcuts();
    // 去重：来源 B 的条目，name 小写与来源 A 任一项相同 → 丢弃 B（A 内部不去重）。
    let af_names: HashSet<String> = from_af.iter().map(|a| a.name.to_lowercase()).collect();
    from_sc.retain(|a| !af_names.contains(&a.name.to_lowercase()));
    let n_af = from_af.len();
    let n_sc = from_sc.len();
    from_af.append(&mut from_sc);
    from_af.sort_by(|a, b| a.keys.sort_key.cmp(&b.keys.sort_key).then(a.name.cmp(&b.name)));
    crate::log::info(&format!(
        "apps::scan af={n_af} sc={n_sc} total={} in {:?}",
        from_af.len(),
        t0.elapsed()
    ));
    from_af
}

/// 来源 A：shell:AppsFolder（开始菜单「所有应用」，覆盖 .lnk 与 UWP）。
fn scan_apps_folder() -> Vec<AppEntry> {
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
                    if let Some(entry) = apps_folder_entry(&child) {
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
fn apps_folder_entry(child: &IShellItem) -> Option<AppEntry> {
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
        Some(AppEntry {
            id,
            name,
            launch: LaunchTarget::AppsFolder(parsing),
            icon_png: None,
            source: AppSource::StartMenu,
            keys: sk,
        })
    }
}

/// 来源 B：桌面与公共桌面的顶层 .lnk / .url（不递归）。
fn scan_desktop_shortcuts() -> Vec<AppEntry> {
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
                    if let Some(a) = lnk_entry(&path) {
                        out.push(a);
                    }
                }
                "url" => {
                    if let Some(a) = url_entry(&path) {
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
fn lnk_entry(path: &Path) -> Option<AppEntry> {
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
        Some(AppEntry {
            id,
            name,
            launch: LaunchTarget::Shortcut(path.to_path_buf()),
            icon_png: None,
            source: AppSource::Desktop,
            keys: sk,
        })
}

/// 解析 .url：URL= 行的 scheme 不是 http/https/file → 应用。
fn url_entry(path: &Path) -> Option<AppEntry> {
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
    Some(AppEntry {
        id,
        name,
        launch: LaunchTarget::Shortcut(path.to_path_buf()),
        icon_png: None,
        source: AppSource::Desktop,
        keys: sk,
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
