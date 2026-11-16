//! High-level vault operations on top of the binary format.
//!
//! Key hierarchy (docs/IDEA.md §3.2):
//! ```text
//! password  --Argon2id(salt, kdf)-------------> KEK_pw    -+
//! recovery  --HKDF(salt, "xpass/kek-recovery")-> KEK_rec  -+- wrap -> VaultKey -> payload
//! Hello sig --HKDF(salt, "xpass/kek-hello")----> KEK_hello -+
//! ```

use rand::{CryptoRng, RngCore};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    crypto::{self, KdfParams, Key, KEY_LEN},
    format::{self, Header, Slot, SlotKind, SlotParams, WRAPPED_LEN},
    model::Payload,
    recovery, Error, Result,
};

const INFO_RECOVERY: &[u8] = b"xpass/kek-recovery/v1";
const INFO_HELLO: &[u8] = b"xpass/kek-hello/v1";
const HELLO_CHALLENGE_PREFIX: &[u8] = b"xpass-hello-v1";

/// How to unlock a vault file.
pub enum Unlock<'a> {
    Password(&'a [u8]),
    /// Printed recovery key, as typed by the user.
    Recovery(&'a str),
    /// Windows Hello signature over [`hello_challenge`] (see spikes/windows-hello).
    Hello {
        signature: &'a [u8],
    },
}

/// Vault in memory after unlock. Holds the VaultKey; dropping it wipes the key
/// and the payload secrets.
#[derive(Clone)]
pub struct UnlockedVault {
    vault_id: [u8; 16],
    vault_key: Key,
    slots: Vec<Slot>,
    pub payload: Payload,
}

impl std::fmt::Debug for UnlockedVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockedVault")
            .field("vault_id", &Uuid::from_bytes(self.vault_id))
            .field("slots", &self.slot_kinds())
            .field("entries", &self.payload.entries.len())
            .finish_non_exhaustive()
    }
}

impl UnlockedVault {
    /// Creates a new vault with a password slot and a recovery slot.
    /// Returns the printable recovery key: show it once (Emergency Kit), never store it.
    pub fn create(
        rng: &mut (impl RngCore + CryptoRng),
        password: &[u8],
        kdf: KdfParams,
    ) -> Result<(Self, Zeroizing<String>)> {
        kdf.validate()?;
        let mut vault = Self {
            vault_id: crypto::random_bytes(rng),
            vault_key: crypto::random_key(rng),
            slots: Vec::new(),
            payload: Payload::default(),
        };
        vault.set_password_slot(rng, password, kdf)?;
        let recovery_text = vault.new_recovery_slot(rng);
        Ok((vault, recovery_text))
    }

    pub fn unlock(file: &[u8], how: Unlock<'_>) -> Result<Self> {
        let parsed = format::parse(file)?;
        let header = &parsed.header;
        let kind = match how {
            Unlock::Password(_) => SlotKind::Password,
            Unlock::Recovery(_) => SlotKind::Recovery,
            Unlock::Hello { .. } => SlotKind::Hello,
        };
        let slot = header
            .slots
            .iter()
            .find(|s| s.params.kind() == kind)
            .ok_or(Error::NoSuchSlot)?;

        let kek = match (&how, &slot.params) {
            (Unlock::Password(pw), SlotParams::Password { kdf, salt }) => {
                kdf.validate()?;
                crypto::kek_from_password(pw, salt, *kdf)?
            }
            (Unlock::Recovery(text), SlotParams::Recovery { salt }) => {
                crypto::kek_from_secret(recovery::decode(text)?.as_ref(), salt, INFO_RECOVERY)
            }
            (Unlock::Hello { signature }, SlotParams::Hello { salt }) => {
                crypto::kek_from_secret(signature, salt, INFO_HELLO)
            }
            _ => unreachable!("slot kind matched above"),
        };

        let aad = Header::slot_aad(&header.vault_id, &slot.params);
        let vk = crypto::open(&kek, &slot.nonce, &slot.wrapped, &aad).ok_or(Error::BadCredentials)?;
        let mut vault_key = Zeroizing::new([0u8; KEY_LEN]);
        vault_key.copy_from_slice(&vk);

        let plaintext = crypto::open(
            &vault_key,
            &header.payload_nonce,
            parsed.ciphertext,
            parsed.header_bytes,
        )
        .ok_or(Error::Tampered)?;
        let payload: Payload = serde_json::from_slice(&plaintext).map_err(|e| Error::Payload(e.to_string()))?;

        Ok(Self {
            vault_id: header.vault_id,
            vault_key,
            slots: header.slots.clone(),
            payload,
        })
    }

