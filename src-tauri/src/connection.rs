// Connection handler: secure handshake, login, then a reader loop (input +
// control) that runs concurrently with a writer task (video + responses), so
// slow video writes can never starve incoming input or TestDelay pings.
use crate::codec::{PeerHalves, Encrypt};
use crate::config::DeviceConfig;
use crate::input;
use crate::proto_gen::message::{
    self, message::Union as mu, EncodedVideoFrame, EncodedVideoFrames, Hash, IdPk, KeyEvent,
    LoginResponse, Message, MouseEvent, PeerInfo, SignedId, SupportedEncoding, TestDelay, VideoFrame,
};
use crate::proto_gen::rendezvous::{RequestRelay, RendezvousMessage};
use crate::video;
use anyhow::{bail, Result};
use log::{info, warn};
use protobuf::Message as ProtoMessage;
use rand::Rng;
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::{box_, sign};
use std::net::SocketAddr;
use std::sync::mpsc as std_mpsc;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

enum InputMsg {
    Mouse(MouseEvent),
    Key(KeyEvent),
}

pub async fn serve_relay(tcp: TcpStream, cfg: DeviceConfig, uuid: String, peer_addr: SocketAddr) -> Result<()> {
    info!("new relay connection from {peer_addr}");
    let PeerHalves {
        mut reader,
        mut writer,
        peer_addr: _,
    } = PeerHalves::from_tcp(tcp, peer_addr);

    // First framed message on the relay connection: RequestRelay.
    let mut req = RequestRelay::new();
    req.licence_key = cfg.licence_key.clone();
    req.uuid = uuid.clone();
    let mut m = RendezvousMessage::new();
    m.set_request_relay(req);
    writer.send_msg(&m).await?;

    // --- Secure handshake ---
    let (our_pk_b, our_sk_b) = box_::gen_keypair();
    let mut idpk = IdPk::new();
    idpk.id = cfg.id.clone();
    idpk.pk = bytes::Bytes::copy_from_slice(&our_pk_b.0);
    let idpk_bytes = idpk.write_to_bytes()?;

    let mut sign_sk_arr = [0u8; sign::SECRETKEYBYTES];
    sign_sk_arr.copy_from_slice(&cfg.sign_sk);
    let sign_sk = sign::SecretKey(sign_sk_arr);
    let signed = sign::sign(&idpk_bytes, &sign_sk);

    let mut sid = SignedId::new();
    sid.id = bytes::Bytes::from(signed);
    let mut msg = Message::new();
    msg.set_signed_id(sid);
    writer.send_msg(&msg).await?;

    let bytes = match reader.next_msg().await {
        Some(Ok(b)) if !b.is_empty() => b,
        Some(Ok(_)) => bail!("handshake: empty frame"),
        Some(Err(e)) => bail!("handshake read error: {e}"),
        None => bail!("handshake: peer closed before PublicKey"),
    };
    let msg_in = Message::parse_from_bytes(&bytes)?;
    let pk = match msg_in.union {
        Some(mu::PublicKey(pk)) => pk,
        _ => bail!("expected PublicKey"),
    };
    let session_key = Encrypt::open_session_key(&pk.symmetric_value, &pk.asymmetric_value, &our_sk_b)?;
    writer.set_key(session_key.clone());
    reader.set_key(session_key);
    info!("secure stream established with {peer_addr}");

    // --- Send Hash (salt + challenge) ---
    let salt = random_hex(16);
    let challenge = random_alnum(6);
    let mut hash = Hash::new();
    hash.salt = salt.clone();
    hash.challenge = challenge.clone();
    let mut hm = Message::new();
    hm.set_hash(hash);
    writer.send_msg(&hm).await?;

    // --- Outgoing channel + writer task ---
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let mut writer = writer;
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if let Err(e) = writer.send_msg(&msg).await {
                warn!("writer send failed: {e}");
                break;
            }
        }
    });

    // --- Input thread (enigo calls are blocking; keep them off the async loop) ---
    let (in_tx, in_rx) = std_mpsc::sync_channel::<InputMsg>(128);
    std::thread::spawn(move || {
        for m in in_rx {
            match m {
                InputMsg::Mouse(e) => input::handle_mouse(&e),
                InputMsg::Key(e) => input::handle_key(&e),
            }
        }
    });

    // --- Video subscriber ---
    let (vtx, mut vrx) = mpsc::channel::<video::EncodedFrame>(2);
    let mut video_started = false;

    // --- Reader loop: incoming messages + video frames ---
    loop {
        tokio::select! {
            biased; // prioritize incoming input/pings over video
            r = reader.next_msg() => {
                match r {
                    Some(Ok(b)) => {
                        let m = match Message::parse_from_bytes(&b) {
                            Ok(m) => m,
                            Err(e) => { warn!("parse msg: {e}"); continue; }
                        };
                        match m.union {
                            Some(mu::LoginRequest(lr)) => {
                                if verify_login(&cfg, &salt, &challenge, &lr.password) {
                                    info!("controller logged in: {}", lr.my_name);
                                    crate::set_peer_connected(true, Some(lr.my_name.clone()));
                                    let _ = out_tx.send(login_ok_msg(&cfg));
                                    if !video_started {
                                        video_started = true;
                                        video::set_subscriber(vtx.clone());
                                    }
                                } else if lr.password.is_empty() {
                                    info!("empty password -> prompting controller");
                                    let _ = out_tx.send(login_err_msg("Empty Password"));
                                } else {
                                    warn!("login rejected (bad password)");
                                    let _ = out_tx.send(login_err_msg("Wrong Password"));
                                }
                            }
                            Some(mu::MouseEvent(me)) => { let _ = in_tx.send(InputMsg::Mouse(me)); }
                            Some(mu::KeyEvent(ke)) => { let _ = in_tx.send(InputMsg::Key(ke)); }
                            Some(mu::TestDelay(td)) => {
                                let mut out = TestDelay::new();
                                out.from_client = false;
                                out.last_delay = td.last_delay;
                                let mut o = Message::new();
                                o.set_test_delay(out);
                                let _ = out_tx.send(o);
                            }
                            Some(mu::Misc(misc)) => {
                                if misc.has_close_reason() {
                                    info!("controller closed connection");
                                    break;
                                }
                            }
                            Some(other) => {
                                warn!("unhandled message: {:?}", other);
                            }
                            None => {}
                        }
                    }
                    Some(Err(e)) => { warn!("stream read error: {e}"); break; }
                    None => { info!("controller disconnected"); break; }
                }
            }
            f = vrx.recv() => {
                if let Some(frame) = f {
                    let _ = out_tx.send(video_frame_msg(frame));
                }
            }
        }
    }

    crate::set_peer_connected(false, None);
    video::clear_subscriber();
    writer_task.abort();
    Ok(())
}

