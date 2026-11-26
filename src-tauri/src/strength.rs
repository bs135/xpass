//! Password strength estimate (zxcvbn), computed in Rust so the frontend needs no
//! extra npm dependency and the setup gate is enforced on this side too.

use serde::Serialize;

/// Minimum zxcvbn score (0–4) for a master password.
pub const MIN_SCORE: u8 = 3;

/// zxcvbn gets slow on very long inputs; anything this long is strong anyway.
const MAX_CHARS: usize = 100;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Strength {
    /// 0 (very weak) to 4 (very strong).
    pub score: u8,
    /// English feedback from zxcvbn; the UI shows it for English only.
    pub warning: Option<String>,
    pub suggestions: Vec<String>,
}

pub fn estimate(password: &str) -> Strength {
    if password.is_empty() {
        return Strength {
            score: 0,
            warning: None,
            suggestions: Vec::new(),
        };
    }
    let end = password
        .char_indices()
        .nth(MAX_CHARS)
        .map_or(password.len(), |(i, _)| i);
    let entropy = zxcvbn::zxcvbn(&password[..end], &["xpass"]);
    let feedback = entropy.feedback();
    Strength {
        score: entropy.score().into(),
        warning: feedback.and_then(|f| f.warning()).map(|w| w.to_string()),
        suggestions: feedback
            .map(|f| f.suggestions().iter().map(ToString::to_string).collect())
            .unwrap_or_default(),
    }
}

/// Score only, with extra words (an entry's title, username, hosts) that count as
/// guessable (password health, v0.8 plan V8).
pub fn score(password: &str, user_inputs: &[&str]) -> u8 {
    if password.is_empty() {
        return 0;
    }
    let mut inputs = vec!["xpass"];
    inputs.extend_from_slice(user_inputs);
    zxcvbn::zxcvbn(capped(password), &inputs).score().into()
}

/// The first [`MAX_CHARS`] characters.
fn capped(password: &str) -> &str {
    let end = password
        .char_indices()
        .nth(MAX_CHARS)
        .map_or(password.len(), |(i, _)| i);
    &password[..end]
}

pub fn is_strong_enough(password: &str) -> bool {
    estimate(password).score >= MIN_SCORE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_and_strong_passwords() {
        assert_eq!(estimate("").score, 0);
        assert!(!is_strong_enough("password1"));
        assert!(!is_strong_enough("xpass2026"));
        assert!(is_strong_enough("correct horse battery staple"));
        assert!(is_strong_enough("vN7#qL2$zR9!"));
    }

    #[test]
    fn weak_password_gets_feedback() {
        let s = estimate("password");
        assert!(s.score < MIN_SCORE);
        assert!(s.warning.is_some() || !s.suggestions.is_empty());
    }

    #[test]
    fn very_long_input_is_handled() {
        let long = "ab1!".repeat(10_000);
        assert!(estimate(&long).score <= 4);
        let multibyte = "mật-khẩu-".repeat(50);
        assert!(estimate(&multibyte).score <= 4);
    }
}
