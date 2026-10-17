//! D2D 绘制（UI 文档 6.7 + 4.2 tofour 落地映射）。所有坐标：DIP（渲染目标 DPI 已设为显示器 DPI）。
use crate::icons::IconCache;
use crate::layout::{FocusZone, Hit, Layout, RectF, ViewMode};
use crate::smooth::{draw_smooth_rect, draw_soft_shadow, fill_smooth_rect, GeoCache};
use crate::theme::*;
use deskpad_engine::FileKind;
use std::collections::HashMap;
use windows::core::*;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, IWICImagingFactory,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

/// 文件类型色块颜色（内容色，UI 文档 6.7 表）。
pub fn chip_color(k: FileKind) -> u32 {
    match k {
        FileKind::Document => 0xFF2B579A,
        FileKind::Spreadsheet => 0xFF217346,
        FileKind::Presentation => 0xFFD24726,
        FileKind::Pdf => 0xFFD93025,
        FileKind::Image => 0xFF8E5BD8,
        FileKind::Video => 0xFFE8710A,
        FileKind::Audio => 0xFFD81B60,
        FileKind::Archive => 0xFF8D6E63,
        FileKind::Installer => 0xFF00897B,
        FileKind::Code => 0xFF455A64,
        FileKind::Other => 0xFF9E9E9E,
    }
}

/// 把 ARGB(u32) 变成 D2D 颜色。
pub fn color(c: u32) -> D2D1_COLOR_F {
    let (a, r, g, b) = argb(c);
    D2D1_COLOR_F { r, g, b, a }
}


/// 固定调色板的画刷表。与 geo / tiles 是不同字段，可以和 `&mut ctx.geo` 同时借用。
pub struct Palette(HashMap<u32, ID2D1Brush>);
impl Palette {
    pub fn get(&self, c: u32) -> Result<&ID2D1Brush> {
        match self.0.get(&c) {
            Some(b) => Ok(b),
            None => {
                deskpad_engine::log::warn(&format!("render: brush {c:#010x} not in palette"));
                Err(Error::from(E_FAIL))
            }
        }
    }
}

/// 每个分层窗口一个渲染上下文；启动台隐藏时整体丢弃。
pub struct RenderCtx {
    pub rt: ID2D1RenderTarget,
    pub dwrite: IDWriteFactory,
    pub wic: IWICImagingFactory,
    pub pal: Palette,
    pub geo: GeoCache,
    pub tiles: HashMap<(u32, u32), ID2D1Bitmap>,
    pub icons: IconCache,
    pub f_title: IDWriteTextFormat,
    pub f_body: IDWriteTextFormat,
    pub f_body_c: IDWriteTextFormat,
    pub f_cap: IDWriteTextFormat,
    pub f_cap_c: IDWriteTextFormat,
    pub f_small: IDWriteTextFormat,
    pub f_small_r: IDWriteTextFormat,
    /// 文件图标底部扩展名标签：9px 600 居中。
    pub f_tag: IDWriteTextFormat,
}

fn make_format(dwrite: &IDWriteFactory, size: f32, weight: DWRITE_FONT_WEIGHT, center: bool, right: bool) -> Result<IDWriteTextFormat> {
    let fam: Vec<u16> = FONT_FAMILY.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: 字族名缓冲区在调用期间有效；格式对象由返回值接管。
    let f = unsafe {
        dwrite.CreateTextFormat(
            PCWSTR(fam.as_ptr()),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("zh-CN"),
        )?
    };
    let ha = if center { DWRITE_TEXT_ALIGNMENT_CENTER } else if right { DWRITE_TEXT_ALIGNMENT_TRAILING } else { DWRITE_TEXT_ALIGNMENT_LEADING };
    // SAFETY: 无外部资源；返回 Option<()> 忽略即可。
    unsafe {
        let _ = f.SetTextAlignment(ha);
        let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        // 所有文字都是单行：不换行，超长按字符截断并补「…」（之前文件名会折成两行、色块里的 DOCX 会断成 DOC/X）
        let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        if let Ok(sign) = dwrite.CreateEllipsisTrimmingSign(&f) {
            let _ = f.SetTrimming(
                &DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 },
                &sign,
            );
        }
    }
    Ok(f)
}

