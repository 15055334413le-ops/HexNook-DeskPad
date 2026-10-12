//! PNG → ID2D1Bitmap（WIC），只为可见项解码（UI 文档 6.7 icons.rs）。
use deskpad_engine::{AppId, Snapshot};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use windows::core::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;

const GENERIC_READ: windows::Win32::Foundation::GENERIC_ACCESS_RIGHTS = windows::Win32::Foundation::GENERIC_ACCESS_RIGHTS(0x8000_0000);

pub struct IconCache {
    map: HashMap<AppId, (PathBuf, ID2D1Bitmap)>,
    /// 文件缩略图（engine 落盘的 t-*.png）；路径含 mtime 哈希，内容变化即换键。
    paths: HashMap<PathBuf, Option<ID2D1Bitmap>>,
}

impl IconCache {
    pub fn new() -> Self {
        IconCache { map: HashMap::new(), paths: HashMap::new() }
    }

    /// 快照更新时：淘汰已消失或图标路径变化的条目。
    pub fn prune(&mut self, snap: &Snapshot) {
        self.map.retain(|id, (p, _)| {
            snap.apps
                .iter()
                .any(|a| a.id == *id && a.icon_png.as_deref() == Some(p.as_path()))
        });
        self.paths.retain(|p, _| snap.recent.iter().any(|f| f.thumb_png.as_deref() == Some(p.as_path())));
    }

    /// 按路径取缩略图位图；解码失败也缓存（None），不在每帧重试。
    pub fn get_path(&mut self, rt: &ID2D1RenderTarget, wic: &IWICImagingFactory, path: &Path) -> Option<ID2D1Bitmap> {
        if let Some(b) = self.paths.get(path) {
            return b.clone();
        }
        let b = decode(rt, wic, path).ok();
        self.paths.insert(path.to_path_buf(), b.clone());
        b
    }

    #[allow(dead_code)] // 快照切换预留
    pub fn clear(&mut self) {
        self.map.clear();
        self.paths.clear();
    }

    /// 取应用的图标位图；PNG 不存在或解码失败 → None（调用方画占位方块）。
    pub fn get(
        &mut self,
        rt: &ID2D1RenderTarget,
        wic: &IWICImagingFactory,
        e: &deskpad_engine::AppEntry,
    ) -> Option<ID2D1Bitmap> {
        if let Some((p, b)) = self.map.get(&e.id) {
            if e.icon_png.as_deref() == Some(p.as_path()) {
                return Some(b.clone());
            }
        }
        let path = e.icon_png.as_ref()?;
        let bmp = decode(rt, wic, path).ok()?;
        self.map.insert(e.id, (path.clone(), bmp.clone()));
        Some(bmp)
    }
}

fn decode(rt: &ID2D1RenderTarget, wic: &IWICImagingFactory, path: &Path) -> Result<ID2D1Bitmap> {
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: 路径字符串在调用期间有效；解码器由返回值接管。
    let decoder = unsafe {
        wic.CreateDecoderFromFilename(
            PCWSTR(wide.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnLoad,
        )?
    };
    // SAFETY: frame 由 decoder 接管并克隆进转换器。
    let frame = unsafe { decoder.GetFrame(0)? };
    // SAFETY: 同上。
    let conv = unsafe { wic.CreateFormatConverter()? };
    // SAFETY: frame 引用在 Initialize 调用期间有效。
    unsafe {
        conv.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
    }
    // SAFETY: 转换器在 CreateBitmapFromWicBitmap 调用期间有效；位图由返回值接管。
    unsafe { rt.CreateBitmapFromWicBitmap(&conv, None) }
}

/// 托盘图标边长（按 DPI 取 SM_CXSMICON）。
#[allow(dead_code)] // 托盘图标尺寸换算预留
pub fn small_icon_size(dpi: u32) -> i32 {
    // SAFETY: 仅查询系统度量。
    unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.max(16)
}
