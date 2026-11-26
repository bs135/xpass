//! Password health (v0.8 plan V8): weak and reused passwords, by entry id only.
//!
//! The passwords are copied out of the session into `Zeroizing` buffers, so the
//! session lock is not held while zxcvbn runs (about a millisecond per entry), and
//! the copies are wiped when the check ends. The result carries ids, never
//! passwords. Reused means an exact match between live entries; empty passwords
//! and tombstones are ignored.

use std::collections::HashMap;

use serde::Serialize;
use uuid::Uuid;
use xpass_core::model::Payload;
use zeroize::Zeroizing;

use crate::{entries::normalize_url, strength};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    /// zxcvbn score below the master-password threshold.
    pub weak: Vec<Uuid>,
    /// Groups of two or more entries with the same password.
    pub reused: Vec<Vec<Uuid>>,
    pub checked_at: i64,
}

/// What the check needs from one entry.
pub struct Candidate {
    id: Uuid,
    password: Zeroizing<String>,
    /// Title, username and URL hosts: a password built from them scores low.
    inputs: Vec<String>,
}

/// Copies the live entries that have a password (call under the session lock).
pub fn candidates(payload: &Payload) -> Vec<Candidate> {
    payload
        .entries
        .iter()
        .filter(|e| !e.deleted && !e.password.is_empty())
        .map(|e| {
            let hosts: Vec<String> = e
                .urls
                .iter()
                .filter_map(|u| normalize_url(u))
                .filter_map(|u| u.host_str().map(str::to_owned))
                .collect();
            let mut inputs = Vec::new();
            for source in [&e.title, &e.username].into_iter().chain(&hosts) {
                add_words(&mut inputs, source);
            }
            Candidate {
                id: e.id,
                password: Zeroizing::new(e.password.clone()),
                inputs,
            }
        })
        .collect()
}

/// zxcvbn matches whole dictionary words: give it the text, each word in it and the
/// words run together ("Example Bank" → "example bank", "example", "bank",
/// "examplebank"; "examplebank.com" → "examplebank", "com").
fn add_words(out: &mut Vec<String>, source: &str) {
    let lower = source.trim().to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .collect();
    for w in words.iter().copied().chain([lower.as_str()]) {
        if !w.is_empty() && !out.iter().any(|o| o == w) {
            out.push(w.to_owned());
        }
    }
    let joined = words.concat();
    if words.len() > 1 && !out.contains(&joined) {
        out.push(joined);
    }
}

pub fn check(candidates: &[Candidate], now_ms: i64) -> HealthReport {
    let weak = candidates
        .iter()
        .filter(|c| {
            let inputs: Vec<&str> = c.inputs.iter().map(String::as_str).collect();
            strength::score(&c.password, &inputs) < strength::MIN_SCORE
        })
        .map(|c| c.id)
        .collect();

    // Borrowed keys: no further copies of the passwords.
    let mut groups: HashMap<&str, Vec<Uuid>> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for c in candidates {
        let ids = groups.entry(c.password.as_str()).or_default();
        if ids.is_empty() {
            order.push(c.password.as_str());
        }
        ids.push(c.id);
    }
    let reused = order
        .into_iter()
        .filter_map(|p| groups.remove(p).filter(|ids| ids.len() > 1))
        .collect();

    HealthReport {
        weak,
        reused,
        checked_at: now_ms,
    }
}

#[cfg(test)]
mod tests {
    use xpass_core::model::Entry;

    use super::*;

    fn entry(title: &str, username: &str, url: &str, password: &str) -> Entry {
        let mut e = Entry::new(title, 1);
        e.username = username.into();
        e.urls = if url.is_empty() { Vec::new() } else { vec![url.into()] };
        e.password = password.into();
        e
    }

    fn payload(entries: Vec<Entry>) -> Payload {
        let mut p = Payload::default();
        p.entries = entries;
        p
    }

    #[test]
    fn finds_weak_and_reused_passwords() {
        let strong = "vN7#qL2$zR9!kP4&";
        let p = payload(vec![
            entry("GitHub", "alice", "github.com", strong),
            entry("Mail", "alice", "mail.example.com", "password1"),
            entry("Bank", "alice", "bank.example.com", strong),
            entry("Forum", "bob", "", "password1"),
            entry("Shop", "carol", "", "Tr0ub4dor&3-horse-staple"),
        ]);
        let ids: Vec<Uuid> = p.entries.iter().map(|e| e.id).collect();
        let r = check(&candidates(&p), 42);
        assert_eq!(r.weak, [ids[1], ids[3]]);
        assert_eq!(r.reused, [vec![ids[0], ids[2]], vec![ids[1], ids[3]]]);
        assert_eq!(r.checked_at, 42);
    }

    #[test]
    fn a_password_made_from_the_entry_is_weak() {
        // Strong on its own, but it is the site name and the username.
        let pw = "examplebankdorothea";
        assert!(strength::score(pw, &[]) >= strength::MIN_SCORE, "premise");
        let p = payload(vec![entry("Example Bank", "dorothea", "https://examplebank.com", pw)]);
        assert_eq!(check(&candidates(&p), 0).weak, [p.entries[0].id]);
    }

    #[test]
    fn user_inputs_are_split_into_words() {
        let mut out = Vec::new();
        add_words(&mut out, " Example Bank ");
        add_words(&mut out, "alice.smith@example.com");
        assert_eq!(
            out,
            [
                "example",
                "bank",
                "example bank",
                "examplebank",
                "alice",
                "smith",
                "com",
                "alice.smith@example.com",
                "alicesmithexamplecom"
            ]
        );
    }

    #[test]
    fn tombstones_and_empty_passwords_are_ignored() {
        let mut gone = entry("Old", "", "", "password1");
        gone.delete(5);
        gone.password = "password1".into(); // a tombstone with content must still be skipped
        let p = payload(vec![
            entry("A", "", "", ""),
            entry("B", "", "", ""),
            gone,
            entry("C", "", "", "password1"),
        ]);
        let r = check(&candidates(&p), 0);
        assert_eq!(r.weak, [p.entries[3].id]);
        assert!(r.reused.is_empty());
    }

    #[test]
    fn the_report_carries_no_password() {
        let p = payload(vec![
            entry("A", "", "", "hunter2-hunter2"),
            entry("B", "", "", "hunter2-hunter2"),
        ]);
        let json = serde_json::to_string(&check(&candidates(&p), 0)).unwrap();
        assert!(!json.contains("hunter2"), "{json}");
        assert!(json.contains("\"weak\"") && json.contains("\"reused\"") && json.contains("\"checkedAt\""));
    }

    /// `cargo test --release -p xpass health_timing -- --ignored --nocapture`
    #[test]
    #[ignore = "timing, release build"]
    fn health_timing() {
        let entries = (0..2000)
            .map(|i| {
                entry(
                    &format!("Site {i}"),
                    &format!("user{i}@example.com"),
                    &format!("https://site{i}.example.com"),
                    &format!("Pw-{i:04}-{}", "x".repeat(i % 24)),
                )
            })
            .collect();
        let p = payload(entries);
        let started = std::time::Instant::now();
        let r = check(&candidates(&p), 0);
        println!("2000 entries: {:?}, weak {}", started.elapsed(), r.weak.len());
    }
}
