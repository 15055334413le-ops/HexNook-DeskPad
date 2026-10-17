//! 设置窗口（UI 文档 6.11）：tofour Modal，含热键单选、自启勾选、重看引导。
//!
//! 借用规则：S 每次只借一下；DestroyWindow / SetForegroundWindow / with_app 等都在借用之外调用。
//! 渲染目标的 DPI 已设为显示器 DPI，所以这里所有坐标都是 DIP，不要再乘缩放系数。
use crate::app::{with_app, HideReason};
use crate::render::{DpiScale, RenderCtx};
use crate::smooth::{draw_smooth_rect, draw_soft_shadow, fill_smooth_rect};
use crate::surface::Surface;
use crate::theme::*;
use crate::win;
use deskpad_engine::HotkeyPreset;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

/// 主体尺寸（DIP，不含阴影边距）。
const W: f32 = 420.0;
/// v0.2 起高 368：新增「更新」一行（版本 + 状态 + 按钮 / 自动检查勾选）。
const H: f32 = 368.0;
const LABEL_X: f32 = 24.0;
const CTRL_X: f32 = 136.0;
const HEADER_H: f32 = 48.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctl {
    Radio(usize), // HotkeyPreset::ALL 下标
    CheckAuto,
    /// 「更新」行的「自动检查更新」勾选。
    CheckUpdate,
    /// 「更新」行右侧按钮：没有新版本时是次按钮「检查更新」，有新版本时是主按钮「立即更新」。
    BtnUpdate,
    BtnReplay,
    BtnCancel,
    BtnSave,
    CloseX,
}

/// Tab 顺序（CloseX 不参与 Tab）。
const TAB_ORDER: [Ctl; 9] = [
    Ctl::Radio(0),
    Ctl::Radio(1),
    Ctl::Radio(2),
    Ctl::CheckAuto,
    Ctl::CheckUpdate,
    Ctl::BtnUpdate,
    Ctl::BtnReplay,
    Ctl::BtnCancel,
    Ctl::BtnSave,
];

/// 控件命中矩形（主体坐标系，DIP）。
fn ctl_rect(c: Ctl) -> D2D_RECT_F {
    let r = |x: f32, y: f32, w: f32, h: f32| D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
    match c {
        Ctl::Radio(i) => r(CTRL_X, 62.0 + i as f32 * 26.0, 240.0, 22.0),
        Ctl::CheckAuto => r(CTRL_X, 184.0, 240.0, 22.0),
        // 「更新」一行：勾选 + 右侧按钮（100×32，与保存同宽）
        Ctl::CheckUpdate => r(CTRL_X, 216.0, 240.0, 22.0),
        Ctl::BtnUpdate => r(W - 24.0 - 100.0, 246.0, 100.0, 32.0),
        Ctl::BtnReplay => r(CTRL_X, 290.0, 120.0, 28.0),
        Ctl::BtnCancel => r(W - 24.0 - 100.0 - 12.0 - 80.0, H - 46.0, 80.0, 32.0),
        Ctl::BtnSave => r(W - 24.0 - 100.0, H - 46.0, 100.0, 32.0),
        Ctl::CloseX => r(W - 40.0, 12.0, 24.0, 24.0),
    }
}

fn ctl_at(x: f32, y: f32) -> Option<Ctl> {
    TAB_ORDER
        .iter()
        .copied()
        .chain(std::iter::once(Ctl::CloseX))
        .find(|&c| {
            let r = ctl_rect(c);
            x >= r.left && x < r.right && y >= r.top && y < r.bottom
        })
}

struct State {
    hwnd: HWND,
    dpi: u32,
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    surface: Surface,
    ctx: Option<RenderCtx>,
    hotkey_idx: usize,
    autostart: bool,
    auto_update: bool,
    update_state: crate::update_ui::UpdateState,
    replay: bool,
    hover: Option<Ctl>,
    pressed: Option<Ctl>,
    focus: Option<Ctl>,
    err: Option<String>,
    /// 拖动标题栏：按下时鼠标相对窗口左上角的偏移（物理像素）。
    drag: Option<(i32, i32)>,
}

