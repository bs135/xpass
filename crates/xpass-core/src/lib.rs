//! xpass core: vault file format, crypto, data model, merge and sync engine.
//!
//! This crate has no UI or OS dependencies so the security-critical code can be
//! unit-tested and fuzzed on any platform. Modules are ported from `spikes/` in
//! tasks T2 (format, crypto, model) and T3 (merge, clock, maintenance).

/// Current vault file format version.
pub const FORMAT_VERSION: u16 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn format_version_is_one() {
        assert_eq!(super::FORMAT_VERSION, 1);
    }
}
