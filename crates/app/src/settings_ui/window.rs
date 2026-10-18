//! 设置窗口的分层窗口、消息循环、命中与键盘（UI 文档 6.11）。
//!
//! 借用规则：S 每次只借一下；DestroyWindow / SetForegroundWindow / with_app / 文件对话框
//! 都在借用之外调用。渲染坐标全是 DIP（渲染目标 DPI 已设为显示器 DPI）。
use super::draw::{self, DynKind, DynRow};
use super::model::*;
use crate::app::with_app;
use crate::render::RenderCtx;
use crate::surface::Surface;
use crate::theme::SHADOW_MARGIN;
use crate::win;
use deskpad_engine::model::Settings;
use std::path::PathBuf;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

/// 命中目标（Clone 因为 Dyn 带路径字符串）。
#[derive(Clone, PartialEq, Debug)]
pub enum Ctl {
    /// 左侧导航（Section::ALL 下标）。
    Nav(usize),
    /// 分段按钮的一段。
    Seg(ItemId, usize),
    /// 开关。
    Toggle(ItemId),
    /// 行按钮（重新显示 / 检查更新 / 打开链接 / 添加…）。
    Btn(ItemId),
    /// 步进器 ±1。
    Stepper(ItemId, i32),
    /// 动态行的行内按钮（取消隐藏 / 移除）。
    Dyn(DynKind, String),
    CloseX,
}

impl Ctl {
    pub fn is_dyn_with(&self, kind: &DynKind, payload: &str) -> bool {
        matches!(self, Ctl::Dyn(k, p) if k == kind && p == payload)
    }
}

pub struct State {
    hwnd: HWND,
    dpi: u32,
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    surface: Surface,
    ctx: Option<RenderCtx>,
    pub section: Section,
    hover: Option<Ctl>,
    pressed: Option<Ctl>,
    focus: Option<Ctl>,
    err: Option<String>,
    update: crate::update_ui::UpdateState,
    scroll: f32,
    /// 拖动 Header：按下时鼠标相对窗口左上角的偏移（物理像素）。
    drag: Option<(i32, i32)>,
}

thread_local! {
    static S: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

/// 检查更新的子进程写完后由 WM_APP_UPDATE 调用：更新状态并重绘。
pub fn set_update_state(hwnd: HWND, state: crate::update_ui::UpdateState) {
    let changed = S.with_borrow_mut(|s| {
        let Some(st) = s.as_mut() else { return false };
        if st.hwnd != hwnd {
            return false;
        }
        st.update = state;
        true
    });
    if changed {
        redraw();
    }
}

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

pub fn total_px(dpi: u32) -> (i32, i32) {
    let s = dpi as f32 / 96.0;
    (
        ((W + 2.0 * SHADOW_MARGIN) * s).round() as i32,
        ((H + 2.0 * SHADOW_MARGIN) * s).round() as i32,
    )
}

#[allow(dead_code)] // 状态查询预留
pub fn is_open() -> bool {
    S.with_borrow(|s| s.is_some())
}

/// 打开设置窗口；已打开时激活已有的那个。不得在 with_app 内调用。
pub fn open(_main: HWND) -> Result<()> {
    if let Some(hwnd) = S.with_borrow(|s| s.as_ref().map(|st| st.hwnd)) {
        // SAFETY: 窗口句柄有效。
        unsafe { let _ = SetForegroundWindow(hwnd); };
        return Ok(());
    }
    // 打开设置前先收起启动台（UI 文档 6.11）
    with_app(|a| crate::pad::hide(a, crate::app::HideReason::System));
    win::register_class(w!("DeskPadSettings"), Some(settings_proc), None)?;
    let Some((factory, dwrite)) = with_app(|a| (a.factory.clone(), a.dwrite.clone())) else {
        return Ok(());
    };
    let (work, dpi) = cursor_monitor();
    let (tw, th) = total_px(dpi);
    let x = work.left + ((work.right - work.left) - tw) / 2;
    let y = work.top + ((work.bottom - work.top) - th) / 2;
    let hwnd = win::create_window(
        w!("DeskPadSettings"),
        w!("HN DeskPad 设置"),
        WS_POPUP,
        WS_EX_LAYERED | WS_EX_TOOLWINDOW,
        None,
        x,
        y,
        tw,
        th,
    )?;
    S.with_borrow_mut(|slot| {
        *slot = Some(State {
            hwnd,
            dpi,
            factory,
            dwrite,
            surface: Surface::new(),
            ctx: None,
            section: Section::General,
            hover: None,
            pressed: None,
            focus: None,
            err: None,
            update: crate::update_ui::state_from(&deskpad_engine::update::load()),
            scroll: 0.0,
            drag: None,
        })
    });
    with_app(|a| a.settings_wnd = Some(hwnd));
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
        with_app(|a| {
            if a.settings_wnd == Some(st.hwnd) {
                a.settings_wnd = None;
            }
        });
        // SAFETY: 窗口由本模块创建；WM_DESTROY 里不会再借用到已取出的状态。
        unsafe { let _ = DestroyWindow(st.hwnd); };
    }
}

