//! Secret policy (decision D23, security §4): a deliberately small,
//! deterministic, dependency-free scanner.
//!
//! ```text
//! high-confidence pattern  → hard refusal (no state change)
//! label + value proximity  → warn, store anyway
//! label alone              → store silently
//! ```
//!
//! Never redact, never mangle. This is *not* a security boundary — the real
//! boundary is 0600/0700 permissions and machine control (security §4.3):
//! unknown secret shapes WILL pass, and users are told injected memory is
//! sent to the provider.

/// Scanner verdict for a content string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretVerdict {
    /// Nothing suspicious.
    None,
    /// High-confidence secret shape — refuse (NO STATE CHANGE).
    Refused,
    /// Warning-only: store, but surface the warning.
    Warn(String),
}

/// Scan content against the D23 pattern set. Pure, deterministic, std-only.
pub fn scan(content: &str) -> SecretVerdict {
    if refusal(content).is_some() {
        return SecretVerdict::Refused;
    }
    if let Some(message) = warn(content) {
        return SecretVerdict::Warn(message);
    }
    SecretVerdict::None
}

/// High-confidence refusal set (security §4.1). `Some(reason)` = refuse.
pub fn refusal(content: &str) -> Option<&'static str> {
    // Private-key PEM armor.
    if content.contains("-----BEGIN ") && content.contains(" PRIVATE KEY-----") {
        return Some("PEM private-key armor");
    }
    // Provider API keys: sk-… / pk-… with 20+ non-space chars after the
    // prefix (OpenAI/Anthropic canonical shapes).
    for prefix in ["sk-", "pk-"] {
        if let Some(rest) = after_prefix(content, prefix) {
            if rest.chars().take_while(|c| !c.is_whitespace()).count() >= 20 {
                return Some("provider API key");
            }
        }
    }
    // GitHub tokens: ghp_… / github_pat_… (long, distinctive).
    for prefix in ["ghp_", "github_pat_"] {
        if let Some(rest) = after_prefix(content, prefix) {
            if rest.chars().take_while(|c| !c.is_whitespace()).count() >= 20 {
                return Some("GitHub token");
            }
        }
    }
    // AWS access key ids: AKIA + 16 uppercase alphanumerics.
    if let Some(rest) = after_prefix(content, "AKIA") {
        if rest
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            .count()
            >= 16
        {
            return Some("AWS access key id");
        }
    }
    // Slack tokens: xox[baprs]-… (distinctive shape, then more).
    for prefix in ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"] {
        if let Some(rest) = after_prefix(content, prefix) {
            if rest.chars().take_while(|c| !c.is_whitespace()).count() >= 10 {
                return Some("Slack token");
            }
        }
    }
    None
}

