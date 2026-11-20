//! xPass CSV export (plan I8). Plaintext: the app shows a warning and asks to confirm.
//!
//! Layout: [`XPASS_HEADERS`]; `urls` and `tags` are newline-separated inside the
//! quoted cell, dates are ISO 8601 UTC, `fields` is a JSON array of
//! `{ name, value, secret }` (empty when there are none). History and tombstones are
//! not exported. No formula escaping: a leading `'` would corrupt passwords that start
//! with `=`, `+`, `-` or `@`; the app warns not to open the file in a spreadsheet app.

use xpass_core::model::{Entry, Payload};
use zeroize::Zeroizing;

pub use crate::preset::XPASS_HEADERS;
use crate::time::format_iso8601;

/// The CSV bytes (UTF-8, no BOM, CRLF line ends) and the number of entries written.
pub fn export_csv(payload: &Payload) -> (Zeroizing<Vec<u8>>, usize) {
    let mut live: Vec<&Entry> = payload.entries.iter().filter(|e| !e.deleted).collect();
    live.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });

    // Reserve generously so the buffer is not reallocated (old copies would not be wiped).
    let estimate: usize = live
        .iter()
        .map(|e| 256 + e.title.len() + e.notes.len() + e.urls.iter().map(String::len).sum::<usize>())
        .sum::<usize>()
        * 2;
    let mut out = Zeroizing::new(Vec::with_capacity(estimate));
    {
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::CRLF)
            .from_writer(&mut *out);
        // Writing into a Vec cannot fail; an error would be a bug in this function.
        writer.write_record(XPASS_HEADERS).expect("write to Vec");
        for e in &live {
            let fields = if e.fields.is_empty() {
                Zeroizing::new(String::new())
            } else {
                Zeroizing::new(serde_json::to_string(&e.fields).expect("serialize custom fields"))
            };
            let urls = Zeroizing::new(e.urls.join("\n"));
            let tags = e.tags.join("\n");
            let created = format_iso8601(e.created_at);
            let updated = format_iso8601(e.updated_at);
            writer
                .write_record([
                    e.title.as_str(),
                    &e.username,
                    &e.password,
                    &urls,
                    &e.notes,
                    &tags,
                    if e.favorite { "true" } else { "false" },
                    &created,
                    &updated,
                    &fields,
                ])
                .expect("write to Vec");
        }
        writer.flush().expect("write to Vec");
    }
    (out, live.len())
}
