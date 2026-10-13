<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Hand-over notes

One file per result, folded into `docs/HANDOFF.md` after the result lands.

A result never edits `docs/HANDOFF.md` itself. Every result used to, and two
results landing in the same days always changed the same lines of it ("State
of main", the open work), so every second pull request conflicted with main
and was redone from scratch (2026-10-13: 77 of 79 redos in 36 hours were
conflicts, most of them in this one file). A note of its own cannot conflict
with anyone else's.

## Writing a note

- File: `docs/handoff/<YYYYMMDD>-<topic>.md`, the date of the result and a
  short topic (`20261013-es2-error-rules-state.md`). Lower case, `-` between
  words. One result, one file; a follow-up to the same work is a new file.
- Content: exactly what `docs/HANDOFF.md` should say after this result, as
  edits to its sections, each under a heading that names the section:

  ```markdown
  # <topic>, <YYYY-MM-DD>, <commit or pull request>

  ## State of main
  <rows to add or change, in the table's format; "main is <sha>" if it moves>

  ## Open work
  <items done (which numbers), changed, or added, in the list's format>

  ## Known defects
  <defects opened or closed, with their issue numbers>

  ## Next board session
  <only when the result changes what the next session must do>
  ```

  Leave out a section the result does not touch. Cite commits, records and
  issue numbers as HANDOFF.md does. Addresses, UART paths and credentials
  never go in a note.
- `tools/handoff-notes.sh` lists the notes waiting to be folded; `--check`
  fails while any remain, which the fold's agent runs last.
- The other docs of a result (`tools/cts/RESULTS.md`, `tools/cts/STATUS.md`,
  `README.md` tables) are still edited directly in the same pull request:
  their rows are per board or per run and seldom collide.

## Folding

ccfarm's `handoff-fold` stage (`.ccfarm/project.toml`) runs after every
merge into main that leaves a note here. Its agent folds each note into the
section it names, updates "Last update" and "main is", deletes the note, and
its change lands as a pull request like any other. The gate does not check
notes: a malformed one is the fold's problem, not the result's.

A person folds the same way: edit `docs/HANDOFF.md`, delete the notes, one
pull request. `tools/handoff-notes.sh --check` passes when nothing waits.
