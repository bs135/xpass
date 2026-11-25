//! Time-based one-time passwords (RFC 6238 over RFC 4226), v0.8 plan V2 / V3.
//!
//! A TOTP secret is stored in an entry as a secret custom field, the way imports
//! already store it: an `otpauth://totp/…` URI, or a bare base32 secret in a
//! field named `TOTP` / `OTP` (KeePassXC). Nothing about the stored shape changes.
//!
//! - [`Totp::parse`] reads a URI or a base32 secret (spaces, dashes, lower case and
//!   missing padding allowed). HOTP (counter-based, it would need a write per code)
//!   and Steam codes are refused.
//! - [`Totp::code_at`] computes the code for a Unix time in milliseconds.
//! - [`Totp::to_uri`] writes the normalized URI a new TOTP field is stored as.
//!
//! The decoded secret lives in a `Zeroizing` buffer and is never formatted.

use std::fmt;

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

use crate::model::CustomField;

pub const DEFAULT_DIGITS: u8 = 6;
pub const DEFAULT_PERIOD: u32 = 30;
pub const MIN_DIGITS: u8 = 6;
pub const MAX_DIGITS: u8 = 8;
pub const MIN_PERIOD: u32 = 15;
pub const MAX_PERIOD: u32 = 120;
/// Decoded secret length in bytes (RFC 4226 asks for at least 128 bits; 80 is the
/// shortest seen in the wild, e.g. 16 base32 characters).
pub const MIN_SECRET: usize = 10;
pub const MAX_SECRET: usize = 128;
/// Longer inputs are refused before parsing.
pub const MAX_INPUT: usize = 2048;

/// Field names that mark a bare base32 value as a TOTP secret (any case).
pub const FIELD_NAMES: [&str; 2] = ["TOTP", "OTP"];

const SCHEME: &str = "otpauth://";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha512 => "SHA512",
        }
    }
}

/// Why a value is not a usable TOTP secret. Carries no part of the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OtpError {
    #[error("counter-based (HOTP) one-time passwords are not supported")]
    Hotp,
    #[error("this kind of one-time password is not supported")]
    Unsupported,
    #[error("not a valid otpauth URI")]
    BadUri,
    #[error("the secret is not valid base32")]
    BadSecret,
    #[error("the secret is too short or too long")]
    SecretLength,
    #[error("unsupported algorithm")]
    Algorithm,
    #[error("digits must be between 6 and 8")]
    Digits,
    #[error("period must be between 15 and 120 seconds")]
    Period,
}

/// A parsed TOTP secret with its parameters.
#[derive(Clone, PartialEq, Eq)]
pub struct Totp {
    secret: Zeroizing<Vec<u8>>,
    pub algorithm: Algorithm,
    pub digits: u8,
    pub period: u32,
    pub issuer: Option<String>,
    pub account: Option<String>,
}

impl fmt::Debug for Totp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Totp")
            .field("secret", &"<redacted>")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .field("issuer", &self.issuer)
            .field("account", &self.account)
            .finish()
    }
}

/// A code and how long it stays valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    /// Zero-padded to the configured number of digits.
    pub code: Zeroizing<String>,
    pub period: u32,
    /// Milliseconds until the next code.
    pub remaining_ms: u64,
}

impl Totp {
    /// Reads an `otpauth://totp/…` URI or a bare base32 secret.
    pub fn parse(input: &str) -> Result<Self, OtpError> {
        let input = input.trim();
        if input.len() > MAX_INPUT {
            return Err(OtpError::BadUri);
        }
        if starts_with_ignore_case(input, SCHEME) {
            parse_uri(input)
        } else {
            Ok(Self::from_secret(decode_base32(input)?))
        }
    }

    /// A secret with the default parameters (SHA-1, 6 digits, 30 s).
    fn from_secret(secret: Zeroizing<Vec<u8>>) -> Self {
        Self {
            secret,
            algorithm: Algorithm::Sha1,
            digits: DEFAULT_DIGITS,
            period: DEFAULT_PERIOD,
            issuer: None,
            account: None,
        }
    }

    /// Code for Unix time `now_ms` (times before 1970 count as 0).
    pub fn code_at(&self, now_ms: i64) -> Code {
        let now_ms = u64::try_from(now_ms).unwrap_or(0);
        let period_ms = u64::from(self.period) * 1000;
        let counter = now_ms / period_ms;
        Code {
            code: hotp(&self.secret, counter, self.algorithm, self.digits),
            period: self.period,
            remaining_ms: period_ms - now_ms % period_ms,
        }
    }

