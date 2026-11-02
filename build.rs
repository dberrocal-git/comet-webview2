fn main() {
    println!("cargo:rerun-if-changed=assets/comet.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set("ProductName", "Comet WebView2")
            .set("FileDescription", env!("CARGO_PKG_DESCRIPTION"))
            .set_icon("assets/comet.ico")
            .compile()
            .expect("Could not embed the Windows application icon");
    }
}
