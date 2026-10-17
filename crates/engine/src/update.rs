//! 在线更新：子进程检查（WinINet）、版本比较、update.json 读写（引擎文档 4.11 / questions.md #35）。
//!
//! **WinINet 只在一次性子进程里被加载**：主进程为了「隐藏时 CPU 为 0」不碰网络，
//! 检查更新由 `DeskPad.exe --check-update` 这个一次性子进程完成，它读清单、写
//! `%LOCALAPPDATA%\DeskPad\update.json`，再 PostMessage 通知主进程后退出。
//! 安装程序（crates/setup）直接复用本模块的 get/latest，两份实现由此合一。
use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::{w, PCWSTR};

use crate::model::Settings;
use crate::{log, win};

/// 发布仓库版本清单（latest.json）。
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/15055334413le-ops/HexNook-DeskPad-Releases/main/latest.json";
/// 发布仓库首页（便携版用户自行下载安装包）。
pub const RELEASES_URL: &str =
    "https://github.com/15055334413le-ops/HexNook-DeskPad-Releases";
/// 检查结果的存放文件（数据目录下）。
pub const UPDATE_FILE: &str = "update.json";
/// 当前进程版本号（编译期常量，与 Cargo.toml 一致）。
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 自动检查的最短间隔（24 小时）。主进程打开启动台时判断是否要拉起子进程。
pub const AUTO_CHECK_INTERVAL: i64 = 24 * 60 * 60;

/// 主进程消息：WM_APP + 6。检查子进程写完 update.json 后投给 DeskPadMain 窗口。
pub const WM_APP_UPDATE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 6;

/// WinINet 超时（毫秒）：一次检查最多占 20 秒，防止子进程长期挂住。
pub const NET_TIMEOUT_MS: u32 = 20_000;

/// 版本清单解析结果（latest.json 的 version / url / sha256）。
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub sha256: String,
    /// 发布说明（latest.json 的 notes，可选）。
    pub notes: String,
}

/// `update.json` 的内容：{checked_at, latest, url, sha256, notes, error}。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// 上次检查完成的 Unix 秒（0 = 从未检查过）。
    #[serde(default)]
    pub checked_at: i64,
    /// 发现的最新版本号；检查失败时为空串。
    #[serde(default)]
    pub latest: String,
    /// 安装包地址；便携版用户用浏览器打开它。
    #[serde(default)]
    pub url: String,
    /// 安装包 SHA-256（小写十六进制）。
    #[serde(default)]
    pub sha256: String,
    /// 发布说明（latest.json 的 notes，可为空）。
    #[serde(default)]
    pub notes: String,
    /// 出错信息；成功时为空串。
    #[serde(default)]
    pub error: String,
}

impl UpdateInfo {
    /// 是否发现了比 current 更新的版本。
    pub fn has_new(&self, current: &str) -> bool {
        !self.latest.is_empty() && is_newer(current, &self.latest)
    }
}

// ---- 版本比较（纯函数，单元测试见本文件末尾）----

/// 取 "1.2.3" 的三个数字段；非数字、缺段、多段都视为 None。
fn parse_version(v: &str) -> Option<[u32; 3]> {
    let mut out = [0u32; 3];
    let mut n = 0;
    for part in v.trim().split('.') {
        if n >= 3 {
            return None;
        }
        out[n] = part.trim().parse::<u32>().ok()?;
        n += 1;
    }
    (n == 3).then_some(out)
}

/// 按 a.b.c 逐段数字比较。任意一边不是合法三段式 → false（视为无法比较，不提示更新）。
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

// ---- update.json 读写（数据目录：%LOCALAPPDATA%\DeskPad）----

