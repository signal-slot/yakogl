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

- commit: the console's `sample: build tree=<sha8>[-dirty] table=...` line
  (`-dirty` kept); else a stage-notes `commit <sha8>`; else the label in the
  verdict line (`board.sh pi4 glmark2-<sha8> ...`). Never the run name's sha8,
  which is the image's sha256: it prints on its own line, `image sha256`.
- verdict line: `INDEX.txt`, `verdict: board.sh <board> <name> rc=.. verdict=..`
  (`tools/board.sh`, "The verdict line").
- records path: the run directory.
- terrain FPS and ms a frame: `[terrain] <info>: FPS: N FrameTime: X ms`, on
  one line or, as on the board, with other lines between `[terrain] <info>:`
  and ` FPS: N FrameTime: X ms` (the next `[other-scene] ` line ends the wait);
  and the `glmark2-frame: [terrain]` line (gl, swap, show, other, frame us).
- Pi 4: the highest-frame `yakogl/v3d: TFU mipmap at frame N: C chains` and
  `tiled copies for draws at frame N, by reason: ...` lines (cumulative).
- AM62P: the count of `render: fence N ... re-posted K kick(s) (M so far)`
  lines, and the last `pvr last completed kick per frame` line, verbatim.

- `glmark2-counters: <scene info>: frames N <key> <value> ...`
  (`docs/notes/glmark2-scene-counters.md`): terrain's line only. Values are
  printed per frame (value / N); `rt.live`, `rt.bytes`, `tile.untimed` and
  `fl.stop` are levels and print as they are. `absent` when the line is
  missing or reads `glmark2-counters: <scene>: absent` (the GC7000 image). The
  two-run table carries a `counter <key> (a frame)` row per key.

- `glmark2-jobs: <scene info>: frame N: J job(s): <job>; <job>; ...`
  (Pi 4 image, `driver/samples/zephyr/glmark2/src/main.cpp` print_jobs),
  terrain's line only. `<job>` = `total T render R[~] idle I[ tfu K/Tus]` and,
  on images built after the bin/prep commit, ` bin B prep P` last (us). Both
  formats are read; an old line gives `bin`/`prep` as none and the block says
  `old format`. The block sums render, idle, bin and prep over the listed jobs.
  `absent (...)` lines are skipped.

The `glmark2-counters` lines in `testdata/` are made up, written to the design,
until a GM image prints them. The other counters are cumulative over the whole console, not terrain's alone.
`testdata/` is synthetic (no board address, host name or UART path).
