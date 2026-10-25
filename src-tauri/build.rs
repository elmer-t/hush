fn main() {
    // Vendored OpenSSL (via SQLCipher) ships objects that reference an `ossl_static.pdb`
    // that never lands in target/, so MSVC prints hundreds of LNK4099 warnings on debug links.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/IGNORE:4099");
    }
    tauri_build::build()
}
