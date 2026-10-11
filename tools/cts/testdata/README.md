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

`cluster-run/20261004-0438-cluster-11236a24/` — a board.sh run directory
replaying the Pi 4 cluster row of 2026-10-04 (the view run, tree `031de9a2`,
`tools/cts/RESULTS.md`), so `common-tally.py`'s CL row can be checked without
a board. `cts-selftest.sh` check 18 and `common-tally.py --self-test` both
replay it. What is real and what is scaffolding:

- `console.txt`'s four `yk-record:` lines are the cluster's record sequence:
  FINGERPRINT, PERF, APP, PROBES (the verdict). The APP and PROBES words are
  verbatim from RESULTS.md; the PERF words carry that run's printed numbers
  (139.8 fps, frame 7.1 / 30.6 ms mean / worst, draw 3.54 ms/Mpx, show 0.1 ms)
  packed as `check/src/lib.rs` packs them; the FINGERPRINT line is the Pi 4
  cluster console form pinned in `recordcmd.rs`'s `throughput_from_a_log`
  (its crc32 is that earlier run's — RESULTS.md does not record the view run's)
- `meta.txt` is scaffolding: the real run's sha is `11236a247d72bdf0...`
  (truncated in RESULTS.md; the fixture pads the tail with zeros), the build
  ids are the run's console build line (`tree=031de9a2 table=1449eb0f`), and
  the times only carry the run's "back after 179 s"

The tally must read from it: verdict `done` from the PROBES record, "cluster
probes pass", `resets (console count) 0`, and the throughput line
`1920x720: 8395 frames in 60019 ms, 139.9 fps, 193.4 Mpx/s; draw 4.9 ms mean,
3.54 ms/Mpx` — the tool rounds the fps where the cluster's console truncated
it to 139.8.

`ladder/n4_k*.vert`: the vertex shaders of a GC7000 wedge ladder (N = 4,
K = 20 to 30). `tests/ladder_rungs_compile.rs` includes each with
`include_str!` and fails if the compiler refuses one, so a rung read as
"wedged" on the board was not a compile refusal.

`wedge-shaders/`: the four dEQP shaders of the closed GC7000 hang
investigation (`wedge-shaders/README.md`). No test reads them.
