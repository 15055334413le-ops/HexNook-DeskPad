//! 开关动画（UI 文档 6.9）：只调 set_alpha_and_pos，不重绘。
use crate::app::{App, HideReason};
use std::time::Instant;
use windows::Win32::Foundation::POINT;

pub const OPEN_MS: u128 = 120;
pub const CLOSE_MS: u128 = 80;
pub const TIMER_ID: usize = 1;
pub const TIMER_MS: u32 = 15;


#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Opening,
    Closing(HideReason),
}

#[derive(Clone, Copy, Debug)]
pub struct Anim {
    pub phase: Phase,
    pub t0: Instant,
}

/// 系统动画开关（SPI_GETCLIENTAREAANIMATION）。关闭时所有动画直接到终态。
pub fn animations_enabled() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SPI_GETCLIENTAREAANIMATION,
    };
    // 注意：pvParam 指向 4 字节的 BOOL，不能用 Rust 的 1 字节 bool（会写坏栈）。
    let mut on = windows::Win32::Foundation::BOOL(1);
    // SAFETY: 标准 BOOL 查询，pvparam 指向本函数栈上的 BOOL。
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    on.as_bool()
}

fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn base_origin(app: &App) -> POINT {
    POINT { x: app.pad_origin.0, y: app.pad_origin.1 }
}

pub fn start_open(app: &mut App) {
    if animations_enabled() {
        app.anim = Some(Anim { phase: Phase::Opening, t0: Instant::now() });
    } else {
        app.anim = None;
        finish_open(app);
    }
}

/// 动画关闭路径下直接到终态。
pub fn finish_open(app: &mut App) {
    app.pad_alpha = 255;
    app.scrim_alpha = 77;
    app.anim = None;
}

pub fn start_close(app: &mut App, reason: HideReason) {
    if animations_enabled() {
        app.anim = Some(Anim { phase: Phase::Closing(reason), t0: Instant::now() });
        // 重新挂动画定时器：hide() 入口已停掉旧定时器，关闭动画由它推进
        // SAFETY: 定时器挂在内容窗口上，动画结束/再次 hide 时统一 Kill。
        unsafe { let _ = windows::Win32::UI::WindowsAndMessaging::SetTimer(app.pad, TIMER_ID, TIMER_MS, None); }
    } else {
        app.anim = None;
        crate::pad::finalize_hide(app, reason);
    }
}

/// WM_TIMER 到达：推进动画。返回 true 表示动画结束。
pub fn tick(app: &mut App) -> bool {
    let Some(anim) = app.anim else { return true };
    let ms = anim.t0.elapsed().as_millis();
    match anim.phase {
        Phase::Opening => {
            let t = ease_out_cubic(ms as f32 / OPEN_MS as f32);
            let alpha = (255.0 * t) as u8;
            app.pad_alpha = alpha;
            app.scrim_alpha = (77.0 * t) as u8;
            // y 从 +8 DIP 移到 0
            let dy = ((1.0 - t) * 8.0 * app.scale()).round() as i32;
            let mut p = base_origin(app);
            p.y += dy;
            let _ = app.pad_surface().set_alpha_and_pos(app.pad, p, alpha);
            crate::pad::set_scrim_alpha(app, app.scrim_alpha);
            if ms >= OPEN_MS {
                finish_open(app);
                return true;
            }
            false
        }
        Phase::Closing(reason) => {
            let t = 1.0 - (ms as f32 / CLOSE_MS as f32).clamp(0.0, 1.0);
            let alpha = (255.0 * t) as u8;
            app.pad_alpha = alpha;
            app.scrim_alpha = (77.0 * t) as u8;
            let _ = app.pad_surface().set_alpha_and_pos(app.pad, base_origin(app), alpha);
            crate::pad::set_scrim_alpha(app, app.scrim_alpha);
            if ms >= CLOSE_MS {
                app.anim = None;
                crate::pad::finalize_hide(app, reason);
                return true;
            }
            false
        }
    }
}

#[allow(dead_code)] // 预留：批量隐藏路径使用
pub fn kill_timer(app: &App) {
    // SAFETY: 定时器挂在内容窗口上，ID 固定。
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(app.pad, TIMER_ID) };
}
