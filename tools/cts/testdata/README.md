<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/cts/testdata/`

Fixtures for the tools in `tools/cts/`, so a parser can be checked without a
board and without a 40-minute sweep.

`refusals-new-format.qpa` — three `TestCaseResult` blocks in dEQP's own XML,
carrying the online compiler's message format from `fc88b41`:

    0:59: &apos;.&apos; : assignment to this is not lowered yet

It exists because that format nearly cost a whole sweep. `cts-refusals.py`'s
first pattern excluded `&` from the message so it would stop at an entity —
correct while the message had no token, and silently capturing **nothing** the
moment one appeared, because dEQP escapes the quotes around the token as
`&apos;` and they begin immediately after the colon. Every refusal would have
bucketed under the empty string, and the histogram would have been one nameless
row. The fixture pins:

- the entity-escaped token is decoded and split from the message
- a case dEQP marked `Pass` is counted as a **correct** refusal, not a gap
- one message over two different tokens splits by construct

`refusals-text-and-notsupported.qpa`: the other refusal forms
`cts-refusals.py` reads. A code-generator refusal written in a `<Text>`
element (not an `<InfoLog>`), one in an `<InfoLog>` with an
entity-escaped message, and a case dEQP marked `NotSupported`. Check 16
of `cts-selftest.sh` runs `cts-refusals.py` over the whole directory, so it
reads both `.qpa` files and expects, among others, one refusal on a
NotSupported case (excused, not a gap) and the
`Usc(Record(MatrixAttrib { index: 0 }))` reason.

`by-message.qpa` — a batch log as deqp-gles3 writes one (`#beginTestCaseResult`
lines around each case, which the CTS's own log parser needs and the two
refusal fixtures do not have), for `cts-summary.py --by-message`
(`cts-selftest.sh` check 21). Session header from a real host run; the four
cases are scaffolding: one Pass, "Got invalid error", and two "Compile failed"
details that differ only in the shader line `0:12:` / `0:7:`, which must fold
into one row of 2. It carries no info log, so `cts-refusals.py`'s counts over
this directory (check 16) do not change.

`deqp-config/` — resolved Zephyr configurations in the form a build writes
its `zephyr/.config` (`CONFIG_X=value` lines, a disabled bool as
`# CONFIG_X is not set`), fed to `check-deqp-images.py --config` by
`cts-selftest.sh` check 17. The dEQP image builders run that same check on
every image they build, and the workers have no Zephyr build, so these
fixtures are its only host evidence:

- `am62p-sweep.config`, `imx8mp-sweep.config`, `pi4-sweep.config` and
  `am62p-ms4.config` (with `--ms4`) — one conformance image per board plus
  the multisample one; the check must pass each
- `am62p-gles3.config`: an AM62P dEQP-GLES3 image; the check must pass
  it with `--gles3`
- `pi4-gles3-sweep.config` — a Pi 4 dEQP-GLES3 image
  (`pi4-deqp-images.sh` with `DEQP_MODULE=gles3`: both `REPORT_ES2` symbols
  off); the check must pass it with `--gles3` and refuse it without, naming
  `YAKOGL_REPORT_ES2`, and must refuse `pi4-sweep.config` with `--gles3`
- `bad-*.config` — one broken image per rule the check enforces: the
  compiler off (`bad-compiler-missing`), the Mesa table chosen
  (`bad-shaderc-offline`), precompiled-only (`bad-precompiled-only`), the
  precompiled tier in front of the compiler (`bad-precompiled-tier`), ES 3.0
  reporting (`bad-report-es3`), the sample's ES 2.0 switch gone
  (`bad-gles3-report`), a sweep image on the multisample render config
  (`bad-sweep-ms4-name`), and the multisample image without it
  (`bad-ms4-name-missing`); the check must fail each with a message naming
  the offending symbol

`glmark2-*.txt`: GM console snippets in board.sh's `console.txt` form
(`HH:MM:SS line`), for `common-tally.py`'s per-scene verdict
(`--self-test`, and `cts-selftest.sh` check 22, which wraps two of them in
Pi 4 run directories with the round-3 GLMARK2 record). The known-bad lines
are in their source's format string and quoted in `tools/cts/RESULTS.md`; the
scene lines are the glmark2 sample's (`main.cpp` print_mem and print_cost,
upstream's `[scene] <options>: FPS:` line). The numbers in the memory and
frame lines, the times and the repeat counts below the real ones are
scaffolding:

