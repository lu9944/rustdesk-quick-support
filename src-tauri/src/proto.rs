use rand::Rng;
use sha2::{Digest, Sha256};

const RENDEZVOUS_MSG_TAG_REGISTER_PEER: u64 = 6;
const RENDEZVOUS_MSG_TAG_REGISTER_PEER_RESPONSE: u64 = 7;
const RENDEZVOUS_MSG_TAG_REGISTER_PK: u64 = 12;
const RENDEZVOUS_MSG_TAG_REGISTER_PK_RESPONSE: u64 = 13;
#[allow(dead_code)]
const RENDEZVOUS_MSG_TAG_PUNCH_HOLE: u64 = 8;
#[allow(dead_code)]
const RENDEZVOUS_MSG_TAG_PUNCH_HOLE_SENT: u64 = 9;
#[allow(dead_code)]
const RENDEZVOUS_MSG_TAG_REQUEST_RELAY: u64 = 10;
#[allow(dead_code)]
const RENDEZVOUS_MSG_TAG_RELAY_RESPONSE: u64 = 11;
#[allow(dead_code)]
const RENDEZVOUS_MSG_TAG_FETCH_LOCAL_ADDR: u64 = 15;
#[allow(dead_code)]
const RENDEZVOUS_MSG_TAG_LOCAL_ADDR: u64 = 16;

type WireType = u64;
const VARINT: WireType = 0;
const LEN: WireType = 2;

fn encode_tag(field_number: u64, wire_type: WireType) -> u64 {
    (field_number << 3) | wire_type
}

fn encode_varint(value: u64) -> Vec<u8> {
    let mut result = Vec::new();
    let mut v = value;
    while v >= 0x80 {
        result.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    result.push(v as u8);
    result
}

fn encode_varint_signed(value: i64) -> Vec<u8> {
    encode_varint(value as u64)
}

fn decode_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    for (i, &byte) in buf.iter().enumerate() {
        if i >= 10 {
            return None;
        }
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None
}

fn encode_bytes(tag: u64, data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&encode_varint(tag));
    result.extend_from_slice(&encode_varint(data.len() as u64));
    result.extend_from_slice(data);
    result
}

fn encode_string(tag: u64, s: &str) -> Vec<u8> {
    encode_bytes(tag, s.as_bytes())
}

fn encode_int32(tag: u64, value: i32) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&encode_varint(tag));
    result.extend_from_slice(&encode_varint_signed(value as i64));
    result
}

#[allow(dead_code)]
fn encode_bool(tag: u64, value: bool) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&encode_varint(tag));
    result.push(if value { 1 } else { 0 });
    result
}

#[allow(dead_code)]
fn encode_enum(tag: u64, value: i32) -> Vec<u8> {
    encode_int32(tag, value)
}

#[allow(dead_code)]
fn encode_message(tag: u64, msg: &[u8]) -> Vec<u8> {
    encode_bytes(tag, msg)
}

fn skip_field(buf: &[u8], wire_type: u64) -> Option<usize> {
    match wire_type {
        VARINT => decode_varint(buf).map(|(_, consumed)| consumed),
        LEN => {
            let (len, varint_consumed) = decode_varint(buf)?;
            Some(varint_consumed + len as usize)
        }
        _ => None,
    }
}

fn find_field(buf: &[u8], target_field: u64) -> Option<(u64, &[u8], usize)> {
    let mut offset = 0;
    while offset < buf.len() {
        let (tag, consumed) = decode_varint(&buf[offset..])?;
        offset += consumed;
        let wire_type = tag & 0x7;
        let field_number = tag >> 3;

        let value_start = offset;
        if let Some(skip) = skip_field(&buf[offset..], wire_type) {
            offset += skip;
            let value_bytes = &buf[value_start..offset];

            if field_number == target_field {
                return Some((wire_type, value_bytes, offset));
            }
        } else {
            return None;
        }
    }
    None
}

fn read_varint_from_field(buf: &[u8], wire_type: u64) -> Option<u64> {
    if wire_type == VARINT {
        decode_varint(buf).map(|(v, _)| v)
    } else {
        None
    }
}

