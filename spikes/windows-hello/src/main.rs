//! Spike: Windows Hello as a key source for xpass.
//!
//! Questions this spike answers:
//! 1. Can a plain Rust process use `KeyCredentialManager` (TPM-backed Hello key)?
//! 2. Is the signature over a fixed challenge deterministic, so it can feed HKDF
//!    and yield the same key-wrapping key (KEK) across runs and reboots?
//! 3. Can the Hello prompt be brought to the foreground from a desktop process?
//!
//! Flow mirrors the planned design:
//!   signature = HelloSign("xpass-hello-v1" || salt)
//!   KEK       = HKDF-SHA256(ikm = signature, salt = salt, info = "xpass/kek-hello/v1")
//!   wrapped   = AES-256-GCM(KEK, nonce, VaultKey, aad = header)
//!
//! Only fingerprints (truncated SHA-256) are printed, never key material.

use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};
use windows::{
    core::{Array, HSTRING},
    Security::{
        Credentials::{
            KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
        },
        Cryptography::CryptographicBuffer,
    },
    Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow},
};
use zeroize::Zeroizing;

const CREDENTIAL_NAME: &str = "xpass-spike";
const CHALLENGE_PREFIX: &[u8] = b"xpass-hello-v1";
const KEK_INFO: &[u8] = b"xpass/kek-hello/v1";
const HEADER_AAD: &[u8] = b"xpass-spike-header-v1";
/// Window class of the Windows Hello / Windows Security credential dialog.
const HELLO_DIALOG_CLASS: &str = "Credential Dialog Xaml Host";

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let focus = !args.iter().any(|a| a == "--no-focus");
    let cmd = args.iter().find(|a| !a.starts_with("--")).map(String::as_str);

    let result = match cmd {
        Some("status") => cmd_status(),
        Some("enroll") => cmd_enroll(focus),
        Some("unlock") => cmd_unlock(focus),
        Some("determinism") => cmd_determinism(focus),
        Some("remove") => cmd_remove(),
        _ => {
            eprintln!(
                "usage: hello-spike <status|enroll|unlock|determinism|remove> [--no-focus]\n\
                 \n  status       check whether Windows Hello is available (no prompt)\
                 \n  enroll       create Hello credential, wrap a random VaultKey, save state\
                 \n  unlock       re-derive KEK via Hello and unwrap the saved VaultKey\
                 \n  determinism  sign the same challenge twice and compare signatures\
                 \n  remove       delete the Hello credential and the state file\
                 \n  --no-focus   do not try to bring the Hello dialog to the foreground"
            );
            std::process::exit(2);
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_status() -> AnyResult<()> {
    let supported = KeyCredentialManager::IsSupportedAsync()?.join()?;
    println!("Windows Hello supported: {supported}");
    if supported {
        let open = KeyCredentialManager::OpenAsync(&HSTRING::from(CREDENTIAL_NAME))?.join()?;
        println!("credential '{CREDENTIAL_NAME}': {}", status_name(open.Status()?));
    }
    println!("state file: {}", state_path().display());
    Ok(())
}

fn cmd_enroll(focus: bool) -> AnyResult<()> {
    ensure_supported()?;
    let credential = open_or_create(focus)?;

    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);

    let started = Instant::now();
    let kek = derive_kek(&credential, &salt, focus)?;
    println!("Hello sign + HKDF took {:?}", started.elapsed());

    let mut vault_key = Zeroizing::new([0u8; 32]);
    rand::thread_rng().fill_bytes(vault_key.as_mut());
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);

    let wrapped = Aes256Gcm::new_from_slice(kek.as_ref())?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload { msg: vault_key.as_ref(), aad: HEADER_AAD },
        )
        .map_err(|_| "wrap failed")?;

    let state = State { salt: salt.to_vec(), nonce: nonce.to_vec(), wrapped, vk_fp: fingerprint(vault_key.as_ref()) };
    state.save()?;

    println!("KEK fingerprint:      {}", fingerprint(kek.as_ref()));
    println!("VaultKey fingerprint: {}", state.vk_fp);
    println!("saved {}", state_path().display());
    Ok(())
}

