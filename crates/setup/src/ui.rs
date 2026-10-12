//! 安装界面：单窗口，D2D 自绘，tofour 设计 Token（与 DeskPad 主程序 theme.rs 一致）。
use crate::install;
use crate::net::{self, Release};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ---- tofour Token ----
const PRIMARY: u32 = 0xFF3878D8;
const PRIMARY_HOVER: u32 = 0xFF4A89E5;
const PRIMARY_PRESSED: u32 = 0xFF2E68C2;
const SURFACE: u32 = 0xFFFAFAFA;
const INPUT: u32 = 0xFFFFFFFF;
const TEXT_PRIMARY: u32 = 0xFF222222;
const TEXT_SECONDARY: u32 = 0xFF888888;
const TEXT_TERTIARY: u32 = 0xFFA3A3A3;
const BORDER: u32 = 0xFFE5E5E5;
const BORDER_HOVER: u32 = 0xFFD9D9D9;
const ICON: u32 = 0xFF65758B;
const ROW_HOVER: u32 = 0xFFF7F7F7;
const DISABLED: u32 = 0xFFB7CCEB;
const DANGER: u32 = 0xFFD95C5C;
const OK_BG: u32 = 0xFFEAF7ED;
const OK_FG: u32 = 0xFF4DAA5A;
const WHITE: u32 = 0xFFFFFFFF;
const RADIUS: f32 = 6.0;
const FONT: PCWSTR = w!("Microsoft YaHei UI");

// ---- 布局（DIP）----
const W: f32 = 520.0;
const H: f32 = 372.0;
const M: f32 = 32.0;
const FIELD: (f32, f32, f32, f32) = (M, 128.0, 400.0, 164.0);
const CHANGE: (f32, f32, f32, f32) = (412.0, 128.0, W - M, 164.0);
const CHECK_Y: f32 = 214.0;
const PRIMARY_BTN: (f32, f32, f32, f32) = (W - M - 120.0, H - 32.0 - 36.0, W - M, H - 32.0);

const WM_APP_PROGRESS: u32 = WM_APP + 1;
const WM_APP_LATEST: u32 = WM_APP + 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Field,
    Change,
    Check,
    Primary,
}

#[derive(Clone, PartialEq)]
enum Phase {
    Ready,
    Working,
    Done(PathBuf),
    Failed(String),
}

#[derive(Default)]
struct Progress {
    stage: String,
    done: u64,
    total: u64,
    finished: Option<Result<PathBuf, String>>,
}

static PROGRESS: Mutex<Option<Progress>> = Mutex::new(None);
static LATEST: Mutex<Option<Result<Release, String>>> = Mutex::new(None);

struct Ui {
    hwnd: HWND,
    factory: ID2D1Factory,
    rt: Option<ID2D1HwndRenderTarget>,
    brushes: HashMap<u32, ID2D1Brush>,
    f_title: IDWriteTextFormat,
    f_body: IDWriteTextFormat,
    f_btn: IDWriteTextFormat,
    f_cap: IDWriteTextFormat,
    f_cap_c: IDWriteTextFormat,
    f_wrap: IDWriteTextFormat,
    f_done: IDWriteTextFormat,
    dpi: u32,
    parent: PathBuf,
    /// Ready：创建桌面快捷方式；Done：完成后启动。
    check_desktop: bool,
    check_launch: bool,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    tracking: bool,
    phase: Phase,
    installed: Option<String>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

pub fn run() {
    // SAFETY: 进程级 DPI 声明（清单里也有）；失败可忽略。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let Ok(ui) = init() else {
        // SAFETY: 常量字符串。
        unsafe { MessageBoxW(None, w!("界面初始化失败。"), w!("HN DeskPad 安装"), MB_ICONERROR) };
        return;
    };
    let hwnd = ui.hwnd;
    UI.with(|u| *u.borrow_mut() = Some(ui));
    // 后台读取最新版本号（显示在标题下方）
    let h = hwnd.0 as isize;
    std::thread::spawn(move || {
        let r = net::latest();
        *LATEST.lock().unwrap() = Some(r);
        // SAFETY: 窗口可能已关闭，PostMessage 对无效句柄只返回错误。
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), WM_APP_LATEST, WPARAM(0), LPARAM(0));
        }
    });
    // SAFETY: 标准消息循环。
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn make_format(dw: &IDWriteFactory, size: f32, weight: DWRITE_FONT_WEIGHT, center: bool, wrap: bool) -> windows::core::Result<IDWriteTextFormat> {
    // SAFETY: 字体名为静态字面量。
    unsafe {
        let f = dw.CreateTextFormat(FONT, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("zh-CN"))?;
        let _ = f.SetTextAlignment(if center { DWRITE_TEXT_ALIGNMENT_CENTER } else { DWRITE_TEXT_ALIGNMENT_LEADING });
        let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        if !wrap {
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            if let Ok(sign) = dw.CreateEllipsisTrimmingSign(&f) {
                let _ = f.SetTrimming(
                    &DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 },
                    &sign,
                );
            }
        }
        Ok(f)
    }
}

