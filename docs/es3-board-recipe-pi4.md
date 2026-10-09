<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 4: the dEQP-GLES3 tree, build, run and tally

Item `pi4-gles3-tree` of plan #4 (OpenGL ES 3.0). The Pi 4 (V3D 4.2) has never
run dEQP-GLES3; this is the recipe the first session (`stage1-pi4`, then
`board-pi4`) follows, with the exact commands, what each prints when it works,
and how to read what it prints when it does not. Nothing here has run on the
board yet: every "expected" line below is the form the tools print, taken from
their sources and from the Pi 4's GLES2 records (`tools/cts/RESULTS.md`
"Pi 4: sweep 30"), not a measured GLES3 result.

The pieces:

| piece | what |
|---|---|
| `tools/cts/pi4-gles3-tree.txt` | the table: 91 images, every one of the 42,810 cases of `tools/cts/gles3-mustpass.txt` selected by exactly one `CONFIG_DEQP_CASE` line |
| `tools/cts/check-gles3-tree.py` | proves that (`91 images, 42810 matched once, 0 not matched, 0 matched more than once, 0 empty images`) and pins each `# image <LABEL>: <count>`; `check-deqp-images.py` runs it on both GLES3 tables, so `tools/board_selftest.sh` and the gate see a broken table |
| `tools/cts/pi4-deqp-images.sh` with `DEQP_MODULE=gles3` | builds the images (`-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n`), checks each resolved `.config` with `check-deqp-images.py --config --gles3`; `--dry-run` lists the images and their arguments without any environment |
| `driver/samples/zephyr/v3d_deqp` | takes `-DDEQP_MODULE=gles2|gles3` (default gles2, as before): the gles3 package, `tes3TestPackageEntry.cpp`, the gles3 test data; refuses a gles3 build whose configuration still reports ES 2.0 |
| `tools/board.sh pi4 tree` | runs the table, one tryboot per image, resumable; `BOARD_MUSTPASS=<abs path>/tools/cts/gles3-mustpass.txt` makes its tally and diff read the GLES3 list (new in this item; unset, the GLES2 list as before) |
| `tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt` | the tally over the records' consoles |

## 1. Environment

Nothing in a file names a machine, a device or an address. The session sets,
in its shell, from the lab (the values of 2026-10-03 are in
`tools/cts/README.md` section 6; the farm's `.ccfarm/project.toml` passes
`${resource.pi4.user}@${resource.pi4.host}` and `${resource.pi4.device}`):

```sh
export BOARD_HOST=<user>@<pi4>                 # key login, passwordless sudo
export BOARD_UART=/dev/serial/by-id/usb-Raspberry_Pi_Debug_Probe__CMSIS-DAP__<serial>-if01
export BOARD_HDMI_TIMINGS='<the panel's 17 hdmi_timings fields>'
```

and the build inputs (`docs/board-sessions/pi4-deqp-stage1.md` lists where the
farm workers keep them):

```sh
export ZEPHYR_WS=<west workspace, Zephyr af17c0c8>   WEST_VENV=$ZEPHYR_WS/.venv
export ZEPHYR_SDK_INSTALL_DIR=<zephyr-sdk-1.0.1>     ZEPHYR_SDK=$ZEPHYR_SDK_INSTALL_DIR
export RUSTUP_TOOLCHAIN=nightly                      # every V3D sample
export CTS=<VK-GL-CTS checkout at 93bca01 with its externals fetched>
export CMAKE_POLICY_VERSION_MINIMUM=3.5               # a host cmake 4.x configuring CTS's zlib
HEAD8=$(git rev-parse --short=8 HEAD)
```

## 2. Stage 1 for gles3, once per CTS revision

A gles3 stage 1 is its own `WORK` beside the gles2 one (the script's rule:
"give each module its own WORK"). In the farm's dependency cache:

```sh
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")" \
    WORK=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b-gles3/work BOARD=rpi_4b MODULE=gles3 \
    tools/cts/cts-build-zephyr.sh
```

Expected: the last line

```
cts-build-zephyr: libraries in <WORK>/build
```

