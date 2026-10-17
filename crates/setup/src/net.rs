//! 下载：WinINet（跟随系统代理设置 —— Clash 等「系统代理」模式下也能访问 GitHub）。
//!
//! 实现直接复用引擎的 `deskpad_engine::update`（同一份代码：`--check-update` 子进程
//! 和安装程序走同一条下载路径，见 questions.md #35）。
pub use deskpad_engine::update::{get, latest, Release};
