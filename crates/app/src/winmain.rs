//! 隐藏主窗口：消息路由（UI 文档 6.2）。注意：必须是普通顶层窗口（收 TaskbarCreated 广播），从不显示。
use crate::app::{with_app, HideReason, ShowReason};
use crate::update_ui;
use windows::Win32::Foundation::*;
use windows::Win32::System::RemoteDesktop::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub unsafe extern "system" fn main_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    // 注册消息在 App 创建后已初始化；这里逐一比对
    let showpad = crate::app::msg_showpad();
    let taskbar = crate::app::msg_taskbar_created();
    let showpad_msg = showpad;
    let _showpad_msg = showpad_msg; // 保持原引用
    if msg >= 0x8000 {
        deskpad_engine::log::info(&format!("main_proc msg={:x} w={:x}", msg, wparam.0));
    }
    match msg {
        WM_HOTKEY => {
            // 5-#4：全屏/演示模式不呼出
            if crate::pad::fullscreen_ignores_hotkey() {
                return LRESULT(0);
            }
            with_app(|a| crate::pad::toggle(a, ShowReason::Hotkey));
            LRESULT(0)
        }
        m if m == crate::app::WM_APP_TRAY => {
            crate::tray::on_tray_event(hwnd, lparam);
            LRESULT(0)
        }
        m if m == crate::app::WM_APP_QUIT => {
            quit();
            LRESULT(0)
        }
        m if m == crate::app::WM_APP_SETTINGS => {
            let _ = crate::settings_ui::open(hwnd);
            LRESULT(0)
        }
        m if m == crate::app::WM_APP_SNAPSHOT => {
            with_app(crate::pad::on_snapshot);
            LRESULT(0)
        }
        m if m == crate::app::WM_APP_UPDATE => {
            // 检查更新的子进程写完 update.json：读回来刷新角标 / 设置窗口
            update_ui::on_checked();
            LRESULT(0)
        }
        m if m == showpad => {
            deskpad_engine::log::info("WM_APP_SHOWPAD received");
            // 5-#17：二次启动唤起已有实例的启动台
            // 已打开时切换关闭（重复双击 exe = 开/关）
            with_app(|a| crate::pad::toggle(a, ShowReason::SecondInstance));
            LRESULT(0)
        }
        m if m == taskbar => {
            // 5-#8：资源管理器重启后托盘图标恢复
            with_app(|a| {
                let _ = crate::tray::add(&a.factory, a.main, a.dpi.max(96));
            });
            LRESULT(0)
        }
        WM_WTSSESSION_CHANGE => {
            if wparam.0 as u32 == WTS_SESSION_LOCK {
                // 5-#6：锁屏时收起
                with_app(|a| crate::pad::hide(a, HideReason::System));
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC {
                with_app(|a| a.eng.refresh());
            }
            LRESULT(1)
        }
        WM_SETTINGCHANGE => {
            // 工作区变化（任务栏移动/自动隐藏）→ 角标钳回可见区
            if wparam.0 as u32 == SPI_SETWORKAREA.0 {
                crate::corner::reclamp();
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == crate::corner::BUBBLE_TIMER {
                crate::corner::on_bubble_timer();
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            with_app(|a| {
                crate::corner::reclamp_with(a);
                if a.pad_visible {
                    crate::pad::hide(a, HideReason::DisplayChange);
                }
            });
            LRESULT(0)
        }
        WM_QUERYENDSESSION => {
            // 5-#7：不拖慢关机
            LRESULT(1)
        }
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                quit();
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            quit();
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => {
            // SAFETY: 默认窗口过程。
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

/// 退出流程（UI 文档第 7 节，顺序不可调换）。
pub fn quit() {
    with_app(|a| {
        if a.quitting {
            return;
        }
        a.quitting = true;
        // SAFETY: 同线程销毁自建窗口；托盘/热键/WTS 依次注销。
        unsafe {
            if let Some(sw) = a.settings_wnd.take() {
                let _ = DestroyWindow(sw);
            }
            let _ = ShowWindow(a.bubble, SW_HIDE);
            let _ = DestroyWindow(a.corner);
            let _ = DestroyWindow(a.pad);
            let _ = DestroyWindow(a.scrim);
            crate::tray::remove(a.main);
            crate::hotkey::unregister(a.main);
            let _ = WTSUnRegisterSessionNotification(a.main);
        }
        a.eng.shutdown();
        // SAFETY: UI 线程上结束消息循环。
        unsafe { PostQuitMessage(0) };
    });
}
