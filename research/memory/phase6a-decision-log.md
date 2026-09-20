# Phase 6A — Decision Log (recommendations)

> Research only. Nothing here modifies production code. These `6A-R`
> recommendations are Phase 6B implementation requirements, not
> architecture changes. **No Phase 2B decision (D1–D25) is revised.**
> One Phase 2B UNRESOLVED item (D17: entry-value rendering) is
> *completed* by new evidence — recorded as a resolution, not a revision
> (6A-R7). Where a recommendation touches a 2B decision it cites it and
> requires no authorization beyond Phase 6B itself.

## 6A-R1 — Injection hook lives inside the two session-create paths

**Context:** D18 (frozen session-start) + recon §3/§12. OWT creates
sessions in exactly two places: `OpenCodeBackend::new_session` and the
`initial_load` auto-create.

**Recommendation:** Phase 6B performs probe + `PUT owt.memory` inline in
both paths before the id is returned. Never on resume (`set_active`,
`hydrate_active`, SSE echoes).

**Touches:** D18 (implements, no change). **Authorization:** Phase 6B only.

## 6A-R2 — Capability probe shape (self-cleaning, version-keyed)

**Context:** Recon §8; Phase 2B version-resilience §2.

**Recommendation:** throwaway session → `PUT owt.probe` → `GET` →
`DELETE` key → `DELETE` session; path preference experimental→stable
alias; cache per `/api/info` version (≈1 h TTL); any failure ⇒ injection
off + warn once. No version-string gating.

**Touches:** none (executes existing design). **Authorization:** Phase 6B only.

## 6A-R3 — Client needs two thin wrappers, nothing else

**Context:** Recon §15. `Client::request()` already speaks PUT/DELETE.

**Recommendation:** add `put_instruction_entry(session, key, value)` and
`delete_instruction_entry(session, key)` reusing `request()`; no client
refactor, no new dependencies.

**Touches:** none. **Authorization:** Phase 6B only.

## 6A-R4 — PUT-before-first-prompt is a code-placement guarantee, not a protocol

**Context:** Recon §12. No API-level ordering primitive exists.

**Recommendation:** placement (R1) *is* the guarantee; add a debug
assertion/test that no `send_prompt` path can run for a session created
after injection was enabled without the PUT-or-skip having resolved.
At most one bounded retry-or-skip; no hot loops.

**Touches:** D18 (implements). **Authorization:** Phase 6B only.

## 6A-R5 — Resume never writes

**Context:** Recon §11. Resumed sessions keep their original frozen
snapshot.

**Recommendation:** no entry PUT/GET/DELETE on any resume path. Memory
drift between sessions is impossible when no write path exists.

**Touches:** D18 (implements). **Authorization:** Phase 6B only.

## 6A-R6 — Config knob `memory.injection = auto | off`

**Context:** Users must be able to run memory-commands-only (Phase 5
behavior) even against a capable server.

**Recommendation:** default `auto`; `off` skips probe + PUT entirely.
No OpenCode-side configuration is ever touched.

**Touches:** none (new surface, additive). **Authorization:** Phase 6B only.

## 6A-R7 — D17 rendering UNRESOLVED → resolved: keep `{"text": block}` and the current fence

**Context:** Recon §6. Controlled model probe shows text-level
system-context rendering with fence/footer intact.

**Recommendation:** Phase 6B consumes `budget::render_block` /
`encoded_block` unchanged. This **resolves** the Phase 2B open item; it
does not revise D17. Re-verify reception with the §6 recipe only if the
block format ever changes.

**Touches:** D17 (completes open item). **Authorization:** none beyond 6B.

## 6A-R8 — Rendering-unverifiable does not block injection

**Context:** Recon §10. Phase 2B worded sandbox verification as a
"precondition".

**Recommendation:** interpret the gate as *write-verified (204 + GET)
plus reception-verified-once (done here)*. A provider-less deploy still
injects; the server-side write contract is independently testable.

**Touches:** clarifies Phase 2B decision-report §8 wording; no semantic
change. **Authorization:** note for Phase 6B, no architecture change.

## 6A-R9 — Post-turn entry-GET anomaly is watch-only

**Context:** Recon §7/§16.1: one empty-body GET after a model turn.

**Recommendation:** no action; V1 never reads entries back. Re-verify only
if a future phase needs read-back.

**Touches:** none. **Authorization:** none.

## 6A-R10 — Phase 6B test contract

**Recommendation:** stub-server matrix (experimental-only /
stable-only / both / neither / low-limit-413 / offline) + live-sandbox
golden tests (PUT→GET round-trip, sequencing, degradation rows) +
regression (memory-off ⇒ byte-identical session behavior) + the §6
reception recipe documented for future re-verification.

**Touches:** executes Phase 2B test strategies. **Authorization:** Phase 6B only.
