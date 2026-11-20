//! Import pipeline against the synthetic fixtures (tests/fixtures/README.md).

use std::collections::BTreeSet;

use uuid::Uuid;
use xpass_core::model::{Entry, Payload, HISTORY_NOTE_IMPORT};
use xpass_interop::{
    apply::{apply_import, DuplicateChoice, ImportOptions, ImportSummary},
    convert::{convert, ImportRow, Parsed, RowWarning, SkipReason, TOTP_FIELD},
    csv_export::export_csv,
    decode::decode,
    dedup::{classify, Duplicate, Match},
    preset::{detect, Mapping, Preset, Unmapped},
    time::parse_iso8601,
    ImportError,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
}

/// Decode + detect + convert, like the app does on open.
fn parse(bytes: &[u8]) -> Parsed {
    let table = decode(bytes).expect("decode");
    convert(&table, detect(&table.headers), None).expect("convert")
}

fn field<'a>(row: &'a ImportRow, name: &str) -> Option<&'a xpass_core::model::CustomField> {
    row.fields.iter().find(|f| f.name == name)
}

#[test]
fn bitwarden() {
    let p = parse(&fixture("bitwarden.csv"));
    assert_eq!(p.preset, Preset::Bitwarden);
    assert_eq!(p.rows.len(), 3);
    assert_eq!(
        p.skipped.iter().map(|s| s.reason).collect::<Vec<_>>(),
        [SkipReason::UnsupportedType, SkipReason::Empty]
    );

    let social = &p.rows[0];
    assert_eq!(social.title, "Example Social");
    assert_eq!(social.username, "alice@example.com");
    assert_eq!(social.password, "Sample-Pass-01");
    assert_eq!(
        social.urls,
        ["https://social.example.com/login", "https://m.social.example.com"]
    );
    assert_eq!(social.notes, "Line one\r\nLine two");
    assert_eq!(social.tags, ["Social"]);
    assert!(social.favorite);
    assert_eq!(field(social, "Recovery email").unwrap().value, "alice@example.com");
    assert_eq!(field(social, "PIN").unwrap().value, "0000");
    let totp = field(social, TOTP_FIELD).unwrap();
    assert!(totp.secret && totp.value.starts_with("otpauth://"));

    let note = &p.rows[1];
    assert_eq!(
        (note.title.as_str(), note.notes.as_str()),
        ("Sample secure note", "This is a synthetic note.")
    );
    assert!(note.password.is_empty() && note.urls.is_empty() && note.fields.is_empty());

    let mail = &p.rows[2];
    assert_eq!(mail.urls, ["mail.example.com"]);
    assert!(!mail.favorite);
}

#[test]
fn bitwarden_organization() {
    let p = parse(&fixture("bitwarden-org.csv"));
    assert_eq!(p.preset, Preset::Bitwarden);
    assert_eq!(p.rows[0].tags, ["Team A", "Team B"]);
}

#[test]
fn chrome() {
    let p = parse(&fixture("chrome.csv"));
    assert_eq!(p.preset, Preset::Chrome);
    assert_eq!(p.rows.len(), 2);
    assert_eq!(p.rows[0].title, "Example Shop");
    let forum = &p.rows[1];
    assert_eq!(forum.title, "forum.example.org");
    assert_eq!(forum.warnings, [RowWarning::TitleFromUrl]);
    assert_eq!(forum.notes, "multi\nline note");
    assert!(forum.tags.is_empty());
}

#[test]
fn keepassxc() {
    let p = parse(&fixture("keepassxc.csv"));
    assert_eq!(p.preset, Preset::KeePassXc);
    assert_eq!(p.rows.len(), 3);
    let bank = &p.rows[0];
    assert!(bank.tags.is_empty(), "the root group is not a tag");
    assert_eq!(bank.created_at, parse_iso8601("2024-01-15T08:30:00Z"));
    assert!(bank.fields.is_empty());
    let mail = &p.rows[1];
    assert_eq!(mail.tags, ["Email"]);
    assert_eq!(mail.notes, "Synthetic note");
    assert!(field(mail, TOTP_FIELD).unwrap().secret);
    let git = &p.rows[2];
    assert_eq!(git.created_at, None);
    assert_eq!(git.warnings, [RowWarning::InvalidDate]);
}