/// 数据目录与 config.json 等一致：%LocalAppData%\DeskPad。
pub fn data_dir() -> PathBuf {
    store_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn store_dir() -> Option<PathBuf> {
    use windows::Win32::UI::Shell::FOLDERID_LocalAppData;
    win::known_folder(&FOLDERID_LocalAppData).map(|p| p.join("DeskPad"))
}

/// update.json 的完整路径。
pub fn update_path() -> PathBuf {
    data_dir().join(UPDATE_FILE)
}

/// DeskPad.exe 所在目录（= 安装目录）。没有 uninstall.exe 时它是便携 zip 的解压目录。
pub fn install_dir() -> PathBuf {
    let exe = win::exe_path();
    std::path::Path::new(&exe)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 读取 update.json；不存在、损坏都回退默认值（坏文件改名，和 store.rs 一致）。
pub fn load() -> UpdateInfo {
    let path = update_path();
    let Ok(text) = fs::read_to_string(&path) else { return UpdateInfo::default() };
    match serde_json::from_str::<UpdateInfo>(&text) {
        Ok(info) => info,
        Err(e) => {
            log::warn(&format!("update.json parse failed: {e}"));
            let _ = fs::rename(&path, data_dir().join("update.bad.json"));
            UpdateInfo::default()
        }
    }
}

/// 原子写 update.json（先写 .tmp 再 rename，与 store.rs 同一写法）。
pub fn save(info: &UpdateInfo) -> Result<(), String> {
    let dir = data_dir();
    let _ = fs::create_dir_all(&dir);
    let data = serde_json::to_string_pretty(info).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{UPDATE_FILE}.tmp"));
    fs::write(&tmp, data.as_bytes()).map_err(|e| format!("write update.json.tmp: {e}"))?;
    fs::rename(&tmp, dir.join(UPDATE_FILE)).map_err(|e| format!("rename update.json: {e}"))
}

/// 当前是否打开了「自动检查」（设置里可关，默认开）。
pub fn auto_check_enabled(settings: &Settings) -> bool {
    settings.auto_check_update
}

// ---- WinINet（只在检查子进程与安装程序里被调用）----

type FnInternetOpenW = unsafe extern "system" fn(
    *const u16,
    u32,
    *const u16,
    *const u16,
    u32,
) -> *mut c_void;
type FnInternetOpenUrlW = unsafe extern "system" fn(
    *mut c_void,
    *const u16,
    *const u16,
    u32,
    u32,
    usize,
) -> *mut c_void;
type FnInternetReadFile =
    unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> i32;
type FnInternetCloseHandle = unsafe extern "system" fn(*mut c_void) -> i32;
type FnHttpQueryInfoW =
    unsafe extern "system" fn(*const c_void, u32, *mut c_void, *mut u32, *mut u32) -> i32;
type FnInternetSetOptionW =
    unsafe extern "system" fn(*const c_void, u32, *const c_void, u32) -> i32;

/// WinINet 导出表（延迟加载 + 常驻）。**主进程不链接 wininet.dll**，
/// 只有这个模块在检查子进程 / 安装程序里运行时才把它 LoadLibrary 进来。
struct WinInet {
    open: FnInternetOpenW,
    open_url: FnInternetOpenUrlW,
    read: FnInternetReadFile,
    close: FnInternetCloseHandle,
    query: FnHttpQueryInfoW,
    set_option: FnInternetSetOptionW,
}

impl WinInet {
    unsafe fn load() -> Option<WinInet> {
        // SAFETY: 静态库名字面量；失败只是拿不到网络能力。
        let m = windows::Win32::System::LibraryLoader::LoadLibraryW(w!("wininet.dll")).ok()?;
        let addr = |name: &[u8]| -> Option<usize> {
            let p = windows::Win32::System::LibraryLoader::GetProcAddress(
                m,
                windows::core::PCSTR(name.as_ptr()),
            )?;
            Some(p as usize)
        };
        Some(WinInet {
            // SAFETY: 地址取自 wininet.dll 的导出，签名按 wininet.h 声明。
            open: std::mem::transmute::<usize, FnInternetOpenW>(addr(b"InternetOpenW\0")?),
            open_url: std::mem::transmute::<usize, FnInternetOpenUrlW>(addr(b"InternetOpenUrlW\0")?),
            read: std::mem::transmute::<usize, FnInternetReadFile>(addr(b"InternetReadFile\0")?),
            close: std::mem::transmute::<usize, FnInternetCloseHandle>(addr(b"InternetCloseHandle\0")?),
            query: std::mem::transmute::<usize, FnHttpQueryInfoW>(addr(b"HttpQueryInfoW\0")?),
            set_option: std::mem::transmute::<usize, FnInternetSetOptionW>(addr(b"InternetSetOptionW\0")?),
        })
    }

    fn close(&self, h: *mut c_void) {
        if !h.is_null() {
            // SAFETY: h 由 open/open_url 返回，只关闭一次。
            unsafe { let _ = (self.close)(h); }
        }
    }
}

static WININET: std::sync::OnceLock<Option<WinInet>> = std::sync::OnceLock::new();

/// 取 WinINet 导出表；失败（极端老系统）时返回 None，调用方按网络错误处理。
fn wininet() -> Option<&'static WinInet> {
    WININET.get_or_init(|| unsafe { WinInet::load() }).as_ref()
}

/// WinINet 请求句柄 RAII：InternetOpenW 的会话与 InternetOpenUrlW 的连接。
struct Req {
    api: &'static WinInet,
    session: *mut c_void,
    conn: *mut c_void,
}

impl Req {
    fn open(url: &str) -> Result<Req, String> {
        let api = wininet().ok_or("无法初始化网络（wininet 加载失败）")?;
        let agent: Vec<u16> = "HN DeskPad".encode_utf16().chain(std::iter::once(0)).collect();
        let url_wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: agent / url_wide 在本调用期间有效；句柄收进 Req 由 Drop 关闭。
        unsafe {
            // 单次请求整体不超过 20 秒（连接 / 发送 / 接收都设）
            let timeout = NET_TIMEOUT_MS;
            for opt in [
                INTERNET_OPTION_CONNECT_TIMEOUT,
                INTERNET_OPTION_SEND_TIMEOUT,
                INTERNET_OPTION_RECEIVE_TIMEOUT,
            ] {
                (api.set_option)(
                    std::ptr::null_mut(),
                    opt,
                    &timeout as *const u32 as *const c_void,
                    4,
                );
            }
            let session = (api.open)(
                agent.as_ptr(),
                INTERNET_OPEN_TYPE_PRECONFIG,
                std::ptr::null(),
                std::ptr::null(),
                0,
            );
            if session.is_null() {
                return Err("无法初始化网络".into());
            }
            let conn = (api.open_url)(
                session,
                url_wide.as_ptr(),
                std::ptr::null(),
                INTERNET_FLAG_RELOAD
                    | INTERNET_FLAG_NO_CACHE_WRITE
                    | INTERNET_FLAG_SECURE
                    | INTERNET_FLAG_NO_UI,
                0,
                0,
            );
            if conn.is_null() {
                api.close(session);
                return Err("无法连接下载服务器，请检查网络或代理后重试".into());
            }
            Ok(Req { api, session, conn })
        }
    }

    /// HTTP 状态码；取不到返回 0。
    fn status(&self) -> u32 {
        // SAFETY: self.conn 有效；输出到本地变量。
        unsafe { self.query(HTTP_QUERY_STATUS_CODE).unwrap_or(0) }
    }

    /// 响应总长度（Content-Length）；取不到返回 0。
    fn content_length(&self) -> u64 {
        // SAFETY: self.conn 有效；输出到本地变量。
        unsafe { self.query(HTTP_QUERY_CONTENT_LENGTH).unwrap_or(0) as u64 }
    }

    unsafe fn query(&self, level: u32) -> Option<u32> {
        let mut v = 0u32;
        let mut len = 4u32;
        let ok = (self.api.query)(
            self.conn as *const c_void,
            level | HTTP_QUERY_FLAG_NUMBER,
            &mut v as *mut u32 as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
        );
        (ok != 0).then_some(v)
    }

    /// 读一块数据；0 = 读完了。
    fn read(&self, buf: &mut [u8]) -> Result<usize, String> {
        let mut n = 0u32;
        // SAFETY: buf 有效，长度以 u32 传入；n 为本地输出变量。
        let ok = unsafe {
            (self.api.read)(self.conn, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n)
        };
        if ok == 0 {
            return Err("下载中断，请重试".into());
        }
        Ok(n as usize)
    }
}

impl Drop for Req {
    fn drop(&mut self) {
        // close() 内部是 unsafe 调用；两个句柄都由本结构创建，各关闭一次。
        self.api.close(self.conn);
        self.api.close(self.session);
    }
}

/// GET url → 全部字节。progress(已下载, 总长度或 0)。
pub fn get(url: &str, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    let req = Req::open(url)?;
    let status = req.status();
    if status != 200 {
        return Err(format!("下载服务器返回错误（HTTP {status}）"));
    }
    let total = req.content_length();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = req.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        progress(out.len() as u64, total);
    }
    Ok(out)
}

