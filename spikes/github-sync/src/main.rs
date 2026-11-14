//! CLI to exercise the sync engine against a real GitHub repository.
//!
//! The token is read from Windows Credential Manager (stored by `set-token`)
//! and is never printed. All files live under `xpass-spike/` in the repo.

use std::{
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rand::RngCore;
use sync_spike::{
    engine::{Device, SyncOutcome},
    github::GitHubStore,
    store::{blob_sha, RemoteStore, StoreError},
};
use vault_format::{crypto::KdfParams, model::Entry};
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "xpass-spike-github";
const KEYRING_USER: &str = "token";
const PREFIX: &str = "xpass-spike";
/// Fast KDF for the demo; the real default is 128 MiB / t=3 / p=4.
const DEMO_KDF: KdfParams = KdfParams { m_kib: 8 * 1024, t: 1, p: 1 };
const DEMO_PW: &[u8] = b"demo password";

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let branch = flag(&args, "--branch").unwrap_or_else(|| "main".into());
    let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with("--")).map(String::as_str).collect();
    let positional: Vec<&str> = positional.into_iter().filter(|a| Some(*a) != flag(&args, "--branch").as_deref()).collect();

    let result = match positional.as_slice() {
        ["set-token"] => set_token(),
        ["delete-token"] => delete_token(),
        ["probe", repo] => probe(repo, &branch),
        ["demo", repo] => demo(repo, &branch),
        ["cleanup", repo] => cleanup(repo, &branch),
        _ => {
            eprintln!(
                "usage: sync-spike <command> [--branch main]\n\
                 \n  set-token            store a fine-grained PAT in Windows Credential Manager\
                 \n  delete-token         remove it\
                 \n  probe   <owner/repo> check Contents API semantics (sha, 409/422, raw GET, size, staleness)\
                 \n  demo    <owner/repo> two simulated devices syncing a real vault, incl. a concurrent push\
                 \n  cleanup <owner/repo> delete the files this spike created under {PREFIX}/"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

fn keyring_entry() -> AnyResult<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?)
}

fn set_token() -> AnyResult<()> {
    let token = Zeroizing::new(rpassword::prompt_password("Fine-grained PAT (input hidden): ")?);
    let token = token.trim();
    if token.is_empty() {
        return Err("empty token".into());
    }
    keyring_entry()?.set_password(token)?;
    println!("token stored in Windows Credential Manager ({KEYRING_SERVICE})");
    Ok(())
}

fn delete_token() -> AnyResult<()> {
    keyring_entry()?.delete_credential()?;
    println!("token removed");
    Ok(())
}

fn store(repo: &str, branch: &str, file: &str) -> AnyResult<GitHubStore> {
    let token = Zeroizing::new(keyring_entry()?.get_password().map_err(|_| "no token: run `set-token` first")?);
    Ok(GitHubStore::new(token, repo, &format!("{PREFIX}/{file}"), branch)?)
}

// ---------------------------------------------------------------------------
// probe
// ---------------------------------------------------------------------------

struct Report {
    failed: u32,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: impl std::fmt::Display) {
        if !ok {
            self.failed += 1;
        }
        println!("[{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    }
    fn info(&self, name: &str, detail: impl std::fmt::Display) {
        println!("[INFO] {name}: {detail}");
    }
}

fn random(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut v);
    v
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed())
}