#[test]
fn xpass_fixture() {
    let p = parse(&fixture("xpass.csv"));
    assert_eq!(p.preset, Preset::XPass);
    let cloud = &p.rows[0];
    assert_eq!(
        cloud.urls,
        ["https://cloud.example.com", "https://admin.cloud.example.com"]
    );
    assert_eq!(cloud.notes, "Note with \"quotes\"");
    assert_eq!(cloud.tags, ["Work", "2fa"]);
    assert!(cloud.favorite);
    assert_eq!(cloud.created_at, parse_iso8601("2024-05-01T12:00:00Z"));
    assert!(field(cloud, "PIN").unwrap().secret);
    assert!(!field(cloud, "Plan").unwrap().secret);
    assert!(!p.rows[1].favorite && p.rows[1].fields.is_empty());
}

#[test]
fn generic_guess_and_unmapped_columns() {
    let bytes = fixture("generic-semicolon.csv");
    let table = decode(&bytes).unwrap();
    assert_eq!(detect(&table.headers), Preset::Generic);

    let p = convert(&table, Preset::Generic, None).unwrap();
    let mapping = p.mapping.clone().unwrap();
    assert_eq!(
        (mapping.url, mapping.username, mapping.password),
        (Some(0), Some(1), Some(2))
    );
    assert_eq!((mapping.notes, mapping.tags, mapping.title), (Some(3), Some(4), None));
    let row = &p.rows[0];
    assert_eq!(row.title, "news.example.com");
    assert_eq!(row.tags, ["News", "Daily"]);
    assert_eq!(row.notes, "Synthetic comment");

    let to_notes = Mapping {
        unmapped: Unmapped::Notes,
        ..mapping.clone()
    };
    let p = convert(&table, Preset::Generic, Some(&to_notes)).unwrap();
    assert_eq!(p.rows[0].notes, "Synthetic comment\nExtra: keep me");

    let to_fields = Mapping {
        unmapped: Unmapped::Fields,
        ..mapping
    };
    let p = convert(&table, Preset::Generic, Some(&to_fields)).unwrap();
    assert_eq!(field(&p.rows[0], "Extra").unwrap().value, "keep me");
    assert!(p.rows[1].fields.is_empty(), "empty unmapped cells are dropped");

    let bad = Mapping {
        password: Some(9),
        ..Mapping::default()
    };
    assert_eq!(
        convert(&table, Preset::Generic, Some(&bad)).err(),
        Some(ImportError::NoSuchColumn { index: 9 })
    );
}

#[test]
fn bad_encoding() {
    assert_eq!(decode(&fixture("bad-encoding.csv")).err(), Some(ImportError::Encoding));
}

// --- duplicates and apply -------------------------------------------------

const TS: i64 = 1_800_000_000_000;

fn existing(title: &str, username: &str, password: &str, url: &str) -> Entry {
    let mut e = Entry::new(title, 1_000);
    e.username = username.into();
    e.password = password.into();
    if !url.is_empty() {
        e.urls = vec![url.into()];
    }
    e
}

fn row(title: &str, username: &str, password: &str, url: &str) -> ImportRow {
    let mut r = ImportRow::default();
    r.title = title.into();
    r.username = username.into();
    r.password = password.into();
    if !url.is_empty() {
        r.urls = vec![url.into()];
    }
    r
}

fn vault(entries: Vec<Entry>) -> Payload {
    let mut p = Payload::default();
    p.entries = entries;
    p
}

