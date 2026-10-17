//! 托盘（UI 文档 6.4）：NOTIFYICON_VERSION_4，原生菜单，气泡通知。
use crate::app::{with_app, WM_APP_TRAY};
use crate::glyph;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1Factory;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

const ID_TRAY: u32 = 1;
// 菜单项 id
const CMD_OPEN: u32 = 1;
const CMD_SETTINGS: u32 = 2;
const CMD_AUTOSTART: u32 = 3;
const CMD_EXIT: u32 = 4;
/// 「更新到 x.y.z」：只在发现新版本时追加（动态 id，不与固定项冲突）。
const CMD_UPDATE: u32 = 5;

pub fn add(factory: &ID2D1Factory, hwnd: HWND, dpi: u32) -> Result<()> {
    let icon = glyph::tray_icon(factory, dpi)?;
    // SAFETY: hwnd/icon 有效；托盘图标在 remove/TaskbarCreated 时管理。
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ID_TRAY,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: icon,
        szTip: utf16_arr("HN DeskPad"),
        ..Default::default()
    };
    // SAFETY: nid 参数完整有效。
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    if !added.as_bool() {
        return Err(windows::core::Error::from_win32());
    }
    nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    // SAFETY: 同上。
    let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &nid) };
    // 图标句柄已被系统复制，销毁临时 HICON。
    // SAFETY: icon 由 glyph::tray_icon 创建，系统已复制，这里销毁原始句柄。
    unsafe { let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon); }
    Ok(())
}

pub fn remove(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ID_TRAY,
        ..Default::default()
    };
    // SAFETY: 与 add 配对；窗口仍有效（退出流程中在销毁窗口前调用）。
    unsafe { let _ = Shell_NotifyIconW(NIM_DELETE, &nid); }
}

/// 系统通知（只用于热键冲突、启动失败两种场景；无声音）。
pub fn notify(hwnd: HWND, text: &str) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ID_TRAY,
        uFlags: NIF_INFO,
        szInfo: utf16_arr_max::<256>(text),
        szInfoTitle: utf16_arr_max::<64>("HN DeskPad"),
        dwInfoFlags: NIIF_INFO | NIIF_NOSOUND,
        ..Default::default()
    };
    // SAFETY: nid 参数完整有效。
    unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &nid); }
}

fn utf16_arr(s: &str) -> [u16; 128] {
    let mut a = [0u16; 128];
    for (i, c) in s.encode_utf16().take(127).enumerate() {
        a[i] = c;
    }
    a
}

fn utf16_arr_max<const N: usize>(s: &str) -> [u16; N] {
    let mut a = [0u16; N];
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        a[i] = c;
    }
    a
}

/// 托盘事件（WM_APP_TRAY 的 lParam，VERSION_4：LOWORD = 事件）。
pub fn on_tray_event(hwnd: HWND, lparam: LPARAM) {
    use windows::Win32::UI::WindowsAndMessaging::{WM_CONTEXTMENU, WM_LBUTTONUP};
    let ev = (lparam.0 as usize & 0xFFFF) as u16;
    match ev as u32 {
        WM_LBUTTONUP | NIN_SELECT => {
            with_app(|a| crate::pad::toggle(a, crate::app::ShowReason::Tray));
        }
        WM_CONTEXTMENU => {
            let x = crate::win::get_x_lparam(lparam);
            let y = crate::win::get_y_lparam(lparam);
            popup_menu(hwnd, POINT { x, y });
        }
        _ => {}
    }
}

fn popup_menu(hwnd: HWND, pt: POINT) {
    let (autostart, new_ver) = with_app(|a| {
        let v = match &a.update {
            crate::update_ui::UpdateState::New(v) => Some(v.clone()),
            _ => None,
        };
        (a.eng.settings().autostart, v)
    })
    .unwrap_or((false, None));
    // SAFETY: 菜单句柄由 HMenuGuard 管理。
    let menu = unsafe { CreatePopupMenu() }.unwrap_or_default();
    if menu.is_invalid() {
        return;
    }
    let _guard = crate::win::HMenuGuard(menu);
    // SAFETY: 菜单句柄有效，文本均为宽字符字面量。
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN as usize, w!("打开启动台"));
        let _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS as usize, w!("设置…"));
        // 发现新版本时在「设置…」下面加一项「更新到 x.y.z」（#35；气泡仍只有热键冲突/启动失败两种）
        if let Some(v) = &new_ver {
            let label = crate::win::to_wide(&format!("更新到 {v}"));
            let _ = AppendMenuW(menu, MF_STRING, CMD_UPDATE as usize, windows::core::PCWSTR(label.as_ptr()));
        }
        let flag = MF_STRING | if autostart { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, flag, CMD_AUTOSTART as usize, w!("开机自启"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT as usize, w!("退出"));
    }
    // SAFETY: 托盘菜单弹出的已知写法：先 SetForegroundWindow，TPM_RETURNCMD 取回选项，再投 WM_NULL。
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            (TPM_RIGHTBUTTON | TPM_RETURNCMD).0,
            pt.x,
            pt.y,
            hwnd,
            None,
        );
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        if cmd.as_bool() {
        match cmd.0 as u32 {
            CMD_OPEN => { with_app(|a| crate::pad::toggle(a, crate::app::ShowReason::Tray)); }
            CMD_SETTINGS => { let _ = crate::settings_ui::open(hwnd); }
            CMD_UPDATE => { crate::update_ui::apply_update(); }
            CMD_AUTOSTART => {
                crate::app::with_app(|a| {
                    let mut s = a.eng.settings();
                    s.autostart = !s.autostart;
                    if let Err(e) = a.eng.save_settings(&s) {
                        deskpad_engine::log::warn(&format!("save autostart: {e:?}"));
                    }
                });
            }
            CMD_EXIT => { crate::winmain::quit(); }
            _ => {}
        }
        }
    }
}

use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, PostMessageW, TrackPopupMenuEx};
use windows::Win32::UI::WindowsAndMessaging::{
    MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
};