fn probe(repo: &str, branch: &str) -> AnyResult<()> {
    let s = store(repo, branch, "probe.bin")?;
    let mut rep = Report { failed: 0 };

    // 1. Initial state
    let initial = s.get()?;
    rep.info("initial GET", match &initial {
        Some(r) => format!("exists, {} bytes", r.bytes.len()),
        None => "404, file does not exist".into(),
    });

    // 2. Create / update with correct sha; PUT sha must equal locally computed blob sha
    let a = random(1024);
    let (sha_a, t) = timed(|| s.put(&a, initial.as_ref().map(|r| r.sha.as_str())));
    let sha_a = sha_a?;
    rep.check("PUT returns git blob sha", sha_a == blob_sha(&a), format!("{} ({t:?})", &sha_a[..12]));

    // 3. Raw GET returns the exact bytes
    let (got, t) = timed(|| s.get());
    let got = got?.ok_or("file missing right after PUT")?;
    rep.check("raw GET returns same bytes", got.bytes == a, format!("{} bytes ({t:?})", got.bytes.len()));
    rep.check("local blob sha of GET == PUT sha", got.sha == sha_a, &got.sha[..12]);
    if got.bytes != a {
        print_get_meta(&s, a.len());
    }

    // 3b. Create-then-read, repeated on a fresh path, to see whether a wrong
    //     first read after *create* is reproducible and whether it converges.
    let fresh = s.with_path(&format!("{PREFIX}/create-probe.bin"));
    let mut wrong_after_create = 0;
    for round in 1..=3 {
        fresh.delete("xpass spike: create-probe reset")?;
        let data = random(1024);
        let sha = fresh.put(&data, None)?;
        let first = fresh.get()?.ok_or("missing after create")?;
        if first.sha != sha {
            wrong_after_create += 1;
            println!("       round {round}: first GET after create wrong ({} bytes, expected {})", first.bytes.len(), data.len());
            print_get_meta(&fresh, data.len());
            for wait in [1u64, 3, 10] {
                thread::sleep(Duration::from_secs(wait));
                let again = fresh.get()?.map(|r| r.sha == sha).unwrap_or(false);
                println!("       after +{wait}s: {}", if again { "correct" } else { "still wrong" });
                if again {
                    break;
                }
            }
        }
    }
    rep.check("first GET after create (3 rounds)", wrong_after_create == 0, format!("{wrong_after_create} wrong"));
    fresh.delete("xpass spike: create-probe cleanup")?;

    // 4. Update with current sha
    let b = random(1024);
    let sha_b = s.put(&b, Some(&sha_a))?;
    rep.check("PUT with current sha succeeds", true, &sha_b[..12]);

    // 5. Stale sha -> Conflict (expected HTTP 409)
    let stale = s.put(&random(1024), Some(&sha_a));
    rep.check("PUT with stale sha -> Conflict", stale == Err(StoreError::Conflict), format!("{stale:?}"));

    // 6. Missing sha on existing file -> Conflict (expected HTTP 422)
    let missing = s.put(&random(1024), None);
    rep.check("PUT without sha on existing file -> Conflict", missing == Err(StoreError::Conflict), format!("{missing:?}"));

    // 7. Read-after-write consistency
    let mut current = sha_b;
    let (mut stale_reads, mut put_ms, mut get_ms) = (0, Vec::new(), Vec::new());
    for _ in 0..5 {
        let data = random(2048);
        let (sha, tp) = timed(|| s.put(&data, Some(&current)));
        current = sha?;
        let (r, tg) = timed(|| s.get());
        if r?.map(|r| r.sha) != Some(current.clone()) {
            stale_reads += 1;
        }
        put_ms.push(tp.as_millis());
        get_ms.push(tg.as_millis());
    }
    rep.check("read-after-write (5 rounds)", stale_reads == 0, format!("{stale_reads} stale reads; PUT ms {put_ms:?}; GET ms {get_ms:?}"));

    // 8. Large files: Contents API GET JSON is limited to 1 MB; raw should go to 100 MB
    let big = s.with_path(&format!("{PREFIX}/big.bin"));
    for size in [1_500_000usize, 5_000_000] {
        let data = random(size);
        let prev = big.get()?.map(|r| r.sha);
        let (put, tp) = timed(|| big.put(&data, prev.as_deref()));
        match put {
            Ok(_) => {
                let (got, tg) = timed(|| big.get());
                let same = got?.map(|r| r.bytes == data).unwrap_or(false);
                rep.check(&format!("{} MB PUT + raw GET", size as f64 / 1e6), same, format!("PUT {tp:?}, GET {tg:?}"));
            }
            Err(e) => rep.check(&format!("{} MB PUT", size as f64 / 1e6), false, e),
        }
    }

    rep.info("rate limit remaining", s.last_rate_limit.lock().unwrap().clone().unwrap_or_else(|| "n/a".into()));
    println!("\n{} check(s) failed", rep.failed);
    if rep.failed > 0 { Err("probe failed".into()) } else { Ok(()) }
}

