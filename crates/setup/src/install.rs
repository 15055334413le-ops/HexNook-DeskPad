//! 安装 / 卸载的实际动作（全部按当前用户进行，不需要管理员）。
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use windows::core::{w, Interface, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const PRODUCT: &str = "HN DeskPad";
pub const PUBLISHER: &str = "Hex Nook";
pub const EXE: &str = "DeskPad.exe";
pub const UNINSTALLER: &str = "uninstall.exe";
const UNINSTALL_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\HN DeskPad");
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APPROVED_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
/// DeskPad 主窗口类名 / 正常退出消息（app.rs WM_APP_QUIT = WM_APP + 5）。
const MAIN_CLASS: PCWSTR = w!("DeskPadMain");
const WM_APP_QUIT: u32 = WM_APP + 5;

/// 用户选的位置 → 实际安装目录：自动在后面加一层「HN DeskPad」文件夹（已经以它结尾就不再加）。
pub fn target_dir(parent: &Path) -> PathBuf {
    let ends = parent
        .file_name()
        .map(|n| n.to_string_lossy().eq_ignore_ascii_case(PRODUCT))
        .unwrap_or(false);
    if ends {
        parent.to_path_buf()
    } else {
        parent.join(PRODUCT)
    }
}

/// 默认位置：已安装过 → 原安装目录的上一级；否则 %LOCALAPPDATA%\Programs。
pub fn default_parent() -> PathBuf {
    if let Some(dir) = installed_dir() {
        if let Some(p) = dir.parent() {
            return p.to_path_buf();
        }
    }
    known_folder(&FOLDERID_UserProgramFiles)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs")))
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

/// 已安装时的安装目录（卸载信息里的 InstallLocation）。
pub fn installed_dir() -> Option<PathBuf> {
    let s = reg_get_string(HKEY_CURRENT_USER, UNINSTALL_KEY, w!("InstallLocation"))?;
    let p = PathBuf::from(s);
    p.join(EXE).exists().then_some(p)
}

pub fn installed_version() -> Option<String> {
    installed_dir()?;
    reg_get_string(HKEY_CURRENT_USER, UNINSTALL_KEY, w!("DisplayVersion"))
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// 创建目录并确认可写（选到需要管理员权限的位置时给出明确提示）。
pub fn ensure_writable(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|_| no_write_msg())?;
    let probe = dir.join(".hn-write-test");
    fs::write(&probe, b"").map_err(|_| no_write_msg())?;
    let _ = fs::remove_file(probe);
    Ok(())
}

fn no_write_msg() -> String {
    "无法写入这个位置（可能需要管理员权限），请换一个文件夹".into()
}

/// 请正在运行的 DeskPad 退出：先发正常退出消息（托盘图标随之消失），2 秒未退出再强制结束。
pub fn stop_running() {
    for _ in 0..3 {
        // SAFETY: 类名为静态字面量。
        let Ok(hwnd) = (unsafe { FindWindowW(MAIN_CLASS, PCWSTR::null()) }) else { return };
        let mut pid = 0u32;
        // SAFETY: hwnd 有效；pid 为本地变量。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        // SAFETY: 打开进程句柄，用完关闭。
        let Ok(h) = (unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) }) else { return };
        // SAFETY: 句柄有效。
        unsafe {
            let _ = PostMessageW(hwnd, WM_APP_QUIT, WPARAM(0), LPARAM(0));
            if WaitForSingleObject(h, 2000) != WAIT_OBJECT_0 {
                let _ = TerminateProcess(h, 0);
                let _ = WaitForSingleObject(h, 3000);
            }
            let _ = CloseHandle(h);
        }
    }
}