// ---- 动态行（从引擎读）----

fn dyn_rows() -> Vec<DynRow> {
    let Some((settings, snap)) = super::apply::read() else { return Vec::new() };
    let mut out = Vec::new();
    for a in snap.all_apps.iter().filter(|a| a.hidden) {
        out.push(DynRow {
            kind: DynKind::HiddenApp,
            label: a.name.clone(),
            payload: a.setting_id.clone(),
            btn: "取消隐藏",
        });
    }
    for p in &settings.extra_scan_dirs {
        out.push(DynRow {
            kind: DynKind::ScanDir,
            label: p.to_string_lossy().to_string(),
            payload: p.to_string_lossy().to_string(),
            btn: "移除",
        });
    }
    for p in &settings.manual_apps {
        out.push(DynRow {
            kind: DynKind::ManualApp,
            label: p.to_string_lossy().to_string(),
            payload: p.to_string_lossy().to_string(),
            btn: "移除",
        });
    }
    out
}

fn current_settings() -> Settings {
    with_app(|a| a.eng.settings()).unwrap_or_default()
}

// ---- 命中矩形（主体坐标系，DIP；scroll 已减）----

/// 分区内容里第 n 个静态行 / 动态行的 y 起点（不含滚动）。
fn row_ys(sec: Section, dyn_rows: &[DynRow]) -> Vec<(RowTarget, f32)> {
    let mut out = Vec::new();
    let mut y = HEADER_H + 16.0 + 36.0; // 上边距 + 分区标题
    for row in &rows(sec) {
        match *row {
            Row::Group(_) => {
                y += 8.0 + GROUP_H - 14.0;
            }
            Row::Item { id, ctl, .. } => {
                out.push((RowTarget::Item(id, ctl), y));
                y += ROW_H;
                if sec == Section::Apps && id == ItemId::SortMode {
                    // 三组列表
                    for (kind, has_add) in [(DynKind::HiddenApp, false), (DynKind::ScanDir, true), (DynKind::ManualApp, true)] {
                        y += 8.0 + GROUP_H - 14.0;
                        let list: Vec<&DynRow> = dyn_rows.iter().filter(|d| d.kind == kind).collect();
                        if list.is_empty() {
                            y += 26.0;
                        } else {
                            for d in list {
                                out.push((RowTarget::Dyn(kind, d.payload.clone()), y));
                                y += 36.0;
                            }
                        }
                        if has_add {
                            let btn_id = if kind == DynKind::ScanDir { ItemId::AddScanDir } else { ItemId::AddManualApp };
                            out.push((RowTarget::AddBtn(btn_id), y));
                            y += 40.0;
                        }
                    }
                }
            }
        }
    }
    out
}

#[derive(Clone)]
enum RowTarget {
    Item(ItemId, Control),
    Dyn(DynKind, String),
    AddBtn(ItemId),
}