fn init() -> windows::core::Result<Ui> {
    // SAFETY: 标准工厂创建 / 窗口类注册 / 窗口创建。
    unsafe {
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: w!("HNDeskPadSetup"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        // 按光标所在显示器 DPI 计算窗口大小，居中
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let dpi = dx.max(96);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = RECT { left: 0, top: 0, right: px(W, dpi), bottom: px(H, dpi) };
        let _ = AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        let x = wa.left + ((wa.right - wa.left) - ww) / 2;
        let y = wa.top + ((wa.bottom - wa.top) - wh) / 2;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("HNDeskPadSetup"),
            w!("HN DeskPad 安装"),
            style,
            x,
            y,
            ww,
            wh,
            None,
            None,
            hinst,
            None,
        )?;
        let installed = install::installed_version();
        Ok(Ui {
            hwnd,
            f_title: make_format(&dwrite, 20.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, false, false)?,
            f_body: make_format(&dwrite, 14.0, DWRITE_FONT_WEIGHT_NORMAL, false, false)?,
            f_btn: make_format(&dwrite, 14.0, DWRITE_FONT_WEIGHT_NORMAL, true, false)?,
            f_cap: make_format(&dwrite, 12.0, DWRITE_FONT_WEIGHT_NORMAL, false, false)?,
            f_cap_c: make_format(&dwrite, 12.0, DWRITE_FONT_WEIGHT_NORMAL, true, false)?,
            f_wrap: make_format(&dwrite, 12.0, DWRITE_FONT_WEIGHT_NORMAL, false, true)?,
            f_done: make_format(&dwrite, 16.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, true, false)?,
            factory,
            rt: None,
            brushes: HashMap::new(),
            dpi: GetDpiForWindow(hwnd).max(96),
            parent: install::default_parent(),
            check_desktop: true,
            check_launch: true,
            hover: None,
            pressed: None,
            tracking: false,
            phase: Phase::Ready,
            installed,
        })
    }
}

fn px(dip: f32, dpi: u32) -> i32 {
    (dip * dpi as f32 / 96.0).round() as i32
}

fn rect(r: (f32, f32, f32, f32)) -> D2D_RECT_F {
    D2D_RECT_F { left: r.0, top: r.1, right: r.2, bottom: r.3 }
}

fn inside(r: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
    x >= r.0 && x < r.2 && y >= r.1 && y < r.3
}

fn check_rect() -> (f32, f32, f32, f32) {
    (M, CHECK_Y, M + 200.0, CHECK_Y + 24.0)
}

