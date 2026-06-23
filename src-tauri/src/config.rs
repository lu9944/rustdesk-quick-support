use dotenv::dotenv;
use log::{error, info};
use rand::Rng;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::proto;

const CHARS: &[char] = &[
    '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k',
    'm', 'n', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

const DEFAULT_RENDEZVOUS_SERVERS: &[&str] = &[
    "rs-ny.rustdesk.com",
    "rs-sg.rustdesk.com",
    "rs-cn.rustdesk.com",
];

const RENDEZVOUS_PORT: u16 = 21116;

static SERVER_RUNNING: AtomicBool = AtomicBool::new(false);

fn config_dir() -> PathBuf {
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join("com.rustdesk.quicksupport")
    }
    #[cfg(target_os = "linux")]
    {
        home.join(".config").join("rustdesk-client")
    }
    #[cfg(target_os = "windows")]
    {
        home.join("AppData")
            .join("Roaming")
            .join("RustDeskClient")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        home.join(".rustdesk-client")
    }
}

fn config_file() -> PathBuf {
    let dir = config_dir();
    fs::create_dir_all(&dir).ok();
    dir.join("config.txt")
}

fn generate_id() -> String {
    if let Ok(Some(ma)) = mac_address::get_mac_address() {
        let bytes = ma.bytes();
        let mut id: u32 = 0;
        for &b in &bytes[2..] {
            id = (id << 8) | (b as u32);
        }
        id &= 0x1FFFFFFF;
        return id.to_string();
    }
    let mut rng = rand::thread_rng();
    rng.gen_range(1_000_000_000u32..2_000_000_000u32).to_string()
}

fn generate_password() -> String {
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| CHARS[rng.gen::<usize>() % CHARS.len()])
        .collect()
}

#[allow(dead_code)]
struct AppConfig {
    id: String,
    password: String,
    rendezvous_server: String,
    key: String,
    socks5_proxy: String,
}

fn load_config() -> AppConfig {
    dotenv().ok();

    let config_path = config_file();
    let mut saved_id = String::new();
    let mut saved_password = String::new();

    if let Ok(content) = fs::read_to_string(&config_path) {
        for line in content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key.trim() {
                    "id" => saved_id = value.trim().to_string(),
                    "password" => saved_password = value.trim().to_string(),
                    _ => {}
                }
            }
        }
    }

    let id = env::var("RUSTDESK_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !saved_id.is_empty() {
                Some(saved_id.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(generate_id);

    let password = env::var("RUSTDESK_PASSWORD")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !saved_password.is_empty() {
                Some(saved_password.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(generate_password);

    if saved_id != id || saved_password != password {
        let content = format!("id={}\npassword={}\n", id, password);
        fs::write(&config_path, content).ok();
    }

    let rendezvous_server = env::var("RUSTDESK_SERVER")
        .unwrap_or_else(|_| DEFAULT_RENDEZVOUS_SERVERS[0].to_string());

    let key = env::var("RUSTDESK_KEY").unwrap_or_default();

    let socks5 = env::var("RUSTDESK_SOCKS5").unwrap_or_default();

    AppConfig {
        id,
        password,
        rendezvous_server,
        key,
        socks5_proxy: socks5,
    }
}

pub fn get_id() -> String {
    load_config().id
}

pub fn get_password() -> String {
    load_config().password
}

pub async fn start_server(_app_handle: tauri::AppHandle) {
    info!("RustDesk QuickSupport server starting...");

    let config = load_config();

    info!("Device ID: {}", config.id);
    info!("Password: {}", config.password);
    info!("Rendezvous Server: {}", config.rendezvous_server);

    let addr = if config.rendezvous_server.contains(':') {
        config.rendezvous_server.clone()
    } else {
        format!("{}:{}", config.rendezvous_server, RENDEZVOUS_PORT)
    };

    SERVER_RUNNING.store(true, Ordering::SeqCst);

    let mut key_pair = proto::generate_keypair();

    loop {
        if !SERVER_RUNNING.load(Ordering::SeqCst) {
            break;
        }

        match connect_and_register(&addr, &config, &mut key_pair).await {
            Ok(_) => {
                info!("Registered with rendezvous server");
            }
            Err(e) => {
                error!("Failed to connect: {}", e);
            }
        }

        tokio::time::sleep(Duration::from_secs(12)).await;
    }
}

async fn connect_and_register(
    addr: &str,
    config: &AppConfig,
    key_pair: &mut proto::KeyPair,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::net::UdpSocket;

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(addr).await?;

    let mut buf = vec![0u8; 2048];

    let register_peer = proto::encode_register_peer(&config.id, 1);
    socket.send(&register_peer).await?;
    info!("RegisterPeer sent to {}", addr);

    match tokio::time::timeout(Duration::from_secs(5), socket.recv(&mut buf)).await {
        Ok(Ok(n)) => {
            info!("Received {} bytes from rendezvous server", n);
            let request_pk = proto::parse_register_peer_response(&buf[..n]);

            if request_pk {
                info!("Server requested PK registration");
                let register_pk = proto::encode_register_pk(&config.id, key_pair);
                socket.send(&register_pk).await?;
                info!("RegisterPk sent");

                let mut pk_buf = vec![0u8; 2048];
                match tokio::time::timeout(Duration::from_secs(5), socket.recv(&mut pk_buf)).await
                {
                    Ok(Ok(n)) => {
                        let result = proto::parse_register_pk_response(&pk_buf[..n]);
                        match result {
                            proto::PkResult::Ok => {
                                info!("PK registration successful");
                            }
                            proto::PkResult::UuidMismatch => {
                                error!("UUID mismatch, regenerating ID...");
                                *key_pair = proto::generate_keypair();
                            }
                            proto::PkResult::Unknown => {
                                info!("PK registration response received (unknown status)");
                            }
                        }
                    }
                    _ => {
                        info!("No PK response from server");
                    }
                }
            }
        }
        Ok(Err(e)) => {
            error!("Receive error: {}", e);
        }
        Err(_) => {
            info!("No response from rendezvous server (timeout)");
        }
    }

    Ok(())
}
