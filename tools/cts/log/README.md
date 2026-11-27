<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# tools/cts/log — board results, one directory per slice

`tools/cts/log/` is where board results reach the tree. A run's raw output
stays under `records/` (gitignored): the per-run directories
`tools/board.sh` writes, with `meta.txt`, `console.txt`, the bridge's
`console.raw` / `console.ts` and `INDEX.txt`. A **slice** is what a board
session commits here: one dated directory, redacted, holding the record and
the files its numbers are re-read from. The RESULTS.md row a slice supports
is replaced by the next run; the slice is the evidence and is not edited or
renamed afterwards.

## Directory name

`<date>-<board>-<set>-<sha8>[-<suffix>]`

- `date`: the run day `YYYY-MM-DD` (board clock). Slices of 2026-11 whose day
  was not kept carry `YYYY-MM` alone;
- `board`: `am62p`, `imx8mp`, `pi4` (`tools/board.sh`'s names), or `t6`,
  `imx95`, `mp2` for the boards the slice was run on by ssh or by hand;
- `set`: what ran, the tree file's stem (`gles3` for
  `tools/cts/am62p-gles3-tree.txt`), `gles2`, `es2`, `gltest`, `glmark2`,
  `sweep`, `uboot`; for `t6-gles2` and `imx95-gles2` the session
  (`smoke`, `S1` … `S4`, with a letter for a split or rerun) follows;
- `sha8`: the `origin/main` commit the board images were built from —
  board images are built from `origin/main` alone;
- `suffix`: what tells slices of one commit apart: `rowsAA-BB` when a tree
  ran in several slices (the rows of the tree file this slice covers,
  counted from 1: `rows01-15`; `rows00-15` where a table counts from 0),
  or a word (`aborts`, `aborts2`, `map`, `rerun`, `fbo`, `tex1`, `s32c`).

Earlier slices predate this name and stand as they are: the Mali session
records (`<date>-<set>`, described by `tools/mali/sweep_passsets.py`), the
Pi 4 slices `2026-10-31-pi4-gles3-a`, `2026-11-pi4-gles3-b` and
`2026-11-pi4-gles3-c` (`<date>-<board>-<set>-<letter>`, the plan item's
letter), and the MP2 slices `2026-11-mp2-gles2-p9` and `2026-11-mp2-gltest-p9`
(`<set>-<phase>`, no commit in the name).

## Slices in this directory

Every directory below holds a slice; each is named here and no other exists.

- Mali session records (`<date>-<set>`):
  `2026-10-27-frameboard`, `2026-10-28-boardref`, `2026-10-28-final`,
  `2026-10-28-mid`, `2026-10-28-splitref`, `2026-10-28-tip`,
  `2026-11-01-final3`, `2026-11-02-pr-217probe`, `2026-11-07-t6-step4a`.
- AM62P partial-render negatives: `2026-10-31-pr-negative`,
  `2026-10-31-pr-neg175`.
- Pi 4 dEQP-GLES3: `2026-10-31-pi4-gles3-a`, `2026-11-pi4-gles3-b`,
  `2026-11-pi4-gles3-c`, `2026-11-17-pi4-gles3-351fb70d-rerun`,
  `2026-11-18-pi4-gles3-ac2fe51f-map`, `2026-11-20-pi4-gles3-21730230-S03-S08`,
  `2026-11-pi4-gles3-625b9f5c-aborts2`, `2026-11-pi4-gles3-87881c78-aborts-a`,
  `2026-11-pi4-gles3-87881c78-es2`, `2026-11-pi4-gles3-87881c78-fbo`,
  `2026-11-pi4-gles3-87881c78-tex1`, `2026-11-pi4-gles3-87881c78-tex2`,
  `2026-11-pi4-gles3-a1b8a613-map`, `2026-11-pi4-gles3-b1b5b845-aborts`,
  `2026-11-pi4-gles3-b1b5b845-aborts2`, `2026-11-pi4-gles3-b1b5b845-aborts3`,
  `2026-11-pi4-gles3-b1b5b845-rows00-15`, `2026-11-pi4-gles3-b1b5b845-rows16-30`, `2026-11-pi4-gles3-b1b5b845-rows31-45`, `2026-11-pi4-gles3-b1b5b845-rows46-60`, `2026-11-pi4-gles3-b1b5b845-rows61-75`, `2026-11-pi4-gles3-b1b5b845-rows76-90`,
  `2026-11-pi4-gles3-b1b5b845-s32c`,
  `2026-11-pi4-gles3-fad0558a-rerun`, `2026-11-pi4-gles3-fad0558a-rerun-s4`.