/// 控件在主体坐标系里的命中矩形（已减滚动）。Ctl::Nav / CloseX 不滚动。
pub fn ctl_rect(sec: Section, c: &Ctl, _settings: &Settings, dyn_rows: &[DynRow], scroll: f32) -> Option<D2D_RECT_F> {
    let r = |x: f32, y: f32, w: f32, h: f32| D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
    match c {
        Ctl::Nav(i) => Some(r(8.0, HEADER_H + 12.0 + *i as f32 * 40.0, NAV_W - 16.0, 36.0)),
        Ctl::CloseX => Some(r(W - 40.0, 12.0, 24.0, 24.0)),
        _ => {
            let ys = row_ys(sec, dyn_rows);
            let right_x = CONTENT_X + CONTENT_W;
            match c {
                Ctl::Seg(id, idx) => {
                    let (RowTarget::Item(_, Control::Seg(labels)), y) = ys.iter().find(|t| matches!(&t.0, RowTarget::Item(i, _) if i == id))?.clone() else { return None };
                    let (w, h) = draw::seg_size(labels);
                    let seg_w = w / labels.len() as f32;
                    Some(r(right_x - w + *idx as f32 * seg_w, y + (ROW_H - h) / 2.0 - scroll, seg_w, h))
                }
                Ctl::Toggle(id) => {
                    let (_, y) = ys.iter().find(|t| matches!(&t.0, RowTarget::Item(i, _) if i == id))?.clone();
                    Some(r(right_x - 44.0, y + (ROW_H - 20.0) / 2.0 - scroll, 36.0, 20.0))
                }
                Ctl::Btn(id) => {
                    if *id == ItemId::AddScanDir || *id == ItemId::AddManualApp {
                        let (_, y) = ys.iter().find(|t| matches!(&t.0, RowTarget::AddBtn(i) if i == id))?.clone();
                        return Some(r(CONTENT_X, y + 2.0 - scroll, 120.0, 28.0));
                    }
                    let (RowTarget::Item(_, Control::Btn(label)), y) = ys.iter().find(|t| matches!(&t.0, RowTarget::Item(i, _) if i == id))?.clone() else { return None };
                    let w = (label.chars().count() as f32 * 13.0 + 28.0).max(96.0);
                    Some(r(right_x - w, y + (ROW_H - 30.0) / 2.0 - scroll, w, 30.0))
                }
                Ctl::Stepper(id, delta) => {
                    let (_, y) = ys.iter().find(|t| matches!(&t.0, RowTarget::Item(i, _) if i == id))?.clone();
                    let ox = if *delta < 0 { 0.0 } else { 78.0 };
                    Some(r(right_x - 108.0 + ox, y + (ROW_H - 30.0) / 2.0 - scroll, 30.0, 30.0))
                }
                Ctl::Dyn(kind, payload) => {
                    let (_, y) = ys.iter().find(|t| matches!(&t.0, RowTarget::Dyn(k, p) if k == kind && p == payload))?.clone();
                    Some(r(right_x - 96.0, y + 2.0 - scroll, 88.0, 28.0))
                }
                Ctl::Nav(_) | Ctl::CloseX => None,
            }
        }
    }
}

/// 当前分区里可 Tab 的控件（顺序 = 视觉顺序）。
fn tab_order(sec: Section, dyn_rows: &[DynRow]) -> Vec<Ctl> {
    let mut out = Vec::new();
    for row in &rows(sec) {
        if let Row::Item { id, ctl, .. } = *row {
            match ctl {
                Control::Seg(labels) => {
                    for i in 0..labels.len() {
                        out.push(Ctl::Seg(id, i));
                    }
                }
                Control::Toggle => out.push(Ctl::Toggle(id)),
                Control::Btn(_) => out.push(Ctl::Btn(id)),
                Control::Stepper => {
                    out.push(Ctl::Stepper(id, -1));
                    out.push(Ctl::Stepper(id, 1));
                }
                Control::Text => {}
            }
        }
    }
    if sec == Section::Apps {
        for d in dyn_rows {
            out.push(Ctl::Dyn(d.kind, d.payload.clone()));
        }
        out.push(Ctl::Btn(ItemId::AddScanDir));
        out.push(Ctl::Btn(ItemId::AddManualApp));
    }
    out
}