- `glmark2-clean.txt`: five scenes (jellyfish, terrain, shadow, refract,
  conditionals) inside their `glmark2-mem:` before setup / end lines, no
  known-bad line: cell green
- `glmark2-terrain-0x502.txt`: terrain's 16 `GL error 0x502 in
  glGenerateMipmap` and its `a texture on unit 0 is incomplete and samples
  (0,0,0,1)` line (RESULTS.md, the gl_test case 180 section); shadow clean
- `glmark2-refract-refused.txt`: the AM62P reclaim round's refract (RESULTS.md
  "glmark2 cell AMBER"): `yakogl/pvr: draw refused: no depth/stencil buffer
  for a 1600x1200 target; ...` once and `glDrawArrays was not recorded (-12);
  the frame is missing it` per frame (4 here, 110 on the board), with no
  `glmark2-mem:` lines, so a line goes to the next scene's result line
- `glmark2-pi4-oom.txt`: the Pi 4's refract `osal(v3d): out of GPU memory
  for 7684096 bytes (...)` and `no GPU memory for a tiled copy of 7684096
  bytes (...)` (3 here, 39 on the board), terrain's tiled-copy refusals of
  1404928 and 356352 bytes (2 here, 399 on the board); shadow clean

- `glmark2-mem-nameless.txt`: terrain (two `GL error 0x502` lines) and shadow with
  the `glmark2-mem:` lines in the sample's real format, `entry N before setup|end:`
  with no scene name (the fixtures above use the older `entry N <info>:` form):
  the tally must not print "no glmark2-mem lines"

`glmark2-speed-a.txt`, `glmark2-speed-b.txt`: two small synthetic runs for
`common-tally.py --gm-table` and `--gm-diff` (`cts-selftest.sh` check 22b).
Result and `glmark2-frame:` lines only, no memory lines. A has build, terrain,
shadow (20 FPS) and an unsupported refract, score 150. B has the same plus
refract (10 FPS) and a new jellyfish; build moves 2%, terrain 0%, shadow
-20% (marked), score 160. All numbers are scaffolding.

`glmark2-glued-header.txt`, `glmark2-twice-before-frame.txt`,
`glmark2-failed-then-fps.txt` (`cts-selftest.sh` check 22b2): rare shapes of
the console for `common-tally.py --gm-table`, none with `glmark2-mem:` lines.
glued-header: two scenes whose header has driver text glued after it and no
`glmark2-frame:` line; the scene name keeps all its leading options.
twice-before-frame: one scene run twice, then two `glmark2-frame:` lines; each
goes to its own run, oldest first. failed-then-fps: a run that set up failed,
then the same scene with an FPS and one `glmark2-frame:` line; the line goes
to the run that has an FPS (a failed run prints none).

`glmark2-picture-{match,mismatch,black,absent}.txt`, `glmark2-pictures-test.txt`:
three-scene consoles with `glmark2-picture:` lines, and a reference table with
a `pi4` column of those scenes, for `common-tally.py`'s picture verdict
(`cts-selftest.sh` check 22c and `--self-test`). match: terrain's centre probe
3 levels off and its crc16 different (still a match); mismatch: terrain's
centre probe and mean far off; black: shadow all black; absent: terrain's
readback failed. All values are scaffolding, not a real picture.

`ladder/n4_k*.vert`: the vertex shaders of a GC7000 wedge ladder (N = 4,
K = 20 to 30). `tests/ladder_rungs_compile.rs` includes each with
`include_str!` and fails if the compiler refuses one, so a rung read as
"wedged" on the board was not a compile refusal.

`wedge-shaders/`: the four dEQP shaders of the closed GC7000 hang
investigation (`wedge-shaders/README.md`). No test reads them.

`imx8mp-pages/`: an i.MX8MP outdir whose GM row has no console.
`glmark2-pages.txt` is `tools/board.sh imx8mp gm-lines` run on the 18 synthetic
page runs of `driver/rust/tests/gm-record/runs` (byte for byte
`driver/rust/tests/gm-record/glmark2-pages.txt`: F pages 0-8 and 10, P pages of
scenes 0, 21, 22, 25, a RATE record, which no row reads). The run directory
holds a synthetic GLMARK2 record; `tree.txt` names it. `cts-selftest.sh` check
22d reads the rows, `--gm-table`, `--gm-diff` and `--gm-pictures` over it.
