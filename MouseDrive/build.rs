fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=image/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    winresource::WindowsResource::new()
        .set_icon("image/icon.ico")
        .set("ProductName", "MouseDrive")
        .set("FileDescription", "MouseDrive")
        .compile()
}