fn ctl_at(st: &State, sec: Section, settings: &Settings, dyn_rows: &[DynRow], x: f32, y: f32) -> Option<Ctl> {
    // 关闭按钮与导航优先（不滚动）
    for c in [Ctl::CloseX].into_iter().chain(Section::ALL.iter().enumerate().map(|(i, _)| Ctl::Nav(i))) {
        if let Some(r) = ctl_rect(sec, &c, settings, dyn_rows, st.scroll) {
            if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                return Some(c);
            }
        }
    }
    // 内容区控件
    for c in tab_order(sec, dyn_rows) {
        if let Some(r) = ctl_rect(sec, &c, settings, dyn_rows, st.scroll) {
            // 只命中内容区可视范围内的
            if r.bottom < HEADER_H || r.top > H {
                continue;
            }
            if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                return Some(c);
            }
        }
    }
    None
}

// ---- 动作（借用之外执行）----

enum After {
    Redraw,
    Close,
    CheckUpdate,
    ApplyUpdate,
    OpenReleases,
    OpenLogs,
    PickFolder,
    PickApp,
    Unhide(String),
    RemoveScanDir(String),
    RemoveManualApp(String),
    ReplayOnboard,
}

fn activate(st: &State, c: &Ctl, settings: &Settings) -> Option<After> {
    match c {
        Ctl::Nav(i) => {
            let sec = Section::ALL[(*i).min(4)];
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.section = sec;
                    st.scroll = 0.0;
                    st.err = None;
                }
            });
            Some(After::Redraw)
        }
        Ctl::Seg(id, idx) => {
            let r = super::apply::apply(*id, *idx, 0);
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = r.err();
                }
            });
            Some(After::Redraw)
        }
        Ctl::Toggle(id) => {
            let r = super::apply::apply(*id, 0, 0);
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = r.err();
                }
            });
            Some(After::Redraw)
        }
        Ctl::Stepper(id, delta) => {
            // 边界处不再 apply
            let cur = settings.max_recent as i32;
            let next = cur + delta;
            if !(1..=20).contains(&next) {
                return Some(After::Redraw);
            }
            let r = super::apply::apply(*id, 0, *delta);
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = r.err();
                }
            });
            Some(After::Redraw)
        }
        Ctl::Btn(id) => match id {
            ItemId::ReplayOnboard => Some(After::ReplayOnboard),
            ItemId::CheckUpdate => {
                if st.update.has_new() {
                    Some(After::ApplyUpdate)
                } else if matches!(st.update, crate::update_ui::UpdateState::Checking) {
                    Some(After::Redraw)
                } else {
                    Some(After::CheckUpdate)
                }
            }
            ItemId::Releases => Some(After::OpenReleases),
            ItemId::OpenLogs => Some(After::OpenLogs),
            ItemId::AddScanDir => Some(After::PickFolder),
            ItemId::AddManualApp => Some(After::PickApp),
            _ => Some(After::Redraw),
        },
        Ctl::Dyn(kind, payload) => match kind {
            DynKind::HiddenApp => Some(After::Unhide(payload.clone())),
            DynKind::ScanDir => Some(After::RemoveScanDir(payload.clone())),
            DynKind::ManualApp => Some(After::RemoveManualApp(payload.clone())),
        },
        Ctl::CloseX => Some(After::Close),
    }
}