fn cmd_unlock(focus: bool) -> AnyResult<()> {
    ensure_supported()?;
    let state = State::load()?;
    let credential = open_existing()?;

    let started = Instant::now();
    let kek = derive_kek(&credential, &state.salt, focus)?;
    println!("Hello sign + HKDF took {:?}", started.elapsed());
    println!("KEK fingerprint:      {}", fingerprint(kek.as_ref()));

    let vault_key = Zeroizing::new(
        Aes256Gcm::new_from_slice(kek.as_ref())?
            .decrypt(
                Nonce::from_slice(&state.nonce),
                Payload { msg: &state.wrapped, aad: HEADER_AAD },
            )
            .map_err(|_| "unwrap failed: KEK differs from enrollment (signature not stable?)")?,
    );

    let fp = fingerprint(&vault_key);
    println!("VaultKey fingerprint: {fp}");
    if fp == state.vk_fp {
        println!("OK: VaultKey recovered, matches enrollment");
        Ok(())
    } else {
        Err("VaultKey fingerprint mismatch".into())
    }
}

fn cmd_determinism(focus: bool) -> AnyResult<()> {
    ensure_supported()?;
    let credential = open_existing()?;
    let salt = [0x42u8; 32];
    let a = sign(&credential, &challenge(&salt), focus)?;
    let b = sign(&credential, &challenge(&salt), focus)?;
    println!("signature length: {} bytes", a.len());
    println!("signature #1 fp: {}", fingerprint(&a));
    println!("signature #2 fp: {}", fingerprint(&b));
    if *a == *b {
        println!("OK: signatures are identical (deterministic)");
        Ok(())
    } else {
        Err("signatures differ: cannot derive a stable key from them".into())
    }
}

