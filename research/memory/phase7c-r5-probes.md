# Phase 7C-R5 — Targeted Probe Addendum

> Research only. Three one-increment probes requested after the 7C
> decision package, executed against OpenCode 2.0.8 on a scratch git
> worktree (`/tmp/7c-work`, removed) and one throwaway session (deleted,
> 204). One extra model turn burned for the edit; one for `review`.
> No production code touched.
>
> Result: all three NEEDS-MORE-RESEARCH items are now **READY FOR
> IMPLEMENTATION** with exact contracts below. Decisions 7C-R9…R11
> appended to `phase7c-decision-log.md`.

## R5-1. Populated diff — VERIFIED

`GET /api/session/{id}/diff` after an agent edit:

```json
{"data": [{
  "file": "notes.txt",
  "patch": "diff --git a/notes.txt b/notes.txt\nindex 2d00bd5..c775bfc 100644\n--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1,2 @@\n line one\n+probe-change-1\n",
  "additions": 1, "deletions": 0, "status": "modified"
}]}
```

Shape: per-file `{file, patch (unified git diff text), additions,
deletions, status}`. Empty case → `{"data":[]}` (already known).
Contract: render `status` + counts + patch body; fetch is read-only,
no events, no side effects. Binary/truncation behavior still unobserved
— cap patch display length client-side (e.g. truncate with marker;
exact cap is implementation detail).

## R5-2. Revert with real files — VERIFIED (destructive semantics confirmed)

Sequence on a session whose turn appended one line to `notes.txt`:

1. **Stage** `POST …/revert/stage {"messageID": <user msg>}` → 200
   `{data: {messageID, snapshot: "<sha>", files: [{file, patch
   (REVERSE diff), additions: 0, deletions: 1, status}]}}`.
   **The filesystem restore happens at stage time** (file already
   reverted when read back immediately after). Boundary recorded in
   `GET session → revert` (messageID + snapshot + files).
2. **Commit** `POST …/revert/commit` → 204. **All messages after the
   boundary are deleted** (direct `GET message/{id}` → 404
   `MessageNotFoundError`); `revert` field → `None`; worktree clean, no
   stash left. **Irreversible via API.**
3. **Delete** `DELETE …/revert` (staged, empty-files boundary) → 204:
   boundary cleared, **one `idle` marker appended, zero messages
   removed**.

Critical distinction (answers recon §10): stage = filesystem undo +
boundary record; commit = conversation-history rollback + boundary
clear. They are two different destructive axes, and commit's history
deletion is the more irreversible of the two (files could at least be
re-done from the recorded reverse patch; deleted messages cannot).

Contract: stage → show affected files from `files[]` + require explicit
confirmation BEFORE commit (7C-R7 stands, strengthened: confirm lists
both file restoration and message deletion); DELETE is the safe
"abandon boundary" path; never auto-stage/auto-commit.

## R5-3. Slash-command execution — VERIFIED (success path)

`POST …/command {"name":"review","text":""}` → **204**. Execution
materializes as a **synthetic `user` message** carrying the command's
prompt template ("You are a code reviewer…"), followed by a normal
tool-using agent turn (reads observed, 3 assistant messages, terminal
idle). Worktree untouched except the pre-seeded uncommitted line —
`review` is read-only; nothing resembling `init` ran uninvited.

Contract: exec = 204 → poll like a normal turn (existing pump covers
it; no new event handling required beyond what turns already emit);
discovery text (`description` from `GET /api/command`) shown before
executing; confirmation required for file-writing commands (`init`
class) — command metadata does not flag writability, so the
implementation must maintain its own confirm-list starting with `init`
(exact list is implementation scope).

## Updated readiness (supersedes recon §16–§17 rows)

| Operation | Was | Now |
|---|---|---|
| Diff | NEEDS MORE RESEARCH | **READY** (render contract above; client-side patch cap) |
| Revert | NEEDS MORE RESEARCH | **READY** (stage-then-confirm-then-commit; DELETE safe path; 7C-R7) |
| Slash commands | NEEDS MORE RESEARCH | **READY** (turn-equivalent polling; confirm-list for writers) |

Remaining UNVERIFIED micro-items (non-blocking, documented): binary/
truncated diffs, revert-during-generation, fork-during-generation,
compact success-path rewrite, switch-dedicated SSE events. None blocks
the ready set.
