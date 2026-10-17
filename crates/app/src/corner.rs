//! 桌面角标按钮 + 首次引导气泡（UI 文档 6.10）。
//!
//! 借用规则：本模块的 thread_local 状态（CORNER / BUBBLE）每次只借一下就放开，
//! 借用期间绝不调用会再次借用它们、或会同步派发窗口消息的函数。
use crate::app::{with_app, App, WM_MOUSELEAVE, ShowReason};
use crate::render::{draw_bubble, draw_corner, RenderCtx};
use crate::surface::Surface;
use crate::theme::SHADOW_MARGIN;
use crate::win;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const BUBBLE_TIMER: usize = 3;
/// 按钮命中区域（DIP）。任务栏内嵌形态只画产品图形，没有背景，所以命中区域就是
/// 图形四周留一点余量的方块（questions.md #33）。
pub const CORNER_SIZE: f32 = 28.0;
/// 产品图形边长（DIP）。
pub const GLYPH_SIZE: f32 = 18.0;

struct CornerState {
    surface: Surface,
    ctx: Option<RenderCtx>,
    /// 当前角标所在显示器的 DPI（与启动台的 DPI 无关）。
    dpi: u32,
    hover: bool,
    pressed: bool,
    /// 按下时的鼠标屏幕坐标（物理像素）。
    down: (i32, i32),
}

thread_local! {
    static CORNER: std::cell::RefCell<CornerState> = std::cell::RefCell::new(CornerState {
        surface: Surface::new(),
        ctx: None,
        dpi: 96,
        hover: false,
        pressed: false,
        down: (0, 0),
    });
    static BUBBLE: std::cell::RefCell<Option<(Surface, Option<RenderCtx>)>> =
        const { std::cell::RefCell::new(None) };
}

fn corner_dpi() -> u32 {
    CORNER.with_borrow(|cs| cs.dpi).max(96)
}

fn monitor_at(x: i32, y: i32, fallback_primary: bool) -> HMONITOR {
    let flag = if fallback_primary { MONITOR_DEFAULTTOPRIMARY } else { MONITOR_DEFAULTTONEAREST };
    // SAFETY: 纯查询。
    unsafe { MonitorFromPoint(POINT { x, y }, flag) }
}

fn work_of(mon: HMONITOR) -> RECT {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: mi 为输出参数。
    unsafe { let _ = GetMonitorInfoW(mon, &mut mi); }
    mi.rcWork
}

fn dpi_of(mon: HMONITOR) -> u32 {
    let (mut dx, mut dy) = (0u32, 0u32);
    // SAFETY: 输出参数由本函数持有。
    unsafe { let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy); }
    if dx == 0 { 96 } else { dx }
}

fn body_px(dpi: u32) -> i32 {
    (CORNER_SIZE * dpi as f32 / 96.0).round() as i32
}

/// 气泡窗口四周的透明边距（物理像素）。只有气泡还带 L2 阴影，按钮本身已经没有背景和阴影。
fn margin_px_bubble(dpi: u32) -> i32 {
    (SHADOW_MARGIN * dpi as f32 / 96.0).round() as i32
}

/// 任务栏窗口（主任务栏）。找不到时返回 None（explorer 未启动 / 被 shell 替换）。
fn taskbar() -> Option<(HWND, RECT)> {
    // SAFETY: 纯查询；返回的句柄只用于读矩形与排序。
    let hwnd = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok()?;
    if hwnd.is_invalid() {
        return None;
    }
    let mut r = RECT::default();
    // SAFETY: hwnd 有效，r 为输出参数。
    unsafe { let _ = GetWindowRect(hwnd, &mut r); }
    Some((hwnd, r))
}

