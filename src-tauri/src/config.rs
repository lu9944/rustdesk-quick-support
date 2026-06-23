// Persistent device config: id, password, Ed25519 signing keypair, uuid, and
// server settings loaded from .env.
use anyhow::Result;
use log::info;
use rand::Rng;
use sodiumoxide::crypto::sign;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

const CHARS: &[char] = &[
    '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k',
    'm', 'n', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

pub const DEFAULT_RENDEZVOUS_SERVERS: &[&str] = &[
    "rs-ny.rustdesk.com",
    "rs-sg.rustdesk.com",
    "rs-cn.rustdesk.com",
];
pub const RENDEZVOUS_PORT: u16 = 21116;
pub const RELAY_PORT: u16 = 21117;

#[derive(Clone)]
pub struct DeviceConfig {
    pub id: String,
    pub password: String,
    pub server: String,
    pub licence_key: String,
    pub socks5: String,
    pub sign_sk: Vec<u8>,
    pub sign_pk: Vec<u8>,
    pub uuid: Vec<u8>,
}

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
        home.join("AppData").join("Roaming").join("RustDeskClient")
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

static CONFIG: OnceLock<DeviceConfig> = OnceLock::new();

pub fn load() -> &'static DeviceConfig {
    CONFIG.get_or_init(|| build_config().expect("failed to build config"))
}

fn build_config() -> Result<DeviceConfig> {
    match dotenv::dotenv() {
        Ok(path) => log::info!(".env loaded from {}", path.display()),
        Err(e) => log::warn!(".env not loaded ({e}); using defaults / saved config"),
    }
    log::info!(
        "raw env: RUSTDESK_SERVER={:?}, RUSTDESK_KEY={} ({} bytes), RUSTDESK_ID={:?}, RUSTDESK_PASSWORD={:?}",
        std::env::var("RUSTDESK_SERVER").ok(),
        if std::env::var("RUSTDESK_KEY").map(|v| !v.is_empty()).unwrap_or(false) {
            "set"
        } else {
            "empty"
        },
        std::env::var("RUSTDESK_KEY").map(|v| v.len()).unwrap_or(0),
        std::env::var("RUSTDESK_ID").ok(),
        std::env::var("RUSTDESK_PASSWORD").ok(),
    );

    // sodiumoxide needs global init for keypair generation.
    sodiumoxide::init().ok();

    let config_path = config_file();
    let mut saved_id = String::new();
    let mut saved_password = String::new();
    let mut saved_sk = String::new();
    let mut saved_pk = String::new();
    let mut saved_uuid = String::new();

    if let Ok(content) = fs::read_to_string(&config_path) {
        for line in content.lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "id" => saved_id = v.trim().to_string(),
                    "password" => saved_password = v.trim().to_string(),
                    "sk" => saved_sk = v.trim().to_string(),
                    "pk" => saved_pk = v.trim().to_string(),
                    "uuid" => saved_uuid = v.trim().to_string(),
                    _ => {}
                }
            }
        }
    }

    let id = env_string("RUSTDESK_ID")
        .or_else(|| if saved_id.is_empty() { None } else { Some(saved_id.clone()) })
        .unwrap_or_else(generate_id);

    let password = env_string("RUSTDESK_PASSWORD")
        .or_else(|| if saved_password.is_empty() { None } else { Some(saved_password.clone()) })
        .unwrap_or_else(generate_password);

    // Ed25519 signing keypair (registered with the rendezvous server as the
    // device public key). Persist so the server sees a stable identity.
    let (sign_sk, sign_pk) = match (hex::decode(&saved_sk), hex::decode(&saved_pk)) {
        (Ok(sk), Ok(pk))
            if sk.len() == sign::SECRETKEYBYTES && pk.len() == sign::PUBLICKEYBYTES =>
        {
            (sk, pk)
        }
        _ => {
            let (pk, sk) = sign::gen_keypair();
            (
                sk.0.to_vec(),
                pk.0.to_vec(),
            )
        }
    };

    let uuid = if saved_uuid.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        saved_uuid.clone()
    };

    // Persist everything for next run.
    let content = format!(
        "id={}\npassword={}\nsk={}\npk={}\nuuid={}\n",
        id,
        password,
        hex::encode(&sign_sk),
        hex::encode(&sign_pk),
        uuid,
    );
    let _ = fs::write(&config_path, content);

    let uuid_bytes = uuid::Uuid::parse_str(&uuid)
        .map(|u| u.as_bytes().to_vec())
        .unwrap_or_default();

    let server = env_string("RUSTDESK_SERVER")
        .unwrap_or_else(|| DEFAULT_RENDEZVOUS_SERVERS[0].to_string());

    let cfg = DeviceConfig {
        id,
        password,
        server,
        licence_key: std::env::var("RUSTDESK_KEY").unwrap_or_default(),
        socks5: std::env::var("RUSTDESK_SOCKS5").unwrap_or_default(),
        sign_sk,
        sign_pk,
        uuid: uuid_bytes,
    };

    info!(
        "Device config: id={}, server={}, pk={}.., uuid={}",
        cfg.id,
        cfg.server,
        &hex::encode(&cfg.sign_pk)[..8],
        uuid,
    );

    Ok(cfg)
}

fn env_string(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn get_id() -> String {
    load().id.clone()
}

pub fn get_password() -> String {
    load().password.clone()
}

/// `host` or `host:port` -> `(host, port)` defaulting to `default_port`.
pub fn split_host_port(addr: &str, default_port: u16) -> (String, u16) {
    if let Some((h, p)) = addr.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.to_string(), port);
        }
    }
    (addr.to_string(), default_port)
}
