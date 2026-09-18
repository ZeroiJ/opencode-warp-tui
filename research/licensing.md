# Licensing

Investigated 2026-09-17 against Warp @ `1bf1c6a2b`. This is research notes,
not legal advice — get review before distributing anything derived from Warp
code.

## Repository license (split)

* Workspace root `Cargo.toml:27`: `license = "AGPL-3.0-only"`,
  `publish = false` (`:28`). Default for all member crates.
* `README.md:52-56`: *"Warp's UI framework (the `warpui_core` and `warpui`
  crates) are licensed under the MIT license. The rest of the code in this
  repository is licensed under the AGPL v3."*
* `crates/warpui_core/Cargo.toml:7` and `crates/warpui/Cargo.toml:7`:
  `license = "MIT"`.
* `crates/warp_tui/Cargo.toml:7`: `license.workspace = true` → **AGPL-3.0-only**.
  Same for `warp_core`, `warp_editor` (via `crates/editor`), `warp_terminal`,
  `markdown_parser`, `vim`, `warp_completer`, `ai`, `command` (all
  `license.workspace = true`, verified).
* License texts: `LICENSE-AGPL` (GNU AFFERO GENERAL PUBLIC LICENSE v3,
  19 Nov 2007, (C) 2007 Free Software Foundation), `LICENSE-MIT`
  ((C) 2020-2026 Denver Technologies, Inc.).

## Does reuse appear permitted?

* **MIT parts** (`warpui_core`, `warpui`): yes — use, copy, modify, sublicense
  subject to keeping the copyright + permission notice
  (`LICENSE-MIT:5`: *"included in all copies or substantial portions"*).
* **AGPL-3.0 parts** (everything else, **including the 7 copied
  `warp_tui` snapshots**): yes — copy/modify/distribute permitted, but the
  copyleft applies: derived work must stay AGPL-3.0, carry prominent change
  notices, preserve license notices, and (AGPL §13) offering it over a network
  counts as conveyance (source-offer obligation). No per-file headers exist;
  the root license files + `Cargo.toml` declarations are the notice.
* Other bundled licenses in-repo (fonts, terminal assets, telemetry SDK,
  Alacritty-derived model code) are irrelevant: nothing from those paths was
  copied.

## Attribution requirements

* MIT reuse: reproduce `Copyright (C) 2020-2026 Denver Technologies, Inc.`
  + the MIT permission text.
* AGPL reuse: keep a pointer to the AGPL-3.0 text, the Warp origin URL +
  commit, and `warp-tui/NOTICE.md` alongside the files. Do not strip it.

## Notices preserved in this project

* `warp-tui/NOTICE.md` — origin, commit, copyright, AGPL-3.0-only status.
* The 7 snapshots retain `cp -p` timestamps and original relative paths
  (`crates/warp_tui/src/` → `warp-tui/src/`).

## Needs additional review

1. Confirm AGPL-3.0-only (no "or-later") handling with the project's intended
   distribution (especially hosted/network use) before Phase 2+.
2. Decide: depend on `warpui_core` from crates.io/git (cleanest, MIT intact)
   vs. vendoring (must carry MIT notice).
3. Decide `markdown_parser` replacement vs. AGPL-vendored use for Markdown
   rendering.
4. Re-check licenses at extraction time — the repo moves fast and per-crate
   declarations could change.