/// Warning-only label/value proximity (security §4.2). `Some(message)` =
/// warn. High false-positive rate is expected and accepted.
fn warn(content: &str) -> Option<String> {
    let labels = [
        "password", "passwd", "pwd", "token", "secret", "api_key", "apikey", "api-key", "auth",
    ];
    let lower = content.to_lowercase();
    let mut found = None;
    for label in labels {
        if let Some(index) = lower.find(label) {
            let after = &lower[index + label.len()..];
            let separator = after
                .chars()
                .next()
                .map(|c| c == ':' || c == '=' || c == ' ')
                .unwrap_or(false);
            // label followed by `:`/`=` (or a space then `:`/`=` after a
            // value-start) with a non-trivial value.
            let value = if after.starts_with(':') || after.starts_with('=') {
                Some(&after[1..])
            } else if separator {
                // "password is hunter2" — skip non-value connector words.
                let words = after.split_whitespace().collect::<Vec<_>>();
                match words.as_slice() {
                    [connector, value, ..] if *connector == "is" || *connector == "=" => {
                        Some(*value)
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(value) = value {
                let value = value.trim();
                if !value.is_empty() && value.chars().count() >= 4 && !value.starts_with(')') {
                    found = Some(format!(
                        "“{content}” may contain a secret ({label}) — stored locally only; it will be sent to the model provider when injected"
                    ));
                    break;
                }
            }
        }
    }
    if found.is_some() {
        return found;
    }
    // Credential-bearing URL: scheme://user:pass@…
    if let Some(rest) = after_prefix(content, "://") {
        if let Some(at) = rest.find('@') {
            let credentials = &rest[..at];
            if let Some(colon) = credentials.find(':') {
                let user = &credentials[..colon];
                let pass = &credentials[colon + 1..];
                if !user.is_empty() && !pass.is_empty() {
                    return Some(
                        "content embeds a user:password@ URL — stored locally only; it will be sent to the model provider when injected"
                            .to_owned(),
                    );
                }
            }
        }
    }
    None
}

/// Everything after the first occurrence of `prefix`, or `None`.
fn after_prefix<'a>(content: &'a str, prefix: &str) -> Option<&'a str> {
    let index = content.find(prefix)?;
    Some(&content[index + prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Secret *shapes* assembled from split literals: the checkout must
    /// never contain a contiguous secret-shaped string (GitHub push
    /// protection scans file content and rejects lines embedding a token).
    /// The runtime values are still full-strength shapes for the scanner.
    fn tok(parts: &[&str]) -> String {
        parts.concat()
    }

    #[test]
    fn hard_refusal_provider_keys() {
        for content in [
            tok(&["sk-", "abcdefghijklmnopqrstuvwxyz"]),
            tok(&["use sk-", "12345678901234567890 now"]),
            tok(&["pk-", "012345678901234567890123"]),
            tok(&["Anthropic key: pk-", "A1B2C3D4E5F6G7H8I9J0K1L2"]),
        ] {
            assert_eq!(scan(&content), SecretVerdict::Refused, "refuse {content:?}");
        }
        // Short sk- strings are NOT refused (not the canonical shape).
        assert_eq!(scan("sk-abc"), SecretVerdict::None);
    }

    #[test]
    fn hard_refusal_private_keys() {
        for content in [
            tok(&["-----BEGIN RSA PRIVATE KEY-----", "\n", "MIIEow..."]),
            tok(&["-----BEGIN OPENSSH PRIVATE KEY-----"]),
            tok(&["-----BEGIN EC PRIVATE KEY-----"]),
        ] {
            assert_eq!(scan(&content), SecretVerdict::Refused, "refuse {content:?}");
        }
    }

    #[test]
    fn hard_refusal_github_aws_slack() {
        for content in [
            tok(&["ghp_", "ABCDEFGHIJKLMNOPQRST1234567890abcdefghij"]),
            tok(&["github_pat_", "1234567890ABCDEFGHIJKLMNOPQRSTUVWX"]),
            tok(&["AKIA", "IOSFODNN7EXAMPLE"]),
            tok(&["xoxb-", "123456789012-123456789012-aBcDeFgHiJkLmNoP"]),
        ] {
            assert_eq!(scan(&content), SecretVerdict::Refused, "refuse {content:?}");
        }
        // AKIA with a short trailing run is not the AWS shape.
        assert_eq!(scan("AKIAaccess denied"), SecretVerdict::None);
        // xox alone (verbose notation) is not a token.
        assert_eq!(scan("the xox protocol"), SecretVerdict::None);
    }

    #[test]
    fn warn_on_label_value_proximity() {
        for content in [
            "password: hunter2",
            "token = abcdef123456",
            "api_key=ABCDefgh1234",
            "my password is hunter2",
        ] {
            assert!(
                matches!(scan(content), SecretVerdict::Warn(_)),
                "should warn on {content:?}"
            );
        }
    }

    #[test]
    fn store_silently_on_label_alone() {
        for content in [
            "my password manager setup",
            "token",
            "I forgot my password",
            "Secret is a great band",
        ] {
            assert_eq!(
                scan(content),
                SecretVerdict::None,
                "store {content:?} silently"
            );
        }
    }

    #[test]
    fn warn_on_credential_urls() {
        assert!(matches!(
            scan("db at https://user:hunter2@example.com/x"),
            SecretVerdict::Warn(_)
        ));
        // No credential → no warning.
        assert_eq!(scan("https://example.com/x"), SecretVerdict::None);
    }

    #[test]
    fn refusal_beats_warning() {
        // An sk- key inside a "password:" label still hard-refuses.
        let content = format!("password: {}", tok(&["sk-", "abcdefghijklmnopqrstuvwxyz"]));
        assert_eq!(scan(&content), SecretVerdict::Refused);
    }
}
