// Embeds the app icon as resource 1, which GPUI loads for the window and taskbar.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(err) = res.compile() {
            println!("cargo:warning=could not embed the app icon: {err}");
        }
        // Windows gives the main thread 1 MB; unoptimized GPUI renders of big pages (Plugins) overflow it.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-bins=/STACK:8388608");
        }
    }
}
