//! RegisterHotKey 封装（UI 文档 6.3）。红线：全局输入只允许通过 RegisterHotKey 获取。
use deskpad_engine::HotkeyPreset;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};

const HOTKEY_ID: i32 = 1;

/// 注册热键。失败（被占用）返回 false，由调用方发托盘通知。
pub fn register(hwnd: HWND, preset: HotkeyPreset) -> bool {
    let (m, vk) = preset.mods_vk();
    // SAFETY: 主窗口句柄有效；热键在退出流程中 UnregisterHotKey。
    unsafe { RegisterHotKey(hwnd, HOTKEY_ID, HOT_KEY_MODIFIERS(m) | MOD_NOREPEAT, vk) }.is_ok()
}

pub fn unregister(hwnd: HWND) {
    // SAFETY: 与 register 配对；重复注销被忽略。
    unsafe { let _ = UnregisterHotKey(hwnd, HOTKEY_ID); }
}

/// 换热键：先注销旧的再注册新的；失败就退回旧的，返回 false 给设置窗口行内报错。
pub fn change(hwnd: HWND, old: HotkeyPreset, new: HotkeyPreset) -> bool {
    unregister(hwnd);
    if register(hwnd, new) {
        return true;
    }
    register(hwnd, old);
    false
}
