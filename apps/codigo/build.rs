// SPDX-License-Identifier: MIT OR Apache-2.0
//! Windows only: embed the app icon and version info into `codigo.exe`, so Explorer, the taskbar
//! and the Start menu shortcut show Código's icon.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from Linux or macOS still links, unless `CODIGO_REQUIRE_WINRES=1` (set by the
//! release packaging) turns it into an error.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/icons/codigo.ico");
    println!("cargo:rerun-if-env-changed=CODIGO_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icons/codigo.ico")
        .set("ProductName", "Código")
        .set("FileDescription", "Código")
        .set("CompanyName", "Nodalia")
        .set(
            "LegalCopyright",
            "Copyright (c) the Código authors. MIT OR Apache-2.0.",
        )
        .set("OriginalFilename", "codigo.exe")
        .set("InternalName", "codigo");
    if let Err(e) = res.compile() {
        if std::env::var_os("CODIGO_REQUIRE_WINRES").is_some() {
            println!("cargo:warning=embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=codigo.exe built without icon/version resources: {e}");
    }
}