    /// Normalized URI: `otpauth://totp/<label>?secret=…[&issuer=…]`, plus
    /// `algorithm`, `digits` and `period` only when they are not the defaults.
    pub fn to_uri(&self) -> Zeroizing<String> {
        // A literal colon separates issuer and account; colons inside them are encoded.
        let label = match (&self.issuer, &self.account) {
            (Some(i), Some(a)) => format!("{}:{}", encode(i), encode(a)),
            (None, Some(a)) if a.contains(':') => format!(":{}", encode(a)),
            (None, Some(a)) => encode(a),
            (Some(i), None) => format!("{}:", encode(i)),
            // Empty, so the URI reads back without an account it never had.
            (None, None) => String::new(),
        };
        let mut uri = Zeroizing::new(format!(
            "{SCHEME}totp/{label}?secret={}",
            BASE32_NOPAD.encode(&self.secret)
        ));
        if let Some(issuer) = &self.issuer {
            uri.push_str("&issuer=");
            uri.push_str(&encode(issuer));
        }
        if self.algorithm != Algorithm::Sha1 {
            uri.push_str("&algorithm=");
            uri.push_str(self.algorithm.name());
        }
        if self.digits != DEFAULT_DIGITS {
            uri.push_str(&format!("&digits={}", self.digits));
        }
        if self.period != DEFAULT_PERIOD {
            uri.push_str(&format!("&period={}", self.period));
        }
        uri
    }
}

/// A custom field that holds a TOTP secret (V2): secret, and either an
/// `otpauth://totp/` URI or a base32 value in a field named `TOTP` / `OTP`.
pub fn is_totp_field(field: &CustomField) -> bool {
    if !field.secret {
        return false;
    }
    let value = field.value.trim();
    let named = FIELD_NAMES.iter().any(|n| field.name.trim().eq_ignore_ascii_case(n));
    if starts_with_ignore_case(value, SCHEME) || named {
        Totp::parse(value).is_ok()
    } else {
        false
    }
}

/// RFC 4226 HOTP value with dynamic truncation.
fn hotp(secret: &[u8], counter: u64, algorithm: Algorithm, digits: u8) -> Zeroizing<String> {
    let msg = counter.to_be_bytes();
    let mac: Zeroizing<Vec<u8>> = Zeroizing::new(match algorithm {
        Algorithm::Sha1 => mac::<Hmac<Sha1>>(secret, &msg),
        Algorithm::Sha256 => mac::<Hmac<Sha256>>(secret, &msg),
        Algorithm::Sha512 => mac::<Hmac<Sha512>>(secret, &msg),
    });
    let offset = usize::from(mac[mac.len() - 1] & 0x0f);
    let binary = u32::from_be_bytes([mac[offset], mac[offset + 1], mac[offset + 2], mac[offset + 3]]) & 0x7fff_ffff;
    let value = binary % 10u32.pow(u32::from(digits));
    Zeroizing::new(format!("{value:0width$}", width = usize::from(digits)))
}

fn mac<M: Mac + hmac::digest::KeyInit>(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = <M as Mac>::new_from_slice(key).expect("HMAC takes keys of any length");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

fn decode_base32(input: &str) -> Result<Zeroizing<Vec<u8>>, OtpError> {
    let cleaned: Zeroizing<String> = Zeroizing::new(
        input
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '\t'))
            .map(|c| c.to_ascii_uppercase())
            .collect(),
    );
    let cleaned = cleaned.trim_end_matches('=');
    if cleaned.is_empty() {
        return Err(OtpError::BadSecret);
    }
    // NOPAD refuses a length that cannot be produced by an encoder (e.g. 1, 3, 6
    // trailing characters), which catches truncated secrets.
    let bytes = Zeroizing::new(
        BASE32_NOPAD
            .decode(cleaned.as_bytes())
            .map_err(|_| OtpError::BadSecret)?,
    );
    if !(MIN_SECRET..=MAX_SECRET).contains(&bytes.len()) {
        return Err(OtpError::SecretLength);
    }
    Ok(bytes)
}

