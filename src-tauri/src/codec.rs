// TCP framing + libsodium crypto wrappers (mirrors hbb_common tcp.rs/stream.rs),
// exposed as independent read and write halves so the connection can read
// incoming input/pings concurrently with outbound video frames.
use crate::bytes_codec::BytesCodec;
use anyhow::{anyhow, bail, Result};
use bytes::BytesMut;
use protobuf::Message;
use sodiumoxide::crypto::{box_, secretbox};
use std::net::SocketAddr;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

pub type SymmetricKey = secretbox::Key;

#[derive(Clone)]
pub struct Encrypt(pub SymmetricKey, pub u64);

impl Encrypt {
    pub fn new(key: SymmetricKey) -> Self {
        Self(key, 0)
    }
    pub fn enc(&mut self, data: &[u8]) -> Vec<u8> {
        self.1 += 1;
        let nonce = nonce_from_seq(self.1);
        secretbox::seal(data, &nonce, &self.0)
    }
    pub fn dec(&mut self, bytes: &mut BytesMut) -> Result<()> {
        if bytes.len() <= 1 {
            return Ok(());
        }
        self.1 += 1;
        let nonce = nonce_from_seq(self.1);
        match secretbox::open(bytes.as_ref(), &nonce, &self.0) {
            Ok(v) => {
                bytes.clear();
                bytes.extend_from_slice(&v);
                Ok(())
            }
            Err(_) => Err(anyhow!("decrypt failure")),
        }
    }
    pub fn open_session_key(
        symmetric_data: &[u8],
        their_pk_b: &[u8],
        our_sk_b: &box_::SecretKey,
    ) -> Result<SymmetricKey> {
        if their_pk_b.len() != box_::PUBLICKEYBYTES {
            bail!("handshake: pk length {}", their_pk_b.len());
        }
        let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
        let mut pk = [0u8; box_::PUBLICKEYBYTES];
        pk.copy_from_slice(their_pk_b);
        let their_pk = box_::PublicKey(pk);
        let symmetric_key =
            box_::open(symmetric_data, &nonce, &their_pk, our_sk_b).map_err(|_| anyhow!("box open"))?;
        if symmetric_key.len() != secretbox::KEYBYTES {
            bail!("handshake: invalid secret key length");
        }
        let mut key = [0u8; secretbox::KEYBYTES];
        key.copy_from_slice(&symmetric_key);
        Ok(secretbox::Key(key))
    }
}

fn nonce_from_seq(seq: u64) -> secretbox::Nonce {
    let mut nonce = [0u8; secretbox::NONCEBYTES];
    nonce[..std::mem::size_of_val(&seq)].copy_from_slice(&seq.to_le_bytes());
    secretbox::Nonce(nonce)
}

/// Read half of a peer connection.
pub struct Reader {
    framed: Framed<ReadHalf<TcpStream>, BytesCodec>,
    enc: Option<Encrypt>,
}
/// Write half of a peer connection.
pub struct Writer {
    framed: Framed<WriteHalf<TcpStream>, BytesCodec>,
    enc: Option<Encrypt>,
}

pub struct PeerHalves {
    pub reader: Reader,
    pub writer: Writer,
    pub peer_addr: SocketAddr,
}

impl PeerHalves {
    pub fn from_tcp(tcp: TcpStream, peer_addr: SocketAddr) -> Self {
        let (r, w) = tokio::io::split(tcp);
        Self {
            reader: Reader {
                framed: Framed::new(r, BytesCodec::new()),
                enc: None,
            },
            writer: Writer {
                framed: Framed::new(w, BytesCodec::new()),
                enc: None,
            },
            peer_addr,
        }
    }

    pub fn set_key(&mut self, key: SymmetricKey) {
        self.reader.enc = Some(Encrypt::new(key.clone()));
        self.writer.enc = Some(Encrypt::new(key));
    }
}

impl Reader {
    pub fn set_key(&mut self, key: SymmetricKey) {
        self.enc = Some(Encrypt::new(key));
    }
    pub async fn next_msg(&mut self) -> Option<Result<BytesMut, std::io::Error>> {
        use futures::StreamExt;
        let mut res = self.framed.next().await;
        if let Some(Ok(ref mut b)) = res {
            if let Some(k) = self.enc.as_mut() {
                if let Err(e) = k.dec(b) {
                    return Some(Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e.to_string(),
                    )));
                }
            }
        }
        res
    }
}

impl Writer {
    pub fn set_key(&mut self, key: SymmetricKey) {
        self.enc = Some(Encrypt::new(key));
    }
    pub async fn send_msg(&mut self, msg: &impl Message) -> Result<()> {
        let raw = msg.write_to_bytes()?;
        self.send_raw(&raw).await
    }
    pub async fn send_raw(&mut self, raw: &[u8]) -> Result<()> {
        let payload: bytes::Bytes = if let Some(k) = self.enc.as_mut() {
            bytes::Bytes::from(k.enc(raw))
        } else {
            bytes::Bytes::copy_from_slice(raw)
        };
        use futures::SinkExt;
        self.framed.send(payload).await?;
        Ok(())
    }
}
