//! 分层窗口离屏表面（UI 文档 6.7/4.4）。
//!
//! 内存（第 9 节）：早先用 DIB + ID2D1DCRenderTarget，实测软件 DC 渲染目标内部还会再留一份
//! 同尺寸缓冲，1752×1152 的启动台等于两份 8MB。现在只保留一份像素：D2D 直接画进 WIC 位图，
//! 推屏时用 GDI 互操作临时取 DC 交给 UpdateLayeredWindow。
use windows::core::{Interface, Result};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Surface {
    pub rt: Option<ID2D1RenderTarget>,
    bmp: Option<IWICBitmap>,
    pub size: (i32, i32),
}

impl Surface {
    pub fn new() -> Self {
        Surface { rt: None, bmp: None, size: (0, 0) }
    }

    /// 尺寸不变时复用；否则重建 WIC 位图 + 软件渲染目标（GDI 兼容）。
    pub fn ensure(&mut self, factory: &ID2D1Factory, w: i32, h: i32, dpi: f32) -> Result<()> {
        if self.rt.is_some() && self.size == (w, h) && w > 0 && h > 0 {
            return Ok(());
        }
        self.release();
        // SAFETY: 标准 COM 创建；调用线程已 CoInitializeEx(STA)。
        let wic: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        // SAFETY: 参数均为值；位图由 self 持有。
        let bmp = unsafe {
            wic.CreateBitmap(w.max(1) as u32, h.max(1) as u32, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?
        };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: dpi,
            dpiY: dpi,
            usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        // SAFETY: 位图与属性有效；渲染目标持有位图引用。
        let rt = unsafe { factory.CreateWicBitmapRenderTarget(&bmp, &props)? };
        self.rt = Some(rt);
        self.bmp = Some(bmp);
        self.size = (w, h);
        Ok(())
    }

    pub fn rt(&self) -> Option<&ID2D1RenderTarget> {
        self.rt.as_ref()
    }

    /// 当前像素（BGRA 预乘、top-down、紧凑行宽 = 宽 × 4）的拷贝。调试截图用。
    pub fn pixels(&self) -> Option<Vec<u8>> {
        let bmp = self.bmp.as_ref()?;
        let (w, h) = self.size;
        let rect = WICRect { X: 0, Y: 0, Width: w, Height: h };
        // SAFETY: 锁定整幅位图只读；锁对象在本函数内释放。
        unsafe {
            let lock = bmp.Lock(&rect, WICBitmapLockRead.0 as u32).ok()?;
            let stride = lock.GetStride().ok()? as usize;
            let mut len = 0u32;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            lock.GetDataPointer(&mut len, &mut ptr).ok()?;
            if ptr.is_null() {
                return None;
            }
            let src = std::slice::from_raw_parts(ptr, len as usize);
            let row = w as usize * 4;
            let mut out = Vec::with_capacity(row * h as usize);
            for y in 0..h as usize {
                out.extend_from_slice(&src[y * stride..y * stride + row]);
            }
            Some(out)
        }
    }

    /// 把当前画面上传到分层窗口（可带整体透明度）。必须在 EndDraw 之后调用。
    pub fn present(&self, hwnd: HWND, pos: POINT, alpha: u8) -> Result<()> {
        let Some(rt) = &self.rt else { return Ok(()) };
        let gdi: ID2D1GdiInteropRenderTarget = rt.cast()?;
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let size = SIZE { cx: self.size.0, cy: self.size.1 };
        let src = POINT { x: 0, y: 0 };
        // SAFETY: GetDC 只能在 BeginDraw/EndDraw 之间调用；COPY 模式拿到的就是已画好的内容，
        // ReleaseDC 之后再 EndDraw。DC 只在本块内使用。
        unsafe {
            rt.BeginDraw();
            let r = match gdi.GetDC(D2D1_DC_INITIALIZE_MODE_COPY) {
                Ok(hdc) => {
                    let r = UpdateLayeredWindow(
                        hwnd,
                        HDC::default(),
                        Some(&pos),
                        Some(&size),
                        hdc,
                        Some(&src),
                        COLORREF(0),
                        Some(&blend),
                        ULW_ALPHA,
                    );
                    let _ = gdi.ReleaseDC(None);
                    if r.is_err() {
                        deskpad_engine::log::warn(&format!("UpdateLayeredWindow failed: {:?}", r));
                    }
                    r
                }
                Err(e) => Err(e),
            };
            let end = rt.EndDraw(None, None);
            r?;
            end
        }
    }

    /// 只改透明度与位置，不重新上传位图（动画专用）。
    pub fn set_alpha_and_pos(&self, hwnd: HWND, pos: POINT, alpha: u8) -> Result<()> {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: hdcSrc 传 None 时 UpdateLayeredWindow 只更新位置与透明度。
        unsafe {
            UpdateLayeredWindow(hwnd, HDC::default(), Some(&pos), None, HDC::default(), None, COLORREF(0), Some(&blend), ULW_ALPHA)
        }
    }

    /// 隐藏时释放全部大块资源（UI 文档 6.5 hide 第 4 步）。
    pub fn release(&mut self) {
        self.rt = None;
        self.bmp = None;
        self.size = (0, 0);
    }
}