fn parse_uri(input: &str) -> Result<Totp, OtpError> {
    let rest = &input[SCHEME.len()..];
    let (kind, rest) = rest.split_once('/').ok_or(OtpError::BadUri)?;
    if kind.eq_ignore_ascii_case("hotp") {
        return Err(OtpError::Hotp);
    }
    if !kind.eq_ignore_ascii_case("totp") {
        return Err(OtpError::Unsupported);
    }
    let (label, query) = rest.split_once('?').unwrap_or((rest, ""));
    // Key URI format: `issuer:account`, the separator a literal or an encoded colon.
    // A literal one wins, so an account with an encoded colon survives `to_uri`.
    let (label_issuer, account) = match split_label(label) {
        Some((i, a)) => (non_empty(&decode(i)?), non_empty(&decode(a)?)),
        None => (None, non_empty(&decode(label)?)),
    };

    let mut secret = None;
    let mut issuer = None;
    let mut algorithm = Algorithm::Sha1;
    let mut digits = DEFAULT_DIGITS;
    let mut period = DEFAULT_PERIOD;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = decode(value)?;
        match key.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(decode_base32(&value)?),
            "issuer" => issuer = non_empty(&value),
            "algorithm" => {
                algorithm = match value.to_ascii_uppercase().as_str() {
                    "SHA1" => Algorithm::Sha1,
                    "SHA256" => Algorithm::Sha256,
                    "SHA512" => Algorithm::Sha512,
                    _ => return Err(OtpError::Algorithm),
                }
            }
            "digits" => {
                digits = value.parse().map_err(|_| OtpError::Digits)?;
                if !(MIN_DIGITS..=MAX_DIGITS).contains(&digits) {
                    return Err(OtpError::Digits);
                }
            }
            "period" => {
                period = value.parse().map_err(|_| OtpError::Period)?;
                if !(MIN_PERIOD..=MAX_PERIOD).contains(&period) {
                    return Err(OtpError::Period);
                }
            }
            // Steam Guard codes use their own alphabet.
            "encoder" => return Err(OtpError::Unsupported),
            _ => {}
        }
    }
    let mut totp = Totp::from_secret(secret.ok_or(OtpError::BadSecret)?);
    totp.algorithm = algorithm;
    totp.digits = digits;
    totp.period = period;
    totp.issuer = issuer.or(label_issuer);
    totp.account = account;
    Ok(totp)
}

fn split_label(label: &str) -> Option<(&str, &str)> {
    if let Some(split) = label.split_once(':') {
        return Some(split);
    }
    let at = label
        .as_bytes()
        .windows(3)
        .position(|w| w.eq_ignore_ascii_case(b"%3A"))?;
    Some((&label[..at], &label[at + 3..]))
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Percent-decoding (`+` stays a plus: otpauth URIs use `%20` for spaces).
fn decode(s: &str) -> Result<String, OtpError> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or(OtpError::BadUri)?;
            let hex = std::str::from_utf8(hex).map_err(|_| OtpError::BadUri)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| OtpError::BadUri)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| OtpError::BadUri)
}

