<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Terrain's overdraw on the host

Counts the overdraw of glmark2 terrain's terrain draw (V3D job 3: the 800x600
terrain target, depth test LEQUAL, depth write on) for
`docs/notes/terrain-overdraw.md`. The host has no GL library and the glmark2
host harness renders nothing, so `overdraw.cpp` is a software rasteriser that
re-creates the draw from glmark2's own sources (method and limits in the note).
It needs g++ only: no GL, no glmark2 checkout, no network.

## Command

```sh
tools/hostharness/terrain-overdraw/run.sh [out dir] [seconds ...]
```

Output directory default `$TMPDIR/terrain-overdraw`; frames default to
glmark2's elapsed time 0 1 2.5 5 7.5 10 s. It writes `overdraw.txt` (one line
a frame), `overdraw-float-depth.txt` (depth compared as float instead of 24
bits, a sensitivity check) and `summary.txt` (the mean), and prints them.
A frame takes a few seconds.

Line fields: `rasterized` (fragments the shader would run on with early Z off),
`covered` (pixels with a fragment), `passed` (fragments that pass LEQUAL as
they arrive: what an exact early Z would shade), `overdraw = rasterized /
covered`, `passed_per_covered = passed / covered`.
