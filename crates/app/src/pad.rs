//! 启动台：遮罩窗口 + 内容窗口的生命周期、显示与隐藏（UI 文档 6.5/6.8）。
use crate::anim;
use crate::app::{with_app, App, WM_MOUSELEAVE, HideReason, ShowReason};
use crate::fmt;
use crate::input::{self, Action};
use crate::layout::{Hit, RectF};
use crate::render::{draw_pad, RenderCtx};
use crate::theme::SHADOW_MARGIN;
use crate::win;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTimeAsFileTime};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::Ime::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::System::Threading::AttachThreadInput;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ANIM_TIMER: usize = 1;
pub const CARET_TIMER: usize = 2;
/// 切换「最近文件 / 常用文件夹」的列表或图标视图：保存设置并按新视图重排。
fn set_view(app: &mut App, zone: crate::layout::FocusZone, mode: crate::layout::ViewMode) {
    use crate::layout::FocusZone;
    let mut s = app.eng.settings();
    match zone {
        FocusZone::Files if s.files_view != mode => s.files_view = mode,
        FocusZone::Dirs if s.dirs_view != mode => s.dirs_view = mode,
        _ => return,
    }
    if let Err(e) = app.eng.save_settings(&s) {
        deskpad_engine::log::warn(&format!("save view mode: {e:?}"));
    }
    app.layout = crate::layout::compute(app.layout.work, s.max_recent as usize, app.snap.dirs.len(), (s.files_view, s.dirs_view));
    if app.input.focus == zone {
        app.input.selected = Some(0);
    }
    app.input.hover = None;
}

pub const TIMER_MS: u32 = 15;

/// 全屏/演示模式时热键不呼出（5-#4）。
pub fn fullscreen_ignores_hotkey() -> bool {
    // SAFETY: 查询系统通知状态，输出为返回值。
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) => matches!(s, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE),
        Err(_) => false,
    }
}

pub fn create_windows() -> Result<()> {
    // 类背景刷必须与类同寿命，故意泄漏（scrim 每次 WM_ERASEBKGND 都会用它）。
    // 之前画刷创建了却没传给窗口类，背景幕整窗是空的——这就是截图里「没有深色遮罩」的原因。
    let brush = win::HBrushGuard::solid(COLORREF(0x001A120F)); // RGB(15,18,26)
    let hbr = brush.0;
    std::mem::forget(brush);
    win::register_class(w!("DeskPadScrim"), Some(scrim_proc), Some(hbr))?;
    win::register_class(w!("DeskPadPad"), Some(pad_proc), None)?;
    let main = with_app(|a| a.main).unwrap_or_default();
    let ex_scrim = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
    let scrim = win::create_window(
        w!("DeskPadScrim"), w!(""), WS_POPUP, ex_scrim, None, 0, 0, 0, 0)?;
    let pad = win::create_window(
        w!("DeskPadPad"), w!(""), WS_POPUP,
        WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        Some(scrim), 0, 0, 0, 0)?;
    with_app(|a| {
        a.scrim = scrim;
        a.pad = pad;
    });
    let _ = main;
    Ok(())
}

