//! Benchmarks: Argon2id cost per parameter set, and file size / save / unlock
//! time as the vault grows. Run with `cargo run --release --example bench`.

use std::time::{Duration, Instant};

use rand::rngs::OsRng;
use vault_format::{crypto::{self, KdfParams}, model::Entry, Unlock, UnlockedVault};

fn main() {
    println!("== Argon2id (median of 3 runs)");
    println!("{:>8} {:>3} {:>3} {:>10}", "m (MiB)", "t", "p", "time");
    for m_mib in [64u32, 128, 256, 512] {
        for t in [2u32, 3] {
            let params = KdfParams { m_kib: m_mib * 1024, t, p: 4 };
            let time = median(3, || {
                crypto::kek_from_password(b"benchmark password", &[7u8; 16], params).unwrap();
            });
            println!("{m_mib:>8} {t:>3} {:>3} {:>8} ms", 4, time.as_millis());
        }
    }

    println!("\n== Vault size (recovery unlock, so no Argon2 in the timings)");
    println!("{:>8} {:>10} {:>10} {:>10}", "entries", "file", "save", "unlock");
    for n in [0usize, 100, 500, 2000] {
        let (mut v, rec) = UnlockedVault::create(&mut OsRng, b"pw", KdfParams { m_kib: 8 * 1024, t: 1, p: 1 }).unwrap();
        for i in 0..n {
            let mut e = Entry::new(&format!("Account {i} at some-service.example"), 1_700_000_000_000);
            e.username = format!("user{i}@example.com");
            e.password = "Zq8#vT2!mL9$wR4^pX6&".into();
            e.url = format!("https://login.some-service-{i}.example/auth");
            e.notes = "Security questions: first pet = ****; recovery codes stored offline.".into();
            e.tags = vec!["work".into(), "2fa".into()];
            v.payload.entries.push(e);
        }
        let mut file = Vec::new();
        let save = median(3, || file = v.save(&mut OsRng).unwrap());
        let unlock = median(3, || {
            UnlockedVault::unlock(&file, Unlock::Recovery(&rec)).unwrap();
        });
        println!("{n:>8} {:>9.1}K {:>10.1?} {:>10.1?}", file.len() as f64 / 1024.0, save, unlock);
    }
}

fn median(runs: usize, mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}
