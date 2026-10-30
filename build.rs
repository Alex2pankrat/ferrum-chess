fn main() {
    println!("cargo:rerun-if-changed=assets/ferrum-icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/ferrum-icon.ico")
            .compile()
            .expect("embed the Windows application icon");
    }
}
