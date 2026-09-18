# Keymap — `owt` Phase 3

Verified live unless marked otherwise. Modifier matching at the element level
is modifier-blind (`TuiEventHandler` matches the bare key name), so bindings
listed with ctrl work because the prompt element declines all ctrl-combos;
a bare key that reaches the session handler implies its modifier was held.

## Prompt (focused input)

| Keys | Action | Verified |
| --- | --- | --- |
| printable chars | insert at cursor | live + unit |
| `enter` | submit | live + unit |
| `ctrl-j`, `shift-enter` | newline (multiline) | live (ctrl-j) + unit (both); shift-enter needs Kitty enhancement, degrades to submit without it |
| `backspace` / `delete` | delete back / forward (joins lines at edges) | live + unit |
| `←` / `→` | move char (across lines) | live + unit |
| `↑` / `↓` | move between prompt lines; at the edge falls through to transcript scroll | live + unit |
| `home` / `end` | line start / end | unit |
| `esc` | close help/menu, else nothing | live |
| `?` on empty prompt | shortcuts overlay (otherwise types `?`) | live |
| paste (bracketed) | insert, multiline-safe, controls sanitized | live + unit |

## Session

| Keys | Action | Verified |
| --- | --- | --- |
| `/`… | slash-command menu (filters, `tab`/`enter` accept, `esc` dismiss, closes on exact match) | live + unit |
| `1` / `2` / `3` on a gate | answer permission/question | live + unit (routing) |
| `pgup` / `pgdn` | scroll page | live |
| `↑` / `↓` (single-line prompt) | scroll line | live |
| wheel up / down | scroll toward older / newer | live (pty SGR) + unit (mapping) |
| `ctrl-p` / `ctrl-n` | previous / next tab (via the strip's own adjacency) | live |
| `ctrl-o` | new session | live |
| `ctrl-t` | simulate agent activity | live |
| `ctrl-c` | clear input → cancel stream → arm exit → exit on second press (1s window) | live |

## Known gaps

* Mouse clicks/hover (links, tabs, collapsibles): wired through Warp-tested
  `TuiHoverable`/`TuiLink`/tab-bar hit targets, but position-dependent clicks
  cannot be synthesized in this sandbox — implemented, not live-verified.
* `shift-enter` without Kitty enhancement arrives as plain enter (submit).
* `j`/`k` type text (prompt is always focused); transcript scroll uses wheel,
  pgup/pgdn, arrows.
