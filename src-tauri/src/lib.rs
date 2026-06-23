use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

mod config;
mod proto;

static APP_STATE: Lazy<Mutex<AppState>> = Lazy::new(|| {
    Mutex::new(AppState {
        server_running: false,
        peer_connected: false,
        peer_name: String::new(),
    })
});

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppState {
    server_running: bool,
    peer_connected: bool,
    peer_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConnectionStatus {
    connected: bool,
    peer_name: String,
    id: String,
    password: String,
}

#[tauri::command]
fn get_id() -> String {
    config::get_id()
}

#[tauri::command]
fn get_password() -> String {
    config::get_password()
}

#[tauri::command]
fn get_status() -> ConnectionStatus {
    let state = APP_STATE.lock().unwrap();
    ConnectionStatus {
        connected: state.peer_connected,
        peer_name: state.peer_name.clone(),
        id: config::get_id(),
        password: config::get_password(),
    }
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_handle = app.handle().clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    config::start_server(app_handle).await;
                });
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_id,
            get_password,
            get_status,
            get_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
