//! 文件 / 文件夹图标：单色线性符号（无底板、统一线宽、圆头圆角），默认用 ICON 灰，悬停/选中用主色。
//! 走「少即是多」的路子：右侧两张卡只靠排版和一根细线符号区分信息，不再用彩色色块。
//! 全部矢量绘制，不使用任何系统或第三方图标资源。坐标单位 DIP。
use crate::render::Palette;
use crate::smooth::{draw_soft_shadow, fill_smooth_rect, GeoCache};
use crate::theme::{BORDER_HOVER, INPUT, SHADOW_L1};
use std::collections::HashMap;
use deskpad_engine::FileKind;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

/// 图标视图用到的额外颜色（预建画刷）。全部是 tofour 的低饱和中性色，
/// 蓝色只出现在悬停/选中（规范 02：蓝色用于交互，不用于大面积装饰）。
pub const FOLD: u32 = 0xFFEEF1F5;
/// 文件夹后板 / 前板 / 描边：在 Background #F5F6F7 与 Icon #65758B 之间取的冷灰阶。
pub const FOLDER_BACK: u32 = 0xFFD7DEE7;
pub const FOLDER_FRONT: u32 = 0xFFEAEEF3;
pub const FOLDER_EDGE: u32 = 0xFFC6CFDA;
/// 前板上的「压印」符号：比面板深一档的灰蓝 + 下方 1px 白色错位（早期 iOS 的 letterpress 质感）。
pub const EMBOSS: u32 = 0xFF8D9AAC;
pub const EMBOSS_LIGHT: u32 = 0xB3FFFFFF;
pub const HILITE: u32 = 0xCCFFFFFF;

/// 视频缩略图上的播放标记底色（白 90%）。
pub const PLAY_BG: u32 = 0xE6FFFFFF;

pub fn extra_colors() -> Vec<u32> {
    let mut v = vec![FOLD, FOLDER_BACK, FOLDER_FRONT, FOLDER_EDGE, EMBOSS, EMBOSS_LIGHT, HILITE, PLAY_BG];
    for (bg, fg) in [TAG_BLUE, TAG_GREEN, TAG_ORANGE, TAG_RED, TAG_GRAY] {
        v.push(bg);
        v.push(fg);
    }
    v
}

#[derive(Clone, Copy)]
enum Sym {
    Lines,
    Sheet,
    DocText,
    Table,
    Slides,
    Pdf,
    Photo,
    Play,
    Note,
    Zip,
    Download,
    Code,
    Desktop,
    Box,
    Folder,
}

fn folder_sym(name: &str) -> Sym {
    match name {
        "桌面" => Sym::Desktop,
        "下载" => Sym::Download,
        "文档" => Sym::DocText,
        "表格" => Sym::Table,
        "演示" => Sym::Slides,
        "PDF" => Sym::Pdf,
        "图片" => Sym::Photo,
        "视频" => Sym::Play,
        "音频" => Sym::Note,
        "压缩包" => Sym::Zip,
        "安装包" => Sym::Box,
        "代码" => Sym::Code,
        _ => Sym::Folder,
    }
}

/// 文件夹符号：常用文件夹用对应符号，其余用文件夹轮廓。
#[allow(clippy::too_many_arguments)]
pub fn draw_folder_icon(rt: &ID2D1RenderTarget, factory: &ID2D1Factory, pal: &Palette, x: f32, y: f32, size: f32, color: u32, name: &str) -> Result<()> {
    Pen::new(rt, factory, pal.get(color)?, x, y, size)?.symbol(folder_sym(name))
}

/// 在 24×24 的符号网格里作画（符号占中间约 14×14），自动缩放到图标尺寸。
struct Pen<'a> {
    rt: &'a ID2D1RenderTarget,
    factory: &'a ID2D1Factory,
    b: &'a ID2D1Brush,
    stroke: ID2D1StrokeStyle,
    ox: f32,
    oy: f32,
    k: f32,
}

