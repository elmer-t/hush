fn main() {
    // Vendored OpenSSL (via SQLCipher) ships objects that reference an `ossl_static.pdb`
    // that never lands in target/, so MSVC prints hundreds of LNK4099 warnings on debug links.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/IGNORE:4099");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        stage_directml();
    }
    tauri_build::build()
}

/// ONNX Runtime's DirectML build loads DirectML.dll, and Windows' own copy in System32
/// can be too old to run the model (#105), so the installer ships the one `ort` fetched.
/// `ort`'s build script (`copy-dylibs`) put it in this build's profile folder; it is
/// copied to `directml/`, a fixed path tauri.windows.conf.json bundles next to hush.exe.
fn stage_directml() {
    use std::path::PathBuf;
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // OUT_DIR is <profile dir>/build/<crate>-<hash>/out, or with newer Cargo
    // <profile dir>/build/<crate>/<hash>/out: take the folder that holds `build`.
    let Some(profile) = out
        .ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "build"))
        .and_then(|build| build.parent())
    else {
        return;
    };
    let from = profile.join("DirectML.dll");
    let to = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("directml");
    println!("cargo:rerun-if-changed={}", from.display());
    if let Err(e) =
        std::fs::create_dir_all(&to).and_then(|()| std::fs::copy(&from, to.join("DirectML.dll")))
    {
        println!(
            "cargo:warning=could not stage {} for the installer: {e}",
            from.display()
        );
    }
}