/// 检查更新的子进程写完后由 WM_APP_UPDATE 调用：更新状态并重绘。
pub fn set_update_state(hwnd: HWND, state: crate::update_ui::UpdateState) {
    let changed = S.with_borrow_mut(|s| {
        let Some(st) = s.as_mut() else { return false };
        if st.hwnd != hwnd {
            return false;
        }
        st.update_state = state;
        true
    });
    if changed {
        redraw();
    }
}

thread_local! {
    static S: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// 鼠标所在显示器的 (工作区, DPI)。
fn cursor_monitor() -> (RECT, u32) {
    let mut pt = POINT::default();
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let (mut dx, mut dy) = (0u32, 0u32);
    // SAFETY: 全部是输出参数查询。
    unsafe {
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(mon, &mut mi);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    }
    (mi.rcWork, if dx == 0 { 96 } else { dx })
}

fn total_px(dpi: u32) -> (i32, i32) {
    let s = dpi as f32 / 96.0;
    (((W + 2.0 * SHADOW_MARGIN) * s).round() as i32, ((H + 2.0 * SHADOW_MARGIN) * s).round() as i32)
}

#[allow(dead_code)] // 状态查询预留
pub fn is_open() -> bool {
    S.with_borrow(|s| s.is_some())
}

/// 打开设置窗口；已打开时激活已有的那个。不得在 with_app 内调用。
pub fn open(_main: HWND) -> Result<()> {
    if let Some(hwnd) = S.with_borrow(|s| s.as_ref().map(|st| st.hwnd)) {
        // SAFETY: 窗口句柄有效。
        unsafe { let _ = SetForegroundWindow(hwnd); }
        return Ok(());
    }
    // 打开设置前先收起启动台（UI 文档 6.11）
    with_app(|a| crate::pad::hide(a, HideReason::System));
    win::register_class(w!("DeskPadSettings"), Some(settings_proc), None)?;
    let Some((factory, dwrite, s)) = with_app(|a| (a.factory.clone(), a.dwrite.clone(), a.eng.settings())) else {
        return Ok(());
    };
    let (work, dpi) = cursor_monitor();
    let (tw, th) = total_px(dpi);
    let x = work.left + ((work.right - work.left) - tw) / 2;
    let y = work.top + ((work.bottom - work.top) - th) / 2;
    let hwnd = win::create_window(
        w!("DeskPadSettings"), w!("DeskPad 设置"), WS_POPUP,
        WS_EX_LAYERED | WS_EX_TOOLWINDOW,
        None, x, y, tw, th,
    )?;
    let hotkey_idx = HotkeyPreset::ALL.iter().position(|&p| p == s.hotkey).unwrap_or(0);
    S.with_borrow_mut(|slot| {
        *slot = Some(State {
            hwnd,
            dpi,
            factory,
            dwrite,
            surface: Surface::new(),
            ctx: None,
            hotkey_idx,
            autostart: s.autostart,
            auto_update: s.auto_check_update,
            update_state: crate::update_ui::state_from(&deskpad_engine::update::load()),
            replay: false,
            hover: None,
            pressed: None,
            focus: None,
            err: None,
            drag: None,
        })
    });
    redraw();
    // SAFETY: 窗口句柄有效。
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

/// 关闭：先把状态取出来，再在借用之外销毁窗口。
fn close() {
    let st = S.with_borrow_mut(|s| s.take());
    if let Some(st) = st {
        // SAFETY: 窗口由本模块创建；WM_DESTROY 里不会再借用到已取出的状态。
        unsafe { let _ = DestroyWindow(st.hwnd); }
    }
}

/// 保存：热键 → 设置 → 引导气泡；热键冲突时留在窗口上显示错误。
fn save() {
    let Some((hk_idx, autostart, auto_update, replay)) = S.with_borrow(|s| {
        s.as_ref().map(|st| (st.hotkey_idx, st.autostart, st.auto_update, st.replay))
    }) else {
        return;
    };
    let result: Option<std::result::Result<(), String>> = with_app(|a| {
        let mut s = a.eng.settings();
        let new_hk = HotkeyPreset::ALL[hk_idx.min(HotkeyPreset::ALL.len() - 1)];
        if s.hotkey != new_hk {
            if !crate::hotkey::change(a.main, s.hotkey, new_hk) {
                return Err(format!("{} 已被其他程序占用，请换一个。", new_hk.label()));
            }
            s.hotkey = new_hk;
        }
        s.autostart = autostart;
        s.auto_check_update = auto_update;
        if replay {
            s.first_run_done = false;
        }
        if let Err(e) = a.eng.save_settings(&s) {
            deskpad_engine::log::warn(&format!("settings save: {e:?}"));
            return Err("写入设置失败，详情见日志".into());
        }
        if replay {
            crate::corner::show_bubble(a);
        }
        Ok(())
    });
    match result {
        Some(Ok(())) => close(),
        Some(Err(msg)) => {
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = Some(msg);
                }
            });
            redraw();
        }
        None => {}
    }
}