/// 每次显示启动台时调用一次（UI 文档 6.5 show）。
pub fn show(app: &mut App, reason: ShowReason) {
    deskpad_engine::log::info("pad::show enter");
    if app.pad_visible {
        deskpad_engine::log::info("pad::show already visible, return");
        return;
    }
    // 先置状态再动窗口：否则显示序列途中到达的 WM_ACTIVATE 会触发
    // hide() 的 early-return，把全屏遮罩永久留在屏幕上吃掉所有鼠标输入
    app.pad_visible = true;
    let _ = reason;
    // 5-#2：记下呼出前的前台窗口
    // SAFETY: 查询前台窗口；失败时无前台可还。
    let fg = unsafe { GetForegroundWindow() };
    app.prev_fg = if fg.is_invalid() { None } else { Some(fg.0 as isize) };
    // 5-#1：鼠标所在显示器
    let mut pt = POINT::default();
    // SAFETY: 输出参数。
    unsafe { let _ = GetCursorPos(&mut pt); }
    // SAFETY: point → monitor。
    let mon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    app.snap = app.eng.snapshot();
    app.eng.refresh();
    app.input.reset();
    app.input.hits = app.eng.search(&app.snap, "");
    if !prepare_frame(app, mon) {
        // 渲染失败：直接以无动画方式展示空面板（记日志）
        deskpad_engine::log::warn("render first frame failed");
    }
    let work = app.work;
    // 背景幕：铺满工作区（纯色 + 整窗透明度，零位图）
    // SAFETY: 窗口句柄有效。
    unsafe {
        let _ = SetWindowPos(
            app.scrim,
            HWND_TOPMOST,
            work.left, work.top, work.right - work.left, work.bottom - work.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
    set_scrim_alpha(app, app.scrim_alpha);
    // 内容窗口显示并取得焦点
    // SAFETY: 窗口句柄有效。
    unsafe {
        let _ = ShowWindow(app.pad, SW_SHOW);
        // 前台锁：后台进程直接 SetForegroundWindow 会被拒绝，
        // 按微软文档方案先 AttachThreadInput 到前台线程再设置。
        let fg = GetForegroundWindow();
        let fg_tid = if fg.is_invalid() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let cur_tid = GetCurrentThreadId();
        let _ = AttachThreadInput(cur_tid, fg_tid, true);
        let _ = SetForegroundWindow(app.pad);
        let _ = AttachThreadInput(cur_tid, fg_tid, false);
    }
    // 定时器
    // SAFETY: 定时器挂在内容窗口上，hide 时统一 Kill。
    unsafe {
        let _ = SetTimer(app.pad, ANIM_TIMER, TIMER_MS, None);
        let blink = GetCaretBlinkTime().max(100);
        let _ = SetTimer(app.pad, CARET_TIMER, blink, None);
    }
    app.pad_visible = true;
    deskpad_engine::log::info("pad::show complete (visible)");
    crate::win::log_mem("pad:shown");
    crate::anim::start_open(app);
    if app.anim.is_none() {
        // 系统动画关闭：直接到终态
        app.pad_alpha = 255;
        app.scrim_alpha = 77;
        let _ = app.pad_surface.set_alpha_and_pos(
            app.pad,
            POINT { x: app.pad_origin.0, y: app.pad_origin.1 },
            255,
        );
        set_scrim_alpha(app, 77);
    }
    // 5-#15/6.10：启动台被打开时，引导气泡消失
    crate::corner::dismiss_bubble(app, true);
}


/// 按显示器 mon 计算布局、建表面并渲染第一帧（alpha 0）。show() 与调试截图共用。
/// 调用前 app.snap / app.input 已就绪。返回 false 表示渲染失败。
pub fn prepare_frame(app: &mut App, mon: windows::Win32::Graphics::Gdi::HMONITOR) -> bool {
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: mi 为输出参数。
    unsafe { let _ = GetMonitorInfoW(mon, &mut mi); }
    let work = mi.rcWork;
    // SAFETY: 显示器句柄有效。
    let (mut dx, mut dy) = (0u32, 0u32);
    // SAFETY: 输出参数由本函数持有。
    unsafe { let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy); };
    let dpi = if dx == 0 { 96u32 } else { dx };
    app.dpi = dpi;
    app.work = work;
    app.now_local = local_now();
    app.tz_offset = tz_offset_sec();
    let s = app.eng.settings();
    let scale = app.scale();
    let work_dip = RectF {
        x: work.left as f32 / scale,
        y: work.top as f32 / scale,
        w: (work.right - work.left) as f32 / scale,
        h: (work.bottom - work.top) as f32 / scale,
    };
    app.layout = crate::layout::compute(work_dip, s.max_recent as usize, app.snap.dirs.len(), (s.files_view, s.dirs_view));
    // 内容窗口物理矩形（面板外接矩形向外扩 SHADOW_MARGIN）
    let ox = ((app.layout.content.x - SHADOW_MARGIN) * scale).round() as i32;
    let oy = ((app.layout.content.y - SHADOW_MARGIN) * scale).round() as i32;
    let pw = ((app.layout.content.w + 2.0 * SHADOW_MARGIN) * scale).round() as i32;
    let ph = ((app.layout.content.h + 2.0 * SHADOW_MARGIN) * scale).round() as i32;
    app.pad_origin = (ox, oy);
    app.pad_size = (pw, ph);
    deskpad_engine::log::info(&format!(
        "layout: dpi={dpi} scale={scale:.2} work_w={ww:.0} content=({cx:.0},{cy:.0} {cw:.0}x{ch:.0}) apps_w={aw:.0} files=({fx:.0},{fy:.0} {fw:.0}x{fh:.0}) dirs_h={dh:.0} cell_w={cwid:.0} pw={pw} ph={ph}",
        dpi = dpi, scale = scale, ww = work_dip.w,
        cx = app.layout.content.x, cy = app.layout.content.y,
        cw = app.layout.content.w, ch = app.layout.content.h,
        aw = app.layout.apps_card.w,
        fx = app.layout.files_card.x, fy = app.layout.files_card.y,
        fw = app.layout.files_card.w, fh = app.layout.files_card.h,
        dh = app.layout.dirs_card.h, cwid = app.layout.cell_w,
        pw = pw, ph = ph
    ));
    if let Some(ctx) = app.pad_ctx.as_mut() {
        ctx.icons.prune(&app.snap);
    }
    // 表面 + 渲染上下文 + 首帧（alpha 0，动画从 0 淡入）
    let ok = (|| -> Result<()> {
        let pf = pad_factory(app);
        app.pad_surface.ensure(&pf, pw, ph, dpi as f32)?;
        if app.pad_ctx.is_none() {
            let rt = app.pad_surface.rt().cloned();
            if let Some(rt) = rt {
                app.pad_ctx = RenderCtx::new(&pad_factory(app), &pad_dwrite(app), rt).ok();
            }
        }
        if let Some(mut ctx) = app.pad_ctx.take() {
            let factory = pad_factory(app);
            let r = draw_pad(app, &mut ctx, &factory);
            app.pad_ctx = Some(ctx);
            r?;
        }
        Ok(())
    })()
    .is_ok();
    app.pad_alpha = 0;
    app.scrim_alpha = 0;
    if ok {
        let _ = app
            .pad_surface
            .set_alpha_and_pos(app.pad, POINT { x: ox, y: oy }, 0);
    }
    ok
}

/// 整帧重绘并推屏。
pub fn render_frame(app: &mut App) -> Result<()> {
    let pf = pad_factory(app);
    app.pad_surface.ensure(&pf, app.pad_size.0, app.pad_size.1, app.dpi as f32)?;
    if let Some(mut ctx) = app.pad_ctx.take() {
        let factory = pad_factory(app);
        let r = draw_pad(app, &mut ctx, &factory);
        app.pad_ctx = Some(ctx);
        r?;
        app.pad_surface.present(
            app.pad,
            POINT { x: app.pad_origin.0, y: app.pad_origin.1 },
            app.pad_alpha,
        )?;
    }
    Ok(())
}

/// 背景幕整窗透明度（背景幕只用 SetLayeredWindowAttributes，零位图）。
pub fn set_scrim_alpha(app: &App, alpha: u8) {
    // SAFETY: 窗口句柄有效；LWA_ALPHA 语义。
    unsafe {
        let _ = SetLayeredWindowAttributes(app.scrim, COLORREF(0), alpha, LWA_ALPHA);
    }
}

fn on_timer(app: &mut App, id: usize) {
    match id {
        anim::TIMER_ID => {
            if anim::tick(app) {
                // SAFETY: 配对删除。
                unsafe { let _ = KillTimer(app.pad, anim::TIMER_ID); }
            }
        }
        CARET_TIMER => {
            app.caret_on = !app.caret_on;
            let _ = render_frame(app);
        }
        _ => {}
    }
}

fn research(app: &mut App) {
    app.input.hits = app.eng.search(&app.snap, &app.input.query);
}

fn exec_action(app: &mut App, act: Action) -> bool {
    match act {
        Action::None => false,
        Action::ClearQuery => { research(app); true }
        Action::Hide(r) => { hide(app, r); false }
        Action::Launch(idx) => {
            // hits 存的是 snap.apps 的下标，启动要用 AppEntry::id
            let Some(id) = app.input.hits.get(idx).and_then(|&i| app.snap.apps.get(i)).map(|e| e.id) else { return false };
            let q = app.input.query.clone();
            match app.eng.launch_app(id, &q) {
                Ok(()) => { hide(app, HideReason::Launch); false }
                Err(e) => { app.error = Some((fmt::error_text(&e), std::time::Instant::now())); true }
            }
        }
        Action::Open(p) => match app.eng.open_path(&p) {
            Ok(()) => { hide(app, HideReason::Launch); false }
            Err(e) => { app.error = Some((fmt::error_text(&e), std::time::Instant::now())); true }
        },
    }
}

fn on_keydown(app: &mut App, vk: u32, _lparam: LPARAM) -> bool {
    // SAFETY: 查询修饰键状态。
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    // SAFETY: 同上。
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    const VK_V: u32 = 0x56;
    if ctrl && vk == VK_V {
        if let Some(text) = clipboard_text(app.pad) {
            input::paste(&mut app.input, &text);
            research(app);
            return true;
        }
        return false;
    }
    let act = input::key(&mut app.input, vk, ctrl, shift, &app.layout, &app.snap);
    match act {
        Action::None => {
            input::ensure_visible(&mut app.input, &app.layout, app.snap.apps.len());
            true
        }
        other => exec_action(app, other),
    }
}

fn on_char(app: &mut App, wparam: WPARAM) -> bool {
    let Some(c) = char::from_u32(wparam.0 as u32) else { return false };
    if c.is_control() { return false; }
    app.input.push_char(c);
    research(app);
    true
}

fn clipboard_text(pad: HWND) -> Option<String> {
    // SAFETY: 剪贴板打开/关闭配对；内存句柄由剪贴板拥有，GlobalLock/Unlock 配对。
    unsafe {
        OpenClipboard(pad).ok()?;
        let h = GetClipboardData(CF_UNICODETEXT.0 as u32).ok();
        let out = h.and_then(|h| {
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(h.0)) as *const u16;
            if ptr.is_null() { return None; }
            let mut len = 0usize;
            while *ptr.add(len) != 0 { len += 1; }
            let slice = std::slice::from_raw_parts(ptr, len);
            let s = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(h.0));
            Some(s)
        });
        let _ = CloseClipboard();
        out
    }
}

