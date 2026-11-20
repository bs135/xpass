//! Small text helpers shared by conversion and duplicate detection.

use url::Url;

/// Host of a URL as typed by people or exported by other managers: a missing scheme
/// means `https://`; any scheme with a host is accepted (e.g. `androidapp://`).
/// Lower-cased, without a leading `www.`.
pub fn host_of(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return None;
    }
    let url = if raw.contains("://") {
        Url::parse(raw).ok()?
    } else {
        Url::parse(&format!("https://{raw}")).ok()?
    };
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").map(str::to_owned).unwrap_or(host);
    (!host.is_empty()).then_some(host)
}

/// Duplicate-detection key (plan I5): host of the first URL with a host, else the
/// lower-cased trimmed title; plus the lower-cased trimmed username.
pub fn match_key(title: &str, username: &str, urls: &[String]) -> (String, String) {
    let site = urls
        .iter()
        .find_map(|u| host_of(u))
        .unwrap_or_else(|| title.trim().to_lowercase());
    (site, username.trim().to_lowercase())
}

/// Trims items, drops empty ones and, with `dedupe`, case-insensitive repeats
/// (same rules as the app's entry form).
pub fn clean_list<'a>(items: impl IntoIterator<Item = &'a str>, dedupe: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items.into_iter().map(str::trim).filter(|s| !s.is_empty()) {
        if !dedupe || !out.iter().any(|o| o.eq_ignore_ascii_case(item)) {
            out.push(item.to_owned());
        }
    }
    out
}

/// `1`, `true`, `yes`, `y`, `x`, `*` (any case) → true.
pub fn is_truthy(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "x" | "*"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(host_of("https://www.Example.com/login").as_deref(), Some("example.com"));
        assert_eq!(host_of("mail.example.com").as_deref(), Some("mail.example.com"));
        assert_eq!(
            host_of("androidapp://com.example.app").as_deref(),
            Some("com.example.app")
        );
        assert_eq!(host_of("   "), None);
        assert_eq!(host_of("not a url"), None);
        assert_eq!(host_of("https://"), None);
    }

    #[test]
    fn keys() {
        let urls = vec![String::new(), "https://www.example.com/a".to_owned()];
        assert_eq!(match_key("X", " Alice ", &urls), ("example.com".into(), "alice".into()));
        assert_eq!(match_key(" My Bank ", "", &[]), ("my bank".into(), String::new()));
    }

    #[test]
    fn lists_and_flags() {
        assert_eq!(clean_list(["Work", " work ", "", "2fa"], true), ["Work", "2fa"]);
        assert_eq!(clean_list(["a", "a"], false), ["a", "a"]);
        assert!(is_truthy(" TRUE ") && is_truthy("1") && !is_truthy("0") && !is_truthy(""));
    }
}