fn redraw() {
    let r = S.with_borrow_mut(|slot| -> Result<()> {
        let Some(st) = slot.as_mut() else { return Ok(()) };
        let (tw, th) = total_px(st.dpi);
        if st.surface.size != (tw, th) {
            st.ctx = None;
        }
        st.surface.ensure(&st.factory, tw, th, st.dpi as f32)?;
        if st.ctx.is_none() {
            if let Some(rt) = st.surface.rt().cloned() {
                st.ctx = Some(RenderCtx::new(&st.factory, &st.dwrite, rt)?);
            }
        }
        draw(st)?;
        let mut wr = RECT::default();
        // SAFETY: 输出参数。
        unsafe { let _ = GetWindowRect(st.hwnd, &mut wr); }
        st.surface.present(st.hwnd, POINT { x: wr.left, y: wr.top }, 255)
    });
    if let Err(e) = r {
        deskpad_engine::log::warn(&format!("settings redraw: {e}"));
    }
}

fn draw(st: &mut State) -> Result<()> {
    let Some(ctx) = st.ctx.as_mut() else { return Ok(()) };
    let rt = ctx.rt.clone();
    // SAFETY: 绘制序列配对。
    unsafe {
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
    }
    let view = View {
        hotkey_idx: st.hotkey_idx,
        autostart: st.autostart,
        auto_update: st.auto_update,
        update_state: st.update_state.clone(),
        replay: st.replay,
        hover: st.hover,
        pressed: st.pressed,
        focus: st.focus,
        err: st.err.clone(),
    };
    let r = draw_inner(ctx, &st.factory, &rt, &view);
    // SAFETY: 与 BeginDraw 配对。
    let end = unsafe { rt.EndDraw(None, None) };
    r?;
    end
}

struct View {
    hotkey_idx: usize,
    autostart: bool,
    auto_update: bool,
    update_state: crate::update_ui::UpdateState,
    replay: bool,
    hover: Option<Ctl>,
    pressed: Option<Ctl>,
    focus: Option<Ctl>,
    err: Option<String>,
}

/// 主体坐标 → 画布坐标（画布原点在主体外扩 SHADOW_MARGIN 处）。
fn at(r: D2D_RECT_F) -> D2D_RECT_F {
    let m = SHADOW_MARGIN;
    D2D_RECT_F { left: r.left + m, top: r.top + m, right: r.right + m, bottom: r.bottom + m }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    at(D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h })
}

fn text(rt: &ID2D1RenderTarget, fmt: &IDWriteTextFormat, s: &str, r: D2D_RECT_F, brush: &ID2D1Brush) {
    let w: Vec<u16> = s.encode_utf16().collect();
    // SAFETY: 字符串在调用期间有效。
    unsafe { rt.DrawText(&w, fmt, &r, brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL) };
}