impl Ui {
    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        match self.phase {
            Phase::Working => None,
            Phase::Done(_) => {
                if inside(check_rect(), x, y) {
                    Some(Hit::Check)
                } else if inside(PRIMARY_BTN, x, y) {
                    Some(Hit::Primary)
                } else {
                    None
                }
            }
            Phase::Ready | Phase::Failed(_) => {
                if inside(FIELD, x, y) {
                    Some(Hit::Field)
                } else if inside(CHANGE, x, y) {
                    Some(Hit::Change)
                } else if inside(check_rect(), x, y) {
                    Some(Hit::Check)
                } else if inside(PRIMARY_BTN, x, y) {
                    Some(Hit::Primary)
                } else {
                    None
                }
            }
        }
    }

    fn target(&self) -> PathBuf {
        install::target_dir(&self.parent)
    }

    fn brush(&mut self, c: u32) -> windows::core::Result<ID2D1Brush> {
        if let Some(b) = self.brushes.get(&c) {
            return Ok(b.clone());
        }
        let rt = self.rt.as_ref().ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let col = D2D1_COLOR_F {
            a: ((c >> 24) & 0xFF) as f32 / 255.0,
            r: ((c >> 16) & 0xFF) as f32 / 255.0,
            g: ((c >> 8) & 0xFF) as f32 / 255.0,
            b: (c & 0xFF) as f32 / 255.0,
        };
        // SAFETY: rt 有效。
        let b: ID2D1Brush = unsafe { rt.CreateSolidColorBrush(&col, None)? }.cast()?;
        self.brushes.insert(c, b.clone());
        Ok(b)
    }

    fn ensure_rt(&mut self) -> windows::core::Result<()> {
        if self.rt.is_some() {
            return Ok(());
        }
        let mut rc = RECT::default();
        // SAFETY: hwnd 有效。
        unsafe { GetClientRect(self.hwnd, &mut rc)? };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: self.dpi as f32,
            dpiY: self.dpi as f32,
            ..Default::default()
        };
        let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.hwnd,
            pixelSize: D2D_SIZE_U { width: (rc.right - rc.left) as u32, height: (rc.bottom - rc.top) as u32 },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        // SAFETY: 属性为值。
        let rt = unsafe { self.factory.CreateHwndRenderTarget(&props, &hp)? };
        // SAFETY: rt 有效。
        unsafe { rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
        self.rt = Some(rt);
        self.brushes.clear();
        Ok(())
    }

    fn text(&mut self, fmt: &IDWriteTextFormat, s: &str, r: D2D_RECT_F, c: u32) -> windows::core::Result<()> {
        let b = self.brush(c)?;
        let w: Vec<u16> = s.encode_utf16().collect();
        if let Some(rt) = &self.rt {
            // SAFETY: 字符串在调用期间有效。
            unsafe { rt.DrawText(&w, fmt, &r, &b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL) };
        }
        Ok(())
    }

    fn rrect(&mut self, r: D2D_RECT_F, rad: f32, fill: Option<u32>, stroke: Option<u32>) -> windows::core::Result<()> {
        let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: rad, radiusY: rad };
        let fb = fill.map(|c| self.brush(c)).transpose()?;
        let sb = stroke.map(|c| self.brush(c)).transpose()?;
        if let Some(rt) = &self.rt {
            // SAFETY: 画刷有效。
            unsafe {
                if let Some(b) = fb {
                    rt.FillRoundedRectangle(&rr, &b);
                }
                if let Some(b) = sb {
                    let inset = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F { left: r.left + 0.5, top: r.top + 0.5, right: r.right - 0.5, bottom: r.bottom - 0.5 },
                        radiusX: rad,
                        radiusY: rad,
                    };
                    rt.DrawRoundedRectangle(&inset, &b, 1.0, None);
                }
            }
        }
        Ok(())
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), c: u32, wd: f32) -> windows::core::Result<()> {
        let br = self.brush(c)?;
        if let Some(rt) = &self.rt {
            let props = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                ..Default::default()
            };
            // SAFETY: 画刷有效；线型临时创建。
            unsafe {
                let ss = self.factory.CreateStrokeStyle(&props, None).ok();
                rt.DrawLine(D2D_POINT_2F { x: a.0, y: a.1 }, D2D_POINT_2F { x: b.0, y: b.1 }, &br, wd, ss.as_ref());
            }
        }
        Ok(())
    }

    fn checkbox(&mut self, on: bool, label: &str, enabled: bool) -> windows::core::Result<()> {
        let y = CHECK_Y + 4.0;
        let bx = D2D_RECT_F { left: M, top: y, right: M + 16.0, bottom: y + 16.0 };
        let hot = self.hover == Some(Hit::Check) && enabled;
        if on {
            self.rrect(bx, 4.0, Some(if hot { PRIMARY_HOVER } else { PRIMARY }), None)?;
            self.line((M + 4.0, y + 8.5), (M + 7.0, y + 11.5), WHITE, 1.6)?;
            self.line((M + 7.0, y + 11.5), (M + 12.5, y + 5.0), WHITE, 1.6)?;
        } else {
            self.rrect(bx, 4.0, Some(INPUT), Some(if hot { ICON } else { BORDER_HOVER }))?;
        }
        let f = self.f_body.clone();
        self.text(&f, label, D2D_RECT_F { left: M + 26.0, top: CHECK_Y, right: M + 260.0, bottom: CHECK_Y + 24.0 }, if enabled { TEXT_PRIMARY } else { TEXT_TERTIARY })
    }

    fn primary(&mut self, label: &str, enabled: bool) -> windows::core::Result<()> {
        let c = if !enabled {
            DISABLED
        } else if self.pressed == Some(Hit::Primary) {
            PRIMARY_PRESSED
        } else if self.hover == Some(Hit::Primary) {
            PRIMARY_HOVER
        } else {
            PRIMARY
        };
        self.rrect(rect(PRIMARY_BTN), RADIUS, Some(c), None)?;
        let f = self.f_btn.clone();
        self.text(&f, label, rect(PRIMARY_BTN), WHITE)
    }

    fn paint(&mut self) -> windows::core::Result<()> {
        self.ensure_rt()?;
        let rt = self.rt.clone().unwrap();
        // SAFETY: BeginDraw/EndDraw 成对。
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F { r: ((SURFACE >> 16) & 0xFF) as f32 / 255.0, g: ((SURFACE >> 8) & 0xFF) as f32 / 255.0, b: (SURFACE & 0xFF) as f32 / 255.0, a: 1.0 }));
        }
        let r = self.draw();
        // SAFETY: 同上。
        let end = unsafe { rt.EndDraw(None, None) };
        if let Err(e) = &end {
            if e.code() == D2DERR_RECREATE_TARGET {
                self.rt = None;
                self.brushes.clear();
            }
        }
        r.and(end)
    }

    fn draw(&mut self) -> windows::core::Result<()> {
        // 头部：产品标记（3×3 网格的线性图形）+ 名称 + 版本
        self.rrect(D2D_RECT_F { left: M, top: 32.0, right: M + 44.0, bottom: 76.0 }, 10.0, Some(INPUT), Some(BORDER))?;
        for i in 0..3 {
            for j in 0..3 {
                let x = M + 11.0 + i as f32 * 8.5;
                let y = 43.0 + j as f32 * 8.5;
                self.rrect(D2D_RECT_F { left: x, top: y, right: x + 5.0, bottom: y + 5.0 }, 1.5, Some(ICON), None)?;
            }
        }
        let f = self.f_title.clone();
        self.text(&f, "HN DeskPad", D2D_RECT_F { left: M + 60.0, top: 30.0, right: W - M, bottom: 56.0 }, TEXT_PRIMARY)?;
        let latest = LATEST.lock().unwrap().clone();
        let mut sub = String::from("Windows 启动台");
        match &latest {
            None => sub.push_str(" · 正在获取最新版本…"),
            Some(Ok(r)) => sub.push_str(&format!(" · 最新版本 {}", r.version)),
            Some(Err(_)) => sub.push_str(" · 暂时无法获取版本信息"),
        }
        if let Some(v) = &self.installed {
            sub.push_str(&format!(" · 已安装 {v}"));
        }
        let f = self.f_cap.clone();
        self.text(&f, &sub, D2D_RECT_F { left: M + 60.0, top: 56.0, right: W - M, bottom: 76.0 }, TEXT_SECONDARY)?;
        // 分隔线
        self.rrect(D2D_RECT_F { left: 0.0, top: 100.0, right: W, bottom: 101.0 }, 0.0, Some(BORDER), None)?;

        let phase = self.phase.clone();
        match phase {
            Phase::Done(dir) => self.draw_done(&dir),
            _ => self.draw_form(&phase),
        }
    }

    fn draw_form(&mut self, phase: &Phase) -> windows::core::Result<()> {
        let working = *phase == Phase::Working;
        let f = self.f_cap.clone();
        self.text(&f, "安装位置", D2D_RECT_F { left: M, top: 104.0, right: W - M, bottom: 124.0 }, TEXT_SECONDARY)?;
        // 路径框（点击也能选择）
        let field_border = if self.hover == Some(Hit::Field) && !working { BORDER_HOVER } else { BORDER };
        self.rrect(rect(FIELD), RADIUS, Some(INPUT), Some(field_border))?;
        let f = self.f_body.clone();
        let p = self.parent.to_string_lossy().to_string();
        self.text(&f, &p, D2D_RECT_F { left: FIELD.0 + 12.0, top: FIELD.1, right: FIELD.2 - 12.0, bottom: FIELD.3 }, if working { TEXT_TERTIARY } else { TEXT_PRIMARY })?;
        // 更改… 按钮（次要）
        let ch_fill = if self.hover == Some(Hit::Change) && !working { ROW_HOVER } else { INPUT };
        self.rrect(rect(CHANGE), RADIUS, Some(ch_fill), Some(BORDER))?;
        let f = self.f_btn.clone();
        self.text(&f, "更改…", rect(CHANGE), if working { TEXT_TERTIARY } else { TEXT_PRIMARY })?;
        // 实际安装目录（自动创建的文件夹）
        let target = self.target();
        let exists = target.join(install::EXE).exists();
        let hint = if exists {
            format!("将更新 {}", target.display())
        } else {
            format!("将创建文件夹并安装到 {}", target.display())
        };
        let f = self.f_cap.clone();
        self.text(&f, &hint, D2D_RECT_F { left: M, top: 170.0, right: W - M, bottom: 190.0 }, TEXT_TERTIARY)?;
        let on = self.check_desktop;
        self.checkbox(on, "创建桌面快捷方式", !working)?;

        // 底部：进度 / 错误 / 按钮
        let status_r = D2D_RECT_F { left: M, top: PRIMARY_BTN.1, right: PRIMARY_BTN.0 - 16.0, bottom: PRIMARY_BTN.3 };
        match phase {
            Phase::Working => {
                let (stage, done, total) = PROGRESS
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|p| (p.stage.clone(), p.done, p.total))
                    .unwrap_or_default();
                let bar_y = PRIMARY_BTN.1 - 20.0;
                self.rrect(D2D_RECT_F { left: M, top: bar_y, right: W - M, bottom: bar_y + 4.0 }, 2.0, Some(BORDER), None)?;
                if total > 0 {
                    let t = (done as f32 / total as f32).clamp(0.0, 1.0);
                    let right = M + (W - 2.0 * M) * t;
                    if right > M + 4.0 {
                        self.rrect(D2D_RECT_F { left: M, top: bar_y, right, bottom: bar_y + 4.0 }, 2.0, Some(PRIMARY), None)?;
                    }
                }
                let f = self.f_cap.clone();
                self.text(&f, &stage, status_r, TEXT_SECONDARY)?;
                self.primary("正在安装…", false)?;
            }
            Phase::Failed(msg) => {
                let f = self.f_wrap.clone();
                self.text(&f, msg, status_r, DANGER)?;
                self.primary("重试", true)?;
            }
            _ => {
                let f = self.f_cap.clone();
                self.text(&f, "安装到当前用户，不需要管理员权限", status_r, TEXT_TERTIARY)?;
                let label = if self.installed.is_some() || exists { "更新" } else { "安装" };
                self.primary(label, true)?;
            }
        }
        Ok(())
    }

    fn draw_done(&mut self, dir: &Path) -> windows::core::Result<()> {
        // 成功标记：tofour「已完成」标签配色
        let (cx, cy) = (W / 2.0, 136.0);
        let b = self.brush(OK_BG)?;
        if let Some(rt) = &self.rt {
            // SAFETY: 画刷有效。
            unsafe { rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: 20.0, radiusY: 20.0 }, &b) };
        }
        self.line((cx - 8.0, cy + 0.5), (cx - 2.5, cy + 6.0), OK_FG, 2.0)?;
        self.line((cx - 2.5, cy + 6.0), (cx + 8.5, cy - 5.5), OK_FG, 2.0)?;
        let f = self.f_done.clone();
        self.text(&f, "安装完成", D2D_RECT_F { left: M, top: 164.0, right: W - M, bottom: 188.0 }, TEXT_PRIMARY)?;
        let f = self.f_cap_c.clone();
        self.text(&f, &format!("按 Alt + Space 打开启动台 · {}", dir.display()), D2D_RECT_F { left: M, top: 188.0, right: W - M, bottom: 208.0 }, TEXT_SECONDARY)?;
        let on = self.check_launch;
        self.checkbox(on, "启动 HN DeskPad", true)?;
        self.primary("完成", true)
    }
}

