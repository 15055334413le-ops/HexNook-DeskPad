//! Win32 RAII 包装与小工具（engine 文档 4.9）。
//!
//! 红线约束：业务代码里禁止散落裸 CloseHandle / DeleteObject，
//! 一律使用本文件的 RAII 包装。
use std::path::PathBuf;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
use windows::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{RegCloseKey, HKEY};
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};

use crate::model::EngineError;

/// 内核句柄 RAII：Drop 时 CloseHandle。
pub struct OwnedHandle(HANDLE);

impl OwnedHandle {
    /// 创建事件对象。manual_reset = false 是自动复位（refresh 用），true 是手动复位（shutdown 用）。
    pub fn event(manual_reset: bool) -> Result<Self, EngineError> {
        // SAFETY: 无名事件、参数固定；成功时返回有效句柄，失败经 map_err 报告。
        let h = unsafe { CreateEventW(None, manual_reset, false, PCWSTR::null()) }
            .map_err(|e| EngineError::Io(format!("CreateEventW: {e}")))?;
        Ok(Self(h))
    }

    pub fn get(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: 句柄由本结构独占创建，Drop 时恰好关闭一次。
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
// SAFETY: 内核句柄可跨线程等待/关闭，本包装独占句柄、无别名访问。
unsafe impl Send for OwnedHandle {}
// SAFETY: 同上。
unsafe impl Sync for OwnedHandle {}

/// SetEvent 的封装：不转移所有权，只触发事件。
pub fn set_event(handle: HANDLE) {
    use windows::Win32::System::Threading::SetEvent;
    // SAFETY: 调用方保证句柄有效且仍由其所有者持有，SetEvent 不转移所有权。
    unsafe {
        let _ = SetEvent(handle);
    }
}

/// FindFirstChangeNotificationW 句柄：Drop 时 FindCloseChangeNotification。
/// E5（变更监听）使用。
pub struct ChangeHandle(HANDLE);

impl ChangeHandle {
    pub fn new(path: &str, subtree: bool, filter: FILE_NOTIFY_CHANGE) -> Result<Self, EngineError> {
        let wide = to_wide(path);
        // SAFETY: wide 以 0 结尾且在本调用期间存活；成功句柄移交给 Self。
        let h = unsafe { FindFirstChangeNotificationW(PCWSTR(wide.as_ptr()), subtree, filter) }
            .map_err(|e| EngineError::Io(format!("FindFirstChangeNotificationW({path}): {e}")))?;
        Ok(Self(h))
    }

    pub fn get(&self) -> HANDLE {
        self.0
    }

    /// 信号到达、消费之后重新武装监听。
    pub fn rearm(&self) {
        // SAFETY: 句柄由本结构持有且有效。
        unsafe {
            let _ = FindNextChangeNotification(self.0);
        }
    }
}

impl Drop for ChangeHandle {
    fn drop(&mut self) {
        // SAFETY: 句柄由本结构独占创建，Drop 时恰好关闭一次。
        unsafe {
            let _ = FindCloseChangeNotification(self.0);
        }
    }
}
// SAFETY: 内核句柄可跨线程使用，本包装独占句柄、无别名访问。
unsafe impl Send for ChangeHandle {}
// SAFETY: 同上。
unsafe impl Sync for ChangeHandle {}

/// HBITMAP RAII：Drop 时 DeleteObject。
/// E3（图标）使用。
#[allow(dead_code)]
pub struct HBitmapGuard(pub HBITMAP);

impl Drop for HBitmapGuard {
    fn drop(&mut self) {
        // SAFETY: 位图由本结构独占创建（IShellItemImageFactory::GetImage），恰好删除一次。
        unsafe {
            let _ = DeleteObject(self.0);
        }
    }
}

/// 注册表键句柄 RAII：Drop 时 RegCloseKey。
pub struct HKeyGuard(pub HKEY);

impl Drop for HKeyGuard {
    fn drop(&mut self) {
        // SAFETY: 键句柄由本结构独占创建，恰好关闭一次。
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// shell 分配的 UTF-16 字符串（CoTaskMemFree 释放）：提供 to_string()。
pub struct CoTaskStr(PWSTR);

impl CoTaskStr {
    pub fn new(p: PWSTR) -> Self {
        Self(p)
    }

    // 命名遵循 engine 文档 4.9（CoTaskStr 提供 to_string()）。
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        if self.0.is_null() {
            return String::new();
        }
        // SAFETY: 指针指向系统分配的以 0 结尾 UTF-16 缓冲区，只在本对象存活期读取。
        unsafe { self.0.to_string().unwrap_or_default() }
    }
}

impl Drop for CoTaskStr {
    fn drop(&mut self) {
        // SAFETY: 指针由 shell/COM 分配（GetDisplayName 等），恰好释放一次。
        unsafe {
            CoTaskMemFree(Some(self.0.as_ptr().cast()));
        }
    }
}

/// COM STA 初始化：Drop 时 CoUninitialize（仅在初始化成功时配对）。
/// COM STA 初始化：Drop 时 CoUninitialize（仅在初始化成功时配对）。
pub struct ComInit {
    ok: bool,
}

impl ComInit {
    pub fn sta() -> Self {
        // SAFETY: 每线程恰好一次；hr.is_ok()（含 S_FALSE）时 Drop 配对 CoUninitialize。
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self { ok: hr.is_ok() }
    }
}

impl Drop for ComInit {
    fn drop(&mut self) {
        if self.ok {
            // SAFETY: 与 sta() 中成功的 CoInitializeEx 一一配对。
            unsafe {
                CoUninitialize();
            }
        }
    }
}

/// 已知文件夹路径；取不到返回 None。
pub fn known_folder(guid: &windows::core::GUID) -> Option<PathBuf> {
    // SAFETY: 返回的 PWSTR 由 CoTaskStr 托管，Drop 时 CoTaskMemFree。
    let pw = unsafe { SHGetKnownFolderPath(guid, KNOWN_FOLDER_FLAG(0), None) }.ok()?;
    Some(PathBuf::from(CoTaskStr::new(pw).to_string()))
}

/// 字符串转以 0 结尾的 UTF-16。
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// FILETIME（UTC 100ns）→ Unix 秒。
/// E4（recent 的 .lnk 修改时间）使用。
#[allow(dead_code)]
pub fn filetime_to_unix(ft: FILETIME) -> i64 {
    let t = (((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64) as i64;
    // FILETIME 是 1601-01-01 起的 100ns 计数；1601→1970 相差 11_644_473_600 秒
    t / 10_000_000 - 11_644_473_600
}

/// 当前 exe 完整路径（GetModuleFileNameW(None)）。
pub fn exe_path() -> String {
    let mut buf = [0u16; 1024];
    // SAFETY: buf 由本函数持有且长度固定；None 表示当前进程模块。
    let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

/// 从 windows crate 的 Error 还原 Win32 错误码（FACILITY_WIN32 时去掉 0x8007 前缀）。
/// 文件属性；不存在返回 None。
pub fn file_attributes(path: &str) -> Option<u32> {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    let wide = to_wide(path);
    // SAFETY: wide 以 0 结尾；INVALID_FILE_ATTRIBUTES 表示失败。
    let a = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if a == INVALID_FILE_ATTRIBUTES {
        None
    } else {
        Some(a)
    }
}

/// E4（launch 错误码）使用。
#[allow(dead_code)]
pub fn win32_code(e: &windows::core::Error) -> u32 {
    let h = e.code().0 as u32;
    if h & 0xFFFF_0000 == 0x8007_0000 {
        h & 0x0000_FFFF
    } else {
        h
    }
}

/// 等待单个句柄最多 ms 毫秒；返回 true 表示句柄已触发。
pub fn wait_one(h: HANDLE, ms: u32) -> bool {
    // SAFETY: 句柄由调用方保证有效；等待不转移所有权。
    unsafe { windows::Win32::System::Threading::WaitForSingleObject(h, ms) == windows::Win32::Foundation::WAIT_OBJECT_0 }
}

/// 专用工作集（MB），只用于日志核对内存预算。
pub fn private_ws_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut pmc = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    // SAFETY: 结构体大小已填写，输出到栈变量。
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb) };
    if ok.as_bool() { pmc.PrivateWorkingSetSize as f64 / 1_048_576.0 } else { -1.0 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn filetime_epoch() {
        use windows::Win32::Foundation::FILETIME;
        // 1970-01-01 00:00:00 UTC = 116444736000000000（100ns）
        let t: u64 = 116_444_736_000_000_000;
        let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
        assert_eq!(super::filetime_to_unix(ft), 0);
    }
}
