# Spike: GitHub sync + merge (offline-first, multi-device)

Status: **passed** (2026-11-14); follow-ups closed in v0.4.0 ([plan](../plan-v0.4.0.md)).
Code: [`spikes/github-sync`](https://github.com/bs135/xpass/tree/v0.5.0/spikes/github-sync).
Related: [IDEA.md §3.4–3.5](../IDEA.md#34-multi-device-sync-and-conflicts), [vault-format.md](vault-format.md).

## Goal

1. Confirm the GitHub Contents API gives the semantics the design relies on: compare-and-swap on the blob SHA,
   conflict status codes, raw downloads beyond 1 MB, read-after-write consistency.
2. Validate a per-entry merge that converges on every device regardless of sync order.
3. Validate an offline-first sync loop: local-first edits, retry on conflict, credential changes on other devices.

## Design under test

### Remote store

```rust
trait RemoteStore {
    fn get(&self) -> Result<Option<Remote /* bytes, blob sha */>, StoreError>;
    fn put(&self, bytes: &[u8], expected_sha: Option<&str>) -> Result<String /* new sha */, StoreError>;
}
// StoreError: Conflict | Offline | Unauthorized | Other
```

- `GitHubStore` (`reqwest` blocking, rustls):
  - `GET …/contents/{path}?ref={branch}` with `Accept: application/vnd.github.raw+json` → raw bytes, no base64/JSON
    wrapper. The blob SHA is computed locally: `sha1("blob <len>\0" ‖ bytes)` (verified against `git hash-object`).
  - `PUT …/contents/{path}` with `{message, content (base64), sha?, branch}`.
    `409` (stale sha) and `422` (sha missing for an existing file) both map to `Conflict`.
- `MemoryStore`: same CAS semantics for tests, plus offline simulation and a hook that lets "another device" push
  between our GET and PUT.
- Token: fine-grained PAT (one repo, `Contents: Read and write`) stored in Windows Credential Manager via `keyring`,
  entered with a hidden prompt, never printed.

### Merge (per-entry last-writer-wins)

For an id present on both sides: higher `updated_at` wins; on a tie the tombstone wins; still tied → the larger
serialized entry wins, so every device picks the same one. `revision = max`. Output sorted by `created_at`, `id`.

### Sync loop (per device)

Local state: `vault.xpv` (encrypted, written atomically via temp file + rename) and `sync-state.txt`
(`base_sha` = remote SHA last agreed on, `dirty`).

```text
edit():  apply → write local vault → dirty = true            (never touches the network)

sync():  repeat up to 5 times:
  remote = GET                      offline → return Offline (keep dirty)
  no remote          → PUT(expected = none)                   → Pushed
  remote.sha == base → clean: UpToDate | dirty: PUT(expected = base) → Pushed
  remote.sha != base → theirs = open_sibling(remote)          (decrypt with in-memory VaultKey, adopt their slots)
                       clean: take theirs                     → Pulled
                       dirty: merge(ours, theirs), PUT(expected = remote.sha) → Merged
  PUT Conflict → retry from GET;  memory and local file are only updated after a successful PUT
```

`open_sibling` (added to `vault-format`) decrypts the remote copy without asking for credentials:
password changed elsewhere → new slots adopted; VaultKey rotated elsewhere → `KeyChanged` (ask for the password,
local edits stay dirty); different `vault_id` → `DifferentVault`.

## Results

### Automated tests (`cargo test`, in-memory store): 12 / 12 passed

| Test | What it proves |
|---|---|
| `blob_sha_matches_git` | Local SHA equals `git hash-object`. |
| `merge_rules` | Newer wins, union of ids, newer edit beats older delete, delete wins a tie. |
| `merge_converges_regardless_of_order` | 300 random cases: merge is commutative, idempotent and associative. |
| `second_sync_is_a_no_op` | No extra PUTs when nothing changed. |
| `disjoint_edits_merge` | A and B add different entries → both have all of them. |
| `same_entry_newer_edit_wins_on_both_devices` | Concurrent edits of one entry → newer wins everywhere, either sync order. |
| `delete_is_not_resurrected_by_stale_device` | A device that never saw a delete does not bring the entry back. |
| `race_between_get_and_put_is_retried` | A push between our GET and PUT → `Conflict` → retry → merged with exactly one successful PUT. |
| `offline_edits_are_kept_and_pushed_later` | Offline edit survives an app restart and is pushed when back online. |
| `password_change_on_other_device_is_adopted` | Synced silently with the in-memory key; local copy now opens with the new password only. |
| `rotation_on_other_device_requires_credentials` | `KeyChanged`, local edit kept dirty. |
| `remote_from_another_vault_is_refused` | `DifferentVault`, nothing overwritten. |

### Real GitHub (`probe`, private test repo, second run): 0 checks failed

| Check | Result |
|---|---|
| PUT returns the git blob SHA | ✅ |
| Raw GET returns the same bytes; local SHA == PUT SHA | ✅ |
| First GET right after **create** (3 rounds on a fresh path) | ✅ 0 wrong |
| PUT with current SHA | ✅ |
| PUT with stale SHA → `Conflict` (409) | ✅ |
| PUT without SHA on an existing file → `Conflict` (422) | ✅ |
| Read-after-write, 5 rounds | ✅ 0 stale reads |
| 1.5 MB PUT + raw GET | ✅ PUT 3.5 s, GET 1.7 s |
| 5 MB PUT + raw GET | ✅ PUT 5.2 s, GET 2.6 s |
| Rate limit after the run | 4967 / 5000 remaining (a full probe is a few dozen calls) |

Latency (small files): **PUT ≈ 1.1–1.4 s, GET ≈ 0.35 s**. A sync with a push takes ~1.5–2 s.

**One anomaly on the very first run**: on a repository created minutes earlier, the first GET right after creating
`probe.bin` returned 1521 bytes instead of the 1024 written (both content checks failed; every later read in that run
was correct). It did not reproduce in the second run (1 create + 3 create-then-read rounds, all correct), and the
probe now prints response headers and a body preview if it happens again. Even when it happens, the engine is safe:
a wrong body fails to parse or decrypt, so sync stops with an error and never PUTs.

### Real GitHub (`demo`, two simulated devices, one real vault): passed

```text
A sync: Pushed                       A creates the vault with "GitHub <alice>"
B cloned: ["GitHub <alice>"]         B joins with the password
                                     offline: A and B both edit "GitHub"; B (newer) also adds "Mail"
A sync: Pushed
B sync: Merged                       B's PUT conflicts → merge
A sync: Pulled
A == B: ["GitHub <alice-from-B (newer)>", "Mail (added on B)"]           ✅ sequential conflict converged

                                     A adds "Bank", B adds "Cloud", both sync at the same time
concurrent: A Pushed, B Merged       B lost the race (409) → retried → merged
follow-up:  A Pulled, B UpToDate
A == B: [GitHub, Mail, Bank, Cloud]                                       ✅ concurrent push converged
```

## Conclusions

1. **The GitHub Contents API is a suitable backend.** SHA-based compare-and-swap (409/422) gives safe concurrent
   writes without any server of our own; raw downloads remove the 1 MB limit (5 MB tested).
2. **Per-entry LWW with tombstones converges** in unit tests (including randomized algebraic checks) and with two
   real devices racing on GitHub.
3. **Offline-first works**: edits never wait for the network, survive restarts, and are reconciled later.
4. **Credential changes propagate correctly**: password change elsewhere is adopted silently; VaultKey rotation
   elsewhere requires the password, with local edits preserved.
5. Stay on the **REST API, not git CLI** (IDEA.md §3.5): no local git, ~1.5 s per push, rate limit far from a concern.

## Known limitations / follow-ups for the real implementation

All closed in v0.4.0 ([plan-v0.4.0.md](../plan-v0.4.0.md)):

- [x] **LWW drops the older of two concurrent edits to the same entry** → D1: the losing version is kept in the
  entry's history with the note `conflict` ([`merge.rs`](../../crates/xpass-core/src/merge.rs)).
- [x] **Clock skew** → D2: HLC-lite `updated_at = max(now, last seen + 1)`
  ([`clock.rs`](../../crates/xpass-core/src/clock.rs)); the session clock is refreshed after every merge.
- [x] **Tombstones grow forever** → D3: purged after 180 days, on the merged payload before every push
  ([`maintenance.rs`](../../crates/xpass-core/src/maintenance.rs), S15).
- [x] **Retry on unreadable remote** → a GET body that does not parse is read again once after 2 s before the
  error is reported ([`sync/mod.rs`](../../crates/xpass-core/src/sync/mod.rs), `UNREADABLE_RETRY_MS`).
- [x] **Every save is a commit** → saves stay local; the worker pushes 10 s after the last edit, every 5 min while
  unlocked, on unlock, on lock and on exit, never two syncs at once
  ([`sync_worker.rs`](../../src-tauri/src/sync_worker.rs), D8).
- [x] **Async client** → `reqwest` async with rustls on Tauri's tokio runtime, 15 s connect / 60 s total timeouts
  ([`github.rs`](../../src-tauri/src/github.rs), D7).
- [x] **401 / 403** → sync pauses with "GitHub token expired or lacks access"; edits stay local and a new token in
  Settings → Sync resumes it ([`SyncProblem.tsx`](../../src/components/SyncProblem.tsx)).

## Running the spike

```bash
cd spikes/github-sync
cargo test                                         # offline, in-memory store
cargo run -q -- set-token                          # fine-grained PAT, hidden input
cargo run -q -- probe   <owner/repo>               # API semantics
cargo run -q -- demo    <owner/repo>               # two devices, one real vault
cargo run -q -- cleanup <owner/repo>               # remove files under xpass-spike/
cargo run -q -- delete-token
```
