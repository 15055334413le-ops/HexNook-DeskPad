//! 即时生效：每个控件改动立刻写设置（UI 文档 6.11 / 任务卡第 2 条）。
//!
//! 热键用 `hotkey::change` 先切再存，注册失败就地报错并回退；其余设置直接写。
//! 所有 with_app 调用都发生在 window 模块的借用之外（由调用方保证）。
use super::model::{self, ItemId};
use crate::app::with_app;
use deskpad_engine::model::{HotkeyPreset, IconStyle, PadSize, Settings, SortMode, VisualStyle};
use deskpad_engine::{EngineError, Snapshot};

/// 读当前设置 + 快照（列表行用）。
pub fn read() -> Option<(Settings, Snapshot)> {
    with_app(|a| (a.eng.settings(), a.eng.snapshot()))
}

/// 一次控件改动：返回 Err(行内错误文案) 时窗口显示错误并回退状态。
pub fn apply(id: ItemId, seg_idx: usize, delta: i32) -> Result<(), String> {
    match id {
        ItemId::Hotkey => set_hotkey(seg_idx),
        ItemId::Autostart => write_settings(|s| s.autostart = !s.autostart),
        ItemId::VisualStyle => write_settings(|s| {
            s.visual_style = if seg_idx == 0 { VisualStyle::Flat } else { VisualStyle::Glass };
        }),
        ItemId::PadSize => write_settings(|s| {
            s.pad_size = if seg_idx == 0 { PadSize::Window } else { PadSize::Fullscreen };
        }),
        ItemId::IconStyle => write_settings(|s| {
            s.icon_style = if seg_idx == 0 { IconStyle::Native } else { IconStyle::Unified };
        }),
        ItemId::FilesView => write_settings(|s| s.files_view = model::idx_view_mode(seg_idx)),
        ItemId::DirsView => write_settings(|s| s.dirs_view = model::idx_view_mode(seg_idx)),
        ItemId::ShowSystem => write_settings(|s| s.show_system_tools = !s.show_system_tools),
        ItemId::SortMode => write_settings(|s| {
            s.sort_mode = if seg_idx == 0 { SortMode::Name } else { SortMode::Frequency };
        }),
        ItemId::RecentCount => write_settings(|s| {
            let n = s.max_recent as i32 + delta;
            s.max_recent = n.clamp(1, 20) as u32;
        }),
        ItemId::AutoUpdate => write_settings(|s| s.auto_check_update = !s.auto_check_update),
        // 按钮类与文字行不在这里改设置。
        ItemId::ReplayOnboard => {
            write_settings(|s| s.first_run_done = false)?;
            with_app(crate::corner::show_bubble);
            Ok(())
        }
        ItemId::CheckUpdate
        | ItemId::Version
        | ItemId::Releases
        | ItemId::OpenLogs
        | ItemId::AddScanDir
        | ItemId::AddManualApp => Ok(()),
    }
}

fn set_hotkey(idx: usize) -> Result<(), String> {
    let new = HotkeyPreset::ALL[idx.min(HotkeyPreset::ALL.len() - 1)];
    let r = with_app(|a| {
        let mut s = a.eng.settings();
        if s.hotkey == new {
            return Ok(());
        }
        let old = s.hotkey;
        if !crate::hotkey::change(a.main, old, new) {
            return Err(format!("{} 已被其他程序占用，请换一个。", new.label()));
        }
        s.hotkey = new;
        if let Err(e) = a.eng.save_settings(&s) {
            // 存盘失败：热键已经换了，尽量换回去
            let _ = crate::hotkey::change(a.main, new, old);
            deskpad_engine::log::warn(&format!("settings save: {e:?}"));
            return Err("写入设置失败，详情见日志".into());
        }
        Ok(())
    });
    r.unwrap_or(Ok(()))
}

fn write_settings(f: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let r = with_app(|a| {
        let mut s = a.eng.settings();
        f(&mut s);
        a.eng.save_settings(&s).map_err(|e| {
            deskpad_engine::log::warn(&format!("settings save: {e:?}"));
            "写入设置失败，详情见日志".to_string()
        })
    });
    r.unwrap_or(Ok(()))
}

// ---- 列表动作（隐藏应用 / 扫描目录 / 手动应用）----

pub fn unhide(setting_id: &str) -> Result<(), String> {
    engine_call(|e| e.set_hidden(setting_id, false))
}
pub fn remove_scan_dir(path: &std::path::Path) -> Result<(), String> {
    engine_call(|e| e.remove_scan_dir(path))
}
pub fn add_scan_dir(path: &std::path::Path) -> Result<(), String> {
    engine_call(|e| e.add_scan_dir(path))
}
pub fn add_manual_app(path: &std::path::Path) -> Result<(), String> {
    engine_call(|e| e.add_manual_app(path))
}
pub fn remove_manual_app(path: &std::path::Path) -> Result<(), String> {
    engine_call(|e| e.remove_manual_app(path))
}

fn engine_call(f: impl FnOnce(&deskpad_engine::EngineHandle) -> Result<(), EngineError>) -> Result<(), String> {
    let r = with_app(|a| {
        f(&a.eng).map_err(|e| {
            deskpad_engine::log::warn(&format!("settings list op: {e:?}"));
            crate::fmt::error_text(&e)
        })
    });
    r.unwrap_or_else(|| Err("出错了，详情见日志".into()))
}

/// 当前设置里开关 / 分段控件的选中态（重绘用）。
pub fn toggle_on(s: &Settings, id: ItemId) -> bool {
    match id {
        ItemId::Autostart => s.autostart,
        ItemId::ShowSystem => s.show_system_tools,
        ItemId::AutoUpdate => s.auto_check_update,
        _ => false,
    }
}

pub fn seg_selected(s: &Settings, id: ItemId) -> usize {
    match id {
        ItemId::Hotkey => model::hotkey_idx(s),
        ItemId::VisualStyle => model::visual_style_idx(s),
        ItemId::PadSize => model::pad_size_idx(s),
        ItemId::IconStyle => model::icon_style_idx(s),
        ItemId::FilesView => model::view_mode_idx(s.files_view),
        ItemId::DirsView => model::view_mode_idx(s.dirs_view),
        ItemId::SortMode => model::sort_mode_idx(s),
        _ => 0,
    }
}
