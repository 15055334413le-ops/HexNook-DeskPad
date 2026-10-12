//! 下载：WinINet（跟随系统代理设置 —— Clash 等「系统代理」模式下也能访问 GitHub）。
use std::ffi::c_void;
use windows::core::{w, HSTRING};
use windows::Win32::Networking::WinInet::*;

/// 版本清单（发布仓库根目录 latest.json）。
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/15055334413le-ops/HexNook-DeskPad-Releases/main/latest.json";

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub sha256: String,
}

struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 句柄由 InternetOpen* 返回，只关闭一次。
            unsafe {
                let _ = InternetCloseHandle(self.0);
            }
        }
    }
}

/// GET url → 全部字节。progress(已下载, 总长度或 0)。
pub fn get(url: &str, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    // SAFETY: 字符串参数在调用期间有效；句柄由 Handle 负责关闭。
    unsafe {
        let session = Handle(InternetOpenW(
            w!("HN DeskPad Setup"),
            INTERNET_OPEN_TYPE_PRECONFIG.0,
            None,
            None,
            0,
        ));
        if session.0.is_null() {
            return Err("无法初始化网络".into());
        }
        let req = Handle(InternetOpenUrlW(
            session.0,
            &HSTRING::from(url),
            None,
            INTERNET_FLAG_RELOAD | INTERNET_FLAG_NO_CACHE_WRITE | INTERNET_FLAG_SECURE | INTERNET_FLAG_NO_UI,
            0,
        ));
        if req.0.is_null() {
            return Err("无法连接下载服务器，请检查网络或代理后重试".into());
        }
        let status = query_number(req.0, HTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if status != 200 {
            return Err(format!("下载服务器返回错误（HTTP {status}）"));
        }
        let total = query_number(req.0, HTTP_QUERY_CONTENT_LENGTH).unwrap_or(0) as u64;
        let mut out = Vec::with_capacity(total.min(64 << 20) as usize);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            InternetReadFile(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n)
                .map_err(|_| "下载中断，请重试".to_string())?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
            progress(out.len() as u64, total);
        }
        Ok(out)
    }
}

unsafe fn query_number(req: *mut c_void, level: u32) -> Option<u32> {
    let mut v = 0u32;
    let mut len = 4u32;
    HttpQueryInfoW(req, level | HTTP_QUERY_FLAG_NUMBER, Some(&mut v as *mut u32 as *mut c_void), &mut len, None).ok()?;
    Some(v)
}

/// 读取最新版本清单。
pub fn latest() -> Result<Release, String> {
    let body = get(MANIFEST_URL, |_, _| {})?;
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| "版本信息格式错误".to_string())?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    match (s("version"), s("url"), s("sha256")) {
        (Some(version), Some(url), Some(sha256)) => Ok(Release { version, url, sha256: sha256.to_lowercase() }),
        _ => Err("版本信息不完整".into()),
    }
}
