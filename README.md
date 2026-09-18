# OpenCode Warp TUI

A standalone experimental TUI project intended to reproduce/adapt the useful
interaction and presentation patterns of Warp's CLI/agent interface while using
OpenCode as the eventual backend.

> **Status: PHASE 2 — standalone foundation on a mock backend.**
> The crate compiles to a working TUI (`cargo run`, binary `owt`) that renders
> Warp-style sessions from scripted mock data. NOT connected to OpenCode yet
> (no SDK, no API calls); the OpenCode adapter lands in a later phase.

## Layout

```text
~/opencode-warp-tui/
├── README.md            # this file
├── AGENTS.md            # working rules for this project
├── NOTICE.md            # attribution + license notices
├── Cargo.toml           # crate `opencode-warp-tui`, binary `owt` (AGPL-3.0-only)
├── mise.toml            # pinned Rust toolchain (project-scoped)
├── src/
│   ├── main.rs          # App bootstrap + TUI driver loop
│   ├── theme.rs         # semantic style seam (replaces Warp's TuiUiBuilder)
│   ├── backend/
│   │   ├── mod.rs       # Backend trait + session/block models
│   │   ├── stream.rs    # shared StreamEvent applier (mock + adapter)
│   │   ├── mock.rs      # scripted MockBackend (validation only)
│   │   └── opencode/    # live adapter (client/config/events/mapper)
│   └── tui/
│       ├── session.rs   # session surface (tabs, transcript, menu, prompt, statusline)
│       ├── transcript.rs# block → Warp-styled rows + markdown-lite
│       ├── prompt.rs    # prompt state + cursor element
│       ├── menus.rs     # slash-command inline menu
│       ├── statusline.rs# footer statusline
│       ├── widgets/     # promoted Warp snapshots (see NOTICE.md)
├── scripts/
│   └── pty_probe.py     # live pty verification harness (18 checks)
├── research/
│   ├── warp-tui-map.md      # what/where the Warp TUI is
│   ├── dependency-map.md    # component → Warp dependency table
│   ├── extraction-plan.md   # what to extract / adapt / rewrite
│   ├── licensing.md         # license findings (read before reusing code)
│   ├── phase2.md            # Phase-2 decisions, changes, verification
│   ├── phase3.md            # Phase-3 decisions, changes, verification
│   ├── phase4.md            # Phase-4 adapter, mappings, integration results
│   ├── opencode-architecture.md  # OpenCode API/event findings with sources
│   └── keymap.md            # verified key bindings
└── warp-tui/            # pristine Phase-1 reference snapshots (frozen)
    ├── NOTICE.md
    └── src/             # 7 presentation-only files, unmodified
```

## Source reference

All findings describe Warp at commit `1bf1c6a2b36bdbb977ad40a380c841182f2995fb`
(2026-09-17), remote `https://github.com/warpdotdev/warp.git`.
The original repository was not modified during this research.

## Run it

```sh
cargo run                                  # mock backend (default)
cargo run -- --backend opencode            # live OpenCode server
```

Try (mock): type text, `enter` to submit (`ctrl-j` for newline), `/` for the
command menu, `/demo b` for a streaming answer, `?` for shortcuts,
`ctrl-t` to simulate agent activity, `ctrl-p/ctrl-n/ctrl-o` for sessions,
mouse wheel / `pgup/pgdn` to scroll, `ctrl-c` (×2) to exit.

## Phases

1. **Phase 1:** repository investigation and TUI extraction. ✅
2. **Phase 2:** standalone compilable foundation on mock data. ✅
3. **Phase 3:** hardened frontend — streaming pump, multiline input,
   hosted Warp tab strip, clean backend boundary. ✅
4. **Phase 4 (this):** OpenCode adapter — `OpenCodeBackend` speaks to a live
   server through the same generic trait (`--backend opencode`; default stays
   mock). ✅
2. Phase 2: make the extracted TUI compile independently.
3. Phase 3: remove Warp-specific backend dependencies.
4. Phase 4: create a clean backend interface.
5. Phase 5: implement an OpenCode backend adapter (official/current SDK/API).
6. Phase 6: connect sessions, messages, tool calls, streaming, permissions, files.
7. Phase 7: reproduce the Warp/Claude-Code/Devin interaction model.
8. Phase 8: UX, keyboard navigation, layout, performance, accessibility.
9. Phase 9: package as a standalone OpenCode TUI frontend.
10. Phase 10: installation and update instructions.

Do NOT proceed to Phase 2 without explicit instruction.
