<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 4: the dEQP-GLES3 tree, build, run and tally

Item `pi4-gles3-tree` of plan #4 (OpenGL ES 3.0). The recipe the Pi 4 (V3D
4.2) board sessions follow, with the exact commands, what each prints when it
works, and how to read what it prints when it does not. Sections 3-6 read as
the board printed them, from three GLES3 session records in the tree —
`tools/cts/log/2026-10-31-pi4-gles3-a/` (12 images, rows 9-20, `0ff58329`),
`tools/cts/log/2026-11-pi4-gles3-b/` (16, rows 21-36, `8990a5cc`) and
`tools/cts/log/2026-11-pi4-gles3-c/` (26, rows 37-52 and 81-90, `009f62a5`)
— 54 of the 91 images on three commits. The first nine (`A1`-`FO1`) had run
at `925dbca9` before them (RESULTS.md "Pi 4, first nine images"); rows 53-80,
the `texture` T images, have run on no commit. What is still written as
"expected" is the line form the tools print at tip, taken from their sources;
no board session has run the whole 91-image tree from one commit, and nothing
here quotes the result of one.

The pieces:

| piece | what |
|---|---|
| `tools/cts/pi4-gles3-tree.txt` | the table: 91 images, every one of the 42,810 cases of `tools/cts/gles3-mustpass.txt` selected by exactly one `CONFIG_DEQP_CASE` line |
| `tools/cts/check-gles3-tree.py` | proves that (`91 images, 42810 matched once, 0 not matched, 0 matched more than once, 0 empty images`) and pins each `# image <LABEL>: <count>`; `check-deqp-images.py` (repo rule 6) and `cts-selftest.sh` check 19 run it on the three GLES3 tables (AM62P, Pi 4, i.MX8MP), so `tools/board_selftest.sh` and the gate see a broken table |
| `tools/cts/pi4-deqp-images.sh` with `DEQP_MODULE=gles3` | builds the images (`-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n`), checks each resolved `.config` with `check-deqp-images.py --config --gles3`; `--dry-run` lists the images and their arguments without any environment. `tools/cts/images-selftest.sh` section 7 (run by `cts-selftest.sh` check 20, in the gate) pins both paths against a fake cmake/ninja: the gles2 dry run and build exactly as before (no module argument, the ES 2.0 check), the gles3 path's three arguments and `--gles3`, and the stage 1 check |
| `driver/samples/zephyr/deqp` | takes `-DDEQP_MODULE=gles2|gles3` (default gles2, as before): the gles3 package, `tes3TestPackageEntry.cpp`, the gles3 test data; refuses a gles3 build whose configuration still reports ES 2.0 |
| `tools/board.sh pi4 tree` | runs the table, one tryboot per image, resumable; `BOARD_MUSTPASS=<abs path>/tools/cts/gles3-mustpass.txt` makes its tally and diff read the GLES3 list (new in this item; unset, the GLES2 list as before) |
| `tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt` | the tally over the records' consoles |

## 1. Environment

Nothing in a file names a machine, a device or an address. The session sets,
in its shell, from the lab (the variables are listed in
`tools/cts/README.md` section 5; ccfarm's settings for the project pass
`${resource.pi4.user}@${resource.pi4.host}` and `${resource.pi4.device}`):

```sh
export BOARD_HOST=<user>@<pi4>                 # key login, passwordless sudo
export BOARD_UART=/dev/serial/by-id/usb-Raspberry_Pi_Debug_Probe__CMSIS-DAP__<serial>-if01
export BOARD_HDMI_TIMINGS='<the panel's 17 hdmi_timings fields>'
```

and the build inputs (each farm worker keeps them in its own dependency
cache; `tools/cts/deps-env.sh` prints the layout):

```sh
export ZEPHYR_WS=<west workspace, Zephyr af17c0c8>   WEST_VENV=$ZEPHYR_WS/.venv
export ZEPHYR_SDK_INSTALL_DIR=<zephyr-sdk-1.0.1>
export RUSTUP_TOOLCHAIN=nightly                      # every V3D sample
export CTS=<VK-GL-CTS checkout at 93bca01 with its externals fetched>
export CMAKE_POLICY_VERSION_MINIMUM=3.5               # a host cmake 4.x configuring CTS's zlib
HEAD8=$(git rev-parse --short=8 HEAD)
```

## 2. Stage 1 for gles3, once per CTS revision

A gles3 stage 1 is its own `WORK` beside the gles2 one (the script's rule:
"give each module its own WORK"). In the dependency layout
(`tools/cts/deps-env.sh --layout`, `$DEPS` below), beside the gles2 stage 1
`$DEPS/deqp-stage1-rpi_4b` that `tools/cts/deps-env.sh --build --board rpi_4b
DEQP_WORK` makes (it also fetches `$DEPS/VK-GL-CTS` at 93bca01), with the
environment of that entry's build in `tools/cts/deps.toml` (the west venv
activated, `ZEPHYR_TOOLCHAIN_VARIANT=zephyr`, `CFLAGS=CXXFLAGS=-O2`):