// ---- 动作 ----

fn choose_folder(hwnd: HWND, current: &Path) -> Option<PathBuf> {
    // SAFETY: 标准文件对话框用法；返回字符串由 CoTaskMemFree 释放。
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let _ = dlg.SetTitle(w!("选择安装位置（会在其中自动创建 HN DeskPad 文件夹）"));
        let _ = dlg.SetOkButtonLabel(w!("选择此文件夹"));
        let mut start = current.to_path_buf();
        while !start.exists() {
            match start.parent() {
                Some(p) => start = p.to_path_buf(),
                None => break,
            }
        }
        if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start.as_os_str()), None) {
            let _ = dlg.SetFolder(&item);
        }
        dlg.Show(hwnd).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn set_progress(stage: &str, done: u64, total: u64, hwnd: isize) {
    if let Some(p) = PROGRESS.lock().unwrap().as_mut() {
        p.stage = stage.to_string();
        p.done = done;
        p.total = total;
    }
    // SAFETY: 只投递消息。
    unsafe {
        let _ = PostMessageW(HWND(hwnd as *mut _), WM_APP_PROGRESS, WPARAM(0), LPARAM(0));
    }
}

fn mb(n: u64) -> String {
    format!("{:.1}", n as f64 / 1_048_576.0)
}

