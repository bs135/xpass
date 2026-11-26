# xPass user guide

How to install xPass, use it day to day, and get your passwords back when something goes wrong. For what xPass
protects against and what it does not, see the [threat model](THREAT_MODEL.md); for the file format, see
[FORMAT.md](FORMAT.md).

**Read [Backups and recovery](#5-backups-and-recovery) before you rely on xPass.** There is no xPass account and
no server: nobody, including the developer, can open your vault or reset your master password.

- [1. Install](#1-install)
- [2. First start](#2-first-start)
- [3. Locking](#3-locking)
- [4. Entries](#4-entries)
- [5. Backups and recovery](#5-backups-and-recovery)
- [6. Sync with GitHub](#6-sync-with-github)
- [7. Import and export](#7-import-and-export)
- [8. Updates](#8-updates)
- [9. Uninstall](#9-uninstall)
- [10. Where xPass keeps its files](#10-where-xpass-keeps-its-files)
- [11. Privacy and network](#11-privacy-and-network)
- [12. Get help](#12-get-help)

## 1. Install

### Windows versions

| Windows | Support |
|---|---|
| Windows 11 (x64) | Supported. |
| Windows 10 22H2 (x64) | Works, **best effort**. Microsoft ended support for Windows 10 on 14 October 2025: without Extended Security Updates the system gets no security fixes, and a password manager is only as safe as the system it runs on. Bugs that only occur on Windows 10 may not be fixed. |
| Older Windows, 32-bit or ARM Windows | Not supported. |

The WebView2 runtime is downloaded by the installer if Windows does not have it yet (it ships with Windows 11).

### Installers

Download one installer from [Releases](https://github.com/bs135/xpass/releases):

- `xpass_<version>_windows_x64-setup.exe` (NSIS, recommended): installs for your Windows user only, without an
  administrator prompt, into `%LOCALAPPDATA%\xpass`. The first install asks for English or Vietnamese; later
  installs and updates reuse that choice.
- `xpass_<version>_windows_x64.msi`: for managed installs (Intune, Group Policy).

Installing a newer version over an older one keeps your vault and settings.

### Check the download

Each release has `SHA256SUMS.txt`; compare the installer's hash with the line for its file name before running it:

```powershell
(Get-FileHash .\xpass_1.0.0_windows_x64-setup.exe -Algorithm SHA256).Hash.ToLower()
```

The installers, `xpass.exe` and the uninstaller carry an Authenticode signature from a **self-signed** xPass
certificate (since v0.7.0). Windows does not know that certificate, so it reports the signature as not trusted;
what matters is **which** certificate signed the file. Check that the thumbprint is exactly
`4D3E068AED69DE2C88578FC2CA46439E78E73D7D` (also in [codesign/xpass-codesign.cer](codesign/xpass-codesign.cer)
and in every release's notes):

```powershell
(Get-AuthenticodeSignature .\xpass_1.0.0_windows_x64-setup.exe).SignerCertificate.Thumbprint
```

The same works later on the installed app (`%LOCALAPPDATA%\xpass\xpass.exe`). Do **not** import the certificate
into "Trusted Root Certification Authorities" or "Trusted Publishers": that would make Windows trust anything signed
with its key, not just xPass. If the thumbprint ever changes, the release notes and the README say so first.

Each installer also has a **build provenance attestation**: a statement signed through GitHub that the file was
built by this repository's Release workflow from a specific commit. With the [GitHub CLI](https://cli.github.com/):

```powershell
gh attestation verify .\xpass_1.0.0_windows_x64-setup.exe --repo bs135/xpass
```

The release also lists what is inside as CycloneDX SBOMs (`…_sbom-rust.cdx.json` for the Rust crates,
`…_sbom-npm.cdx.json` for the bundled npm packages), attested for the same installers and covered by
`SHA256SUMS.txt`.

### Windows SmartScreen

Because the certificate is self-signed, SmartScreen may show "Windows protected your PC" (Unknown publisher) on the
first run. It means Windows does not know the publisher, not that the file is harmful. After checking the hash and
the thumbprint, choose **More info** → **Run anyway**. Download xPass only from the Releases page above. Later
versions are installed from Settings → About (in-app update, signature checked by xPass itself).

## 2. First start

1. **Master password.** Choose a long password you can remember; xPass refuses weak ones. It is never stored and
   cannot be reset by anyone.
2. **Emergency Kit.** xPass shows a **recovery key** once and asks you to confirm part of it. Print the kit or
   write the key down, and keep it somewhere safe and separate from the PC (not in a file next to the vault, not in
   the same cloud folder). It is the only way back in if you forget the master password.
3. **Windows Hello** (optional): Settings → Security → turn on Windows Hello to unlock with face, fingerprint or PIN.
   The master password keeps working; Hello is set up per PC.

On a second PC with sync, choose **Use my vault from GitHub** instead (see [Sync](#6-sync-with-github)).

## 3. Locking

The vault locks — xPass forgets the key and secrets in memory and clears a password it copied — when:

- **Windows locks** (`Win+L`, switching user), the **screen turns off**, or the PC **sleeps or hibernates**;
- you **quit** xPass (tray menu → Quit) or choose **Lock** (header, tray, `Ctrl+L`);
- there was **no input for the set time** (Settings → General → Lock after inactivity: 1, 5, 15, 30 or 60 minutes, or **never**; default
  5). The timer counts input on the **whole PC**, not only in xPass: while you work in other apps the vault stays
  unlocked. Use `Win+L` or Lock when you leave the PC;
- optionally, when xPass is **minimized or hidden** (Settings → General, off by default).

Closing the window does **not** lock: by default xPass keeps running in the tray, unlocked (Settings → General →
"Close button hides xPass to the tray"). Turn that off to quit on close.

Wrong passwords are slowed down: the first two attempts have no delay, then 1, 2, 4, 8, 16 and at most 30 seconds.

## 4. Entries

- **Fields**: title, username, password, websites, tags, notes, favorite, and custom fields. **Notes and
  non-secret custom fields are shown whenever the entry is open**, like the username. Put secrets (PINs, recovery
  codes, security answers) in the password or in a custom field marked **Secret**, which stays hidden until you
  reveal it.
- **Custom fields**: add, rename, reorder and delete them in the edit form. A secret value stays hidden until you
  choose Reveal or Replace. Custom fields have **no history**: changing or deleting a secret value asks first, and
  only the local backups keep the old value.
- **Password history**: the last 20 passwords of each entry, with reveal, copy and **Use this password**.
- **Copy**: copied secrets are excluded from clipboard history (`Win+V`) and cloud clipboard, and are cleared after
  the timeout (default 20 s) unless you copied something else meanwhile.
- **Generator**: random characters or EFF passphrases, with an entropy estimate.

### One-time passwords

A TOTP secret is stored as a secret custom field named `TOTP` (an `otpauth://totp/…` link). The entry shows the
current code with a countdown; **Copy code** copies the code, not the secret. To add one, open the entry's edit
form → **Add one-time password**, and paste the `otpauth://` link or the key the site shows when you turn on
two-factor authentication (the text under the QR code). xPass shows the current code so you can compare it with
the site before saving. QR codes cannot be scanned. HOTP (counter-based) and Steam codes are not supported.

Keeping the TOTP secret in the same vault as the password is convenient, but anyone who opens the vault then has
**both** factors of that account. For accounts where that matters, keep the TOTP in a separate authenticator app.

### Password health

The shield button in the entry list shows entries whose password is **weak** (it would not be accepted as a master
password; words from the entry's own title, username and website count against it) or **used in several
entries**. The check runs on this PC only; nothing is sent anywhere.

## 5. Backups and recovery

### What you need, and what xPass keeps for you

| Thing | Where | Why |
|---|---|---|
| Master password | Your memory | Opens the vault. Nobody can reset it. |
| Emergency Kit (recovery key) | Printed or written down, stored apart from the PC | Opens the vault when the master password is forgotten. |
| `vault.xpv` | `%APPDATA%\com.bs135.xpass\` | The vault, encrypted. |
| Local versions | `%APPDATA%\com.bs135.xpass\backups\`, the last 10 | A copy of the previous file is made before every save. |
| GitHub copy (if sync is on) | Your private repository, with its history | Another copy, and the way to move to a new PC. |

**If you lose both the master password and the recovery key, the vault cannot be opened, by anyone.** There is no
back door. Windows Hello can still open it on a PC where it is set up (see below), but you cannot rely on that.

### I forgot my master password

1. On the unlock screen choose **Forgot your password? Use recovery key**.
2. Type the recovery key from your Emergency Kit (case, spaces and dashes do not matter; `0`, `1` and `8` are read
   as `O`, `I` and `B`).
3. Choose a new master password. With sync on, it reaches your other PCs with the next sync.
4. The old recovery key still works. If the kit may have been seen by someone, create a new one: Settings →
   Security → **Create new Emergency Kit**, then destroy the old kit.

### I lost my Emergency Kit (and still know the password)

Settings → Security → **Create new Emergency Kit**. The old key stops working for the vault from now on, but still opens
older copies (local versions, git history). If someone may have the old kit, also **rotate the vault key**
(Settings → Security) and, with sync, **Remove old versions** from GitHub.

### I lost both the password and the kit

Only one way is left: Windows Hello, on a PC where it was turned on for this vault. **Without Hello, the vault is
lost.** With it, move the entries into a new vault:

1. Unlock with Windows Hello. Do not lock or restart before step 3 if you can avoid it.
2. If sync is on: Settings → Sync → **Turn off sync** (the new vault would otherwise clash with the old one on
   GitHub as a "different vault").
3. Settings → Import & export → **Export as CSV**, and confirm with Windows Hello (exports accept it instead of the
   password). Save it on this PC, not in a cloud-synced folder. The xPass CSV keeps websites, notes, tags, favorites
   and custom fields (TOTP secrets included); it does **not** keep password history. The file is **plaintext**.
4. Quit xPass (tray → Quit). Move `vault.xpv` out of `%APPDATA%\com.bs135.xpass\` into another folder (keep it:
   Hello still opens it on this PC). Leave the `backups` folder alone.
5. Start xPass: it shows the first-start screen. Create a new vault with a new master password and **write down
   the new Emergency Kit**.
6. Settings → Import & export → **Choose CSV file…**, pick the export (format **xPass**), import, then **delete the
   CSV** on the result screen and empty the Recycle Bin.
7. Turn Windows Hello on again for the new vault; set up sync again, with a new path or repository (the old file on
   GitHub belongs to the old vault).

An **Export as KeePass (KDBX)** keeps the password history too and is encrypted, but xPass cannot import KDBX files;
it is a good extra copy to make at step 3.

### My PC was lost or stolen

- The vault on it is encrypted; how long it resists guessing depends on your master password. A locked or
  switched-off PC holds no key.
- On a new PC, install xPass and choose **Use my vault from GitHub** (sync). Without sync, use your own backup: do
  not create a vault on the first-start screen; quit xPass (tray → Quit), create the folder
  `%APPDATA%\com.bs135.xpass\` if it does not exist, copy your backup into it as `vault.xpv`, then start xPass.
- On GitHub, **revoke the token** the lost PC used (Settings → Developer settings → Personal access tokens) and
  create a new one.
- If the thief may know or guess your master password, or the PC was taken unlocked: change the master password,
  **rotate the vault key**, remove old versions from GitHub, and change the passwords of important accounts.
- The lost PC's Windows Hello key is in its TPM and useless anywhere else.

### Windows Hello stopped working

After Windows Hello was reset on the PC (for example after a forgotten PIN) or the vault key was rotated, Hello can no longer unlock the
vault. Unlock with the master password; xPass shows **Set up again**. Nothing is lost.

### I deleted or changed something by mistake

Settings → **Restore a previous version** lists the last 10 versions on this PC, one per save. Restoring puts the
entries back as they were (entries added later are deleted; changed ones get their old content back, with the
current values in their history). The restore is saved as a new change, so it can be undone the same way, and with
sync it reaches your other PCs. A version saved before a vault key rotation asks for the master password of that
time.

### The vault file is damaged or missing

- **Damaged:** xPass says the file was modified or is not a vault.
- **Missing:** xPass shows the first-start screen as if you had no vault. **Do not create a new vault there.**

In both cases:

1. Quit xPass (tray → Quit). If `vault.xpv` is still there, rename it (for example `vault.damaged.xpv`) instead
   of deleting it.
2. With sync: start xPass and choose **Use my vault from GitHub** on the first-start screen. Done.
3. Without sync: in `%APPDATA%\com.bs135.xpass\backups\`, copy the newest `vault-<date-time>.xpv` into
   `%APPDATA%\com.bs135.xpass\` and **rename the copy to `vault.xpv`**. Then start xPass.

Keep in mind:
- a backup is the file as it was **before** a save, so the newest one may lack your last change;
- a backup opens with the master password (and recovery key) that was valid when it was written: one from before
  a password change needs the old password;
- never edit `vault.xpv` by hand.

### "This vault was saved by a newer version of xPass"

Another PC with a newer xPass wrote the vault (directly or through sync). Update xPass on this PC (Settings →
About), and keep all your PCs on the same version. Nothing was changed.

### Make your own backups

The local versions and GitHub protect against mistakes, not against losing the PC and the repository at once.

- Copy `vault.xpv` to a USB drive or a cloud folder now and then. It is encrypted, so it is safe to store elsewhere
  **if your master password is strong**; it opens with the password (or recovery key) that was valid when it was
  copied.
- Keep the **Emergency Kit** apart from those backups.
- For a copy readable by other software, use **Export as KeePass (KDBX)** with a strong export password. It does
  not update itself; export again after big changes.
- Never keep a **CSV export** as a backup: it is plaintext.
- Now and then, test a backup: sign in to another Windows user account, install xPass there, copy the backup into
  that account's `%APPDATA%\com.bs135.xpass\` as `vault.xpv` before the first start, and open it.

## 6. Sync with GitHub

xPass can keep one vault in sync across your Windows PCs through a single file in a **private** GitHub repository.
There is no xPass server: each PC talks to GitHub directly with a token that can reach only that repository.

### Create a repository and a token

1. Create a **private** repository on GitHub for the vault (an empty one is fine).
2. Open GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained tokens** → Generate new
   token.
3. Repository access: **Only select repositories**, and pick the vault repository.
4. Permissions: **Contents: Read and write** (Metadata: Read-only is added automatically). Nothing else.
5. Choose an expiration date and generate the token. Copy it straight into xPass; you do not need to keep it
   anywhere else.

### Set up the first PC

Settings → **Sync**: enter the repository (`owner/name` or its URL), optionally a branch (default: the repository's
default branch) and the path (default `xpass/vault.xpv`), paste the token and choose **Test connection**. Save is
enabled after a passing test; a public repository needs an explicit confirmation. The first sync creates the file.

### Add another PC

Install xPass and choose **Use my vault from GitHub** on the first screen: the same repository, branch and path, a
token (the same one or a new one), then your master password. The vault is downloaded and opened; your existing
Emergency Kit keeps working, and you can turn Windows Hello on for this PC.

### How it behaves

- Changes are saved on the PC first and synced 10 s after the last edit, every 5 min while unlocked, when you unlock,
  and when the vault locks or the app quits (8 s at most). **Sync now** is in the header, in Settings and in the
  tray menu. Without a connection everything keeps working; changes are synced later.
- Edits of different entries on different PCs are merged. If the same entry was edited on two PCs, the newer edit
  wins and the older password is kept in the entry's history. Other changed fields of the older edit (notes, custom
  fields) are not kept.
- A new master password set on one PC reaches the others with the next sync (they show a notice once).
  Windows Hello is set up per PC and keeps working.
- **Rotating the vault key** with sync on needs a connection: the new file reaches GitHub first. The other PCs then
  ask for the master password once and keep their unsynced changes.
- The header icon shows the state. Problems that need you have a banner with the fix: an expired token (enter a new
  one in Settings → Sync), a vault key changed on another PC (enter the master password), or a different vault at
  the configured path (use another path, or replace this PC's vault with the one on GitHub; the current file is
  kept as a backup).

### What GitHub sees

Each sync that changes something is one commit with the message `xpass: update vault`; **Remove old versions**
makes one commit `xpass: history removed`. Anyone who can read the repository can see **when** you synced (commit
times), the **size** of the vault file and its history, but not your entries: the file is encrypted with your
vault key. Old versions stay in the git history, so a leaked old file can be attacked with the master password
that was valid then; use a strong master password and keep the repository private.

### Remove old versions

After you rotate the vault key (Settings → Security), the older versions in the repository history still open with
the old master password or recovery key. Settings → Sync → **Remove old versions** (also offered right after a
rotation) replaces the sync branch with a single commit holding the current files:

- this PC must be in sync with GitHub, with no changes waiting; you confirm with the master password or Windows Hello;
- your other PCs keep syncing; if one of them pushed at that very moment, its change comes back with its next sync;
- other files in the same branch stay, but lose their history too (the dialog lists them);
- if branch protection or a ruleset blocks force pushes on that branch, GitHub refuses and nothing changes.

What it cannot do: GitHub may still serve an old version to someone who knows its commit ID for a while, and forks,
clones and caches keep their copies. Removing every trace needs GitHub Support.

### Turn sync off or remove the token

Settings → Sync → **Turn off sync** deletes the token from Credential Manager and the sync settings; the vault on
this PC stays. Also revoke the token on GitHub (Settings → Developer settings → Personal access tokens) when you
no longer need it. To replace an expired token, choose **Change** → **Replace token**.

## 7. Import and export

Both are in Settings → **Import & export**.

### Import a CSV file

1. Export your passwords as CSV from the old password manager:
   - **Bitwarden**: Tools → Export vault → File format **.csv** (not the encrypted or JSON formats).
   - **Chrome / Edge**: Settings → Passwords (Password Manager) → Export passwords.
   - **KeePassXC**: Database → Export → CSV file.
   - **Other managers**: any CSV export with a header row.
2. In xPass choose **Choose CSV file…**. The format is detected from the header row and can be changed; for other
   CSV files, pick which column holds the title, username, password, website, notes, tags and favorite, and what
   happens to the other columns (leave out, add to notes, or keep as custom fields).
3. Check the preview. Rows already in the vault (same site, username and password) are skipped. Rows with the same
   site and username but another password are *possible duplicates*: skip them (default), import them as new
   entries, or update the existing entries (the old password stays in their history). Untick rows you do not want.
4. **Import**. The whole import is one change: with sync on it reaches your other PCs at once, and Restore a
   previous version undoes it.
5. **Delete the source CSV** on the result screen. The file is not encrypted. This is an ordinary delete: on an SSD
   or in a cloud-synced folder, copies may remain, so do not export to such a folder in the first place.

What is imported: title, username, password, websites (all of them), notes, tags (Bitwarden folders, KeePassXC
groups), favorites, Bitwarden custom fields, TOTP secrets (as a secret custom field named `TOTP`, which shows its
codes), Bitwarden secure notes, and the creation date from KeePassXC. Not imported: Bitwarden cards and identities
(counted as left out), attachments, passkeys, and encrypted or JSON exports. Files must be UTF-8 (with or without
BOM), comma, semicolon or tab separated, at most 10 MB and 10 000 rows.

### Export

- **Export as KeePass (KDBX)**: an encrypted KDBX 4 file (ChaCha20, Argon2id 64 MiB) protected by a new *export password*.
  KeePassXC and KeePass 2 open it. Entries go into one group `xPass`; extra websites are stored as `KP2A_URL_1`, …
  (KeePassXC shows them), favorites as the tag `favorite`, secret custom fields as protected fields, a TOTP secret
  also as KeePassXC's `otp` attribute, and password history as entry history.
- **Export as CSV**: a **plaintext** file, readable by anyone who gets it. Do not open it in Excel or another spreadsheet app
  (they can change values), do not save it in a folder that syncs to the cloud, and delete it (and empty the Recycle
  Bin) when done. Its columns are `title, username, password, urls, notes, tags, favorite, created, updated,
  fields`; xPass imports it back with the **xPass** format.

Every export asks for the master password (or Windows Hello) and then where to save the file. Deleted entries are
not exported, and neither is password history in the CSV.

## 8. Updates

Settings → About → **Check for updates** shows a newer version with its notes (optionally also checked once a day
at start; off by default, it only shows a badge). Nothing is downloaded or installed until you choose **Install and
restart**. Then xPass downloads the installer, checks its signature against the update key built into the app (and
that it comes from this repository's release for that version), locks the vault, uploads unsynced changes and runs
the installer, which restarts xPass. A file that fails a check is never run. Your vault and settings stay as they
are.

The release notes shown before the install are not covered by the signature; the installer is. If something in the
notes looks wrong, check the release page on GitHub.

- **From v0.6.x:** those versions cannot install updates; install the latest release by hand once.
- **If the update key ever changes** (lost or leaked), xPass cannot accept updates signed with the new key: the
  release notes and the README will say so, and that version must be installed by hand once.

## 9. Uninstall

Settings → Apps → Installed apps → **xpass** (the installed app may be listed in lower case) → Uninstall. By default
your data is **not** removed: the vault, its backups and settings stay in `%APPDATA%\com.bs135.xpass`, and the
GitHub token stays in Windows Credential Manager (`xpass/github-token`) unless you turn sync off before
uninstalling. Also left behind: the Windows Hello key `xpass-<vault id>` (turn Hello off in xPass before
uninstalling to remove it) and the WebView2 cache in `%LOCALAPPDATA%\com.bs135.xpass` (no secrets; safe to
delete).

The NSIS uninstaller also offers a checkbox to delete the application data. Ticking it **deletes the vault and all
local backups** in `%APPDATA%\com.bs135.xpass`; only do that if the vault is synced to GitHub or you no longer need
it. The MSI uninstaller has no such option: delete that folder yourself if you want it gone.

## 10. Where xPass keeps its files

| Path | Content |
|---|---|
| `%APPDATA%\com.bs135.xpass\vault.xpv` | The vault (encrypted). |
| `%APPDATA%\com.bs135.xpass\vault.xpv.tmp` | Only during a save: the new file is written here, flushed, then renamed over `vault.xpv`. |
| `%APPDATA%\com.bs135.xpass\backups\vault-<UTC date-time>.xpv` | The previous file, copied before each save; the last 10 are kept. Encrypted like the vault. |
| `%APPDATA%\com.bs135.xpass\settings.json` | Settings: language, theme, lock timeouts, sync repository / branch / path. Not encrypted or signed; it holds no secrets, and a damaged file falls back to defaults. |
| `%APPDATA%\com.bs135.xpass\sync-state.json` | Which remote version this PC last synced with, and when. |
| Windows Credential Manager, `xpass/github-token` | The GitHub token, if sync is set up. |
| `%LOCALAPPDATA%\xpass\` | The program (NSIS install). |

## 11. Privacy and network

- xPass has **no telemetry, no analytics and no crash reporting**. Windows Error Reporting is told to leave the
  process out (no report and no dump through WER); a debugger or an administrator's dump policy can still capture
  it.
- It connects only to:
  - `api.github.com`: sync, only when you set it up, with your token;
  - `github.com/bs135/xpass/releases`: the update check (when you ask, or once a day at start if you turned that
    on) and the installer download (only when you choose Install).
- Connections use HTTPS only and the Windows proxy settings. Links in entries open in your default browser, never
  inside xPass. The xPass window itself has no network access (Content Security Policy).
- GitHub sees what [What GitHub sees](#what-github-sees) describes; your entries stay encrypted.

## 12. Get help

- A bug or a question: [GitHub Issues](https://github.com/bs135/xpass/issues). Reproduce it with a test vault and
  made-up entries; **never** attach a vault file, a password, a recovery key, a TOTP secret, a token or an export.
- A security problem: report it privately, as [SECURITY.md](../SECURITY.md) explains, never in a public issue.
