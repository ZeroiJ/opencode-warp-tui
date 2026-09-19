//! Forgetting leaves a tombstone: an append-only record of the SHA-256 of
//! the forgotten memory's *canonical identity* — nothing else (no content,
//! no quote, no session id — security §4, storage §11).
//!
//! Canonical identity (storage §11.2):
//!
//! ```text
//! canonical = kind + "\x00" + scope + "\x00" + normalize(content)
//! hash      = "sha256:" + hex(SHA-256(canonical))
//! ```
//!
//! Two memories are "the same" when kind + scope + normalized statement
//! match, regardless of key/id/formatting — that is the resurrection case
//! the tombstone blocks for V2 extraction/import. V1 explicit re-`remember`
//! is always allowed (storage §11.4); the tombstone is a one-way record.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use super::record::{normalize_content, Kind, Scope};
use super::store::OPEN_NOFOLLOW;
use super::MemoryError;

/// One tombstone line: the minimum needed to prevent silent resurrection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tombstone {
    pub v: i64,
    pub hash: String,
    pub scope: Scope,
    pub kind: Kind,
    pub forgotten_at: String,
}

impl Tombstone {
    pub fn to_json_line(&self) -> String {
        format!(
            r#"{{"v":{},"hash":"{}","scope":"{}","kind":"{}","forgotten_at":"{}"}}"#,
            self.v,
            self.hash,
            self.scope.as_str(),
            self.kind.as_str(),
            self.forgotten_at
        )
    }
}

/// Read the hashes currently recorded in a tombstone file. Missing and
/// empty files are valid; a torn final line is ignored; a malformed middle
/// line is skipped with a warning (count only — never content).
#[allow(dead_code)] // read-side API (audit/dedupe); no consumer in Phase 5.
pub fn load_hashes(path: &Path) -> Result<Vec<String>, MemoryError> {
    let mut hashes = Vec::new();
    if !path.exists() {
        return Ok(hashes);
    }
    let file = File::open(path).map_err(io_error(path, "open tombstones"))?;
    let mut malformed = 0usize;
    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };
        let Some(hash) = parsed.get("hash").and_then(serde_json::Value::as_str) else {
            malformed += 1;
            continue;
        };
        if hash.starts_with("sha256:") {
            hashes.push(hash.to_owned());
        } else {
            malformed += 1;
        }
    }
    if malformed > 0 {
        log::warn!("memory: {malformed} malformed tombstone line(s) ignored");
    }
    Ok(hashes)
}

/// Append one tombstone (`O_APPEND` + fsync), creating the file if needed.
/// Private mode (0600), no-follow, matching every other store file.
pub fn append_tombstone(path: &Path, tombstone: &Tombstone) -> Result<(), MemoryError> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        if OPEN_NOFOLLOW != 0 {
            options.custom_flags(OPEN_NOFOLLOW);
        }
    }
    let mut file = options
        .open(path)
        .map_err(io_error(path, "open tombstones for append"))?;
    let mut line = tombstone.to_json_line();
    line.push('\n');
    file.write_all(line.as_bytes())
        .map_err(io_error(path, "append tombstone"))?;
    file.sync_all()
        .map_err(io_error(path, "fsync tombstones"))?;
    Ok(())
}

fn io_error<'a>(path: &Path, action: &'a str) -> impl FnOnce(std::io::Error) -> MemoryError + 'a {
    let display = path.display().to_string();
    move |error| MemoryError::StoreUnavailable(format!("{action} {display}: {error}"))
}

/// `sha256:<hex(SHA-256(key + "\0" + scope + "\0" + normalized_content))>`.
pub fn canonical_hash(kind: Kind, scope: Scope, content: &str) -> String {
    let canonical = format!(
        "{}\0{}\0{}",
        kind.as_str(),
        scope.as_str(),
        normalize_content(content)
    );
    format!("sha256:{}", hex(&sha256(canonical.as_bytes())))
}

/// Whether the tombstone set already covers this canonical identity —
/// the check V2 extraction/import must make before re-adding content.
#[allow(dead_code)] // read-side API (audit/dedupe); no consumer in Phase 5.
pub fn contains(hashes: &[String], kind: Kind, scope: Scope, content: &str) -> bool {
    hashes
        .iter()
        .any(|hash| hash == &canonical_hash(kind, scope, content))
}

