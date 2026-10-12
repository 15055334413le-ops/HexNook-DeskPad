//! 图标提取 → 96px PNG 磁盘缓存（engine 文档 4.2）。
//!
//! 统一路径：IShellItemImageFactory::GetImage(SIZE{96,96}, SIIGBF_ICONONLY)。
//! 缓存策略：7 天内命中复用；过期先复用旧图并排入重提取队列；失败不写文件，
//! 同一 id 进程内最多尝试 3 次；写盘失败只记 warn。
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use image::imageops::FilterType;
use image::RgbaImage;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{SIZE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Gdi::{
    GetDC, GetDIBits, GetObjectW, ReleaseDC, BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS,
};
use windows::Win32::System::Com::CoFreeUnusedLibrariesEx;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::UI::Shell::{SHCreateItemFromParsingName, IShellItemImageFactory, SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY};

use crate::log;
use crate::model::{AppEntry, AppId, LaunchTarget};
use crate::win;

const MAX_ATTEMPTS: u32 = 3;
const SEVEN_DAYS: Duration = Duration::from_secs(7 * 24 * 3600);

pub(crate) fn cache_path(data_dir: &Path, id: AppId) -> PathBuf {
    data_dir.join("icons").join(format!("{id:016x}.png"))
}

/// 扫描后按磁盘缓存给每个应用挂图标；返回超过 7 天、需要后台重提取的 id。
pub(crate) fn resolve(apps: &mut [AppEntry], data_dir: &Path) -> Vec<AppId> {
    let now = SystemTime::now();
    let mut stale = Vec::new();
    for a in apps.iter_mut() {
        let path = cache_path(data_dir, a.id);
        match fs::metadata(&path) {
            Ok(md) => {
                let fresh = md
                    .modified()
                    .ok()
                    .map(|m| now.duration_since(m).unwrap_or(Duration::ZERO) < SEVEN_DAYS)
                    .unwrap_or(false);
                a.icon_png = Some(path);
                if !fresh {
                    stale.push(a.id);
                }
            }
            Err(_) => a.icon_png = None,
        }
    }
    stale
}

/// 逐个补齐缺失/过期图标（4.8.2）：每 40 个发布一次。返回本次实际提取成功数。
pub(crate) fn fill_missing(
    inner: &crate::Inner,
    targets: &[(AppId, LaunchTarget)],
    stale: &[AppId],
    failures: &mut HashMap<AppId, u32>,
) -> u32 {
    let mut extracted = 0u32;
    let mut batch: HashMap<AppId, PathBuf> = HashMap::new();
    for (id, launch) in targets {
        let stale_id = stale.contains(id);
        if !stale_id && failures.get(id).copied().unwrap_or(0) >= MAX_ATTEMPTS {
            continue;
        }
        // 提取过程中也要响应 shutdown。
        // SAFETY: 句柄由 Inner 持有且进程内存活；等待 0ms 只做状态查询。
        let shutting_down = unsafe { WaitForSingleObject(inner.shutdown_event.get(), 0) };
        if shutting_down == WAIT_OBJECT_0 {
            break;
        }
        match extract(launch, &inner.data_dir, *id) {
            Ok(path) => {
                failures.remove(id);
                extracted += 1;
                if !stale_id {
                    // 缺失图标：把新路径记入待发布批次。
                    batch.insert(*id, path);
                }
                // 过期图标：文件已被覆盖，快照路径不变，无需改条目。
                if extracted.is_multiple_of(40) {
                    apply_icons(inner, &batch);
                    batch.clear();
                }
            }
            Err(e) => {
                log::warn(&format!("icon extract failed for {id:016x}: {e}"));
                *failures.entry(*id).or_insert(0) += 1;
            }
        }
    }
    if !batch.is_empty() {
        apply_icons(inner, &batch);
    }
    if extracted > 0 {
        // 卸载 shell 扩展 DLL，压住常驻内存（4.2）。
        // SAFETY: 无参数系统调用。
        unsafe {
            CoFreeUnusedLibrariesEx(0, 0);
        }
    }
    extracted
}

/// 把一批提取结果写进快照并发布（4.8.2 发布语义）。
fn apply_icons(inner: &crate::Inner, batch: &HashMap<AppId, PathBuf>) -> bool {
    let mut snap = crate::write_lock(&inner.snap);
    let apps = std::sync::Arc::make_mut(&mut snap.apps);
    let mut changed = false;
    for a in apps.iter_mut() {
        if let Some(p) = batch.get(&a.id) {
            if a.icon_png.as_deref() != Some(p.as_path()) {
                a.icon_png = Some(p.clone());
                changed = true;
            }
        }
    }
    if changed {
        snap.generation += 1;
    }
    drop(snap);
    if changed {
        crate::fire_on_change(inner);
    }
    changed
}

/// 提取单个应用图标并写盘；返回 PNG 缓存路径。
fn extract(launch: &LaunchTarget, data_dir: &Path, id: AppId) -> Result<PathBuf, String> {
    let path_str = match launch {
        LaunchTarget::AppsFolder(p) => format!("shell:AppsFolder\\{p}"),
        LaunchTarget::Shortcut(p) => p.to_string_lossy().to_string(),
    };
    let out = cache_path(data_dir, id);
    extract_to(&path_str, &out)?;
    Ok(out)
}

/// 任意 shell 解析路径 → 96×96 PNG 写到 out（只取图标，不取缩略图）。
fn extract_to(path_str: &str, out: &Path) -> Result<(), String> {
    let wide = win::to_wide(path_str);
    // SAFETY: 扫描线程 COM STA 已初始化；wide 以 0 结尾且调用期间存活。
    let hbmp = unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                .map_err(|e| format!("SHCreateItemFromParsingName({path_str}): {e}"))?;
        factory
            .GetImage(SIZE { cx: 96, cy: 96 }, SIIGBF_ICONONLY)
            .map_err(|e| format!("GetImage({path_str}): {e}"))?
    };
    let png = hbitmap_to_png(hbmp)?;
    if let Err(e) = fs::write(out, &png) {
        log::warn(&format!("icon write failed ({}): {e}", out.display()));
        return Err(format!("write icon: {e}"));
    }
    Ok(())
}

