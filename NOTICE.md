# NOTICE

## This project

`opencode-warp-tui` is an experimental standalone agent TUI. The crate as a
whole is **AGPL-3.0-only** (see `Cargo.toml`), because it incorporates
AGPL-3.0-only Warp `warp_tui` snapshots (promoted under `src/tui/widgets/`)
and depends on AGPL Warp workspace crates transitively. This is not legal
advice; get review before distributing derived work, especially hosted or
network use (AGPL §13).

## Warp (upstream reference and partial source origin)

* Origin: `https://github.com/warpdotdev/warp.git`
* Pinned revision: `1bf1c6a2b36bdbb977ad40a380c841182f2995fb` (2026-09-17)
* Copyright: Copyright (C) 2020-2026 Denver Technologies, Inc.
* `warp_tui` snapshots (`src/tui/widgets/*`, originally
  `crates/warp_tui/src/*`): **AGPL-3.0-only** (workspace license). Only change
  from upstream: removed `#[cfg(test)] mod tests;` declarations (Warp test
  harness files were not extracted); see `research/phase2.md`.
* `warpui_core` (used as a git dependency, `tui` feature): **MIT license**
  (Copyright (C) 2020-2026 Denver Technologies, Inc.). Upstream license text:
  Warp `LICENSE-MIT`. The above copyright + permission notice is reproduced
  here per the MIT terms for the substantial portions promoted into this
  crate:

> Permission is hereby granted, free of charge, to any person obtaining a
> copy of this software and associated documentation files (the "Software"),
> to deal in the Software without restriction, including without limitation
> the rights to use, copy, modify, merge, publish, distribute, sublicense,
> and/or sell copies of the Software, and to permit persons to whom the
> Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in
> all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
> FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
> DEALINGS IN THE SOFTWARE.

* AGPL-3.0 license text: Warp `LICENSE-AGPL` (GNU Affero General Public
  License v3, 19 November 2007, © 2007 Free Software Foundation).
* Pristine Phase-1 reference snapshots are preserved unmodified under
  `warp-tui/` (see `warp-tui/NOTICE.md`).
