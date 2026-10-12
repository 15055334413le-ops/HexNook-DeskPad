//! 产品图形：2×2 圆角小方块（UI 文档 6.12）+ 运行时生成的托盘图标。
use crate::smooth::{fill_smooth_rect, GeoCache};
use crate::theme::PRIMARY;
use windows::core::Result;
use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SelectObject, DeleteObject, HGDIOBJ, HDC};
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO, SM_CXSMICON};

/// 在 (cx, cy) 中心绘制产品图形，整体边长 size（DIP）。
pub fn draw_glyph(
    rt: &ID2D1RenderTarget,
    factory: &ID2D1Factory,
    cache: &mut GeoCache,
    brush: &ID2D1Brush,
    cx: f32,
    cy: f32,
    size: f32,
) -> Result<()> {
    let edge = size * 7.0 / 16.0;
    let gap = size * 2.0 / 16.0;
    let r = edge * 2.0 / 7.0;
    for (dx, dy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let x = cx + dx * (gap + edge) / 2.0 - edge / 2.0;
        let y = cy + dy * (gap + edge) / 2.0 - edge / 2.0;
        fill_smooth_rect(
            factory,
            cache,
            rt,
            D2D_RECT_F { left: x, top: y, right: x + edge, bottom: y + edge },
            r,
            brush,
        )?;
    }
    Ok(())
}

/// 运行时生成托盘图标：D2D 画到 32bpp DIB 上再 CreateIconIndirect（颜色位图 + 全 0 mask）。
pub fn tray_icon(factory: &ID2D1Factory, dpi: u32) -> Result<HICON> {
    // SAFETY: 仅查询系统度量。
    let n = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.max(16);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = n;
    bmi.bmiHeader.biHeight = -n;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: bmi 在调用期间有效；位图句柄由 HBitmapGuard 管理。
    let hbmp = unsafe {
        CreateDIBSection(HDC::default(), &bmi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0)?
    };
    let guard = crate::win::HBitmapGuard(hbmp);
    let memdc = crate::win::MemDc::create()?;
    // SAFETY: 位图与 DC 均由本函数持有。
    unsafe { SelectObject(memdc.hdc, hbmp) };

    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: dpi as f32,
        dpiY: dpi as f32,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    // SAFETY: factory 有效；rt 只在本函数内使用。
    let rt = unsafe { factory.CreateDCRenderTarget(&props)? };
    // SAFETY: memdc 在 rt 使用期间有效。
    unsafe { rt.BindDC(memdc.hdc, &RECT { left: 0, top: 0, right: n, bottom: n })? };
    // SAFETY: 绘制序列配对。
    unsafe { rt.BeginDraw() };
    // SAFETY: 清成全透明。
    unsafe { rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })) };
    let brush_col = crate::theme::color(PRIMARY);
    // SAFETY: rt 有效。
    let brush = unsafe { ID2D1RenderTarget::CreateSolidColorBrush(&rt, &brush_col, None)? };
    let mut cache = GeoCache::default();
    let size_dip = n as f32 * 96.0 / dpi as f32;
    draw_glyph(&rt, factory, &mut cache, &brush, size_dip / 2.0, size_dip / 2.0, size_dip * 0.6)?;
    // SAFETY: 绘制序列配对。
    unsafe { rt.EndDraw(None, None)? };

    // 全 0 单色 mask
    // SAFETY: mask 位图在 CreateIconIndirect（系统复制位图）后由本函数删除。
    let mask = unsafe { CreateBitmap(n, n, 1, 1, None) };
    let info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: hbmp,
    };
    // SAFETY: 两个位图句柄均有效；系统复制后由本函数删除 mask（颜色位图由 guard 删除）。
    let icon = unsafe { CreateIconIndirect(&info)? };
    drop(guard);
    // SAFETY: mask 已被系统复制，删除原始句柄。
    unsafe { let _ = DeleteObject(HGDIOBJ(mask.0)); }
    Ok(icon)
}
