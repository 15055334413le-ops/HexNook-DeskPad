//! HKCU Run 键 + StartupApproved（engine 文档 4.7 autostart 部分）。
use std::path::Path;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY, REG_OPTION_NON_VOLATILE,
    REG_VALUE_TYPE, REG_SZ,
};

use crate::model::{EngineError, Settings};
use crate::win;

fn run_subkey() -> PCWSTR {
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
}
fn approved_subkey() -> PCWSTR {
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run")
}
fn value_name() -> PCWSTR {
    w!("DeskPad")
}

/// 真实状态 = Run 值存在，且 StartupApproved 里的 DeskPad 值不存在，或其第一个字节为偶数。
pub(crate) fn real_state() -> bool {
    if query_run_value().is_none() {
        return false;
    }
    match query_approved_first_byte() {
        None => true,
        Some(b) => b % 2 == 0,
    }
}

/// 启用：写 Run 值 + 删除 StartupApproved 里的禁用标记；禁用：删 Run 值。
/// 只有 exe 文件名是 DeskPad.exe 时才允许写注册表，其他进程（probe 等）一律跳过并记 info。
pub(crate) fn set(enable: bool) -> Result<(), EngineError> {
    let exe = win::exe_path();
    let is_deskpad = Path::new(&exe)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase() == "deskpad.exe")
        .unwrap_or(false);
    if !is_deskpad {
        crate::log::info(&format!("autostart::set skipped (exe is not DeskPad.exe): {exe}"));
        return Ok(());
    }
    if enable {
        let cmd = format!("\"{exe}\" --autostart");
        let wide = win::to_wide(&cmd);
        let data = wide_bytes(&wide);
        write_run_value(data)?;
        clear_approved_flag()?;
        Ok(())
    } else {
        delete_run_value()
    }
}

/// init 时：用真实状态覆盖 settings.autostart；真实启用但 Run 路径与当前 exe 不一致 → 重写。
pub(crate) fn sync_on_init(s: &mut Settings) {
    let real = real_state();
    s.autostart = real;
    if real {
        if let Some(v) = query_run_value() {
            let expected = format!("\"{}\" --autostart", win::exe_path());
            if v.trim() != expected {
                if let Err(e) = set(true) {
                    crate::log::warn(&format!("autostart rewrite after exe move failed: {e:?}"));
                }
            }
        }
    }
}

fn write_run_value(data: &[u8]) -> Result<(), EngineError> {
    let mut hk = HKEY::default();
    // SAFETY: phkresult 指向本函数持有的 HKEY，成功后由 HKeyGuard 关闭。
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            run_subkey(),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hk,
            None,
        )
    };
    if err != ERROR_SUCCESS {
        return Err(EngineError::Io(format!("RegCreateKeyExW(Run): {err:?}")));
    }
    let _g = win::HKeyGuard(hk);
    // SAFETY: 句柄有效；数据指针/长度来自调用方切片。
    let err = unsafe { RegSetValueExW(hk, value_name(), 0, REG_SZ, Some(data)) };
    if err != ERROR_SUCCESS {
        return Err(EngineError::Io(format!("RegSetValueExW(Run): {err:?}")));
    }
    Ok(())
}

fn clear_approved_flag() -> Result<(), EngineError> {
    let mut hk = HKEY::default();
    // SAFETY: 同 write_run_value。
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            approved_subkey(),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hk,
            None,
        )
    };
    if err != ERROR_SUCCESS {
        return Err(EngineError::Io(format!("RegCreateKeyExW(StartupApproved): {err:?}")));
    }
    let _g = win::HKeyGuard(hk);
    // SAFETY: 句柄有效；值不存在（ERROR_FILE_NOT_FOUND）视为已清理。
    let err = unsafe { RegDeleteValueW(hk, value_name()) };
    if err != ERROR_SUCCESS && err != ERROR_FILE_NOT_FOUND {
        return Err(EngineError::Io(format!("RegDeleteValueW(StartupApproved): {err:?}")));
    }
    Ok(())
}

fn delete_run_value() -> Result<(), EngineError> {
    let mut hk = HKEY::default();
    // SAFETY: phkresult 指向本函数持有的 HKEY。
    let err = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, run_subkey(), 0, KEY_SET_VALUE, &mut hk) };
    if err != ERROR_SUCCESS {
        return Ok(()); // Run 键都不存在 → 已是禁用状态
    }
    let _g = win::HKeyGuard(hk);
    // SAFETY: 句柄有效；值不存在视为已禁用。
    let err = unsafe { RegDeleteValueW(hk, value_name()) };
    if err != ERROR_SUCCESS && err != ERROR_FILE_NOT_FOUND {
        return Err(EngineError::Io(format!("RegDeleteValueW(Run): {err:?}")));
    }
    Ok(())
}

fn query_run_value() -> Option<String> {
    let mut hk = HKEY::default();
    // SAFETY: phkresult 指向本函数持有的 HKEY。
    let err = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, run_subkey(), 0, KEY_QUERY_VALUE, &mut hk) };
    if err != ERROR_SUCCESS {
        return None;
    }
    let _g = win::HKeyGuard(hk);
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let mut ty = REG_VALUE_TYPE::default();
    // SAFETY: 缓冲区与长度都由本函数持有；size 为系统写入的字节数。
    let err = unsafe {
        RegQueryValueExW(
            hk,
            value_name(),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS || ty != REG_SZ {
        return None;
    }
    let chars = (size as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..chars]);
    Some(s.trim_end_matches('\0').to_string())
}

fn query_approved_first_byte() -> Option<u8> {
    let mut hk = HKEY::default();
    // SAFETY: phkresult 指向本函数持有的 HKEY。
    let err =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, approved_subkey(), 0, KEY_QUERY_VALUE, &mut hk) };
    if err != ERROR_SUCCESS {
        return None;
    }
    let _g = win::HKeyGuard(hk);
    let mut buf = [0u8; 32];
    let mut size = buf.len() as u32;
    let mut ty = REG_VALUE_TYPE::default();
    // SAFETY: 缓冲区与长度都由本函数持有。
    let err = unsafe {
        RegQueryValueExW(
            hk,
            value_name(),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS || ty != REG_BINARY || size == 0 {
        return None;
    }
    Some(buf[0])
}

/// UTF-16 切片按字节视图（含结尾 0），供 RegSetValueExW 使用。
fn wide_bytes(v: &[u16]) -> &[u8] {
    // SAFETY: u16 → u8 只改变解释方式，缓冲区仍由调用方持有且对齐满足 u8。
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 2) }
}
