fn main() {
    println!("cargo:rerun-if-changed=assets/sentineltask.ico");
    println!("cargo:rerun-if-changed=assets/sentineltask.manifest");
    #[cfg(windows)]
    windows_resources();
    // Cross-compiling from another OS: build.rs runs on the host, so check
    // the *target* instead of cfg(windows).
    #[cfg(not(windows))]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cross_resources();
    }
}

#[cfg(windows)]
fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/sentineltask.ico")
        .set_manifest_file("assets/sentineltask.manifest")
        .set("FileDescription", "SentinelTask macro recorder")
        .set("ProductName", "SentinelTask")
        .set("CompanyName", "SentinelTask contributors")
        .set("LegalCopyright", "MIT License");
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed Windows resources: {e}");
    }
}

#[cfg(not(windows))]
fn cross_resources() {
    use std::path::PathBuf;
    use std::process::Command;
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let rc = out.join("resources.rc");
    let icon = manifest_dir.join("assets/sentineltask.ico");
    let manifest = manifest_dir.join("assets/sentineltask.manifest");
    std::fs::write(
        &rc,
        format!(
            "1 ICON \"{}\"\n1 24 \"{}\"\n",
            icon.display().to_string().replace('\\', "/"),
            manifest.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let obj = out.join("resources.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
    match Command::new(&windres)
        .arg(&rc)
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&obj)
        .status()
    {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bins={}", obj.display());
        }
        _ => println!("cargo:warning=windres not found; building without icon/manifest"),
    }
}
