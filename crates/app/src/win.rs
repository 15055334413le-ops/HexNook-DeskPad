//! Win32 RAII 包装与小工具（UI 文档 2/win.rs）。
use windows::core::{Error, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn module() -> windows::core::Result<HINSTANCE> {
    // SAFETY: GetModuleHandleW(None) 取当前 exe 模块句柄，无副作用。
    let hmod = unsafe { GetModuleHandleW(PCWSTR::null())? };
    Ok(HINSTANCE(hmod.0))
}

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 注册窗口类。重复注册（ERROR_CLASS_ALREADY_EXISTS）视为成功。
pub fn register_class(name: PCWSTR, proc_fn: WNDPROC, background: Option<HBRUSH>) -> windows::core::Result<()> {
    // SAFETY: 系统预定义光标，无需释放。没有类光标时鼠标会保留上一个窗口的形状（例如沙漏/调整大小）。
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
    let wc = WNDCLASSW {
        hCursor: cursor,
        lpfnWndProc: proc_fn,
        hInstance: module()?,
        lpszClassName: name,
        hbrBackground: background.unwrap_or_default(),
        ..Default::default()
    };
    // SAFETY: wc 的所有指针字段指向静态数据或当前栈，调用期间有效。
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        let e = Error::from_win32();
        // 0x00000584 = ERROR_CLASS_ALREADY_EXISTS：允许（比如重复建类）。
        if e.code() != windows::core::HRESULT(0x00000584u32 as i32) {
            return Err(e);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn create_window(
    class: PCWSTR,
    name: PCWSTR,
    style: WINDOW_STYLE,
    ex: WINDOW_EX_STYLE,
    parent: Option<HWND>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> windows::core::Result<HWND> {
    // SAFETY: 窗口类已注册；窗口由消息循环与销毁逻辑管理。
    unsafe { CreateWindowExW(ex, class, name, style, x, y, w, h, parent.unwrap_or_default(), None, module()?, None) }
}

/// STA COM 初始化守卫。
pub struct ComInit;
impl ComInit {
    pub fn sta() -> windows::core::Result<Self> {
        // SAFETY: HRESULT 表示初始化结果；S_FALSE（已初始化）也视为成功，Drop 时配对 CoUninitialize。
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(ComInit)
    }
}
impl Drop for ComInit {
    fn drop(&mut self) {
        // SAFETY: 与 sta() 中的 CoInitializeEx 严格配对。
        unsafe { CoUninitialize() };
    }
}

/// GetDC 的 RAII（窗口客户区 DC，ReleaseDC 归还）。
#[allow(dead_code)] // 规范 §2 要求的 HDC RAII 包装（预留）
pub struct WindowDc {
    pub hwnd: HWND,
    pub hdc: HDC,
}
impl WindowDc {
    #[allow(dead_code)]
    pub fn get(hwnd: HWND) -> windows::core::Result<Self> {
        // SAFETY: DC 由系统分配，Drop 时 ReleaseDC 归还。
        let hdc = unsafe { GetDC(hwnd) };
        if hdc.is_invalid() {
            return Err(Error::from_win32());
        }
        Ok(WindowDc { hwnd, hdc })
    }
}
impl Drop for WindowDc {
    fn drop(&mut self) {
        // SAFETY: hdc 由 get() 分配，配对释放。
        unsafe { let _ = ReleaseDC(self.hwnd, self.hdc); }
    }
}

/// 内存 DC 的 RAII（DeleteDC 销毁）。
pub struct MemDc {
    pub hdc: HDC,
}
impl MemDc {
    pub fn create() -> windows::core::Result<Self> {
        // SAFETY: 兼容 DC 由本结构持有，Drop 时 DeleteDC。
        let hdc = unsafe { CreateCompatibleDC(None) };
        if hdc.is_invalid() {
            return Err(Error::from_win32());
        }
        Ok(MemDc { hdc })
    }
}
impl Drop for MemDc {
    fn drop(&mut self) {
        // SAFETY: 与 create() 配对。
        unsafe { let _ = DeleteDC(self.hdc); }
    }
}

/// GDI 位图的 RAII（DeleteObject 销毁）。
pub struct HBitmapGuard(pub HBITMAP);
impl Drop for HBitmapGuard {
    fn drop(&mut self) {
        // SAFETY: 位图由 CreateDIBSection 等创建，Drop 时唯一所有者负责销毁。
        unsafe { let _ = DeleteObject(HGDIOBJ(self.0 .0)); }
    }
}

/// GDI 画刷的 RAII。
pub struct HBrushGuard(pub HBRUSH);
impl HBrushGuard {
    pub fn solid(color: COLORREF) -> Self {
        // SAFETY: 画刷句柄由本结构持有并负责销毁。
        HBrushGuard(unsafe { CreateSolidBrush(color) })
    }
}
impl Drop for HBrushGuard {
    fn drop(&mut self) {
        // SAFETY: 与 solid() 配对。
        unsafe { let _ = DeleteObject(HGDIOBJ(self.0 .0)); }
    }
}

/// HICON 的 RAII（DestroyIcon 销毁）。
#[allow(dead_code)] // 规范 §2 要求的 HICON RAII 包装（预留）
pub struct HIconGuard(pub HICON);
impl Drop for HIconGuard {
    fn drop(&mut self) {
        // SAFETY: 图标由 glyph::tray_icon 创建，Drop 时唯一所有者负责销毁。
        unsafe { let _ = DestroyIcon(self.0); }
    }
}

/// HMENU 的 RAII（DestroyMenu 销毁）。
pub struct HMenuGuard(pub HMENU);
impl Drop for HMenuGuard {
    fn drop(&mut self) {
        // SAFETY: 菜单由 CreatePopupMenu 创建，Drop 时唯一所有者负责销毁。
        unsafe { let _ = DestroyMenu(self.0); }
    }
}

pub fn get_x_lparam(l: LPARAM) -> i32 {
    ((l.0 as usize) & 0xFFFF) as u16 as i16 as i32
}
pub fn get_y_lparam(l: LPARAM) -> i32 {
    (((l.0 as usize) >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// 当前进程内存（MB）：(专用提交 PrivateUsage, 专用工作集 PrivateWorkingSet)。
/// 规范第 9 节的指标是任务管理器默认那一列「内存(专用工作集)」，对应第二个值。
pub fn mem_mb() -> (f64, f64) {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut pmc = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    // SAFETY: 结构体大小已填写，输出到栈变量。
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb) };
    if ok.as_bool() {
        (pmc.PrivateUsage as f64 / 1_048_576.0, pmc.PrivateWorkingSetSize as f64 / 1_048_576.0)
    } else {
        (-1.0, -1.0)
    }
}

/// 写一条带内存读数的日志：`mem[tag] commit=…MB private_ws=…MB`。
pub fn log_mem(tag: &str) {
    let (p, w) = mem_mb();
    deskpad_engine::log::info(&format!("mem[{tag}] commit={p:.1}MB private_ws={w:.1}MB"));
}