fn draw_inner(ctx: &mut RenderCtx, factory: &ID2D1Factory, rt: &ID2D1RenderTarget, v: &View) -> Result<()> {
    let scale = rt.dpi_scale();
    let body = rect(0.0, 0.0, W, H);
    // 底板：L3 阴影 + 白底 + 12px 平滑圆角 + 1px 边框
    draw_soft_shadow(factory, &mut ctx.tiles, rt, body, RADIUS_MODAL, SHADOW_L3.0, SHADOW_L3.1, SHADOW_L3.2, scale)?;
    fill_smooth_rect(factory, &mut ctx.geo, rt, body, RADIUS_MODAL, ctx.pal.get(INPUT)?)?;
    draw_smooth_rect(factory, &mut ctx.geo, rt, body, RADIUS_MODAL, ctx.pal.get(BORDER)?, 1.0)?;

    // Header：标题 + × + 分隔线
    text(rt, &ctx.f_title, "设置", rect(LABEL_X, 12.0, 200.0, 24.0), ctx.pal.get(TEXT_PRIMARY)?);
    let x_col = if v.hover == Some(Ctl::CloseX) { TEXT_PRIMARY } else { TEXT_SECONDARY };
    let xr = at(ctl_rect(Ctl::CloseX));
    if v.hover == Some(Ctl::CloseX) {
        fill_smooth_rect(factory, &mut ctx.geo, rt, xr, RADIUS_INPUT, ctx.pal.get(ROW_HOVER)?)?;
    }
    text(rt, &ctx.f_cap_c, "✕", xr, ctx.pal.get(x_col)?);
    fill_smooth_rect(factory, &mut ctx.geo, rt, rect(0.0, HEADER_H, W, 1.0), 0.0, ctx.pal.get(BORDER)?)?;

    // 左侧标签
    for (label, y) in [("快捷键", 62.0), ("开机自启", 184.0), ("更新", 216.0), ("首次引导", 290.0)] {
        text(rt, &ctx.f_body, label, rect(LABEL_X, y, CTRL_X - LABEL_X - 8.0, 22.0), ctx.pal.get(TEXT_PRIMARY)?);
    }

    // 快捷键单选
    for (i, p) in HotkeyPreset::ALL.iter().enumerate() {
        let r = at(ctl_rect(Ctl::Radio(i)));
        let sel = v.hotkey_idx == i;
        let cy = (r.top + r.bottom) / 2.0;
        let dot = D2D_RECT_F { left: r.left, top: cy - 8.0, right: r.left + 16.0, bottom: cy + 8.0 };
        fill_smooth_rect(factory, &mut ctx.geo, rt, dot, 8.0, ctx.pal.get(if sel { PRIMARY } else { INPUT })?)?;
        if !sel {
            let border = if v.hover == Some(Ctl::Radio(i)) { BORDER_HOVER } else { BORDER };
            draw_smooth_rect(factory, &mut ctx.geo, rt, dot, 8.0, ctx.pal.get(border)?, 1.0)?;
        } else {
            let inner = D2D_RECT_F { left: dot.left + 5.0, top: dot.top + 5.0, right: dot.right - 5.0, bottom: dot.bottom - 5.0 };
            fill_smooth_rect(factory, &mut ctx.geo, rt, inner, 3.0, ctx.pal.get(INPUT)?)?;
        }
        let tr = D2D_RECT_F { left: r.left + 24.0, top: r.top, right: r.right, bottom: r.bottom };
        text(rt, &ctx.f_body, p.label(), tr, ctx.pal.get(TEXT_PRIMARY)?);
    }
    text(rt, &ctx.f_small, "Alt + Space 会替代窗口左上角菜单的系统快捷键",
        rect(CTRL_X, 140.0, W - CTRL_X - 24.0, 16.0), ctx.pal.get(TEXT_SECONDARY)?);
    if let Some(e) = &v.err {
        text(rt, &ctx.f_small, e, rect(CTRL_X, 158.0, W - CTRL_X - 24.0, 16.0), ctx.pal.get(DANGER)?);
    }

    // 勾选框
    {
        checkbox(ctx, factory, rt, Ctl::CheckAuto, v.autostart, "开机时自动启动", v)?;
    }

    // 「更新」一行：左 = 当前版本 + 状态；右 = 检查更新 / 立即更新按钮；下方 = 自动检查勾选
    draw_update_row(ctx, factory, rt, v)?;

    secondary_btn(ctx, factory, rt, Ctl::BtnReplay, if v.replay { "将重新显示 ✓" } else { "重新显示引导" }, v, v.replay)?;

    // Footer
    fill_smooth_rect(factory, &mut ctx.geo, rt, rect(0.0, H - 60.0, W, 1.0), 0.0, ctx.pal.get(BORDER)?)?;
    secondary_btn(ctx, factory, rt, Ctl::BtnCancel, "取消", v, false)?;
    let save = at(ctl_rect(Ctl::BtnSave));
    let save_col = if v.pressed == Some(Ctl::BtnSave) {
        PRIMARY_PRESSED
    } else if v.hover == Some(Ctl::BtnSave) {
        PRIMARY_HOVER
    } else {
        PRIMARY
    };
    fill_smooth_rect(factory, &mut ctx.geo, rt, save, RADIUS_INPUT, ctx.pal.get(save_col)?)?;
    text(rt, &ctx.f_body_c, "保存", save, ctx.pal.get(INPUT)?);

    // 键盘焦点环：1px PRIMARY，与控件间隔 2px
    if let Some(f) = v.focus {
        let r = at(ctl_rect(f));
        let ring = D2D_RECT_F { left: r.left - 2.0, top: r.top - 2.0, right: r.right + 2.0, bottom: r.bottom + 2.0 };
        draw_smooth_rect(factory, &mut ctx.geo, rt, ring, RADIUS_INPUT + 2.0, ctx.pal.get(PRIMARY)?, 1.0)?;
    }
    Ok(())
}