fn verify_login(cfg: &DeviceConfig, salt: &str, challenge: &str, got: &[u8]) -> bool {
    if got.is_empty() {
        return false;
    }
    let mut h1 = Sha256::new();
    h1.update(cfg.password.as_bytes());
    h1.update(salt.as_bytes());
    let h1 = h1.finalize();
    let mut h2 = Sha256::new();
    h2.update(h1);
    h2.update(challenge.as_bytes());
    let h2 = h2.finalize();
    constant_eq(&h2[..], got)
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn login_ok_msg(cfg: &DeviceConfig) -> Message {
    let mut pi = PeerInfo::new();
    pi.username = whoami::username();
    pi.hostname = whoami::devicename();
    pi.platform = platform_str().to_string();
    pi.current_display = 0;
    pi.sas_enabled = cfg!(target_os = "windows");
    pi.version = env!("CARGO_PKG_VERSION").to_string();
    if let Ok(monitors) = xcap::Monitor::all() {
        if let Some(m) = monitors.into_iter().next() {
            let mut di = message::DisplayInfo::new();
            di.x = 0;
            di.y = 0;
            di.width = m.width().unwrap_or(0) as i32;
            di.height = m.height().unwrap_or(0) as i32;
            di.name = m.name().unwrap_or_default();
            di.online = true;
            pi.displays.push(di);
        }
    }
    let mut enc = SupportedEncoding::new();
    enc.h264 = true;
    pi.encoding = Some(enc).into();
    let mut res = LoginResponse::new();
    res.set_peer_info(pi);
    let mut m = Message::new();
    m.set_login_response(res);
    m
}

fn login_err_msg(err: &str) -> Message {
    let mut res = LoginResponse::new();
    res.set_error(err.to_string());
    let mut m = Message::new();
    m.set_login_response(res);
    m
}

fn video_frame_msg(frame: video::EncodedFrame) -> Message {
    let mut evf = EncodedVideoFrame::new();
    evf.data = bytes::Bytes::from(frame.data);
    evf.key = frame.key;
    evf.pts = frame.pts;
    let mut evfs = EncodedVideoFrames::new();
    evfs.frames.push(evf);
    let mut vf = VideoFrame::new();
    vf.set_h264s(evfs);
    vf.display = 0;
    let mut m = Message::new();
    m.set_video_frame(vf);
    m
}

fn platform_str() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Windows"
    }
    #[cfg(target_os = "linux")]
    {
        "Linux"
    }
    #[cfg(target_os = "macos")]
    {
        "Mac OS"
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        "Unknown"
    }
}

fn random_hex(n: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..n).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
}

fn random_alnum(n: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| {
            let b = rng.gen_range(0..36);
            if b < 10 {
                (b'0' + b) as char
            } else {
                (b'a' + b - 10) as char
            }
        })
        .collect()
}
