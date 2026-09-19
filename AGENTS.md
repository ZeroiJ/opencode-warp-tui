# AGENTS.md — opencode-warp-tui

Working rules for this project. Warp (`/home/zeroij/warp`) is a
UI/architecture **reference**, not the backend. OpenCode integration stays
isolated from the TUI layer until its dedicated phase.

## DO

* Read source before modifying it.
* Understand dependencies before extracting code.
* Keep the project independent from the original Warp repository.
* Preserve useful attribution/license information (see `warp-tui/NOTICE.md`,
  `research/licensing.md`).
* Prefer clean interfaces over copying tightly coupled backend code.
* Separate presentation from backend logic.
* Keep OpenCode integration isolated from the TUI layer.
* Document architectural decisions in `research/`.
* Keep commits/changes logically separated by phase.
* Test after meaningful changes.
* Prefer small incremental changes.
* Preserve the ability to replace the backend later.
* Treat Warp as a UI/architecture reference, not as the backend.
* Reuse ideas and structures only where legally and technically appropriate.
* Ask for confirmation if a destructive or ambiguous operation is required.

## DON'T

* Do not modify the original Warp repository.
* Do not delete Warp files.
* Do not overwrite Warp files.
* Do not rename Warp files.
* Do not modify Warp's package structure.
* Do not copy the entire Warp repository.
* Do not copy unrelated Warp functionality.
* Do not connect OpenCode yet (adapter lands in a later phase).
* Do not invent APIs.
* Do not assume a file is part of the TUI without reading it.
* Do not hide compilation errors.
* Do not silently replace dependencies with random alternatives.
* Do not create a giant monolithic application.
* Do not tightly couple the future OpenCode backend to rendering code.
* Do not remove license or attribution information.
* Do not claim something is reusable until its dependencies have been checked.

## Phase discipline

* Only the current authorized phase may be worked on. Today: **Phase 5**
  (Memory Engine Foundation: Memory API + `MemoryStore` + JSONL persistence
  + `/memory` command routing; **no memory injection** into any session —
  that is Phase 6; no OpenCode/global config changes).
* `warp-tui/src/` files are pristine Phase-1 AGPL-3.0-only reference snapshots
  (see `research/licensing.md`, `warp-tui/NOTICE.md`). Do not modify them;
  the compilable promotions live under `src/tui/widgets/` (see
  `research/phase2.md`). Do not relicense either copy.
* This crate is AGPL-3.0-only overall (see root `NOTICE.md`).
* Verify the Warp checkout is still clean (`git status --porcelain=v1` shows
  nothing) after any investigation step.
