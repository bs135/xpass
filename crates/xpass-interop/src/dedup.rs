//! Duplicate detection (plan I5).
//!
//! Key = (host of the first URL, else the lower-cased title; lower-cased username).
//! A row is compared with the live entries of the vault first, then with the earlier
//! rows of the same file that are not excluded:
//! - same key and same password → [`Duplicate::Exact`] (never imported);
//! - same key, different password → [`Duplicate::Possible`] (user's choice);
//! - otherwise [`Duplicate::None`].

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;
use uuid::Uuid;
use xpass_core::model::Payload;

use crate::{convert::ImportRow, text::match_key};

/// What a duplicate row matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum Match {
    /// A live entry of the vault.
    Entry { id: Uuid },
    /// An earlier row of the file (index into the rows).
    Row { index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Duplicate {
    None,
    Exact { of: Match },
    Possible { of: Match },
}

/// Duplicate status of every row; excluded rows are [`Duplicate::None`] and are not
/// matched against.
pub fn classify(payload: &Payload, rows: &[ImportRow], excluded: &BTreeSet<usize>) -> Vec<Duplicate> {
    let mut vault: HashMap<(String, String), Vec<(Uuid, &str)>> = HashMap::new();
    for e in payload.entries.iter().filter(|e| !e.deleted) {
        vault
            .entry(match_key(&e.title, &e.username, &e.urls))
            .or_default()
            .push((e.id, e.password.as_str()));
    }

    let mut earlier: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut out = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        if excluded.contains(&index) {
            out.push(Duplicate::None);
            continue;
        }
        let key = match_key(&row.title, &row.username, &row.urls);
        let in_vault = vault.get(&key).map_or(&[][..], Vec::as_slice);
        let in_file = earlier.get(&key).map_or(&[][..], Vec::as_slice);

        let exact = in_vault
            .iter()
            .find(|(_, pw)| *pw == row.password)
            .map(|(id, _)| Match::Entry { id: *id })
            .or_else(|| {
                in_file
                    .iter()
                    .find(|i| rows[**i].password == row.password)
                    .map(|i| Match::Row { index: *i })
            });
        let possible = in_vault
            .first()
            .map(|(id, _)| Match::Entry { id: *id })
            .or_else(|| in_file.first().map(|i| Match::Row { index: *i }));

        out.push(match (exact, possible) {
            (Some(of), _) => Duplicate::Exact { of },
            (None, Some(of)) => Duplicate::Possible { of },
            (None, None) => Duplicate::None,
        });
        earlier.entry(key).or_default().push(index);
    }
    out
}