/// 解压发布包（zip，顶层一个 DeskPad/ 文件夹）到 dir，去掉顶层目录。返回写入的字节数。
pub fn extract(zip: &[u8], dir: &Path) -> Result<u64, String> {
    let bad = || "安装包已损坏，请重试".to_string();
    let u16le = |i: usize| zip.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32le = |i: usize| zip.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let eocd = (0..zip.len().saturating_sub(21)).rev().find(|&i| u32le(i) == Some(0x0605_4b50)).ok_or_else(bad)?;
    let mut i = u32le(eocd + 16).ok_or_else(bad)? as usize;
    let mut total = 0u64;
    while u32le(i) == Some(0x0201_4b50) {
        let method = u16le(i + 10).ok_or_else(bad)?;
        let csize = u32le(i + 20).ok_or_else(bad)? as usize;
        let usize_ = u32le(i + 24).ok_or_else(bad)? as usize;
        let nlen = u16le(i + 28).ok_or_else(bad)? as usize;
        let xlen = u16le(i + 30).ok_or_else(bad)? as usize;
        let clen = u16le(i + 32).ok_or_else(bad)? as usize;
        let loff = u32le(i + 42).ok_or_else(bad)? as usize;
        let name = String::from_utf8_lossy(zip.get(i + 46..i + 46 + nlen).ok_or_else(bad)?).replace('\\', "/");
        i += 46 + nlen + xlen + clen;
        if name.ends_with('/') {
            continue;
        }
        // 去掉顶层文件夹；拒绝 .. 之类的路径
        let rel: Vec<&str> = name.split('/').filter(|s| !s.is_empty()).collect();
        let rel = if rel.len() > 1 { &rel[1..] } else { &rel[..] };
        if rel.iter().any(|s| *s == ".." || s.contains(':')) {
            return Err(bad());
        }
        if u32le(loff) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let skip = 30 + u16le(loff + 26).ok_or_else(bad)? as usize + u16le(loff + 28).ok_or_else(bad)? as usize;
        let data = zip.get(loff + skip..loff + skip + csize).ok_or_else(bad)?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, usize_).map_err(|_| bad())?,
            _ => return Err(bad()),
        };
        let out = rel.iter().fold(dir.to_path_buf(), |p, s| p.join(s));
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).map_err(|_| no_write_msg())?;
        }
        fs::write(&out, &bytes).map_err(|e| format!("写入 {} 失败：{e}", out.display()))?;
        total += bytes.len() as u64;
    }
    if !dir.join(EXE).exists() {
        return Err(bad());
    }
    Ok(total)
}

/// 把安装程序自身复制成卸载程序。
pub fn copy_uninstaller(dir: &Path) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dst = dir.join(UNINSTALLER);
    // 发布包里已经带了 uninstall.exe（安装程序自己）时不再复制自身（questions.md #35）
    if dst.exists() {
        return Ok(());
    }
    // 从 %TEMP% 启动（自更新）时绝不能把 %TEMP% 下的自己当成卸载程序写进去
    if is_temp_copy(&me) {
        return Ok(());
    }
    if me != dst {
        fs::copy(&me, &dst).map_err(|e| format!("写入卸载程序失败：{e}"))?;
    }
    Ok(())
}

/// 是否正运行在 %TEMP% 里的副本（自更新时由 DeskPad 复制出来的那份）。
fn is_temp_copy(exe: &Path) -> bool {
    let temp = std::env::var_os("TEMP")
        .or_else(|| std::env::var_os("TMP"))
        .map(PathBuf::from)
        .unwrap_or_default();
    if temp.as_os_str().is_empty() {
        return false;
    }
    // 前缀比较（大小写不敏感）：TEMP 与 exe 都规范化成绝对路径
    let exe_lower = exe.to_string_lossy().to_lowercase().replace('/', "\\");
    let temp_lower = temp.to_string_lossy().to_lowercase().replace('/', "\\");
    exe_lower.starts_with(&temp_lower)
}

pub fn start_menu_link() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs).map(|p| p.join(format!("{PRODUCT}.lnk")))
}
pub fn desktop_link() -> Option<PathBuf> {
    known_folder(&FOLDERID_Desktop).map(|p| p.join(format!("{PRODUCT}.lnk")))
}

/// 创建快捷方式（调用线程须已初始化 COM）。
pub fn make_link(link: &Path, target: &Path) -> Result<(), String> {
    let err = |e: windows::core::Error| format!("创建快捷方式失败：{e}");
    // SAFETY: 标准 COM 调用；字符串在调用期间有效。
    unsafe {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        sl.SetPath(&HSTRING::from(target.as_os_str())).map_err(err)?;
        if let Some(d) = target.parent() {
            sl.SetWorkingDirectory(&HSTRING::from(d.as_os_str())).map_err(err)?;
        }
        sl.SetDescription(w!("HN DeskPad 启动台（Alt + Space）")).map_err(err)?;
        let pf: IPersistFile = sl.cast().map_err(err)?;
        pf.Save(&HSTRING::from(link.as_os_str()), true).map_err(err)?;
    }
    Ok(())
}

