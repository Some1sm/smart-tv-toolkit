//! Windows: embed the icon and version details shown in Explorer's
//! Properties dialog. Other platforms get their icon from the app bundle.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "Smart TV Toolkit")
        .set("FileDescription", "Smart TV Toolkit")
        .set("LegalCopyright", "Copyright (C) 2026 Some1sm and steven-ahfu. AGPL-3.0.");
    if let Err(e) = res.compile() {
        // Without the resource compiler (rc.exe) the app still builds, just
        // without an Explorer icon; say so instead of failing.
        println!("cargo:warning=Windows icon not embedded: {e}");
    }
}
