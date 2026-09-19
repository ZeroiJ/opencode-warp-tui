//! Key normalization/validation and dependency-free id generation.
//!
//! Keys follow OpenCode's own instruction-entry pattern
//! `^[a-z0-9][a-z0-9._-]{0,63}$` (storage §12, D16) and are normalized to
//! lowercase at creation — case-insensitive by construction.
//!
//! Ids are `owt_<unix_millis>_<pid>_<seq>` (storage §4, D16): seq is a
//! process-global counter, so ids are unique across both scopes within one
//! process (a `show <id>` can never be ambiguous) and never reused.

use std::sync::atomic::{AtomicU64, Ordering};

use super::MemoryError;

static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Validate + normalize a key: trim, lowercase, match the D16 pattern.
pub fn validate_key(key: &str) -> Result<String, MemoryError> {
    let key = key.trim().to_lowercase();
    let mut chars = key.chars();
    let first = chars.next();
    let first_ok = matches!(first, Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit());
    let rest_ok =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    let length_ok = (1..=super::record::MAX_KEY_CHARS).contains(&key.chars().count());
    if first_ok && rest_ok && length_ok {
        Ok(key)
    } else {
        Err(MemoryError::InvalidKey(key))
    }
}

/// True when `id` has the generated shape `owt_<millis>_<pid>_<seq>`.
/// The command layer uses this to decide whether a handle token is an id
/// (`owt_…`) or a key before resolving it.
pub fn is_id_shape(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    let mut parts = id.split('_');
    parts.next() == Some("owt")
        && parts.clone().count() == 3
        && parts.all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// Generate `owt_<unix_millis>_<pid>_<seq>`.
pub fn generate_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("owt_{millis}_{}_{seq}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_accepts_valid_forms() {
        for key in [
            "a",
            "z9",
            "lang",
            "db-choice",
            "pref.rust",
            "a_b-c.d",
            "  padded  ",
        ] {
            assert!(validate_key(key).is_ok(), "should accept {key:?}");
        }
        assert_eq!(validate_key("  Lang ").unwrap(), "lang");
        assert_eq!(validate_key("DB-Choice").unwrap(), "db-choice");
    }

    #[test]
    fn key_rejects_invalid_forms() {
        let too_long = "a".repeat(65);
        for key in [
            "",
            "-lead",
            ".lead",
            "_lead",
            "has space",
            "has$ymbol",
            "has/slash",
            too_long.as_str(),
        ] {
            assert!(validate_key(key).is_err(), "should reject {key:?}");
        }
        // First char may be a digit.
        assert!(validate_key("9a").is_ok());
    }

    #[test]
    fn key_lowercases_and_trims() {
        assert_eq!(validate_key("  Prefer.Rust ").unwrap(), "prefer.rust");
    }

    #[test]
    fn generated_ids_are_unique_and_shaped() {
        let a = generate_id();
        let b = generate_id();
        assert_ne!(a, b);
        for id in [&a, &b] {
            let mut parts = id.split('_');
            assert_eq!(parts.next(), Some("owt"));
            assert_eq!(parts.clone().count(), 3);
            assert!(parts.all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())));
        }
    }
}