fn secondary_btn(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    c: Ctl,
    label: &str,
    v: &View,
    active: bool,
) -> Result<()> {
    let r = at(ctl_rect(c));
    let (bg, fg) = if active {
        (SELECT_BG, PRIMARY)
    } else if v.hover == Some(c) {
        (ROW_HOVER, TEXT_PRIMARY)
    } else {
        (INPUT, TEXT_PRIMARY)
    };
    fill_smooth_rect(factory, &mut ctx.geo, rt, r, RADIUS_INPUT, ctx.pal.get(bg)?)?;
    let border = if v.hover == Some(c) { BORDER_HOVER } else { BORDER };
    draw_smooth_rect(factory, &mut ctx.geo, rt, r, RADIUS_INPUT, ctx.pal.get(border)?, 1.0)?;
    text(rt, &ctx.f_body_c, label, r, ctx.pal.get(fg)?);
    Ok(())
}

/// 勾选框 + 右侧标签（开机自启 / 自动检查更新共用）。
fn checkbox(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    c: Ctl,
    checked: bool,
    label: &str,
    v: &View,
) -> Result<()> {
    let r = at(ctl_rect(c));
    let cy = (r.top + r.bottom) / 2.0;
    let b = D2D_RECT_F { left: r.left, top: cy - 8.0, right: r.left + 16.0, bottom: cy + 8.0 };
    fill_smooth_rect(factory, &mut ctx.geo, rt, b, 4.0, ctx.pal.get(if checked { PRIMARY } else { INPUT })?)?;
    if checked {
        // 白色对勾
        let brush = ctx.pal.get(INPUT)?;
        // SAFETY: 画刷有效；两段线组成对勾。
        unsafe {
            rt.DrawLine(D2D_POINT_2F { x: b.left + 3.5, y: cy + 0.5 }, D2D_POINT_2F { x: b.left + 6.5, y: cy + 3.5 }, brush, 1.6, None);
            rt.DrawLine(D2D_POINT_2F { x: b.left + 6.5, y: cy + 3.5 }, D2D_POINT_2F { x: b.left + 12.5, y: cy - 3.0 }, brush, 1.6, None);
        }
    } else {
        let border = if v.hover == Some(c) { BORDER_HOVER } else { BORDER };
        draw_smooth_rect(factory, &mut ctx.geo, rt, b, 4.0, ctx.pal.get(border)?, 1.0)?;
    }
    let tr = D2D_RECT_F { left: r.left + 24.0, top: r.top, right: r.right, bottom: r.bottom };
    text(rt, &ctx.f_body, label, tr, ctx.pal.get(TEXT_PRIMARY)?);
    Ok(())
}

