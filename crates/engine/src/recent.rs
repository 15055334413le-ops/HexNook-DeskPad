//! 最近文件 + 桌面文件（engine 文档 4.3）。
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesExW, GetDriveTypeW,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM, GetFileExInfoStandard,
    WIN32_FILE_ATTRIBUTE_DATA,
};
use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Recent};

use crate::filekind::from_ext;

/// GetDriveTypeW 的「网络驱动器」返回值（常量在 Win32_System_WindowsProgramming，为它单开 feature 不值得）。
const DRIVE_REMOTE: u32 = 4;
use crate::model::FileEntry;
use crate::win;

/// 来源 R（Recent .lnk，先按 .lnk 修改时间降序解析，收集到 max_recent*2 停止）
/// + 来源 D（桌面顶层文件）→ 按路径小写去重（mtime 大者优先）→ mtime 降序 → 截断。
pub(crate) fn scan(max_recent: u32) -> Vec<FileEntry> {
    let mut all = scan_recent(max_recent);
    all.append(&mut scan_desktop());
    let mut by_path: HashMap<String, FileEntry> = HashMap::new();
    for e in all {
        let key = e.path.to_string_lossy().to_lowercase();
        match by_path.get_mut(&key) {
            Some(old) if old.mtime >= e.mtime => {}
            Some(old) => *old = e,
            None => {
                by_path.insert(key, e);
            }
        }
    }
    let mut out: Vec<FileEntry> = by_path.into_values().collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.mtime));
    out.truncate(max_recent as usize);
    out
}

fn scan_recent(max_recent: u32) -> Vec<FileEntry> {
    let Some(dir) = win::known_folder(&FOLDERID_Recent) else { return Vec::new() };
    let mut lnks: Vec<(PathBuf, i64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            let is_lnk = p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false);
            if is_lnk {
                if let Some(m) = file_mtime(&p) {
                    lnks.push((p, m));
                }
            }
        }
    }
    lnks.sort_by_key(|a| std::cmp::Reverse(a.1));
    let mut out = Vec::new();
    let cap = (max_recent * 2) as usize;
    for (path, mtime) in lnks {
        if out.len() >= cap {
            break;
        }
        // 同 4.1.2 的 IShellLinkW 方式解析，不调用 Resolve()。
        let Some(target) = crate::apps::lnk_target(&path) else { continue };
        if target.is_empty() {
            continue;
        }
        let target = if target.contains('%') { expand_env(&target) } else { target };
        // UNC 或网络盘 → 跳过，不做存在性检查（避免离线网络盘卡住扫描）。
        if target.starts_with("\\\\") || is_remote_drive(&target) {
            continue;
        }
        // 文件不存在或目录 → 跳过。
        let Some(attrs) = win::file_attributes(&target) else { continue };
        if attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            continue;
        }
        let pb = PathBuf::from(&target);
        let ext = pb
            .extension()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let name = pb
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        let kind = from_ext(&ext);
        out.push(FileEntry { path: pb, name, ext, kind, mtime, thumb_png: None });
    }
    out
}

fn scan_desktop() -> Vec<FileEntry> {
    let Some(dir) = win::known_folder(&FOLDERID_Desktop) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        let file_name = path
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        if file_name.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        let ext = path
            .extension()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext == "lnk" || ext == "url" {
            continue;
        }
        let Some(attrs) = win::file_attributes(&path.to_string_lossy()) else { continue };
        if attrs & (FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0
        {
            continue;
        }
        let Some(mtime) = file_mtime(&path) else { continue };
        let kind = from_ext(&ext);
        out.push(FileEntry { path, name: file_name, ext, kind, mtime, thumb_png: None });
    }
    out
}

/// 文件修改时间（Unix 秒），失败返回 None。
pub(crate) fn file_mtime(path: &Path) -> Option<i64> {
    let wide = win::to_wide(&path.to_string_lossy());
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    // SAFETY: 缓冲区由本函数持有；调用失败以 Err 报告。
    unsafe {
        GetFileAttributesExW(
            PCWSTR(wide.as_ptr()),
            GetFileExInfoStandard,
            &mut data as *mut _ as *mut core::ffi::c_void,
        )
        .ok()?;
    }
    Some(win::filetime_to_unix(data.ftLastWriteTime))
}

fn expand_env(s: &str) -> String {
    let wide = win::to_wide(s);
    let mut buf = [0u16; 1024];
    // SAFETY: 输入以 0 结尾；输出缓冲区由本函数持有。返回值含结尾 0 的字符数。
    let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..n.saturating_sub(1)])
}

fn is_remote_drive(path: &str) -> bool {
    let cs: Vec<char> = path.chars().collect();
    if cs.len() < 2 || !cs[0].is_ascii_alphabetic() || cs[1] != ':' {
        return false;
    }
    let root = format!("{}:\\", cs[0]);
    let wide = win::to_wide(&root);
    // SAFETY: wide 以 0 结尾。
    unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) == DRIVE_REMOTE }
}
