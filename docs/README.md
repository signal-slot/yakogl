<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Documentation

What each place holds. Every file here describes the current state or a
design the code follows; there are no run logs or session diaries.

| Where | What |
| --- | --- |
| `es2-cts-failures.md`, `es3-cts-failures.md` | dEQP-GLES2 and dEQP-GLES3: what is accepted, what fails now and why. |
| `es2/` | The NanoPC-T6 (Mali-G610) dEQP-GLES2 board recipe, `es2/board-recipe-mali.md`: library, deqp-gles2 build, tree table, runner, kernel messages, tally and board rules. |
| `es3/` | ES 3.0 designs per feature, the board recipes, and the current per-case failure lists (`*.tsv`). |
| `online-shaderc/` | The on-board shader compiler: front end, IR, back ends, ISAs and ABIs. |
| `notes/` | Designs and findings that the code relies on (comments cite them), one file per subject. Each file's title line says what it covers: `grep -m1 '^# ' docs/notes/*.md` lists them. This row names no file. |
| `release/` | Release stage: building guide, checklist, licence and redaction audits. |
| `ci.md`, `source-form.md`, `standalone-boot.md`, `v3d-noseamless-cube.md`, `notes/v3d-texture-size-words.md` | The CI gate, the source-form rule, standalone boot, two V3D designs: the non-seamless cube, and the texture size uniform words 44-47. |
| `notes/v3d71-cube-seamless.md` | Mesa's 7.1 cube-map and seamless reading (packets, driver, CI): what the 7.1 table sets for a cube, the U8 prediction, and the case board71-tex runs. |

Elsewhere: board results are `tools/cts/RESULTS.md` (current results only)
and its Japanese summary `tools/cts/STATUS.md`; the stages are `ROADMAP.md`;
the driver's own documents are under `driver/docs/`; component notes sit next
to their code (`src/*_NOTES.md`, `src/*/*_NOTES.md`, `tools/*/README.md`).

## Where a new finding goes

- A board run: its row in `tools/cts/RESULTS.md` (replacing the old one), the
  summary in `tools/cts/STATUS.md`, and a comment on its GitHub issue. No
  session file.
- A design, or a finding the code depends on: `notes/` (or next to the code),
  cited from the code.
- Open work and known defects: GitHub issues, never a file here.
- Anything else (how a run went, what was tried): the commit message.

## Removed records

Board-session records, investigations and diaries that only recorded past
runs were removed on 2026-10-16 (`docs/board-sessions/`,
`docs/investigations/`, the old per-session logs in `tools/cts/log/`, and
the old measurement log in `tools/cts/RESULTS.md`). `tools/cts/log/` has
held files again since e839242b (2026-10-27): the slices that RESULTS.md
rows cite, as `tools/cts/log/README.md` defines them. Comments that still name one write it as
`ab43a20c5:<path>`. The Mali board captures (`tools/mali/board-ref/`) and
the last two session records were removed on 2026-10-26; comments name
them as `0dce654a8:<path>`. The excerpts the tests read are kept in
`tools/mali/board-dump/`.

These hashes, and every commit hash the docs, records and code comments
cite from before the first public commit, name commits of the non-public
working repository the project was developed in. They cannot be read in
the public repository (CONTRIBUTING.md, "Branches and commits").
