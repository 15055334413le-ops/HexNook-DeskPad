//! 布局计算 + 命中测试（UI 文档 6.6；单位：DIP；纯函数，有单元测试）。
use crate::theme::{GAP, GAP_CARD, PAD_CARD, SHADOW_MARGIN};
use deskpad_engine::PadSize;
pub use deskpad_engine::ViewMode;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl RectF {
    pub fn right(&self) -> f32 { self.x + self.w }
    pub fn bottom(&self) -> f32 { self.y + self.h }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusZone {
    Apps,
    Files,
    Dirs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// 应用格：hits 中的绝对下标（已含滚动偏移换算）。
    App(usize),
    File(usize),
    Dir(usize),
    Search,
    /// 搜索框右侧的设置按钮。
    Settings,
    /// 面板空白处（不关闭）。
    Card,
    /// 卡片右上角的视图切换（Files / Dirs 区，切到哪种视图）。
    Toggle(FocusZone, ViewMode),
}

#[derive(Clone, Debug)]
pub struct Layout {
    #[allow(dead_code)] // 测试用基准
    pub work: RectF,
    pub content: RectF,
    pub apps_card: RectF,
    pub files_card: RectF,
    pub dirs_card: RectF,
    /// 相对 content 的坐标（下同）。
    pub search: RectF,
    pub grid: RectF,
    pub cell_w: f32,
    pub cell_h: f32,
    pub cols: usize,
    pub visible_rows: usize,
    pub visible_files: usize,
    pub dir_cols: usize,
    pub dir_cell_h: f32,
    pub files_view: ViewMode,
    pub dirs_view: ViewMode,
    /// 最近文件：每行几个（列表 1，图标 4）、格高、行间距。
    pub file_cols: usize,
    pub file_cell_h: f32,
    /// 可见的文件夹个数（列表/图标通用）。
    pub visible_dirs: usize,
}

/// 图标视图参数：文件每行 3 个（文件名长，给宽一点），文件夹每行 4 个；格间距 4。
pub const FILE_GRID_COLS: usize = 3;
pub const DIR_GRID_COLS: usize = 4;
pub const GRID_GAP: f32 = 4.0;
pub const FILE_GRID_H: f32 = 88.0;
pub const DIR_GRID_H: f32 = 80.0;
/// 全屏模式下每列的目标宽度；实际列宽按可用宽度均分。
pub const APP_COL_TARGET_W: f32 = 128.0;
/// 切换按钮：宽 52 高 24，右距卡片 16，顶距 12。
/// 搜索框右侧设置按钮边长。
pub const SETTINGS_BTN: f32 = 36.0;
pub const TOGGLE_W: f32 = 52.0;
pub const TOGGLE_H: f32 = 24.0;

/// work：工作区（DIP）。返回所有矩形；应用卡内部矩形以 content 原点为基准（即已含卡片位置）。
pub fn compute(work: RectF, max_recent: usize, dirs_len: usize, views: (ViewMode, ViewMode)) -> Layout {
    compute_with(work, max_recent, dirs_len, views, PadSize::Window)
}

pub fn compute_with(
    work: RectF,
    max_recent: usize,
    dirs_len: usize,
    views: (ViewMode, ViewMode),
    pad_size: PadSize,
) -> Layout {
    let fullscreen = pad_size == PadSize::Fullscreen;
    let content = if fullscreen {
        RectF {
            x: work.x + SHADOW_MARGIN,
            y: work.y + SHADOW_MARGIN,
            w: (work.w - 2.0 * SHADOW_MARGIN).max(0.0),
            h: (work.h - 2.0 * SHADOW_MARGIN).max(0.0),
        }
    } else {
        let content_w = (work.w - 96.0).min(1120.0);
        let content_h = (work.h - 96.0).min(720.0);
        RectF {
            x: work.x + (work.w - content_w) / 2.0,
            y: work.y + (work.h - content_h) / 2.0,
            w: content_w,
            h: content_h,
        }
    };
    let right_w = if content.w < 1000.0 { 280.0 } else { 320.0 };
    let apps_card = RectF {
        x: content.x,
        y: content.y,
        w: content.w - GAP_CARD - right_w,
        h: content.h,
    };
    let files_h = (64.0 + max_recent as f32 * 36.0).min(content.h - GAP_CARD - 168.0);
    let files_card = RectF { x: apps_card.right() + GAP_CARD, y: content.y, w: right_w, h: files_h };
    let dirs_card = RectF {
        x: files_card.x,
        y: files_card.bottom() + GAP_CARD,
        w: right_w,
        h: content.h - files_h - GAP_CARD,
    };

    // 应用卡内部（相对 content）
    let search = RectF { x: apps_card.x + PAD_CARD, y: apps_card.y + PAD_CARD, w: apps_card.w - 32.0 - SETTINGS_BTN - 8.0, h: 36.0 };
    let grid = RectF {
        x: apps_card.x + PAD_CARD,
        y: apps_card.y + 68.0,
        w: apps_card.w - 32.0,
        h: apps_card.h - 68.0 - PAD_CARD,
    };
    let cols = if fullscreen {
        (((grid.w + GAP) / (APP_COL_TARGET_W + GAP)).floor() as usize).max(6)
    } else {
        6
    };
    let cell_w = (grid.w - 5.0 * GAP) / cols as f32;
    let cell_h = 104.0;
    let visible_rows = ((grid.h + GAP) / (cell_h + GAP)).floor() as usize;

    // 文件卡 / 文件夹卡：列表（行高 36）或图标（每行 4 个）
    let row_h = 36.0;
    let (files_view, dirs_view) = views;
    let fits = |card: &RectF, cell_h: f32, gap: f32| -> usize {
        (((card.h - 56.0 + gap) / (cell_h + gap)).floor().max(0.0)) as usize
    };
    let (file_cols, file_cell_h) = match files_view {
        ViewMode::List => (1, row_h),
        ViewMode::Grid => (FILE_GRID_COLS, FILE_GRID_H),
    };
    let visible_files = match files_view {
        ViewMode::List => (((files_card.h - 64.0) / row_h).floor().max(0.0) as usize).min(max_recent),
        ViewMode::Grid => (fits(&files_card, FILE_GRID_H, GRID_GAP) * FILE_GRID_COLS).min(max_recent),
    };
    let (dir_cols, dir_cell_h) = match dirs_view {
        ViewMode::List => (1usize, 36.0),
        ViewMode::Grid => (DIR_GRID_COLS, DIR_GRID_H),
    };
    let visible_dirs = match dirs_view {
        ViewMode::List => (((dirs_card.h - 64.0) / 36.0).floor().max(0.0) as usize).min(dirs_len),
        ViewMode::Grid => (fits(&dirs_card, DIR_GRID_H, GRID_GAP) * DIR_GRID_COLS).min(dirs_len),
    };

    Layout {
        work,
        content,
        apps_card,
        files_card,
        dirs_card,
        search,
        grid,
        cell_w,
        cell_h,
        cols,
        visible_rows,
        visible_files,
        dir_cols,
        dir_cell_h,
        files_view,
        dirs_view,
        file_cols,
        file_cell_h,
        visible_dirs,
    }
}

/// 卡片内第 i 项：列表时整行（左右各留 8），图标时 cols 列网格（左右各留 8、间距 GRID_GAP）。
fn grid_cell(card: &RectF, i: usize, cols: usize, cell_h: f32, view: ViewMode) -> RectF {
    match view {
        ViewMode::List => RectF { x: card.x + 8.0, y: card.y + 48.0 + i as f32 * cell_h, w: card.w - 16.0, h: cell_h },
        ViewMode::Grid => {
            let inner = card.w - 16.0;
            let w = (inner - (cols - 1) as f32 * GRID_GAP) / cols as f32;
            let (r, c) = (i / cols, i % cols);
            RectF { x: card.x + 8.0 + c as f32 * (w + GRID_GAP), y: card.y + 48.0 + r as f32 * (cell_h + GRID_GAP), w, h: cell_h }
        }
    }
}

impl Layout {
    /// 应用格矩形（与其他矩形同一坐标系：工作区 DIP）。idx 为 hits 中的绝对下标。
    pub fn app_cell(&self, idx: usize, scroll_row: usize) -> RectF {
        let col = (idx % self.cols) as f32;
        let disp_row = (idx / self.cols) as f32 - scroll_row as f32;
        RectF {
            x: self.grid.x + col * (self.cell_w + GAP),
            y: self.grid.y + disp_row * (self.cell_h + GAP),
            w: self.cell_w,
            h: self.cell_h,
        }
    }

    /// 文件行相对 content 的矩形（visible 下标）。
    /// 最近文件第 i 项的矩形（列表=整行，图标=格子）。
    pub fn file_row(&self, i: usize) -> RectF {
        grid_cell(&self.files_card, i, self.file_cols, self.file_cell_h, self.files_view)
    }

    /// 视图切换按钮（整体），Files / Dirs 卡右上角。
    pub fn toggle_rect(&self, zone: FocusZone) -> RectF {
        let card = if zone == FocusZone::Dirs { &self.dirs_card } else { &self.files_card };
        RectF { x: card.right() - 16.0 - TOGGLE_W, y: card.y + 12.0, w: TOGGLE_W, h: TOGGLE_H }
    }


    /// 文件夹格相对 content 的矩形（绝对下标）。
    pub fn dir_cell(&self, idx: usize) -> Option<RectF> {
        if idx >= self.visible_dirs { return None; }
        Some(grid_cell(&self.dirs_card, idx, self.dir_cols, self.dir_cell_h, self.dirs_view))
    }



    /// 命中测试。pt 与布局矩形同一坐标系（工作区 DIP）。
    /// 设置按钮：搜索框右侧 8px，36×36。
    pub fn settings_btn(&self) -> RectF {
        RectF { x: self.search.right() + 8.0, y: self.search.y, w: SETTINGS_BTN, h: self.search.h }
    }

    pub fn hit_test(&self, pt: (f32, f32), scroll_row: usize, n_apps: usize, n_files: usize, n_dirs: usize) -> Hit {
        if self.search.contains(pt.0, pt.1) { return Hit::Search; }
        if self.settings_btn().contains(pt.0, pt.1) { return Hit::Settings; }
        // 视图切换按钮：左半 = 图标视图，右半 = 列表视图
        for zone in [FocusZone::Files, FocusZone::Dirs] {
            let t = self.toggle_rect(zone);
            if t.contains(pt.0, pt.1) {
                let mode = if pt.0 < t.x + t.w / 2.0 { ViewMode::Grid } else { ViewMode::List };
                return Hit::Toggle(zone, mode);
            }
        }
        // 文件行
        for i in 0..self.visible_files.min(n_files) {
            if self.file_row(i).contains(pt.0, pt.1) { return Hit::File(i); }
        }
        // 文件夹格
        for i in 0..n_dirs {
            if let Some(r) = self.dir_cell(i) {
                if r.contains(pt.0, pt.1) { return Hit::Dir(i); }
            }
        }
        // 应用格
        if self.grid.contains(pt.0, pt.1) {
            let col = (((pt.0 - self.grid.x) / (self.cell_w + GAP)).floor() as usize).min(self.cols - 1);
            let row_f = (pt.1 - self.grid.y) / (self.cell_h + GAP);
            if row_f >= 0.0 {
                let row = row_f as usize;
                let vis_idx = row * self.cols + col;
                let base = scroll_row * self.cols;
                let idx = base + vis_idx;
                if vis_idx < self.visible_rows * self.cols && idx < n_apps {
                    // 点在格子上（列内非间隙）：
                    let cell_x = self.grid.x + col as f32 * (self.cell_w + GAP);
                    if pt.0 <= cell_x + self.cell_w {
                        return Hit::App(idx);
                    }
                }
            }
        }
        Hit::Card
    }

    /// 应用区可见的最后一批数量。
    pub fn visible_app_capacity(&self) -> usize {
        self.visible_rows * self.cols
    }
}

#[cfg(test)]
mod t {
    use super::*;

    fn check(work: (f32, f32)) {
        let l = compute(RectF { x: 0.0, y: 0.0, w: work.0, h: work.1 }, 8, 6, (ViewMode::Grid, ViewMode::Grid));
        assert!(l.apps_card.w > 0.0 && l.files_card.w > 0.0 && l.dirs_card.w > 0.0);
        // 全部位于工作区内
        assert!(l.apps_card.x >= l.work.x && l.apps_card.right() <= l.work.right());
        assert!(l.content.bottom() <= l.work.bottom());
        // 三卡不重叠
        assert!(l.files_card.x >= l.apps_card.right());
        assert!(l.dirs_card.y >= l.files_card.bottom() - 0.5);
        // 格宽足够放图标
        assert!(l.cell_w > 62.0, "cell_w = {}", l.cell_w);
        // 搜索框在应用卡内
        assert!(l.search.w > 100.0 && l.search.x >= l.apps_card.x);
    }

    #[test]
    fn layout_1920() { check((1920.0, 1040.0)); }

    #[test]
    fn layout_1024() { check((1024.0, 640.0)); }

    /// 物理工作区换算为布局使用的 DIP；任务栏按 40 物理像素预留。
    fn fullscreen_work(px_w: f32, px_h: f32, dpi: u32) -> Layout {
        let scale = dpi as f32 / 96.0;
        let work = RectF {
            x: 0.0,
            y: 0.0,
            w: px_w / scale,
            h: (px_h - 40.0) / scale,
        };
        compute_with(
            work,
            8,
            6,
            (ViewMode::Grid, ViewMode::Grid),
            PadSize::Fullscreen,
        )
    }

    #[test]
    fn fullscreen_reflows_supported_resolutions_and_scales() {
        for (px_w, px_h, dpi) in [
            (1920.0, 1080.0, 96u32),
            (1920.0, 1080.0, 144u32),
            (2560.0, 1440.0, 96u32),
            (2560.0, 1440.0, 144u32),
            (3840.0, 2160.0, 96u32),
            (3840.0, 2160.0, 144u32),
        ] {
            let l = fullscreen_work(px_w, px_h, dpi);
            assert!(l.apps_card.w > 0.0);
            assert!(l.files_card.w <= 320.0, "{px_w}x{px_h}@{dpi}: {}", l.files_card.w);
            assert!(l.files_card.w >= 280.0);
            assert!(l.apps_card.x >= l.work.x);
            assert!(l.dirs_card.bottom() <= l.work.bottom() + 0.5);
            assert!(l.files_card.x >= l.apps_card.right());
            assert!(l.dirs_card.y >= l.files_card.bottom() - 0.5);
            assert!(l.cell_w > 62.0, "{px_w}x{px_h}@{dpi}: {}", l.cell_w);
            // 内容外扩阴影边距后正好覆盖工作区。
            assert!((l.content.x - crate::theme::SHADOW_MARGIN - l.work.x).abs() < 0.01);
            assert!((l.content.y - crate::theme::SHADOW_MARGIN - l.work.y).abs() < 0.01);
            assert!(
                (l.content.w + 2.0 * crate::theme::SHADOW_MARGIN - l.work.w).abs() < 0.01
            );
            assert!(
                (l.content.h + 2.0 * crate::theme::SHADOW_MARGIN - l.work.h).abs() < 0.01
            );
        }
    }

    #[test]
    fn fullscreen_columns_grow_with_available_width() {
        let fhd = fullscreen_work(1920.0, 1080.0, 96);
        let qhd = fullscreen_work(2560.0, 1440.0, 96);
        let uhd = fullscreen_work(3840.0, 2160.0, 96);
        assert!(fhd.cols > 6);
        assert!(qhd.cols > fhd.cols);
        assert!(uhd.cols > qhd.cols);
        assert_eq!(compute(RectF { x: 0.0, y: 0.0, w: 1920.0, h: 1040.0 }, 8, 6,
            (ViewMode::Grid, ViewMode::Grid)).cols, 6);
    }

    #[test]
    fn hit_test_basics() {
        let l = compute(RectF { x: 0.0, y: 0.0, w: 1920.0, h: 1040.0 }, 8, 6, (ViewMode::Grid, ViewMode::Grid));
        // 搜索框
        assert_eq!(l.hit_test((l.search.x + 5.0, l.search.y + 5.0), 0, 20, 8, 6), Hit::Search);
        // 第一行第一列应用
        let c = l.app_cell(0, 0);
        assert_eq!(l.hit_test((c.x + 5.0, c.y + 5.0), 0, 20, 8, 6), Hit::App(0));
        // 第二行第一列
        let c2 = l.app_cell(6, 0);
        assert_eq!(l.hit_test((c2.x + 5.0, c2.y + 5.0), 0, 20, 8, 6), Hit::App(6));
        // 文件行
        let f = l.file_row(0);
        assert_eq!(l.hit_test((f.x + 5.0, f.y + 5.0), 0, 20, 8, 6), Hit::File(0));
        // 文件夹格
        let d = l.dir_cell(0).unwrap();
        assert_eq!(l.hit_test((d.x + 5.0, d.y + 5.0), 0, 20, 8, 6), Hit::Dir(0));
        // 卡片空白
        assert_eq!(l.hit_test((l.apps_card.right() - 2.0, l.apps_card.bottom() - 2.0), 0, 20, 8, 6), Hit::Card);
    }
}