/// 按钮位置：内嵌 Windows 底部任务栏——左端靠底（questions.md #30）。
/// 位置固定、不支持拖动（questions.md #33）。
/// 任务栏不存在时退回旧的桌面左下角。
fn default_pos() -> (i32, i32, u32) {
    if let Some((_, tr)) = taskbar() {
        let mon = monitor_at((tr.left + tr.right) / 2, (tr.top + tr.bottom) / 2, true);
        let dpi = dpi_of(mon);
        let s = dpi as f32 / 96.0;
        let gap = (8.0 * s).round() as i32;
        let body = body_px(dpi);
        // 垂直方向在任务栏里居中；任务栏比按钮高不多，居中即"靠底"
        let y = tr.top + ((tr.bottom - tr.top) - body) / 2;
        return (tr.left + gap, y, dpi);
    }
    let mon = monitor_at(0, 0, true);
    let work = work_of(mon);
    let dpi = dpi_of(mon);
    let s = dpi as f32 / 96.0;
    let gap = (10.0 * s).round() as i32;
    (work.left + gap, work.bottom - gap - body_px(dpi), dpi)
}

/// 创建角标与气泡窗口并摆放（位置固定，见 [`default_pos`]）。
pub fn create() -> Result<()> {
    win::register_class(w!("DeskPadCorner"), Some(corner_proc), None)?;
    win::register_class(w!("DeskPadBubble"), Some(bubble_proc), None)?;
    let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    // 顶层分层窗口，定位在任务栏左端、层级 HWND_TOPMOST（questions.md #31）。
    let corner = win::create_window(w!("DeskPadCorner"), w!(""), WS_POPUP, ex, None, 0, 0, 0, 0)?;
    let bubble = win::create_window(w!("DeskPadBubble"), w!(""), WS_POPUP, ex, None, 0, 0, 0, 0)?;
    with_app(|a| {
        a.corner = corner;
        a.bubble = bubble;
        a.corner_pos = None;
    });
    reposition();
    Ok(())
}

/// 按 app.corner_pos 摆放（None = 默认位置）。
pub fn reposition() {
    with_app(reposition_with);
}

fn reposition_with(a: &mut App) {
    // 位置固定：始终贴任务栏左端（questions.md #33，不再支持拖动）。
    a.corner_pos = None;
    let (bx, by, dpi) = default_pos();
    CORNER.with_borrow_mut(|cs| cs.dpi = dpi);
    let total = body_px(dpi);
    // 屏幕物理像素定位；z 序由 WM_WINDOWPOSCHANGING 设为 HWND_TOPMOST。
    // 不做 SetParent：跨进程子窗口会被任务栏自己的绘制盖住（questions.md #31）。
    // SAFETY: 窗口句柄有效。
    unsafe {
        let _ = SetWindowPos(a.corner, HWND_TOPMOST, bx, by, total, total, SWP_NOACTIVATE);
    }
    if let Err(e) = present_corner(a, (bx, by)) {
        deskpad_engine::log::warn(&format!("corner present failed: {e}"));
    }
    // SAFETY: 不抢焦点地显示。
    unsafe { let _ = ShowWindow(a.corner, SW_SHOWNOACTIVATE); }
}

/// 当前按钮主体左上角（物理像素）。
fn body_pos(_a: &App) -> (i32, i32) {
    let (x, y, _) = default_pos();
    (x, y)
}

fn present_corner(a: &App, body: (i32, i32)) -> Result<()> {
    CORNER.with_borrow_mut(|cs| {
        let dpi = cs.dpi.max(96);
        let total = body_px(dpi);
        if cs.surface.size != (total, total) {
            cs.ctx = None; // DPI 变了，旧 RT 上的设备资源作废
        }
        cs.surface.ensure(&a.factory, total, total, dpi as f32)?;
        if cs.ctx.is_none() {
            if let Some(rt) = cs.surface.rt().cloned() {
                cs.ctx = Some(RenderCtx::new(&a.factory, &a.dwrite, rt)?);
            }
        }
        let hover = cs.hover;
        if let Some(ctx) = cs.ctx.as_mut() {
            draw_corner(a, ctx, &a.factory, hover)?;
        }
        cs.surface.present(a.corner, POINT { x: body.0, y: body.1 }, 255)
    })
}

