# dEQP-GLES2 against YakoGL on the boards

The OpenGL ES 2.0 conformance suite, run on the boards against YakoGL, so
progress can be measured instead of guessed. On every board it runs in a
Zephyr image (`driver/samples/zephyr/deqp`, one `CONFIG_DEQP_CASE` filter per
image) booted from the bootloader by `tools/board.sh`; the record comes back
on the UART (AM62P, Pi 4) or in the SNVS (i.MX8MP).

```
tools/cts/
  cts-build-zephyr.sh          stage 1: dEQP's sources and data for the Zephyr images
  am62p-deqp-images.sh         stage 2, AM62P: one image per line of a filter file
  am62p-full-tree.txt          the whole GLES2 tree for the AM62P, 17 images
  am62p-ms4-tree.txt           the AM62P's 18th image: multisample.* on rgba8888d24s8ms4
  check-deqp-images.py         do the image recipes still build what conformance needs (2d)
  imx8mp-deqp-build.sh         the i.MX8MP image with the 29-entry table
  imx8mp-full-tree.txt         the whole GLES2 tree for the i.MX8MP, 29 entries
  pi4-deqp-images.sh           stage 2, Pi 4
  pi4-full-tree.txt            the whole GLES2 tree for the Pi 4, 29 images
  deqp-uart-tally.py           a captured console -> per-group tally and names
  qpa-from-uart.py             a captured console (DEQP_ECHO_QPA) -> .qpa
  cts-images.py                .qpa -> the result, reference and mask PNGs
  cts_resume.py                which cases still have no result, and why
  cts-summary.py               .qpa -> per-group Pass/Fail/NotSupported/Crash
  gles2-mustpass.txt           the 17165 cases a conformance run must pass
  host-deqp-run.sh             dEQP-GLES2 on THIS host: compile/link outcomes only
  cts-build.sh                 the yakogl dEQP target (2b needs it); see section 1
  RESULTS.md                   the measured results
```

## 1. Build

The board images: stage 1 is `tools/cts/cts-build-zephyr.sh` (dEQP's sources
and data for Zephyr, `WORK=`), stage 2 the per-board image script of section 2.

`cts-build.sh` below cross-builds the Linux aarch64 `deqp-gles2` and
`deqp-egl` that the retired Linux pair ran; nothing runs those binaries now.
`host-deqp-run.sh` (2b) still needs the yakogl target it applies to the CTS
checkout (step 2), so it stays until that step has its own home (Work list).

```sh
tools/cts/cts-build.sh
```

Needs a VK-GL-CTS checkout (default `~/com/github/KhronosGroup/VK-GL-CTS`, set
`CTS=` for another). The script

1. unpacks the Bootlin `aarch64--glibc--stable-2024.05-1` SDK under
   `build/cts/toolchain/` and relocates it — **`~/pvr-lab/aarch64--glibc--stable-2024.05-1`
   cannot be used**: that copy is pruned and has no `Scrt1.o`/`crti.o`, so it
   compiles (all the pvrlab `.ko` needs) but cannot link an executable. Put a
   downloaded tarball at `build/cts/toolchain/aarch64--glibc--stable-2024.05-1.tar.xz`
   (or `BOOTLIN_TARBALL=`) to build without network;
2. applies `cts-yakogl-target.patch` to the CTS checkout if the target is not
   already there, and leaves it **uncommitted** — the checkout is the user's;
3. builds into `build/cts/aarch64` (gitignored). About 780 steps, two minutes on
   64 cores.

The toolchain's glibc 2.39 / libstdc++ 6.0.32 match Torizon OS 7.6.1 exactly, so
nothing has to be shipped alongside the binaries.

### What the target is

`DEQP_TARGET=yakogl` builds `framework/platform/yakogl/tcuYakoGLPlatform.cpp`,
which registers a native display whose `getLegacyNative()` is `EGL_DEFAULT_DISPLAY`
— the only display YakoGL's `eglGetDisplay()` accepts — and **no window and no
pixmap factory**. `eglu::GLContextFactory` turns that into a `NotSupportedError`
for those surface types and falls through to `eglCreatePbufferSurface()`. So the
platform never has to produce a `struct gles_native_window`, which a test process
could not allocate.

Nothing GL or EGL is linked: `libGLESv2.so.2` and `libEGL.so.1` are `dlopen`ed by
SONAME at run time, which is why `BOARD_LIB_DIR` is all the runner needs.

## 2. Run on a board