- Pi 4 ES 2.0 and gl_test: `2026-11-pi4-es2-b1b5b845`,
  `2026-11-pi4-es2-fad0558a`, `2026-11-pi4-gltest`,
  `2026-11-pi4-gltest-87881c78`.
- i.MX8MP dEQP-GLES3: `2026-11-imx8mp-gles3-9bc82915-rows01-13`,
  `2026-11-imx8mp-gles3-9bc82915-rows17-34`,
  `2026-11-imx8mp-gles3-9bc82915-rows35-51`.
- MP2: `2026-11-mp2-gles2-p9`, `2026-11-mp2-gltest-p9`.
- NanoPC-T6 (`t6-gles2` slices are described in their own section below):
  `2026-11-12-t6-gles2-S1-a50d87b5`, `2026-11-12-t6-gles2-S1b-a50d87b5`,
  `2026-11-13-t6-gles2-S2-1bd2074f`, `2026-11-13-t6-gles2-S2b-1bd2074f`,
  `2026-11-13-t6-gles2-S3-4ecefc7b`, `2026-11-13-t6-gles2-S3b-4ecefc7b`,
  `2026-11-13-t6-gles2-S4-9b5daae8`, `2026-11-15-t6-gles2-S1-37305608`,
  `2026-11-15-t6-gles2-S2-37305608`, `2026-11-15-t6-gles2-S3-37305608`,
  `2026-11-15-t6-gles2-S4-37305608`, `2026-11-21-t6-gles2-R373-a2482523`,
  `2026-11-24-t6-gles2-S1-8ff01055`, `2026-11-24-t6-gles2-S2-8ff01055`,
  `2026-11-24-t6-gles2-S3-8ff01055`, `2026-11-24-t6-gles2-S4-8ff01055`,
  `2026-11-26-t6-gles2-R406-c8fbf34e`, `2026-11-25-t6-glmark2-038b7d7d`,
  `2026-11-25-t6-glmark2-22c5fc5f`, `2026-11-25-t6-uboot-65af54b5`,
  `2026-11-26-t6-sweep-d9efa418`, `2026-11-t6-sweep-1a22bd38`,
  `2026-11-t6-sweep336`, `2026-11-t6-uboot-s1-cbb55860`.
- i.MX95 (`imx95-gles2`, same files as the `t6-gles2` slices):
  `2026-11-24-imx95-gles2-S1a-e63ab28e`, `2026-11-24-imx95-gles2-S1b-e63ab28e`,
  `2026-11-24-imx95-gles2-S2-e63ab28e`, `2026-11-24-imx95-gles2-S3a-e63ab28e`,
  `2026-11-24-imx95-gles2-S3b-e63ab28e`, `2026-11-25-imx95-gles2-S4-e63ab28e`.

## README.md: the record itself