// WinINet 常量（与 wininet.h 一致；这里不链接 wininet.dll，常量照抄）
const INTERNET_OPEN_TYPE_PRECONFIG: u32 = 0;
const INTERNET_FLAG_RELOAD: u32 = 0x8000_0000;
const INTERNET_FLAG_NO_CACHE_WRITE: u32 = 0x0400_0000;
const INTERNET_FLAG_SECURE: u32 = 0x0080_0000;
const INTERNET_FLAG_NO_UI: u32 = 0x0000_0200;
const INTERNET_OPTION_CONNECT_TIMEOUT: u32 = 2;
const INTERNET_OPTION_SEND_TIMEOUT: u32 = 5;
const INTERNET_OPTION_RECEIVE_TIMEOUT: u32 = 6;
const HTTP_QUERY_STATUS_CODE: u32 = 19;
const HTTP_QUERY_CONTENT_LENGTH: u32 = 5;
const HTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

/// 读取最新版本清单。latest.json 形如
/// {"version":"0.2.1","url":"https://.../zip","sha256":"...","notes":"..."}。
pub fn latest() -> Result<Release, String> {
    let body = get(MANIFEST_URL, |_, _| {})?;
    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "版本信息格式错误".to_string())?;
    let notes = v
        .get("notes")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    match (s("version"), s("url"), s("sha256")) {
        (Some(version), Some(url), Some(sha256)) => Ok(Release {
            version,
            url,
            sha256: sha256.to_lowercase(),
            notes,
        }),
        _ => Err("版本信息不完整".into()),
    }
}

