# Import fixtures

Synthetic sample files for the import tests and the v0.5.0 manual checklist. Every value is made up:
hosts are `example.com` / `example.net` / `example.org`, passwords are `Sample-Pass-NN`, TOTP secrets are
placeholders. Never add data from a real vault here.

| File | Shape |
|---|---|
| `bitwarden.csv` | Bitwarden personal vault export (folder, favorite, several URIs, custom fields, TOTP, secure note, card row) |
| `bitwarden-org.csv` | Bitwarden organization export (`collections` instead of `folder`) |
| `chrome.csv` | Chrome / Edge password export (empty name, multi-line note) |
| `keepassxc.csv` | KeePassXC CSV export (groups, TOTP, dates, one bad date) |
| `xpass.csv` | xPass CSV export (several URLs and tags, custom fields as JSON) |
| `generic-semicolon.csv` | `;`-separated file from an unknown app, with an extra column |
| `bad-encoding.csv` | Windows-1252 bytes (not UTF-8) |