impl RenderCtx {
    pub fn new(_factory: &ID2D1Factory, dwrite: &IDWriteFactory, rt: ID2D1RenderTarget) -> Result<Self> {
        // SAFETY: 标准 COM 工厂创建，进程内可重入。
        let wic = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let f_title = make_format(dwrite, FS_TITLE, DWRITE_FONT_WEIGHT_SEMI_BOLD, false, false)?;
        let f_body = make_format(dwrite, FS_BODY, DWRITE_FONT_WEIGHT_NORMAL, false, false)?;
        let f_body_c = make_format(dwrite, FS_BODY, DWRITE_FONT_WEIGHT_NORMAL, true, false)?;
        let f_cap = make_format(dwrite, FS_CAPTION, DWRITE_FONT_WEIGHT_NORMAL, false, false)?;
        let f_cap_c = make_format(dwrite, FS_CAPTION, DWRITE_FONT_WEIGHT_NORMAL, true, false)?;
        let f_small = make_format(dwrite, FS_SMALL, DWRITE_FONT_WEIGHT_NORMAL, false, false)?;
        let f_small_r = make_format(dwrite, FS_SMALL, DWRITE_FONT_WEIGHT_NORMAL, false, true)?;
        let f_tag = make_format(dwrite, 8.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, true, false)?;
        // 画刷在创建 RT 时一次性建好（调色板是固定的），绘制路径上 brush() 只读查表，
        // 避免 &mut self 与 &mut ctx.geo 同时借用。
        let mut brushes: HashMap<u32, ID2D1Brush> = HashMap::new();
        let palette = [
            PRIMARY, PRIMARY_HOVER, PRIMARY_PRESSED, BACKGROUND, SURFACE, INPUT, TEXT_PRIMARY,
            TEXT_SECONDARY, TEXT_TERTIARY, BORDER, BORDER_HOVER, ICON, SELECT_BG, ROW_HOVER,
            DANGER, SCRIM, HIT_SHIELD, 0x0A000000, 0xFFFFFFFF, 0xFFFF0000,
        ];
        let chips = [
            FileKind::Document, FileKind::Spreadsheet, FileKind::Presentation, FileKind::Pdf,
            FileKind::Image, FileKind::Video, FileKind::Audio, FileKind::Archive,
            FileKind::Installer, FileKind::Code, FileKind::Other,
        ];
        for c in palette.into_iter().chain(chips.into_iter().map(chip_color)).chain(crate::fileicons::extra_colors()) {
            if brushes.contains_key(&c) { continue; }
            // SAFETY: rt 有效；画刷与 rt 同寿命（同存于本结构）。
            let b = unsafe { rt.CreateSolidColorBrush(&color(c), None)? };
            brushes.insert(c, b.cast::<ID2D1Brush>()?);
        }
        Ok(RenderCtx {
            rt,
            dwrite: dwrite.clone(),
            wic,
            pal: Palette(brushes),
            geo: GeoCache::default(),
            tiles: HashMap::new(),
            icons: IconCache::new(),
            f_title,
            f_body,
            f_body_c,
            f_cap,
            f_cap_c,
            f_small,
            f_small_r,
            f_tag,
        })
    }

    #[allow(dead_code)] // 快照切换预留
    pub fn clear_caches(&mut self) {
        self.geo = GeoCache::default();
        self.tiles.clear();
        self.icons.clear();
    }



    fn card(
        &mut self,
        factory: &ID2D1Factory,
        rect: D2D_RECT_F,
    ) -> Result<()> {
        let (blur, dy, alpha) = SHADOW_L3;
        let shadow_brush_col = color(0xFF0F121A);
        // 阴影用黑色画刷（alpha 由贴图控制）
        let _ = shadow_brush_col;
        draw_soft_shadow(factory, &mut self.tiles, &self.rt, rect, RADIUS_CARD, blur, dy, alpha, self.rt.dpi_scale())?;
        fill_smooth_rect(factory, &mut self.geo, &self.rt, rect, RADIUS_CARD, self.pal.get(SURFACE)?)?;
        draw_smooth_rect(factory, &mut self.geo, &self.rt, rect, RADIUS_CARD, self.pal.get(BORDER)?, 1.0)?;
        Ok(())
    }
}

/// D2D 渲染目标的 DPI 缩放系数（相对 96）。
pub trait DpiScale {
    fn dpi_scale(&self) -> f32;
}
impl DpiScale for ID2D1RenderTarget {
    fn dpi_scale(&self) -> f32 {
        let mut dx = 96.0f32;
        let mut dy = 96.0f32;
        // SAFETY: 输出参数由本函数栈持有。
        unsafe { self.GetDpi(&mut dx, &mut dy) };
        dx / 96.0
    }
}

