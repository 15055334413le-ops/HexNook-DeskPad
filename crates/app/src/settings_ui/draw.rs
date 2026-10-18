//! 设置窗口的 D2D 绘制（UI 文档 6.11 + 第 4 节 tofour Token）。
//!
//! 布局：白底 RADIUS_MODAL + L3 阴影；左侧 160 分区导航（SURFACE 底），右侧内容区
//! 「分组标题 + 48 高设置行」。内容超出时右侧画 4px 细滚动条。
use super::model::*;
use crate::render::{DpiScale, RenderCtx};
use crate::smooth::{draw_smooth_rect, draw_soft_shadow, fill_smooth_rect};
use crate::theme::*;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

/// 主体坐标（不含阴影边距）→ 绘制坐标（加 SHADOW_MARGIN）。
pub fn at(r: D2D_RECT_F) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left + SHADOW_MARGIN,
        top: r.top + SHADOW_MARGIN,
        right: r.right + SHADOW_MARGIN,
        bottom: r.bottom + SHADOW_MARGIN,
    }
}

pub fn text(rt: &ID2D1RenderTarget, fmt: &IDWriteTextFormat, s: &str, r: D2D_RECT_F, brush: &ID2D1Brush) {
    let w: Vec<u16> = s.encode_utf16().collect();
    // SAFETY: 字符串在调用期间有效。
    unsafe { rt.DrawText(&w, fmt, &r, brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL) };
}

/// 动态行（列表项），由窗口侧从引擎快照/设置生成。
#[derive(Clone, Debug)]
pub struct DynRow {
    pub kind: DynKind,
    /// 主文字：应用名或路径。
    pub label: String,
    /// 引擎操作要用的稳定 id（setting_id 或路径字符串）。
    pub payload: String,
    pub btn: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DynKind {
    HiddenApp,
    ScanDir,
    ManualApp,
}

/// 一帧绘制的全部输入。
pub struct View<'a> {
    pub section: Section,
    pub hover: Option<super::window::Ctl>,
    pub pressed: Option<super::window::Ctl>,
    pub focus: Option<super::window::Ctl>,
    pub settings: &'a deskpad_engine::model::Settings,
    pub update: &'a crate::update_ui::UpdateState,
    pub err: Option<&'a str>,
    pub dyn_rows: &'a [DynRow],
    /// 内容区滚动偏移（DIP）。
    pub scroll: f32,
    /// 内容总高（> 可视高度时画滚动条）。
    pub content_h: f32,
}