```sh
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")" CTS=$DEPS/VK-GL-CTS \
    WORK=$DEPS/deqp-stage1-rpi_4b-gles3 BOARD=rpi_4b MODULE=gles3 \
    tools/cts/cts-build-zephyr.sh
```

Expected: the last line

```
cts-build-zephyr: libraries in <WORK>/build
```

and `<WORK>/build/modules/gles3/libdeqp-gles3-package.a` present (the file the
image builder checks for). The gles2 stage 1 built it in 101 s on 64 cores and
314 s on 12 loaded ones (2026-10-08, see "Notes from the gles2 stage 1"); gles3 has about 15 % more
sources. Measured on 2026-10-11 (`stage1-pi4`,
`ab43a20c5:tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md`): the gles3 stage 1 built
in 203 s on 12 cores, and image A1 linked at 161,256 KB (61.51 % of the window).
Measured again on 2026-10-31 at `ff7b9882` (`stage1-pi4`, a farm worker
with 4 cores, no board): the gles2 stage 1 and the CTS fetch through
`deps-env.sh --build`, then the gles3 stage 1 in 223 s (453 ninja steps,
`WORK` 2.0 GB, `libdeqp-gles3-package.a` present), then image A1 alone
(section 3, `... A1`) in 92 s: `A1 rc=0 build=ff7b9882`, `RAM: 161836 KB
256 MB 61.74%`, `zephyr.bin` 16,533,472 bytes, the `--gles3` conformance
line, and both `REPORT_ES2` symbols unset in the resolved `.config`. Keep `<WORK>` as `DEQP_WORK` for the farm:
`<ccfarm-env name="DEQP_WORK_GLES3" ...>` in the session's report, since
`DEQP_WORK` is the gles2 stage 1 that the ES 2.0 regression rows still need.

### Notes from the gles2 stage 1 (2026-10-08, two farm workers)

Measured when the gles2 stage 1 for `rpi_4b` was built and its row D image
was linked as a proof (no board was used):

- Stage 1: rc 0 in 100.7 s on 64 cores and 313.8 s on 12 cores at load 15
  to 30; 417 or 418 ninja steps. `WORK` is 2.0 GB (`cts` 1.7 GB, `build`
  87 MB, `flags` 167 MB). The inputs it uses take 4.2 GB (VK-GL-CTS),
  3.5 GB (the west workspace) and 1.5 GB (the SDK), shared with the other
  boards.
- Row D (`# image D: 292`): `D rc=0`, the image links at RAM 155,444 KB of
  256 MB (59.30 %). Cold build 59.8 s, warm (ccache) 16.1 s. A second build
  gave the same sha256: the image build is reproducible.
- `capi/yakogl_capi.cmake` needs a `cargo` with `rustc` beside it. A cargo
  shim (kache) without one fails; put `~/.cargo/bin` first on `PATH`.