- **What ran, against what**: the commit the board images were built from
  (an `origin/main` commit: 8-hex short and full sha) and, when the tree was
  built from a commit on top of main (Mali's prediction commits), that commit
  too; the plan item and issue; each image's sha256 and build stamp.
- **The exact commands**, with the knobs in effect: host environment names
  (`BOARD_BOOT=uboot-mmc`, `TREE_ROWS`, `TREE_IMAGE_DIR`, `RUN_BOUND` —
  names only, never values) and the Kconfig compiled into the image
  (`CONFIG_DEQP_PVR_NATIVE_3D`, `CONFIG_GL_TEST_CASES`, the watchdog arm).
  When a farm console lease held the bridge and the board steps were done
  by hand, each `tools/board.sh` step is listed in its order, as
  `2026-10-31-pi4-gles3-a` does.
- **One verdict line per row, verbatim as `tools/board.sh` prints it** (or
  recomputed by `tools/board.sh tally`): `board.sh am62p img12 rc=0
  verdict=done boot=uboot-mmc row boot_verdict=done result=... resets=absent
  record=... back_after=531s`. A clean row is `verdict=done` with `resets`
  absent or 0; `done_with_resets`, `done_lossy` (with `lost=<n>`),
  `done_stopped` (dEQP ended the session early),
  `scan_failed`, `died_after=<stage>` and a row that took the 600 s RTI
  watchdog are named as such, with the rerun that replaced them.
- **Resets**: per row and for the session — GPU recoveries (AM62P: from the
  console, `board.sh`'s `pvr_resets`) and board reboots (an unchanged
  `boot_id` across the session shows none).
- **Span**: the session window (board clock, timezone) and each row's time
  (`back_after`; for dEQP also the span from the
  first to the last `Test case` line, `tools/cts/README.md` section 2e).

## cases.tsv — per-case status

One line per mustpass case the slice's rows selected:
`<case name><TAB><Status>`. The Status is dEQP's own word — `Pass`, `Fail`,
`NotSupported`, `QualityWarning`, `CompatibilityWarning`, `ResourceError`,
`InternalError`, `Crash`, `Timeout` — and `NoResult` for a case of an
aborted image that never began (`tools/cts/README.md` section 3). It is read
off the rows' consoles: dEQP prints `deqp: <Status> <case name>` per case
(`tools/cts/deqp-uart-tally.py` keeps each case's last status; its
`--statuses` view gives the same pairs space-separated). A case no row's filter selects is
absent from the file: not run is not NoResult.

## totals.txt

The numbers the RESULTS.md row cites: mustpass cases in the slice's scope,
begun, with a result, then the per-status totals (pass / fail / n.s. /
QualityWarning / CompatibilityWarning / other / none), GPU recoveries, and
the per-row begun / span / verdict lines measured.

## Other files a slice holds

Whatever its record re-reads from: per-row consoles (`<LABEL>.console.txt`)
and tallies (`<LABEL>.tally.txt`), the whole-slice `table.txt`, the boot
`cycles.txt`, `*.verdicts.txt` and `predictions.txt` (the Mali records),
board reference programs (`*.c`), scrubbed console excerpts. Every committed
`.txt` starts with three `#` lines (SPDX twice, then a title), as the
existing log text files do; the tally reads past them. Raw captures stay
out.

## t6-gles2 slices: the NanoPC-T6 dEQP-GLES2 runs

`<YYYY-MM-DD>-t6-gles2-<session>-<sha8>` (`<session>` is `smoke`, `S1` … `S4`,
as the runner's `--session`; `<sha8>` is the `commit` of `meta.txt`) is what
`tools/cts/mali-deqp-run.sh --out` writes, redacted, from a run of
`tools/cts/mali-gles2-tree.txt` rows over the board's Armbian
(`tools/cts/README.md` section 6). It holds no README of its own kind of
console: the board's output is the files below, and the record's text is the
commit message and the RESULTS.md row. Its files:

- `meta.txt`: `key=value` lines: `commit` (the `origin/main` commit the
  `deqp-gles2` and the `libGLESv2.so.2` / `libEGL.so.1` were built from; the
  runner refuses a build whose commit is not on `origin/main`), `session`,
  `rows` (`A-B` of the tree table), `reruns` (only in a rerun slice:
  the earlier slice directory names it replaces the Lost rows of, comma-separated;
  the runner's `--reruns`), `tree`, `lib` (the libraries' build stamp or
  sha256), `started`, then `boot_id_before`, `kernel` (`uname -r`),
  `boot_id_after` and `ended`. `boot_id_after=unknown` marks a run that stopped
  early.
- `chunks.tsv`: a header line, then one line per tree row run:
  `row label cases status seconds kmsg resets ssh_after boot_same exit_line`.
  `status` is `done`, `crash`, `hang`, `fault` (kernel lines with panthor
  resets or faults between the chunk's markers) or `lost`; `kmsg` is the
  number of kernel lines between the markers, `resets` the panthor job-timeout
  and reset lines among them, `ssh_after` whether the board answered ssh
  after the chunk (`yes`/`no`), `boot_same` whether `boot_id` was unchanged,
  `exit_line` the last `yakogl-mali-exit:` line. `-` is an unknown value (a row
  never run, a lost capture).
- `results.tsv`: a header line, then `case status detail` per case of the rows:
  dEQP's status and its Result text, or `Lost` with `chunk-crash`,
  `chunk-hang`, `board-lost`, `not-run` or `wrong-lib` as the detail.
- `refusals.tsv` (slices since 2026-11-15): a header line, then `case refusal`
  for each case whose log holds one: the first shader or program info log in
  which the library refuses, whitespace folded (dEQP logs an info log in two
  forms, `<InfoLog>` and a `Section` named `Info Log`; both are read). The
  tally takes a case's cause from it.
- `refusals/<label>.txt` (optional): one chunk's stdout and stderr merged, in
  the order the process wrote them: dEQP's `Test case '<case>'..` lines and
  the backend's per-draw lines `yakogl-mali-refusal: <errno> <reason>`
  (`<reason>` is the bare word `errno` when the cause was already logged
  elsewhere). Only `mali-deqp-tally.py --tally --per-case` reads it: a
  refusal belongs to the last `Test case` line before it, the case's first
  line is its cause, and a case without one, or a chunk without the file,
  keeps the default cause. The `yakogl: a draw was refused:` line (first
  refusal of the process only) is not used. Caveat: dEQP's stdout buffering
  into a file can reorder lines against the `Test case` lines, so check the
  order in the raw `.out` before trusting a slice; the library's rate limit
  also writes nothing for two consecutive cases that fail on their first
  draw for one reason (`docs/notes/mali-frame-refusals.md`).
- `kmsg/<label>.txt`: the kernel lines between that chunk's markers, to be
  classified with `tools/cts/mali-kmsg.py` (`docs/notes/mali-panthor-kmsg.md`).
- `<file>.license` beside each of them (the runner writes it since
  2026-11-15): the licence of our results.

The raw `.qpa`, stdout and stderr and the whole `dmesg` capture stay in the
runner's `RAW_DIR` and are never committed. A slice is checked for the
redaction rules below by the runner itself; read it again before committing.
`tools/cts/mali-gles2-tree.txt` is the four-session tree table and
`tools/cts/mali-deqp-tally.py` the cover check and tally over slices
(`docs/es2/board-recipe-mali.md` section 7); this section describes the
slice, not them.

## Redaction: what must never appear

Board addresses are environment (CLAUDE.md): nothing under `tools/cts/log/`
carries `BOARD_HOST` or `BOARD_UART` values — no user@host logins, no
machine or board host names or SoM serials, no tailnet / LAN / IPv6
addresses, no `host:port` pairs, no port numbers, no `/dev/tty…` paths or
UART device names, no credentials or keys. Environment **names** are fine;
values are not. `tools/board.sh redact_dir` scrubs the copies it keeps:
host words → `<board-host>`, IPv4 → `192.0.2.<n>`, MAC →
`02:00:00:00:00:<nn>`. A hand-made console excerpt is scrubbed the same way
— keep the run's result lines, drop login banners and pointer dumps, name
what remains `<board-host>`, `<host:port>` or `<ipv6>` — and before
committing, read the file for `@` logins, dotted-quad numbers, `:`-ports
and `/dev/`.