One image runs one filter; a whole tree runs one image per line of the
board's table (or, on the i.MX8MP, one image with the table and a selector).
`BOARD_HOST=user@host` comes from the environment only; the variables are in
`tools/board.sh`'s header.

- **AM62P** (U-Boot TFTP):

  ```sh
  ZEPHYR_WS=... DEQP_WORK=<stage 1> tools/cts/am62p-deqp-images.sh tools/cts/am62p-full-tree.txt <img-dir> <tag> > <img-dir>/build.out
  TREE_IMAGE_DIR=<img-dir> TREE_TAG=<tag> tools/board.sh am62p tree tools/cts/am62p-full-tree.txt <outdir>
  tools/board.sh am62p diff <earlier outdir> <outdir>
  ```

  The tree can start on image 1 while the later ones build (it waits for each
  `img<N> rc=` line in `build.out`). `<tag>` names the image directories
  (`img<N>-<tag>`) and must be the same in both commands: the builder's
  default is the checkout's short hash, board.sh's is empty.
  The multisample.* group needs dEQP's 4x multisampled config, a second image
  (`tools/cts/am62p-ms4-tree.txt`, section 2d); the 17 images of the full tree
  run on the single-sampled config, where those 16 cases are NotSupported.
- **i.MX8M Plus**: section 2c.
- **Raspberry Pi 4**: section 6.

### The baseline: five groups, one command per board

`tools/cts/baseline-tree.txt` holds the five groups that say whether a board
is worth measuring further, one image each: `info` (6 mustpass cases),
`prerequisite` (3), `negative_api` (246), `state_query` (396) and
`shaders.functions` (256), labelled BI, BP, BN, BQ and BF. The same table
runs on all three boards:

```sh
tools/cts/cts-baseline.sh <am62p|pi4|imx8mp> build <img-dir>        # the board's own builder
tools/cts/cts-baseline.sh <am62p|pi4|imx8mp> run <img-dir> <outdir> [<earlier outdir>]
```

`build` calls `am62p-deqp-images.sh` (which names a labelled row's directory
by its label), `pi4-deqp-images.sh`, or `imx8mp-deqp-build.sh --table` (one
image, the rows by selector), each with its usual environment. `run` is
`tools/board.sh <board> tree` on the table: from the bootloader, resumable, a
tally per group from the records (passed, failed, not supported), and with an
earlier baseline's outdir the per-case diff. `--dry-run` before the board
prints every command and does nothing. The baseline is the full-tree method
with a five-row table, not a second runner; `tools/board_selftest.sh` checks
the three boards' dry runs.

### The common set: five images, one command per board

Before anything else, every board runs the common set
(`tools/cts/common-tree.txt`, driver/docs/one-method.md): glmark2 (GM), slint
with the window copied to the screen (SO), slint with the window a view into
the scan-out buffer (SV), the AGL cluster sample animating its cluster UI at
the board's panel size — 1024x600 on the AM62P, 1280x800 on the i.MX8MP,
1920x720 on the Pi 4 (CL) — and gl_test with the sample's default case list
(GT), one image each, from the bootloader:

```sh
tools/cts/cts-common.sh <am62p|pi4|imx8mp> build <img-dir>
tools/cts/cts-common.sh <am62p|pi4|imx8mp> run <img-dir> <outdir> [<earlier outdir>]
tools/cts/cts-common.sh <am62p|pi4|imx8mp> tally <outdir> [<earlier outdir>]
```