// ---------------------------------------------------------------------------
// SHA-256 — compact std-only implementation (FIPS 180-4), tested against
// published vectors. No Cargo dependency needed (AGENTS.md rule).
// ---------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Compute the SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for (i, &k) in K.iter().enumerate() {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    let mut digest = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Lowercase hex encoding.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap());
        out.push(char::from_digit(u32::from(byte & 0x0F), 16).unwrap());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        // FIPS 180-4 / NIST test vectors.
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            hex(&sha256(b"hello world")),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn sha256_multi_block() {
        let long = "a".repeat(1_000_000);
        assert_eq!(
            hex(&sha256(long.as_bytes())),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn canonical_hash_normalizes_content() {
        let kind = Kind::Fact;
        let a = canonical_hash(kind, Scope::User, "  Prefer\nRust  ");
        let b = canonical_hash(kind, Scope::User, "Prefer Rust");
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
        assert_eq!(a.len(), 7 + 64);
    }

    #[test]
    fn canonical_hash_distinguishes_kind_and_scope() {
        let content = "Prefer Rust";
        let fact_user = canonical_hash(Kind::Fact, Scope::User, content);
        let pref_user = canonical_hash(Kind::Preference, Scope::User, content);
        let fact_project = canonical_hash(Kind::Fact, Scope::Project, content);
        assert_ne!(fact_user, pref_user);
        assert_ne!(fact_user, fact_project);
    }

    #[test]
    fn tombstone_line_is_content_free() {
        let tombstone = Tombstone {
            v: 1,
            hash: canonical_hash(Kind::Fact, Scope::User, "the secret statement"),
            scope: Scope::User,
            kind: Kind::Fact,
            forgotten_at: "2026-09-18T12:00:00Z".to_owned(),
        };
        let line = tombstone.to_json_line();
        assert!(!line.contains("secret statement"));
        assert!(line.contains("sha256:"));
        assert!(line.contains("2026-09-18T12:00:00Z"));
    }

    #[test]
    fn contains_matches_canonical() {
        let hashes = vec![canonical_hash(Kind::Fact, Scope::User, "  Prefer\nRust ")];
        assert!(contains(&hashes, Kind::Fact, Scope::User, "Prefer Rust"));
        assert!(!contains(
            &hashes,
            Kind::Preference,
            Scope::User,
            "Prefer Rust"
        ));
        assert!(!contains(
            &hashes,
            Kind::Fact,
            Scope::Project,
            "Prefer Rust"
        ));
    }

    #[test]
    fn load_hashes_handles_missing_empty_and_torn() {
        let dir = std::env::temp_dir().join(format!("owt-tomb-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tombstones.jsonl");
        let _ = std::fs::remove_file(&path);

        assert_eq!(load_hashes(&path).unwrap(), Vec::<String>::new());

        std::fs::write(&path, "").unwrap();
        assert_eq!(load_hashes(&path).unwrap(), Vec::<String>::new());

        // Valid line + torn final line (no trailing newline) + garbage middle.
        let good = canonical_hash(Kind::Fact, Scope::User, "first");
        let good2 = canonical_hash(Kind::Fact, Scope::User, "second");
        std::fs::write(
            &path,
            format!(
                "{{\"v\":1,\"hash\":\"{good}\",\"scope\":\"user\",\"kind\":\"fact\",\"forgotten_at\":\"2026-09-18T12:00:00Z\"}}\nnot json\n{{\"v\":1,\"hash\":\"{good2}\",\"scope\":\"user\",\"kind\":\"fact\",\"forgotten_at\":\"2026-09-18T12:00:00Z\"}}\n{{torn",
            ),
        )
        .unwrap();
        let hashes = load_hashes(&path).unwrap();
        assert_eq!(hashes, vec![good, good2]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn append_tombstone_creates_and_appends() {
        let dir = std::env::temp_dir().join(format!("owt-tomb-append-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tombstones.jsonl");
        let _ = std::fs::remove_file(&path);

        let one = Tombstone {
            v: 1,
            hash: canonical_hash(Kind::Fact, Scope::User, "one"),
            scope: Scope::User,
            kind: Kind::Fact,
            forgotten_at: "2026-09-18T12:00:00Z".to_owned(),
        };
        let two = Tombstone {
            v: 1,
            hash: canonical_hash(Kind::Fact, Scope::User, "two"),
            scope: Scope::User,
            kind: Kind::Fact,
            forgotten_at: "2026-09-18T12:01:00Z".to_owned(),
        };
        append_tombstone(&path, &one).unwrap();
        append_tombstone(&path, &two).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(&one.hash));
        assert!(lines[1].contains(&two.hash));
        assert_eq!(load_hashes(&path).unwrap(), vec![one.hash, two.hash]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
