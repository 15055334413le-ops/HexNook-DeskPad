//! 单实例互斥（engine 文档 4.7 single 部分）。
use crate::model::EngineError;
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

pub struct InstanceGuard {
    handle: HANDLE,
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: 句柄由 acquire 创建并只在本结构体持有，Drop 时唯一关闭。
        unsafe { let _ = CloseHandle(self.handle); }
    }
}

/// 返回的 guard 必须一直持有到进程结束。已有实例 → Err(AlreadyRunning)。
pub fn acquire() -> Result<InstanceGuard, EngineError> {
    // SAFETY: 互斥体句柄由本函数创建，成功时移交给 InstanceGuard 持有。
    let handle = unsafe {
        CreateMutexW(None, false, w!("Local\\HexNook.DeskPad.SingleInstance"))
            .map_err(|e| EngineError::Io(format!("CreateMutexW: {e}")))?
    };
    // SAFETY: GetLastError 紧跟在 CreateMutexW 之后调用，取到的是该调用的错误码。
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: 提前退出路径，句柄尚未移交给任何所有者，在此关闭。
        unsafe { let _ = CloseHandle(handle); }
        return Err(EngineError::AlreadyRunning);
    }
    Ok(InstanceGuard { handle })
}