fn run(after: Option<After>) {
    match after {
        Some(After::Redraw) => redraw(),
        Some(After::Close) => close(),
        Some(After::CheckUpdate) => {
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.update = crate::update_ui::UpdateState::Checking;
                }
            });
            redraw();
            crate::update_ui::check_now();
        }
        Some(After::ApplyUpdate) => {
            crate::update_ui::apply_update();
            close();
        }
        Some(After::OpenReleases) => open_url(deskpad_engine::update::RELEASES_URL),
        Some(After::OpenLogs) => open_log_folder(),
        Some(After::PickFolder) => {
            if let Some(p) = pick_folder() {
                let r = super::apply::add_scan_dir(&p);
                S.with_borrow_mut(|s| {
                    if let Some(st) = s.as_mut() {
                        st.err = r.err();
                    }
                });
                redraw();
            }
        }
        Some(After::PickApp) => {
            if let Some(p) = pick_app() {
                let r = super::apply::add_manual_app(&p);
                S.with_borrow_mut(|s| {
                    if let Some(st) = s.as_mut() {
                        st.err = r.err();
                    }
                });
                redraw();
            }
        }
        Some(After::Unhide(id)) => {
            let r = super::apply::unhide(&id);
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = r.err();
                }
            });
            redraw();
        }
        Some(After::RemoveScanDir(p)) => {
            let r = super::apply::remove_scan_dir(std::path::Path::new(&p));
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = r.err();
                }
            });
            redraw();
        }
        Some(After::RemoveManualApp(p)) => {
            let r = super::apply::remove_manual_app(std::path::Path::new(&p));
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = r.err();
                }
            });
            redraw();
        }
        Some(After::ReplayOnboard) => {
            let r = super::apply::apply(ItemId::ReplayOnboard, 0, 0);
            S.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.err = r.err();
                }
            });
            redraw();
        }
        None => {}
    }
}

/// 打开日志文件夹（%LocalAppData%\DeskPad，config.json 的兄弟目录）。
fn open_log_folder() {
    with_app(|a| {
        let dir = deskpad_engine::update::data_dir();
        let _ = a.eng.open_path(&dir);
    });
}

fn open_url(url: &str) {
    // SAFETY: 字符串在调用期间有效。
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            &windows::core::HSTRING::from(url),
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 系统文件夹选择框（IFileOpenDialog，FOS_PICKFOLDERS）。
fn pick_folder() -> Option<PathBuf> {
    pick_shell(true)
}

/// 系统文件选择框（IFileOpenDialog，过滤 exe / lnk）。
fn pick_app() -> Option<PathBuf> {
    pick_shell(false)
}

fn app_filter_spec() -> &'static str {
    "*.exe;*.lnk;*.url"
}

fn pick_shell(folder: bool) -> Option<PathBuf> {
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH, FOS_FILEMUSTEXIST, FOS_PICKFOLDERS,
    };
    // SAFETY: 标准 COM 对话框；当前线程已 STA。
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL).ok()?;
        let mut opts = dlg.GetOptions().ok()?;
        opts |= if folder { FOS_PICKFOLDERS } else { FOS_FILEMUSTEXIST };
        dlg.SetOptions(opts).ok()?;
        if !folder {
            let name = win::to_wide("应用程序");
            let spec = win::to_wide(app_filter_spec());
            let filters = [COMDLG_FILTERSPEC {
                pszName: windows::core::PCWSTR(name.as_ptr()),
                pszSpec: windows::core::PCWSTR(spec.as_ptr()),
            }];
            let _ = dlg.SetFileTypes(&filters);
        }
        let owner = S.with_borrow(|s| s.as_ref().map(|st| st.hwnd)).unwrap_or_default();
        if dlg.Show(owner).is_err() {
            return None; // 用户取消
        }
        let item = dlg.GetResult().ok()?;
        let pw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = pw.to_string().ok()?;
        CoTaskMemFree(Some(pw.0 as _));
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    }
}

// ---- 重绘 ----

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
        draw_frame(st)?;
        let mut wr = RECT::default();
        // SAFETY: 输出参数。
        unsafe { let _ = GetWindowRect(st.hwnd, &mut wr); };
        st.surface.present(st.hwnd, POINT { x: wr.left, y: wr.top }, 255)
    });
    if let Err(e) = r {
        deskpad_engine::log::warn(&format!("settings redraw: {e}"));
    }
}

