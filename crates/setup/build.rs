//! 给安装程序嵌入清单：asInvoker（文件名含 Setup 时，Windows 的「安装程序检测」会在没有清单的 exe 上自动要求管理员；
//! DeskPad 拒绝以管理员运行，安装程序也必须是普通权限，安装到当前用户），PerMonitorV2 DPI，Common Controls v6。
fn main() {
    println!("cargo:rerun-if-changed=setup.manifest");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let m = std::path::Path::new(&dir).join("setup.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
    }
}
