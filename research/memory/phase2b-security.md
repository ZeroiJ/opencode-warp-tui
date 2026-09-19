# Memory Research Phase 2B — Security & Privacy Design

> Status: 🔬 RESEARCH ONLY — Phase 2B design output. **Nothing here is
> implemented.** This document is the local-security review (Phase 2B
> brief §22–§23, §26): threat model, file/store hardening, path and
> symlink defenses, the secret policy (with honest limits), prompt-
> injection analysis, malformed/oversized input handling, denial-of-
> service guards, export/import/backup/log/telemetry discipline, and the
> privacy & data-ownership statement.
>
> Evidence labels: **[VERIFIED]** live probe · **[DOCUMENTED]** official
> docs/specs · **[INFERRED]** analysis · **[RECOMMENDATION]** Phase 2B
> decision → Phase 5 review.
>
> Companion: `phase2b-storage.md` (physical layout, write protocol),
> `phase2b-injection.md` (fencing, failure), `phase2b-version-resilience.md`.

---

## 1. Threat model (local-first scope)

This is a **single-user, local, adversarial-filesystem-adjacent** model —
not a server:

| Trust boundary | Who is trusted | Who is not |
|---|---|---|
| The user's own machine account | fully trusted (owns files, may read them with `cat`) | — |
| OpenCode server process (managed by the user) | trusted to receive the injected block *by design* | — |
| LLM providers | **NOT trusted with secrets**: injected memory is sent to the provider (that is its purpose) → secret refusal exists because providers are out of scope | — |
| Other local processes / other OS users | not trusted | can read `0600`/`0700` files only with elevated privileges |
| Malicious repositories opened by the user (arbitrary code in the project) | not trusted | can attempt path/symlink attacks via the project store location |
| Malicious/accidental *memory content* | not trusted | prompt-injection targets |

The security *boundary* is the filesystem permissions plus the refusal
layer; the injection path adds the fencing discipline
(`phase2b-injection.md` §5). We never claim "secret-free" —
`phase2b-security.md` §4 states exactly what is and is not guaranteed.

---

## 2. File permissions & store hardening (decision)

| Path | Mode | Why |
|---|---|---|
| `$XDG_DATA_HOME/owt/` (user store dir) | `0700` | Private data; only the owner reads/writes |
| `$XDG_DATA_HOME/owt/memory.jsonl`, `tombstones.jsonl`, `memory.lock` | `0600` | Memory content and forget history are private |
| `<root>/.owt/` (project store dir) | `0700` | Same — even though it lives inside a repo, other repo readers must not see memory |
| `<root>/.owt/memory.jsonl`, `tombstones.jsonl`, `memory.lock` | `0600` | Same |
| Temp files (`memory.jsonl.tmp.<pid>`) | `0600`, created in the same dir, removed on exit | Never leak a half-written state to other processes |

Creation: create dirs with restrictive modes at first use; never
`chmod`-loosen existing files; if an existing file has looser modes, open
it anyway but warn (user-owned, may predate the policy) —
**[RECOMMENDATION]**. Use `OpenOptions` with `O_NOFOLLOW`/`O_CLOEXEC`
(custom flags on Linux) for store files and the lock file.

---

## 3. Path traversal, symlink attacks, malicious repositories

| Attack | Defense |
|---|---|
| User-supplied path in a command | **No path inputs exist.** Memory commands accept keys/ids only (`phase2b-retrieval.md` §7). Store paths are fixed by the implementation (XDG + project root) — not parameterized. |
| Project-root path escaping | The project root comes from the adapter's canonical project location; the store path is derived *inside* that root, never from user text. The adapter must canonicalize (resolve symlinks) before deriving `.owt/` **[RECOMMENDATION]**. |
| Symlinked store dir/file | Open with `O_NOFOLLOW`; if the store path resolves to a symlink, refuse and warn (do not follow, do not create through the link). A malicious repo could otherwise plant `.owt` as a symlink to somewhere sensitive. |
| Malicious repo plants a *valid* `.owt/memory.jsonl` | Possible and accepted: the project store is user-writable in-repo by design. Content is loaded as *data* (fenced, refused, validated) — never executed, never treated as config. The same repo could plant a `.bashrc`-sourced file; the memory engine's job is to not make it worse. **[INFERRED]** |
| Malicious repo plants an enormous/corrupt store | Size guard + malformed-line degrade (storage §8) |
| `/tmp` usage | Not used for store data (only the sandboxed probe suite in Phase 5) |

