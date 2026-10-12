//! 全局状态 App（单例，只在 UI 线程访问）+ 消息常量（UI 文档 2/app.rs）。
use deskpad_engine::{EngineHandle, Snapshot};
use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::WM_APP;
use windows::Win32::Foundation::WPARAM;

pub const WM_APP_TRAY: u32 = WM_APP + 1;
pub const WM_APP_SNAPSHOT: u32 = WM_APP + 2;
/// 启动台里的「设置」按钮：投递给主窗口，在 with_app 之外打开设置窗口。
pub const WM_APP_SETTINGS: u32 = WM_APP + 4;
/// 安装/卸载程序请求正常退出（等同托盘「退出」），以便替换 exe 时不留托盘残影。
pub const WM_APP_QUIT: u32 = WM_APP + 5;
/// WM_MOUSELEAVE 在 windows crate 里属于 Win32_UI_Controls feature，这里直接定义常量，免得为一个常量多开 feature。
pub const WM_MOUSELEAVE: u32 = 0x02A3;

/// 注册消息（懒初始化，注册表项在进程内恒定）。
pub const WM_APP_SHOWPAD: u32 = WM_APP + 3;

pub fn msg_showpad() -> u32 {
    // 注：原用 RegisterWindowMessageW 注册 "HexNook.DeskPad.ShowPad"，
    // 实测跨进程取到的原子值不一致（见 questions.md），改为固定 WM_APP+3。
    // 该消息只发给本应用唯一的 DeskPadMain 窗口，WM_APP 区段无冲突风险。
    WM_APP_SHOWPAD
}
pub fn msg_taskbar_created() -> u32 {
    static M: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *M.get_or_init(|| {
        // SAFETY: 同上。
        unsafe { windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowReason {
    Hotkey,
    Tray,
    Corner,
    SecondInstance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideReason {
    Escape,
    Hotkey,
    ClickOutside,
    Launch,
    System,
    Deactivate,
    DisplayChange,
}

pub struct App {
    pub eng: EngineHandle,
    pub main: HWND,
    /// 启动台背景幕窗口。
    pub scrim: HWND,
    /// 启动台内容窗口。
    pub pad: HWND,
    /// 桌面角标按钮。
    pub corner: HWND,
    /// 首次引导气泡。
    pub bubble: HWND,
    /// 设置窗口（存在时）。
    pub settings_wnd: Option<HWND>,
    /// 引导气泡是否正在显示。
    pub bubble_visible: bool,
    /// D2D 工厂（进程内单份，生命周期同进程）。
    pub factory: windows::Win32::Graphics::Direct2D::ID2D1Factory,
    /// 启动台专用的 D2D 工厂：显示时创建、隐藏时丢弃。软件渲染的暂存缓冲挂在工厂上，
    /// 只释放渲染目标还不够，要连工厂一起放掉内存才会还回去（第 9 节）。
    pub pad_factory: Option<windows::Win32::Graphics::Direct2D::ID2D1Factory>,
    /// 启动台专用的 DWrite 工厂（ISOLATED：字体缓存跟着它走，隐藏时一起释放）。
    pub pad_dwrite: Option<windows::Win32::Graphics::DirectWrite::IDWriteFactory>,
    /// DWrite 工厂。
    pub dwrite: windows::Win32::Graphics::DirectWrite::IDWriteFactory,
    /// 启动台内容窗口的离屏表面。
    pub pad_surface: crate::surface::Surface,
    /// 启动台渲染上下文（隐藏时丢弃）。
    pub pad_ctx: Option<crate::render::RenderCtx>,
    /// 背景幕窗口左上角（物理像素）。
    #[allow(dead_code)] // 遮罩不位移，预留
    pub scrim_origin: (i32, i32),
    /// IME 候选窗位置（内容窗口客户区物理像素）。
    pub ime_pos: (i32, i32),
    pub snap: Snapshot,
    pub input: crate::input::State,
    pub layout: crate::layout::Layout,
    /// 鼠标所在显示器 DPI（96 的倍数，如 144）。
    pub dpi: u32,
    /// 呼出时鼠标所在显示器的工作区（物理像素）。
    pub work: RECT,
    /// 呼出时记录的前台窗口（关闭时归还焦点）。
    pub prev_fg: Option<isize>,
    pub pad_visible: bool,
    /// 内容窗口左上角的屏幕物理坐标（含 SHADOW_MARGIN）。
    pub pad_origin: (i32, i32),
    /// 内容窗口物理尺寸。
    pub pad_size: (i32, i32),
    /// 内容窗口整体不透明度（动画用，0..=255）。
    pub pad_alpha: u8,
    pub scrim_alpha: u8,
    pub caret_on: bool,
    pub anim: Option<crate::anim::Anim>,
    /// 角标按钮位置（物理像素）；None = 默认。
    pub corner_pos: Option<(i32, i32)>,
    /// 错误条（文案 + 出现时间），3 秒后消失。
    pub error: Option<(String, std::time::Instant)>,
    /// 已退出标志（防止 PostQuitMessage 后重复清理）。
    pub quitting: bool,
    /// 本地时区偏移（秒）：本地时间 = Unix 时间 + tz_offset。
    pub tz_offset: i64,
    /// 当前的本地时间（呼出时取一次，用于相对时间格式化）。
    pub now_local: crate::fmt::LocalNow,
}

impl App {
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    pub fn pad_surface(&self) -> &crate::surface::Surface {
        &self.pad_surface
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// 在 UI 线程上以可变借用访问 App。WndProc 只在 UI 线程触发。
/// 重入安全：Win32 调用（ShowWindow / SetForegroundWindow / DestroyWindow 等）会在外层借用期间
/// 同步派发消息回到 WndProc；此时内层直接返回 None，由外层负责状态（不能 panic，release 为 abort）。
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

pub fn set_app(a: App) {
    APP.with_borrow_mut(|s| *s = Some(a));
}

#[allow(dead_code)] // 异常退出路径预留
pub fn take_app() -> Option<App> {
    APP.with_borrow_mut(|s| s.take())
}

/// 触发一次快照刷新消息（引擎回调在扫描线程上调用本函数）。
pub fn post_snapshot(hwnd_raw: isize) {
    // SAFETY: PostMessageW 允许跨线程调用；hwnd 由 UI 线程持有，进程退出前有效。
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            HWND(hwnd_raw as _),
            WM_APP_SNAPSHOT,
            WPARAM(0),
            LPARAM(0),
        );
    }
}