fn cmd_remove() -> AnyResult<()> {
    KeyCredentialManager::DeleteAsync(&HSTRING::from(CREDENTIAL_NAME))?.join()?;
    println!("deleted credential '{CREDENTIAL_NAME}'");
    let path = state_path();
    if path.exists() {
        fs::remove_file(&path)?;
        println!("deleted {}", path.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Hello helpers
// ---------------------------------------------------------------------------

fn ensure_supported() -> AnyResult<()> {
    if KeyCredentialManager::IsSupportedAsync()?.join()? {
        Ok(())
    } else {
        Err("Windows Hello is not set up on this machine".into())
    }
}

fn open_existing() -> AnyResult<KeyCredential> {
    let result = KeyCredentialManager::OpenAsync(&HSTRING::from(CREDENTIAL_NAME))?.join()?;
    match result.Status()? {
        KeyCredentialStatus::Success => Ok(result.Credential()?),
        s => Err(format!("open credential: {} (run `enroll` first)", status_name(s)).into()),
    }
}

fn open_or_create(focus: bool) -> AnyResult<KeyCredential> {
    let name = HSTRING::from(CREDENTIAL_NAME);
    let open = KeyCredentialManager::OpenAsync(&name)?.join()?;
    if open.Status()? == KeyCredentialStatus::Success {
        println!("reusing existing credential '{CREDENTIAL_NAME}'");
        return Ok(open.Credential()?);
    }
    println!("creating credential '{CREDENTIAL_NAME}' (Hello prompt)...");
    let op = KeyCredentialManager::RequestCreateAsync(&name, KeyCredentialCreationOption::FailIfExists)?;
    let result = with_focus(focus, || op.join())?;
    match result.Status()? {
        KeyCredentialStatus::Success => Ok(result.Credential()?),
        s => Err(format!("create credential: {}", status_name(s)).into()),
    }
}

fn challenge(salt: &[u8]) -> Vec<u8> {
    [CHALLENGE_PREFIX, salt].concat()
}

fn sign(credential: &KeyCredential, data: &[u8], focus: bool) -> AnyResult<Zeroizing<Vec<u8>>> {
    let buffer = CryptographicBuffer::CreateFromByteArray(data)?;
    let op = credential.RequestSignAsync(&buffer)?;
    let result = with_focus(focus, || op.join())?;
    match result.Status()? {
        KeyCredentialStatus::Success => {
            let mut out = Array::<u8>::new();
            CryptographicBuffer::CopyToByteArray(&result.Result()?, &mut out)?;
            Ok(Zeroizing::new(out.to_vec()))
        }
        s => Err(format!("sign: {}", status_name(s)).into()),
    }
}

fn derive_kek(credential: &KeyCredential, salt: &[u8], focus: bool) -> AnyResult<Zeroizing<[u8; 32]>> {
    let signature = sign(credential, &challenge(salt), focus)?;
    let mut kek = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(salt), &signature)
        .expand(KEK_INFO, kek.as_mut())
        .map_err(|_| "hkdf expand failed")?;
    Ok(kek)
}

/// Runs a blocking Hello call while a watcher thread tries to bring the
/// credential dialog to the foreground (desktop apps often get it behind).
fn with_focus<T>(focus: bool, call: impl FnOnce() -> windows::core::Result<T>) -> windows::core::Result<T> {
    if !focus {
        return call();
    }
    let done = Arc::new(AtomicBool::new(false));
    let watcher = {
        let done = done.clone();
        thread::spawn(move || {
            let class = HSTRING::from(HELLO_DIALOG_CLASS);
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
                if let Ok(hwnd) = unsafe { FindWindowW(&class, None) } {
                    let ok = unsafe { SetForegroundWindow(hwnd) }.as_bool();
                    println!("[focus] Hello dialog found, SetForegroundWindow = {ok}");
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            if !done.load(Ordering::Relaxed) {
                println!("[focus] Hello dialog window not found within 10s");
            }
        })
    };
    let result = call();
    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    result
}

fn status_name(s: KeyCredentialStatus) -> &'static str {
    match s {
        KeyCredentialStatus::Success => "Success",
        KeyCredentialStatus::UnknownError => "UnknownError",
        KeyCredentialStatus::NotFound => "NotFound",
        KeyCredentialStatus::UserCanceled => "UserCanceled",
        KeyCredentialStatus::UserPrefersPassword => "UserPrefersPassword",
        KeyCredentialStatus::CredentialAlreadyExists => "CredentialAlreadyExists",
        KeyCredentialStatus::SecurityDeviceLocked => "SecurityDeviceLocked",
        _ => "Unknown status",
    }
}

// ---------------------------------------------------------------------------
// State file (spike only; real app stores this in the vault header)
// ---------------------------------------------------------------------------

struct State {
    salt: Vec<u8>,
    nonce: Vec<u8>,
    wrapped: Vec<u8>,
    vk_fp: String,
}

impl State {
    fn save(&self) -> AnyResult<()> {
        let text = format!(
            "salt={}\nnonce={}\nwrapped={}\nvk_fp={}\n",
            hex(&self.salt),
            hex(&self.nonce),
            hex(&self.wrapped),
            self.vk_fp
        );
        fs::write(state_path(), text)?;
        Ok(())
    }

    fn load() -> AnyResult<Self> {
        let text = fs::read_to_string(state_path()).map_err(|_| "no state file (run `enroll` first)")?;
        let get = |key: &str| -> AnyResult<String> {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .map(str::to_owned)
                .ok_or_else(|| format!("state file missing '{key}'").into())
        };
        Ok(Self {
            salt: unhex(&get("salt")?)?,
            nonce: unhex(&get("nonce")?)?,
            wrapped: unhex(&get("wrapped")?)?,
            vk_fp: get("vk_fp")?,
        })
    }
}

fn state_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("hello-spike-state.txt")))
        .unwrap_or_else(|| PathBuf::from("hello-spike-state.txt"))
}

fn fingerprint(data: &[u8]) -> String {
    hex(&Sha256::digest(data)[..8])
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> AnyResult<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err("bad hex".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.into()))
        .collect()
}
