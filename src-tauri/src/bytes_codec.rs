// Length-delimited codec vendored from RustDesk's hbb_common (bytes_codec.rs).
// Wire format: variable-length header (1..=4 bytes). Low 2 bits of the first
// byte encode the header width; the remaining bits (big across the header
// bytes, little-endian) shifted right by 2 give the payload length.
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

const MAX_PREALLOCATED_PAYLOAD_LEN: usize = 256 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct BytesCodec {
    state: DecodeState,
    raw: bool,
    max_packet_length: usize,
}

#[derive(Debug, Clone, Copy)]
enum DecodeState {
    Head,
    Data(usize),
}

impl Default for BytesCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl BytesCodec {
    pub fn new() -> Self {
        Self {
            state: DecodeState::Head,
            raw: false,
            max_packet_length: usize::MAX,
        }
    }

    pub fn set_raw(&mut self) {
        self.raw = true;
    }

    fn decode_head(&mut self, src: &mut BytesMut) -> io::Result<Option<usize>> {
        if src.is_empty() {
            return Ok(None);
        }
        let head_len = ((src[0] & 0x3) + 1) as usize;
        if src.len() < head_len {
            return Ok(None);
        }
        let mut n = src[0] as usize;
        if head_len > 1 {
            n |= (src[1] as usize) << 8;
        }
        if head_len > 2 {
            n |= (src[2] as usize) << 16;
        }
        if head_len > 3 {
            n |= (src[3] as usize) << 24;
        }
        n >>= 2;
        if n > self.max_packet_length {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Too big packet"));
        }
        src.advance(head_len);
        src.reserve(n.saturating_sub(src.len()).min(MAX_PREALLOCATED_PAYLOAD_LEN));
        Ok(Some(n))
    }

    fn decode_data(&self, n: usize, src: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        if src.len() < n {
            return Ok(None);
        }
        Ok(Some(src.split_to(n)))
    }
}

impl Decoder for BytesCodec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
        if self.raw {
            if !src.is_empty() {
                let len = src.len();
                return Ok(Some(src.split_to(len)));
            } else {
                return Ok(None);
            }
        }
        let n = match self.state {
            DecodeState::Head => match self.decode_head(src)? {
                Some(n) => {
                    self.state = DecodeState::Data(n);
                    n
                }
                None => return Ok(None),
            },
            DecodeState::Data(n) => n,
        };
        match self.decode_data(n, src)? {
            Some(data) => {
                self.state = DecodeState::Head;
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }
}

impl Encoder<Bytes> for BytesCodec {
    type Error = io::Error;

    fn encode(&mut self, data: Bytes, buf: &mut BytesMut) -> Result<(), io::Error> {
        if self.raw {
            buf.reserve(data.len());
            buf.put(data);
            return Ok(());
        }
        if data.len() <= 0x3F {
            buf.put_u8((data.len() << 2) as u8);
        } else if data.len() <= 0x3FFF {
            buf.put_u16_le((data.len() << 2) as u16 | 0x1);
        } else if data.len() <= 0x3FFFFF {
            let h = (data.len() << 2) as u32 | 0x2;
            buf.put_u16_le((h & 0xFFFF) as u16);
            buf.put_u8((h >> 16) as u8);
        } else if data.len() <= 0x3FFFFFFF {
            buf.put_u32_le((data.len() << 2) as u32 | 0x3);
        } else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Overflow"));
        }
        buf.extend(data);
        Ok(())
    }
}

/// Encode a single payload using the codec framing rules (no tokio needed).
pub fn encode_frame(payload: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    BytesCodec::new()
        .encode(Bytes::copy_from_slice(payload), &mut buf)
        .ok();
    buf
}

/// Decode a single complete frame from a buffer that starts with a header and
/// contains the full payload. Returns None if the buffer is incomplete.
pub fn decode_one(buf: &[u8]) -> Option<(BytesMut, usize)> {
    if buf.is_empty() {
        return None;
    }
    let head_len = ((buf[0] & 0x3) + 1) as usize;
    if buf.len() < head_len {
        return None;
    }
    let mut n = buf[0] as usize;
    if head_len > 1 {
        n |= (buf[1] as usize) << 8;
    }
    if head_len > 2 {
        n |= (buf[2] as usize) << 16;
    }
    if head_len > 3 {
        n |= (buf[3] as usize) << 24;
    }
    n >>= 2;
    let total = head_len + n;
    if buf.len() < total {
        return None;
    }
    Some((BytesMut::from(&buf[head_len..total]), total))
}
