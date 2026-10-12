//! ShellExecuteExW 启动/打开（engine 文档 4.5）。
use windows::core::{w, PCWSTR};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::model::EngineError;
use crate::win;

/// 统一 ShellExecuteExW：SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI，verb="open"，SW_SHOWNORMAL。
/// 返回 FALSE 时 Err(LaunchFailed{name, code: GetLastError()}) 并记一条 warn。
pub(crate) fn execute(lp_file: &str, name: &str) -> Result<(), EngineError> {
    let wide = win::to_wide(lp_file);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(wide.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: 结构体与 wide 缓冲区都存活到调用结束；调用线程已 CoInitializeEx(STA)。
    if let Err(e) = unsafe { ShellExecuteExW(&mut sei) } {
        let code = win::win32_code(&e);
        crate::log::warn(&format!("ShellExecuteExW({lp_file}) failed: {e}"));
        return Err(EngineError::LaunchFailed { name: name.to_string(), code });
    }
    Ok(())
}
