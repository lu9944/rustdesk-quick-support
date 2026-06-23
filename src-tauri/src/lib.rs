use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

mod bytes_codec;
mod codec;
mod config;
mod connection;
mod input;
mod proto_gen;
mod rendezvous;
mod video;

static APP_STATE: Lazy<Mutex<AppState>> = Lazy::new(|| {
    Mutex::new(AppState {
        server_online: false,
        peer_connected: false,
        peer_name: String::new(),
    })
});

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppState {
    server_online: bool,
    peer_connected: bool,
    peer_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConnectionStatus {
    server_online: bool,
    connected: bool,
    peer_name: String,
    server: String,
    id: String,
    password: String,
}

pub fn set_server_online(online: bool) {
    if let Ok(mut s) = APP_STATE.lock() {
        s.server_online = online;
    }
}

pub fn set_peer_connected(connected: bool, name: Option<String>) {
    if let Ok(mut s) = APP_STATE.lock() {
        s.peer_connected = connected;
        s.peer_name = name.unwrap_or_default();
    }
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
    let cfg = config::load();
    ConnectionStatus {
        server_online: state.server_online,
        connected: state.peer_connected,
        peer_name: state.peer_name.clone(),
        server: cfg.server.clone(),
        id: config::get_id(),
        password: config::get_password(),
    }
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn run() {
    // Default to INFO logging even when RUST_LOG is not set, so `cargo run`
    // shows the rendezvous connection progress in the terminal.
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let _app_handle = app.handle().clone();
            video::start_global();
            std::thread::spawn(|| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(rendezvous::run());
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
