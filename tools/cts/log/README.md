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

`<YYYY-MM-DD>-<board>-<set>-<sha8>[-rowsAA-BB]`

- `YYYY-MM-DD`: the run day (board clock);
- `board`: `am62p`, `imx8mp` or `pi4` (`tools/board.sh`'s names);
- `set`: what ran, the tree file's stem (`gles3` for
  `tools/cts/am62p-gles3-tree.txt`);
- `sha8`: the `origin/main` commit the board images were built from —
  board images are built from `origin/main` alone;
- `-rowsAA-BB`: only when a tree ran in several slices: the rows of the
  tree file this slice covers, counted from 1 (`-rows01-15`).

Earlier slices predate this name and stand as they are: the Mali session
records (`2026-10-27-frameboard` … `2026-11-01-final3`, `<date>-<set>`,
described by `tools/mali/sweep_passsets.py`), the AM62P partial-render
negatives (`2026-10-31-pr-negative`, `2026-10-31-pr-neg175`), and the Pi 4
dEQP-GLES3 slice `2026-10-31-pi4-gles3-a` (`<date>-<board>-<set>-<letter>`,
the plan item's letter) — the newest one and the model for what follows.

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

`<YYYY-MM-DD>-t6-gles2-<session>` (`<session>` is `smoke`, `S1` … `S4`, as the
runner's `--session`; the NanoPC-T6 is not a `tools/board.sh` board, so the name
carries no `sha8`: the commit is in `meta.txt`) is what
`tools/cts/mali-deqp-run.sh --out` writes, redacted, from a run of
`tools/cts/mali-gles2-tree.txt` rows over the board's Armbian
(`tools/cts/README.md` section 6). It holds no README of its own kind of
console: the board's output is the files below, and the record's text is the
commit message and the RESULTS.md row. Its files:

- `meta.txt`: `key=value` lines: `commit` (the `origin/main` commit the
  `deqp-gles2` and the `libGLESv2.so.2` / `libEGL.so.1` were built from; the
  runner refuses a build whose commit is not on `origin/main`), `session`,
  `rows` (`A-B` of the tree table), `tree`, `lib` (the libraries' build stamp or
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
- `kmsg/<label>.txt`: the kernel lines between that chunk's markers, to be
  classified with `tools/cts/mali-kmsg.py` (`docs/notes/mali-panthor-kmsg.md`).

The raw `.qpa`, stdout and stderr and the whole `dmesg` capture stay in the
runner's `RAW_DIR` and are never committed. A slice is checked for the
redaction rules below by the runner itself; read it again before committing.
`tools/cts/mali-gles2-tree.txt` (the four-session tree table) and
`tools/cts/mali-deqp-tally.py` (the tally over slices) are coming; this
section describes the slice, not them.

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