- A host cmake 4.x rejects CTS's zlib (`cmake_minimum_required(VERSION
  2.4.4)`): set `CMAKE_POLICY_VERSION_MINIMUM=3.5` (section 1), or use the
  venv's cmake 3.31.
- A Zephyr tree imported from a tarball (no git history) must still be a git
  repository with `refs/heads/manifest-rev` at HEAD, or `west list` and
  `zephyr_module.py` fail the manifest import from `west.yml`.
- The image's `build=` stamp comes from the checkout the script runs from
  (`git -C "$REPO" describe`), so run it from the checkout whose stamp the
  image should carry.
- The stage 1 CTS copy is a `--shared` clone of the CTS checkout, and its
  externals are symlinks into the checkout's `external/*/src`. Keep the
  checkout while `WORK/cts` is in use.

## 3. The images

Dry run first, no environment needed:

```sh
DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh --dry-run tools/cts/pi4-gles3-tree.txt x | head -3
```

prints one line per image, label, filter and the arguments beyond the common ones:

```
A1 dEQP-GLES3.functional.attribute_location.*,...,dEQP-GLES3.functional.prerequisite.* [-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n]
B1 dEQP-GLES3.functional.buffer.* [-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n]
C1 dEQP-GLES3.functional.clipping.* [-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n]
```

(without `DEQP_MODULE` the same command on `pi4-full-tree.txt` prints
`AL dEQP-GLES2.functional.attribute_location.*` and so on, T5 with
`[-DEXTRA_CONF_FILE=texture-2048.conf]`: the gles2 path adds nothing.)

Then the build, into a directory that does not exist yet:

```sh
export DEQP_WORK=$DEPS/deqp-stage1-rpi_4b-gles3
DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt build/deqp/pi4-gles3-$HEAD8
```

Expected: two header lines, then one line per image as it finishes, each also
written to `build/deqp/pi4-gles3-$HEAD8/<LABEL>.built` (measured at
`009f62a5`: a `build from 009f62a5 (<40-hex>)` header, then 26 lines of the
form `S17 rc=0 sha256=<64 hex> build=009f62a5`):

```
pi4-deqp-images: build from <git describe> (<40-hex commit>)
pi4-deqp-images: DEQP_MODULE=gles3 (-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n)
A1 rc=0 sha256=<64 hex> build=<git describe>
B1 rc=0 sha256=<64 hex> build=<git describe>
...
VA2 rc=0 sha256=<64 hex> build=<git describe>
```

91 `rc=0` lines. Each `<LABEL>.log` ends with the memory report and the
conformance verdict:

```
             RAM:     <n> KB       256 MB     <p>%
check-deqp-images: the dEQP image recipes build what conformance needs
```

`<git describe>` is `git describe --always --dirty` run from the checkout the
script builds from ("Notes from the gles2 stage 1"); with no tag in reach
that is the 8-hex commit, and every image of a session carries it: the
`009f62a5` images printed `build=009f62a5`, and every console of that record
prints `v3d_deqp: build 009f62a5` and no other build. A `<hash>-dirty`,
`unknown` or `none` stamp is refused by `tools/cts/pi4-gles3-preflight.sh
--commit <sha>` ("Only a clean hash that is a prefix of (or extends) --commit
counts").

What else a line can say: `rc=not-conformance` (the resolved `.config` failed
`--gles3`: a `REPORT_ES2` symbol still `y`, the compiler not the back end, or a
table back end on; read the FAIL line at the end of the log), `rc=exists` (the
directory was there: the builder never reuses one), `rc=1` with a cmake or
ninja error in the log. Size is measured, not risked: every GLES3 image of
the three records linked at 161,836 KB (`0ff58329`) or 161,844-161,848 KB
(`8990a5cc`, `009f62a5`) of the 256 MiB window — 61.74 % — and all 54 passed
`tools/board.sh pi4`'s window check (`LOAD_END`) on the way to the board.
That is 2.4 points over the gles2 image D (59 %, "Notes from the gles2 stage
1"); the margin is what a larger build would cut, so still read the `RAM:`
line of the first image before building the other 90 (`... A1` builds that
one alone).

The tree can start as soon as `A1.built` exists; `board.sh` waits for each
label's `.built` line and skips a label whose `rc` is not 0 (`<LABEL>: not
built (<rc>), skipped: see <img-dir>/<LABEL>.log`), and skips a label that
already has a finished run of the same image in the record directory
(`<LABEL>: finished run <record dir>, skipped`).

## 4. The run

```sh
export BOARD_MUSTPASS=$PWD/tools/cts/gles3-mustpass.txt    # absolute: board.sh runs the tally from the record directory's view
TREE_IMAGE_DIR=build/deqp/pi4-gles3-$HEAD8 \
    tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt records/pi4/gles3-$HEAD8
```

Per image, in table order: the run header, the console, the verdict line and
the closing rc line. The console excerpt is S17 of the `009f62a5` record
(`tools/cts/log/2026-11-pi4-gles3-c/S17.console.txt`) with `board.sh`'s run
header and verdict lines around it; every GLES3 record console has this
shape. Between `the record at 210 s` and `deqp: done` sit dEQP's progress
lines and one `v3d_deqp: case #N ...` line per case; between the record
header and the end line sit the image's own record (keep region, carve-out,
jobs, BASELINE and FIRST FAILURE lines where applicable) and the per-group
totals. The excerpt is as the record file holds these lines; `board.sh`'s
own echo keeps a subset — `Booting Zephyr`, `: build`, `deqp: done` /
`deqp: summary`, `DONE|INCOMPLETE|STOPPED` and `recoveries:` matches —
indented two spaces:

```
=== S17 (zephyr.bin, selector -) 2026-11-DD HH:MM:SS
*** Booting Zephyr OS build af17c0c8aa2a ***
v3d_deqp: build 009f62a5
board: PM watchdog resets the SoC ~250 s after boot (tryboot -> Linux)
v3d_deqp: the record at 210 s after boot at the latest
deqp: done: 720 passed, 0 failed, 0 not supported, 0 warnings, complete=1
deqp: summary: 720 cases begun, 720 with a result, 0 without
v3d_deqp DONE, all passed: build 009f62a5, filter 'dEQP-GLES3.functional.shaders.operator.angle_and_trigonometry.*,dEQP-GLES3.functional.shaders.operator.bi...
  720 cases in 34270 ms, record at 34 s after boot, reset at ~250 s
  totals: P 720  (R: not passed, a draw was refused in it; passed with one refused: 0)
slow jobs: 0 (past 1000 ms, ended; the slowest 0 ms); MMU faults: 0; recoveries: 0; hung: none
v3d_deqp: build 009f62a5 DONE, all passed: 720 cases, P 720 F 0 R 0 N 0 other 0
board.sh pi4 S17 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=<s>s
=== S17 rc=0 2026-11-DD HH:MM:SS
```

(The record line's ending is one of `DONE, all passed` / `DONE, none failed`
/ `DONE, dEQP threw` / `DONE` / `STOPPED EARLY by dEQP (complete=0; later
cases never began)` / `INCOMPLETE (the photo deadline came first)` /
`FATAL (reason <n>, pc <hex> lr <hex> far <hex>)` (v3d_board.c), and the
filter is printed truncated with `...`. The verdict line's form was checked
on a synthetic record through `tools/board.sh pi4 tally` at tip; the three
records ran `board.sh`'s steps by hand (see "On the farm"), so they hold no
verdict line. The end line the verdict rule matches (`END_RE`) is the last
of `deqp: summary: `, `v3d_deqp: build <sha8> (DONE|INCOMPLETE|STOPPED)...`
or `v3d_gl_test RECORD end`, and it accepts all three deqp endings — DONE,
INCOMPLETE and an aborted STOPPED all read `boot_verdict=complete
verdict=done`. `result=legacy:v3d_deqp_0` is what a deqp console without a
`yk-record:` line yields: `legacy_result` takes the last line matching that
pattern — the image's end line — and names it `v3d_deqp_<its last word>`;
the end line's last word is its `other <o>` count (`R1` at `0ff58329` ends
`... other 5` and would read `v3d_deqp_5`), not a recovery count —
recoveries are the separate `resets=` field. An aborted or INCOMPLETE run
yields the same `result=legacy:v3d_deqp_0` as a clean one. `record=absent`
is expected.)

The fields that matter: `rc=0 verdict=done` and `resets=0` — and
`verdict=done` is completion, not a pass: an image with failures reads
`done`, and so does a run dEQP aborted (the abort bullet below). Times
measured over the three records: a run's cases finish in 0.2-55.7 s (longest
`DONE` run: S12's 55.1 s at `8990a5cc`, then S24's 54.9 s at `009f62a5`);
the PM watchdog ends the Zephyr side at ~250 s; Linux answers again
296-302 s after the reboot (`0ff58329`), 297-304 s (`8990a5cc`), 301-345 s
(`009f62a5`) — inside `board.sh`'s bounds (capture `RUN_BOUND` 420 s, then
`RETURN_BOUND` 300 s of ssh margin; rc 5 only past both). A row costs about
5.3-5.5 min end to end (12 rows in 66 min, 16 in 83, 26 in 137), so the 91
images are about 8 h; the tree is resumable (`TREE_ROWS=<a>-<b>`, rows
counted from 0 in table order; a finished run of the same image is skipped),
and a session that cannot stay that long runs it in ranges. A session names
its rows as a closed range, `TREE_ROWS=0-15`, then `16-30`, ... (or a list
such as `1,4,19`): an open range such as `3-` is refused with rc 2
(`tools/cts/images-selftest.sh` checks it). Every image of the range must
already be built (section 3), and `TREE_IMAGE_DIR` must be set for the tree
step, or `board.sh` runs the row from no image directory. At the end
`board.sh` prints its tally (section 5) and `TREE-EXIT`.

What a row can say instead, and what to do:

- `verdict=no_end_line`: the console holds no end line at all — a lost
  capture, or a crash before the image's last line (a Zephyr panic prints
  `ZEPHYR FATAL ERROR` and reads `verdict=fatal`; a dEQP `die()` prints
  `deqp: fatal:`, which the tally reads `FATAL: dEQP ended the run (die());
  the cases after it did not run`). An image that outlived its window is
  *not* this verdict: its end line
  `v3d_deqp: build <sha8> INCOMPLETE (the photo deadline came first): ...`
  is there, `END_RE` accepts it, and the row reads
  `rc=0 verdict=done boot_verdict=complete` while its tally marks the log
  MISSING (`no summary line`) — or reads `INCOMPLETE: <n> cases have a
  result line here, the summary says <m> had a result` — and
  `check-pi4-gles3-log.py` fails it (no `complete=1`). No run has outlived
  the window so far. In both cases the row's cases are not a result: split
  the row in `pi4-gles3-tree.txt` (section 6, *for time*) and rerun it
  (`TREE_ROWS=<its index>`).
- `verdict=done_with_resets` (`resets=<n>` from the OSAL's
  `GPU reset (recovery` lines): a result, never a clean pass; name the row in
  the report. 0 recoveries in all 54 board runs so far.
- **A run dEQP aborted also reads `rc=0 verdict=done`.** A refused draw ends
  the whole run long inside the window. The back end prints
  `yakogl/v3d: draw refused: ...`, the GL core reports `GL_OUT_OF_MEMORY`
  (`GL error 0x505 in glDrawArrays`; `yakogl: glDrawArrays was not recorded
  (-22)`), dEQP records a ResourceError and stops: `Test run was ABORTED!`,
  `complete=0`, and the cases behind the refusing one never began. The
  console still ends with an end line — `v3d_deqp: build <sha8> STOPPED
  EARLY by dEQP (complete=0; later cases never began): <n> cases, P <p>
  F <f> R 1 N 0 other <o>` — so `board.sh` reads `verdict=done`, and
  `deqp-uart-tally.py` reads the log whole with exit 0 (its line is
  `deqp: done: ... complete=0 | deqp: summary: ...`; no
  `MISSING`/`INCOMPLETE`/`FATAL`): neither tool can see the abort. Detect
  it from the console (`Test run was ABORTED!`, the refusal line,
  `STOPPED EARLY`, `complete=0`), from the tally (`complete=0` in its per-log
  line; `mustpass cases with a result: <r>` short of the image's mustpass
  count; the named `ResourceError <case>` line; the `--table` view's `none`
  column), and over the record with `tools/cts/check-pi4-gles3-log.py`,
  which exits 1 reporting `aborted, no complete deqp summary (<n>):
  <labels>` for every tally without `complete=1` (the record contract,
  "On the farm").
  Aborted so far, with their 0-based rows of the 91-image table: FO6 (13)
  and M1 (17) at `0ff58329`, S03-S08 (23-28) at `8990a5cc`, S32 (52),
  TF1 (81) and TF2 (82) at `009f62a5`. Not everything big is an abort:
  S24 (44), S26 (46) and VA2 (90) ran `DONE, all passed` at `009f62a5`,
  and LT (16) ran `DONE` at `0ff58329` (next bullet). Section 6 splits
  an aborted row *at the refusing case*.
- `LT` (lifetime.\*, 56 cases): ran at `0ff58329`, 58 cases in 1.5 s,
  `DONE: P 56 F 2` — mustpass view 55 pass / 1 Fail
  (`lifetime.attach.deleted_output.buffer_transform_feedback`; the second
  `F` is a case outside the list). The old expected outcome — its init
  linking interleaved transform feedback of `gl_Position`, dEQP ending the
  run at a `deqp: fatal:` (RESULTS.md 1r) — did not happen: the capture is
  on main since `c4dce9125` (the GL core runs the vertex shader on the CPU,
  the same on the three GPUs; the host passes `lifetime` 56 / 56). A FATAL
  here is a finding.
- `rc=3` (refused: the board does not answer, another session is on it, the
  UART is held, `/boot/firmware` short of space): nothing changed on the
  board; the chain stops (`TREE_RETRIES` is 0 on the Pi 4). `rc=5`: the board
  did not come back to Linux; the chain stops, a person looks at the board.

### On the farm: the Pi 4 resource

`<pi4 resource>` is the farm's Pi 4 (resource kind `pi4`). A ccfarm session
asks for it with `requirements.resources: ["resource:<pi4 resource>"]` (or
`"target:pi4"` plus `"capability:flash"` and `"capability:serial-console"`),
and ccfarm fills `${resource.pi4.user}`, `${resource.pi4.host}` and
`${resource.pi4.device}` the way the `deqp-tree-pi4` stage of ccfarm's
settings for the project does. From the repository root, with stage 1 as `DEQP_WORK_GLES3`
(`ab43a20c5:tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md`) and the images built by
section 3 into `build/deqp/pi4-gles3-$HEAD8`, the exact lines are:

```sh
HEAD8=$(git rev-parse --short=8 HEAD)
export BOARD_HOST=${resource.pi4.user}@${resource.pi4.host} BOARD_UART=${resource.pi4.device}
export BOARD_MUSTPASS=$PWD/tools/cts/gles3-mustpass.txt
# the first image alone: proves the size, the boot and the ES 3.0 context before 8 h of board time
TREE_IMAGE_DIR=build/deqp/pi4-gles3-$HEAD8 TREE_ROWS=0 \
    tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt records/pi4/gles3-$HEAD8
# the rest (resumable: a finished row of the same image is skipped)
TREE_IMAGE_DIR=build/deqp/pi4-gles3-$HEAD8 \
    tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt records/pi4/gles3-$HEAD8
tools/board.sh pi4 tally records/pi4/gles3-$HEAD8
```

and, unset `BOARD_MUSTPASS`, the ES 2.0 rows of section 8 with the gles2
stage 1 (`DEQP_WORK`):

```sh
unset BOARD_MUSTPASS
DEQP_WORK=$DEQP_WORK tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-$HEAD8 D FB1 T1 T3 TF
TREE_IMAGE_DIR=build/deqp/pi4-$HEAD8 TREE_ROWS=1,4,19,21,24 \
    tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt records/pi4/es2-$HEAD8
tools/board.sh pi4 tally records/pi4/es2-$HEAD8
```

The verdict lines to expect, one per image (A1's shown; every other label
the same form):

```
board.sh pi4 A1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=<s>s
=== A1 rc=0 <date> <time>
```

for a row whose image crashed on the board or died inside dEQP, `verdict=`
other than `done` with a `ZEPHYR FATAL ERROR` or `deqp: fatal:` line in its
record (not what `LT` did: it ran `DONE` at `0ff58329`, the bullet above),
and after the last row of a whole-tree run the tally's

```
== mustpass: 42810 cases in these records' case filters (0 outside them, not run, left out); <r> with a result (<l> case lines seen)
```

(a partial session prints its own numbers — section 5). `TREE_ROWS=0` is A1
(rows count from 0 in table order). The commands above were checked against
`tools/board.sh` and `pi4-deqp-images.sh` at `eb911ac`, again at `c3f4820`
and again at `e4f9c78f` (dry runs, `check-gles3-tree.py` on the table,
`tools/board_selftest.sh`, `cts-selftest.sh`); the console, tally and time
lines quoted in sections 3-6 are from board runs since.

All three records ran `board.sh`'s steps **by hand**, in `board.sh`'s order:
the Pi 4's UART bridge takes one client at a time and the leased serial
console (an MCP server connected forwards) held it, so `board.sh`'s own
capture was not available. Their "Method" sections list the steps — the
`--dry-run` image checks, the ssh prechecks, staging to `/boot/firmware`
and `tryboot.txt` with both read back before the reboot, `sudo reboot
"0 tryboot"`, the console read up to the end line, the wait for Linux,
`pi4_undo` read-back — and record that every cycle read back as before,
the board left as found (`config.txt` untouched throughout). A session that
can take the UART bridge instead runs the `board.sh` chain above.

The record a GLES3 session leaves is checked by

```sh
python3 tools/cts/check-pi4-gles3-log.py --commit <full 40-hex sha> \
    --tree tools/cts/pi4-gles3-tree.txt <session-dir> ...
```

which exits 1 for: a tree label with no `<LABEL>.tally.txt` in any directory
or tallies of one label in two directories; a `README.md` with no line
starting `commit: <sha>` (or `commit <sha>`) naming that commit; a tally
without `complete=1` and its full `deqp: summary: N cases begun, N with a
result, 0 without` line — the abort check, printed as `aborted, no complete
deqp summary (<n>): <labels>`; and any file holding a home path, an IPv4
address, a `.ts.net` name or a `/dev/tty` path. Session directories are
named `tools/cts/log/<YYYY-MM>-pi4-gles3-<sha8>-rowsAA-BB/` (the checker's
default glob). Per image a record holds `<LABEL>.console.txt` (that boot's
console, the firmware's first line to the end line, CRs and NULs dropped,
no time stamps) and `<LABEL>.tally.txt` (section 5's per-log command, run
with relative paths), plus `table.txt` (the `--table` view over all
consoles) and `cycles.txt` (per image: the image sha256, the Linux boot id
before the reboot, the reboot time, the seconds until Linux was back, the
clean-up read-back, the `cases in` line and the end line). The three
records above predate this contract: plain `-a`, `-b`, `-c` directory names,
and a `commit:` line only in `-c`'s README.

## 5. The tally

The authoritative command, over every record's console — `board.sh`'s own
scratch records nest each run in a directory of its own (the `*/` glob
fits), while a session record directory is flat (`*.console.txt`):

```sh
(cd records/pi4/gles3-$HEAD8 && python3 ../../../tools/cts/deqp-uart-tally.py \
    --mustpass ../../../tools/cts/gles3-mustpass.txt --table */console.txt)
