//! config.json / usage.json / index.json 原子读写（engine 文档 4.6）。
//!
//! 数据目录：%LocalAppData%\DeskPad\。原子写 = 先写 xxx.tmp，flush 后
//! std::fs::rename 覆盖正式文件（Windows 上底层是 MoveFileEx + REPLACE_EXISTING）。
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::UI::Shell::FOLDERID_LocalAppData;

use crate::model::{AppEntry, EngineError, Settings, SETTINGS_VERSION};
use crate::search::Usage;
use crate::{log, win};

pub(crate) fn data_dir() -> Result<PathBuf, EngineError> {
    win::known_folder(&FOLDERID_LocalAppData)
        .map(|p| p.join("DeskPad"))
        .ok_or_else(|| EngineError::Io("SHGetKnownFolderPath(FOLDERID_LocalAppData) failed".into()))
}

/// 读取 config.json；不存在或损坏时回退默认值（坏文件改名为 config.bad.json），并钳制。
pub(crate) fn load_settings(dir: &Path) -> Settings {
    let path = dir.join("config.json");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return clamp(Settings::default()),
    };
    match serde_json::from_str::<Settings>(&text) {
        Ok(s) => clamp(s),
        Err(e) => {
            log::warn(&format!("config.json parse failed: {e}; falling back to defaults"));
            let _ = fs::rename(&path, dir.join("config.bad.json"));
            clamp(Settings::default())
        }
    }
}

pub(crate) fn save_settings(dir: &Path, s: &Settings) -> Result<(), EngineError> {
    atomic_write_json(dir, "config.json", s)
}

pub(crate) fn load_usage(dir: &Path) -> Usage {
    match fs::read_to_string(dir.join("usage.json")) {
        Ok(text) => serde_json::from_str::<Usage>(&text).unwrap_or_else(|e| {
            log::warn(&format!("usage.json parse failed: {e}; falling back to defaults"));
            Usage::default()
        }),
        Err(_) => Usage::default(),
    }
}

/// E4（launch_app 更新使用记录）使用。
#[allow(dead_code)]
pub(crate) fn save_usage(dir: &Path, u: &Usage) -> Result<(), EngineError> {
    atomic_write_json(dir, "usage.json", u)
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    apps: Vec<AppEntry>,
}

/// index.json 读失败 → None（视为首次运行）。
pub(crate) fn load_index(dir: &Path) -> Option<Vec<AppEntry>> {
    let text = fs::read_to_string(dir.join("index.json")).ok()?;
    let f: IndexFile = serde_json::from_str(&text).ok()?;
    Some(f.apps)
}

pub(crate) fn save_index(dir: &Path, apps: &[AppEntry]) -> Result<(), EngineError> {
    atomic_write_json(dir, "index.json", &IndexFile { version: 1, apps: apps.to_vec() })
}

/// 加载 Settings 后钳制：max_recent 1..=20；version 低于当前值时按迁移表升级（v0.1 无迁移）。
fn clamp(mut s: Settings) -> Settings {
    s.max_recent = s.max_recent.clamp(1, 20);
    if s.version < SETTINGS_VERSION {
        s.version = SETTINGS_VERSION;
    }
    s
}

fn atomic_write_json<T: Serialize>(dir: &Path, name: &str, value: &T) -> Result<(), EngineError> {
    let data = serde_json::to_string_pretty(value)
        .map_err(|e| EngineError::Io(format!("serialize {name}: {e}")))?;
    let tmp = dir.join(format!("{name}.tmp"));
    {
        let mut f = fs::File::create(&tmp)
            .map_err(|e| EngineError::Io(format!("create {name}.tmp: {e}")))?;
        f.write_all(data.as_bytes())
            .and_then(|()| f.flush())
            .map_err(|e| EngineError::Io(format!("write {name}.tmp: {e}")))?;
    }
    fs::rename(&tmp, dir.join(name)).map_err(|e| EngineError::Io(format!("rename {name}: {e}")))?;
    Ok(())
}