    /// Decrypts another copy of the same vault (e.g. the remote file during sync)
    /// with the VaultKey already in memory, without asking for credentials.
    ///
    /// The result carries the *other* file's key slots (a password change on
    /// another device is adopted) and our VaultKey.
    /// - `DifferentVault`: vault_id differs.
    /// - `KeyChanged`: slots differ and the key does not fit (rotated elsewhere).
    /// - `Tampered`: slots are identical but the payload does not decrypt.
    pub fn open_sibling(&self, file: &[u8]) -> Result<Self> {
        let parsed = format::parse(file)?;
        let header = &parsed.header;
        if header.vault_id != self.vault_id {
            return Err(Error::DifferentVault);
        }
        let plaintext = match crypto::open(
            &self.vault_key,
            &header.payload_nonce,
            parsed.ciphertext,
            parsed.header_bytes,
        ) {
            Some(p) => p,
            None if header.slots == self.slots => return Err(Error::Tampered),
            None => return Err(Error::KeyChanged),
        };
        let payload: Payload = serde_json::from_slice(&plaintext).map_err(|e| Error::Payload(e.to_string()))?;
        Ok(Self {
            vault_id: self.vault_id,
            vault_key: self.vault_key.clone(),
            slots: header.slots.clone(),
            payload,
        })
    }