and `<WORK>/build/modules/gles3/libdeqp-gles3-package.a` present (the file the
image builder checks for). The gles2 stage 1 built it in 101 s on 64 cores and
314 s on 12 loaded ones (`pi4-deqp-stage1.md`); gles3 has about 15 % more
sources. Keep `<WORK>` as `DEQP_WORK` for the farm:
`<ccfarm-env name="DEQP_WORK_GLES3" ...>` in the session's report, since
`DEQP_WORK` is the gles2 stage 1 that the ES 2.0 regression rows still need.

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
export DEQP_WORK=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b-gles3/work
DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt build/deqp/pi4-gles3-$HEAD8
```

Expected: two header lines, then one line per image as it finishes, each also
written to `build/deqp/pi4-gles3-$HEAD8/<LABEL>.built`:

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

What else a line can say: `rc=not-conformance` (the resolved `.config` failed
`--gles3`: a `REPORT_ES2` symbol still `y`, the compiler not the back end, or a
table back end on; read the FAIL line at the end of the log), `rc=exists` (the
directory was there: the builder never reuses one), `rc=1` with a cmake or
ninja error in the log. The one risk specific to gles3 is size: the gles2
image D linked at 59 % of the 256 MiB window (`pi4-deqp-stage1.md`), the gles3
functional library and test data are larger, and `tools/board.sh pi4` refuses
an image past the window (`LOAD_END`), so read the `RAM:` line of the first
image before building the other 90 (`... A1` builds that one alone).

The tree can start as soon as `A1.built` exists; `board.sh` waits for each
label's `.built` line and skips a label whose `rc` is not 0 (`<LABEL>: not
built (rc=...), skipped`).

## 4. The run

```sh
export BOARD_MUSTPASS=$PWD/tools/cts/gles3-mustpass.txt    # absolute: board.sh runs the tally from the record directory's view
TREE_IMAGE_DIR=build/deqp/pi4-gles3-$HEAD8 \
    tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt records/pi4/gles3-$HEAD8
```

Per image, in table order: the run header, the record's summary lines, the
verdict line and the closing rc line, in this form (the Pi 4's GLES2 records
print the same lines with GLES2 names):

```
=== A1 (zephyr.bin, selector -) 2026-10-DD HH:MM:SS
  *** Booting Zephyr OS build af17c0c8aa2a ***
  v3d_deqp: build <describe>
  deqp: done: <p> passed, <f> failed, <n> not supported, <w> warnings, complete=1
  deqp: summary: <b> cases begun, <r> with a result, 0 without
  v3d_deqp DONE...: build <describe>, filter dEQP-GLES3.functional.attribute_location.*,...
  recoveries: 0
