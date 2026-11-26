# Security policy

xPass is a client-side encrypted password manager maintained by one person. Reports are welcome and are taken
seriously; please read this page first.

## Supported versions

Only the **latest release** gets security fixes. xPass updates itself from Settings → About (signed in-app update,
since v0.7.0), so moving to the newest version is the fix. Older versions do not get patch releases.

| Version | Supported |
|---|---|
| Latest release | Yes |
| Anything older | No: update in-app (v0.7.0 and later) or install the latest release by hand (v0.6.x and earlier) |

At 1.0 this may become a range of supported versions; this page will say so.

## Reporting a vulnerability

Report it **privately** through GitHub: open the repository's **Security** tab → **Report a vulnerability**
([direct link](https://github.com/bs135/xpass/security/advisories/new)). Do not open a public issue, pull request
or discussion for a vulnerability.

Please include:
- the xPass version (Settings → About) and Windows version;
- what an attacker needs (see the attackers in [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) §3) and what they get;
- steps to reproduce, or a proof of concept. **Never send a real vault, password, recovery key or GitHub token**:
  make a test vault with made-up entries.

## What to expect

A one-person project, so these are best-effort targets, not guarantees:

- acknowledgement within **7 days**;
- a first assessment (accepted, needs more information, or out of scope) within **14 days**;
- for an accepted issue: a fix released as soon as it is ready, through the normal release workflow (signed
  installers and in-app update), and a GitHub security advisory crediting you unless you ask not to be named.

Please give the fix time to ship before publishing details; 90 days is the default, and less once a fixed release
is out. There is no bug bounty.

## Scope

In scope: anything that breaks what [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) says xPass protects, for example:
- reading or changing vault contents without a credential, or bypassing a check of the file format
  ([docs/FORMAT.md](docs/FORMAT.md));
- the WebView getting secrets, file access or network access it should not have;
- installing an update that is not signed with the project's update key, or from another source;
- secrets left in logs, settings, crash reports or the clipboard against the documented rules;
- sync losing or exposing data, or the GitHub token leaking.

Out of scope (accepted limits, THREAT_MODEL §5): malware running as the same Windows user, someone with the
unlocked PC, the self-signed code-signing certificate not being trusted by Windows or SmartScreen, metadata visible
to GitHub, and data in exports the user chose to write. Reports about these are still read, but they are not
treated as vulnerabilities unless they show something the threat model does not already accept.

## Verifying releases

How to check a download (SHA-256 checksums, the Authenticode thumbprint, build provenance attestations): README
["Check the download"](README.md#check-the-download).
