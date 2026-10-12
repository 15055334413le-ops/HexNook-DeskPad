//! 常用文件夹（engine 文档 4.4）。
//!
//! 固定顺序：桌面、下载、文档、图片、视频；随后追加桌面上实际存在的
//! 分类子目录（按固定名单顺序，路径小写去重）。
use std::collections::HashSet;
use std::path::Path;

use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_Videos,
};

use crate::model::DirEntry;
use crate::win;

const DESKTOP_SUBDIRS: [&str; 12] = [
    "文档", "表格", "演示", "PDF", "图片", "视频", "音频", "压缩包", "安装包", "代码", "快捷方式",
    "其他",
];

pub(crate) fn scan() -> Vec<DirEntry> {
    let mut out: Vec<DirEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let fixed = [
        (&FOLDERID_Desktop, "桌面"),
        (&FOLDERID_Downloads, "下载"),
        (&FOLDERID_Documents, "文档"),
        (&FOLDERID_Pictures, "图片"),
        (&FOLDERID_Videos, "视频"),
    ];
    for (guid, name) in fixed {
        if let Some(p) = win::known_folder(guid) {
            if seen.insert(p.to_string_lossy().to_lowercase()) {
                let count = count_entries(&p);
                out.push(DirEntry { path: p, name: name.to_string(), count });
            }
        }
    }
    if let Some(desktop) = win::known_folder(&FOLDERID_Desktop) {
        for name in DESKTOP_SUBDIRS {
            let p = desktop.join(name);
            if p.is_dir() && seen.insert(p.to_string_lossy().to_lowercase()) {
                let count = count_entries(&p);
                out.push(DirEntry { path: p, name: name.to_string(), count });
            }
        }
    }
    out
}

/// 顶层条目数（跳过隐藏和系统，不递归），数到 9999 就停。
fn count_entries(dir: &Path) -> u32 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0u32;
    for e in rd.flatten() {
        let p = e.path();
        let Some(attrs) = win::file_attributes(&p.to_string_lossy()) else { continue };
        if attrs & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0 {
            continue;
        }
        n += 1;
        if n >= 9999 {
            break;
        }
    }
    n
}