#[test]
fn classifies_against_vault_and_file() {
    let shop = existing("Shop", "Bob", "pw-1", "https://www.shop.example.com/login");
    let shop_id = shop.id;
    let mut gone = existing("Gone", "x", "pw", "gone.example.com");
    gone.delete(2_000);
    let payload = vault(vec![shop, gone]);

    let rows = vec![
        row("Other title", " bob ", "pw-1", "shop.example.com"), // exact (host + user, same pw)
        row("Shop", "bob", "pw-2", "https://shop.example.com"),  // possible (different pw)
        row("Gone", "x", "pw", "gone.example.com"),              // tombstones do not count
        row("New", "eve", "a", ""),                              // new
        row("new", "EVE", "a", ""),                              // exact of row 3
        row("New", "eve", "b", ""),                              // possible of row 3
    ];
    let none = BTreeSet::new();
    assert_eq!(
        classify(&payload, &rows, &none),
        [
            Duplicate::Exact {
                of: Match::Entry { id: shop_id }
            },
            Duplicate::Possible {
                of: Match::Entry { id: shop_id }
            },
            Duplicate::None,
            Duplicate::None,
            Duplicate::Exact {
                of: Match::Row { index: 3 }
            },
            Duplicate::Possible {
                of: Match::Row { index: 3 }
            },
        ]
    );

    // Unticking row 3 makes row 4 the first of its key.
    let excluded = BTreeSet::from([3]);
    let plan = classify(&payload, &rows, &excluded);
    assert_eq!(plan[3], Duplicate::None);
    assert_eq!(plan[4], Duplicate::None);
    assert_eq!(
        plan[5],
        Duplicate::Possible {
            of: Match::Row { index: 4 }
        }
    );
}

fn import(payload: &mut Payload, rows: &[ImportRow], duplicates: DuplicateChoice) -> ImportSummary {
    apply_import(
        payload,
        rows,
        &ImportOptions {
            duplicates,
            excluded: BTreeSet::new(),
        },
        TS,
    )
}

fn live(p: &Payload) -> Vec<&Entry> {
    p.entries.iter().filter(|e| !e.deleted).collect()
}

#[test]
fn apply_skip_new_update() {
    let shop = existing("Shop", "bob", "old", "shop.example.com");
    let shop_id = shop.id;
    let rows = vec![
        row("Shop", "bob", "old", "shop.example.com"),
        row("Shop", "bob", "new", "shop.example.com"),
        row("Fresh", "", "f", ""),
    ];

    let mut p = vault(vec![shop.clone()]);
    let s = import(&mut p, &rows, DuplicateChoice::Skip);
    assert_eq!(
        s,
        ImportSummary {
            added: 1,
            updated: 0,
            skipped: 2
        }
    );
    assert_eq!(live(&p).len(), 2);

    let mut p = vault(vec![shop.clone()]);
    let s = import(&mut p, &rows, DuplicateChoice::New);
    assert_eq!((s.added, s.updated, s.skipped), (2, 0, 1));
    assert_eq!(live(&p).len(), 3);

    let mut p = vault(vec![shop]);
    let s = import(&mut p, &rows, DuplicateChoice::Update);
    assert_eq!((s.added, s.updated, s.skipped), (1, 1, 1));
    let updated = p.entries.iter().find(|e| e.id == shop_id).unwrap();
    assert_eq!(updated.password, "new");
    assert_eq!(updated.updated_at, TS);
    assert_eq!(updated.history.len(), 1);
    assert_eq!(updated.history[0].password, "old");
    assert_eq!(updated.history[0].note.as_deref(), Some(HISTORY_NOTE_IMPORT));
    assert_eq!(updated.history[0].at, TS);
}

#[test]
fn update_chains_within_the_file() {
    let rows = vec![
        row("Site", "u", "one", ""),
        row("Site", "u", "two", ""),
        row("Site", "u", "three", ""),
    ];
    let mut p = Payload::default();
    let s = import(&mut p, &rows, DuplicateChoice::Update);
    assert_eq!((s.added, s.updated, s.skipped), (1, 2, 0));
    let e = &p.entries[0];
    assert_eq!(e.password, "three");
    let old: Vec<&str> = e.history.iter().map(|h| h.password.as_str()).collect();
    assert_eq!(old.len(), 2);
    assert!(old.contains(&"one") && old.contains(&"two"));
}