/// 窗口原点 = content 左上角向外扩 SHADOW_MARGIN（DIP）。
pub fn window_dip_offset() -> (f32, f32) {
    (SHADOW_MARGIN, SHADOW_MARGIN)
}

/// 把 content 系坐标转成窗口系（窗口原点在 content 外扩 margin 处）。
fn win_rect(card: &RectF, content: &RectF) -> D2D_RECT_F {
    let (ox, oy) = window_dip_offset();
    D2D_RECT_F {
        left: card.x - content.x + ox,
        top: card.y - content.y + oy,
        right: card.right() - content.x + ox,
        bottom: card.bottom() - content.y + oy,
    }
}

fn draw_text_simple(rt: &ID2D1RenderTarget, fmt: &IDWriteTextFormat, text: &str, rect: D2D_RECT_F, brush: &ID2D1Brush) {
    let w: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: 字符串切片在调用期间有效。
    unsafe { rt.DrawText(&w, fmt, &rect, brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL) };
}

fn draw_text_trimmed(rt: &ID2D1RenderTarget, dwrite: &IDWriteFactory, fmt: &IDWriteTextFormat, text: &str, rect: D2D_RECT_F, brush: &ID2D1Brush) {
    let w: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: 字符串切片在调用期间有效。
    let layout = match unsafe { dwrite.CreateTextLayout(&w, fmt, rect.right - rect.left, rect.bottom - rect.top) } {
        Ok(l) => l,
        Err(_) => return draw_text_simple(rt, fmt, text, rect, brush),
    };
    // SAFETY: layout 对象在本函数内有效。
    unsafe {
        // 截断规则（按字符 + 「…」）已设在文本格式上，布局会继承，这里不要再覆盖成「无省略号」。
        rt.DrawTextLayout(
            D2D_POINT_2F { x: rect.left, y: rect.top },
            &layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
        );
    }
}

fn text_width(dwrite: &IDWriteFactory, fmt: &IDWriteTextFormat, text: &str) -> f32 {
    let w: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: 字符串切片在调用期间有效。
    match unsafe { dwrite.CreateTextLayout(&w, fmt, 10000.0, 40.0) } {
        Ok(l) => {
            let mut m = DWRITE_TEXT_METRICS::default();
            // SAFETY: metrics 输出到栈变量。
            unsafe { let _ = l.GetMetrics(&mut m); }
            m.width
        }
        Err(_) => 0.0,
    }
}

/// 启动台内容窗口整帧绘制。app.layout 与 app.input 为当前状态。
pub fn draw_pad(
    app: &mut crate::app::App,
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
) -> Result<()> {
    let rt = ctx.rt.clone();
    // SAFETY: 绘制序列在 Begin/End 之间；RT 已由 surface BindDC。
    unsafe { rt.BeginDraw() };
    // SAFETY: 清成全透明。
    unsafe { rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })) };

    let r = draw_pad_inner(app, ctx, factory);
    let end = unsafe { rt.EndDraw(None, None) };
    r?;
    end?;
    Ok(())
}