/// 设置变化后重绘。
pub fn redraw() {
    with_app(|a| {
        let body = body_pos(a);
        let _ = present_corner(a, body);
    });
}

/// WM_DISPLAYCHANGE / WM_SETTINGCHANGE(SPI_SETWORKAREA)：重新钳回可见工作区。
pub fn reclamp() {
    with_app(reclamp_with);
}

pub fn reclamp_with(a: &mut App) {
    reposition_with(a);
}

/// 显示首次引导气泡（first_run_done 为 false 时）。调用方已持有 App。
pub fn show_bubble(a: &mut App) {
    let s = a.eng.settings();
    if s.first_run_done {
        return;
    }
    let text = format!("点击这里，或按 {} 打开启动台", s.hotkey.label());
    let (bx, by) = body_pos(a);
    let dpi = corner_dpi();
    let scale = dpi as f32 / 96.0;
    let (w_dip, h_dip) = crate::render::bubble_size();
    let total_w = ((w_dip + 2.0 * SHADOW_MARGIN) * scale).round() as i32;
    let total_h = ((h_dip + 2.0 * SHADOW_MARGIN) * scale).round() as i32;
    let body = body_px(dpi);
    let gap = (8.0 * scale).round() as i32;
    let work = work_of(monitor_at(bx, by, false));
    let m = margin_px_bubble(dpi);
    // 右侧：气泡（含自身阴影边距 m）左边距按钮 8 DIP，垂直居中对齐按钮
    let mut px = bx + body + gap - m;
    let mut py = by + body / 2 - total_h / 2;
    if px + total_w - m > work.right {
        // 右侧放不下 → 放到按钮上方
        px = bx - m;
        py = by - gap - (total_h - m);
    }
    let shown = BUBBLE.with_borrow_mut(|b| -> Result<()> {
        let pair = b.get_or_insert_with(|| (Surface::new(), None));
        if pair.0.size != (total_w, total_h) {
            pair.1 = None;
        }
        pair.0.ensure(&a.factory, total_w, total_h, dpi as f32)?;
        if pair.1.is_none() {
            if let Some(rt) = pair.0.rt().cloned() {
                pair.1 = Some(RenderCtx::new(&a.factory, &a.dwrite, rt)?);
            }
        }
        if let Some(ctx) = pair.1.as_mut() {
            draw_bubble(a, ctx, &a.factory, &text)?;
        }
        pair.0.present(a.bubble, POINT { x: px, y: py }, 255)
    });
    if shown.is_ok() {
        // SAFETY: 不抢焦点显示；8 秒后由主窗口 WM_TIMER 收起。
        unsafe {
            let _ = ShowWindow(a.bubble, SW_SHOWNA);
            let _ = SetTimer(a.main, BUBBLE_TIMER, 8000, None);
        }
        a.bubble_visible = true;
    } else {
        deskpad_engine::log::warn("bubble render failed");
    }
}

/// 隐藏气泡。mark_done = true 时写入 first_run_done。
pub fn dismiss_bubble(a: &mut App, mark_done: bool) {
    if !a.bubble_visible {
        return;
    }
    a.bubble_visible = false;
    // SAFETY: 窗口句柄有效；定时器配对删除。
    unsafe {
        let _ = KillTimer(a.main, BUBBLE_TIMER);
        let _ = ShowWindow(a.bubble, SW_HIDE);
    }
    BUBBLE.with_borrow_mut(|b| *b = None); // 释放位图
    if mark_done {
        let mut s = a.eng.settings();
        if !s.first_run_done {
            s.first_run_done = true;
            let _ = a.eng.save_settings(&s);
        }
    }
}

/// 主窗口 WM_TIMER(BUBBLE_TIMER)：气泡 8 秒超时。
pub fn on_bubble_timer() {
    with_app(|a| dismiss_bubble(a, true));
}

