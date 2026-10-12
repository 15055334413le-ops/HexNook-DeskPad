//! 调试截图（开发自检用，不属于产品功能）：
//! `DeskPad.exe --render-test <out.bmp> [--query <q>]`
//! 不创建任何可见窗口、不占单实例锁：在主显示器上按真实数据排版并渲染启动台一帧，
//! 叠在示意桌面色 + 背景幕上，写成 32 位 BMP。
use crate::app::{with_app, App};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};

pub fn run(out: &str, query: &str) -> bool {
    // 等引擎数据就绪（最多 8 秒）：应用列表 + 文件夹 + 图标基本补齐
    let start = std::time::Instant::now();
    loop {
        let done = with_app(|a| {
            let s = a.eng.snapshot();
            let icons = s.apps.iter().filter(|e| e.icon_png.is_some()).count();
            s.ready && !s.dirs.is_empty() && icons * 10 >= s.apps.len() * 9
        })
        .unwrap_or(true);
        if done || start.elapsed() > std::time::Duration::from_secs(8) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    with_app(|a| render(a, out, query)).unwrap_or(false)
}

fn render(a: &mut App, out: &str, query: &str) -> bool {
    a.snap = a.eng.snapshot();
    a.input.reset();
    for c in query.chars() {
        a.input.push_char(c);
    }
    a.input.hits = a.eng.search(&a.snap, &a.input.query);
    // SAFETY: 纯查询。
    let mon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    crate::win::log_mem("shot:before");
    let _ = crate::pad::prepare_frame(a, mon);
    crate::win::log_mem("shot:after-first-frame");
    a.caret_on = true;
    if let Err(e) = crate::pad::draw_only(a) {
        deskpad_engine::log::warn(&format!("render-test draw: {e}"));
        return false;
    }
    crate::win::log_mem("shot:after-draw");
    let (w, h) = a.pad_surface.size;
    let Some(px) = a.pad_surface.pixels() else { return false };
    // 合成：示意桌面色 → 背景幕 rgba(15,18,26,0.30) → 内容（预乘 BGRA）
    let desk = [0x8Bu32, 0x7B, 0x6E]; // B,G,R
    let scrim = [26u32, 18, 15];
    let mut img = vec![0u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        let sa = px[i * 4 + 3] as u32;
        for c in 0..3 {
            let base = (desk[c] * 179 + scrim[c] * 77) / 256;
            let v = px[i * 4 + c] as u32 + base * (255 - sa) / 255;
            img[i * 4 + c] = v.min(255) as u8;
        }
        img[i * 4 + 3] = 255;
    }
    write_bmp(out, w, h, &img).is_ok()
}

fn write_bmp(path: &str, w: i32, h: i32, bgra: &[u8]) -> std::io::Result<()> {
    let data_len = bgra.len() as u32;
    let mut f = Vec::with_capacity(54 + bgra.len());
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(54 + data_len).to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&(-h).to_le_bytes()); // top-down
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    f.extend_from_slice(&data_len.to_le_bytes());
    f.extend_from_slice(&2835i32.to_le_bytes());
    f.extend_from_slice(&2835i32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(bgra);
    std::fs::write(path, f)
}
