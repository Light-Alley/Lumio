//! Small security helpers shared by several modules.

use anyhow::{anyhow, Result};

/// `n` random bytes from the operating system, as lower-case hex. Fails (rather than
/// falling back to something guessable) when the system has no random source.
pub fn random_hex(n: usize) -> Result<String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| anyhow!("no secure random numbers available: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Compares two byte strings in time that depends only on their lengths.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A URL for the log: scheme, host and path only, so tokens in the query never reach it.
pub fn redact_url(u: &str) -> String {
    match reqwest::Url::parse(u) {
        Ok(mut url) => {
            let had_query = url.query().is_some();
            url.set_query(None);
            url.set_fragment(None);
            let _ = url.set_username("");
            let _ = url.set_password(None);
            let mut s = url.to_string();
            if had_query {
                s.push_str("?…");
            }
            s
        }
        Err(_) => u.split(['?', '#']).next().unwrap_or("").to_string(),
    }
}

/// Error text from a server, shortened and with anything that looks like a token removed.
pub fn redact_body(body: &str) -> String {
    let mut out = String::new();
    for word in body.split_inclusive(char::is_whitespace) {
        let w = word.trim();
        let secretish = w.len() >= 24 && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_=.+/".contains(c));
        let key_value = ["oauth_token", "oauth_token_secret", "access_token", "api_key", "secret"]
            .iter()
            .any(|k| w.to_ascii_lowercase().contains(k) && w.contains('='));
        if secretish || key_value {
            out.push_str("[redacted]");
            out.push_str(&word[word.trim_end().len()..]);
        } else {
            out.push_str(word);
        }
        if out.len() > 300 {
            out.truncate((0..=300).rev().find(|i| out.is_char_boundary(*i)).unwrap_or(0));
            out.push('…');
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_hex_is_random_and_sized() {
        let a = random_hex(16).unwrap();
        let b = random_hex(16).unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn urls_lose_their_query() {
        assert_eq!(
            redact_url("https://api.smugmug.com/api/v2/x?oauth_token=abc&x=1"),
            "https://api.smugmug.com/api/v2/x?…"
        );
        assert_eq!(redact_url("https://u:p@h.example/a"), "https://h.example/a");
        assert_eq!(redact_url("not a url?token=1"), "not a url");
    }

    #[test]
    fn bodies_lose_tokens() {
        let r = redact_body("oauth_token=abcdef bad request 0123456789abcdef0123456789abcdef");
        assert!(!r.contains("abcdef0123"));
        assert!(!r.contains("oauth_token=abcdef"));
        assert!(r.contains("bad request"));
        assert!(redact_body(&"x ".repeat(400)).len() < 320);
    }
}