impl<'a> Pen<'a> {
    fn new(rt: &'a ID2D1RenderTarget, factory: &'a ID2D1Factory, b: &'a ID2D1Brush, x: f32, y: f32, size: f32) -> Result<Self> {
        let props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        // SAFETY: 标准工厂调用，属性为值。
        let stroke = unsafe { factory.CreateStrokeStyle(&props, None)? };
        Ok(Pen { rt, factory, b, stroke, ox: x, oy: y, k: size / 24.0 })
    }
    fn p(&self, x: f32, y: f32) -> D2D_POINT_2F {
        D2D_POINT_2F { x: self.ox + x * self.k, y: self.oy + y * self.k }
    }
    fn w(&self) -> f32 {
        (1.5 * self.k).max(1.1)
    }
    fn poly(&self, pts: &[(f32, f32)], closed: bool, filled: bool) -> Result<()> {
        // SAFETY: 几何在本函数内建立并关闭；画刷、线型有效。
        unsafe {
            let g = self.factory.CreatePathGeometry()?;
            let s = g.Open()?;
            s.BeginFigure(self.p(pts[0].0, pts[0].1), if filled { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
            for &(x, y) in &pts[1..] {
                s.AddLine(self.p(x, y));
            }
            s.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            s.Close()?;
            if filled {
                self.rt.FillGeometry(&g, self.b, None);
            } else {
                self.rt.DrawGeometry(&g, self.b, self.w(), &self.stroke);
            }
        }
        Ok(())
    }
    fn line(&self, a: (f32, f32), b: (f32, f32)) {
        // SAFETY: 画刷、线型有效。
        unsafe { self.rt.DrawLine(self.p(a.0, a.1), self.p(b.0, b.1), self.b, self.w(), &self.stroke) };
    }
    fn rrect(&self, l: f32, t: f32, r: f32, b: f32, rad: f32) {
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F { left: self.ox + l * self.k, top: self.oy + t * self.k, right: self.ox + r * self.k, bottom: self.oy + b * self.k },
            radiusX: rad * self.k,
            radiusY: rad * self.k,
        };
        // SAFETY: 同上。
        unsafe { self.rt.DrawRoundedRectangle(&rr, self.b, self.w(), &self.stroke) };
    }
    fn dot(&self, x: f32, y: f32, r: f32) {
        let e = D2D1_ELLIPSE { point: self.p(x, y), radiusX: r * self.k, radiusY: r * self.k };
        // SAFETY: 画刷有效。
        unsafe { self.rt.FillEllipse(&e, self.b) };
    }
    fn page(&self) -> Result<()> {
        self.poly(&[(7.5, 5.0), (13.5, 5.0), (17.0, 8.5), (17.0, 19.0), (7.5, 19.0)], true, false)?;
        self.poly(&[(13.5, 5.0), (13.5, 8.5), (17.0, 8.5)], false, false)
    }
    fn symbol(&self, sym: Sym) -> Result<()> {
        match sym {
            // 纸面内的正文行（不画纸张轮廓）
            Sym::Lines => {
                self.line((5.0, 7.0), (19.0, 7.0));
                self.line((5.0, 11.0), (19.0, 11.0));
                self.line((5.0, 15.0), (19.0, 15.0));
                self.line((5.0, 19.0), (13.0, 19.0));
            }
            // 纸面内的表格：表头一行 + 三列
            Sym::Sheet => {
                self.rrect(4.5, 5.0, 19.5, 19.5, 1.5);
                self.line((4.5, 9.5), (19.5, 9.5));
                self.line((4.5, 14.5), (19.5, 14.5));
                self.line((9.5, 5.0), (9.5, 19.5));
                self.line((14.5, 5.0), (14.5, 19.5));
            }
            Sym::DocText => {
                self.page()?;
                self.line((10.0, 12.0), (14.5, 12.0));
                self.line((10.0, 15.0), (14.5, 15.0));
            }
            Sym::Pdf => {
                self.page()?;
                self.poly(&[(9.8, 16.5), (12.2, 11.0), (14.6, 16.5)], false, false)?;
            }
            Sym::Table => {
                self.rrect(5.5, 6.5, 18.5, 17.5, 2.0);
                self.line((5.5, 10.5), (18.5, 10.5));
                self.line((10.5, 10.5), (10.5, 17.5));
            }
            Sym::Slides => {
                self.rrect(5.0, 6.0, 19.0, 15.5, 2.0);
                self.line((12.0, 15.5), (12.0, 18.5));
                self.line((9.0, 18.5), (15.0, 18.5));
            }
            Sym::Photo => {
                self.rrect(5.0, 6.0, 19.0, 18.0, 2.5);
                self.poly(&[(6.5, 16.0), (10.0, 12.0), (12.5, 14.5), (14.5, 12.5), (17.5, 16.0)], false, false)?;
                self.dot(15.0, 9.5, 1.3);
            }
            Sym::Play => {
                self.poly(&[(9.0, 6.5), (17.5, 12.0), (9.0, 17.5)], true, true)?;
            }
            Sym::Note => {
                self.line((14.5, 6.0), (14.5, 15.5));
                self.poly(&[(14.5, 6.0), (18.0, 7.5), (18.0, 9.5), (14.5, 8.2)], true, true)?;
                self.dot(11.8, 15.8, 2.8);
            }
            Sym::Zip => {
                self.rrect(6.5, 5.0, 17.5, 19.0, 2.5);
                self.line((11.0, 7.0), (12.5, 7.0));
                self.line((11.5, 9.0), (13.0, 9.0));
                self.line((11.0, 11.0), (12.5, 11.0));
                self.rrect(10.5, 13.5, 13.5, 16.5, 0.8);
            }
            Sym::Download => {
                self.line((12.0, 5.5), (12.0, 15.0));
                self.poly(&[(8.0, 11.0), (12.0, 15.0), (16.0, 11.0)], false, false)?;
                self.line((6.5, 18.5), (17.5, 18.5));
            }
            Sym::Code => {
                self.poly(&[(9.0, 8.0), (5.0, 12.0), (9.0, 16.0)], false, false)?;
                self.poly(&[(15.0, 8.0), (19.0, 12.0), (15.0, 16.0)], false, false)?;
                self.line((13.2, 6.5), (10.8, 17.5));
            }
            Sym::Desktop => {
                self.rrect(4.5, 5.5, 19.5, 15.5, 2.0);
                self.line((12.0, 15.5), (12.0, 18.5));
                self.line((8.5, 18.5), (15.5, 18.5));
            }
            Sym::Box => {
                self.poly(&[(12.0, 4.8), (18.5, 8.3), (18.5, 15.7), (12.0, 19.2), (5.5, 15.7), (5.5, 8.3)], true, false)?;
                self.poly(&[(5.5, 8.3), (12.0, 11.8), (18.5, 8.3)], false, false)?;
                self.line((12.0, 11.8), (12.0, 19.2));
            }
            Sym::Folder => {
                self.poly(&[(5.0, 8.0), (5.0, 6.5), (10.0, 6.5), (11.5, 8.0), (19.0, 8.0), (19.0, 17.5), (5.0, 17.5)], true, false)?;
                self.line((5.0, 10.5), (19.0, 10.5));
            }
        }
        Ok(())
    }
}

/// 设置按钮的线性齿轮（24 栅格：8 齿外轮廓 + 中心圆），(x, y) 左上角。
pub fn draw_gear(rt: &ID2D1RenderTarget, factory: &ID2D1Factory, pal: &Palette, x: f32, y: f32, size: f32, color: u32) -> Result<()> {
    let pen = Pen::new(rt, factory, pal.get(color)?, x, y, size)?;
    let (c, r_in, r_out) = (12.0f32, 7.0f32, 9.5f32);
    let mut pts = Vec::with_capacity(32);
    for i in 0..8 {
        let a = (i as f32) * std::f32::consts::FRAC_PI_4;
        for (da, r) in [(-0.36f32, r_in), (-0.2, r_out), (0.2, r_out), (0.36, r_in)] {
            pts.push((c + r * (a + da).cos(), c + r * (a + da).sin()));
        }
    }
    pen.poly(&pts, true, false)?;
    let e = D2D1_ELLIPSE { point: pen.p(c, c), radiusX: 3.0 * pen.k, radiusY: 3.0 * pen.k };
    // SAFETY: 画刷、线型有效。
    unsafe { rt.DrawEllipse(&e, pen.b, pen.w(), &pen.stroke) };
    Ok(())
}

/// 文件类型的配色：纸面图形主色 + 底部类型标签（底色 / 字色）。
/// 标签色直接取 tofour 第 14 节「状态标签」的四组低饱和配色，其余类型用中性灰。
pub struct DocStyle {
    pub ink: u32,
    pub tag_bg: u32,
    pub tag_fg: u32,
    sym: Sym,
}

pub const TAG_BLUE: (u32, u32) = (0xFFEAF2FF, 0xFF3878D8);
pub const TAG_GREEN: (u32, u32) = (0xFFEAF7ED, 0xFF4DAA5A);
pub const TAG_ORANGE: (u32, u32) = (0xFFFFF5E3, 0xFFD99124);
pub const TAG_RED: (u32, u32) = (0xFFFDECEC, 0xFFD95C5C);
pub const TAG_GRAY: (u32, u32) = (0xFFF0F2F5, 0xFF65758B);

pub fn doc_style(ext: &str, kind: FileKind) -> DocStyle {
    let (tag, sym) = match (ext, kind) {
        ("doc" | "docx" | "rtf" | "odt" | "wps", _) => (TAG_BLUE, Sym::Lines),
        (_, FileKind::Document) => (TAG_GRAY, Sym::Lines),
        (_, FileKind::Spreadsheet) => (TAG_GREEN, Sym::Sheet),
        (_, FileKind::Presentation) => (TAG_ORANGE, Sym::Slides),
        (_, FileKind::Pdf) => (TAG_RED, Sym::Lines),
        (_, FileKind::Image) => (TAG_GRAY, Sym::Photo),
        (_, FileKind::Video) => (TAG_GRAY, Sym::Play),
        (_, FileKind::Audio) => (TAG_GRAY, Sym::Note),
        (_, FileKind::Archive) => (TAG_GRAY, Sym::Zip),
        (_, FileKind::Installer) => (TAG_GRAY, Sym::Box),
        (_, FileKind::Code) => (TAG_GRAY, Sym::Code),
        (_, FileKind::Other) => (TAG_GRAY, Sym::Lines),
    };
    DocStyle { ink: tag.1, tag_bg: tag.0, tag_fg: tag.1, sym }
}

/// 扁平文件图标：白纸 + 折角 + 1px 细边（无阴影），纸面上半是类型图形（Word 是正文行、Excel 是表格、
/// PPT 是幻灯片……），底部是扩展名标签。k = 缩放（1.0 → 34×42）；show_tag=false 用于列表里的小图标。
/// 标签文字由调用方用 DWrite 画在返回的矩形里。
#[allow(clippy::too_many_arguments)]
pub fn draw_doc_tile(
    rt: &ID2D1RenderTarget,
    factory: &ID2D1Factory,
    pal: &Palette,
    x: f32,
    y: f32,
    k: f32,
    st: &DocStyle,
    show_tag: bool,
) -> Result<Option<D2D_RECT_F>> {
    let (w, h, r, f) = (34.0 * k, 42.0 * k, 3.0 * k, 10.0 * k);
    let p = |px: f32, py: f32| D2D_POINT_2F { x: px, y: py };
    let arc = |px: f32, py: f32| D2D1_ARC_SEGMENT {
        point: p(px, py),
        size: D2D_SIZE_F { width: r, height: r },
        rotationAngle: 0.0,
        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
        arcSize: D2D1_ARC_SIZE_SMALL,
    };
    // SAFETY: 几何在本函数内建立并关闭；画刷有效。
    unsafe {
        let g = factory.CreatePathGeometry()?;
        let sk = g.Open()?;
        sk.BeginFigure(p(x + r, y), D2D1_FIGURE_BEGIN_FILLED);
        sk.AddLine(p(x + w - f, y));
        sk.AddLine(p(x + w, y + f));
        sk.AddLine(p(x + w, y + h - r));
        sk.AddArc(&arc(x + w - r, y + h));
        sk.AddLine(p(x + r, y + h));
        sk.AddArc(&arc(x, y + h - r));
        sk.AddLine(p(x, y + r));
        sk.AddArc(&arc(x + r, y));
        sk.EndFigure(D2D1_FIGURE_END_CLOSED);
        sk.Close()?;
        rt.FillGeometry(&g, pal.get(INPUT)?, None);
        rt.DrawGeometry(&g, pal.get(BORDER_HOVER)?, 1.0, None);
        let fg = factory.CreatePathGeometry()?;
        let fs = fg.Open()?;
        fs.BeginFigure(p(x + w - f, y), D2D1_FIGURE_BEGIN_FILLED);
        fs.AddLine(p(x + w - f, y + f));
        fs.AddLine(p(x + w, y + f));
        fs.EndFigure(D2D1_FIGURE_END_CLOSED);
        fs.Close()?;
        rt.FillGeometry(&fg, pal.get(FOLD)?, None);
        rt.DrawGeometry(&fg, pal.get(BORDER_HOVER)?, 1.0, None);
    }
    // 类型图形：纸面上部居中
    let gs = if show_tag { 20.0 * k } else { 22.0 * k };
    let gy = if show_tag { y + 5.0 * k } else { y + (h - gs) / 2.0 + 2.0 * k };
    Pen::new(rt, factory, pal.get(st.ink)?, x + (w - gs) / 2.0, gy, gs)?.symbol(st.sym)?;
    if !show_tag {
        return Ok(None);
    }
    let tag = D2D_RECT_F { left: x + 3.0 * k, top: y + h - 14.0 * k, right: x + w - 3.0 * k, bottom: y + h - 3.0 * k };
    let rr = D2D1_ROUNDED_RECT { rect: tag, radiusX: 2.5 * k, radiusY: 2.5 * k };
    // SAFETY: 画刷有效。
    unsafe { rt.FillRoundedRectangle(&rr, pal.get(st.tag_bg)?) };
    Ok(Some(tag))
}

/// 内容缩略图（图片 / 视频帧 / 幻灯片封面 / PDF 首页）：按比例放进 box，6px 连续曲率圆角
/// （tofour 05：图片 6px），1px 细灰边；视频再叠一个白色圆形播放标记。
#[allow(clippy::too_many_arguments)]
pub fn draw_thumb(
    rt: &ID2D1RenderTarget,
    factory: &ID2D1Factory,
    geo: &mut GeoCache,
    pal: &Palette,
    bmp: &ID2D1Bitmap,
    bx: D2D_RECT_F,
    radius: f32,
    play: bool,
) -> Result<D2D_RECT_F> {
    use crate::smooth::draw_smooth_rect;
    // SAFETY: 位图有效。
    let sz = unsafe { bmp.GetSize() };
    let (bw, bh) = (bx.right - bx.left, bx.bottom - bx.top);
    let s = (bw / sz.width.max(1.0)).min(bh / sz.height.max(1.0));
    let (dw, dh) = (sz.width * s, sz.height * s);
    let dst = D2D_RECT_F {
        left: bx.left + (bw - dw) / 2.0,
        top: bx.top + (bh - dh) / 2.0,
        right: bx.left + (bw + dw) / 2.0,
        bottom: bx.top + (bh + dh) / 2.0,
    };
    // 位图画刷：坐标相对 fill_smooth_rect 的局部原点（dst 左上角），按 s 缩放
    // SAFETY: 位图有效；画刷只在本函数内使用。
    let brush = unsafe {
        let b = rt.CreateBitmapBrush(bmp, None, None)?;
        b.SetTransform(&windows::Foundation::Numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: 0.0, M32: 0.0 });
        b
    };
    let brush: ID2D1Brush = windows::core::Interface::cast(&brush)?;
    fill_smooth_rect(factory, geo, rt, dst, radius, &brush)?;
    draw_smooth_rect(factory, geo, rt, dst, radius, pal.get(crate::theme::BORDER)?, 1.0)?;
    if play {
        let c = D2D_POINT_2F { x: (dst.left + dst.right) / 2.0, y: (dst.top + dst.bottom) / 2.0 };
        let e = D2D1_ELLIPSE { point: c, radiusX: 9.0, radiusY: 9.0 };
        // SAFETY: 画刷有效。
        unsafe { rt.FillEllipse(&e, pal.get(PLAY_BG)?) };
        Pen::new(rt, factory, pal.get(crate::theme::TEXT_PRIMARY)?, c.x - 7.0, c.y - 7.0, 14.0)?.symbol(Sym::Play)?;
    }
    Ok(dst)
}