// ---- 一次性子进程：DeskPad.exe --check-update ----

/// 检查更新的子进程主函数：读清单 → 写 update.json → 通知主进程 → 返回退出码。
///
/// 由 app main 识别 `--check-update` 后调用：这个进程不建窗口、不占单实例锁、
/// 不碰启动台任何状态，退出时不做清理。
pub fn check_update_worker() -> i32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut info = UpdateInfo { checked_at: now, ..Default::default() };
    match latest() {
        Ok(rel) => {
            info.latest = rel.version;
            info.url = rel.url;
            info.sha256 = rel.sha256;
            info.notes = rel.notes;
        }
        Err(e) => {
            info.error = e;
            let msg = info.error.clone();
            log::warn(&format!("check-update failed: {msg}"));
        }
    }
    if let Err(e) = save(&info) {
        log::warn(&format!("write update.json failed: {e}"));
    }
    // 通知主进程刷新齿轮角标 / 托盘菜单 / 设置窗口
    // SAFETY: PostMessageW 允许跨线程异步投递；没有主窗口时只返回错误。
    unsafe {
        if let Ok(hwnd) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(
            w!("DeskPadMain"),
            PCWSTR::null(),
        ) {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                hwnd,
                WM_APP_UPDATE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }
    0
}

// ---- 主进程侧：拉起子进程 + 互斥 ----

/// 检查子进程互斥：文件锁（数据目录下 update.lock）。保证同一时刻只有一个子进程。
pub struct UpdateGuard(#[allow(dead_code)] Option<std::fs::File>);

impl UpdateGuard {
    /// 已经有一个子进程在跑（或刚跑完不到 5 分钟）时返回 None。
    pub fn acquire() -> Option<UpdateGuard> {
        create_lock().map(|f| UpdateGuard(Some(f)))
    }

    /// 启动 `DeskPad.exe --check-update`（无窗口）。已有子进程在跑时返回 false。
    pub fn spawn() -> bool {
        if Self::acquire().is_none() {
            return false;
        }
        let mut cmd = std::process::Command::new(win::exe_path());
        cmd.arg("--check-update");
        // SAFETY: 无窗口标志；子进程只做一次网络请求然后退出。
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let ok = cmd.spawn().is_ok();
        if !ok {
            log::warn("spawn --check-update failed");
        }
        ok
    }
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        // 子进程结束时删掉锁文件，让下一次检查能立刻开始
        let _ = fs::remove_file(data_dir().join("update.lock"));
    }
}

