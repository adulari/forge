fn main() {
    // src/vblank.rs: libwebkit's `drmWaitVBlank` calls only resolve to the executable's definition
    // when the executable exports it in its dynamic symbol table.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic-symbol=drmWaitVBlank");
    }
    tauri_build::build()
}