fn print_get_meta(s: &GitHubStore, expected_len: usize) {
    println!("       expected {expected_len} raw bytes; last GET:");
    println!("       {}", s.last_get_meta.lock().unwrap().clone().unwrap_or_else(|| "n/a".into()));
}

// ---------------------------------------------------------------------------
// demo
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn titles<S: RemoteStore>(d: &Device<S>) -> Vec<String> {
    d.payload()
        .entries
        .iter()
        .map(|e| if e.deleted { format!("[deleted {}]", &e.id.to_string()[..8]) } else { format!("{} <{}>", e.title, e.username) })
        .collect()
}

fn demo(repo: &str, branch: &str) -> AnyResult<()> {
    let s = store(repo, branch, "vault.xpv")?;
    if s.delete("xpass spike: reset demo vault")? {
        println!("removed previous demo vault");
    }
    let base = std::env::temp_dir().join(format!("xpass-sync-demo-{}", std::process::id()));
    let (dir_a, dir_b) = (base.join("device-a"), base.join("device-b"));
    println!("local dirs: {}", base.display());

    // Device A creates the vault and pushes it.
    let (mut a, _recovery) = Device::create(&dir_a, &s, DEMO_PW, DEMO_KDF)?;
    let shared_id = {
        let mut e = Entry::new("GitHub", now_ms());
        e.username = "alice".into();
        let id = e.id;
        a.edit(|p| p.entries.push(e))?;
        id
    };
    println!("A sync: {:?}", a.sync()?);

    // Device B joins with the password.
    let mut b = Device::clone_remote(&dir_b, &s, DEMO_PW)?;
    println!("B cloned: {:?}", titles(&b));

    // Offline edits on both devices, touching the same entry.
    a.edit(|p| {
        let e = p.entries.iter_mut().find(|e| e.id == shared_id).unwrap();
        e.username = "alice-from-A".into();
        e.updated_at = now_ms();
    })?;
    thread::sleep(Duration::from_millis(5));
    b.edit(|p| {
        let e = p.entries.iter_mut().find(|e| e.id == shared_id).unwrap();
        e.username = "alice-from-B (newer)".into();
        e.updated_at = now_ms();
        p.entries.push(Entry::new("Mail (added on B)", now_ms()));
    })?;

    println!("A sync: {:?}", a.sync()?);
    let merged = b.sync()?;
    println!("B sync: {merged:?}");
    println!("A sync: {:?}", a.sync()?);
    let ok1 = merged == SyncOutcome::Merged && titles(&a) == titles(&b);
    println!("A: {:?}\nB: {:?}", titles(&a), titles(&b));
    println!("[{}] sequential conflict merged and converged", if ok1 { "PASS" } else { "FAIL" });

    // Concurrent push: both devices edit and sync at the same time.
    a.edit(|p| p.entries.push(Entry::new("Bank (added on A)", now_ms())))?;
    b.edit(|p| p.entries.push(Entry::new("Cloud (added on B)", now_ms())))?;
    let (ra, rb) = thread::scope(|sc| {
        let ha = sc.spawn(|| a.sync());
        let hb = sc.spawn(|| b.sync());
        (ha.join().unwrap(), hb.join().unwrap())
    });
    println!("concurrent: A {:?}, B {:?}", ra?, rb?);
    let (fa, fb) = (a.sync()?, b.sync()?);
    println!("follow-up: A {fa:?}, B {fb:?}");
    let ok2 = titles(&a) == titles(&b) && a.payload().entries.len() == 4;
    println!("A: {:?}\nB: {:?}", titles(&a), titles(&b));
    println!("[{}] concurrent push converged with all 4 entries", if ok2 { "PASS" } else { "FAIL" });

    let _ = std::fs::remove_dir_all(&base);
    if ok1 && ok2 { Ok(()) } else { Err("demo failed".into()) }
}

// ---------------------------------------------------------------------------
// cleanup
// ---------------------------------------------------------------------------

fn cleanup(repo: &str, branch: &str) -> AnyResult<()> {
    for file in ["probe.bin", "create-probe.bin", "big.bin", "vault.xpv"] {
        let s = store(repo, branch, file)?;
        let deleted = s.delete("xpass spike: cleanup")?;
        println!("{}: {}", s.path(), if deleted { "deleted" } else { "not present" });
    }
    Ok(())
}