fn screen_cursor() -> (i32, i32) {
    let mut pt = POINT::default();
    // SAFETY: 输出参数。
    unsafe { let _ = GetCursorPos(&mut pt); }
    (pt.x, pt.y)
}

fn track_leave(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    // SAFETY: 结构体参数完整；重复登记是幂等的。
    unsafe { let _ = TrackMouseEvent(&mut tme); }
}

/// 拖动中：按鼠标位置计算新的主体位置（含网格吸附），并移动窗口。
pub unsafe extern "system" fn corner_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        // 层级（UI 文档 6.10）：内嵌任务栏 → 置顶（与任务栏同属 topmost 段，
        // 绘制在任务栏之上，不被其绘制流程盖住）。
        WM_WINDOWPOSCHANGING => {
            let wp = lparam.0 as *mut WINDOWPOS;
            if !wp.is_null() {
                // SAFETY: lParam 指向系统提供的 WINDOWPOS，改写 hwndInsertAfter 合法。
                (*wp).hwndInsertAfter = HWND_TOPMOST;
                (*wp).flags &= !SWP_NOZORDER;
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            // SAFETY: 捕获鼠标直到 BUTTONUP / CAPTURECHANGED。
            unsafe { let _ = SetCapture(hwnd); }
            let cur = screen_cursor();
            CORNER.with_borrow_mut(|cs| {
                cs.pressed = true;
                cs.down = cur;
            });
            with_app(|a| dismiss_bubble(a, true));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            let dpi = corner_dpi();
            let x = win::get_x_lparam(lparam);
            let y = win::get_y_lparam(lparam);
            let body = body_px(dpi);
            let inside = x >= 0 && y >= 0 && x < body && y < body;
            let changed = CORNER.with_borrow_mut(|cs| {
                if cs.hover != inside {
                    cs.hover = inside;
                    true
                } else {
                    false
                }
            });
            if changed {
                redraw();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let changed = CORNER.with_borrow_mut(|cs| std::mem::replace(&mut cs.hover, false));
            if changed {
                redraw();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let pressed = CORNER.with_borrow_mut(|cs| std::mem::replace(&mut cs.pressed, false));
            // SAFETY: 与 SetCapture 配对（会触发 WM_CAPTURECHANGED，此时状态已清零）。
            unsafe { let _ = ReleaseCapture(); }
            if pressed {
                with_app(|a| crate::pad::toggle(a, ShowReason::Corner));
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            CORNER.with_borrow_mut(|cs| cs.pressed = false);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let (x, y) = screen_cursor();
            corner_menu(POINT { x, y });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // DPI 变化：按新 DPI 重排（位置不变，固定贴任务栏左端）
            reposition();
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // SAFETY: 默认窗口过程。
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn corner_menu(pt: POINT) {
    let Some(main) = with_app(|a| a.main) else { return };
    // SAFETY: 菜单创建/弹出/销毁配对；SetForegroundWindow + TPM_RETURNCMD + WM_NULL 是弹出菜单的标准写法。
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _g = crate::win::HMenuGuard(menu);
        let _ = AppendMenuW(menu, MF_STRING, 1, w!("打开启动台"));
        let _ = AppendMenuW(menu, MF_STRING, 2, w!("设置…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 3, w!("退出"));
        let _ = SetForegroundWindow(main);
        let cmd = TrackPopupMenuEx(menu, (TPM_RIGHTBUTTON | TPM_RETURNCMD).0, pt.x, pt.y, main, None);
        let _ = PostMessageW(main, WM_NULL, WPARAM(0), LPARAM(0));
        cmd.0 as u32
    };
    match cmd {
        1 => {
            with_app(|a| crate::pad::show(a, ShowReason::Corner));
        }
        2 => {
            let _ = crate::settings_ui::open(main);
        }
        3 => crate::winmain::quit(),
        _ => {}
    }
}

pub unsafe extern "system" fn bubble_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP => {
            with_app(|a| dismiss_bubble(a, true));
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // SAFETY: 默认窗口过程。
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