/// 「更新」一行（y 216..278）：左上是「当前版本 + 状态」，右侧是按钮，左下是自动检查勾选。
/// 出错信息用 TEXT_SECONDARY 小字（卡片 UI 规则：错误不抢主色）。
fn draw_update_row(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    v: &View,
) -> Result<()> {
    // 第一行：「当前 0.2.0」 + 状态文案
    let row_y = 216.0;
    text(
        rt,
        &ctx.f_body,
        &format!("当前 {}", deskpad_engine::update::CURRENT_VERSION),
        rect(CTRL_X, row_y, 120.0, 22.0),
        ctx.pal.get(TEXT_PRIMARY)?,
    );
    let (status, status_color, secondary_font) = match &v.update_state {
        crate::update_ui::UpdateState::Unknown => (String::new(), TEXT_SECONDARY, true),
        crate::update_ui::UpdateState::Checking => ("正在检查…".into(), TEXT_SECONDARY, true),
        crate::update_ui::UpdateState::Latest => ("已是最新".into(), TEXT_SECONDARY, true),
        crate::update_ui::UpdateState::New(ver) => (format!("发现新版本 {ver}"), TEXT_PRIMARY, false),
        crate::update_ui::UpdateState::Failed(e) => (format!("检查失败：{e}"), TEXT_SECONDARY, true),
    };
    if !status.is_empty() {
        let fmt = if secondary_font { &ctx.f_small } else { &ctx.f_body };
        text(
            rt,
            fmt,
            &status,
            rect(CTRL_X + 128.0, row_y + (if secondary_font { 3.0 } else { 0.0 }), W - CTRL_X - 24.0, 22.0),
            ctx.pal.get(status_color)?,
        );
    }

    // 右侧按钮：没有新版本 → 次按钮「检查更新」；有新版本 → 主按钮「立即更新」
    let has_new = v.update_state.has_new();
    let label = if has_new { "立即更新" } else { "检查更新" };
    if has_new {
        let b = at(ctl_rect(Ctl::BtnUpdate));
        let col = if v.pressed == Some(Ctl::BtnUpdate) {
            PRIMARY_PRESSED
        } else if v.hover == Some(Ctl::BtnUpdate) {
            PRIMARY_HOVER
        } else {
            PRIMARY
        };
        fill_smooth_rect(factory, &mut ctx.geo, rt, b, RADIUS_INPUT, ctx.pal.get(col)?)?;
        text(rt, &ctx.f_body_c, label, b, ctx.pal.get(INPUT)?);
    } else {
        // 检查中按钮禁用态：只画框，hover/按下都不响应（After 里也会拦）
        let checking = matches!(v.update_state, crate::update_ui::UpdateState::Checking);
        secondary_btn(
            ctx,
            factory,
            rt,
            Ctl::BtnUpdate,
            if checking { "正在检查…" } else { label },
            v,
            false,
        )?;
    }

    // 第二行：「自动检查更新」勾选
    checkbox(
        ctx,
        factory,
        rt,
        Ctl::CheckUpdate,
        v.auto_update,
        "自动检查更新",
        v,
    )?;
    Ok(())
}

/// 点击/空格触发控件。返回要在借用之外执行的动作。
enum After {
    Redraw,
    Close,
    Save,
    /// 点「检查更新」/「立即更新」。
    Update,
}

fn activate(st: &mut State, c: Ctl) -> After {
    match c {
        Ctl::Radio(i) => {
            st.hotkey_idx = i;
            st.err = None;
            After::Redraw
        }
        Ctl::CheckAuto => {
            st.autostart = !st.autostart;
            After::Redraw
        }
        Ctl::CheckUpdate => {
            st.auto_update = !st.auto_update;
            After::Redraw
        }
        Ctl::BtnUpdate => {
            st.err = None;
            if st.update_state.has_new() {
                // 有新版本：直接用安装程序自更新（或打开发布仓库）
                crate::update_ui::apply_update();
                After::Close
            } else if matches!(st.update_state, crate::update_ui::UpdateState::Checking) {
                // 检查中的按钮不响应重复点击（同一时刻只允许一个子进程）
                After::Redraw
            } else {
                st.update_state = crate::update_ui::UpdateState::Checking;
                After::Update
            }
        }
        Ctl::BtnReplay => {
            st.replay = !st.replay;
            After::Redraw
        }
        Ctl::BtnCancel | Ctl::CloseX => After::Close,
        Ctl::BtnSave => After::Save,
    }
}

fn run(after: Option<After>) {
    match after {
        Some(After::Redraw) => redraw(),
        Some(After::Close) => close(),
        Some(After::Save) => save(),
        Some(After::Update) => {
            // 先重绘（显示「正在检查…」），再拉起一次性子进程
            redraw();
            crate::update_ui::check_now();
        }
        None => {}
    }
}

