# Spike: Windows Hello as a key source

Status: **passed** (2026-11-14). Code: [`spikes/windows-hello`](../../spikes/windows-hello).
Related: [IDEA.md §3.1](../IDEA.md#31-windows-hello--credential-manager-critical).

## Goal

Confirm that Windows Hello can produce a **stable key-wrapping key (KEK)** for xpass, instead of acting as a
yes/no prompt in front of a secret stored in Credential Manager.

Questions:

1. Can a plain (unpackaged) Rust desktop process use `KeyCredentialManager`?
2. Is the Hello signature over a fixed challenge deterministic, so it can feed HKDF?
3. Can the Hello prompt be brought to the foreground?

## Design under test

```
signature = HelloSign("xpass-hello-v1" || salt)          // RSA-2048 key in TPM, PKCS#1 v1.5
KEK       = HKDF-SHA256(ikm = signature, salt = salt, info = "xpass/kek-hello/v1")
wrapped   = AES-256-GCM(KEK, nonce, VaultKey, aad = header)
```

- Credential name: `xpass-spike` (`KeyCredentialManager::RequestCreateAsync` / `OpenAsync`).
- `salt`, `nonce`, `wrapped` are stored in a state file; in the real app they belong to the vault header.
- Only truncated SHA-256 fingerprints are printed, never key material.
- Focus workaround: a watcher thread finds the window of class `Credential Dialog Xaml Host` and calls
  `SetForegroundWindow` while the Hello call is pending (`--no-focus` disables it).
- Stack: `windows` 0.62 (`Security_Credentials`, `Security_Cryptography`, `Win32_UI_WindowsAndMessaging`),
  `hkdf`, `sha2`, `aes-gcm`, `zeroize`. Async WinRT calls are awaited with the blocking `.join()`.

## Results

Machine: ThinkPad T14, Windows 11 Pro (build 26300), Hello configured.

| Step | Result |
|---|---|
| `status` | `Windows Hello supported: true` |
| `enroll` | Credential created, VaultKey wrapped. KEK fp `32846b0ebe9cb256`, VaultKey fp `748b9e2ae4b4db3e`. |
| `unlock` (new process) | Same KEK fp, VaultKey unwrapped and matched enrollment. |
| `determinism` | Two signatures over the same challenge: identical, 256 bytes (RSA-2048). |
| `unlock --no-focus` | Same KEK fp, unwrap OK. |
| `unlock` after reboot | Same KEK fp `32846b0ebe9cb256`, VaultKey unwrapped and matched enrollment. |
| Focus watcher | Dialog found every time, `SetForegroundWindow = true`. |

Timing: `enroll` sign + HKDF took ~100 ms (no second prompt right after credential creation); `unlock` took ~3 s,
which is the user verification time, not computation.

## Conclusions

1. **Works from an unpackaged Rust process.** No MSIX packaging or app identity is required.
2. **Signatures are deterministic**, so `HKDF(signature)` gives a stable KEK across processes and reboots. The planned design
   in IDEA.md §3.1 is confirmed.
3. **Foreground**: the dialog window can be found and focused from a console process. Whether it is needed in a
   Tauri/WebView2 window is not yet known (see open items).
4. The KEK never touches disk; only the wrapped VaultKey and the salt do. Reading Credential Manager or the state
   file is not enough to unlock — a Hello signature is required.

## Open items

- [x] **Reboot**: `unlock` after a restart derived the same KEK and unwrapped the VaultKey.
- [ ] **Tauri window**: repeat focus behaviour from a real Tauri window (the target), with and without the watcher.
- [ ] **Hello reset**: removing the PIN / resetting Hello deletes the TPM key, so the Hello KEK is lost for good.
  The app must detect `NotFound` and fall back to the master password, then offer re-enrollment. Expected, untested.
- [ ] **Credential name**: unpackaged apps share the credential namespace per user. Use a specific name
  (e.g. `xpass/<vault-id>`) per vault.

## Residual risk

Any process running as the user can call `RequestSignAsync` on the same credential name. It still triggers the Hello
prompt, so it needs the user's consent, but malware could time the prompt to coincide with a real unlock. This is
accepted: malware running as the user while the vault is unlocked is outside the threat model (IDEA.md §4).

## Running the spike

```bash
cd spikes/windows-hello
cargo run -q -- status        # no prompt
cargo run -q -- enroll        # create credential, wrap a random VaultKey
cargo run -q -- unlock        # re-derive KEK, unwrap
cargo run -q -- determinism   # sign the same challenge twice
cargo run -q -- remove        # delete credential and state file
```
