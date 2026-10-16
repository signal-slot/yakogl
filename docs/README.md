<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Documentation

What each place holds. Every file here describes the current state or a
design the code follows; there are no run logs or session diaries.

| Where | What |
| --- | --- |
| `HANDOFF.md` | The state of main, the open work in order, the known defects and the next board session. Results add a note under `handoff/` instead of editing it (`handoff/README.md`). |
| `es2-cts-failures.md`, `es3-cts-failures.md` | dEQP-GLES2 and dEQP-GLES3: what is accepted, what fails now and why. |
| `es3/` | ES 3.0 designs per feature, the board recipes, and the current per-case failure lists (`*.tsv`). |
| `online-shaderc/` | The on-board shader compiler: front end, IR, back ends, ISAs and ABIs. |
| `notes/` | Designs and findings that the code relies on (comments cite them): the PowerVR render pool and fence wait, the GC7000 instruments, board bring-up and netboot, and the open investigations of `HANDOFF.md`. |
| `release/` | Release stage: building guide, checklist, licence and redaction audits. |
| `ci.md`, `source-form.md`, `standalone-boot.md`, `v3d-noseamless-cube.md` | The CI gate, the source-form rule, standalone boot, one V3D design. |

Elsewhere: board results are `tools/cts/RESULTS.md` (current results only)
and its Japanese summary `tools/cts/STATUS.md`; the stages are `ROADMAP.md`;
the driver's own documents are under `driver/docs/`; component notes sit next
to their code (`src/*_NOTES.md`, `src/*/*_NOTES.md`, `tools/*/README.md`).

## Where a new finding goes

- A board run: its row in `tools/cts/RESULTS.md` (replacing the old one), the
  summary in `tools/cts/STATUS.md`, and a handoff note. No session file.
- A design, or a finding the code depends on: `notes/` (or next to the code),
  cited from the code.
- Anything else (how a run went, what was tried): the commit message.

## Removed records

Board-session records, investigations and diaries that only recorded past
runs were removed on 2026-10-16 (`docs/board-sessions/`,
`docs/investigations/`, `tools/cts/log/`, and the old measurement log in
`tools/cts/RESULTS.md`). Comments that still name one write it as
`ab43a20c5:<path>`; read it with `git show ab43a20c5:<path>`.
