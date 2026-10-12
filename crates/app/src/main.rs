//! 引导（UI 文档 6.1，顺序不可调换）。
#![windows_subsystem = "windows"]

mod anim;
mod app;
mod corner;
mod fileicons;
mod fmt;
mod glyph;
mod hotkey;
mod icons;
mod input;
mod layout;
mod pad;
mod render;
mod settings_ui;
mod shot;
mod smooth;
mod surface;
mod theme;
mod tray;
mod win;
mod winmain;

use app::{set_app, with_app, App};
use deskpad_engine::{EngineHandle, EngineError};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::System::RemoteDesktop::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn fatal(msg: &str) {
    let text = win::to_wide(msg);
    // SAFETY: 字符串缓冲区在调用期间有效。
    unsafe {
        let _ = MessageBoxW(HWND::default(), PCWSTR(text.as_ptr()), w!("HN DeskPad"), MB_ICONERROR);
    }
}

fn main() {
    // 0. PDF 缩略图一次性子进程：渲染完就退出，不建窗口、不碰单实例锁（engine docthumb.rs）
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("--thumb-pdf") {
        std::process::exit(deskpad_engine::thumb_pdf_worker(&argv[2..]));
    }
    // 1. DPI 感知：必须在创建任何窗口之前
    // SAFETY: 进程级 DPI 声明，重复调用失败可忽略。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // 2. 5-#14：拒绝管理员
    // SAFETY: 查询当前进程令牌。
    if unsafe { IsUserAnAdmin() }.as_bool() {
        fatal("请不要以管理员身份运行 HN DeskPad。");
        return;
    }
    // 3. STA COM
    let com = match win::ComInit::sta() {
        Ok(c) => c,
        Err(_) => {
            fatal("COM 初始化失败。");
            return;
        }
    };
    // 开发自检：--render-test <out.bmp> [--query <q>]（不建可见窗口、不占单实例锁）
    let args: Vec<String> = std::env::args().collect();
    let arg_after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let render_test = arg_after("--render-test");
    // 4. 单实例；已有实例 → 唤起它的启动台后退出（5-#17）
    let guard = match if render_test.is_some() { Ok(None) } else { deskpad_engine::single::acquire().map(Some) } {
        Ok(g) => g,
        Err(EngineError::AlreadyRunning) => {
            // SAFETY: 常量参数。
            unsafe { let _ = AllowSetForegroundWindow(ASFW_ANY); }
            // SAFETY: 类名为静态字面量。
            if let Ok(hwnd) = unsafe { FindWindowW(w!("DeskPadMain"), PCWSTR::null()) } {
                // SAFETY: 窗口句柄有效。
                unsafe {
                    let _ = PostMessageW(hwnd, app::msg_showpad(), WPARAM(0), LPARAM(0));
                }
            }
            return;
        }
        Err(e) => {
            fatal(&format!("HN DeskPad 启动失败：{e:?}"));
            return;
        }
    };
    // 5. 引擎
    let eng = match EngineHandle::init() {
        Ok(e) => e,
        Err(e) => {
            deskpad_engine::log::error(&format!("init: {e:?}"));
            fatal("HN DeskPad 启动失败，详情见日志。");
            return;
        }
    };
    // 6. D2D / DWrite 工厂（进程内单份；WIC 在 RenderCtx 里按需创建）
    // SAFETY: 标准 COM 工厂创建，接口泛型与类型一致。
    let factory = match unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) } {
        Ok(f) => f,
        Err(e) => {
            fatal(&format!("图形初始化失败：{e}"));
            return;
        }
    };
    // SAFETY: 同上。
    let dwrite = match unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) } {
        Ok(f) => f,
        Err(e) => {
            fatal(&format!("文本初始化失败：{e}"));
            return;
        }
    };
    // 7. 隐藏主窗口（普通顶层窗口，从不显示——收 TaskbarCreated 广播）
    if win::register_class(w!("DeskPadMain"), Some(winmain::main_proc), None).is_err() {
        fatal("注册主窗口类失败。");
        return;
    }
    let main = match win::create_window(w!("DeskPadMain"), w!(""), WS_POPUP, WINDOW_EX_STYLE(0), None, 0, 0, 0, 0) {
        Ok(h) => h,
        Err(_) => {
            fatal("创建主窗口失败。");
            return;
        }
    };
    // 8. 全局状态
    set_app(App {
        eng: eng.clone(),
        main,
        scrim: HWND::default(),
        pad: HWND::default(),
        corner: HWND::default(),
        bubble: HWND::default(),
        settings_wnd: None,
        bubble_visible: false,
        factory: factory.clone(),
        pad_factory: None,
        pad_dwrite: None,
        dwrite: dwrite.clone(),
        pad_surface: surface::Surface::new(),
        pad_ctx: None,
        scrim_origin: (0, 0),
        ime_pos: (0, 0),
        snap: eng.snapshot(),
        input: input::State::new(),
        layout: layout::compute(layout::RectF { x: 0.0, y: 0.0, w: 1600.0, h: 860.0 }, 8, 0, (layout::ViewMode::Grid, layout::ViewMode::Grid)),
        dpi: 96,
        work: RECT::default(),
        prev_fg: None,
        pad_visible: false,
        pad_origin: (0, 0),
        pad_size: (0, 0),
        pad_alpha: 255,
        scrim_alpha: 0,
        caret_on: true,
        anim: None,
        corner_pos: None,
        error: None,
        quitting: false,
        tz_offset: 0,
        now_local: fmt::LocalNow { year: 2026, month: 1, day: 1, hour: 0, minute: 0 },
    });
    if let Some(out) = render_test {
        let q = arg_after("--query").unwrap_or_default();
        let ok = shot::run(&out, &q);
        eng.shutdown();
        std::process::exit(if ok { 0 } else { 1 });
    }
    // 9. 引擎回调 → PostMessage 回主线程
    deskpad_engine::log::info(&format!("showpad_msg={:x} taskbar_msg={:x}", app::msg_showpad(), app::msg_taskbar_created()));
    let hwnd_raw = main.0 as isize;
    eng.set_on_change(Box::new(move || app::post_snapshot(hwnd_raw)));
    // 10. 托盘 → 热键 → 启动台窗口 → 角标 → WTS
    let settings = eng.settings();
    let sys_dpi = unsafe { GetDpiForSystem() };
    if tray::add(&factory, main, sys_dpi).is_err() {
        deskpad_engine::log::warn("tray add failed");
    }
    if !hotkey::register(main, settings.hotkey) {
        tray::notify(
            main,
            &format!(
                "{} 已被其他程序占用。可以点击桌面左下角的图标，或在设置中换一个快捷键。",
                settings.hotkey.label()
            ),
        );
    }
    if pad::create_windows().is_err() {
        fatal("创建启动台窗口失败。");
        return;
    }
    if let Err(e) = corner::create() {
        deskpad_engine::log::warn(&format!("corner create failed: {e}"));
    }
    // SAFETY: 主窗口句柄有效。
    unsafe { let _ = WTSRegisterSessionNotification(main, NOTIFY_FOR_THIS_SESSION); }
    // 11. 首次引导（--autostart 时跳过，5-#15）
    let autostart = std::env::args().any(|a| a == "--autostart");
    if !autostart && !settings.first_run_done {
        with_app(corner::show_bubble);
    }
    crate::win::log_mem("app:idle-start");
    // 12. 消息循环
    let mut msg = MSG::default();
    // SAFETY: 标准消息循环；TranslateMessage 生成 WM_CHAR（IME 上屏依赖）。
    unsafe {
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    eng.shutdown();
    drop(guard);
    drop(com);
}