#[test]
fn new_entries_get_import_timestamp_and_source_creation_date() {
    let mut with_date = row("Dated", "", "p", "");
    with_date.created_at = Some(1_000_000);
    let mut future = row("Future", "", "p", "");
    future.created_at = Some(TS + 1);
    let mut p = Payload::default();
    let excluded_row = row("Excluded", "", "p", "");
    let rows = vec![with_date, future, excluded_row];
    let s = apply_import(
        &mut p,
        &rows,
        &ImportOptions {
            duplicates: DuplicateChoice::Skip,
            excluded: BTreeSet::from([2]),
        },
        TS,
    );
    assert_eq!((s.added, s.skipped), (2, 1));
    let ids: BTreeSet<Uuid> = p.entries.iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), 2, "fresh ids");
    assert!(p.entries.iter().all(|e| e.updated_at == TS && !e.deleted));
    assert_eq!(p.entries[0].created_at, 1_000_000);
    assert_eq!(p.entries[1].created_at, TS, "a future date is not kept");
}

#[test]
fn importing_the_same_file_twice_adds_nothing() {
    let p1 = parse(&fixture("bitwarden.csv"));
    let mut p = Payload::default();
    assert_eq!(import(&mut p, &p1.rows, DuplicateChoice::Skip).added, 3);
    let s = import(&mut p, &p1.rows, DuplicateChoice::Update);
    assert_eq!((s.added, s.updated, s.skipped), (0, 0, 3));
}

// --- export round trip --------------------------------------------------------

#[test]
fn export_then_import_round_trips() {
    let mut src = Payload::default();
    for name in ["bitwarden.csv", "keepassxc.csv", "xpass.csv", "chrome.csv"] {
        let rows = parse(&fixture(name)).rows;
        import(&mut src, &rows, DuplicateChoice::New);
    }
    // A tombstone and history are not exported.
    src.entries[0].history.push(xpass_core::model::HistoryItem {
        at: 5,
        password: "older".into(),
        username: None,
        note: None,
    });
    let mut dead = Entry::new("Deleted", 1);
    dead.delete(2);
    src.entries.push(dead);

    let (bytes, count) = export_csv(&src);
    assert_eq!(count, live(&src).len());
    let back = parse(&bytes);
    assert_eq!(back.preset, Preset::XPass);
    assert!(back.skipped.is_empty());

    let mut dst = Payload::default();
    import(&mut dst, &back.rows, DuplicateChoice::New);
    let key = |e: &&Entry| (e.title.to_lowercase(), e.username.clone(), e.password.clone());
    let mut a = live(&src);
    a.sort_by_key(key);
    let mut b = live(&dst);
    b.sort_by_key(key);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.title, y.title);
        assert_eq!(x.username, y.username);
        assert_eq!(x.password, y.password);
        assert_eq!(x.urls, y.urls);
        assert_eq!(x.notes, y.notes);
        assert_eq!(x.tags, y.tags);
        assert_eq!(x.favorite, y.favorite);
        assert_eq!(x.fields, y.fields);
        assert_eq!(x.created_at, y.created_at);
        assert!(y.history.is_empty());
    }
}

#[test]
fn export_layout() {
    let mut e = Entry::new("Formula", 1_705_307_400_123);
    e.password = "=1+1,\"x\"".into();
    e.urls = vec!["a.example.com".into(), "b.example.com".into()];
    let (bytes, count) = export_csv(&vault(vec![e]));
    assert_eq!(count, 1);
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.starts_with("title,username,password,urls,notes,tags,favorite,created,updated,fields\r\n"));
    assert!(
        text.contains(",\"=1+1,\"\"x\"\"\","),
        "no formula escaping, standard quoting: {text}"
    );
    assert!(text.contains("\"a.example.com\nb.example.com\""));
    assert!(text.contains("2024-01-15T08:30:00.123Z"));
    assert!(text.ends_with(",\r\n"), "empty fields cell");
}