    /// Serializes and encrypts the vault. Bumps `payload.revision` and uses a
    /// fresh payload nonce on every call.
    pub fn save(&mut self, rng: &mut (impl RngCore + CryptoRng)) -> Result<Vec<u8>> {
        self.payload.revision += 1;
        let header = Header {
            vault_id: self.vault_id,
            slots: self.slots.clone(),
            payload_nonce: crypto::random_bytes(rng),
        };
        let header_bytes = header.encode();
        let plaintext = Zeroizing::new(serde_json::to_vec(&self.payload).map_err(|e| Error::Payload(e.to_string()))?);
        let ciphertext = crypto::seal(&self.vault_key, &header.payload_nonce, &plaintext, &header_bytes);
        let mut out = header_bytes;
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Re-wraps the *same* VaultKey under a new password.
    ///
    /// Not a compromise response: old files (e.g. git history) still unlock with
    /// the old password and expose the same VaultKey. Use [`Self::rotate`] for that.
    pub fn change_password(
        &mut self,
        rng: &mut (impl RngCore + CryptoRng),
        new_password: &[u8],
        kdf: KdfParams,
    ) -> Result<()> {
        kdf.validate()?;
        self.set_password_slot(rng, new_password, kdf)
    }

    /// Compromise response (docs/IDEA.md §3.3): new VaultKey, new password slot,
    /// new recovery key. The Hello slot is dropped and must be re-enrolled,
    /// because wrapping needs a fresh Hello signature.
    /// Returns the new printable recovery key.
    pub fn rotate(
        &mut self,
        rng: &mut (impl RngCore + CryptoRng),
        password: &[u8],
        kdf: KdfParams,
    ) -> Result<Zeroizing<String>> {
        kdf.validate()?;
        self.vault_key = crypto::random_key(rng);
        self.slots.clear();
        self.set_password_slot(rng, password, kdf)?;
        Ok(self.new_recovery_slot(rng))
    }

    /// Hello enrollment step 1: pick a salt, then ask Hello to sign
    /// `hello_challenge(vault_id, salt)`.
    pub fn new_hello_salt(rng: &mut (impl RngCore + CryptoRng)) -> [u8; 32] {
        crypto::random_bytes(rng)
    }

    /// Hello enrollment step 2: wrap the VaultKey with the KEK derived from the signature.
    pub fn set_hello_slot(&mut self, rng: &mut (impl RngCore + CryptoRng), salt: [u8; 32], signature: &[u8]) {
        let kek = crypto::kek_from_secret(signature, &salt, INFO_HELLO);
        self.put_slot(rng, SlotParams::Hello { salt }, &kek);
    }

    pub fn remove_hello_slot(&mut self) {
        self.slots.retain(|s| s.params.kind() != SlotKind::Hello);
    }

    pub fn vault_id(&self) -> Uuid {
        Uuid::from_bytes(self.vault_id)
    }

    /// Truncated SHA-256 of the VaultKey, for tests and diagnostics only.
    #[doc(hidden)]
    pub fn vault_key_fingerprint(&self) -> [u8; 8] {
        use sha2::{Digest, Sha256};
        Sha256::digest(self.vault_key.as_ref())[..8]
            .try_into()
            .expect("8 bytes")
    }

    pub fn slot_kinds(&self) -> Vec<SlotKind> {
        self.slots.iter().map(|s| s.params.kind()).collect()
    }

    /// Argon2id parameters of the password slot, if there is one.
    pub fn password_kdf(&self) -> Option<KdfParams> {
        self.slots.iter().find_map(|s| match s.params {
            SlotParams::Password { kdf, .. } => Some(kdf),
            _ => None,
        })
    }

    /// D6: true when the password slot uses weaker KDF parameters than
    /// [`KdfParams::DEFAULT`] (e.g. created by an older build). After a successful
    /// password unlock the app re-wraps it with
    /// `change_password(rng, same_password, KdfParams::DEFAULT)` and saves.
    pub fn password_slot_needs_rewrap(&self) -> bool {
        self.password_kdf()
            .is_some_and(|kdf| kdf.is_weaker_than(&KdfParams::DEFAULT))
    }

    fn set_password_slot(
        &mut self,
        rng: &mut (impl RngCore + CryptoRng),
        password: &[u8],
        kdf: KdfParams,
    ) -> Result<()> {
        let salt = crypto::random_bytes(rng);
        let kek = crypto::kek_from_password(password, &salt, kdf)?;
        self.put_slot(rng, SlotParams::Password { kdf, salt }, &kek);
        Ok(())
    }

    fn new_recovery_slot(&mut self, rng: &mut (impl RngCore + CryptoRng)) -> Zeroizing<String> {
        let (secret, text) = recovery::generate(rng);
        let salt = crypto::random_bytes(rng);
        let kek = crypto::kek_from_secret(secret.as_ref(), &salt, INFO_RECOVERY);
        self.put_slot(rng, SlotParams::Recovery { salt }, &kek);
        text
    }

    /// Wraps the VaultKey under `kek` and replaces any slot of the same kind.
    fn put_slot(&mut self, rng: &mut (impl RngCore + CryptoRng), params: SlotParams, kek: &[u8; KEY_LEN]) {
        let nonce = crypto::random_bytes(rng);
        let aad = Header::slot_aad(&self.vault_id, &params);
        let wrapped: [u8; WRAPPED_LEN] = crypto::seal(kek, &nonce, self.vault_key.as_ref(), &aad)
            .try_into()
            .expect("wrapped VaultKey is key + tag");
        let kind = params.kind();
        self.slots.retain(|s| s.params.kind() != kind);
        self.slots.push(Slot { params, nonce, wrapped });
        self.slots.sort_by_key(|s| s.params.kind() as u8);
    }
}

/// Challenge for Hello to sign: bound to the vault and the slot salt.
pub fn hello_challenge(vault_id: Uuid, salt: &[u8; 32]) -> Vec<u8> {
    [HELLO_CHALLENGE_PREFIX, vault_id.as_bytes(), salt].concat()
}

/// Reads the Hello challenge from a locked vault file, so the app can ask
/// Hello for a signature before calling [`UnlockedVault::unlock`].
pub fn hello_challenge_for_file(file: &[u8]) -> Result<Vec<u8>> {
    let parsed = format::parse(file)?;
    parsed
        .header
        .slots
        .iter()
        .find_map(|s| match &s.params {
            SlotParams::Hello { salt } => Some(hello_challenge(Uuid::from_bytes(parsed.header.vault_id), salt)),
            _ => None,
        })
        .ok_or(Error::NoSuchSlot)
}
