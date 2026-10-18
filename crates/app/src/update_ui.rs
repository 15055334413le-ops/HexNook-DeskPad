//! 在线更新的 UI 侧：齿轮角标、检查时机、执行更新（UI 文档 6.11 / questions.md #35）。
//!
//! 三条红线在这里落地：**不加定时器、UI 不开线程、不发托盘气泡**。检查更新只发生在
//! ① 启动台每次打开时（auto_check_update 且距上次 ≥24 小时）和 ② 设置窗口点「检查更新」，
//! 都是拉起一个一次性子进程 `DeskPad.exe --check-update`；子进程写完 update.json 后
//! PostMessage WM_APP_UPDATE 回来，主进程只在那一刻读一次文件、重绘一次。
use crate::app::with_app;
use deskpad_engine::update::{self, UpdateInfo};
use std::time::{SystemTime, UNIX_EPOCH};

/// 当前显示给用户的更新状态（设置窗口那一行 + 齿轮角标 + 托盘菜单都看它）。
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    /// 还没检查过（或检查失败了）：只显示「检查更新」次按钮。
    Unknown,
    /// 子进程正在跑。
    Checking,
    /// 检查完成：已是最新。
    Latest,
    /// 检查完成：发现新版本 x.y.z。
    New(String),
    /// 检查完成但出错（网络 / 清单格式）。
    Failed(String),
}

impl UpdateState {
    /// 是否有「立即更新」可点。
    pub fn has_new(&self) -> bool {
        matches!(self, UpdateState::New(_))
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 从 update.json 推导当前状态；checked_at 为 0（从未检查）→ Unknown。
pub fn state_from(info: &UpdateInfo) -> UpdateState {
    if info.checked_at <= 0 {
        return UpdateState::Unknown;
    }
    if !info.error.is_empty() {
        return UpdateState::Failed(info.error.clone());
    }
    if info.has_new(update::CURRENT_VERSION) {
        return UpdateState::New(info.latest.clone());
    }
    UpdateState::Latest
}

/// 启动台每次 open 时调用：满足条件就拉起一次性子进程（不加定时器、不开线程）。
///
/// 在 pad::show 的 with_app 借用**之外**调用（借用重入时 `with_app` 会直接返回 None，
/// 那样就永远查不到了）。调用方负责先取出设置。
pub fn maybe_auto_check(enabled: bool, last: i64) {
    // 距上次检查不足 24 小时就不查（避免每次打开启动台都联网）
    if last > 0 && now_unix().saturating_sub(last) < update::AUTO_CHECK_INTERVAL {
        return;
    }
    if !enabled {
        return;
    }
    update::UpdateGuard::spawn();
}

/// 设置窗口「检查更新」按钮：立即拉起一次子进程。
pub fn check_now() {
    update::UpdateGuard::spawn();
}

/// WM_APP_UPDATE：子进程写完 update.json，读回来刷新状态并重绘角标 / 设置窗口。
pub fn on_checked() {
    let info = update::load();
    let state = state_from(&info);
    with_app(|a| a.update = state.clone());
    sync_ui(state);
}

/// 把最新状态同步到角标（重绘）和设置窗口（如果开着）。
fn sync_ui(state: UpdateState) {
    // 先取出窗口句柄再调用：set_update_state 会重绘，重绘里还要借 App，
    // 放在 with_app 里面会被重入保护挡掉，设置窗口就一直停在「正在检查…」
    if let Some(sw) = with_app(|a| a.settings_wnd).flatten() {
        crate::settings_ui::set_update_state(sw, state.clone());
    }
    crate::corner::redraw();
}

/// 「立即更新」：安装目录里有 uninstall.exe → 用安装程序自更新；否则打开发布仓库。
pub fn apply_update() {
    let dir = update::install_dir();
    let un = dir.join("uninstall.exe");
    if !un.exists() {
        // 便携 zip 用户：打开发布仓库自己下载
        open_url(update::RELEASES_URL);
        return;
    }
    {
        let temp = std::env::temp_dir().join("HN-DeskPad-Update.exe");
        if std::fs::copy(&un, &temp).is_err() {
            deskpad_engine::log::warn("copy uninstaller to temp failed");
            return;
        }
        // 复制到 %TEMP% 再以 --update 启动，安装程序会让 DeskPad 自己退出
        let mut cmd = std::process::Command::new(&temp);
        cmd.arg("--update").arg(&dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Err(e) = cmd.spawn() {
            deskpad_engine::log::warn(&format!("spawn --update failed: {e}"));
        } else {
            // DeskPad 这边什么都不用做：安装程序 stop_running() 会让我们退出
        }
    }
}

/// 用默认浏览器打开一个网址（ShellExecute open）。
fn open_url(url: &str) {
    // SAFETY: 字符串在调用期间有效；不关心返回值。
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            &windows::core::HSTRING::from(url),
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}