fn draw_pad_inner(
    app: &mut crate::app::App,
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
) -> Result<()> {
    let l: &Layout = &app.layout;
    let rt = ctx.rt.clone();
    let content = l.content;

    // 1. 三块面板（阴影 → 填充 → 边框）
    for card in [l.apps_card, l.files_card, l.dirs_card] {
        ctx.card(factory, win_rect(&card, &content))?;
    }

    // 2. 搜索框
    let search = RectF {
        x: l.search.x - content.x + SHADOW_MARGIN,
        y: l.search.y - content.y + SHADOW_MARGIN,
        w: l.search.w,
        h: l.search.h,
    };
    fill_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&search), RADIUS_INPUT, ctx.pal.get(INPUT)?)?;
    // 轻内阴影：顶部 1px 极浅灰线
    fill_smooth_rect(
        factory, &mut ctx.geo, &rt,
        D2D_RECT_F { left: search.x + 1.0, top: search.y + 1.0, right: search.right() - 1.0, bottom: search.y + 2.0 },
        1.0, ctx.pal.get(0x0A000000)?,
    )?;
    // 聚焦边框（启动台显示期间搜索框始终聚焦）
    draw_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&search), RADIUS_INPUT, ctx.pal.get(PRIMARY)?, 1.0)?;

    // 搜索文本 / 占位符 / 光标
    let tx = search.x + 10.0;
    let ty = search.y;
    let th = search.h;
    let caret_x = if app.input.query.is_empty() {
        draw_text_simple(&rt, &ctx.f_body, "搜索应用", D2D_RECT_F { left: tx, top: ty, right: search.right() - 10.0, bottom: ty + th }, ctx.pal.get(TEXT_TERTIARY)?);
        tx
    } else {
        let w = text_width(&ctx.dwrite, &ctx.f_body, &app.input.query);
        draw_text_simple(&rt, &ctx.f_body, &app.input.query, D2D_RECT_F { left: tx, top: ty, right: search.right() - 10.0, bottom: ty + th }, ctx.pal.get(TEXT_PRIMARY)?);
        tx + w
    };
    if app.caret_on {
        let _ = fill_smooth_rect(factory, &mut ctx.geo, &rt,
            D2D_RECT_F { left: caret_x + 2.0, top: ty + th / 2.0 - 9.0, right: caret_x + 3.0, bottom: ty + th / 2.0 + 9.0 },
            0.5, ctx.pal.get(TEXT_PRIMARY)?);
    }
    // IME 候选窗位置（客户区物理像素，CFS_POINT）
    app.ime_pos = (((caret_x + 2.0) * app.scale()).round() as i32, ((search.bottom() + 2.0) * app.scale()).round() as i32);
    // 设置按钮（搜索框右侧）：默认只有线性齿轮，悬停浅底 + 主色
    {
        let b = l.settings_btn();
        let br = D2D_RECT_F { left: b.x - content.x + SHADOW_MARGIN, top: b.y - content.y + SHADOW_MARGIN, right: b.right() - content.x + SHADOW_MARGIN, bottom: b.bottom() - content.y + SHADOW_MARGIN };
        let hot = app.input.hover == Some(Hit::Settings);
        if hot {
            fill_smooth_rect(factory, &mut ctx.geo, &rt, br, RADIUS_INPUT, ctx.pal.get(ROW_HOVER)?)?;
        }
        let s = 20.0;
        crate::fileicons::draw_gear(&rt, factory, &ctx.pal, (br.left + br.right - s) / 2.0, (br.top + br.bottom - s) / 2.0, s, if hot { PRIMARY } else { ICON })?;
        // 有新版本时齿轮右上角一个小 PRIMARY 圆点（6 DIP；不发气泡，#13）
        if app.update.has_new() {
            let r = 3.0; // 直径 6
            let cx = br.right - 1.0 - r;
            let cy = br.top + 1.0 + r;
            fill_smooth_rect(factory, &mut ctx.geo, &rt,
                D2D_RECT_F { left: cx - r, top: cy - r, right: cx + r, bottom: cy + r },
                r, ctx.pal.get(PRIMARY)?)?;
        }
    }

    // 错误条（3 秒）
    if let Some((msg, at)) = &app.error {
        if at.elapsed().as_millis() < 3000 {
            draw_text_simple(&rt, &ctx.f_cap, msg,
                D2D_RECT_F { left: search.x, top: search.bottom() + 4.0, right: search.right(), bottom: search.bottom() + 24.0 },
                ctx.pal.get(DANGER)?);
        }
    }

    // 3. 应用网格
    let n_apps = app.input.hits.len();
    let grid = RectF {
        x: l.grid.x - content.x + SHADOW_MARGIN,
        y: l.grid.y - content.y + SHADOW_MARGIN,
        w: l.grid.w,
        h: l.grid.h,
    };
    if n_apps == 0 {
        let msg = if !app.snap.ready || app.input.query.is_empty() {
            "正在读取应用…".to_string()
        } else {
            format!("没有找到“{}”", app.input.query)
        };
        draw_text_simple(&rt, &ctx.f_body, &msg,
            D2D_RECT_F { left: grid.x, top: grid.y + grid.h / 2.0 - 12.0, right: grid.right(), bottom: grid.y + grid.h / 2.0 + 12.0 },
            ctx.pal.get(TEXT_SECONDARY)?);
    } else {
        let cap = l.visible_app_capacity();
        for vi in 0..cap {
            let idx = app.input.scroll_row * l.cols + vi;
            if idx >= n_apps { break; }
            let app_id = app.input.hits[idx];
            let Some(entry) = app.snap.apps.get(app_id) else { continue };
            let cell = l.app_cell(idx, app.input.scroll_row);
            let cell = RectF { x: cell.x - content.x + SHADOW_MARGIN, y: cell.y - content.y + SHADOW_MARGIN, w: cell.w, h: cell.h };
            let is_sel = app.input.focus == FocusZone::Apps && app.input.selected == Some(idx);
            let is_hover = matches!(app.input.hover, Some(Hit::App(i)) if i == idx);
            if is_sel {
                // 选中：1px PRIMARY 边框 + SELECT_BG（内层圆角 = 面板 10 − 4 = 6）
                fill_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&cell), RADIUS_ITEM, ctx.pal.get(SELECT_BG)?)?;
                draw_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&cell), RADIUS_ITEM, ctx.pal.get(PRIMARY)?, 1.0)?;
            } else if is_hover {
                // 先阴影后填充
                draw_soft_shadow(factory, &mut ctx.tiles, &rt, to_d2d(&cell), RADIUS_ITEM, SHADOW_L2.0, SHADOW_L2.1, SHADOW_L2.2, rt.dpi_scale())?;
                fill_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&cell), RADIUS_ITEM, ctx.pal.get(INPUT)?)?;
            }
            // 图标 62×62，水平居中，距格顶 10；hover/选中时上移 2px
            let up = if is_hover || is_sel { 2.0 } else { 0.0 };
            let ir = icon_rect(&cell, up);
            let bitmap = ctx.icons.get(&rt, &ctx.wic, entry);
            match bitmap {
                Some(bmp) => {
                    // SAFETY: 位图由 icons 缓存持有，绘制期间有效。
                    unsafe { rt.DrawBitmap(&bmp, Some(&D2D_RECT_F { left: ir.x, top: ir.y, right: ir.x + ir.w, bottom: ir.y + ir.h }), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) };
                }
                None => {
                    fill_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&ir), RADIUS_ITEM, ctx.pal.get(BORDER)?)?;
                }
            }
            // 名称
            let name_rect = D2D_RECT_F {
                left: cell.x + 4.0,
                top: cell.y + 80.0,
                right: cell.x + cell.w - 4.0,
                bottom: cell.y + 80.0 + 16.0,
            };
            draw_text_trimmed(&rt, &ctx.dwrite, &ctx.f_cap_c, &entry.name, name_rect, ctx.pal.get(TEXT_PRIMARY)?);
        }
        // 滚动条（内容超出时，右侧 4px 内）
        let total_rows = n_apps.div_ceil(l.cols);
        if total_rows > l.visible_rows && l.visible_rows > 0 {
            let track_h = grid.h;
            let thumb_h = (track_h * l.visible_rows as f32 / total_rows as f32).max(12.0);
            let max_scroll = (total_rows - l.visible_rows) as f32;
            let t = (app.input.scroll_row as f32 / max_scroll).clamp(0.0, 1.0);
            let y = grid.y + t * (track_h - thumb_h);
            fill_smooth_rect(factory, &mut ctx.geo, &rt,
                D2D_RECT_F { left: grid.right() - 4.0, top: y, right: grid.right() - 1.0, bottom: y + thumb_h },
                1.0, ctx.pal.get(BORDER_HOVER)?)?;
        }
    }

    // 4. 文件卡（列表 / 图标两种视图，右上角切换）
    let files_t = win_rect(&l.files_card, &content);
    draw_text_simple(&rt, &ctx.f_title, "最近文件",
        D2D_RECT_F { left: files_t.left + 16.0, top: files_t.top + 12.0, right: files_t.right - 16.0 - 60.0, bottom: files_t.top + 36.0 },
        ctx.pal.get(TEXT_PRIMARY)?);
    draw_view_toggle(ctx, factory, &rt, &l.toggle_rect(FocusZone::Files), &content, l.files_view)?;
    if app.snap.recent.is_empty() {
        draw_text_simple(&rt, &ctx.f_body, "最近没有打开过文件",
            D2D_RECT_F { left: files_t.left + 16.0, top: files_t.top + 48.0, right: files_t.right - 16.0, bottom: files_t.top + 72.0 },
            ctx.pal.get(TEXT_SECONDARY)?);
    }
    let scale = rt.dpi_scale();
    for i in 0..l.visible_files {
        let Some(e) = app.snap.recent.get(i) else { break };
        let r = l.file_row(i);
        let cell = RectF { x: r.x - content.x + SHADOW_MARGIN, y: r.y - content.y + SHADOW_MARGIN, w: r.w, h: r.h };
        let hovered = matches!(app.input.hover, Some(Hit::File(j)) if j == i);
        let sel = app.input.focus == FocusZone::Files && app.input.selected == Some(i);
        let hot = hovered || sel;
        match l.files_view {
            ViewMode::List => {
                if hot {
                    fill_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&cell), RADIUS_ITEM, ctx.pal.get(if sel { SELECT_BG } else { ROW_HOVER })?)?;
                }
                let thumb = e.thumb_png.as_deref().and_then(|p| ctx.icons.get_path(&rt, &ctx.wic, p));
                if let Some(b) = thumb {
                    let bx = D2D_RECT_F { left: cell.x + 8.0, top: cell.y + 6.0, right: cell.x + 32.0, bottom: cell.y + 30.0 };
                    crate::fileicons::draw_thumb(&rt, factory, &mut ctx.geo, &ctx.pal, &b, bx, 4.0, false)?;
                } else {
                    let st = crate::fileicons::doc_style(&e.ext, e.kind);
                    crate::fileicons::draw_doc_tile(&rt, factory, &ctx.pal, cell.x + 11.0, cell.y + 6.5, 0.55, &st, false)?;
                }
                let name_rect = D2D_RECT_F { left: cell.x + 40.0, top: cell.y, right: cell.x + cell.w - 16.0 - 64.0, bottom: cell.bottom() };
                draw_text_trimmed(&rt, &ctx.dwrite, &ctx.f_body, &e.name, name_rect, ctx.pal.get(TEXT_PRIMARY)?);
                let time_rect = D2D_RECT_F { left: cell.right() - 64.0 - 8.0, top: cell.y, right: cell.right() - 8.0, bottom: cell.bottom() };
                draw_text_simple(&rt, &ctx.f_small_r, &crate::fmt::rel_time(e.mtime + app.tz_offset, &app.now_local), time_rect, ctx.pal.get(TEXT_TERTIARY)?);
            }
            ViewMode::Grid => {
                grid_cell_state(ctx, factory, &rt, &cell, hovered, sel)?;
                let up = if hot { 2.0 } else { 0.0 };
                let cx = cell.x + cell.w / 2.0;
                let thumb = e.thumb_png.as_deref().and_then(|p| ctx.icons.get_path(&rt, &ctx.wic, p));
                if let Some(b) = thumb {
                    let bx = D2D_RECT_F { left: cx - 34.0, top: cell.y + 8.0 - up, right: cx + 34.0, bottom: cell.y + 53.0 - up };
                    crate::fileicons::draw_thumb(&rt, factory, &mut ctx.geo, &ctx.pal, &b, bx, 6.0, e.kind == FileKind::Video)?;
                } else {
                    let st = crate::fileicons::doc_style(&e.ext, e.kind);
                    if let Some(tag) = crate::fileicons::draw_doc_tile(&rt, factory, &ctx.pal, cx - 17.0, cell.y + 9.0 - up, 1.0, &st, true)? {
                        let label: String = e.ext.chars().take(4).collect::<String>().to_uppercase();
                        draw_text_simple(&rt, &ctx.f_tag, &label, tag, ctx.pal.get(st.tag_fg)?);
                    }
                }
                draw_text_trimmed(&rt, &ctx.dwrite, &ctx.f_cap_c, &e.name,
                    D2D_RECT_F { left: cell.x + 3.0, top: cell.y + 58.0, right: cell.right() - 3.0, bottom: cell.y + 74.0 },
                    ctx.pal.get(TEXT_PRIMARY)?);
            }
        }
    }

    // 5. 文件夹卡
    let dirs_t = win_rect(&l.dirs_card, &content);
    draw_text_simple(&rt, &ctx.f_title, "常用文件夹",
        D2D_RECT_F { left: dirs_t.left + 16.0, top: dirs_t.top + 12.0, right: dirs_t.right - 16.0 - 60.0, bottom: dirs_t.top + 36.0 },
        ctx.pal.get(TEXT_PRIMARY)?);
    draw_view_toggle(ctx, factory, &rt, &l.toggle_rect(FocusZone::Dirs), &content, l.dirs_view)?;
    for (i, d) in app.snap.dirs.iter().enumerate() {
        let Some(r) = l.dir_cell(i) else { break };
        let cell = RectF { x: r.x - content.x + SHADOW_MARGIN, y: r.y - content.y + SHADOW_MARGIN, w: r.w, h: r.h };
        let hovered = matches!(app.input.hover, Some(Hit::Dir(j)) if j == i);
        let sel = app.input.focus == FocusZone::Dirs && app.input.selected == Some(i);
        let hot = hovered || sel;
        let ink = if hot { PRIMARY } else { ICON };
        match l.dirs_view {
            ViewMode::List => {
                if hot {
                    fill_smooth_rect(factory, &mut ctx.geo, &rt, to_d2d(&cell), RADIUS_ITEM, ctx.pal.get(if sel { SELECT_BG } else { ROW_HOVER })?)?;
                }
                crate::fileicons::draw_folder_icon(&rt, factory, &ctx.pal, cell.x + 10.0, cell.y + 8.0, 20.0, ink, &d.name)?;
                draw_text_simple(&rt, &ctx.f_body, &d.name,
                    D2D_RECT_F { left: cell.x + 40.0, top: cell.y, right: cell.right() - 72.0, bottom: cell.bottom() },
                    ctx.pal.get(TEXT_PRIMARY)?);
                draw_text_simple(&rt, &ctx.f_small_r, &format!("{} 项", d.count),
                    D2D_RECT_F { left: cell.right() - 72.0, top: cell.y, right: cell.right() - 8.0, bottom: cell.bottom() },
                    ctx.pal.get(TEXT_TERTIARY)?);
            }
            ViewMode::Grid => {
                grid_cell_state(ctx, factory, &rt, &cell, hovered, sel)?;
                let up = if hot { 2.0 } else { 0.0 };
                let cx = cell.x + cell.w / 2.0;
                crate::fileicons::draw_folder_tile(&rt, factory, &mut ctx.geo, &mut ctx.tiles, &ctx.pal, scale, cx - 23.0, cell.y + 8.0 - up, hot, &d.name)?;
                draw_text_trimmed(&rt, &ctx.dwrite, &ctx.f_cap_c, &d.name,
                    D2D_RECT_F { left: cell.x + 3.0, top: cell.y + 58.0, right: cell.right() - 3.0, bottom: cell.y + 74.0 },
                    ctx.pal.get(TEXT_PRIMARY)?);
            }
        }
    }
    Ok(())
}

