//! 相对时间格式化、错误文案（UI 文档 6.13；纯函数，有单元测试）。
use deskpad_engine::EngineError;

/// 本地时间的年月日时分（由 UI 用 GetLocalTime 取得后传入，保持纯函数可测）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalNow {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// 民用日（Howard Hinnant civil_from_days）：自 1970-01-01 起的天数 → (年, 月, 日)。
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    ((y + i64::from(m <= 2)) as i32, m, d)
}

/// 相对时间。`local_mtime` 是已换算到本地时区的 Unix 秒（unix + 时区偏移）。
pub fn rel_time(local_mtime: i64, now: &LocalNow) -> String {
    let diff = {
        // now 的本地时间换回“本地秒”：用天数差近似（同一时区内线性）。
        let now_days = days_from_civil(now.year, now.month, now.day);
        now_unix_aligned(now_days, now.hour, now.minute) - local_mtime
    };
    let (y, m, d) = civil_from_days(local_mtime.div_euclid(86_400));
    let secs_of_day = local_mtime.rem_euclid(86_400);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    if diff < 60 {
        return "刚刚".into();
    }
    if diff < 3600 {
        return format!("{} 分钟前", diff / 60);
    }
    let now_days = days_from_civil(now.year, now.month, now.day);
    let mt_days = local_mtime.div_euclid(86_400);
    if now_days == mt_days {
        return format!("今天 {hh:02}:{mm:02}");
    }
    if now_days - mt_days == 1 {
        return format!("昨天 {hh:02}:{mm:02}");
    }
    if now_days - mt_days < 7 {
        return format!("{} 天前", now_days - mt_days);
    }
    if y == now.year {
        return format!("{m}月{d}日");
    }
    format!("{y}/{m}/{d}")
}

/// 用“本地天序号 + 时分”构造与 local_mtime 同尺度的秒值（本地秒）。
fn now_unix_aligned(days: i64, hour: u32, minute: u32) -> i64 {
    days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60
}

/// Howard Hinnant days_from_civil（公历日 → 自 1970-01-01 的天数）。
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + u64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// EngineError → 用户文案（UI 文档 6.13 表）。
pub fn error_text(e: &EngineError) -> String {
    match e {
        EngineError::LaunchFailed { name, .. } => format!("无法启动 {name}"),
        EngineError::PathNotFound(p) => {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.to_string_lossy().to_string());
            format!("找不到 {name}，它可能已被移动或删除")
        }
        EngineError::UnknownApp(_) => "这个应用已不存在".into(),
        _ => "出错了，详情见日志".into(),
    }
}

#[cfg(test)]
mod t {
    use super::*;

    fn now() -> LocalNow {
        LocalNow { year: 2026, month: 10, day: 5, hour: 9, minute: 30 }
    }
    // 2026-10-05 09:30 本地秒（直接用 civil 天数构造，避免时区假设）
    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        days_from_civil(y, mo, d) * 86_400 + i64::from(h) * 3600 + i64::from(mi) * 60
    }

    #[test]
    fn rel_time_cases() {
        let n = now();
        let base = local(2026, 10, 5, 9, 30);
        assert_eq!(rel_time(base, &n), "刚刚");
        assert_eq!(rel_time(base - 30, &n), "刚刚");
        assert_eq!(rel_time(base - 5 * 60, &n), "5 分钟前");
        assert_eq!(rel_time(base - 2 * 3600, &n), "今天 07:30");
        assert_eq!(rel_time(local(2026, 10, 4, 23, 5), &n), "昨天 23:05");
        assert_eq!(rel_time(local(2026, 10, 1, 12, 0), &n), "4 天前");
        assert_eq!(rel_time(local(2026, 9, 20, 12, 0), &n), "9月20日");
        assert_eq!(rel_time(local(2025, 3, 8, 12, 0), &n), "2025/3/8");
    }

    #[test]
    fn error_texts() {
        assert_eq!(error_text(&EngineError::UnknownApp(1)), "这个应用已不存在");
        assert_eq!(
            error_text(&EngineError::LaunchFailed { name: "微信".into(), code: 2 }),
            "无法启动 微信"
        );
    }
}
