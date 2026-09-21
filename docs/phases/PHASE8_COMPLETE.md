PHASE 8 IMPLEMENTATION REPORT

## Implemented (§9-A gate only)

- Rule-based proposal generation (`memory/extract.rs`): deterministic
  user-sentences → typed drafts (imperative-v1, preference-v1,
  constraint-v1, plus needs-review goal/relation/qualified patterns).
- User-message-only extraction: structural `type == "user"` filter
  (`user_texts`); assistant/tool/shell/system/command echoes excluded.
- Session-end generation: `set_active` leave-hook + suggest-time refresh
  on both backends; no daemon, no per-turn, no timers.
- Candidate filtering: questions, code, diffs, errors, URLs, <3 words,
  session-local refs, ephemeral task state, over-long content.
- Security: D23 `scan` on every candidate (refused AND warned both
  drop, counts only in logs); security precedes queue visibility.
- Tombstone pre-check at propose AND re-check at confirm
  (`canonical_hash` kind+scope+normalized, frozen semantics).
- Normalized dedup vs ACTIVE store (same kind+scope) + within-batch.
- Quarantine queue (`memory/proposal.rs`): `proposals.jsonl` +
  `discarded.jsonl` per scope dir, 0600, atomic temp+fsync+rename,
  corrupt-tolerant load, MAX 200 FIFO, discarded 500 FIFO, lazy 30-day
  expiry on open.
- Deterministic lexical ordering: suggest indices positional in
  (created_at, id); scorer (`memory/lexical.rs`) orders confirm-context
  and `ordered_active_scored` (empty terms = byte-identical D24).
- `/memory suggest|confirm|discard` via the existing parser + in-band
  replies; confirm `<n|id> [--scope] [--as]`; discard `<n|id>` and
  `discard all` (friction: bare form only explains, `--confirm` executes).
- Optional `method` field only (`explicit` | `rule:<name>`); absent =
  explicit; no migration; pre-8 files byte-stable.
- Confirm writes through `MemoryApi::remember` (existing validation,
  secret policy, supersession); originating session + quote preserved;
  same-scope context listed; dup-vs-active becomes already-stored notice.

## Files changed

- `memory/record.rs`: `Method` enum + `method` field (record/NewMemory),
  emit-if-present serialization, tolerant load, validation.
- `memory/api.rs`: `NewMemoryArgs.method/quote/session_id`, passthrough,
  `active_in`, `has_project`, `tombstone_hit`, `ordered_active_scored`,
  `proposal_queue`.
- `memory/store.rs`: `tombstone_contains` (+1 test fixture field).
- `memory/command.rs`: Suggest/Confirm/Discard verbs, parsing, help text
  (+1 test fixture field).
- `memory/lexical.rs`, `extract.rs`, `proposal.rs`, `triage.rs`: NEW.
- `memory/mod.rs`: module decls. `opencode/mod.rs`: suggest/confirm/
  discard arms, set_active hook, history user-text fetch, adapter tests.
- `mock.rs`: temp-dir engine, triage routing, set_active hook, tests.
- `opencode/memory_inject.rs`: test-literal field only.

## Tests: 239 passed / 0 failed / 1 ignored (was 195/0/1)

New coverage: 3 rules, user-only gate, 10 negative filters, all D23
patterns + fuzz-adjacent cases, prompt-injection-like text, tombstone
resurrection (propose + confirm paths), normalized/batch/dedup,
discarded-set FIFO + no-resurface, queue persist/reload/corrupt/perms/
bounds/expiry/ordering, suggest/confirm(-by-index,-by-id,-overrides)/
discard(-by-index,-by-id,-all reject/accept), confirm failure keeps
state, secret re-screen at confirm, quarantine-never-injects, scorer
determinism/goldens/identity, method round-trip/compat, mock parity
flows, adapter stub-history end-to-end, ordered_active_scored identity.

## Precision evaluation

30-item committed fixture corpus: 16/16 proposed, 0 false positives →
precision 1.00 (gate ≥ 0.80), zero secret/zero poison proposals,
deterministic repeat runs. Caveat, stated honestly: the corpus is
authored alongside the rules (no independent labeled set exists in
repo); the number measures fixture agreement, not field performance.

## Security

D23 reused unchanged (refuse + warn both drop pre-queue); user-only
input kills tool/model/file poisoning structurally; tombstones close
resurrection; queue files 0600 beside their scope store; confirm
re-screens + re-checks; discard-all needs the literal `--confirm`;
push-protection-safe split literals in secret tests. Residuals per
security-review: paraphrase tombstone-evasion (human backstop),
confirmer click-through (friction only mitigates).

## Dependencies: none added. OpenCode changes: none (read history via
existing client; no new routes). Laya: untouched (zero references).
Phase 7 regressions: none (full suite + PTY green).

## Known limitations

- Confirm has no same-key update path (proposals are keyless):
  normalized dup-vs-active yields an already-stored notice instead.
- Suggest indices are positional at command time (tab switch may add
  proposals between suggest and confirm).
- Queue has no cross-process lock (production path is single-threaded;
  tests use unique temp dirs).
- ordered_active_scored is dormant-specified (allow(dead_code), tested).
- Live coverage: one scripted PONG turn + history read + delete;
  execution paths covered by stubs/fixtures.

## Git status

Modified: memory/{api,budget,command,mod,record,store}.rs,
mock.rs, opencode/{mod,memory_inject}.rs. New: memory/{extract,
lexical,proposal,triage}.rs. Untouched: Cargo.*, ~/warp, TUI, Phase
5/6/7 semantics, phases.md (pre-existing unrelated hunk left alone).