```

Expected head and tail (the group rows between them, one per
`dEQP-GLES3.functional.<group>` plus `info`):

```
== mustpass: 42810 cases in these records' case filters (0 outside them, not run, left out); <r> with a result (<l> case lines seen)
group                           pass   fail   n.s.     QW  other   none
attribute_location               ...
...
vertex_arrays                    ...
TOTAL                            <p>    <f>    <n>    <q>    <o>   <none>
== not Pass, by name
<Status> dEQP-GLES3.functional....
```

`0 outside them` only when the consoles really are the whole tree: a partial
record prints its own numbers — the 26 consoles of `009f62a5` head their
`table.txt` with `17578 cases in these records' case filters (25232 outside
them, not run, left out)`, the 12 of `0ff58329` with `5749 ... (37061
outside them)` — the rows it did not run, not a defect; the whole-tree view
of a multi-session campaign comes from merging the session directories,
`python3 tools/cts/gles3-notpass.py --pi4 <session-dir> ...` (it reads each
record's commit line). `none` is a mustpass case whose records hold no
result line for it: an INCOMPLETE image, or the cases after an abort that
never began — 37 at FO6 and 62 at M1 (`0ff58329`), 1,142 across S03-S08
(`8990a5cc`), 1,002 as S32's 137, TF1's 439 and TF2's 426 (`009f62a5`). The
`none` column exists in the `--table` view. With `BOARD_MUSTPASS`
set, `tools/board.sh pi4 tally records/pi4/gles3-$HEAD8` prints the same table
after the per-run verdict lines, the per-record `deqp: done` / `deqp: summary`
lines and the recoveries per record, and `tools/board.sh pi4 diff <earlier>
<this>` is the per-case diff of two GLES3 trees (the same scope rule).

The per-image verdicts without the table:

```sh
(cd records/pi4/gles3-$HEAD8 && python3 ../../../tools/cts/deqp-uart-tally.py \
    --mustpass ../../../tools/cts/gles3-mustpass.txt */console.txt)