/// 图标矩形：62×62，水平居中，距格顶 10；up 为上移量。
fn icon_rect(cell: &RectF, up: f32) -> RectF {
    RectF { x: cell.x + (cell.w - 62.0) / 2.0, y: cell.y + 10.0 - up, w: 62.0, h: 62.0 }
}

/// 图标视图格子的状态底：与应用格一致（悬停白底 + L2 阴影，选中浅蓝底 + 1px 主色边）。
fn grid_cell_state(ctx: &mut RenderCtx, factory: &ID2D1Factory, rt: &ID2D1RenderTarget, cell: &RectF, hovered: bool, sel: bool) -> Result<()> {
    if sel {
        fill_smooth_rect(factory, &mut ctx.geo, rt, to_d2d(cell), RADIUS_ITEM, ctx.pal.get(SELECT_BG)?)?;
        draw_smooth_rect(factory, &mut ctx.geo, rt, to_d2d(cell), RADIUS_ITEM, ctx.pal.get(PRIMARY)?, 1.0)?;
    } else if hovered {
        draw_soft_shadow(factory, &mut ctx.tiles, rt, to_d2d(cell), RADIUS_ITEM, SHADOW_L2.0, SHADOW_L2.1, SHADOW_L2.2, rt.dpi_scale())?;
        fill_smooth_rect(factory, &mut ctx.geo, rt, to_d2d(cell), RADIUS_ITEM, ctx.pal.get(INPUT)?)?;
    }
    Ok(())
}

