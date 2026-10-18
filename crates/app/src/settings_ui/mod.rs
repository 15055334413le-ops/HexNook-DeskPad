//! 设置窗口（UI 文档 6.11）：680×500 DIP，左侧 160 分区导航 + 右侧分组设置行，即时生效（无取消/保存）。
//!
//! 结构：五个分区（通用 / 外观 / 应用 / 更新 / 关于），每个分区内用「分组标题 + 48 高设置行」；
//! 控件（开关 / 分段按钮 / 次按钮 / 步进器）改了立刻写设置（热键注册失败就地报错并回退）。
//!
//! 借用规则：S 每次只借一下，DestroyWindow / SetForegroundWindow / with_app 都在借用之外。
//! 渲染坐标全是 DIP（渲染目标 DPI 已设为显示器 DPI）。
mod apply;
mod draw;
mod model;
mod png;
mod window;

pub use model::Section;
pub use window::{open, set_update_state};

/// --render-test --settings <分区>：离屏绘制一个分区（无窗口），返回 PNG 字节。
pub fn render_shot(section: Section, scale: f32) -> windows::core::Result<Vec<u8>> {
    window::render_shot(section, scale)
}
