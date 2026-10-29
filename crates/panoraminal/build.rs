fn main() {
    println!("cargo:rerun-if-changed=assets/icons/panoraminal_icon.ico");
    // Embed the application icon and version info in the Windows executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/panoraminal_icon.ico");
        res.set("ProductName", "Panoraminal");
        res.set("FileDescription", "Panoraminal — 360° panorama painter");
        res.set("CompanyName", "bugzinho");
        res.set("LegalCopyright", "© cleitinhodev / bugzinho — MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows resources: {e}");
        }
    }
}
