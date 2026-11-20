//! Bytes → table of strings: UTF-8 check, delimiter sniffing, size and row limits.

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::ImportError;

/// Largest file accepted (plan I4).
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
/// Most data rows accepted (plan I4).
pub const MAX_ROWS: usize = 10_000;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// A decoded CSV file: the header row and the data rows. Zeroized on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Table {
    /// Header cells, trimmed.
    pub headers: Vec<String>,
    pub rows: Vec<Record>,
}

/// One data row and the line it starts on (1-based, for messages).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Record {
    pub line: u64,
    pub cells: Vec<String>,
}

impl Record {
    /// The cell at `index`, or `""` when the column is unmapped or the row is short.
    pub fn cell(&self, index: Option<usize>) -> &str {
        index.and_then(|i| self.cells.get(i)).map_or("", String::as_str)
    }
}

/// Decodes a CSV file. The first non-empty record is the header row.
pub fn decode(bytes: &[u8]) -> Result<Table, ImportError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge {
            max_bytes: MAX_FILE_BYTES,
        });
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    // UTF-16 files (BOM FF FE / FE FF) are not valid UTF-8 anyway; NUL bytes mean a
    // binary file or UTF-16 without a BOM.
    let text = std::str::from_utf8(bytes).map_err(|_| ImportError::Encoding)?;
    if text.contains('\0') {
        return Err(ImportError::Encoding);
    }

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(sniff_delimiter(text))
        .from_reader(text.as_bytes());

    let mut headers: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    let mut record = csv::StringRecord::new();
    loop {
        match reader.read_record(&mut record) {
            Ok(false) => break,
            Ok(true) => {}
            Err(e) => {
                return Err(ImportError::InvalidCsv {
                    line: e.position().map(csv::Position::line),
                })
            }
        }
        if record.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let line = record.position().map_or(0, csv::Position::line);
        let cells: Vec<String> = record.iter().map(str::to_owned).collect();
        if headers.is_none() {
            headers = Some(cells.iter().map(|h| h.trim().to_owned()).collect());
            continue;
        }
        if rows.len() == MAX_ROWS {
            return Err(ImportError::TooManyRows { max_rows: MAX_ROWS });
        }
        rows.push(Record { line, cells });
    }
    record.clear();

    match headers {
        Some(headers) if !rows.is_empty() => Ok(Table { headers, rows }),
        _ => Err(ImportError::Empty),
    }
}

/// Picks `,`, `;` or tab: whichever occurs most often outside quotes in the first line
/// (ties → `,`).
fn sniff_delimiter(text: &str) -> u8 {
    let mut counts = [0usize; 3];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' | '\r' if !quoted => break,
            ',' if !quoted => counts[0] += 1,
            ';' if !quoted => counts[1] += 1,
            '\t' if !quoted => counts[2] += 1,
            _ => {}
        }
    }
    let best = (0..3).fold(0, |best, i| if counts[i] > counts[best] { i } else { best });
    b",;\t"[best]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(t: &Table) -> Vec<Vec<&str>> {
        t.rows
            .iter()
            .map(|r| r.cells.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn decodes_with_bom_quotes_and_blank_lines() {
        let t = decode(b"\xEF\xBB\xBF name , url\n\n\"a, b\",\"x\ny\"\r\nc,d\n").unwrap();
        assert_eq!(t.headers, ["name", "url"]);
        assert_eq!(cells(&t), [vec!["a, b", "x\ny"], vec!["c", "d"]]);
        assert!(t.rows[0].line > 1 && t.rows[1].line > t.rows[0].line);
    }

    #[test]
    fn sniffs_delimiters() {
        assert_eq!(sniff_delimiter("a;b;\"c,d,e\"\n1,2,3,4"), b';');
        assert_eq!(sniff_delimiter("a\tb\tc"), b'\t');
        assert_eq!(sniff_delimiter("single"), b',');
        let t = decode(b"a;b\n1;2\n").unwrap();
        assert_eq!(cells(&t), [vec!["1", "2"]]);
    }

    #[test]
    fn short_rows_are_padded_by_cell() {
        let t = decode(b"a,b,c\n1\n").unwrap();
        assert_eq!(t.rows[0].cell(Some(0)), "1");
        assert_eq!(t.rows[0].cell(Some(2)), "");
        assert_eq!(t.rows[0].cell(None), "");
    }

    #[test]
    fn limits_and_errors() {
        assert_eq!(decode(b"").err(), Some(ImportError::Empty));
        assert_eq!(decode(b"a,b\n").err(), Some(ImportError::Empty));
        assert_eq!(decode(b"a,b\n,\n").err(), Some(ImportError::Empty));
        assert_eq!(decode(b"name\ncaf\xe9\n").err(), Some(ImportError::Encoding));
        assert_eq!(decode(b"\xFF\xFEa\0,\0").err(), Some(ImportError::Encoding));
        assert_eq!(decode(b"a\0b\n1\n").err(), Some(ImportError::Encoding));

        let big = vec![b'a'; MAX_FILE_BYTES + 1];
        assert!(matches!(decode(&big), Err(ImportError::TooLarge { .. })));

        let mut many = String::from("h\n");
        for i in 0..=MAX_ROWS {
            many.push_str(&format!("{i}\n"));
        }
        assert!(matches!(decode(many.as_bytes()), Err(ImportError::TooManyRows { .. })));
        many.truncate(many.trim_end().rfind('\n').unwrap() + 1);
        assert_eq!(decode(many.as_bytes()).unwrap().rows.len(), MAX_ROWS);
    }
}
