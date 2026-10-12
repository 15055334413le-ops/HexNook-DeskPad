//! tofour 设计 Token（UI 文档 4.1 原样照抄）。
// 色彩（ARGB 存储序，0xAARRGGBB）
pub const PRIMARY: u32 = 0xFF3878D8;        // 主色：只用于 Active / Selected / Focus
pub const PRIMARY_HOVER: u32 = 0xFF4A89E5;
pub const PRIMARY_PRESSED: u32 = 0xFF2E68C2;
pub const BACKGROUND: u32 = 0xFFF5F6F7;     // 页面背景
pub const SURFACE: u32 = 0xFFFAFAFA;        // 卡片 / 面板
pub const INPUT: u32 = 0xFFFFFFFF;          // 输入框
pub const TEXT_PRIMARY: u32 = 0xFF222222;
pub const TEXT_SECONDARY: u32 = 0xFF888888;
pub const TEXT_TERTIARY: u32 = 0xFFA3A3A3;
pub const BORDER: u32 = 0xFFE5E5E5;         // 1px 默认边框
pub const BORDER_HOVER: u32 = 0xFFD9D9D9;
pub const ICON: u32 = 0xFF65758B;
pub const SELECT_BG: u32 = 0xFFEAF2FF;      // 选中 / 浅蓝背景
pub const ROW_HOVER: u32 = 0xFFF7F7F7;      // 文件行 hover
pub const DANGER: u32 = 0xFFD93025;         // 错误提示文字
pub const SCRIM: u32 = 0x4D0F121A;          // 背景幕 rgba(15,18,26,0.30)
// 圆角（4 的倍数递进；连续曲率，见 smooth.rs）
pub const RADIUS_CARD: f32 = 10.0;          // 启动台三大面板
pub const RADIUS_INPUT: f32 = 6.0;          // 搜索框
pub const RADIUS_ITEM: f32 = 6.0;           // 应用格、文件行 hover 块、文件夹格（同心：10 − 4）
pub const RADIUS_MODAL: f32 = 12.0;         // 设置窗口
pub const RADIUS_TOAST: f32 = 8.0;          // 引导气泡
// 阴影 (blur, dy, alpha)
pub const SHADOW_L1: (f32, f32, f32) = (4.0, 1.0, 0.06);    // 角标按钮
pub const SHADOW_L2: (f32, f32, f32) = (6.0, 2.0, 0.08);    // Hover / Toast
pub const SHADOW_L3: (f32, f32, f32) = (12.0, 4.0, 0.12);   // 面板 / Modal
#[allow(dead_code)] // tofour Token，规范 4.1 原样照抄
pub const SHADOW_INPUT: (f32, f32, f32) = (2.0, 1.0, 0.04); // 搜索框内阴影 inset
pub const SHADOW_MARGIN: f32 = 24.0;        // 带阴影窗口四周预留的透明边距（≥ blur*1.5 + dy）
// 字体
pub const FONT_FAMILY: &str = "Microsoft YaHei UI";
pub const FS_TITLE: f32 = 16.0;    // 面板标题，600
pub const FS_BODY: f32 = 14.0;     // 文件名、搜索框、按钮，400
pub const FS_CAPTION: f32 = 12.0;  // 应用名、文件夹名、计数，400
pub const FS_SMALL: f32 = 11.0;    // 时间、类型色块文字，400（类型色块用 600）
// 间距（8px 网格）
pub const PAD_CARD: f32 = 16.0;    // 卡片内边距
pub const GAP: f32 = 12.0;         // 组件间距、应用格间距
pub const GAP_CARD: f32 = 16.0;    // 三块面板之间

/// 预创建画刷所需的全部颜色（含文件类型色块内容色）。
#[allow(dead_code)] // 备用：完整调色板清单（RenderCtx 内联了同清单）
pub fn eager_palette() -> Vec<u32> {
    vec![
        PRIMARY, PRIMARY_HOVER, PRIMARY_PRESSED, BACKGROUND, SURFACE, INPUT,
        TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, BORDER, BORDER_HOVER,
        ICON, SELECT_BG, ROW_HOVER, DANGER,
        0x0A000000, 0xFFFFFFFF, 0xFFFF0000,
        // 文件类型色块内容色
        0xFF2B579A, 0xFF217346, 0xFFD24726, 0xFFD93025, 0xFF8E5BD8,
        0xFFE8710A, 0xFFD81B60, 0xFF8D6E63, 0xFF00897B, 0xFF455A64, 0xFF9E9E9E,
    ]
}

/// ARGB(u32) → D2D 颜色。
pub fn color(c: u32) -> windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
    let (a, r, g, b) = argb(c);
    windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F { r, g, b, a }
}

/// ARGB(u32) → D2D 的 (a, r, g, b) 四元组（D2D1_COLOR_F 分量是 0..1 浮点）。
pub const fn argb(c: u32) -> (f32, f32, f32, f32) {
    (
        ((c >> 24) & 0xFF) as f32 / 255.0,
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    )
}