fn draw_frame(st: &mut State) -> Result<()> {
    let Some(ctx) = st.ctx.as_mut() else { return Ok(()) };
    let rt = ctx.rt.clone();
    // SAFETY: 绘制序列配对。
    unsafe {
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
    }
    let settings = current_settings();
    let dyn_rows = dyn_rows();
    let content_h = draw::content_height(st.section, &dyn_rows);
    let view = draw::View {
        section: st.section,
        hover: st.hover.clone(),
        pressed: st.pressed.clone(),
        focus: st.focus.clone(),
        settings: &settings,
        update: &st.update,
        err: st.err.as_deref(),
        dyn_rows: &dyn_rows,
        scroll: st.scroll,
        content_h,
    };
    let r = draw::frame(ctx, &st.factory, &view);
    // SAFETY: BeginDraw 已调用。
    let end = unsafe { rt.EndDraw(None, None) };
    r.and(end)
}

/// 客户区物理像素 → 主体 DIP 坐标。
fn to_body(st: &State, lparam: LPARAM) -> (f32, f32) {
    let s = st.dpi as f32 / 96.0;
    (
        win::get_x_lparam(lparam) as f32 / s - SHADOW_MARGIN,
        win::get_y_lparam(lparam) as f32 / s - SHADOW_MARGIN,
    )
}

fn clamp_scroll(st: &mut State, dyn_rows: &[DynRow]) {
    let view_h = H - HEADER_H - 1.0;
    let max = (draw::content_height(st.section, dyn_rows) - view_h).max(0.0);
    st.scroll = st.scroll.clamp(0.0, max);
}