/// 图标视图·文件夹（tofour 轻立体）：冷灰两层（后板带标签 + 前板）、1px 细边、前板顶部 1px 白色高光、
/// L1 轻阴影；常用文件夹在前板上压印单色符号（悬停/选中时符号变主色）。(x, y) 左上角，整体 46×36。
#[allow(clippy::too_many_arguments)]
pub fn draw_folder_tile(
    rt: &ID2D1RenderTarget,
    factory: &ID2D1Factory,
    geo: &mut GeoCache,
    tiles: &mut HashMap<(u32, u32), ID2D1Bitmap>,
    pal: &Palette,
    scale: f32,
    x: f32,
    y: f32,
    hot: bool,
    name: &str,
) -> Result<()> {
    use crate::smooth::draw_smooth_rect;
    let (w, h) = (46.0f32, 36.0f32);
    let tab = D2D_RECT_F { left: x, top: y, right: x + 18.0, bottom: y + 9.0 };
    let back = D2D_RECT_F { left: x, top: y + 4.0, right: x + w, bottom: y + h };
    let front = D2D_RECT_F { left: x, top: y + 9.0, right: x + w, bottom: y + h };
    draw_soft_shadow(factory, tiles, rt, back, 4.0, SHADOW_L1.0, SHADOW_L1.1, SHADOW_L1.2, scale)?;
    // 后板 + 标签（同色，描同一圈细边）
    fill_smooth_rect(factory, geo, rt, tab, 3.0, pal.get(FOLDER_BACK)?)?;
    draw_smooth_rect(factory, geo, rt, tab, 3.0, pal.get(FOLDER_EDGE)?, 1.0)?;
    fill_smooth_rect(factory, geo, rt, back, 4.0, pal.get(FOLDER_BACK)?)?;
    draw_smooth_rect(factory, geo, rt, back, 4.0, pal.get(FOLDER_EDGE)?, 1.0)?;
    // 盖住标签与后板交界处的那段描边
    fill_smooth_rect(factory, geo, rt, D2D_RECT_F { left: x + 1.0, top: y + 3.0, right: x + 17.0, bottom: y + 6.0 }, 0.0, pal.get(FOLDER_BACK)?)?;
    // 前板
    fill_smooth_rect(factory, geo, rt, front, 4.0, pal.get(FOLDER_FRONT)?)?;
    draw_smooth_rect(factory, geo, rt, front, 4.0, pal.get(FOLDER_EDGE)?, 1.0)?;
    // SAFETY: 画刷有效。
    unsafe {
        rt.DrawLine(D2D_POINT_2F { x: x + 3.5, y: y + 10.5 }, D2D_POINT_2F { x: x + w - 3.5, y: y + 10.5 }, pal.get(HILITE)?, 1.0, None);
    }
    let sym = folder_sym(name);
    if !matches!(sym, Sym::Folder) {
        let (sx, sy, sz) = (x + w / 2.0 - 9.0, y + 9.0 + (h - 9.0) / 2.0 - 9.0, 18.0);
        if hot {
            Pen::new(rt, factory, pal.get(crate::theme::PRIMARY)?, sx, sy, sz)?.symbol(sym)?;
        } else {
            // 压印：先画下移 1px 的白色，再画深一档的灰蓝
            Pen::new(rt, factory, pal.get(EMBOSS_LIGHT)?, sx, sy + 1.0, sz)?.symbol(sym)?;
            Pen::new(rt, factory, pal.get(EMBOSS)?, sx, sy, sz)?.symbol(sym)?;
        }
    }
    Ok(())
}