```

Run from inside the session directory, with relative console paths: the tally
prints each path as given, so an absolute one would put the worker's home
path into the tally files (`tools/release/redaction-scan.py` and the record
checker flag it). A tally labels each log after the directory the tally runs
in — the `009f62a5` tallies, run from a scratch `out/` directory, print
`out: deqp: summary ...` — so a `<LABEL>.tally.txt` is written with the name
given:

```sh
python3 tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt \
    --label S32 S32.console.txt          # --label takes exactly one log
```

prints one line per log (`<path>: deqp: done: ... | deqp: summary: ...`, an
aborted log reading `complete=0` there and being otherwise tallied whole, or
`| MISSING: no summary line`, `| INCOMPLETE: <n> cases have a result line
here, the summary says <m> had a result`, `| FATAL: dEQP ended the run
(die()); the cases after it did not run`), then `mustpass cases with a
result: <r> (of <l> seen)`, the group table (`group pass fail n/s other`;
with `--table`, `pass fail n.s. QW other none`), every non-passing case by
name, and the per-image "the image's own counters" line. Its exit status is
1 only for a MISSING, INCOMPLETE or FATAL log — all 54 consoles of the three
records tally with exit 0, the eight aborted ones included; the abort is
caught by `check-pi4-gles3-log.py` (section 4), not by the tally.

For RESULTS.md the numbers to carry are the TOTAL row, the per-group rows and,
per image, its `cases in` line as `cycles.txt` keeps it (section 6).

## 6. Sizing, and re-splitting after the first run

The window is `CONFIG_V3D_DEQP_LIFETIME_S` (240 s): its watchdog is fed to
that point and resets the board at most 10 s later, so every console prints
`board: PM watchdog resets the SoC ~250 s after boot (tryboot -> Linux)`
and every run ends `reset at ~250 s`. The record is written before it,
`CONFIG_V3D_DEQP_PHOTO_S` (30 s) ahead of the lifetime: every console
prints `v3d_deqp: the record at 210 s after boot at the latest`, and a run
still going at 210 s ends with its record marked
`INCOMPLETE (the photo deadline came first)`. The table's sizes come from the
Pi 4's GLES2 rates (25-60 ms per case; 2,949 `shaders.operator` cases in
111 s; about 1.2 s per failing random shader) and from what the AM62P's 600 s
window could not hold (its texture line; `texture.filtering.3d` at 19 s per
case under a wildcard, a few seconds named): shader groups up to about 1,200
names an image, `fragment_ops` / `ubo` / `uniform_api` / `state_query` up to
about 1,000, `fbo` / `fragment_out` / `texture` / `buffer` up to about 700,
`builtin_functions.precision` about 200 (each case checks 16k sampled inputs
against a CPU reference), `texture.filtering.3d` as named lists of at most 15
(11 images, F3C1-F3C4, F3F1-F3F5, F3S1-F3S2), `lifetime.*` alone.

Every record prints its run line (v3d_board.c), and `cycles.txt` keeps it
per image:

```
  720 cases in 34270 ms, record at 34 s after boot, reset at ~250 s
