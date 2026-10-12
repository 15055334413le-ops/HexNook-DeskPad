//! HN DeskPad 安装程序（在线下载器）。
//! - 无参数：安装 / 更新界面。选择位置后自动在其中创建「HN DeskPad」文件夹。
//! - `--uninstall [--quiet]`：卸载（由「应用和功能」调用）。
#![windows_subsystem = "windows"]

mod install;
mod net;
mod ui;

use std::path::PathBuf;
use windows::core::{w, HSTRING};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::*;

fn main() {
    // SAFETY: 主线程首次初始化 COM（STA：文件夹选择对话框、快捷方式）。
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--uninstall") {
        let quiet = args.iter().any(|a| a == "--quiet");
        run_uninstall(quiet);
        return;
    }
    ui::run();
}

fn run_uninstall(quiet: bool) {
    let dir = install::installed_dir()
        .or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)));
    let Some(dir) = dir else { return };
    if !quiet {
        let msg = "确定要卸载 HN DeskPad 吗？\n\n程序文件、快捷方式、开机自启以及设置和缓存都会被删除。";
        // SAFETY: 字符串在调用期间有效。
        let r = unsafe { MessageBoxW(None, &HSTRING::from(msg), w!("卸载 HN DeskPad"), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) };
        if r != IDYES {
            return;
        }
    }
    install::uninstall(&dir, true);
    if !quiet {
        // SAFETY: 同上。
        unsafe { MessageBoxW(None, w!("HN DeskPad 已卸载。"), w!("卸载 HN DeskPad"), MB_OK | MB_ICONINFORMATION) };
    }
}
