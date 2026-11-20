//! Import errors. No free-form strings: nothing from the file can leak through an error.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("the file is larger than {max_bytes} bytes")]
    TooLarge { max_bytes: usize },
    #[error("the file has more than {max_rows} rows")]
    TooManyRows { max_rows: usize },
    #[error("the file is not UTF-8 text")]
    Encoding,
    #[error("the file is not a valid CSV file (line {line:?})")]
    InvalidCsv { line: Option<u64> },
    #[error("the file has no data rows")]
    Empty,
    #[error("the column mapping refers to column {index}, which does not exist")]
    NoSuchColumn { index: usize },
    #[error("the column mapping maps none of title, username, password or URL")]
    NothingMapped,
}