pub unsafe extern "system" fn settings_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let drag = S.with_borrow(|s| s.as_ref().and_then(|st| st.drag));
            if let Some((ox, oy)) = drag {
                let mut pt = POINT::default();
                // SAFETY: 输出参数；移动自己的窗口。
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                    let _ = SetWindowPos(hwnd, HWND::default(), pt.x - ox, pt.y - oy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
                return LRESULT(0);
            }
            let changed = S.with_borrow_mut(|s| {
                let Some(st) = s.as_mut() else { return false };
                let (x, y) = to_body(st, lparam);
                let settings = current_settings();
                let dr = dyn_rows();
                let h = ctl_at(st, st.section, &settings, &dr, x, y);
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
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as u16 as i16 as i32;
            let changed = S.with_borrow_mut(|s| {
                let Some(st) = s.as_mut() else { return false };
                let dr = dyn_rows();
                st.scroll -= delta as f32 / 120.0 * 48.0;
                clamp_scroll(st, &dr);
                true
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
                let settings = current_settings();
                let dr = dyn_rows();
                st.pressed = ctl_at(st, st.section, &settings, &dr, x, y);
                if st.pressed.is_some() {
                    st.focus = st.pressed.clone().filter(|c| *c != Ctl::CloseX);
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
            unsafe { let _ = SetCapture(hwnd); };
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
                let settings = current_settings();
                let dr = dyn_rows();
                let pressed = st.pressed.take();
                match (pressed, ctl_at(st, st.section, &settings, &dr, x, y)) {
                    (Some(p), Some(h)) if p == h => activate(st, &p, &settings),
                    (Some(_), _) => Some(After::Redraw),
                    _ => None,
                }
            });
            // SAFETY: 与 SetCapture 配对。
            unsafe { let _ = ReleaseCapture(); };
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
                let settings = current_settings();
                let dr = dyn_rows();
                match wparam.0 as u32 {
                    0x1B => Some(After::Close), // Esc
                    0x09 => {
                        // Tab / Shift+Tab
                        let order = tab_order(st.section, &dr);
                        if order.is_empty() {
                            return Some(After::Redraw);
                        }
                        let cur = st.focus.as_ref().and_then(|f| order.iter().position(|c| c == f));
                        let next = match (cur, shift) {
                            (None, false) => 0,
                            (None, true) => order.len() - 1,
                            (Some(i), false) => (i + 1) % order.len(),
                            (Some(i), true) => (i + order.len() - 1) % order.len(),
                        };
                        st.focus = Some(order[next].clone());
                        // 确保聚焦控件可见
                        if let Some(r) = ctl_rect(st.section, &order[next], &settings, &dr, st.scroll) {
                            if r.top < HEADER_H + 8.0 {
                                st.scroll -= HEADER_H + 8.0 - r.top;
                            } else if r.bottom > H - 8.0 {
                                st.scroll += r.bottom - (H - 8.0);
                            }
                            clamp_scroll(st, &dr);
                        }
                        Some(After::Redraw)
                    }
                    0x20 | 0x0D => {
                        // Space / Enter 触发聚焦控件
                        st.focus.clone().and_then(|f| activate(st, &f, &settings))
                    }
                    0x26 | 0x28 => {
                        // ↑/↓ 切分区
                        let n = Section::ALL.len();
                        let cur = st.section.index();
                        let next = if wparam.0 == 0x26 { (cur + n - 1) % n } else { (cur + 1) % n };
                        st.section = Section::ALL[next];
                        st.scroll = 0.0;
                        st.err = None;
                        st.focus = None;
                        Some(After::Redraw)
                    }
                    0x25 | 0x27 => {
                        // ←/→ 在分段按钮内移动
                        if let Some(Ctl::Seg(id, idx)) = st.focus.clone() {
                            if let Some(labels) = rows(st.section).iter().find_map(|r| match r {
                                Row::Item { id: i, ctl: Control::Seg(l), .. } if *i == id => Some(*l),
                                _ => None,
                            }) {
                                let n = labels.len();
                                let ni = if wparam.0 == 0x25 { (idx + n - 1) % n } else { (idx + 1) % n };
                                st.focus = Some(Ctl::Seg(id, ni));
                                return activate(st, &Ctl::Seg(id, ni), &settings);
                            }
                        }
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

// ---- 无头截图（--render-test --settings <分区>）----

/// 离屏绘制一个分区，返回 PNG 字节。scale 一般用 1.5（150%）。
/// 需要 App 已初始化（引擎 / 工厂在）；不创建任何窗口。
pub fn render_shot(section: Section, scale: f32) -> Result<Vec<u8>> {
    let Some((factory, dwrite)) = with_app(|a| (a.factory.clone(), a.dwrite.clone())) else {
        return Err(windows::core::Error::from(E_FAIL));
    };
    let dpi = 96.0 * scale;
    let (tw, th) = (
        ((W + 2.0 * SHADOW_MARGIN) * scale).round() as i32,
        ((H + 2.0 * SHADOW_MARGIN) * scale).round() as i32,
    );
    let mut surface = Surface::new();
    surface.ensure(&factory, tw, th, dpi)?;
    let Some(rt) = surface.rt().cloned() else { return Err(windows::core::Error::from(E_FAIL)) };
    let mut ctx = RenderCtx::new(&factory, &dwrite, rt)?;
    let settings = current_settings();
    let dr = dyn_rows();
    let update = crate::update_ui::state_from(&deskpad_engine::update::load());
    let view = draw::View {
        section,
        hover: None,
        pressed: None,
        focus: None,
        settings: &settings,
        update: &update,
        err: None,
        dyn_rows: &dr,
        scroll: 0.0,
        content_h: draw::content_height(section, &dr),
    };
    // SAFETY: 绘制序列配对。
    unsafe {
        ctx.rt.BeginDraw();
        ctx.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
    }
    let r = draw::frame(&mut ctx, &factory, &view);
    // SAFETY: BeginDraw 已调用。
    let end = unsafe { ctx.rt.EndDraw(None, None) };
    r.and(end)?;
    let Some(px) = surface.pixels() else { return Err(windows::core::Error::from(E_FAIL)) };
    Ok(super::png::encode(tw as u32, th as u32, &px))
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn manual_app_picker_accepts_dp04b_extensions() {
        assert_eq!(app_filter_spec(), "*.exe;*.lnk;*.url");
    }
}
