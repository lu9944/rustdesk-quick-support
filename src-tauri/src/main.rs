#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(target_os = "windows", target_env = "gnu"))]
mod dll {
    const DLL_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/WebView2Loader.dll"));

    pub fn ensure_webview2_loader() {
        let exe_path =
            std::env::current_exe().expect("Failed to get exe path");
        let exe_dir = exe_path.parent().expect("Failed to get exe dir");
        let dll_path = exe_dir.join("WebView2Loader.dll");

        if !dll_path.exists() {
            std::fs::write(&dll_path, DLL_BYTES)
                .expect("Failed to extract WebView2Loader.dll");
        }
    }
}

#[cfg(not(all(target_os = "windows", target_env = "gnu")))]
mod dll {
    pub fn ensure_webview2_loader() {}
}

fn main() {
    dll::ensure_webview2_loader();
    rustdesk_client::run()
}