fn start_install(ui: &mut Ui) {
    ui.phase = Phase::Working;
    ui.hover = None;
    *PROGRESS.lock().unwrap() = Some(Progress { stage: "正在准备…".into(), ..Default::default() });
    let dir = ui.target();
    let desktop = ui.check_desktop;
    let h = ui.hwnd.0 as isize;
    std::thread::spawn(move || {
        // SAFETY: 工作线程自己的 COM 套间（快捷方式需要）。
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let r = do_install(&dir, desktop, h);
        if let Some(p) = PROGRESS.lock().unwrap().as_mut() {
            p.finished = Some(r);
        }
        set_progress("", 0, 0, h);
    });
}

fn do_install(dir: &Path, desktop: bool, h: isize) -> Result<PathBuf, String> {
    install::ensure_writable(dir)?;
    set_progress("正在获取版本信息…", 0, 0, h);
    let cached = LATEST.lock().unwrap().clone();
    let rel = match cached {
        Some(Ok(r)) => r,
        _ => {
            let r = net::latest();
            *LATEST.lock().unwrap() = Some(r.clone());
            r?
        }
    };
    set_progress("正在下载…", 0, 0, h);
    let zip = net::get(&rel.url, |d, t| {
        let s = if t > 0 { format!("正在下载 {} / {} MB", mb(d), mb(t)) } else { format!("正在下载 {} MB", mb(d)) };
        set_progress(&s, d, t, h);
    })?;
    set_progress("正在校验…", 1, 1, h);
    if install::sha256_hex(&zip) != rel.sha256 {
        return Err("下载的文件校验失败（可能被网络篡改或不完整），请重试".into());
    }
    set_progress("正在安装…", 1, 1, h);
    install::stop_running();
    let size = install::extract(&zip, dir)?;
    install::copy_uninstaller(dir)?;
    let exe = dir.join(install::EXE);
    if let Some(l) = install::start_menu_link() {
        install::make_link(&l, &exe)?;
    }
    if desktop {
        if let Some(l) = install::desktop_link() {
            install::make_link(&l, &exe)?;
        }
    }
    install::register(dir, &rel.version, size)?;
    Ok(dir.to_path_buf())
}