fn read_bytes_from_field(buf: &[u8], wire_type: u64) -> Option<&[u8]> {
    if wire_type == LEN {
        let (len, consumed) = decode_varint(buf)?;
        if consumed + len as usize <= buf.len() {
            Some(&buf[consumed..consumed + len as usize])
        } else {
            None
        }
    } else {
        None
    }
}

fn read_bool_from_field(buf: &[u8], wire_type: u64) -> Option<bool> {
    read_varint_from_field(buf, wire_type).map(|v| v != 0)
}

fn read_message_from_field(buf: &[u8], wire_type: u64) -> Option<&[u8]> {
    read_bytes_from_field(buf, wire_type)
}

pub struct KeyPair {
    #[allow(dead_code)]
    pub private_key: [u8; 32],
    pub public_key: [u8; 32],
}

pub fn generate_keypair() -> KeyPair {
    let mut rng = rand::thread_rng();
    let mut private_key = [0u8; 32];
    let mut public_key = [0u8; 32];

    for byte in private_key.iter_mut() {
        *byte = rng.gen();
    }

    let mut hasher = Sha256::new();
    hasher.update(private_key);
    let hash = hasher.finalize();
    public_key.copy_from_slice(&hash[..32]);

    KeyPair {
        private_key,
        public_key,
    }
}

pub fn encode_register_peer(id: &str, serial: i32) -> Vec<u8> {
    let mut inner = Vec::new();
    inner.extend_from_slice(&encode_string(encode_tag(1, LEN), id));
    inner.extend_from_slice(&encode_int32(encode_tag(2, VARINT), serial));

    let mut result = Vec::new();
    result.extend_from_slice(&encode_varint(encode_tag(RENDEZVOUS_MSG_TAG_REGISTER_PEER, LEN)));
    result.extend_from_slice(&encode_varint(inner.len() as u64));
    result.extend_from_slice(&inner);
    result
}

pub fn parse_register_peer_response(buf: &[u8]) -> bool {
    if let Some((wire_type, value, _)) =
        find_field(buf, RENDEZVOUS_MSG_TAG_REGISTER_PEER_RESPONSE)
    {
        if let Some(inner) = read_message_from_field(value, wire_type) {
            if let Some((bool_wire, bool_val, _)) = find_field(inner, 3) {
                return read_bool_from_field(bool_val, bool_wire).unwrap_or(false);
            }
        }
    }
    false
}

pub fn encode_register_pk(id: &str, key_pair: &KeyPair) -> Vec<u8> {
    let mut inner = Vec::new();
    inner.extend_from_slice(&encode_string(encode_tag(1, LEN), id));
    inner.extend_from_slice(&encode_string(encode_tag(2, LEN), &hex::encode(key_pair.public_key)));
    inner.extend_from_slice(&encode_bytes(encode_tag(3, LEN), &key_pair.public_key));

    let mut result = Vec::new();
    result.extend_from_slice(&encode_varint(encode_tag(RENDEZVOUS_MSG_TAG_REGISTER_PK, LEN)));
    result.extend_from_slice(&encode_varint(inner.len() as u64));
    result.extend_from_slice(&inner);
    result
}

pub enum PkResult {
    Ok,
    UuidMismatch,
    Unknown,
}

pub fn parse_register_pk_response(buf: &[u8]) -> PkResult {
    if let Some((wire_type, value, _)) =
        find_field(buf, RENDEZVOUS_MSG_TAG_REGISTER_PK_RESPONSE)
    {
        if let Some(inner) = read_message_from_field(value, wire_type) {
            if let Some((enum_wire, enum_val, _)) = find_field(inner, 1) {
                if let Some(result_code) = read_varint_from_field(enum_val, enum_wire) {
                    return match result_code {
                        0 => PkResult::Ok,
                        2 => PkResult::UuidMismatch,
                        _ => PkResult::Unknown,
                    };
                }
            }
        }
    }
    PkResult::Unknown
}