/// 卡片右上角的视图切换：一个 52×24 的分段按钮，左 = 图标视图，右 = 列表视图。
fn draw_view_toggle(ctx: &mut RenderCtx, factory: &ID2D1Factory, rt: &ID2D1RenderTarget, r: &RectF, content: &RectF, mode: ViewMode) -> Result<()> {
    let x = r.x - content.x + SHADOW_MARGIN;
    let y = r.y - content.y + SHADOW_MARGIN;
    let outer = D2D_RECT_F { left: x, top: y, right: x + r.w, bottom: y + r.h };
    fill_smooth_rect(factory, &mut ctx.geo, rt, outer, RADIUS_INPUT, ctx.pal.get(INPUT)?)?;
    draw_smooth_rect(factory, &mut ctx.geo, rt, outer, RADIUS_INPUT, ctx.pal.get(BORDER)?, 1.0)?;
    let half = r.w / 2.0;
    let sel_x = if mode == ViewMode::Grid { x } else { x + half };
    fill_smooth_rect(factory, &mut ctx.geo, rt, D2D_RECT_F { left: sel_x + 2.0, top: y + 2.0, right: sel_x + half - 2.0, bottom: y + r.h - 2.0 }, RADIUS_INPUT - 2.0, ctx.pal.get(SELECT_BG)?)?;
    let grid_ink = ctx.pal.get(if mode == ViewMode::Grid { PRIMARY } else { ICON })?.clone();
    let list_ink = ctx.pal.get(if mode == ViewMode::List { PRIMARY } else { ICON })?.clone();
    let (gx, gy) = (x + half / 2.0, y + r.h / 2.0);
    // SAFETY: 画刷有效。
    unsafe {
        for (dx, dy) in [(-4.5f32, -4.5f32), (0.5, -4.5), (-4.5, 0.5), (0.5, 0.5)] {
            let q = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: gx + dx, top: gy + dy, right: gx + dx + 4.0, bottom: gy + dy + 4.0 }, radiusX: 1.0, radiusY: 1.0 };
            rt.FillRoundedRectangle(&q, &grid_ink);
        }
        let lx = x + half + half / 2.0;
        for dy in [-3.5f32, 0.0, 3.5] {
            rt.DrawLine(D2D_POINT_2F { x: lx - 5.0, y: gy + dy }, D2D_POINT_2F { x: lx + 5.0, y: gy + dy }, &list_ink, 1.4, None);
        }
    }
    Ok(())
}

