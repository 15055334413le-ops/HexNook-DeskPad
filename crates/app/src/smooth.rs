//! 连续曲率圆角 + 软阴影（移植自 D:\MyWork\HexNookIME\server\smooth_corner.cpp，几何公式与退化规则与原实现一致）。
use std::collections::HashMap;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

const K_DEG: f32 = std::f32::consts::PI / 180.0;
const CORNER_SMOOTHING: f32 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GeoKey {
    w: u32,
    h: u32,
    r: u32,
    ring: u32, // 0 = 填充；>0 = 该宽度的描边环
}

#[derive(Default)]
pub struct GeoCache {
    map: HashMap<GeoKey, ID2D1PathGeometry>,
}

#[derive(Clone, Copy)]
struct Pt {
    x: f32,
    y: f32,
}
impl Pt {
    fn at(o: Pt, e1: Pt, u: f32, e2: Pt, v: f32) -> Pt {
        Pt { x: o.x + e1.x * u + e2.x * v, y: o.y + e1.y * u + e2.y * v }
    }
}

struct RectF {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}
impl RectF {
    fn inflate(&self, by: f32) -> RectF {
        RectF { l: self.l - by, t: self.t - by, r: self.r + by, b: self.b + by }
    }
}

fn add_figure(sink: &ID2D1GeometrySink, r: &RectF, radius: f32) {
    // SAFETY: sink 由调用方在 Open/Close 之间提供，只追加几何段，不保留指针。
    unsafe {
        let budget = ((r.r - r.l).min(r.b - r.t) / 2.0).max(0.0);
        let rad = radius.min(budget);
        if rad < 0.5 {
            // 圆角太小时画直角
            sink.BeginFigure(Pt { x: r.l, y: r.t }.into(), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&[
                Pt { x: r.r, y: r.t }.into(),
                Pt { x: r.r, y: r.b }.into(),
                Pt { x: r.l, y: r.b }.into(),
            ]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            return;
        }
        let mut xi = CORNER_SMOOTHING;
        let mut p = (1.0 + xi) * rad;
        if p > budget {
            xi = (budget / rad - 1.0).max(0.0);
            p = budget;
        }
        let arc_deg = 90.0 * (1.0 - xi);
        let arc_len = (arc_deg / 2.0 * K_DEG).sin() * rad * 2.0f32.sqrt();
        let c = rad * ((90.0 - arc_deg) / 4.0 * K_DEG).tan() * (45.0 * xi * K_DEG).cos();
        let d = c * (45.0 * xi * K_DEG).tan();
        let b = (p - arc_len - c - d) / 3.0;
        let a = 2.0 * b;
        let h = 4.0 / 3.0 * (arc_deg * K_DEG / 4.0).tan();

        // 右上、右下、左下、左上。e1 沿进入角的边指向角，e2 沿离开角的边
        struct Corner { c: Pt, e1: Pt, e2: Pt }
        let corners = [
            Corner { c: Pt { x: r.r, y: r.t }, e1: Pt { x: 1.0, y: 0.0 }, e2: Pt { x: 0.0, y: 1.0 } },
            Corner { c: Pt { x: r.r, y: r.b }, e1: Pt { x: 0.0, y: 1.0 }, e2: Pt { x: -1.0, y: 0.0 } },
            Corner { c: Pt { x: r.l, y: r.b }, e1: Pt { x: -1.0, y: 0.0 }, e2: Pt { x: 0.0, y: -1.0 } },
            Corner { c: Pt { x: r.l, y: r.t }, e1: Pt { x: 0.0, y: -1.0 }, e2: Pt { x: 1.0, y: 0.0 } },
        ];
        sink.BeginFigure(Pt { x: r.l + p, y: r.t }.into(), D2D1_FIGURE_BEGIN_FILLED);
        for k in &corners {
            let start = Pt::at(k.c, k.e1, -p, k.e2, 0.0);
            sink.AddLine(start.into());
            let arc_start = Pt::at(start, k.e1, a + b + c, k.e2, d);
            sink.AddBeziers(&[D2D1_BEZIER_SEGMENT {
                point1: Pt::at(start, k.e1, a, k.e2, 0.0).into(),
                point2: Pt::at(start, k.e1, a + b, k.e2, 0.0).into(),
                point3: arc_start.into(),
            }]);
            let arc_end = Pt::at(arc_start, k.e1, arc_len, k.e2, arc_len);
            let o = Pt::at(k.c, k.e1, -rad, k.e2, rad); // 圆心
            let handle = |q: Pt, sign: f32| Pt {
                x: q.x - (q.y - o.y) * h * sign,
                y: q.y + (q.x - o.x) * h * sign,
            };
            sink.AddBeziers(&[D2D1_BEZIER_SEGMENT {
                point1: handle(arc_start, 1.0).into(),
                point2: handle(arc_end, -1.0).into(),
                point3: arc_end.into(),
            }]);
            sink.AddBeziers(&[D2D1_BEZIER_SEGMENT {
                point1: Pt::at(arc_end, k.e1, d, k.e2, c).into(),
                point2: Pt::at(arc_end, k.e1, d, k.e2, b + c).into(),
                point3: Pt::at(arc_end, k.e1, d, k.e2, a + b + c).into(),
            }]);
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }
}

impl From<Pt> for D2D_POINT_2F {
    fn from(p: Pt) -> Self {
        D2D_POINT_2F { x: p.x, y: p.y }
    }
}

fn rounded_geometry(
    factory: &ID2D1Factory,
    cache: &mut GeoCache,
    w: f32,
    h: f32,
    r: f32,
    ring: f32,
) -> Result<ID2D1PathGeometry> {
    let key = GeoKey { w: (w * 8.0) as u32, h: (h * 8.0) as u32, r: (r * 8.0) as u32, ring: (ring * 8.0) as u32 };
    if let Some(g) = cache.map.get(&key) {
        return Ok(g.clone());
    }
    let geo = unsafe { factory.CreatePathGeometry()? };
    // SAFETY: sink 由 geometry 打开，Close 后立即释放，生命周期短于 geometry。
    unsafe {
        let sink = geo.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
        let rect = RectF { l: 0.0, t: 0.0, r: w, b: h };
        if ring > 0.0 {
            add_figure(&sink, &rect.inflate(ring / 2.0), r + ring / 2.0);
            add_figure(&sink, &rect.inflate(-ring / 2.0), (r - ring / 2.0).max(0.0));
        } else {
            add_figure(&sink, &rect, r);
        }
        sink.Close()?;
    }
    cache.map.insert(key, geo.clone());
    Ok(geo)
}

/// 填充平滑圆角矩形（r 在 (w,h) 坐标系内，原点为矩形左上角）。
pub fn fill_smooth_rect(
    factory: &ID2D1Factory,
    cache: &mut GeoCache,
    rt: &ID2D1RenderTarget,
    rect: D2D_RECT_F,
    radius: f32,
    brush: &ID2D1Brush,
) -> Result<()> {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if radius <= 0.0 || w < radius * 2.0 || h < radius * 2.0 {
        // SAFETY: rt 与 brush 均由调用方保证有效。
        unsafe { rt.FillRectangle(&rect, brush) };
        return Ok(());
    }
    let geo = rounded_geometry(factory, cache, w, h, radius, 0.0)?;
    // 路径建在 (0,0)-(w,h)，用平移变换搬到 rect 位置（画完恢复）。
    // SAFETY: rt/几何/画刷在绘制期间有效；变换在块内设置并立即恢复。
    unsafe {
        rt.SetTransform(&windows::Foundation::Numerics::Matrix3x2 {
            M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: rect.left, M32: rect.top,
        });
        rt.FillGeometry(&geo, brush, None);
        rt.SetTransform(&windows::Foundation::Numerics::Matrix3x2 {
            M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0,
        });
    }
    Ok(())
}

/// 描边画成外圈减内圈的环，粗细、圆角严格同心。
pub fn draw_smooth_rect(
    factory: &ID2D1Factory,
    cache: &mut GeoCache,
    rt: &ID2D1RenderTarget,
    rect: D2D_RECT_F,
    radius: f32,
    brush: &ID2D1Brush,
    width: f32,
) -> Result<()> {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if radius <= 0.0 || w < radius * 2.0 || h < radius * 2.0 {
        // SAFETY: 同上。
        unsafe { rt.DrawRectangle(&rect, brush, width, None) };
        return Ok(());
    }
    let geo = rounded_geometry(factory, cache, w, h, radius, width)?;
    // SAFETY: 同上；描边环同样需要平移到 rect 位置。
    unsafe {
        rt.SetTransform(&windows::Foundation::Numerics::Matrix3x2 {
            M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: rect.left, M32: rect.top,
        });
        rt.FillGeometry(&geo, brush, None);
        rt.SetTransform(&windows::Foundation::Numerics::Matrix3x2 {
            M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0,
        });
    }
    Ok(())
}

/// 柔和投影：按 (radius_px, blur_px) 预渲染角贴图并缓存，每帧九宫格拉伸。
#[allow(clippy::too_many_arguments)]
pub fn draw_soft_shadow(
    _factory: &ID2D1Factory,
    tiles: &mut HashMap<(u32, u32), ID2D1Bitmap>,
    rt: &ID2D1RenderTarget,
    rect: D2D_RECT_F,
    radius: f32,
    blur: f32,
    dy: f32,
    alpha: f32,
    scale: f32,
) -> Result<()> {
    if blur <= 0.0 || alpha <= 0.0 || rect.right <= rect.left || rect.bottom <= rect.top {
        return Ok(());
    }
    let sigma = blur / 2.0 * scale;
    let rad = radius * scale;
    let reach = 3.0 * sigma;
    let big_e = (rad + 2.0 * reach).ceil() as i32 + 1;
    let big_w = big_e as f32 - reach;
    let n = 2 * big_e;
    let key = ((rad * 8.0) as u32, (blur * scale * 8.0) as u32);
    if let std::collections::hash_map::Entry::Vacant(e) = tiles.entry(key) {
        let mut img = vec![0u32; (n as usize) * (n as usize)];
        let k = 1.0 / (sigma * 2.0f32.sqrt());
        for y in 0..n {
            for x in 0..n {
                let qx = ((x as f32 + 0.5 - big_e as f32).abs()) - (big_w - rad);
                let qy = ((y as f32 + 0.5 - big_e as f32).abs()) - (big_w - rad);
                let dist =
                    (qx.max(0.0).hypot(qy.max(0.0))) + (qx.max(qy).min(0.0)) - rad;
                let a = (alpha * 0.5 * erfc(dist * k)).clamp(0.0, 1.0);
                img[(y as usize) * (n as usize) + (x as usize)] =
                    ((a * 255.0).round() as u32) << 24; // 预乘黑色
            }
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
        };
        // SAFETY: img 的长度与 pitch 匹配，CreateBitmap 只读取不保留指针。
        let bmp = unsafe {
            rt.CreateBitmap(
                D2D_SIZE_U { width: n as u32, height: n as u32 },
                Some(img.as_ptr() as *const std::ffi::c_void),
                (n * 4) as u32,
                &props,
            )?
        };
        e.insert(bmp);
    }
    let bmp = &tiles[&key];

    // 九宫格：四角 1:1，边和中间拉伸模板正中的两行/两列
    let ex = reach / scale;
    let corner = (big_e - 1) as f32 / scale;
    let o = RectF {
        l: rect.left - ex,
        t: rect.top - ex + dy,
        r: rect.right + ex,
        b: rect.bottom + ex + dy,
    };
    let cw = corner.min((o.r - o.l) / 2.0);
    let ch = corner.min((o.b - o.t) / 2.0);
    let xs = [o.l, o.l + cw, o.r - cw, o.r];
    let ys = [o.t, o.t + ch, o.b - ch, o.b];
    let npx = n as f32 / scale;
    let mid1 = (big_e - 1) as f32 / scale;
    let mid2 = (big_e + 1) as f32 / scale;
    let src = [0.0, mid1, mid2, npx];
    for j in 0..3 {
        for i in 0..3 {
            let dst = D2D_RECT_F { left: xs[i], top: ys[j], right: xs[i + 1], bottom: ys[j + 1] };
            if dst.right <= dst.left || dst.bottom <= dst.top { continue; }
            let sr = D2D_RECT_F { left: src[i], top: src[j], right: src[i + 1], bottom: src[j + 1] };
            // SAFETY: 位图由 tiles 持有，绘制期间有效。
            unsafe { rt.DrawBitmap(bmp, Some(&dst), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, Some(&sr)) };
        }
    }
    Ok(())
}

fn erfc(x: f32) -> f32 {
    // 高精度互补误差函数（Abramowitz & Stegun 7.1.26），最大误差 1.5e-7
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let tau = t * (-z * z - 1.265_512_2
        + t * (1.000_023_7
            + t * (0.37409196
                + t * (0.09678418
                    + t * (-0.18628806
                        + t * (0.27886807
                            + t * (-1.135_204
                                + t * (1.488_515_9
                                    + t * (-0.82215223 + t * 0.17087277)))))))))
        .exp();
    if x >= 0.0 { tau } else { 2.0 - tau }
}