/// HBITMAP → 非预乘 RGBA8 PNG（96×96 画布）。
fn hbitmap_to_png(hbmp: windows::Win32::Graphics::Gdi::HBITMAP) -> Result<Vec<u8>, String> {
    let (w, h, buf) = hbitmap_rgba(hbmp)?;
    let img = RgbaImage::from_raw(w as u32, h as u32, buf).ok_or("bad pixel buffer")?;
    let final_img = if (w as u32, h as u32) == (96, 96) {
        img
    } else {
        // 双线性缩放到 96 以内并居中放到 96×96 透明画布。
        let scale = 96.0f32 / w.max(h) as f32;
        let nw = ((w as f32 * scale).round() as u32).clamp(1, 96);
        let nh = ((h as f32 * scale).round() as u32).clamp(1, 96);
        let resized = image::imageops::resize(&img, nw, nh, FilterType::Triangle);
        let mut canvas = RgbaImage::new(96, 96);
        let ox = ((96 - nw) / 2) as i64;
        let oy = ((96 - nh) / 2) as i64;
        image::imageops::overlay(&mut canvas, &resized, ox, oy);
        canvas
    };
    encode_png(final_img)
}

fn encode_png(img: RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| format!("png encode: {e}"))?;
    Ok(out)
}

/// 缩略图边长上限（DIP 64 × 200% 缩放 ≈ 128，再留余量）。
pub(crate) const THUMB_MAX: u32 = 160;


/// 文件内容缩略图（图片 / 视频帧 / 演示封面 / PDF 首页），缓存 icons/t-{fnv(path|mtime)}.png，长边 ≤ THUMB_MAX。
pub(crate) enum Thumb {
    /// 缓存里已有 / 刚生成的 PNG
    Ready(PathBuf),
    /// 系统拿不到的 PDF：交给 docthumb::render_pdfs 渲染到这个路径
    NeedPdf(PathBuf),
    /// 没有缩略图（已写失败标记，同一 mtime 不再重试）
    Missing,
}

fn thumb_paths(data_dir: &Path, path: &Path, mtime: i64) -> (PathBuf, PathBuf) {
    let key = crate::keys::fnv1a64(format!("{}|{mtime}", path.to_string_lossy().to_lowercase()).as_bytes());
    let dir = data_dir.join("icons");
    (dir.join(format!("t-{key:016x}.png")), dir.join(format!("t-{key:016x}.none")))
}

/// 失败标记：PDF 子进程结束后仍没有输出时由调用方写。
pub(crate) fn mark_no_thumb(data_dir: &Path, path: &Path, mtime: i64) {
    let _ = fs::write(thumb_paths(data_dir, path, mtime).1, b"");
}

