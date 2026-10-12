//! 键鼠与输入法状态机（UI 文档 6.8）。
use crate::layout::{FocusZone, Hit, Layout};
use crate::app::HideReason;
use deskpad_engine::Snapshot;
use std::path::PathBuf;

// 虚拟键码放在模块级：match 分支里的常量必须在作用域内，否则会被当成绑定变量（匹配一切）。
const VK_ESCAPE: u32 = 0x1B;
const VK_RETURN: u32 = 0x0D;
const VK_BACK: u32 = 0x08;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;
const VK_LEFT: u32 = 0x25;
const VK_RIGHT: u32 = 0x27;
const VK_TAB: u32 = 0x09;
const VK_HOME: u32 = 0x24;
const VK_END: u32 = 0x23;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_V: u32 = 0x56;

pub struct State {
    pub query: String,
    pub focus: FocusZone,
    /// 当前焦点区内的下标（Apps 区是 hits 下标，Files/Dirs 区是快照列表下标）。
    pub selected: Option<usize>,
    pub hover: Option<Hit>,
    pub scroll_row: usize,
    /// search() 结果（apps 下标）。
    pub hits: Vec<usize>,
}

impl State {
    pub fn new() -> Self {
        State {
            query: String::new(),
            focus: FocusZone::Apps,
            selected: Some(0),
            hover: None,
            scroll_row: 0,
            hits: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.query.clear();
        self.focus = FocusZone::Apps;
        self.selected = Some(0);
        self.hover = None;
        self.scroll_row = 0;
        self.hits.clear();
    }

    /// 追加字符后需要重新搜索。
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.focus = FocusZone::Apps;
        self.selected = Some(0);
        self.scroll_row = 0;
    }

    /// 退格（按字素簇；代理对成对删除）。
    pub fn backspace(&mut self, ctrl: bool) {
        if ctrl {
            self.query.clear();
        } else {
            if let Some((idx, _)) = self.query.char_indices().last() {
                self.query.truncate(idx);
            }
        }
        self.focus = FocusZone::Apps;
        self.selected = Some(0);
        self.scroll_row = 0;
    }

    #[allow(dead_code)] // 粘贴路径预留
    pub fn set_query(&mut self, q: &str) {
        self.query = q.chars().take(64).collect();
        self.focus = FocusZone::Apps;
        self.selected = Some(0);
        self.scroll_row = 0;
    }
}

pub enum Action {
    /// 什么都不做（调用方自行决定是否重绘）。
    None,
    Hide(HideReason),
    /// 启动应用（hits 下标）。
    Launch(usize),
    /// 打开路径。
    Open(PathBuf),
    /// 清空了 query（仍需重绘 + 重搜）。
    ClearQuery,
}

/// 处理非字符键。返回动作；重绘由调用方统一决定。
#[allow(clippy::too_many_arguments)]
pub fn key(
    st: &mut State,
    vk: u32,
    ctrl: bool,
    shift: bool,
    layout: &Layout,
    snap: &Snapshot,
) -> Action {
    match vk {
        VK_ESCAPE => {
            if !st.query.is_empty() {
                st.query.clear();
                st.selected = Some(0);
                st.scroll_row = 0;
                Action::ClearQuery
            } else {
                Action::Hide(HideReason::Escape)
            }
        }
        VK_RETURN => {
            match st.focus {
                FocusZone::Apps => {
                    let idx = st.selected.or(st.hits.first().copied());
                    match idx {
                        Some(i) if i < st.hits.len() => Action::Launch(i), // hits 下标，由 pad 换算成 AppId
                        _ => Action::None,
                    }
                }
                FocusZone::Files => {
                    let idx = st.selected.unwrap_or(0);
                    match snap.recent.get(idx) {
                        Some(f) => Action::Open(f.path.clone()),
                        None => Action::None,
                    }
                }
                FocusZone::Dirs => {
                    let idx = st.selected.unwrap_or(0);
                    match snap.dirs.get(idx) {
                        Some(d) => Action::Open(d.path.clone()),
                        None => Action::None,
                    }
                }
            }
        }
        VK_BACK => {
            st.backspace(ctrl);
            Action::ClearQuery
        }
        VK_TAB => {
            st.focus = match (st.focus, shift) {
                (FocusZone::Apps, false) => FocusZone::Files,
                (FocusZone::Files, false) => FocusZone::Dirs,
                (FocusZone::Dirs, false) => FocusZone::Apps,
                (FocusZone::Apps, true) => FocusZone::Dirs,
                (FocusZone::Dirs, true) => FocusZone::Files,
                (FocusZone::Files, true) => FocusZone::Apps,
            };
            st.selected = Some(0);
            st.scroll_row = 0;
            Action::None
        }
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {
            move_selection(st, vk, layout, snap);
            Action::None
        }
        VK_HOME => {
            st.selected = Some(0);
            if st.focus == FocusZone::Apps { st.scroll_row = 0; }
            Action::None
        }
        VK_END => {
            match st.focus {
                FocusZone::Apps => {
                    if !st.hits.is_empty() {
                        st.selected = Some(st.hits.len() - 1);
                        ensure_visible(st, layout, snap.apps.len());
                    }
                }
                FocusZone::Files => st.selected = Some(snap.recent.len().saturating_sub(1)),
                FocusZone::Dirs => st.selected = Some(snap.dirs.len().saturating_sub(1)),
            }
            Action::None
        }
        VK_PRIOR | VK_NEXT => {
            let vis_rows = layout.visible_rows.max(1);
            if st.focus == FocusZone::Apps {
                let page = vis_rows * layout.cols;
                let cur = st.selected.unwrap_or(0);
                let new = if vk == VK_PRIOR { cur.saturating_sub(page) } else { (cur + page).min(st.hits.len().saturating_sub(1)) };
                st.selected = Some(new);
                ensure_visible(st, layout, snap.apps.len());
            } else {
                let len = if st.focus == FocusZone::Files { snap.recent.len() } else { snap.dirs.len() };
                let cur = st.selected.unwrap_or(0);
                let new = if vk == VK_PRIOR { cur.saturating_sub(vis_rows) } else { (cur + vis_rows).min(len.saturating_sub(1)) };
                st.selected = Some(new);
            }
            Action::None
        }
        VK_V if ctrl => {
            // 粘贴由调用方完成（剪贴板在 pad.rs 里取），这里仅预留给键位表。
            Action::None
        }
        _ => Action::None,
    }
}

