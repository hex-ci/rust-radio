//! Windows 资源：把图标和版本信息编进 exe。
//!
//! 任务栏按钮的图标取自窗口的 ICON_BIG；winit 只设了 ICON_SMALL（egui 的
//! `ViewportBuilder::with_icon` 走的也是这条），窗口类本身 `hIcon: 0`，
//! 于是任务栏会退回 exe 里的图标资源——没有内嵌资源就是系统默认的空图标。
//! 资源管理器、固定到任务栏、Alt-Tab 的图标同样来自这里。

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");

    // 资源和 rc.exe 只对 Windows 目标有意义；build script 按构建机平台编译，
    // 所以非 Windows 构建机这里整段不参与编译。
    #[cfg(windows)]
    windows_resource();
}

#[cfg(windows)]
fn windows_resource() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/app.ico");
    res.set("ProductName", "云听电台");
    res.set("FileDescription", "云听电台");
    res.set(
        "LegalCopyright",
        "MIT License. 非官方客户端，仅供学习与个人使用。",
    );
    res.set("OriginalFilename", "rust-radio.exe");
    if let Err(err) = res.compile() {
        // 缺 rc.exe / windres 时不要让整条构建挂掉，只提示
        println!("cargo:warning=Windows 资源编译失败，exe 将会没有图标：{err}");
    }
}
