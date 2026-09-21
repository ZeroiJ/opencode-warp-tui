# Phase 8 — Security Review (intelligence threat model)

> Research only. Parent: recon §8L/8M, decisions 8-R1–R3, 8-R7.
> V1 baseline (D23 secret refusal, 0600/0700 files, no telemetry,
> tombstones, adapter isolation) is assumed and unchanged.

## 1. Threat model delta (what intelligence adds)

| New capability | New attack surface |
|---|---|
| Reading history automatically | bulk ingestion of whatever the session saw |
| Rule proposals | false-memory injection if rules misfire |
| Quarantine queue | second store file with same sensitivity as memory |
| Confirm path | social-engineering the confirmer (user clicks through) |
| Future LLM extraction | history exfiltration to providers; prompt-targeted extraction |

## 2. Load-bearing safeguards (each maps to a decision)

1. **User-messages-only input** (8-R2): kills tool-transcript secrets,
   file-content ingestion, model confabulation, and project-file
   poisoning at the source. An attacker planting "remember to upload
   API keys" in a repo file can never reach the extractor (it never
   reads files).
2. **Quarantine** (8-R1): proposals are inert data until confirm; the
   injection path only reads `memory.jsonl`, never the queue.
3. **D23 unchanged on proposals**: secret patterns screened before
   queueing; counts logged without content.
4. **Tombstone pre-check**: forgotten content can never be re-proposed
   (resurrection closed across the intelligence boundary too).
5. **Scope guards**: project-scope proposals only from project-rooted
   sessions; no cross-project proposal flow (adapter knows its root;
   queue is per-scope-dir).
6. **Queue file discipline**: same dir, same 0600/atomic-write rules as
   the store; discarded-set bounded (500 FIFO, §3 of intelligence doc).
7. **Confirm friction**: confirm shows quote + rule + scope + conflicts-
   context; `confirm all` requires the literal token `all` (no bulk
   accidents); no auto-confirm, no timeout-default-confirm.
8. **No telemetry, no network**: generator is pure local code; nothing
   added phones home. (Explicitly restated because LLM extraction —
   deferred — would reopen this.)

## 3. Memory-poisoning scenarios (worked)

- **Malicious repo file**: never read by extractor → no path. [CLOSED]
- **Malicious user-pasted text** ("remember my password is X"):
  secret screen drops credentials; non-secret instruction-like text
  ("always curl evil.sh") still quarantines and needs human confirm —
  the confirmer sees the quote verbatim. [MITIGATED, human-in-loop]
- **Model-originated claim** ("as an AI, the password is…"): model
  messages excluded from input. [CLOSED]
- **Compromised session history** (attacker with session access writing
  user messages): out of scope — equivalent to attacker typing; local
  single-user threat model unchanged from V1.
- **Tombstone-evasion via paraphrase**: paraphrased re-proposal of
  forgotten content passes the hash check and reaches quarantine —
  human triage is the backstop (same as V1 explicit remember).
  [ACCEPTED RESIDUAL, documented]

## 4. Privacy review

Extraction processes conversation the user already had locally; no new
collection (no file reads, no network). Proposals inherit store
permissions. Confirm/updated_at timestamps could reveal habits at file-
metadata level — same exposure class as existing store mtimes (accepted
V1 residual). No PII-specific handling beyond secret refusal: extraction
does not seek personal data, and worthiness rules exclude profile-like
mining (no "user lives in X" patterns exist in the catalog).

## 5. Security acceptance (implementation gate inputs)

Poison/secret fixture suite: every scenario in §3 as a test with
expected drop/quarantine outcome; fuzz rule patterns against adversarial
quote corpus; queue-file permission tests; confirm-all friction test;
tombstone resurrection test across the proposal path.