board.sh pi4 A1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:deqp_summary resets=0 record=absent back_after=<s>s
=== A1 rc=0 2026-10-DD HH:MM:SS
```

(The verdict line's form was checked on a synthetic A1 record through
`tools/board.sh pi4 tally`; `result=legacy:deqp_summary` is what a deqp
console without a `yk-record:` line yields, and `record=absent` is expected.)

The fields that matter: `rc=0 verdict=done` and `resets=0`. Each image costs
its 240 s lifetime plus the two boots, about 5 min 24 s (sweep 30), so the 91
images are about 8 h 15 min; the tree is resumable (`TREE_ROWS=<a>-<b>`,
rows counted from 0 in table order; a finished run of the same image is
skipped), and a session that cannot stay that long runs it in halves. At the
end `board.sh` prints its tally (section 5) and `TREE-EXIT`.

What a row can say instead, and what to do:

- `verdict=no_end_line`: the image outlived its window. The record on the
  console says `v3d_deqp INCOMPLETE (the photo deadline came first)` and there
  is no `deqp: summary` line; the tally marks the log MISSING. The row's cases
  are not a result. Split the row in `pi4-gles3-tree.txt` (section 6) and
  rerun it (`TREE_ROWS=<its index>`).
- `verdict=done_with_resets` (`resets=<n>` from the OSAL's
  `GPU reset (recovery` lines): a result, never a clean pass; name the row in
  the report.
- `LT` (lifetime.\*, 56 cases) is expected to end with `deqp: fatal:` until
  transform-feedback capture lands (`tf-capture`): its init links a program
  with interleaved transform feedback of `gl_Position` and dEQP ends the run
  when that fails (RESULTS.md 1r). The tally reads it as FATAL and the 56
  cases as without a result; that is the expected first-run outcome, not a
  row to rerun.
- `rc=3` (refused: the board does not answer, another session is on it, the
  UART is held, `/boot/firmware` short of space): nothing changed on the
  board; the chain stops (`TREE_RETRIES` is 0 on the Pi 4). `rc=5`: the board
  did not come back to Linux; the chain stops, a person looks at the board.

## 5. The tally

The authoritative command, over every record's console:

```sh
python3 tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt --table \
    records/pi4/gles3-$HEAD8/*/console.txt
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

`0 outside them` says the records' filters cover the whole list (the table's
property, proven by `check-gles3-tree.py`); `none` is a case whose record has
no line for it (an INCOMPLETE image, the 56 of `LT`). With `BOARD_MUSTPASS`
set, `tools/board.sh pi4 tally records/pi4/gles3-$HEAD8` prints the same table
after the per-run verdict lines, the per-record `deqp: done` / `deqp: summary`
lines and the recoveries per record, and `tools/board.sh pi4 diff <earlier>
<this>` is the per-case diff of two GLES3 trees (the same scope rule).

The per-image verdicts without the table:

```sh
python3 tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt records/pi4/gles3-$HEAD8/*/console.txt
```

prints one line per log (`<path>: deqp: done: ... | deqp: summary: ...`, or
`| MISSING: no summary line`, `| INCOMPLETE: ...`, `| FATAL: ...`), then
`mustpass cases with a result: <r> (of <l> seen)`, the depth-3 group table and
every non-passing case by name; its exit status is 1 when any log is MISSING,
INCOMPLETE or FATAL, which the `LT` image makes it until transform feedback
exists.

For RESULTS.md the numbers to carry are the TOTAL row, the per-group rows and,
per image, the `<n> cases in <ms> ms` line of its record (section 6).

## 6. Sizing, and re-splitting after the first run

The window is `CONFIG_V3D_DEQP_LIFETIME_S` (240 s) less `CONFIG_V3D_DEQP_PHOTO_S`
(30 s): about 200 s of run after a 7 s boot. The table's sizes come from the
Pi 4's GLES2 rates (25-60 ms per case; 2,949 `shaders.operator` cases in
111 s; about 1.2 s per failing random shader) and from what the AM62P's 600 s
window could not hold (its texture line; `texture.filtering.3d` at 19 s per
case under a wildcard, a few seconds named): shader groups up to about 1,200
names an image, `fragment_ops` / `ubo` / `uniform_api` / `state_query` up to
about 1,000, `fbo` / `fragment_out` / `texture` / `buffer` up to about 700,
`builtin_functions.precision` about 200 (each case checks 16k sampled inputs
against a CPU reference), `texture.filtering.3d` as named lists of at most 15
(11 images, F3C1-F3C4, F3F1-F3F5, F3S1-F3S2), `lifetime.*` alone.

Every record prints `<n> cases in <ms> ms, record at <s> s after boot`:

```sh
grep -h 'cases in' records/pi4/gles3-$HEAD8/*/console.txt
```

An image over about 150 s is split before the next sweep (a failing case is
slower than a passing one, so a fix can make an image slower); one that
reached the deadline has no result at all. To split: edit the table (two
`# image` blocks in place of one, the counts in the notes), then

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
| transform feedback (`transform_feedback` 461, `lifetime` 56) | the API exists (`tf-api`), capture does not: the same failures and the FATAL `LT` image | `tf-capture` |
| shadow look-ups and 2D-array sampling refused by the PowerVR code generator | the V3D compiler is a different back end: `v3d-shaderc-gaps` counted 159 refused ES3 programs on the host (`tests/es3_shaderc_v3d.rs`); expect those as `Failed to compile shader program` | `v3d-shaderc-gaps` |
| occlusion queries 62, `rasterizer_discard` 51, `draw_buffers_indexed` 76 n.s. | the same | `oq-v3d`, `tf-capture` (discard), `mrt4` |

What has no AM62P analogue and is V3D's own: the 25 cube cases of the ES 2.0
sweep are judged by the ES 3.0 rule here (seamless filtering is correct under
ES 3.0, so `texture.filtering.cube` and `texture.mipmap.cube` should not show
the GLES2 reference-selection difference); ASTC and ETC2 (`texture.compressed`,
`texture.wrap`, 1,706 cases) depend on what the V3D back end advertises.

## 8. The ES 2.0 regression rows, the same session

ES 2.0 stays the default and plan #1 left it at zero failures; a board session
that runs GLES3 images also runs the GLES2 rows `docs/HANDOFF.md` "Next board
session" names (D, FB1, T1, T3, TF of `pi4-full-tree.txt`), unchanged, with
the gles2 stage 1 as `DEQP_WORK` and no `DEQP_MODULE`:

```sh
DEQP_WORK=<gles2 stage 1> tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-$HEAD8 D FB1 T1 T3 TF
TREE_IMAGE_DIR=build/deqp/pi4-$HEAD8 TREE_ROWS=1,4,19,21,24 tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt records/pi4/es2-$HEAD8
tools/board.sh pi4 tally records/pi4/es2-$HEAD8                    # BOARD_MUSTPASS unset: the GLES2 list
```

Expected: each row's tally equal to sweep 30's (`RESULTS.md` "Pi 4: sweep 30")
with the changes HANDOFF predicts for `ec5ba4f` and `4e9397c`, and nothing
moved from Pass.