pub(crate) fn thumbnail(data_dir: &Path, path: &Path, mtime: i64) -> Thumb {
    let (out, none) = thumb_paths(data_dir, path, mtime);
    if out.exists() {
        return Thumb::Ready(out);
    }
    if none.exists() {
        return Thumb::Missing;
    }
    if let Some(img) = shell_thumbnail(path) {
        if write_thumb(img, &out).is_some() {
            return Thumb::Ready(out);
        }
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pptx" | "pptm" | "ppsx" | "ppsm" | "potx" | "potm" => {
            let img = crate::docthumb::ooxml_thumbnail(path)
                .and_then(|b| image::load_from_memory(&b).ok())
                .map(|i| i.to_rgba8());
            if let Some(img) = img {
                if write_thumb(img, &out).is_some() {
                    return Thumb::Ready(out);
                }
            }
        }
        "pdf" => return Thumb::NeedPdf(out),
        _ => {}
    }
    let _ = fs::write(&none, b"");
    Thumb::Missing
}

fn shell_thumbnail(path: &Path) -> Option<RgbaImage> {
    let wide = win::to_wide(&path.to_string_lossy());
    // SAFETY: 扫描线程 COM STA 已初始化；wide 以 0 结尾且调用期间存活。
    let hbmp = unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        factory
            .GetImage(SIZE { cx: THUMB_MAX as i32, cy: THUMB_MAX as i32 }, SIIGBF_THUMBNAILONLY)
            .ok()?
    };
    let (w, h, buf) = hbitmap_rgba(hbmp).ok()?;
    RgbaImage::from_raw(w as u32, h as u32, buf)
}

/// 长边缩到 THUMB_MAX 以内后写 PNG。
fn write_thumb(img: RgbaImage, out: &Path) -> Option<()> {
    let (w, h) = img.dimensions();
    let fit = THUMB_MAX as f32 / w.max(h).max(1) as f32;
    let img = if fit < 1.0 {
        let nw = ((w as f32 * fit).round() as u32).max(1);
        let nh = ((h as f32 * fit).round() as u32).max(1);
        image::imageops::resize(&img, nw, nh, FilterType::Triangle)
    } else {
        img
    };
    let png = encode_png(img).ok()?;
    fs::write(out, png).ok()
}

/// 清理 7 天前的缩略图缓存（最近文件会变，旧图没用了）。
pub(crate) fn prune_thumbnails(data_dir: &Path) {
    let Ok(rd) = fs::read_dir(data_dir.join("icons")) else { return };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("t-") {
            continue;
        }
        let old = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|m| now.duration_since(m).unwrap_or(Duration::ZERO) > SEVEN_DAYS)
            .unwrap_or(false);
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// HBITMAP → 非预乘 RGBA8（宽, 高, 像素）。
fn hbitmap_rgba(hbmp: windows::Win32::Graphics::Gdi::HBITMAP) -> Result<(usize, usize, Vec<u8>), String> {
    let _guard = win::HBitmapGuard(hbmp);
    let mut bm = BITMAP::default();
    // SAFETY: bm 由本函数持有，长度为 size_of::<BITMAP>()。
    let n = unsafe {
        GetObjectW(
            hbmp,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut core::ffi::c_void),
        )
    };
    if n == 0 {
        return Err("GetObjectW failed".into());
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
        return Err(format!("bad bitmap size {w}x{h}"));
    }
    let (w, h) = (w as usize, h as usize);

    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w as i32;
    bi.bmiHeader.biHeight = -(h as i32); // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB.0;
    let mut buf = vec![0u8; w * h * 4];
    // SAFETY: 屏幕 DC 用完即还；缓冲区由本函数持有，长度 w*h*4 与请求行数一致。
    unsafe {
        let hdc = GetDC(None);
        GetDIBits(
            hdc,
            hbmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = ReleaseDC(None, hdc);
    }

    // BGRA → RGBA，同时做 alpha 规范化（4.2）。
    let mut all_zero = true;
    let mut non_premult = false;
    for p in buf.as_chunks::<4>().0 {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        if a != 0 {
            all_zero = false;
        }
        if r.max(g).max(b) > a {
            non_premult = true;
        }
    }
    for p in buf.as_chunks_mut::<4>().0 {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        let (r2, g2, b2, a2) = if all_zero {
            // 老式无 alpha 图标：全部置 255。
            (r, g, b, 255u8)
        } else if non_premult {
            // 本来就是非预乘，原样保留。
            (r, g, b, a)
        } else if a == 0 {
            // 预乘数据：a=0 的像素颜色置 0。
            (0, 0, 0, 0)
        } else {
            // 预乘 → 反预乘 c = min(255, c*255/a)。
            let f = |c: u8| -> u8 { ((c as u32 * 255) / a as u32).min(255) as u8 };
            (f(r), f(g), f(b), a)
        };
        p[0] = r2;
        p[1] = g2;
        p[2] = b2;
        p[3] = a2;
    }

    Ok((w, h, buf))
}

