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
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const BUBBLE_TIMER: usize = 3;
pub const CORNER_SIZE: f32 = 40.0;
const GRID_DIP: f32 = 40.0;

struct CornerState {
    surface: Surface,
    ctx: Option<RenderCtx>,
    /// 当前角标所在显示器的 DPI（与启动台的 DPI 无关）。
    dpi: u32,
    hover: bool,
    pressed: bool,
    dragging: bool,
    /// 按下时鼠标相对按钮主体左上角的偏移（物理像素）。
    grab: (i32, i32),
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
        dragging: false,
        grab: (0, 0),
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

fn margin_px(dpi: u32) -> i32 {
    (SHADOW_MARGIN * dpi as f32 / 96.0).round() as i32
}

/// 默认位置：主显示器工作区左下角，距左 10 DIP、距下 10 DIP。
fn default_pos() -> (i32, i32, u32) {
    let mon = monitor_at(0, 0, true);
    let work = work_of(mon);
    let dpi = dpi_of(mon);
    let s = dpi as f32 / 96.0;
    let gap = (10.0 * s).round() as i32;
    (work.left + gap, work.bottom - gap - body_px(dpi), dpi)
}

/// 把按钮主体钳进它中心点所在显示器的工作区；返回 (x, y, dpi)。
fn clamp_body(bx: i32, by: i32) -> (i32, i32, u32) {
    let probe_dpi = corner_dpi();
    let half = body_px(probe_dpi) / 2;
    let mon = monitor_at(bx + half, by + half, false);
    let work = work_of(mon);
    let dpi = dpi_of(mon);
    let body = body_px(dpi);
    let x = bx.clamp(work.left, (work.right - body).max(work.left));
    let y = by.clamp(work.top, (work.bottom - body).max(work.top));
    (x, y, dpi)
}

/// 创建角标与气泡窗口，读取设置里的位置并摆放。
pub fn create() -> Result<()> {
    win::register_class(w!("DeskPadCorner"), Some(corner_proc), None)?;
    win::register_class(w!("DeskPadBubble"), Some(bubble_proc), None)?;
    let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let corner = win::create_window(w!("DeskPadCorner"), w!(""), WS_POPUP, ex, None, 0, 0, 0, 0)?;
    let bubble = win::create_window(w!("DeskPadBubble"), w!(""), WS_POPUP, ex, None, 0, 0, 0, 0)?;
    with_app(|a| {
        a.corner = corner;
        a.bubble = bubble;
        a.corner_pos = a.eng.settings().launcher_pos;
    });
    reposition();
    Ok(())
}

/// 按 app.corner_pos 摆放（None = 默认位置）。
pub fn reposition() {
    with_app(reposition_with);
}

fn reposition_with(a: &mut App) {
    let (bx, by, dpi) = match a.corner_pos {
        Some((x, y)) => clamp_body(x, y),
        None => default_pos(),
    };
    // 只有用户拖动过才记住位置；默认位置不写回 corner_pos，换主屏后仍回到默认角落
    if a.corner_pos.is_some() {
        a.corner_pos = Some((bx, by));
    }
    CORNER.with_borrow_mut(|cs| cs.dpi = dpi);
    let m = margin_px(dpi);
    let total = body_px(dpi) + 2 * m;
    // SAFETY: 窗口句柄有效；层级在 WM_WINDOWPOSCHANGING 里强制为 HWND_BOTTOM。
    unsafe {
        let _ = SetWindowPos(a.corner, HWND_BOTTOM, bx - m, by - m, total, total, SWP_NOACTIVATE);
    }
    let _ = present_corner(a, (bx, by));
    // SAFETY: 不抢焦点地显示。
    unsafe { let _ = ShowWindow(a.corner, SW_SHOWNOACTIVATE); }
}

/// 当前按钮主体左上角（物理像素）。
fn body_pos(a: &App) -> (i32, i32) {
    match a.corner_pos {
        Some(p) => p,
        None => {
            let (x, y, _) = default_pos();
            (x, y)
        }
    }
}

fn present_corner(a: &App, body: (i32, i32)) -> Result<()> {
    CORNER.with_borrow_mut(|cs| {
        let dpi = cs.dpi.max(96);
        let m = margin_px(dpi);
        let total = body_px(dpi) + 2 * m;
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
        cs.surface.present(a.corner, POINT { x: body.0 - m, y: body.1 - m }, 255)
    })
}

/// 设置变化后重绘。
pub fn redraw() {
    with_app(|a| {
        let body = body_pos(a);
        let _ = present_corner(a, body);
    });
}

/// 设置窗口 [重置到默认] 保存后调用。
pub fn reposition_to_default(a: &mut App) {
    a.corner_pos = None;
    reposition_with(a);
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
    let m = margin_px(dpi);
    let body = body_px(dpi);
    let gap = (8.0 * scale).round() as i32;
    let work = work_of(monitor_at(bx, by, false));
    // 右侧：气泡主体左边距按钮 8 DIP，垂直居中对齐按钮
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
fn drag_to(a: &mut App, cursor: (i32, i32), grab: (i32, i32)) {
    let dpi = corner_dpi();
    let mut nx = cursor.0 - grab.0;
    let mut ny = cursor.1 - grab.1;
    if a.eng.settings().grid_snap {
        let g = ((GRID_DIP * dpi as f32 / 96.0).round() as i32).max(1);
        let work = work_of(monitor_at(cursor.0, cursor.1, false));
        let snap = |v: i32, origin: i32| origin + ((v - origin) as f32 / g as f32).round() as i32 * g;
        nx = snap(nx, work.left);
        ny = snap(ny, work.top);
    }
    a.corner_pos = Some((nx, ny));
    reposition_with(a);
}

pub unsafe extern "system" fn corner_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        // 始终待在普通窗口下面，像桌面的一部分（UI 文档 6.10）
        WM_WINDOWPOSCHANGING => {
            let wp = lparam.0 as *mut WINDOWPOS;
            if !wp.is_null() {
                // SAFETY: lParam 指向系统提供的 WINDOWPOS，改写 hwndInsertAfter 合法。
                (*wp).hwndInsertAfter = HWND_BOTTOM;
                (*wp).flags &= !SWP_NOZORDER;
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            // SAFETY: 捕获鼠标直到 BUTTONUP / CAPTURECHANGED。
            unsafe { let _ = SetCapture(hwnd); }
            let cur = screen_cursor();
            let body = with_app(|a| body_pos(a)).unwrap_or(cur);
            CORNER.with_borrow_mut(|cs| {
                cs.pressed = true;
                cs.dragging = false;
                cs.down = cur;
                cs.grab = (cur.0 - body.0, cur.1 - body.1);
            });
            with_app(|a| dismiss_bubble(a, true));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            let cur = screen_cursor();
            let dpi = corner_dpi();
            // SAFETY: 纯查询。
            let (tx, ty) = unsafe {
                (GetSystemMetricsForDpi(SM_CXDRAG, dpi), GetSystemMetricsForDpi(SM_CYDRAG, dpi))
            };
            let x = win::get_x_lparam(lparam);
            let y = win::get_y_lparam(lparam);
            let m = margin_px(dpi);
            let body = body_px(dpi);
            let inside = x >= m && y >= m && x < m + body && y < m + body;
            // 先在一次短借用里算出要做什么，再在借用之外执行
            let (drag, grab, hover_changed) = CORNER.with_borrow_mut(|cs| {
                if cs.pressed && !cs.dragging
                    && ((cur.0 - cs.down.0).abs() > tx || (cur.1 - cs.down.1).abs() > ty)
                {
                    cs.dragging = true;
                }
                let changed = !cs.dragging && cs.hover != inside;
                if changed {
                    cs.hover = inside;
                }
                (cs.dragging, cs.grab, changed)
            });
            if drag {
                with_app(|a| drag_to(a, cur, grab));
            } else if hover_changed {
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
            let (pressed, dragging) = CORNER.with_borrow_mut(|cs| {
                let r = (cs.pressed, cs.dragging);
                cs.pressed = false;
                cs.dragging = false;
                r
            });
            // SAFETY: 与 SetCapture 配对（会触发 WM_CAPTURECHANGED，此时状态已清零）。
            unsafe { let _ = ReleaseCapture(); }
            if dragging {
                with_app(|a| {
                    reposition_with(a); // 钳回工作区
                    let mut s = a.eng.settings();
                    s.launcher_pos = a.corner_pos;
                    let _ = a.eng.save_settings(&s);
                });
            } else if pressed {
                with_app(|a| crate::pad::toggle(a, ShowReason::Corner));
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            CORNER.with_borrow_mut(|cs| {
                cs.pressed = false;
                cs.dragging = false;
            });
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let (x, y) = screen_cursor();
            corner_menu(POINT { x, y });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // 拖到另一块 DPI 不同的显示器：按新 DPI 重排（位置不变）
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