fn update_ime(app: &App) {
    // SAFETY: IME 上下文按窗口获取，末尾配对释放。
    unsafe {
        let himc = ImmGetContext(app.pad);
        if himc.is_invalid() {
            return;
        }
        let cf = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x: app.ime_pos.0, y: app.ime_pos.1 },
            ..Default::default()
        };
        let _ = ImmSetCompositionWindow(himc, &cf);
        let cand = CANDIDATEFORM {
            dwIndex: 0,
            dwStyle: CFS_CANDIDATEPOS,
            ptCurrentPos: POINT { x: app.ime_pos.0, y: app.ime_pos.1 + 4 },
            rcArea: RECT::default(),
        };
        let _ = ImmSetCandidateWindow(himc, &cand);
        let _ = ImmReleaseContext(app.pad, himc);
    }
}

pub unsafe extern "system" fn scrim_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(3), // MA_NOACTIVATE
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            with_app(|a| hide(a, HideReason::ClickOutside));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub unsafe extern "system" fn pad_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_app(|a| on_timer(a, wparam.0));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let redraw = with_app(|a| on_keydown(a, wparam.0 as u32, lparam)).unwrap_or(false);
            if redraw {
                with_app(|a| { let _ = crate::pad::render_frame(a); a.ime_pos });
                with_app(|a| update_ime(a));
            }
            LRESULT(0)
        }
        WM_CHAR => {
            let redraw = with_app(|a| on_char(a, wparam)).unwrap_or(false);
            if redraw {
                with_app(|a| { let _ = crate::pad::render_frame(a); });
                with_app(|a| update_ime(a));
            }
            LRESULT(0)
        }
        WM_IME_STARTCOMPOSITION => {
            with_app(|a| update_ime(a));
            // SAFETY: 交给默认处理以继续 IME 流程。
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let x = win::get_x_lparam(lparam);
            let y = win::get_y_lparam(lparam);
            let changed = with_app(|a| {
                let pt = client_to_layout(a, x, y);
                let hit = a.layout.hit_test(pt, a.input.scroll_row, a.input.hits.len(), a.snap.recent.len(), a.snap.dirs.len());
                if a.input.hover != Some(hit) {
                    a.input.hover = Some(hit);
                    true
                } else { false }
            }).unwrap_or(false);
            if changed {
                with_app(|a| { let _ = render_frame(a); });
            }
            // 首次移动时注册 WM_MOUSELEAVE
            track_mouse_leave(hwnd);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(|a| {
                if a.input.hover.is_some() {
                    a.input.hover = None;
                    let _ = render_frame(a);
                }
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as u16 as i16 as i32;
            with_app(|a| {
                let n_apps = a.input.hits.len();
                input::scroll(&mut a.input, &a.layout, -delta / 120, n_apps); // 滚轮向上 delta>0 → 往上滚
                let _ = render_frame(a);
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_app(|a| {
                let x = win::get_x_lparam(lparam);
                let y = win::get_y_lparam(lparam);
                let pt = client_to_layout(a, x, y);
                let hit = a.layout.hit_test(pt, a.input.scroll_row, a.input.hits.len(), a.snap.recent.len(), a.snap.dirs.len());
                // 抬起时触发（UI 文档 6.8）
                let act = match hit {
                    Hit::App(i) => Action::Launch(i),
                    Hit::File(i) => match a.snap.recent.get(i) {
                        Some(f) => Action::Open(f.path.clone()),
                        None => Action::None,
                    },
                    Hit::Dir(i) => match a.snap.dirs.get(i) {
                        Some(d) => Action::Open(d.path.clone()),
                        None => Action::None,
                    },
                    Hit::Toggle(zone, mode) => {
                        set_view(a, zone, mode);
                        Action::None
                    }
                    Hit::Settings => {
                        // SAFETY: 主窗口句柄有效；消息异步处理，避开 with_app 重入。
                        unsafe { let _ = PostMessageW(a.main, crate::app::WM_APP_SETTINGS, WPARAM(0), LPARAM(0)); }
                        Action::None
                    }
                    Hit::Search | Hit::Card => Action::None,
                };
                exec_action(a, act);
                let _ = render_frame(a);
            });
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let state = wparam.0 & 0xFFFF; // LOWORD
            if state == 0 {
                // WA_INACTIVE
                with_app(|a| {
                    if a.pad_visible {
                        hide(a, HideReason::Deactivate);
                    }
                });
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            with_app(|a| hide(a, HideReason::System));
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => {
            // SAFETY: 默认窗口过程。
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

/// 内容窗口客户区物理像素 → 布局坐标（工作区 DIP）。窗口原点 = content 外扩 SHADOW_MARGIN。
fn client_to_layout(a: &App, x: i32, y: i32) -> (f32, f32) {
    let scale = a.scale();
    (
        x as f32 / scale - SHADOW_MARGIN + a.layout.content.x,
        y as f32 / scale - SHADOW_MARGIN + a.layout.content.y,
    )
}

fn track_mouse_leave(hwnd: HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{TRACKMOUSEEVENT, TME_LEAVE, TrackMouseEvent};
    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
    // SAFETY: 结构体参数完整。
    unsafe { let _ = TrackMouseEvent(&mut tme); }
}

/// 时区偏移（秒）：本地时间 = Unix 时间 + 偏移。用 GetLocalTime 与 GetSystemTimeAsFileTime 求差。
pub fn tz_offset_sec() -> i64 {
    // SAFETY: 无参查询，返回值即结果。
    let st = unsafe { GetLocalTime() };
    let local = crate::fmt::days_from_civil(st.wYear as i32, st.wMonth as u32, st.wDay as u32) * 86_400
        + i64::from(st.wHour) * 3600
        + i64::from(st.wMinute) * 60
        + i64::from(st.wSecond);
    // SAFETY: 无参查询，返回值即结果。
    let ft = unsafe { GetSystemTimeAsFileTime() };
    let sys_unix = (((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64) / 10_000_000 - 11_644_473_600;
    local - sys_unix
}

/// 当前本地时间。
pub fn local_now() -> crate::fmt::LocalNow {
    // SAFETY: 无参查询，返回值即结果。
    let st = unsafe { GetLocalTime() };
    crate::fmt::LocalNow {
        year: st.wYear as i32,
        month: st.wMonth as u32,
        day: st.wDay as u32,
        hour: st.wHour as u32,
        minute: st.wMinute as u32,
    }
}

pub fn hide(app: &mut App, reason: HideReason) {
    deskpad_engine::log::info(&format!("pad::hide enter ({:?})", reason));
    app.pad_visible = false;
    // SAFETY: 光标闪烁定时器停掉；ANIM 定时器由关闭动画自行管理。
    unsafe {
        let _ = KillTimer(app.pad, CARET_TIMER);
    }
    if reason == HideReason::Launch {
        finalize_hide(app, reason);
    } else {
        crate::anim::start_close(app, reason);
    }
}

/// 隐藏的最终步骤（淡出结束或 Launch 直接进入）：隐藏窗口、释放资源、归还焦点。
pub fn finalize_hide(app: &mut App, reason: HideReason) {
    // SAFETY: 窗口句柄有效。
    unsafe {
        let _ = ShowWindow(app.pad, SW_HIDE);
        let _ = ShowWindow(app.scrim, SW_HIDE);
    }
    // 释放内存（UI 文档 6.5 第 4 步）：RT、DIB、图标位图、几何缓存
    app.pad_ctx = None;
    app.pad_surface.release();
    app.pad_factory = None;
    app.pad_dwrite = None;
    // 把释放掉的大块堆内存还给系统（不是压工作集，只是归还已经空闲的堆页）
    // SAFETY: 对本进程默认堆做整理，无副作用。
    unsafe {
        if let Ok(h) = windows::Win32::System::Memory::GetProcessHeap() {
            let _ = windows::Win32::System::Memory::HeapCompact(h, windows::Win32::System::Memory::HEAP_FLAGS(0));
        }
        windows::Win32::System::Com::CoFreeUnusedLibrariesEx(0, 0);
    }
    crate::win::log_mem("pad:hidden");
    // 5-#2：焦点还给呼出前的窗口
    if matches!(reason, HideReason::Escape | HideReason::Hotkey) {
        if let Some(h) = app.prev_fg {
            let hwnd = HWND(h as _);
            // SAFETY: IsWindow 校验后再 SetForegroundWindow。
            unsafe {
                if IsWindow(hwnd).as_bool() {
                    let _ = SetForegroundWindow(hwnd);
                }
            }
        }
    }
    app.prev_fg = None;
}

pub fn toggle(app: &mut App, reason: ShowReason) {
    if app.pad_visible {
        hide(app, HideReason::Hotkey);
    } else {
        show(app, reason);
    }
}

/// 引擎快照到达（WM_APP_SNAPSHOT）：重取数据、保持选中项、重绘。
pub fn on_snapshot(app: &mut App) {
    let sel_app_id = app
        .input
        .selected
        .and_then(|i| app.input.hits.get(i).copied())
        .and_then(|idx| app.snap.apps.get(idx))
        .map(|a| a.id);
    app.snap = app.eng.snapshot();
    app.input.hits = app.eng.search(&app.snap, &app.input.query);
    app.input.selected = sel_app_id
        .and_then(|id| {
            app.input
                .hits
                .iter()
                .position(|&i| app.snap.apps.get(i).map(|a| a.id) == Some(id))
        })
        .or(if app.snap.ready { Some(0) } else { None });
    if let Some(ctx) = app.pad_ctx.as_mut() {
        ctx.icons.prune(&app.snap);
    }
    if app.pad_visible {
        let _ = render_frame(app);
    }
}

/// 不上屏的整帧绘制（shot.rs 调试截图用）：只把像素画进 DIB，不调 present。
pub fn draw_only(app: &mut App) -> Result<()> {
    if app.pad_ctx.is_none() {
        let rt = app.pad_surface.rt().cloned();
        if let Some(rt) = rt {
            app.pad_ctx = RenderCtx::new(&pad_factory(app), &pad_dwrite(app), rt).ok();
        }
    }
    if let Some(mut ctx) = app.pad_ctx.take() {
        let factory = pad_factory(app);
        let r = draw_pad(app, &mut ctx, &factory);
        app.pad_ctx = Some(ctx);
        r?;
    }
    Ok(())
}

/// 启动台专用 D2D 工厂（按需创建；隐藏时在 finalize_hide 里丢弃）。
fn pad_factory(app: &mut App) -> windows::Win32::Graphics::Direct2D::ID2D1Factory {
    use windows::Win32::Graphics::Direct2D::{D2D1CreateFactory, ID2D1Factory, D2D1_FACTORY_TYPE_SINGLE_THREADED};
    if let Some(f) = &app.pad_factory {
        return f.clone();
    }
    // SAFETY: 标准 COM 工厂创建。
    match unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) } {
        Ok(f) => {
            app.pad_factory = Some(f.clone());
            f
        }
        Err(_) => app.factory.clone(), // 退回进程级工厂，功能不受影响
    }
}

/// 启动台专用 DWrite 工厂（ISOLATED，按需创建；隐藏时丢弃）。
fn pad_dwrite(app: &mut App) -> windows::Win32::Graphics::DirectWrite::IDWriteFactory {
    use windows::Win32::Graphics::DirectWrite::{DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_ISOLATED};
    if let Some(f) = &app.pad_dwrite {
        return f.clone();
    }
    // SAFETY: 标准 COM 工厂创建。
    match unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_ISOLATED) } {
        Ok(f) => {
            app.pad_dwrite = Some(f.clone());
            f
        }
        Err(_) => app.dwrite.clone(),
    }
}