`build` is `tools/cts/common-images.sh` (its header has the environment:
`ZEPHYR_WS`, `GLMARK2_SRC`, `SLINT_NOSTD_DIR`, `AGL_SLINT_CLUSTER_DIR` and the
optional `NOTO_SANS_DIR` for the cluster row, and the AM62P firmware header;
the slint and cluster rows build their Rust with `-Z build-std`, so they need
a nightly toolchain); it gives every image its board's watchdog self-arm and
reports an image without the arm line as `rc=no_marker`. The AM62P's board
layer has no scan-out view, so its SV row is the copy path again (the cluster
sample's own `prj.conf` turns the view on where the board layer has one). `run` is
`tools/board.sh <board> tree` on the table, then `tools/cts/common-tally.py`:
per row the record words, board.sh's verdict recomputed from the files, the
glmark2 score with its failed and unsupported scenes, the slint and cluster
probes (the cluster row also reports its throughput — frames, fps, Mpx/s and
mean draw, from the console's FINGERPRINT, PERF and APP records, computed by
`yakogl-tools record --log`, which rounds where the cluster's own console line
truncates; the i.MX8MP
has no UART and its SNVS keeps only the verdict record, so there the tally
says `throughput absent`), gl_test's passed of run with the failing case
numbers (on the i.MX8MP only the
first: its CASE record carries no more), the GPU resets with their source
(`resets (record) N` when the record carries a count, `resets (console count) N`
from board.sh's count of the console's recovery lines, else `resets absent`) and the image's sha256
and build ids; with an earlier common run, the row-by-row diff. `--dry-run`
prints every command and does nothing. `common-tally.py --self-test` runs in
`tools/check.sh` (it replays the cluster fixture in `tools/cts/testdata`);
`tools/board_selftest.sh` checks the three boards' dry runs.

The board images compile shaders in the image (the online compiler) or look
them up in the offline table.

### Seeing the pixels behind a failure

`Got faulty pixels` is not something to reason about from a case name. Build
one image for a small slice with `CONFIG_DEQP_ECHO_QPA=y`, run it, and rebuild
the log from the console:

```sh
tools/cts/qpa-from-uart.py <run dir>/console.txt slice.qpa
tools/cts/cts-images.py --out images slice.qpa
```

That writes `<case>/Images.TestImage.png`, `Images.ReferenceImage.png` and
`Images.DifferenceMask.png` per failing case. The images take seconds each at
the console's baud rate: a slice, never a suite.

## 2b. On the host, without a board

```sh
tools/cts/host-deqp-run.sh                   # the whole mustpass list
tools/cts/host-deqp-run.sh shaders.struct    # cases whose name contains this
GPU=powervr tools/cts/host-deqp-run.sh       # the PowerVR's online back end
```

Builds deqp-gles2 for the host (`build/host-deqp/deqp`, once) and the library
with the linux OSAL, the GC7000 back end and the online compiler
(`build/host-deqp/lib`; with `GPU=powervr` the PowerVR's back end, in
`build/host-deqp/lib-powervr`, and the run in `run-<commit>-powervr`), then
runs the list in fresh processes of 100 cases.
The linux OSAL keeps the command stream instead of executing it, so **every
read-back is black and no render verdict from this run is ever quoted**. What
is real is each case's compile and link outcome with our info log;
`cts-refusals.py` buckets the refusals, separating gaps (cases dEQP failed)
from correct refusals (negative cases it passed). The script prints how many
cases began against how many were asked for, and each chunk's exit status: a
134 is a panic that cost that chunk. Label every number from it HOST-ONLY.

## 2c. The whole tree on the Verdin i.MX8M Plus, one command

`tools/cts/imx8mp-deqp-build.sh <name> 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt` builds
one dEQP image carrying the 29-entry table; `TREE_IMAGE=<that zephyr.bin> tools/board.sh imx8mp tree
tools/cts/imx8mp-full-tree.txt <outdir>` runs every entry as one boot with the selector in SNVS word 1
(`BOARD_BOOT=uboot-tftp`, the default, is a row; `BOARD_BOOT=kexec-dev` is a development row), decodes each
SNVS record, reads the failing names from the DEQP_FAILS pages, and writes `results.tsv` (one line per entry:
index, filter, begun, passed, failed, notsup, other, noresult, names) and `rows.md`; `TREE_PREV=<an earlier
outdir>` adds the per-entry diff. `BOARD_HOST=user@host` comes from the environment only; the variables are
in `tools/board.sh`'s header. The 2026-10-03 run at 29460f96 is `imx8mp-full-tree-29460f96.results.tsv`.

## 2d. The whole GLES2 tree from a detached checkout of main, per board

What a conformance image is (checked on the host by
`tools/cts/check-deqp-images.py`, which `tools/board_selftest.sh` runs, and on
each built image's `.config` by the three builders): the in-process compiler
(`CONFIG_YAKOGL_SHADERC_ONLINE=y`, the choice's default), no precompiled tier
(`YAKOGL_PRECOMPILED_SHADERS`, `YAKOGL_SHADERC_PRECOMPILED_ONLY` off; the V3D
cannot take them anyway), the context reporting ES 2.0
(`CONFIG_YAKOGL_REPORT_ES2=y`, and `CONFIG_DEQP_REPORT_ES2=y`, which selects
it), no test knob set (the sample selects `YAKOGL_TEST_KNOBS` and calls none),
and the memory of the last passing sweeps: Pi 4 48 MiB carve-out with 16 MiB
of tiled copies (image T5: `texture-2048.conf`, 64 / 24 MiB), i.MX8MP 160 MiB
C heap, AM62P 256 MiB GPU carve-out (`boards/*.overlay`). A sweep image runs
on dEQP's default single-sampled config; only the AM62P's MS4 image names
`rgba8888d24s8ms4`. A builder whose image fails the check reports
`rc=not-conformance` and the tree skips it. A dEQP-GLES3 image
(`DEQP_MODULE=gles3`) is checked with `--gles3`: both report switches must be
off (`-DCONFIG_DEQP_REPORT_ES2=n` only drops the select; the AM62P builder also
passes `-DCONFIG_YAKOGL_REPORT_ES2=n`), because dEQP's GLES3 context queries
`GL_NUM_EXTENSIONS` at start and an ES 2.0 report ends the run there.

Common to all three (the scripts build the checkout they live in, so run them
from the detached one):

```sh
git worktree add --detach <sweep> main        # or: git clone ... && git checkout --detach main
SW=<sweep>; HEAD8=$(git -C $SW rev-parse --short=8 HEAD)
export ZEPHYR_WS=<west workspace> WEST_VENV=$ZEPHYR_WS/.venv ZEPHYR_SDK_INSTALL_DIR=<zephyr sdk 1.0.1>
export BOARD_HOST=<user@host>                 # tools/board.sh's header has the board variables
$SW/tools/board.sh --dry-run <board> tree <table> <outdir>   # prints every command, touches nothing
```

Stage 1 (dEQP's libraries, about 1.9 GB, once per CTS revision and board
family; no duration is recorded in this repository) is
`ZEPHYR_BASE=$ZEPHYR_WS/zephyr ZEPHYR_MODULES="$($SW/tools/cts/zephyr-modules.sh $ZEPHYR_WS)" WORK=<stage 1> BOARD=<zephyr board> CTS=<VK-GL-CTS> $SW/tools/cts/cts-build-zephyr.sh`.

**AM62P** (`BOARD=verdin_am62p/am62p54/a53`; `BOARD_BOOT=uboot-tftp`, or
`uboot-mmc` for the eMMC boot; `BOARD_UART=host:port` of the bridge). The
checkout needs `driver/src/fw/pvr_fw_blob.h` (`driver/tools/mkfwblob.sh
<rgx.fw>`, never committed).

```sh
export DEQP_WORK=<stage 1>
$SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-full-tree.txt <img> $HEAD8 > <img>/build.out    # 17 images
$SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-ms4-tree.txt <img-ms4> ms4 > <img-ms4>/build.out # 1 image
TREE_IMAGE_DIR=<img> TREE_TAG=$HEAD8 $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-full-tree.txt <rec>/full
TREE_IMAGE_DIR=<img-ms4> TREE_TAG=ms4 $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-ms4-tree.txt <rec>/ms4
$SW/tools/board.sh am62p tally <rec>/full      # per image, the mustpass table, every case not Pass
$SW/tools/board.sh am62p tally <rec>/ms4
```

Time: each image runs until dEQP is done or the 600 s watchdog feeder stops
(`CONFIG_YAKOGL_AM62P_WDT_SECONDS`), plus the boot: at most about 11 min per
image, 18 images, so under 3.3 h on the board with the builds overlapping the
run; no full-tree wall time is recorded. Last sweep: RESULTS.md section 1z2
(d60b9d76): 17,141 pass / 0 fail / 19 NotSupported / 5 warnings of 17,165, the
multisample cases from the MS4 image. The tally of a new sweep is that table:
17 image rows plus `ms4`, its `fail` column all 0, and
`tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles2-mustpass.txt <console logs of both trees>`
for the combined mustpass count (it keeps the last status of a case, so the
MS4 log replaces image 17's NotSupported multisample rows when named last).

**i.MX8M Plus** (`BOARD=verdin_imx8mp/mimx8ml8/a53`; `BOARD_BOOT=uboot-tftp`
or `uboot-mmc`; no UART: the record is in the SNVS). One image carries all 29
rows, picked by selector:

```sh
export ZEPHYR_WORKSPACE=$ZEPHYR_WS ZEPHYR_VENV=$ZEPHYR_WS/.venv IMX8MP_OUT=<out> DEQP_STAGE1=<stage 1>
$SW/tools/cts/imx8mp-deqp-stage1.sh                                   # stage 1, log $IMX8MP_OUT/deqp-stage1.log
$SW/tools/cts/imx8mp-deqp-build.sh wt-$HEAD8 'dEQP-GLES2.info.*' --table $SW/tools/cts/imx8mp-full-tree.txt
TREE_IMAGE=$IMX8MP_OUT/wt-$HEAD8/zephyr/zephyr.bin $SW/tools/board.sh imx8mp tree $SW/tools/cts/imx8mp-full-tree.txt <rec>/full
$SW/tools/board.sh imx8mp tally <rec>/full                            # results.tsv, rows.md
```

The build prints `rc=0` and, for the image's `.config`, the
`check-deqp-images.py` verdict. Time: 29 selector boots, each at most the
600 s WDOG1 bound (`CONFIG_GC_BOARD_WDOG_SECONDS`) plus the boot, so at most
about 5 h; not recorded as a whole. The tally is `results.tsv` (one line per
row: index, filter, begun, passed, failed, notsup, other, noresult, names); the
committed reference is `imx8mp-full-tree-29460f96.results.tsv`
(2026-10-03), and RESULTS.md's i.MX8MP section at d8583f20 has the per-entry
table. Compare with `diff <(cut -f1-8 <that file>) <(cut -f1-8 <rec>/full/results.tsv)`
or `tools/board.sh imx8mp diff <earlier tree dir> <rec>/full` when the
earlier tree's directory is kept. Known intermittent rows: see
`docs/HANDOFF.md` (#25, #9, #8).

**Raspberry Pi 4** (`BOARD=rpi_4b` for stage 1; backend tryboot, so no
`BOARD_BOOT`; `BOARD_UART` the debug probe's by-id device,
`BOARD_HDMI_TIMINGS` as the lab's; the environment values are in section 6).
Needs `RUSTUP_TOOLCHAIN=nightly` as for any V3D sample, and the 256 MiB
`dram0` overlay the sample already carries.

```sh
export DEQP_WORK=<stage 1 for rpi_4b>
$SW/tools/cts/pi4-deqp-images.sh $SW/tools/cts/pi4-full-tree.txt <img>      # 29 images, <img>/<LABEL>.built
TREE_IMAGE_DIR=<img> $SW/tools/board.sh pi4 tree $SW/tools/cts/pi4-full-tree.txt <rec>/full
$SW/tools/board.sh pi4 tally <rec>/full
$SW/tools/board.sh pi4 diff <earlier tree> <rec>/full
```

Time: measured, an image costs its 240 s lifetime plus the boots, about
5.4 min (images D and E: 5 min 24 s and 5 min 21 s), so 29 images are about
2 h 40 min on the board. Last sweep: RESULTS.md "Pi 4: sweep 30" (fe5e8e73,
2026-10-01): 17,165 of 17,165 with a result, 17,075 pass / 25 fail / 62
NotSupported / 3 QualityWarning; the 25 are the cube cases of #12. Any image
whose tally says anything but those 25 is a regression.

For all three, an image with no `deqp: summary` line, or fewer case lines than
its summary counts, is a missing result, never a pass
(`deqp-uart-tally.py`'s MISSING, INCOMPLETE and FATAL); rerun that image
(`TREE_ROWS=<n>`; a finished run of the same image is skipped).

## 3. Reading the numbers

| outcome | what it means |
|---|---|
| `Pass` / `Fail` | what the test decided. |
| `NotSupported` | the test asked for something the implementation does not advertise and skipped itself. **Allowed** in an AOSP mustpass run — it is not a failure, but a large count means a capability is not being advertised. |
| `QualityWarning`, `CompatibilityWarning` | allowed too: right answer, outside a preferred precision or behaviour. |
| `Crash`, `Timeout`, `InternalError`, `ResourceError` | a driver bug. Each one also costs a process restart, so a run with many of them is much slower than its case count suggests. |
| `noexec` | no result in any log. With the restart loop this should only be cases the run never reached — if it is anything else, read the `no progress` line above it. |

Conformance means every one of the 17165 mustpass cases is `Pass` or
`NotSupported`. The `src/*_NOTES.md` files say what is implemented and what is
stubbed. Treat a group's number as
a measurement, not a verdict on the group.

## 4. Case counts in the mustpass

17165 cases. The ten largest groups:

| group | cases |
|---|---|
| `functional.shaders` | 10101 |
| `functional.fragment_ops` | 1923 |
| `functional.uniform_api` | 1267 |
| `functional.texture` | 807 |
| `functional.fbo` | 709 |
| `functional.clipping` | 602 |
| `functional.vertex_arrays` | 571 |
| `functional.state_query` | 396 |
| `functional.negative_api` | 246 |
| `functional.draw` | 101 |

`gles2-mustpass.txt` is a copy of
`external/openglcts/data/gl_cts/data/mustpass/gles/aosp_mustpass/main/gles2-main.txt`.
It is tracked here so a baseline is reproducible against a fixed list.
(`android/cts/main/gles2-main-2026-03-01.txt` is a 0-byte placeholder in this
checkout — not the list to use.) The binary knows 19825 cases in all; the
`accuracy`, `performance` and `stress` groups are outside the mustpass.

## 6. Raspberry Pi 4 (V3D 4.2, Zephyr only)

The Pi 4 boots Raspberry Pi OS by default. Each Zephyr image runs once through
the firmware's tryboot and ends with its PM watchdog reset, which brings the
board back to Linux (`driver/docs/rpi4-v3d-backend-plan.md`, "On the board").
The record is read from the Pi's UART (GPIO 14/15), never from the HDMI panel.

- `tools/board.sh pi4 run <zephyr.bin> <logdir> [name]` (backend tryboot):
  one image, once. It refuses (exit 3, nothing changed on the board) when the
  board does not answer, when `who` there shows a session from another
  machine, when a process on this host holds the UART, or when
  `/boot/firmware` has less than twice the image free; it reads the staged
  image and `tryboot.txt` back before the reboot, captures the UART into the
  run directory's `console.txt` until the record's end line, waits for Linux
  (a new boot_id), removes the image and puts the old `tryboot.txt` back.
- `tools/cts/pi4-deqp-images.sh <table> <img-dir>` builds the 29 images of
  `pi4-full-tree.txt` (one `<img-dir>/<LABEL>/` each, never reused);
  `TREE_IMAGE_DIR=<img-dir> tools/board.sh pi4 tree pi4-full-tree.txt <outdir>`
  runs them in file order and tallies: per group with every non-Pass case by
  name, the recoveries per record; `tools/board.sh pi4 diff <a> <b>` is the
  per-case diff of two trees.

Nothing in the scripts names a machine. The values of the lab they were
written in (2026-10-03), for the environment:

```sh
export BOARD_HOST=signal-slot@<pi4>            # key login, passwordless sudo (PI4_HOST is read too)
# the Raspberry Pi Debug Probe; its ttyACM number changes when USB re-enumerates
# (ttyACM0 on 2026-09-25, ttyACM2 on 2026-10-03), the by-id name does not
export BOARD_UART=/dev/serial/by-id/usb-Raspberry_Pi_Debug_Probe__CMSIS-DAP__<serial>-if01
# the 1920x720 bar panel (17 fields, as the board's own tryboot.txt had them)
export BOARD_HDMI_TIMINGS='1920 1 50 44 50 720 1 10 20 50 0 0 0 60 0 99070000 1'
# build
export ZEPHYR_WS=$HOME/com/github/signal-slot/zephyr-ws-yakogl   # Zephyr af17c0c8
export WEST_VENV=$ZEPHYR_WS/.venv ZEPHYR_SDK_INSTALL_DIR=$HOME/org/zephyrproject/zephyr-sdk-1.0.1
export RUSTUP_TOOLCHAIN=nightly
export DEQP_WORK=<stage 1: cts-build-zephyr.sh with BOARD=rpi_4b>
tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/img
TREE_IMAGE_DIR=build/img tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt records/pi4/<tree>
tools/board.sh pi4 diff records/pi4/<earlier tree> records/pi4/<tree>
```

A dEQP image costs its lifetime (`CONFIG_V3D_DEQP_LIFETIME_S`, 240 s) plus
the boots, whatever its case count: images D and E took 5 min 24 s and
5 min 21 s from the start of the tryboot run to its end (2026-10-03).

An image without a screen (the framebuffer mailbox call failing) prints no
v3d_deqp board record: no build line, no DONE line, no OSAL diag line. Its
last line is then the deqp sample's `deqp: summary:`, which is why
`tools/board.sh pi4` takes that as an end line too, and recoveries are counted from
the OSAL's `GPU reset (recovery N)` lines, which are printed either way.
From 2026-10-01 until the commit that disabled Zephyr's own mailbox driver in
`driver/samples/zephyr/common/rpi_4b_sram.dtsi`, every image built on Zephyr
af17c0c8 ran that way (the mailbox replies were taken by Zephyr's ISR;
`driver/docs/rpi4-v3d-backend-plan.md`, "Zephyr").