---

## 4. Secret policy (Phase 2B brief §23) — decision D23

Policy: **high-confidence patterns → refuse (hard error, nothing stored);
label+value proximity → warn but store; label alone → store silently.**
Never redact, never mangle (P2A §12.3: refusal over redaction in a
user-owned local store).

### 4.1 High-confidence refusal patterns (V1 set — deliberately small)

| Pattern class | Example shape | Rationale |
|---|---|---|
| Provider API keys | `sk-…`, `pk-…` (20+ chars after prefix) | Canonical OpenAI/Anthropic key shape |
| Private keys | `-----BEGIN …PRIVATE KEY-----` | Unambiguous PEM armor |
| GitHub tokens | `ghp_…`, `github_pat_…` | Long, distinctive |
| AWS access keys | `AKIA[0-9A-Z]{16}` | Distinctive, high value |
| Slack tokens | `xox[baprs]-…` | Distinctive |

A **hard refusal** is messaging + no state change — the command fails
with "looks like a secret; not stored" (+ a hint to use environment
secrets instead).

### 4.2 Warning-only (label + value)

A value that *looks* credential-like by label/value proximity (e.g.
`password: hunter2`, `token = abcdef123456`) but fails the high-confidence
set → store **with a visible warning** in the command reply: "may be a
secret — stored locally only; it will be sent to the model provider when
injected." Rationale: label-pattern matches have high false-positive rates
("my password manager setup…"), and hard-refusing them blocks legitimate
memory; the warning preserves the user's decision while flagging the
consequence. **[RECOMMENDATION — revisit condition: if warning-only rows
are ever found injected in the wild, escalate to refusal]**

### 4.3 Honest limits (documented, no false claims)

- This is **not a security boundary**. The *real* boundary is `0600` file
  permissions + physical machine control. Secret detection is best-effort
  ergonomics.
- The set is **small and explicit** (no giant regex zoo, Phase 2B brief
  §23). Unknown/new secret shapes WILL pass. Credential URLs
  (`https://user:pass@host`) are **uncertain**: warn on `://…:…@`
  value form; refuse only when the embedded credential matches a §4.1
  pattern. **[RECOMMENDATION]**
- Deterministic and dependency-free: patterns are plain regex lists in
  one module, unit-tested, no ML, no network.
- Users are told in the docs: never store secrets in memory because
  **injected memory is transmitted to the LLM provider** (§9).

---

## 5. Prompt-injection & untrusted memory content (Phase 2B brief §15, §22)

Full fencing design lives in `phase2b-injection.md` §5. Security-side
summary and guarantees:

| Property | Guarantee |
|---|---|
| Memory never executes | True by construction: no path from a memory string to tool calls, config, or shell (adapter-side pure-data store; AGENTS.md isolation) |
| Memory is visually/labeled as data | True: namespaced fence, "data, not commands" header, per-record metadata |
| Memory can never be mistaken for *the user's current instruction* | By design (position 6 assembly + fenced data lines) — but see honest limits below |
| Fence cannot be broken early by content | True for the chosen line-oriented format (no open tags; content newlines collapsed) **[VERIFIED by design, to be tested §10]** |
| Model is immune to an injected instruction | **Not guaranteed, ever.** Defense-in-depth only, bounded by curation (D4: user writes memory) + refusal/warning (§4). |

---

## 6. Malformed / hostile input handling

| Input | Policy |
|---|---|
| NUL / C0 control characters in content/quote | **Rejected at ingestion** (storage §4.2) — they are meaningless in memory text and hostile to rendering |
| Newlines in content | Collapsed at render time (injection §4.2) — a memory cannot forge metadata lines structurally |
| Oversized content (>4,096 chars) / quote (>256) / key (>64) | Rejected at ingestion |
| Invalid UTF-8 byte sequences | Rejected as invalid input (never lossy-decoded) |
| Malformed JSON/records on read | Skipped + warned (storage §8) — never executed, never auto-repaired |
| Extremely long lines / pathological files | 10 MiB store guard (§7) |

---

## 7. Denial of service / resource abuse