fn to_d2d(r: &RectF) -> D2D_RECT_F {
    D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() }
}


/// 桌面角标按钮整帧绘制（40×40 主体 + 阴影边距）。
pub fn draw_corner(
    _app: &crate::app::App,
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    hover: bool,
) -> Result<()> {
    let rt = ctx.rt.clone();
    unsafe { rt.BeginDraw() };
    unsafe { rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })) };
    let r = (|| -> Result<()> {
        // 任务栏内嵌形态：无背景、无边框、无阴影，只画产品图形（questions.md #33）。
        // 任务栏本身有底色，白色卡片会显得像贴上去的一块，破坏"像系统自带"的目标。
        let size = crate::corner::CORNER_SIZE;
        // 命中区底：alpha=1 的近乎透明填充。分层窗口按像素 alpha 命中测试，四块图形
        // 之间的缝隙会漏到任务栏上，点起来发飘；铺一层看不见的底让整块 28 DIP 都能点。
        fill_smooth_rect(factory, &mut ctx.geo, &rt, D2D_RECT_F { left: 0.0, top: 0.0, right: size, bottom: size }, RADIUS_ITEM, ctx.pal.get(HIT_SHIELD)?)?;
        let ink = if hover { PRIMARY_HOVER } else { PRIMARY };
        crate::glyph::draw_glyph(&rt, factory, &mut ctx.geo, ctx.pal.get(ink)?, size / 2.0, size / 2.0, crate::corner::GLYPH_SIZE)?;
        Ok(())
    })();
    let end = unsafe { rt.EndDraw(None, None) };
    r?;
    end?;
    Ok(())
}

