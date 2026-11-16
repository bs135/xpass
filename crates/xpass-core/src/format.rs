//! Binary envelope, format v1 (all integers little-endian).
//!
//! ```text
//! header:
//!   magic         4   "XPSV"
//!   version       u16 = 1
//!   vault_id      16  random, stable for the vault's life
//!   cipher        u8  = 1 (XChaCha20-Poly1305)
//!   slot_count    u8  (1..=8)
//!   slots         slot_count * slot
//!   payload_nonce 24
//! body:
//!   ciphertext    rest of file (payload JSON + 16-byte tag), AAD = header bytes
//!
//! slot:
//!   kind          u8  1 = password, 2 = recovery, 3 = hello
//!   params_len    u16
//!   params        password: m_kib u32, t u32, p u32, salt 16
//!                 recovery: salt 16
//!                 hello:    salt 32
//!   nonce         24
//!   wrapped_len   u16 (= 48: VaultKey 32 + tag 16)
//!   wrapped       VaultKey encrypted with the slot KEK, AAD = slot_aad()
//! ```

use crate::{
    crypto::{KdfParams, KEY_LEN, NONCE_LEN, TAG_LEN},
    Error, Result,
};

pub const MAGIC: &[u8; 4] = b"XPSV";
pub const VERSION: u16 = 1;
pub const CIPHER_XCHACHA20_POLY1305: u8 = 1;
pub const MAX_SLOTS: usize = 8;
pub const WRAPPED_LEN: usize = KEY_LEN + TAG_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Password = 1,
    Recovery = 2,
    Hello = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotParams {
    Password { kdf: KdfParams, salt: [u8; 16] },
    Recovery { salt: [u8; 16] },
    Hello { salt: [u8; 32] },
}

impl SlotParams {
    pub fn kind(&self) -> SlotKind {
        match self {
            SlotParams::Password { .. } => SlotKind::Password,
            SlotParams::Recovery { .. } => SlotKind::Recovery,
            SlotParams::Hello { .. } => SlotKind::Hello,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            SlotParams::Password { kdf, salt } => {
                out.extend_from_slice(&kdf.m_kib.to_le_bytes());
                out.extend_from_slice(&kdf.t.to_le_bytes());
                out.extend_from_slice(&kdf.p.to_le_bytes());
                out.extend_from_slice(salt);
            }
            SlotParams::Recovery { salt } => out.extend_from_slice(salt),
            SlotParams::Hello { salt } => out.extend_from_slice(salt),
        }
        out
    }

    fn decode(kind: u8, bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let params = match kind {
            1 => SlotParams::Password {
                kdf: KdfParams {
                    m_kib: r.u32()?,
                    t: r.u32()?,
                    p: r.u32()?,
                },
                salt: r.array()?,
            },
            2 => SlotParams::Recovery { salt: r.array()? },
            3 => SlotParams::Hello { salt: r.array()? },
            _ => return Err(Error::Format("unknown slot kind")),
        };
        r.finish("trailing bytes in slot params")?;
        Ok(params)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub params: SlotParams,
    pub nonce: [u8; NONCE_LEN],
    pub wrapped: [u8; WRAPPED_LEN],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub vault_id: [u8; 16],
    pub slots: Vec<Slot>,
    pub payload_nonce: [u8; NONCE_LEN],
}

impl Header {
    pub fn encode(&self) -> Vec<u8> {
        assert!(!self.slots.is_empty() && self.slots.len() <= MAX_SLOTS);
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.vault_id);
        out.push(CIPHER_XCHACHA20_POLY1305);
        out.push(self.slots.len() as u8);
        for slot in &self.slots {
            let params = slot.params.encode();
            out.push(slot.params.kind() as u8);
            out.extend_from_slice(&(params.len() as u16).to_le_bytes());
            out.extend_from_slice(&params);
            out.extend_from_slice(&slot.nonce);
            out.extend_from_slice(&(WRAPPED_LEN as u16).to_le_bytes());
            out.extend_from_slice(&slot.wrapped);
        }
        out.extend_from_slice(&self.payload_nonce);
        out
    }

    /// AAD for wrapping the VaultKey in a slot: binds the slot to this vault,
    /// this format version and its own parameters (salt, KDF cost).
    pub fn slot_aad(vault_id: &[u8; 16], params: &SlotParams) -> Vec<u8> {
        let mut aad = Vec::new();
        aad.extend_from_slice(MAGIC);
        aad.extend_from_slice(&VERSION.to_le_bytes());
        aad.extend_from_slice(vault_id);
        aad.push(params.kind() as u8);
        aad.extend_from_slice(&params.encode());
        aad
    }
}

/// Parsed file: header, the exact header bytes (AAD) and the ciphertext.
pub struct VaultFile<'a> {
    pub header: Header,
    pub header_bytes: &'a [u8],
    pub ciphertext: &'a [u8],
}

pub fn parse(bytes: &[u8]) -> Result<VaultFile<'_>> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != MAGIC {
        return Err(Error::Format("bad magic"));
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    let vault_id = r.array()?;
    if r.u8()? != CIPHER_XCHACHA20_POLY1305 {
        return Err(Error::Format("unknown cipher"));
    }
    let slot_count = r.u8()? as usize;
    if slot_count == 0 || slot_count > MAX_SLOTS {
        return Err(Error::Format("bad slot count"));
    }
    let mut slots = Vec::with_capacity(slot_count);
    for _ in 0..slot_count {
        let kind = r.u8()?;
        let params_len = r.u16()? as usize;
        let params = SlotParams::decode(kind, r.take(params_len)?)?;
        let nonce = r.array()?;
        if r.u16()? as usize != WRAPPED_LEN {
            return Err(Error::Format("bad wrapped key length"));
        }
        let wrapped = r.array()?;
        if slots.iter().any(|s: &Slot| s.params.kind() == params.kind()) {
            return Err(Error::Format("duplicate slot kind"));
        }
        slots.push(Slot { params, nonce, wrapped });
    }
    let payload_nonce = r.array()?;
    let header_len = r.pos;
    let ciphertext = &bytes[header_len..];
    if ciphertext.len() < TAG_LEN {
        return Err(Error::Format("ciphertext too short"));
    }
    Ok(VaultFile {
        header: Header {
            vault_id,
            slots,
            payload_nonce,
        },
        header_bytes: &bytes[..header_len],
        ciphertext,
    })
}

/// Bounds-checked cursor; every read returns `Error::Format` instead of panicking.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or(Error::Format("truncated"))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take returned N bytes"))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn finish(&self, msg: &'static str) -> Result<()> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(Error::Format(msg))
        }
    }
}