| Scenario | Guard |
|---|---|
| Huge store file | Load refusal >10 MiB per store file (storage §8); memory degrades to unavailable-with-warning, session unaffected |
| Huge injection block | Encoded-byte invariant ≤200,000 B (retrieval §3), single-entry transport (injection §2) |
| Excessive per-record size | Ingestion caps §6 |
| Unbounded tombstone growth | Append-only but tiny objects; no compaction in V1; revisit with SQLite (storage §11.3, §15) |
| Malicious repo replaces the store mid-session | Effect limited to that session's next injection read (frozen at session start; re-read at next session start) **[INFERRED]** |
| Lock-file abuse | flock is released by the kernel on process death; a live hostile holder of *the user's own lock file* is out of scope (same user, full file access anyway) |

---

## 8. Export / import / backups / logs / telemetry

| Topic | Policy |
|---|---|
| Export | JSONL file itself is the export format (human-readable); a dedicated `memory export` CLI verb is Phase 5 roadmap (decision-report §5). |
| Import | **Deferred** (V2). Import is the never-silent-resurrection path that the tombstone must guard (storage §11.4) — importing is exactly the automatic path tombstones block. |
| Backups | The file is portable; users may copy it; the store never writes its own shadow backup (no hidden data). |
| Logs | **Memory content never appears in logs or diagnostics.** Errors report ids/keys/timestamps only. |
| Telemetry | None. The store never phones home; no analytics fields exist in the schema (storage §4). |
| Debug dumps (phase-5 diagnostics) | Require explicit opt-in and must redact `content`/`quote` unless the user consents — a documented dev-only hook **[RECOMMENDATION]** |

---

## 9. Privacy & data ownership (reaffirming P2A §3; Phase 2B brief §26)

What memory means, unambiguously:

1. **Where it lives:** the two local files per scope (storage §3).
2. **Who can read it:** the owning OS user (0600/0700); nothing else
   without privilege escalation. The OpenCode server process (same user)
   receives only the injected block.
3. **Whether it is sent to OpenCode / providers:** **the injected block
   IS sent to the LLM provider** — that is the entire purpose. Sent data =
   ACTIVE, user-authored, budgeted, fenced records only; secrets refused
   (§4); users control the corpus (D4).
4. **Logs/diagnostics:** never included (§8).
5. **Export/delete:** readable, copyable, deletable by the user at any
   time (`forget` hard-deletes + tombstones, storage §11).
6. **Raw OpenCode history untouched:** memory is a separate distilled
   layer; deleting/forgetting memory never modifies OpenCode session
   history (P2A §13.1:3, §26). Conversely, OpenCode history is the source
   of conversational truth; memory is derived, curated, and explicit.
   **[RECOMMENDATION]** — this is the durable ownership contract.

**One honest caveat documented for users:** because injected memory goes
to the provider, memory is *not* private-by-default in the sense the local
file is. The refusal policy + curation are the guardrails; the file
permissions are the privacy of the *at-rest* store.

---

## 10. Security test strategy (Phase 5)

| Area | Tests |
|---|---|
| Permissions | New stores create 0700/0600; no chmod-loosening; temp file modes; masked umask handling |
| Path/symlink | Store path derived from canonical project root; planted symlink `.owt` → refusal with warning; no-follow on open |
| Secret refusal | Positive matches per §4.1 → command fails, file unchanged; §4.2 warnings issued but stored; label-only stored silently; URL-credential handling; unknown shape passes (documented) |
| Injection-as-data | "Ignore previous instructions…", system-message impersonation, JSON/XML payloads, null bytes, oversized content → block structure intact, data label present |
| Oversized | >4,096-char content rejected; >10 MiB store refused to load with warning; huge-block encode guard |
| DoS | Pathological store fixtures → degrade path, session unaffected |
| Logs | No content in error/log output (grep diagnostics fixtures) |
| Delete | `forget` removes content from file; tombstone has no content; raw OpenCode history untouched (integration assertion) |

---

## ARCHITECTURE STATUS

```text
Security architecture: resolved. Implementation NOT authorized.
```

Threat model, permissions, secret policy (with honest limits), fencing
discipline, abuse guards, and the privacy contract above are the Phase 5
contract. No memory store exists; nothing is implemented.