/// 首次引导气泡（tofour Toast，白底 1px 灰边框 8px 圆角 L2 阴影）。
pub fn draw_bubble(app: &crate::app::App, ctx: &mut RenderCtx, factory: &ID2D1Factory, text: &str) -> Result<()> {
    let rt = ctx.rt.clone();
    unsafe { rt.BeginDraw() };
    unsafe { rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })) };
    let r = (|| -> Result<()> {
        let m = SHADOW_MARGIN;
        let (w, h) = bubble_size();
        let body = D2D_RECT_F { left: m, top: m, right: m + w, bottom: m + h };
        draw_soft_shadow(factory, &mut ctx.tiles, &rt, body, RADIUS_TOAST, SHADOW_L2.0, SHADOW_L2.1, SHADOW_L2.2, rt.dpi_scale())?;
        fill_smooth_rect(factory, &mut ctx.geo, &rt, body, RADIUS_TOAST, ctx.pal.get(INPUT)?)?;
        draw_smooth_rect(factory, &mut ctx.geo, &rt, body, RADIUS_TOAST, ctx.pal.get(BORDER)?, 1.0)?;
        draw_text_simple(&rt, &ctx.f_body, text,
            D2D_RECT_F { left: m + 12.0, top: m + 8.0, right: m + w - 12.0, bottom: m + h - 8.0 },
            ctx.pal.get(TEXT_PRIMARY)?);
        Ok(())
    })();
    let end = unsafe { rt.EndDraw(None, None) };
    r?;
    end?;
    let _ = app;
    Ok(())
}

/// 气泡内容尺寸（DIP）。
pub fn bubble_size() -> (f32, f32) {
    (250.0, 46.0)
}