fn move_selection(st: &mut State, vk: u32, layout: &Layout, snap: &Snapshot) {
    match st.focus {
        FocusZone::Apps => {
            if st.hits.is_empty() { return; }
            let cols = layout.cols;
            let len = st.hits.len();
            let cur = st.selected.unwrap_or(0);
            let new = match vk {
                VK_LEFT => cur.saturating_sub(1),
                VK_RIGHT => (cur + 1).min(len - 1),
                VK_UP => cur.saturating_sub(cols),
                VK_DOWN => (cur + cols).min(len - 1),
                _ => cur,
            };
            st.selected = Some(new);
            ensure_visible(st, layout, len);
        }
        FocusZone::Files => {
            let len = snap.recent.len().min(layout.visible_files);
            if len == 0 { return; }
            let cur = st.selected.unwrap_or(0);
            let cols = layout.file_cols;
            let new = match vk {
                VK_LEFT if cols > 1 => cur.saturating_sub(1),
                VK_RIGHT if cols > 1 => (cur + 1).min(len - 1),
                VK_UP => cur.saturating_sub(cols),
                VK_DOWN => (cur + cols).min(len - 1),
                _ => cur,
            };
            st.selected = Some(new);
        }
        FocusZone::Dirs => {
            let len = snap.dirs.len().min(layout.visible_dirs);
            if len == 0 { return; }
            let cur = st.selected.unwrap_or(0);
            let cols = layout.dir_cols;
            let new = match vk {
                VK_LEFT => cur.saturating_sub(1),
                VK_RIGHT => (cur + 1).min(len - 1),
                VK_UP => cur.saturating_sub(cols),
                VK_DOWN => (cur + cols).min(len - 1),
                _ => cur,
            };
            st.selected = Some(new);
        }
    }
}

/// 选中项滚出可见区时调整 scroll_row。
pub fn ensure_visible(st: &mut State, layout: &Layout, _n_apps: usize) {
    if st.focus != FocusZone::Apps { return; }
    let Some(sel) = st.selected else { return };
    let vis = layout.visible_app_capacity();
    if vis == 0 { return; }
    let first = st.scroll_row * layout.cols;
    let last = first + vis - 1;
    if sel < first {
        st.scroll_row = sel / layout.cols;
    } else if sel > last {
        st.scroll_row = (sel + layout.cols - vis) / layout.cols;
    }
}

/// Ctrl+V 粘贴文本进 query（最多 64 字符，去掉首尾空白，取第一行）。
pub fn paste(st: &mut State, text: &str) {
    let line = text.lines().next().unwrap_or("").trim();
    st.query = line.chars().take(64).collect();
    st.focus = FocusZone::Apps;
    st.selected = Some(0);
    st.scroll_row = 0;
}

/// 滚动一行。
pub fn scroll(st: &mut State, layout: &Layout, rows: i32, n_apps: usize) {
    if st.focus != FocusZone::Apps { return; }
    let total_rows = n_apps.div_ceil(layout.cols);
    let vis_rows = layout.visible_rows.max(1);
    let max_row = total_rows.saturating_sub(vis_rows);
    if max_row == 0 { return; }
    let new = st.scroll_row as i32 + rows;
    st.scroll_row = (new.clamp(0, max_row as i32)) as usize;
    // 选中项跟随可见区（不强制）
    let _ = layout;
}
