<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# terrain-records

`parse.py` pulls terrain's board-run fields out of a records directory, so the
before and after notes (`docs/notes/pi4-slow-terrain.md`,
`am62p-slow-terrain.md`) record each run the same way.

    python3 -I tools/hostharness/terrain-records/parse.py <dir|log> [<later dir|log>]
    python3 -I tools/hostharness/terrain-records/parse.py --selftest

The argument is a run directory (`records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/`
with `console.txt`, `INDEX.txt`), a stage directory holding several runs (the
last run with a terrain line is used; a `stage-notes.txt` `commit <sha8>` line
beats the run name's sha8), or a console log. `HH:MM:SS ` prefixes are stripped.
Given two, it prints both blocks and a table of the changes (second - first).

Fields, with the line each comes from (`absent` if the line is not there):

- commit: the run name's `<sha8>`.
- verdict line: `INDEX.txt`, `verdict: board.sh <board> <name> rc=.. verdict=..`
  (`tools/board.sh`, "The verdict line").
- records path: the run directory.
- terrain FPS and ms a frame: `[terrain] <info>: FPS: N FrameTime: X ms`;
  and the `glmark2-frame: [terrain]` line (gl, swap, show, other, frame us).
- Pi 4: the highest-frame `yakogl/v3d: TFU mipmap at frame N: C chains` and
  `tiled copies for draws at frame N, by reason: ...` lines (cumulative).
- AM62P: the count of `render: fence N ... re-posted K kick(s) (M so far)`
  lines, and the last `pvr last completed kick per frame` line, verbatim.

The counters are cumulative over the whole console, not terrain's alone.
`testdata/` is synthetic (no board address, host name or UART path).