```

```sh
grep -h 'cases in' records/pi4/gles3-$HEAD8/*/console.txt                  # board.sh scratch dirs
grep -h 'cases in' tools/cts/log/2026-11-pi4-gles3-c/*.console.txt        # a session record
```

Measured over all 54 board runs: 0.2-55.7 s per image (`record at 1 s` ...
`record at 55 s`) — nowhere near the 210 s record line; no run has reached
the deadline yet, and `2026-11-pi4-gles3-c/README.md` writes its 54.9 s run
as "well inside the 200 s window, so no image needs splitting for time".

Splitting answers two different faults, and they split two different ways.

- **An overrun (*for time*).** An image over about 150 s is split before
  the next sweep (a failing case is slower than a passing one, so a fix can
  make an image slower); one that reached the 210 s record line has its
  record marked INCOMPLETE and its unfinished cases have no result at all
  (section 4). Split by case count, the way the table was sized. Not
  observed yet.
- **An abort (*at the refusing case*).** No deadline involved, and
  `board.sh` read `verdict=done` (section 4): split the image **at the
  case where dEQP stopped**, so the cases behind it get results. At
  `0ff58329`: FO6 at #834 `fragment_ops.scissor.clear_int_buffer` (31
  later cases, 37 mustpass without a result) and M1 at #448
  `pbo.renderbuffer.rg16i_clears` (58 later, 62 without) — the integer
  clear refusal, RC1 of `docs/es3-cts-failures.md`. At `009f62a5`: S32 at
  #589 `shaders.texture_functions.texturesize.sampler2dshadow_fragment`
  (the 32-bit-channel texture sampling refusal, RC2; 137 without) and TF1
  at #4 / TF2 at #11 (`the job builder refused the draw (a uniform,
  program or primitive it does not write)`, interleaved `ivec4` / `mat4`
  transform feedback of line primitives; 439 and 426 without — the 15
  transform-feedback cases that ran before the refusals all passed).
  `2026-11-pi4-gles3-c/README.md`: "Until each is fixed, the image needs
  splitting at that case to get results for the cases behind it." An
  abort at *every* image's first case does not split — the six
  `precision` images at `8990a5cc` all stopped at #0 on one refusal
  ("splitting the images does not help, every image fails at its first
  case"): there the fix is the back end taking the target, not the table.
  A session record cannot keep an aborted tally (the record checker,
  section 4): re-split and rerun the aborted rows, or report the refusal
  cause.

To split: edit the table (two `# image` blocks in place of one, the counts in
the notes), then

```sh
python3 tools/cts/check-gles3-tree.py tools/cts/pi4-gles3-tree.txt
```

must end with `check-gles3-tree: tools/cts/pi4-gles3-tree.txt: <N> images,
42810 matched once, 0 not matched, 0 matched more than once, 0 empty images`
(N the new image count; the header comment's "91" follows it), and
`tools/cts/cts-selftest.sh` check 19 and `check-deqp-images.py` say the same.

## 7. Reading the first run against the AM62P's

The AM62P's GLES3 run (RESULTS.md 1r, `396d667a`) had 8,835 failures in 78
message buckets; the GL-core ones apply to the Pi 4 as they are, the back-end
ones have V3D counterparts in the plan:

| AM62P bucket | V3D expectation | plan item |
|---|---|---|
| integer colour renderbuffers refused (`shaders.builtin_functions` 1,483, `fragment_out` 531) | the same until the V3D integer render targets land | `int-rt-v3d` |
| sized texture formats refused at upload / sampled wrong (`texture.specification`, `texture.format`, `texture_functions` isampler) | GL core accepts them since `sized-formats-gl`; sampling needs the V3D format map | `formats-v3d` |
| transform feedback (`transform_feedback` 461, `lifetime` 56) | the API (`tf-api`) and the CPU capture (`c4dce9125`) are on main; the host passes `transform_feedback` 882 / 882 and `lifetime` 56 / 56; on the Pi 4 `LT` ran `DONE` (55 pass / 1 mustpass Fail, `0ff58329`) and TF1/TF2 aborted on the job-builder refusal (`009f62a5`, section 6) | `tf-capture` (built) |
| shadow look-ups and 2D-array sampling refused by the PowerVR code generator | the V3D compiler is a different back end: on the host it builds 3,453 of the 3,461 dEQP ES 3.00 programs that must build (`docs/es3/shaderc-coverage.md`; the 159 refusals `v3d-shaderc-gaps` once counted came from a test that is not on main); expect the rest as `Failed to compile shader program` | `v3d-shaderc-gaps` |
| occlusion queries 62, `rasterizer_discard` 51, `draw_buffers_indexed` 76 n.s. | occlusion queries are counted on the V3D since `7526b084` (merged in `452d6c31`; `OCCLUSION_QUERY_COUNTER`): board-proven on the Pi 4 at `67b27e26` (gl_test case 196 `occl` PASS, `tools/cts/log/2026-11-pi4-gltest/`; the earlier `cd738b1a` board run read all results 0, `9b1ce2ff` being the occlusion-counters fix); discard is in `tf-api`; `GL_MAX_DRAW_BUFFERS` is 3 | `oq-v3d` (built), `tf-api`, `mrt4` |

What has no AM62P analogue and is V3D's own: the 25 cube cases of the ES 2.0
sweep are judged by the ES 3.0 rule here (seamless filtering is correct under
ES 3.0, so `texture.filtering.cube` and `texture.mipmap.cube` should not show
the GLES2 reference-selection difference); ASTC and ETC2 (`texture.compressed`,
`texture.wrap`, 1,706 cases) depend on what the V3D back end advertises.

## 8. The ES 2.0 regression rows, the same session

ES 2.0 stays the default, and the Pi 4 reads 0 failures there (17,126 / 0 /
34, `tools/cts/RESULTS.md`, "Pi 4: the whole dEQP-GLES2 tree, 0 fail"). A board
session that runs GLES3 images also runs a few GLES2 rows of
`pi4-full-tree.txt` as a regression check (D, FB1, T1, T3, TF here), unchanged,
with the gles2 stage 1 as `DEQP_WORK` and no `DEQP_MODULE`:

```sh
DEQP_WORK=<gles2 stage 1> tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-$HEAD8 D FB1 T1 T3 TF
TREE_IMAGE_DIR=build/deqp/pi4-$HEAD8 TREE_ROWS=1,4,19,21,24 tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt records/pi4/es2-$HEAD8
tools/board.sh pi4 tally records/pi4/es2-$HEAD8                    # BOARD_MUSTPASS unset: the GLES2 list
```

Expected: each row's tally equal to the whole tree on `a27d69c0`
(`records/pi4/deqp-a27d69c0/full`), with T3 and TF as on `1cf946a9`
(`records/pi4/deqp-1cf946a9/rows`: all pass), and nothing moved from Pass.
The rows of the commands are verified against `pi4-full-tree.txt` at tip,
0-based: D 1, FB1 4, T1 19, T3 21, TF 24.