/// 整个窗口一帧。
pub fn frame(ctx: &mut RenderCtx, factory: &ID2D1Factory, v: &View) -> Result<()> {
    let rt = ctx.rt.clone();
    let rt = &rt;
    let scale = rt.dpi_scale();
    // 底板：L3 阴影 + 白底 + 12px 平滑圆角
    let body = at(rect(0.0, 0.0, W, H));
    draw_soft_shadow(factory, &mut ctx.tiles, rt, body, RADIUS_MODAL, SHADOW_L3.0, SHADOW_L3.1, SHADOW_L3.2, scale)?;
    fill_smooth_rect(factory, &mut ctx.geo, rt, body, RADIUS_MODAL, ctx.pal.get(INPUT)?)?;

    // 左侧导航底（SURFACE）+ 右侧 1px 分隔线
    fill_smooth_rect(factory, &mut ctx.geo, rt, at(rect(0.0, 0.0, NAV_W, H)), RADIUS_MODAL, ctx.pal.get(SURFACE)?)?;
    // 导航底右边缘用直角盖回（导航区右侧不需要圆角）
    fill_smooth_rect(factory, &mut ctx.geo, rt, at(rect(NAV_W - RADIUS_MODAL, 0.0, RADIUS_MODAL, H)), 0.0, ctx.pal.get(SURFACE)?)?;
    fill_smooth_rect(factory, &mut ctx.geo, rt, at(rect(NAV_W, HEADER_H, 1.0, H - HEADER_H)), 0.0, ctx.pal.get(BORDER)?)?;

    // Header：标题 + × + 分隔线
    text(rt, &ctx.f_title, "设置", at(rect(24.0, 0.0, 200.0, HEADER_H)), ctx.pal.get(TEXT_PRIMARY)?);
    let xr = at(rect(W - 40.0, 12.0, 24.0, 24.0));
    if v.hover == Some(super::window::Ctl::CloseX) {
        fill_smooth_rect(factory, &mut ctx.geo, rt, xr, RADIUS_INPUT, ctx.pal.get(ROW_HOVER)?)?;
    }
    let x_col = if v.hover == Some(super::window::Ctl::CloseX) { TEXT_PRIMARY } else { TEXT_SECONDARY };
    text(rt, &ctx.f_cap_c, "✕", xr, ctx.pal.get(x_col)?);
    fill_smooth_rect(factory, &mut ctx.geo, rt, at(rect(0.0, HEADER_H, W, 1.0)), 0.0, ctx.pal.get(BORDER)?)?;

    // 导航项
    for (i, sec) in Section::ALL.iter().enumerate() {
        let y = HEADER_H + 12.0 + i as f32 * 40.0;
        let r = at(rect(8.0, y, NAV_W - 16.0, 36.0));
        let cur = *sec == v.section;
        let hovered = v.hover == Some(super::window::Ctl::Nav(i));
        if cur {
            fill_smooth_rect(factory, &mut ctx.geo, rt, r, RADIUS_INPUT, ctx.pal.get(SELECT_BG)?)?;
        } else if hovered {
            fill_smooth_rect(factory, &mut ctx.geo, rt, r, RADIUS_INPUT, ctx.pal.get(ROW_HOVER)?)?;
        }
        let col = if cur { PRIMARY } else { TEXT_PRIMARY };
        text(rt, &ctx.f_body, sec.label(), at(rect(24.0, y, NAV_W - 40.0, 36.0)), ctx.pal.get(col)?);
    }

    // 内容区裁剪
    let clip = at(rect(NAV_W + 1.0, HEADER_H + 1.0, W - NAV_W - 1.0, H - HEADER_H - 1.0));
    // SAFETY: Push/Pop 配对。
    unsafe { rt.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    let r = draw_content(ctx, factory, rt, v);
    // SAFETY: 与上面的 Push 配对。
    unsafe { rt.PopAxisAlignedClip() };
    r?;

    // 右侧细滚动条（4px 圆角，仅内容超出时）
    let view_h = H - HEADER_H - 1.0;
    if v.content_h > view_h + 1.0 {
        let track = at(rect(W - 8.0, HEADER_H + 8.0, 4.0, view_h - 16.0));
        let ratio = view_h / v.content_h;
        let thumb_h = (track.bottom - track.top) * ratio.clamp(0.08, 1.0);
        let max_scroll = v.content_h - view_h;
        let ty = track.top + (track.bottom - track.top - thumb_h) * (v.scroll / max_scroll.max(1.0));
        fill_smooth_rect(
            factory,
            &mut ctx.geo,
            rt,
            D2D_RECT_F { left: track.left, top: ty, right: track.right, bottom: ty + thumb_h },
            2.0,
            ctx.pal.get(BORDER_HOVER)?,
        )?;
    }

    // 焦点环：1px PRIMARY，与控件间隔 2px
    if let Some(f) = v.focus.clone() {
        if let Some(r) = super::window::ctl_rect(v.section, &f, v.settings, v.dyn_rows, v.scroll) {
            let ring = at(D2D_RECT_F { left: r.left - 2.0, top: r.top - 2.0, right: r.right + 2.0, bottom: r.bottom + 2.0 });
            draw_smooth_rect(factory, &mut ctx.geo, rt, ring, RADIUS_INPUT + 2.0, ctx.pal.get(PRIMARY)?, 1.0)?;
        }
    }
    Ok(())
}

/// 内容区（分区标题 + 行）。在裁剪内绘制。
fn draw_content(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    v: &View,
) -> Result<()> {
    let mut y = HEADER_H + 16.0 - v.scroll;
    text(rt, &ctx.f_title, v.section.label(), at(rect(CONTENT_X, y, CONTENT_W, 24.0)), ctx.pal.get(TEXT_PRIMARY)?);
    y += 36.0;

    for row in &rows(v.section) {
        match *row {
            Row::Group(label) => {
                y += 8.0;
                text(rt, &ctx.f_cap, label, at(rect(CONTENT_X, y, CONTENT_W, GROUP_H - 14.0)), ctx.pal.get(TEXT_SECONDARY)?);
                y += GROUP_H - 14.0;
            }
            Row::Item { id, title, desc, ctl } => {
                draw_row(ctx, factory, rt, v, id, title, desc, ctl, y)?;
                y += ROW_H;
                // 应用分区：排序行后接三组列表
                if v.section == Section::Apps && id == ItemId::SortMode {
                    y = draw_lists(ctx, factory, rt, v, y)?;
                }
            }
        }
    }
    if let Some(e) = v.err {
        y += 4.0;
        text(rt, &ctx.f_small, e, at(rect(CONTENT_X, y, CONTENT_W, 16.0)), ctx.pal.get(DANGER)?);
    }
    Ok(())
}

/// 应用分区的三组列表（隐藏的应用 / 额外扫描的文件夹 / 手动添加的应用）。
fn draw_lists(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    v: &View,
    mut y: f32,
) -> Result<f32> {
    draw_list_group(ctx, factory, rt, v, DynKind::HiddenApp, "隐藏的应用", "没有隐藏的应用", &mut y)?;
    draw_list_group(ctx, factory, rt, v, DynKind::ScanDir, "额外扫描的文件夹", "还没有额外扫描的文件夹", &mut y)?;
    draw_add_btn(ctx, factory, rt, v, super::window::Ctl::Btn(ItemId::AddScanDir), "添加文件夹", y)?;
    y += 40.0;
    draw_list_group(ctx, factory, rt, v, DynKind::ManualApp, "手动添加的应用", "还没有手动添加的应用", &mut y)?;
    draw_add_btn(ctx, factory, rt, v, super::window::Ctl::Btn(ItemId::AddManualApp), "添加应用", y)?;
    y += 40.0;
    Ok(y)
}

#[allow(clippy::too_many_arguments)]
fn draw_list_group(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    v: &View,
    kind: DynKind,
    title: &str,
    empty_hint: &str,
    y: &mut f32,
) -> Result<()> {
    *y += 8.0;
    text(rt, &ctx.f_cap, title, at(rect(CONTENT_X, *y, CONTENT_W, GROUP_H - 14.0)), ctx.pal.get(TEXT_SECONDARY)?);
    *y += GROUP_H - 14.0;
    let mut any = false;
    for d in v.dyn_rows.iter().filter(|d| d.kind == kind) {
        any = true;
        draw_dyn_row(ctx, factory, rt, v, d, *y)?;
        *y += 36.0;
    }
    if !any {
        text(rt, &ctx.f_small, empty_hint, at(rect(CONTENT_X, *y + 4.0, CONTENT_W, 16.0)), ctx.pal.get(TEXT_TERTIARY)?);
        *y += 26.0;
    }
    Ok(())
}

fn draw_dyn_row(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    v: &View,
    d: &DynRow,
    y: f32,
) -> Result<()> {
    let ctl = super::window::Ctl::Dyn(d.kind, d.payload.clone());
    let row = rect(CONTENT_X, y, CONTENT_W, 32.0);
    if v.hover.as_ref().map(|c| c.is_dyn_with(&d.kind, &d.payload)) == Some(true) {
        fill_smooth_rect(factory, &mut ctx.geo, rt, at(row), RADIUS_ITEM, ctx.pal.get(ROW_HOVER)?)?;
    }
    text(rt, &ctx.f_body, &d.label, at(rect(CONTENT_X + 4.0, y + 5.0, CONTENT_W - 116.0, 22.0)), ctx.pal.get(TEXT_PRIMARY)?);
    let br = rect(CONTENT_X + CONTENT_W - 96.0, y + 2.0, 88.0, 28.0);
    let hovered = v.hover == Some(ctl.clone());
    let pressed = v.pressed == Some(ctl);
    let (bg, fg) = if pressed {
        (SELECT_BG, PRIMARY)
    } else if hovered {
        (ROW_HOVER, TEXT_PRIMARY)
    } else {
        (INPUT, TEXT_PRIMARY)
    };
    fill_smooth_rect(factory, &mut ctx.geo, rt, at(br), RADIUS_INPUT, ctx.pal.get(bg)?)?;
    draw_smooth_rect(factory, &mut ctx.geo, rt, at(br), RADIUS_INPUT, ctx.pal.get(BORDER)?, 1.0)?;
    text(rt, &ctx.f_cap_c, d.btn, at(br), ctx.pal.get(fg)?);
    Ok(())
}

fn draw_add_btn(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    v: &View,
    ctl: super::window::Ctl,
    label: &str,
    y: f32,
) -> Result<()> {
    let br = rect(CONTENT_X, y + 2.0, 120.0, 28.0);
    let (bg, border, fg) = if v.pressed == Some(ctl.clone()) {
        (SELECT_BG, PRIMARY, PRIMARY)
    } else if v.hover == Some(ctl.clone()) {
        (ROW_HOVER, BORDER_HOVER, TEXT_PRIMARY)
    } else {
        (INPUT, BORDER, TEXT_PRIMARY)
    };
    fill_smooth_rect(factory, &mut ctx.geo, rt, at(br), RADIUS_INPUT, ctx.pal.get(bg)?)?;
    draw_smooth_rect(factory, &mut ctx.geo, rt, at(br), RADIUS_INPUT, ctx.pal.get(border)?, 1.0)?;
    text(rt, &ctx.f_cap_c, label, at(br), ctx.pal.get(fg)?);
    Ok(())
}

/// 一行 48 DIP：左侧标题 + 说明，右侧控件。
#[allow(clippy::too_many_arguments)]
fn draw_row(
    ctx: &mut RenderCtx,
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    v: &View,
    id: ItemId,
    title: &str,
    desc: &str,
    ctl: Control,
    y: f32,
) -> Result<()> {
    text(rt, &ctx.f_body, title, at(rect(CONTENT_X, y + 4.0, 230.0, 20.0)), ctx.pal.get(TEXT_PRIMARY)?);
    if !desc.is_empty() {
        text(rt, &ctx.f_small, desc, at(rect(CONTENT_X, y + 25.0, 280.0, 16.0)), ctx.pal.get(TEXT_SECONDARY)?);
    }
    let right_x = CONTENT_X + CONTENT_W;
    match ctl {
        Control::Toggle => {
            let on = super::apply::toggle_on(v.settings, id);
            let tr = rect(right_x - 44.0, y + (ROW_H - 20.0) / 2.0, 36.0, 20.0);
            fill_smooth_rect(factory, &mut ctx.geo, rt, at(tr), 10.0, ctx.pal.get(if on { PRIMARY } else { BORDER })?)?;
            let kx = if on { tr.left + 18.0 } else { tr.left + 2.0 };
            fill_smooth_rect(factory, &mut ctx.geo, rt, at(rect(kx, tr.top + 2.0, 16.0, 16.0)), 8.0, ctx.pal.get(INPUT)?)?;
        }
        Control::Seg(labels) => {
            let sel = super::apply::seg_selected(v.settings, id);
            let (w, h) = seg_size(labels);
            let x = right_x - w;
            let top = y + (ROW_H - h) / 2.0;
            let outer = rect(x, top, w, h);
            fill_smooth_rect(factory, &mut ctx.geo, rt, at(outer), RADIUS_INPUT, ctx.pal.get(INPUT)?)?;
            draw_smooth_rect(factory, &mut ctx.geo, rt, at(outer), RADIUS_INPUT, ctx.pal.get(BORDER)?, 1.0)?;
            let n = labels.len();
            let seg_w = w / n as f32;
            for (i, label) in labels.iter().enumerate() {
                let sr = rect(x + i as f32 * seg_w, top, seg_w, h);
                if i == sel {
                    let a = at(sr);
                    let inner = D2D_RECT_F { left: a.left + 2.0, top: a.top + 2.0, right: a.right - 2.0, bottom: a.bottom - 2.0 };
                    fill_smooth_rect(factory, &mut ctx.geo, rt, inner, RADIUS_INPUT - 2.0, ctx.pal.get(SELECT_BG)?)?;
                } else if v.hover == Some(super::window::Ctl::Seg(id, i)) {
                    fill_smooth_rect(factory, &mut ctx.geo, rt, at(sr), RADIUS_INPUT, ctx.pal.get(ROW_HOVER)?)?;
                }
                let col = if i == sel { PRIMARY } else { TEXT_PRIMARY };
                text(rt, &ctx.f_cap_c, label, at(sr), ctx.pal.get(col)?);
            }
        }
        Control::Btn(label) => {
            // 更新行：「立即更新」是主按钮
            let (label, primary) = if id == ItemId::CheckUpdate {
                match v.update {
                    crate::update_ui::UpdateState::New(_) => ("立即更新", true),
                    crate::update_ui::UpdateState::Checking => ("正在检查…", false),
                    _ => (label, false),
                }
            } else {
                (label, false)
            };
            let w = (label.chars().count() as f32 * 13.0 + 28.0).max(96.0);
            let br = rect(right_x - w, y + (ROW_H - 30.0) / 2.0, w, 30.0);
            let c = super::window::Ctl::Btn(id);
            if primary {
                let col = if v.pressed == Some(c.clone()) {
                    PRIMARY_PRESSED
                } else if v.hover == Some(c.clone()) {
                    PRIMARY_HOVER
                } else {
                    PRIMARY
                };
                fill_smooth_rect(factory, &mut ctx.geo, rt, at(br), RADIUS_INPUT, ctx.pal.get(col)?)?;
                text(rt, &ctx.f_cap_c, label, at(br), ctx.pal.get(INPUT)?);
            } else {
                let bg = if v.hover == Some(c.clone()) { ROW_HOVER } else { INPUT };
                fill_smooth_rect(factory, &mut ctx.geo, rt, at(br), RADIUS_INPUT, ctx.pal.get(bg)?)?;
                draw_smooth_rect(factory, &mut ctx.geo, rt, at(br), RADIUS_INPUT, ctx.pal.get(BORDER)?, 1.0)?;
                text(rt, &ctx.f_cap_c, label, at(br), ctx.pal.get(TEXT_PRIMARY)?);
            }
            // 更新行按钮左边画状态文字
            if id == ItemId::CheckUpdate {
                let (s, small) = match v.update {
                    crate::update_ui::UpdateState::Unknown => (String::new(), true),
                    crate::update_ui::UpdateState::Checking => (String::new(), true),
                    crate::update_ui::UpdateState::Latest => ("已是最新".into(), true),
                    crate::update_ui::UpdateState::New(ver) => (format!("发现新版本 {ver}"), false),
                    crate::update_ui::UpdateState::Failed(e) => (format!("检查失败：{e}"), true),
                };
                if !s.is_empty() {
                    let fmt = if small { &ctx.f_small } else { &ctx.f_body };
                    text(rt, fmt, &s, at(rect(CONTENT_X + 240.0, y + 13.0, CONTENT_W - 240.0 - w - 12.0, 22.0)), ctx.pal.get(TEXT_SECONDARY)?);
                }
            }
        }
        Control::Stepper => {
            let outer = rect(right_x - 108.0, y + (ROW_H - 30.0) / 2.0, 108.0, 30.0);
            fill_smooth_rect(factory, &mut ctx.geo, rt, at(outer), RADIUS_INPUT, ctx.pal.get(INPUT)?)?;
            draw_smooth_rect(factory, &mut ctx.geo, rt, at(outer), RADIUS_INPUT, ctx.pal.get(BORDER)?, 1.0)?;
            for (dx, glyph, delta) in [(0.0f32, "−", -1i32), (78.0, "+", 1)] {
                let r = rect(outer.left + dx, outer.top, 30.0, 30.0);
                if v.hover == Some(super::window::Ctl::Stepper(id, delta)) {
                    fill_smooth_rect(factory, &mut ctx.geo, rt, at(r), RADIUS_INPUT, ctx.pal.get(ROW_HOVER)?)?;
                }
                text(rt, &ctx.f_body_c, glyph, at(r), ctx.pal.get(TEXT_PRIMARY)?);
            }
            let val = format!("{}", v.settings.max_recent);
            text(rt, &ctx.f_body_c, &val, at(rect(outer.left + 30.0, outer.top, 48.0, 30.0)), ctx.pal.get(TEXT_PRIMARY)?);
        }
        Control::Text => {
            let s = match id {
                ItemId::Version => deskpad_engine::update::CURRENT_VERSION,
                _ => "",
            };
            text(rt, &ctx.f_body, s, at(rect(right_x - 160.0, y + 13.0, 160.0, 22.0)), ctx.pal.get(TEXT_SECONDARY)?);
        }
    }
    Ok(())
}

/// 分段按钮尺寸：按最长标签取段宽，高 28。
pub fn seg_size(labels: &[&'static str]) -> (f32, f32) {
    let max = labels.iter().map(|l| l.chars().count()).max().unwrap_or(4) as f32;
    let seg = (max * 7.5 + 20.0).max(56.0);
    (seg * labels.len() as f32, 28.0)
}

/// 内容总高（DIP）：分区标题 + 行 + 列表 + 错误行余量。
pub fn content_height(sec: Section, dyn_rows: &[DynRow]) -> f32 {
    let mut h = 16.0 + 36.0; // 上边距 + 分区标题
    for row in &rows(sec) {
        match row {
            Row::Group(_) => h += 8.0 + GROUP_H - 14.0,
            Row::Item { id, .. } => {
                h += ROW_H;
                if sec == Section::Apps && *id == ItemId::SortMode {
                    // 三组列表：每组标题 + 行/空提示，扫描目录与手动应用各加一个添加按钮
                    let hidden = dyn_rows.iter().filter(|d| d.kind == DynKind::HiddenApp).count();
                    let scan = dyn_rows.iter().filter(|d| d.kind == DynKind::ScanDir).count();
                    let manual = dyn_rows.iter().filter(|d| d.kind == DynKind::ManualApp).count();
                    h += (8.0 + GROUP_H - 14.0) * 3.0;
                    h += if hidden == 0 { 26.0 } else { hidden as f32 * 36.0 };
                    h += if scan == 0 { 26.0 } else { scan as f32 * 36.0 };
                    h += 40.0;
                    h += if manual == 0 { 26.0 } else { manual as f32 * 36.0 };
                    h += 40.0;
                }
            }
        }
    }
    h + 24.0
}