/// 客户区物理像素 → 主体 DIP 坐标。
fn to_body(st: &State, lparam: LPARAM) -> (f32, f32) {
    let s = st.dpi as f32 / 96.0;
    (
        win::get_x_lparam(lparam) as f32 / s - SHADOW_MARGIN,
        win::get_y_lparam(lparam) as f32 / s - SHADOW_MARGIN,
    )
}

pub unsafe extern "system" fn settings_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            // 拖动标题栏
            let drag = S.with_borrow(|s| s.as_ref().and_then(|st| st.drag));
            if let Some((ox, oy)) = drag {
                let mut pt = POINT::default();
                // SAFETY: 输出参数；移动自己的窗口。
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                    let _ = SetWindowPos(hwnd, HWND::default(), pt.x - ox, pt.y - oy, 0, 0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
                return LRESULT(0);
            }
            let changed = S.with_borrow_mut(|s| {
                let Some(st) = s.as_mut() else { return false };
                let (x, y) = to_body(st, lparam);
                let h = ctl_at(x, y);
                if h != st.hover {
                    st.hover = h;
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
        WM_LBUTTONDOWN => {
            let start_drag = S.with_borrow_mut(|s| {
                let st = s.as_mut()?;
                let (x, y) = to_body(st, lparam);
                st.pressed = ctl_at(x, y);
                if st.pressed.is_some() {
                    st.focus = st.pressed.filter(|c| *c != Ctl::CloseX);
                    return None;
                }
                if (0.0..HEADER_H).contains(&y) && (0.0..W).contains(&x) {
                    let px = win::get_x_lparam(lparam);
                    let py = win::get_y_lparam(lparam);
                    st.drag = Some((px, py));
                    return Some(());
                }
                None
            });
            // SAFETY: 捕获鼠标直到 BUTTONUP。
            unsafe { let _ = SetCapture(hwnd); }
            if start_drag.is_none() {
                redraw();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let after = S.with_borrow_mut(|s| {
                let st = s.as_mut()?;
                st.drag = None;
                let (x, y) = to_body(st, lparam);
                let pressed = st.pressed.take();
                match (pressed, ctl_at(x, y)) {
                    (Some(p), Some(h)) if p == h => Some(activate(st, p)),
                    (Some(_), _) => Some(After::Redraw),
                    _ => None,
                }
            });
            // SAFETY: 与 SetCapture 配对。
            unsafe { let _ = ReleaseCapture(); }
            run(after);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.drag = None;
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            // SAFETY: 查询修饰键状态。
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let after = S.with_borrow_mut(|s| {
                let st = s.as_mut()?;
                match wparam.0 as u32 {
                    0x1B => Some(After::Close), // Esc = 取消
                    0x0D => Some(After::Save),  // Enter = 保存
                    0x09 => {
                        // Tab / Shift+Tab
                        let n = TAB_ORDER.len();
                        let cur = st.focus.and_then(|f| TAB_ORDER.iter().position(|&c| c == f));
                        let next = match (cur, shift) {
                            (None, false) => 0,
                            (None, true) => n - 1,
                            (Some(i), false) => (i + 1) % n,
                            (Some(i), true) => (i + n - 1) % n,
                        };
                        st.focus = Some(TAB_ORDER[next]);
                        Some(After::Redraw)
                    }
                    0x20 => st.focus.map(|f| activate(st, f)), // Space
                    0x26 | 0x28 => {
                        // ↑ / ↓ 在三个快捷键单选之间切换
                        let n = HotkeyPreset::ALL.len();
                        st.hotkey_idx = if wparam.0 == 0x26 { (st.hotkey_idx + n - 1) % n } else { (st.hotkey_idx + 1) % n };
                        st.focus = Some(Ctl::Radio(st.hotkey_idx));
                        st.err = None;
                        Some(After::Redraw)
                    }
                    _ => None,
                }
            });
            run(after);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),
        WM_DESTROY => {
            // close() 已经把状态取走；这里兜底（例如系统直接销毁窗口），借用失败就忽略。
            S.with(|c| {
                if let Ok(mut g) = c.try_borrow_mut() {
                    if g.as_ref().map(|st| st.hwnd) == Some(hwnd) {
                        *g = None;
                    }
                }
            });
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // SAFETY: 默认窗口过程。
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