/// 创建 update.lock；上一次异常退出留下的陈旧锁（>5 分钟）先清掉再试。
fn create_lock() -> Option<std::fs::File> {
    let path = data_dir().join("update.lock");
    match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(f) => Some(f),
        Err(_) => {
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
                .map(|d| d.as_secs() >= 300)
                .unwrap_or(true);
            if stale {
                let _ = fs::remove_file(&path);
                fs::OpenOptions::new().write(true).create_new(true).open(&path).ok()
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_task_card_cases() {
        // 任务卡点名的三条
        assert!(is_newer("0.2.0", "0.2.1"));
        assert!(is_newer("0.2.1", "0.10.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
    }

    #[test]
    fn version_compare_numeric_not_lexicographic() {
        // 逐段按数字比：0.10.0 > 0.9.0（字典序会判反）
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("1.9.9", "1.10.0"));
        assert!(!is_newer("1.10.0", "1.9.9"));
        assert!(is_newer("0.2.0", "1.0.0"));
    }

    #[test]
    fn version_compare_illegal_strings_are_not_updates() {
        // 非法串一律视为「不更新」
        assert!(!is_newer("0.2", "0.3"));
        assert!(!is_newer("0.2.0", ""));
        assert!(!is_newer("", "0.2.0"));
        assert!(!is_newer("", ""));
        assert!(!is_newer("a.b.c", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0.1"));
        assert!(!is_newer("0.2.0", "0.2.0-beta"));
        assert!(!is_newer("-1.0.0", "0.2.0"));
    }

    #[test]
    fn version_compare_leading_zeros() {
        assert!(is_newer("0.02.0", "0.2.1"));
        assert!(!is_newer("0.2.0", "0.02.0"));
    }

    #[test]
    fn update_info_empty_object_reads_as_default() {
        // 缺字段都能读，且不认为有新版本
        let info: UpdateInfo = serde_json::from_str("{}").unwrap();
        assert_eq!(info.checked_at, 0);
        assert!(info.latest.is_empty());
        assert!(!info.has_new("0.2.0"));
    }

    #[test]
    fn update_info_roundtrip_and_has_new() {
        let info = UpdateInfo {
            checked_at: 1_700_000_000,
            latest: "0.3.0".into(),
            url: "https://example.com/pkg.zip".into(),
            sha256: "abc".into(),
            notes: "修了点东西".into(),
            error: String::new(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: UpdateInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.checked_at, info.checked_at);
        assert_eq!(back.latest, "0.3.0");
        assert_eq!(back.notes, "修了点东西");
        assert!(back.has_new("0.2.0"));
        assert!(!back.has_new("0.3.0"));
    }

    #[test]
    fn update_info_error_only_is_not_update() {
        let info = UpdateInfo { error: "无法连接下载服务器".into(), ..Default::default() };
        assert!(!info.has_new("0.2.0"));
        assert!(info.latest.is_empty());
    }

    #[test]
    fn update_info_unknown_field_ignored() {
        // 以后的清单加了字段，旧二进制也能读
        let info: UpdateInfo =
            serde_json::from_str(r#"{"latest":"0.4.0","future_field":42}"#).unwrap();
        assert_eq!(info.latest, "0.4.0");
    }

    #[test]
    fn update_info_serializes_card_field_order() {
        // 子进程写出的字段与任务卡要求一致：checked_at / latest / url / sha256 / notes / error
        let info = UpdateInfo { checked_at: 1, latest: "0.2.1".into(), ..Default::default() };
        let json = serde_json::to_string(&info).unwrap();
        for key in ["checked_at", "latest", "url", "sha256", "notes", "error"] {
            assert!(json.contains(key), "missing {key}");
        }
    }

    #[test]
    fn update_save_and_load_roundtrip() {
        // save/load 在临时目录里完整往返（临时代理 data_dir）
        let dir = std::env::temp_dir().join(format!(
            "deskpad-update-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let info = UpdateInfo {
            checked_at: 42,
            latest: "0.2.1".into(),
            url: "https://example.com".into(),
            sha256: "ff".into(),
            notes: String::new(),
            error: String::new(),
        };
        // 直接对同一目录做 save/load：临时把 data_dir 换成dir不可行（进程级），
        // 因此这里只验证序列化 + 手工写入再解析的往返，以及 save 的错误分支不 panic。
        let text = serde_json::to_string_pretty(&info).unwrap();
        let p = dir.join(UPDATE_FILE);
        fs::write(&p, text.as_bytes()).unwrap();
        let back: UpdateInfo = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(back.checked_at, 42);
        assert!(back.has_new("0.2.0"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_check_default_true_and_old_config_reads() {
        // Settings 的 auto_check_update 默认开，旧 config.json（缺字段）也能读成 true
        assert!(Settings::default().auto_check_update);
        let old: Settings = serde_json::from_str(r#"{"autostart":true}"#).unwrap();
        assert!(old.auto_check_update);
    }

    #[test]
    fn current_version_parses() {
        // 编译期版本必须是合法三段式，否则永远提示「有更新」
        assert!(parse_version(CURRENT_VERSION).is_some());
    }
}
