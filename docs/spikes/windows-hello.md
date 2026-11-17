# Spike: Windows Hello as a key source

Status: **passed** (2026-11-14). Code: [`spikes/windows-hello`](../../spikes/windows-hello).
Related: [IDEA.md §3.1](../IDEA.md#31-windows-hello--credential-manager-critical).

## Goal

Confirm that Windows Hello can produce a **stable key-wrapping key (KEK)** for xPass, instead of acting as a
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
3. **Foreground**: the dialog window can be found and focused from a console process, and from the Tauri/WebView2
   window in v0.3 (see open items).
4. The KEK never touches disk; only the wrapped VaultKey and the salt do. Reading Credential Manager or the state
   file is not enough to unlock — a Hello signature is required.

## Open items

- [x] **Reboot**: `unlock` after a restart derived the same KEK and unwrapped the VaultKey.
- [x] **Tauri window**: in v0.3 the Hello dialog appeared in front of the xPass window every time (enable, unlock
  button, auto-prompt on lock and on start, restore from minimize). Tested with the watcher only; it stays on,
  because `KeyCredentialManager` has no HWND-owned interop to make the dialog a child of the window.
- [x] **Hello reset**: handled in v0.3. A missing credential (`NotFound`) or a signature that no longer opens the
  slot marks Hello as broken: the unlock screen explains it and hides the Hello button, the next password or
  recovery unlock removes the dead slot, and the app offers to set Hello up again. Covered by unit tests with a
  fake Hello backend; the check on a real PC (removing the PIN) is in the
  [v0.3.0 manual checklist](../plan-v0.3.0.md#manual-test-checklist-v030).
- [x] **Credential name**: `xpass-<vault-id>` per vault (a dash instead of a slash to stay clear of key-container
  naming rules). A VaultKey rotation deletes the credential; re-enrolling creates it again.

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