/// Percent-encoding of everything but unreserved characters (RFC 3986).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA1_KEY: &[u8] = b"12345678901234567890";
    const SHA256_KEY: &[u8] = b"12345678901234567890123456789012";
    const SHA512_KEY: &[u8] = b"1234567890123456789012345678901234567890123456789012345678901234";

    fn totp(key: &[u8], algorithm: Algorithm, digits: u8) -> Totp {
        let mut t = Totp::from_secret(Zeroizing::new(key.to_vec()));
        t.algorithm = algorithm;
        t.digits = digits;
        t
    }

    #[test]
    fn rfc4226_hotp_vectors() {
        let expected = [
            "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871", "520489",
        ];
        for (counter, want) in expected.iter().enumerate() {
            assert_eq!(
                *hotp(SHA1_KEY, counter as u64, Algorithm::Sha1, 6),
                *want,
                "counter {counter}"
            );
        }
    }

    #[test]
    fn rfc6238_totp_vectors() {
        // RFC 6238 Appendix B: (time, SHA-1, SHA-256, SHA-512), 8 digits.
        let table: [(i64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1_111_111_109, "07081804", "68084774", "25091201"),
            (1_111_111_111, "14050471", "67062674", "99943326"),
            (1_234_567_890, "89005924", "91819424", "93441116"),
            (2_000_000_000, "69279037", "90698825", "38618901"),
            (20_000_000_000, "65353130", "77737706", "47863826"),
        ];
        for (t, sha1, sha256, sha512) in table {
            let ms = t * 1000;
            assert_eq!(
                *totp(SHA1_KEY, Algorithm::Sha1, 8).code_at(ms).code,
                sha1,
                "SHA-1 at {t}"
            );
            assert_eq!(
                *totp(SHA256_KEY, Algorithm::Sha256, 8).code_at(ms).code,
                sha256,
                "SHA-256 at {t}"
            );
            assert_eq!(
                *totp(SHA512_KEY, Algorithm::Sha512, 8).code_at(ms).code,
                sha512,
                "SHA-512 at {t}"
            );
        }
    }

    #[test]
    fn remaining_time_and_period() {
        let t = totp(SHA1_KEY, Algorithm::Sha1, 6);
        let c = t.code_at(59_000);
        assert_eq!((c.period, c.remaining_ms), (30, 1_000));
        assert_eq!(t.code_at(60_000).remaining_ms, 30_000);
        assert_eq!(t.code_at(-5).code, t.code_at(0).code, "negative time counts as 0");
        assert_eq!(t.code_at(0).code.len(), 6);
    }

    #[test]
    fn parses_uris() {
        let t = Totp::parse(
            "otpauth://totp/ACME%20Co:alice@example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=ACME%20Co&algorithm=SHA256&digits=8&period=60",
        )
        .unwrap();
        assert_eq!(*t.secret, SHA1_KEY);
        assert_eq!(t.algorithm, Algorithm::Sha256);
        assert_eq!((t.digits, t.period), (8, 60));
        assert_eq!(t.issuer.as_deref(), Some("ACME Co"));
        assert_eq!(t.account.as_deref(), Some("alice@example.com"));

        // Issuer only in the label, lower-case scheme and keys, defaults.
        let t = Totp::parse("OTPAUTH://TOTP/Example:bob?SECRET=gezdgnbvgy3tqojqgezdgnbvgy3tqojq").unwrap();
        assert_eq!(t.issuer.as_deref(), Some("Example"));
        assert_eq!((t.algorithm, t.digits, t.period), (Algorithm::Sha1, 6, 30));
    }

    #[test]
    fn parses_bare_base32() {
        for input in [
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
            "gezd gnbv gy3t qojq gezd gnbv gy3t qojq",
            "GEZD-GNBV-GY3T-QOJQ-GEZD-GNBV-GY3T-QOJQ",
            " GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ==== ",
        ] {
            assert_eq!(*Totp::parse(input).unwrap().secret, SHA1_KEY, "{input}");
        }
    }

    #[test]
    fn refuses_what_it_cannot_use() {
        let cases = [
            ("otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQ&counter=1", OtpError::Hotp),
            (
                "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&encoder=steam",
                OtpError::Unsupported,
            ),
            ("otpauth://yubi/x?secret=GEZDGNBVGY3TQOJQ", OtpError::Unsupported),
            ("otpauth://totp", OtpError::BadUri),
            ("otpauth://totp/x?issuer=a", OtpError::BadSecret),
            ("otpauth://totp/x%zz?secret=GEZDGNBVGY3TQOJQ", OtpError::BadUri),
            (
                "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&algorithm=MD5",
                OtpError::Algorithm,
            ),
            ("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=5", OtpError::Digits),
            ("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=x", OtpError::Digits),
            ("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&period=0", OtpError::Period),
            ("not base32!", OtpError::BadSecret),
            ("", OtpError::BadSecret),
            ("GEZDGNBV", OtpError::SecretLength),
            ("GEZDGNBVG", OtpError::BadSecret),
        ];
        for (input, want) in cases {
            assert_eq!(Totp::parse(input).unwrap_err(), want, "{input}");
        }
        assert_eq!(Totp::parse(&"A".repeat(MAX_INPUT + 1)).unwrap_err(), OtpError::BadUri);
        assert_eq!(Totp::parse(&"A".repeat(240)).unwrap_err(), OtpError::SecretLength);
    }

    #[test]
    fn uri_round_trip() {
        let mut t = totp(SHA1_KEY, Algorithm::Sha1, 6);
        assert_eq!(Totp::parse(&t.to_uri()).unwrap(), t, "no issuer, no account");
        assert_eq!(*t.to_uri(), "otpauth://totp/?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        t.issuer = Some("ACME Co".into());
        t.account = Some("alice@example.com".into());
        t.algorithm = Algorithm::Sha512;
        t.digits = 8;
        t.period = 60;
        let uri = t.to_uri();
        assert_eq!(
            *uri,
            "otpauth://totp/ACME%20Co:alice%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=ACME%20Co&algorithm=SHA512&digits=8&period=60"
        );
        assert_eq!(Totp::parse(&uri).unwrap(), t);

        t.account = None;
        assert_eq!(Totp::parse(&t.to_uri()).unwrap(), t, "issuer without account");
        t.issuer = None;
        t.account = Some("a:b".into());
        assert_eq!(Totp::parse(&t.to_uri()).unwrap(), t, "colon in the account");
    }

    #[test]
    fn totp_fields() {
        let field = |name: &str, value: &str, secret: bool| CustomField {
            name: name.into(),
            value: value.into(),
            secret,
        };
        let b32 = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let uri = format!("otpauth://totp/x?secret={b32}");
        assert!(is_totp_field(&field("TOTP", b32, true)));
        assert!(is_totp_field(&field(" otp ", b32, true)));
        assert!(is_totp_field(&field("Login code", &uri, true)));
        assert!(!is_totp_field(&field("TOTP", b32, false)), "not secret");
        assert!(
            !is_totp_field(&field("Recovery code", b32, true)),
            "base32 under another name"
        );
        assert!(!is_totp_field(&field("TOTP", "not a secret", true)));
        assert!(!is_totp_field(&field(
            "TOTP",
            "otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQ",
            true
        )));
    }

    #[test]
    fn debug_hides_the_secret() {
        let text = format!("{:?}", totp(SHA1_KEY, Algorithm::Sha1, 6));
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("49, 50"), "{text}");
    }
}
