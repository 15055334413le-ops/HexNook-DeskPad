//! 文件日志（engine 文档 4.9）。
//!
//! 文件：%LocalAppData%\DeskPad\log.txt，超过 1 MB 轮转为 log.old.txt。
//! 格式：`2026-10-05 01:57:03.123 [INFO] [tid] 消息`。写入失败静默忽略。
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentThreadId;

static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// 在 EngineHandle::init 第 1 步调用。未调用前 info/warn/error 静默丢弃。
pub(crate) fn init(dir: &Path) {
    let _ = DIR.set(dir.to_path_buf());
    let _ = FILE.set(Mutex::new(open_log(dir)));
}

pub fn info(msg: &str) {
    emit("INFO", msg);
}
pub fn warn(msg: &str) {
    emit("WARN", msg);
}
pub fn error(msg: &str) {
    emit("ERROR", msg);
}

fn open_log(dir: &Path) -> Option<File> {
    let path = dir.join("log.txt");
    if let Ok(md) = fs::metadata(&path) {
        if md.len() > 1_000_000 {
            // 超过 1 MB：改名为 log.old.txt（覆盖旧的）再新建。
            let _ = fs::rename(&path, dir.join("log.old.txt"));
        }
    }
    OpenOptions::new().create(true).append(true).open(&path).ok()
}

fn emit(level: &str, msg: &str) {
    let line = format!("{} [{level}] [{}] {}", timestamp(), thread_id(), msg);
    #[cfg(debug_assertions)]
    eprintln!("{line}");
    let Some(cell) = FILE.get() else { return };
    let Some(dir) = DIR.get() else { return };
    let mut g = match cell.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Ok(md) = fs::metadata(dir.join("log.txt")) {
        if md.len() > 1_000_000 {
            *g = open_log(dir);
        }
    }
    if let Some(f) = g.as_mut() {
        let _ = writeln!(f, "{line}");
    }
}

fn timestamp() -> String {
    // SAFETY: 无参查询，返回值即结果。
    let st = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}

fn thread_id() -> u32 {
    // SAFETY: 无参数，总是成功。
    unsafe { GetCurrentThreadId() }
}