/// 写「应用和功能」里的卸载信息（HKCU）。
pub fn register(dir: &Path, version: &str, size_bytes: u64) -> Result<(), String> {
    let exe = dir.join(EXE);
    let un = dir.join(UNINSTALLER);
    let mut key = HKEY::default();
    // SAFETY: 输出句柄为本地变量，用完关闭。
    let rc = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, UNINSTALL_KEY, 0, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
    };
    if rc != ERROR_SUCCESS {
        return Err("写入卸载信息失败".into());
    }
    let s = |name: PCWSTR, v: &str| set_sz(key, name, v);
    s(w!("DisplayName"), PRODUCT);
    s(w!("DisplayVersion"), version);
    s(w!("Publisher"), PUBLISHER);
    s(w!("InstallLocation"), &dir.to_string_lossy());
    s(w!("DisplayIcon"), &format!("{},0", exe.display()));
    s(w!("UninstallString"), &format!("\"{}\" --uninstall", un.display()));
    s(w!("QuietUninstallString"), &format!("\"{}\" --uninstall --quiet", un.display()));
    set_dword(key, w!("NoModify"), 1);
    set_dword(key, w!("NoRepair"), 1);
    set_dword(key, w!("EstimatedSize"), (size_bytes / 1024).max(1) as u32);
    // SAFETY: 句柄由 RegCreateKeyExW 返回。
    unsafe {
        let _ = RegCloseKey(key);
    }
    Ok(())
}

/// 卸载：退出程序 → 删开机自启、快捷方式、卸载信息、用户数据 → 稍后由 cmd 删除安装目录（包括本卸载程序）。
pub fn uninstall(dir: &Path, remove_data: bool) {
    stop_running();
    // SAFETY: 删除不存在的值也只是返回错误码。
    unsafe {
        let mut k = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_SET_VALUE, &mut k) == ERROR_SUCCESS {
            let _ = RegDeleteValueW(k, w!("DeskPad"));
            let _ = RegCloseKey(k);
        }
        if RegOpenKeyExW(HKEY_CURRENT_USER, APPROVED_KEY, 0, KEY_SET_VALUE, &mut k) == ERROR_SUCCESS {
            let _ = RegDeleteValueW(k, w!("DeskPad"));
            let _ = RegCloseKey(k);
        }
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, UNINSTALL_KEY);
    }
    for l in [start_menu_link(), desktop_link()].into_iter().flatten() {
        let _ = fs::remove_file(l);
    }
    if remove_data {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let _ = fs::remove_dir_all(PathBuf::from(local).join("DeskPad"));
        }
    }
    // 除自身外的文件立即删；目录在本进程退出后由隐藏的 cmd 删除
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().eq_ignore_ascii_case(UNINSTALLER) {
                continue;
            }
            let p = e.path();
            let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
        }
    }
    let cmd = format!("/c ping 127.0.0.1 -n 3 >nul & rmdir /s /q \"{}\"", dir.display());
    let _ = std::process::Command::new("cmd.exe").raw_arg_compat(&cmd).spawn();
}

/// std::process::Command 的 raw_arg 只在 Windows 上有；其他平台（本地静态检查）退化为普通参数。
trait RawArg {
    fn raw_arg_compat(&mut self, s: &str) -> &mut Self;
}
impl RawArg for std::process::Command {
    #[cfg(windows)]
    fn raw_arg_compat(&mut self, s: &str) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000).raw_arg(s)
    }
    #[cfg(not(windows))]
    fn raw_arg_compat(&mut self, s: &str) -> &mut Self {
        self.arg(s)
    }
}

pub fn launch(exe: &Path) {
    // SAFETY: 字符串在调用期间有效。
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(exe.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    // SAFETY: 返回的字符串由 CoTaskMemFree 释放。
    unsafe {
        let p: PWSTR = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn reg_get_string(root: HKEY, sub: PCWSTR, name: PCWSTR) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: 缓冲区长度以字节传入。
    let rc = unsafe {
        RegGetValueW(root, sub, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn set_sz(key: HKEY, name: PCWSTR, v: &str) {
    let w: Vec<u16> = v.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: 字节切片在调用期间有效。
    unsafe {
        let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
        let _ = RegSetValueExW(key, name, 0, REG_SZ, Some(bytes));
    }
}

fn set_dword(key: HKEY, name: PCWSTR, v: u32) {
    // SAFETY: 同上。
    unsafe {
        let _ = RegSetValueExW(key, name, 0, REG_DWORD, Some(&v.to_le_bytes()));
    }
}
