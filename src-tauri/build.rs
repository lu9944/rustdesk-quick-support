use std::env;
use std::path::Path;

fn main() {
    tauri_build::build();

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "gnu" {
        return;
    }

    let profile = env::var("PROFILE").unwrap();
    let target = env::var("TARGET").unwrap();
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    let workspace = manifest_dir.parent().unwrap_or(manifest_dir);
    let dll_src = workspace.join("target").join(&target).join(&profile).join("WebView2Loader.dll");

    if dll_src.exists() {
        let out_dir = env::var("OUT_DIR").unwrap();
        let dll_dst = Path::new(&out_dir).join("WebView2Loader.dll");
        std::fs::copy(&dll_src, &dll_dst).expect("Failed to copy WebView2Loader.dll to OUT_DIR");
        println!("cargo:rerun-if-changed={}", dll_src.display());
    }
}