fn activate(ui: &mut Ui, hit: Hit) {
    match (&ui.phase, hit) {
        (Phase::Working, _) => {}
        (Phase::Done(dir), Hit::Primary) => {
            if ui.check_launch {
                install::launch(&dir.join(install::EXE));
            }
            // SAFETY: 关闭自身窗口。
            unsafe {
                let _ = DestroyWindow(ui.hwnd);
            }
        }
        (Phase::Done(_), Hit::Check) => ui.check_launch = !ui.check_launch,
        (_, Hit::Check) => ui.check_desktop = !ui.check_desktop,
        (_, Hit::Field) | (_, Hit::Change) => {
            if let Some(p) = choose_folder(ui.hwnd, &ui.parent) {
                ui.parent = p;
                if matches!(ui.phase, Phase::Failed(_)) {
                    ui.phase = Phase::Ready;
                }
            }
        }
        (_, Hit::Primary) => start_install(ui),
    }
}

fn invalidate(hwnd: HWND) {
    // SAFETY: hwnd 有效。
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn dip(lparam: LPARAM, dpi: u32) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x * 96.0 / dpi as f32, y * 96.0 / dpi as f32)
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: BeginPaint/EndPaint 成对。
            unsafe { BeginPaint(hwnd, &mut ps) };
            with_ui(|u| {
                let _ = u.paint();
            });
            // SAFETY: 同上。
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            with_ui(|u| {
                if let Some(rt) = &u.rt {
                    let w = (lparam.0 & 0xFFFF) as u32;
                    let h = ((lparam.0 >> 16) & 0xFFFF) as u32;
                    // SAFETY: rt 有效。
                    unsafe {
                        let _ = rt.Resize(&D2D_SIZE_U { width: w, height: h });
                    }
                }
            });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            with_ui(|u| {
                u.dpi = dpi.max(96);
                u.rt = None;
                u.brushes.clear();
            });
            // SAFETY: lparam 指向系统建议的新窗口矩形。
            unsafe {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let changed = with_ui(|u| {
                if !u.tracking {
                    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    // SAFETY: 结构体为本地变量。
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    u.tracking = true;
                }
                let (x, y) = dip(lparam, u.dpi);
                let h = u.hit(x, y);
                let c = h != u.hover;
                u.hover = h;
                c
            })
            .unwrap_or(false);
            if changed {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        0x02A3 => {
            // WM_MOUSELEAVE
            with_ui(|u| {
                u.tracking = false;
                u.hover = None;
                u.pressed = None;
            });
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            with_ui(|u| {
                let (x, y) = dip(lparam, u.dpi);
                u.pressed = u.hit(x, y);
            });
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let act = with_ui(|u| {
                let (x, y) = dip(lparam, u.dpi);
                let h = u.hit(x, y);
                let p = u.pressed.take();
                if h.is_some() && h == p {
                    h
                } else {
                    None
                }
            })
            .flatten();
            if let Some(h) = act {
                with_ui(|u| activate(u, h));
            }
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = VIRTUAL_KEY(wparam.0 as u16);
            if vk == VK_RETURN {
                with_ui(|u| activate(u, Hit::Primary));
                invalidate(hwnd);
            } else if vk == VK_ESCAPE {
                // SAFETY: 投递关闭消息（安装中由 WM_CLOSE 拦下）。
                unsafe {
                    let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        m if m == WM_APP_PROGRESS => {
            with_ui(|u| {
                let fin = PROGRESS.lock().unwrap().as_mut().and_then(|p| p.finished.take());
                match fin {
                    Some(Ok(dir)) => {
                        u.installed = install::installed_version();
                        u.phase = Phase::Done(dir);
                    }
                    Some(Err(e)) => u.phase = Phase::Failed(e),
                    None => {}
                }
            });
            invalidate(hwnd);
            LRESULT(0)
        }
        m if m == WM_APP_LATEST => {
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            let busy = with_ui(|u| u.phase == Phase::Working).unwrap_or(false);
            if !busy {
                // SAFETY: 关闭自身窗口。
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: 结束消息循环。
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: 其余消息交给系统默认处理。
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
