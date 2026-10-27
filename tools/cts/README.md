<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP against YakoGL on the boards

The OpenGL ES 2.0 and 3.0 conformance suites, run on the boards against
YakoGL, so progress can be measured instead of guessed. On every board it runs
in a Zephyr image (`driver/samples/zephyr/deqp`, one `CONFIG_DEQP_CASE` filter
per image) booted from the bootloader by `tools/board.sh`; the record comes
back on the UART (AM62P, Pi 4) or in the SNVS (i.MX8MP). The current numbers
per board are in [STATUS.md](STATUS.md) (Japanese); the current results are in
[RESULTS.md](RESULTS.md).

```
tools/cts/
  README.md, STATUS.md, RESULTS.md   this file, the current state, the current results
  testdata/                    fixtures of the parsers' self-tests (testdata/README.md)
  wedge_probe/                 the closed GC7000 shader-hang investigation (wedge_probe/README.md)

  deps.toml                    every external dependency: name, path, marker, build, downloads, packages
  deps-env.sh, deps_env.py     the dependency layout as environment lines (ZEPHYR_WS, PCO_SHC, ...); --check, --build
  deps-selftest.sh             host checks of deps.toml and deps-env.sh (run by cts-selftest.sh)
  zephyr-modules.sh            the Zephyr module list of a west workspace, for the builds below
  cts-build-zephyr.sh          stage 1: dEQP's sources and libraries for the Zephyr images
  cts-yakogl-zephyr.patch      applied by cts-build-zephyr.sh to its own clone of the CTS
  cts-yakogl-target.patch      the yakogl dEQP target, applied by cts-build.sh (section 1)
  cts-build.sh                 applies that target; also cross-builds the retired Linux binaries
  toolchain-aarch64-bootlin.cmake  cts-build.sh's toolchain file

  am62p-deqp-images.sh         stage 2, AM62P: one image per line of a filter file
  am62p-full-tree.txt          the whole GLES2 tree for the AM62P, 17 images
  am62p-ms4-tree.txt           the AM62P's 18th image: multisample.* on rgba8888d24s8ms4
  am62p-gles3-tree.txt         the dEQP-GLES3 tree for the AM62P, 46 images (section 2e)
  gen-am62p-gles3-tree.py      writes am62p-gles3-tree.txt; --check is a gate row
  imx8mp-env.sh                sourced by the i.MX8MP build scripts: checks their environment
  imx8mp-deqp-stage1.sh        stage 1 for the i.MX8MP (a wrapper of cts-build-zephyr.sh)
  imx8mp-deqp-build.sh         the i.MX8MP image with the 29-entry table
  imx8mp-full-tree.txt         the whole GLES2 tree for the i.MX8MP, 29 entries
  imx8mp-full-tree-29460f96.results.tsv  the committed reference tally of that tree (2026-10-03)
  imx8mp-gltest-build.sh       one gl_test image in the sweep configuration
  pi4-deqp-images.sh           stage 2, Pi 4
  pi4-full-tree.txt            the whole GLES2 tree for the Pi 4, 29 images
  pi4-gles3-tree.txt           the whole GLES3 tree for the Pi 4, 91 images
  check-deqp-images.py         do the image recipes still build what conformance needs (2d)
  check-gles3-tree.py          does a tree table select every mustpass case exactly once

  baseline-tree.txt, cts-baseline.sh     the five-group baseline (section 2)
  common-tree.txt, cts-common.sh         the common set (section 2)
  common-images.sh, common-tally.py      its images and its tally

  gles2-mustpass.txt           the 17165 cases a GLES2 conformance run must pass
  gles3-mustpass.txt           the 42,810 cases of the GLES3 mustpass
  gen-gles3-mustpass.sh        writes gles3-mustpass.txt from a CTS checkout
  deqp-uart-tally.py           a captured console -> per-group tally and names, Lost in capture
  qpa-from-uart.py             a captured console (DEQP_ECHO_QPA) -> .qpa
  cts-images.py                .qpa -> the result, reference and mask PNGs
  cts_resume.py                which cases still have no result, and why
  cts-summary.py               .qpa -> per-group Pass/Fail/NotSupported/Crash
  cts-refusals.py              the compiler's refusals in a host run, bucketed by reason (2b)
  cts-selftest.sh              host self-tests of the parsers above
  fault-record.py              decodes the A53 fault record a Zephyr sample leaves in DDR
  check-es100-skips.py         the GL_TEST_ES100 skip list, checked against the code
  readme-glmark2-check.py      the README glmark2 table: one main commit, a RESULTS.md row per board
  glmark2-ms.py                a board's glmark2 console log -> per-scene ms a frame table
  host-deqp-run.sh             dEQP on THIS host: compile/link outcomes only (2b)
```

`readme-glmark2-check.py [--readme F] [--results F] [--ref R] [--scenes LIST]` reads the
glmark2 table of `README.md` and fails (exit 1, one `FAIL:` line each) when a
board (AM62P, Raspberry Pi 4, i.MX8MP) is missing, the boards name more than
one commit, the commit is not an ancestor of `--ref` (default `origin/main`;
`none` skips it, and an unavailable git or ref only warns), or the glmark2 table
of `RESULTS.md` has no row for a board citing that commit (short hashes match
by prefix). Exit 2 is a parse error (a file or table not found). It fails on the
live README until the boards are re-measured at one commit. `--scenes
terrain,shadow,refract` (off by default; the default output and exit codes are
unchanged) also checks each board's `RESULTS.md` row at that commit: every
named scene needs a number as `<scene> <n> ms` (markdown bold ignored; both the
`glmark2-ms.py --cells` form `terrain 178.86 ms (6 FPS)` and the `RESULTS.md`
form `terrain 250 ms (4 FPS)` match), and the row must cite a record directory
`records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/` (`<board>` is `pi4`, `imx8mp`
or `am62p`; `<sha8>` matches the common commit by prefix). A missing or
non-numeric figure and a missing record directory are one `FAIL:` line each
naming the board. The gate does
not run it on the live files: `cts-selftest.sh` check 27 runs it on
`testdata/readme-glmark2/`. Run it by hand before handing in a glmark2 table.

`glmark2-ms.py [--cells] <console log>` reads a glmark2-es2 console log of any
of the three board images and prints a Markdown table: one row per scene with
the scene string, upstream's `FrameTime` in ms and the FPS, then the final
score. `--cells` prints only `terrain 178.86 ms (6 FPS), shadow ..., refract
...`, the ms-a-frame cells for a `RESULTS.md` glmark2 row (a board reports ms a
frame, not fps). Exit 0: parsed; 1: a scene line damaged, a scene that started
and printed no result, a ran count that disagrees with the lines, no score, or
(with `--cells`) one of the three scenes without a time; 2: no glmark2 output.
`cts-selftest.sh` check 28 runs it on `testdata/glmark2-ms/` (a pass log in
each board's format with its expected table and cells, and one log per exit).

## 0. The dependencies outside the repository

`deps.toml` is the one list of everything the gate and the board tools need
that is not in the repository (the west workspace, the Zephyr SDK, `pco_shc`,
the panfrost tree, glmark2, the firmware header, the boards' dEQP trees, ...).
Each `[[dep]]` has its environment variable (`name`, `aliases`), its `path` in
the layout directory (`tools/cts/deps-env.sh --layout`: connected's
`$CONNECTED_DEPS_DIR` in a farm turn, else `~/.local/share/connected/deps/yakogl`), the `marker` file only a usable tree
has, the `board` it belongs to, a `build` command that makes it from nothing,
the `inputs` that build downloads (url and sha256) and the distribution
`packages` it needs per family. The key list is in the header of `deps.toml`.

- `tools/cts/deps-env.sh --check [board]` is what every host and image is
  judged by: one ok / missing line per entry, exit 1 when one is missing.
- `tools/cts/deps-env.sh [board]` prints the `NAME=value` lines.
- `tools/cts/deps-env.sh --packages apt` prints the packages of all entries;
  `tools/setup-gate-host.sh` installs them with its own base list. An image
  recipe reads the same list at the commit it builds.
- `tools/cts/deps-env.sh --build NAME...` (or `--build-missing [board]`) makes
  entries on a host or an image, on purpose: it fetches the inputs, runs the
  `build` with `$DEPS` set to the layout directory, and checks the marker. It
  never touches an entry that is present. The gate never builds them. To try
  it without touching the host, point `CONNECTED_DEPS_DIR` at an empty directory.
- `tools/env/build-gate-image.sh --pvr-fw-blob <path>` makes a fresh VM into
  the gate's image from these tools alone: `setup-gate-host.sh --install`,
  the firmware header copied into the layout (the one hand step, it is never
  committed), `deps-env.sh --build-missing`, then `deps-env.sh --check`.
  `tools/env/gate-image.sh` runs it on a fresh orion machine through the
  manifest `yakogl-gate.image.yaml` and the recipe `yakogl-gate.recipe.yaml`
  (which also installs the GitHub runner), as the next generation of the
  pool's image. `tools/env/yakogl-gate.yaml` is the pool itself, the
  definition the lab's orion server holds (`orion pool apply`).
- A change that adds a gate row needing an external tree adds its `[[dep]]` in
  the same commit. Without it a host or an image cannot learn from the
  repository what to install and build (PR #1078 needed the `MESA_PAN_TREE`
  entry fixed by hand on every runner). A tree that cannot be rebuilt (the
  firmware header) has no `build` and a `note` saying why.
- `deps-selftest.sh` checks all of this against a fake `deps.toml`.

## 1. Build

The board images: stage 1 is `tools/cts/cts-build-zephyr.sh` (dEQP's sources
and libraries for Zephyr, `WORK=`), stage 2 the per-board image script of
section 2.

`cts-build.sh` has one job that is still used: it applies
`cts-yakogl-target.patch` to the CTS checkout (step 2 below), and
`host-deqp-run.sh` (2b) refuses to run without that target. Its other job,
the cross-build of the Linux aarch64 `deqp-gles2` and `deqp-egl` that the
retired Linux pair ran, has no user now.

```sh
tools/cts/cts-build.sh
```

Needs a VK-GL-CTS checkout (default `~/com/github/KhronosGroup/VK-GL-CTS`, set
`CTS=` for another). The script

1. unpacks the Bootlin `aarch64--glibc--stable-2024.05-1` SDK under
   `build/cts/toolchain/` and relocates it. A pruned copy of that SDK without
   `Scrt1.o` and `crti.o` cannot link an executable. Put a downloaded tarball
   at `build/cts/toolchain/aarch64--glibc--stable-2024.05-1.tar.xz` (or
   `BOOTLIN_TARBALL=`) to build without network;
2. applies `cts-yakogl-target.patch` to the CTS checkout if the target is not
   already there, and leaves it **uncommitted**: the checkout is the user's;
3. builds into `build/cts/aarch64` (gitignored).

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
- **Raspberry Pi 4**: section 5.

**ES 3.0 gl_test cases.** An ES 3.0 gl_test image needs `CONFIG_GL_TEST_CASES` for `gl_test`; `tools/cts/common-images.sh --cases 202,203 am62p ...` sets `CONFIG_GL_TEST_CASES` for the gl_test row (the Pi 4's `CONFIG_V3D_GL_TEST_CASE` for one case, `CONFIG_V3D_GL_TEST_CASES` for a list; `--pipe` adds `-pipe` to the compile; use a new `<img-dir>` when the case list changes, a build directory made with other arguments is refused). The expected readings are in the cases themselves (`driver/tools/pvrlab/gl_test.c`): 196 `occl` q0 to q4 `available 1` with 0, 1, 0, 1, 1; 202 `blitconv` 12 / 12; 203 `tfcap` 9 / 9; 204 `srgb` 8 / 8; 205 `mappbo` 15 / 15. Each skips by name in an ES 2.0 context.

Board rules for every run: one GL client per board (check that the board is idle first); images only from commits on origin/main, built from a detached checkout (section 2d), with the commit recorded beside each result; `BOARD_BOOT=uboot-mmc` on the two Verdins, tryboot (board.sh's default) on the Pi 4. The ES 2.0 sign-off runs are one command each, `tools/cts/board-stage.sh <board> common|deqp` (and `am62p ms4`); that script's header names the environment.

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

### The common set: two images, one command per board

Before anything else, every board runs the common set
(`tools/cts/common-tree.txt`, driver/docs/one-method.md): glmark2 (GM) and
gl_test with the sample's default case list (GT), one image each, from the
bootloader. Tests and benchmarks only: the Slint demo is kept working but is
no row (the user's decision of 2026-10-19; until then the set also held two
Slint rows, SO and SV, and the AGL cluster sample, CL, since removed).

```sh
tools/cts/cts-common.sh <am62p|pi4|imx8mp> build <img-dir>
tools/cts/cts-common.sh <am62p|pi4|imx8mp> run <img-dir> <outdir> [<earlier outdir>]
tools/cts/cts-common.sh <am62p|pi4|imx8mp> tally <outdir> [<earlier outdir>]
```

`build` is `tools/cts/common-images.sh` (its header has the environment:
`ZEPHYR_WS`, `GLMARK2_SRC` and the AM62P firmware header); it gives every
image its board's watchdog self-arm and reports an image without the arm line
as `rc=no_marker`. `run` is
`tools/board.sh <board> tree` on the table, then `tools/cts/common-tally.py`:
per row the record words, board.sh's verdict recomputed from the files, the
glmark2 score with its failed and unsupported scenes and, from the GM run's
`console.txt` (the AM62P and the Pi 4), a per-scene verdict: the lines of the
`GLMARK2_BAD_LINES` table at the top of the script (a GL error, an incomplete
texture sampled, a refused or unrecorded draw, out of GPU memory, a refused
tiled copy; each row cites its RESULTS.md section) are given to their scene by
the next scene's result line (the sample's `glmark2-mem: entry N before
setup|end:` lines carry no scene name; a console without any such line gets the
remark "no glmark2-mem lines"), and the cell reads `amber` with each scene and its line counts
when one has any and the verdict is done, `failed` when the verdict is, else
`green` (`green (record only)` on the i.MX8MP, which has no console; amber
does not change the exit status), gl_test's passed of run with the failing case
numbers (on the i.MX8MP only the
first: its CASE record carries no more), the GPU resets with their source
(`resets (record) N` when the record carries a count, `resets (console count) N`
from board.sh's count of the console's recovery lines, else `resets absent`) and the image's sha256
and build ids; with an earlier common run, the row-by-row diff (for the GM row
also the cell and each scene's changed line counts). `--dry-run`
prints every command and does nothing. `common-tally.py --self-test` runs in
`tools/check.sh` (it replays the `glmark2-*.txt`
console snippets in `tools/cts/testdata`; `cts-selftest.sh` check 22 runs the
GM row and its diff through the command line);
`tools/board_selftest.sh` checks the three boards' dry runs.

Speed is read per scene, in ms per frame, from same-board glmark2 runs, never
from a demo's fps. Two modes of `common-tally.py` do it; they judge nothing and
exit 0 (2 when a console has no scene line). `SRC` is a glmark2 console file or
a run directory (its `console.txt`):

    tools/cts/common-tally.py --gm-table SRC
    tools/cts/common-tally.py --gm-diff A B

`--gm-table` prints one row per scene (name with options, FPS, ms/frame, `ms
from`) read from the scene's result line, then, where the console has the
`glmark2-frame:` line, the GL-call time per frame (`gl ms`) and the `end waited
for in N of N` counts; then the score. ms/frame is glmark2's own `FrameTime`
(three decimals) when the line prints it, and 1000 / FPS only when it does not
(the FPS is an integer: refract at FPS 3 is 1000 / 3 = 333.33 but its
FrameTime is 403.8 ms). `ms from` says which was used, `FrameTime` or
`1000/FPS`, and the last line counts both. A scene that set up failed or is
unsupported has `-`. A scene run twice is `name #2`.

glmark2 prints `<scene info>:` when a scene starts and ` FPS: n FrameTime: x
ms` when it ends, so everything the scene prints in between (the driver's
lines, GL errors) sits between the two and the result line has no scene in
front of it. The parser names such a line by, in this order, the
`glmark2-mem: entry N <info>: before setup` line that opened the scene, the
`glmark2-frame:` line right after the result line (it has the full info), or
the header line before it cut at the first part that is not an option (the
last way loses nothing but a trailing option that holds no `=`). Until
2026-10-18 only a result line with its scene was read: five scenes (use-vbo=false,
gouraud, blinn-phong-inf, terrain, shadow) were missing from 28 of 33.

`--gm-diff` takes two consoles of the same board, A the earlier. Per scene it
prints the ms/frame of A and B, the delta in ms and in % (B against A,
positive is slower) and `*` when the scene moved more than 5%; a scene with no
FPS in one run or present in one run only is named so, and a row whose A or B
ms is 1000 / FPS says `(A: 1000/FPS)` or `(B: 1000/FPS)`. The last lines count
the scenes from FrameTime and from 1000 / FPS in each run, then give the score
of each run and its change. Examples, on fixtures:
`common-tally.py --gm-diff tools/cts/testdata/glmark2-speed-a.txt
tools/cts/testdata/glmark2-speed-b.txt` (`cts-selftest.sh` check 22b) and
`glmark2-speed-late-{a,b}.txt`, the shape of a real console: FPS lines after
other lines, refract with FPS 3 and FrameTime 403.8, a scene with no
FrameTime.

### Resuming an interrupted common set

`tools/cts/board-stage.sh <board> common` can be run again at the same commit
and picks up where it stopped, with no hand cleanup. `common-images.sh` writes
`<img-dir>/<LABEL>/.complete` (HEAD, the image's sha256, the build ids) as the
last step of a good build. A directory with the marker, the same HEAD and an
image that still matches its sha256 is reused (the row's `.built` line is
written again, nothing is built); a directory without the marker is an
interrupted build and is deleted and rebuilt; a marked directory from another
commit, or whose image changed, is refused (`rc=exists`) as before. On the
two Verdins, `board-stage.sh` also removes any `once-*.bin` that a killed run
left in the board's stage directory and says so in one line. Rows with a
finished record stay skipped (`board.sh tree`).

### What `board-stage.sh` requires per board

`board-stage.sh` checks the dependency layout (`deps-env.sh --check`) but
stops only for the entries the board and the set read. An entry that is
missing and not in the list below is ignored: the shader compilers and Mesa
trees (`PCO_SHC`, `MESA_PAN_TREE`, `GLSLANG`, `GLSL2USC_CACHE`) and
`HAL_NXP_DIR` belong to the gate, and no image is built with them. A missing
entry in the list stops the run (exit 2) with the same message as before.

| Set | Required entries |
| --- | --- |
| every image | `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` |
| `common` (add) | `GLMARK2_SRC` |
| `am62p` (add) | `PVR_FW_BLOB` |
| `imx8mp` (add) | `IMX8MP_OUT` |
| `deqp`, `ms4` (add) | `DEQP_WORK` (am62p, pi4), `DEQP_STAGE1` (imx8mp) |

The list is `needed_deps` in `board-stage.sh`; `images-selftest.sh` checks
both directions against the real `deps.toml`, and against what the builders
read: every `deps.toml` entry (`${NAME`) that `cts-common.sh`,
`common-images.sh`, `zephyr-modules.sh`, `*-deqp-images.sh`,
`cts-build-zephyr.sh` or an `imx8mp-*.sh` script reads must be in some row
of the table; a builder that
starts to read another tree adds it to `needed_deps` in the same commit, and
a new builder joins the `builders` list of the self-test (which plants such
a read in a scratch copy to show the guard fires). The Pi 4 uses
`MESA_V3D_TREE` in no stage; it is not a `deps.toml` entry, so it has no
row.

### The glmark2 picture

A score says how fast a scene ran, not that it drew the right picture. The
glmark2 image prints a `glmark2-picture:` line per scene (probes, crc16, mean,
black; `driver/docs/glmark2.md`, "The picture value"), and the tally judges
it against `tools/cts/glmark2-pictures.txt`, one line per scene and column:

    <column> | <scene info string> | <the text after "glmark2-picture: <scene>: ">

The column is the board (`am62p`, `pi4`, `imx8mp`) the run was made on, or
`host`. A GM row is judged against the column of its board; each scene is:

- `match`: every probe's R, G, B and the mean's R, G, B are within the
  table's `tolerance` (8 levels, as the Slint probes allow a few levels of
  rounding between GPUs) of the reference, and the size is the same. `crc16`
  is informational: a differing crc16 is named, never judged.
- `mismatch`: a probe or the mean is further off, or the size differs.
- `black`: all black where the reference is not (a lost picture).
- `absent`: the readback failed (the value starts with `absent`:
  `glmark2-picture: <scene>: absent ...`).
- `unreadable`: the line has a value that does not parse (one character
  too many or too few, a console that lost a character). The row prints the
  first 80 characters of the value as it stands in the console; read the
  line itself (`grep -a glmark2-picture console.txt | cat -A`). It is not a
  failed readback.
- `no reference`: the board's column has no line for the scene.

The cell: `black` makes it `failed`; `mismatch`, `absent`, `unreadable` and `no reference`
make it `amber` (the verdict stays `done`, so the exit status is 0); all
`match` leaves it as the known-bad lines made it. A console with no picture
line (an image from before the picture value) is not judged on pictures. The
row prints a summary and each scene that is not a plain match; `--prev`
names a scene whose state changed.

    tools/cts/common-tally.py --gm-pictures SRC COLUMN [--table FILE]
    tools/cts/common-tally.py --gm-picture-lines SRC COLUMN

`--gm-pictures` judges one console against a column (exit 1 unless every
scene matches, 2 with no picture line). `--gm-picture-lines` prints the
console's pictures as table lines for COLUMN: a board session runs it on a
console whose pictures were looked at, and adds the lines to the table. Never
copy a column from another board: a board's reference is its own run.

The `host` column is what the host harness (`tools/hostharness/glmark2`)
prints: it draws nothing, so every scene is black with the crc16 of an
all-zero grid. It checks the parser and the table; no board is judged
against it. Fixtures: `testdata/glmark2-picture-{match,mismatch,black,absent,unreadable}.txt`
against `testdata/glmark2-pictures-test.txt` (`cts-selftest.sh` check 22c;
`COMMON_TALLY_PICTURES` names another table).

#### The i.MX8MP pages

The i.MX8MP has no console. `tools/board.sh imx8mp gm <image> <outdir>
fps|pictures` runs the page boots and `gm-lines <outdir>` decodes them
into `<outdir>/glmark2-pages.txt`: console-style lines (`driver/docs/glmark2.md`,
"Per scene without a console"; design `docs/notes/imx8mp-gm-record.md`). The
tally reads that file:

- **The GM row**, when it has no `console.txt` and a `glmark2-pages.txt` is in
  its run directory or the directory that holds it (a tree's outdir), takes
  the speed and the pictures from it. The cell is `green (record and pages)`,
  or `amber` / `failed` by the pictures as for a console; the known-bad lines
  (GL error, incomplete texture, refusals) are not in the record, so the
  per-scene known-bad verdict is not given and the row says so. A console, if
  there is one, wins.
- **`--gm-table`, `--gm-diff`, `--gm-pictures`, `--gm-picture-lines`** take the
  file as they take a console. ms/frame is 1000 / the integer FPS (`ms from`
  says `1000/FPS` on every row): the file's `FrameTime` is derived from the same
  integer, which glmark2 rounds up, so it is not glmark2's own. A diff of a
  pages file against a console marks the rows `(A: 1000/FPS)`.
- **What the record drops is `not in record`, never a failure**: `FrameTime`,
  the `glmark2-frame:` means (the `gl ms` and `end waited` cells of the table),
  the `glmark2-mem:` lines, the GL error lines, the score of a page run (the
  GLMARK2 record has the full run's), and in a picture the alpha and the low
  bits of each probe channel (RGB565; a step is 8 in R and B, 4 in G, inside the
  table's tolerance 8). A scene whose P0 or P1 page was not run has
  `absent (page P0 of scene N missing)` in the file: the tally calls its picture
  `not in record` (counted, listed, no amber); a failed readback is still
  `absent`.

Fixture: `testdata/imx8mp-pages/` (`glmark2-pages.txt` is `board.sh gm-lines`
run on the 18 synthetic page runs of `driver/rust/tests/gm-record/runs`, byte
for byte `driver/rust/tests/gm-record/glmark2-pages.txt`; a GM run directory
with a synthetic record and a `tree.txt` beside it);
`cts-selftest.sh` check 22d.

The board images compile shaders in the image (the online compiler) or look
them up in the offline table.

Running the pages: build the GM image as the common set does (`tools/cts/common-images.sh imx8mp`) and run the full common set first (score, scene counts and GL error counters come from it). Then `tools/board.sh imx8mp gm <GM image> <common outdir> pictures` (66 boots, about 1.9 h; `fps` is 11 boots, 23 min; resumable by rerunning; `GM_ROWS=22` runs one scene). The `gm` verb ends with `gm-lines`, which writes and prints `<outdir>/glmark2-pages.txt`; its first line names every missing page: rerun those (`GM_ROWS`), then `tools/board.sh imx8mp gm-lines <outdir>`. `common-tally.py --gm-table <outdir>/glmark2-pages.txt` gives the per-scene ms/frame; `--gm-picture-lines <outdir>/glmark2-pages.txt imx8mp` prints the column lines. An A/B of the screen buffers needs only `fps`.
- A killed `gm` run leaves a `once-*.bin` in the board's stage directory that must be removed (as `board-stage.sh` does, or `board.sh imx8mp disarm`) before the runner proceeds. Background tasks need a timeout of at least 7200000 ms for the 66 boots.
- Caveats: the probes are RGB565 floors (alpha `00`), so the `imx8mp` column must come from this board's pages; a `mismatch` off by exactly 16 between two runs of the same image is two 565 steps, not noise. Each page runs its scenes first in a fresh boot, so compare page FPS with page FPS, not with the full run's score (the warm-up difference between a cut list and the full run is not measured). A page run's own GLMARK2 record is replaced by the page (its score and GL error counters are not read). A scene that sets up but draws no frame is stored with fps 0 (the decode prints `FPS: 0`, the tally gives it no ms/frame).

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
GPU=v3d tools/cts/host-deqp-run.sh           # the V3D's online back end
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

`GPU` is `vivante` (the default), `powervr` or `v3d` (`build/host-deqp/lib-v3d`,
the run in `run-<commit>-v3d`; the script exports
`YAKOGL_V3D_TILED_COPIES_BYTES=16777216` unless the caller set it, as the Pi 4
image does). The whole gles3 mustpass on all three: `ab43a20c5:docs/es3-host-run.md`.
`MODULE=gles3` runs dEQP-GLES3 instead: it builds `deqp-gles3` in
its own directory, links a library that reports ES 3.0
(`YAKOGL_REPORT_ES2=OFF`), and runs the union of the checkout's
`android/cts/main/gles3-main-*.txt`. The argument is a substring filter on
case names, never a command.

```sh
export CTS=<VK-GL-CTS>                                   # checked out at 93bca01, the revision RESULTS.md uses
python3 $CTS/external/fetch_sources.py                   # once per checkout
git -C $CTS apply $PWD/tools/cts/cts-yakogl-target.patch # or tools/cts/cts-build.sh, which applies it
tools/cts/host-deqp-run.sh info                          # gles2
MODULE=gles3 tools/cts/host-deqp-run.sh dEQP-GLES3.info. # gles3
```

| module | deqp build dir | library dir | run dir |
|---|---|---|---|
| gles2 | `build/host-deqp/deqp` | `build/host-deqp/lib` (`lib-powervr`, `lib-v3d`) | `build/host-deqp/run-<commit>` (`-powervr`, `-v3d`) |
| gles3 | `build/host-deqp/deqp-gles3` | `build/host-deqp/lib-es3` (`lib-powervr-es3`, `lib-v3d-es3`) | `build/host-deqp/run-<commit>-gles3` (`-powervr-gles3`, `-v3d-gles3`) |

A run removes and recreates its run directory, so two runs of the same module
and commit at once need their own `OUT=`. On 2026-10-08 the gles3 `info`
cases reported `OpenGL ES 3.0 YakoGL` and `OpenGL ES GLSL ES 3.00`
(HOST-ONLY).

## 2c. The whole tree on the Verdin i.MX8M Plus, one image

The i.MX8MP has no UART, so its tree is one dEQP image that carries the
29-entry table (`imx8mp-deqp-build.sh --table tools/cts/imx8mp-full-tree.txt`).
`tools/board.sh imx8mp tree` boots it once per entry with the selector in SNVS
word 1, decodes each SNVS record, reads the failing names from the DEQP_FAILS
pages, and writes `results.tsv` (one line per entry: index, filter, begun,
passed, failed, notsup, other, noresult, names) and `rows.md`. `TREE_PREV=<an
earlier outdir>` adds the per-entry diff. The commands are in 2d.

gl_test images on the i.MX8MP:

- Environment of the i.MX8MP build scripts: `export ZEPHYR_VENV=$ZEPHYR_WS/.venv IMX8MP_OUT=<out> DEQP_STAGE1=<stage 1>` (`tools/cts/imx8mp-env.sh` checks it and sets `ZEPHYR_BASE` from the workspace).
- gl_test builds need `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`, else `board.sh` refuses the image (rc=3). A `;` inside a sweep list is written `\;`.
- An interrupted run can leave `/var/yakogl/once-*.bin` on the board: `tools/board.sh imx8mp disarm` before retrying (`board-stage.sh` removes them itself). One boot takes about 2 to 2.5 minutes.
- Free space in the deps cache (`IMX8MP_OUT`) before a tree run: a row once stopped on a full local disk quota (board.sh could not write its `once` token) and was rerun from the same records directory.

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
run; no full-tree wall time is recorded. The last tree's numbers are in
STATUS.md. The tally of a new sweep is that table: 17 image rows plus `ms4`,
and `tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles2-mustpass.txt <console logs of both trees>`
for the combined mustpass count (it keeps the last status of a case, so the
MS4 log replaces image 17's NotSupported multisample rows when named last).

**i.MX8M Plus** (`BOARD=verdin_imx8mp/mimx8ml8/a53`; `BOARD_BOOT=uboot-tftp`
or `uboot-mmc`; no UART: the record is in the SNVS). One image carries all 29
rows, picked by selector:

```sh
export ZEPHYR_VENV=$ZEPHYR_WS/.venv IMX8MP_OUT=<out> DEQP_STAGE1=<stage 1>
$SW/tools/cts/imx8mp-deqp-stage1.sh                                   # stage 1, log $IMX8MP_OUT/deqp-stage1.log
$SW/tools/cts/imx8mp-deqp-build.sh wt-$HEAD8 'dEQP-GLES2.info.*' --table $SW/tools/cts/imx8mp-full-tree.txt
TREE_IMAGE=$IMX8MP_OUT/wt-$HEAD8/zephyr/zephyr.bin $SW/tools/board.sh imx8mp tree $SW/tools/cts/imx8mp-full-tree.txt <rec>/full
$SW/tools/board.sh imx8mp tally <rec>/full                            # results.tsv, rows.md
```

The build prints `rc=0` and, for the image's `.config`, the
`check-deqp-images.py` verdict. Time: 29 selector boots, each at most the
600 s WDOG1 bound (`CONFIG_GC_BOARD_WDOG_SECONDS`) plus the boot, so at most
about 5 h; not recorded as a whole. The tally is `results.tsv` (2c); the
committed reference is `imx8mp-full-tree-29460f96.results.tsv`
(2026-10-03), and RESULTS.md's i.MX8MP section at d8583f20 has the per-entry
table. Compare with `diff <(cut -f1-8 <that file>) <(cut -f1-8 <rec>/full/results.tsv)`
or `tools/board.sh imx8mp diff <earlier tree dir> <rec>/full` when the
earlier tree's directory is kept. Known intermittent rows: see
issues #25, #9, #8.

**Raspberry Pi 4** (`BOARD=rpi_4b` for stage 1; backend tryboot, so no
`BOARD_BOOT`; `BOARD_UART` the debug probe's by-id device or its bridge's `host:port`,
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
2 h 40 min on the board. The last tree's numbers are in STATUS.md; compare a
new tree with `tools/board.sh pi4 diff`.

For all three, an image with no `deqp: summary` line, or fewer case lines than
its summary counts, is a missing result, never a pass
(`deqp-uart-tally.py`'s MISSING, INCOMPLETE and FATAL); rerun that image
(`TREE_ROWS=<n>`; a finished run of the same image is skipped).

The tally tells a case the capture lost from a case that never ran. NoResult
is a mustpass case in the records' case filters with no line, which the image
never began. Lost is a case the image counted as run (`deqp: summary: N cases
begun, M with a result`; `deqp: done: ...`; on the Pi 4 also `v3d_deqp: build B
DONE...: N cases, P p F f ...`) whose lines the console never delivered: the
image's M against the result lines seen, placed by the gaps in the
`v3d_deqp: case #N` numbering (the AM62P image prints no such
number, so there the count is known and the cases stay in `none`). A
`Lost in capture` line names each range, the cases either side, the counters,
and (time-stamped logs) a silent stretch of the console; `--table` adds a
`lost` column (only when there is one), and the pass/fail totals stay the lines
seen, with the image's counters printed under them. `deqp-uart-tally.py --lost
<console>` prints the count; `board.sh` turns it into the verdict `done_lossy`
with `lost=<n>` (rc 4; every other verdict keeps its meaning, and
`common-tally.py` counts it as not `done`). Fixture:
`tools/testdata/pi4-tryboot-F1rerun2-a27d69c0.console.txt`, the deqp lines of the
Pi 4 F1 rerun that lost 260 of 1923 cases (self-test and `board_selftest.sh`).

## 2e. The AM62P dEQP-GLES3 tree

`am62p-gles3-tree.txt` is generated (`tools/cts/gen-am62p-gles3-tree.py`; `--check`
is a gate row in `tools/check.sh`) from `gles3-mustpass.txt`: 46 images, every
mustpass case matched by exactly one line. The board resets the SoC 600 s after
an image booted (the RTI watchdog, `CONFIG_YAKOGL_AM62P_WDT_SECONDS`, minus the
boot), so a row's run must end well inside that; the generator keeps each image
under about 480 s by splitting what the board measured as slow, and a new slow
group is split there, not in the table.

Measured on 2026-10-10 from main `c4ccc966`
(`records/am62p/deqp-gles3-c4ccc966/{full,full2}`): the span from the first to the
last `Test case` line of each image's console (cases are those dEQP began, which
includes cases outside the mustpass list that the filters also match).

| Image (old numbering) | cases begun | span |
|---|---|---|
| 1 | 1,726 | 78 s |
| 2 | 1,956 | 71 s |
| 3 | 1,683 | 77 s |
| 4 | 1,981 | 150 s |
| 5 | 1,085 (stopped early, see below) | 44 s |
| 6 | 1,121 | 59 s |
| 7 | 1,085 (0 passed, stopped early) | 116 s |
| 8 | 2,333 | 79 s |
| 9 | 2,051 | 83 s |
| 10 | 1,425 | 69 s |
| 11 | 1,818 | 83 s |
| 12 | 1,890 | 97 s |
| 13 | 1,924 | 99 s |
| 14 | 1,926 | 97 s |
| 15 | 1,437 | 55 s |
| 16 | 2,020 | 76 s |
| 17 | 1,466 | 57 s (the first run left no record, `scan_failed`; the rerun passed) |
| 18 | 2,021 | 91 s |
| 19 | 485 begun, never finished | 459 s, then the watchdog |
| 20 | 1,723 | 258 s |
| 21 | 1,440 | 45 s |
| 22 | 2,421 | 90 s |
| 23 | 1,916 | 87 s |
| 24 | 1,532 | 50 s |
| 25 | 1,011 | 103 s |

Old image 19 (`texture.filtering.*`, `format`, `mipmap`, `shadow`, `size`) ran
`filtering.2d` (234 cases begun, 13 s), `2d_array` (222 begun, 100 s) and then
`filtering.3d.combinations`: the 27 `linear_linear_*` cases took 0 to 1 s each,
then `linear_mipmap_linear_linear_clamp_clamp_clamp` took 337 s and the next one
(`..._clamp_clamp_mirror`) had run 141 s more when the watchdog fired. Those
`linear_mipmap_*` and `nearest_mipmap_*` 3D cases are not in the mustpass list
(its 3d.combinations are `linear_linear_*`, `linear_nearest_*` and `nearest_*`),
so the tally never counted them; the `3d.combinations` filter now names only
the mustpass prefixes. `filtering.3d.formats` and `3d.sizes` are named the
same way: one image each for their cases but the `*_linear_mipmap_linear` ones,
and each of those 15 cases in an image of its own (`ISOLATE` in the generator),
so a hang or a slow check there costs only that case
(`docs/notes/pvr-3d-mipmap-hang.md`).

Old image 11 had four GPU context resets, all in `shaders.metamorphic.bubblesort_flag`
(`variant_1` three times, `variant_2` once); the driver recovered within 3 s each
and the image finished with 1,816 pass and 2 fail in 83 s, so it keeps its row.

Not caused by time and not changed here: old image 5 reports `complete=0`
(it ends after `implementation_limits.max_server_wait_timeout`).

Old image 7 (`builtin_functions`) stopped at
`precision.clamp.highp_fragment.vec3` with `ResourceError` until `ffee428a`.
On the board on 2026-10-25 (main `be84094b`, bridge on the console) that case
passed, the 48 `common.round` / `roundeven` cases passed, and `common.modf`
(24 cases) failed. But the image ran only 976 cases before the 600 s watchdog:
the `precision` cases take about 0.7 to 1.1 s each (584 of the 1,147 in 420 s;
a second image ran 561 in 600 s). The generator now weights them 6x (`WEIGHT`)
and divides `builtin_functions` and `builtin_functions.precision` by function
name, so `precision` is spread over several images (31 then; 46 with the
`3d.formats` / `3d.sizes` split above). The 6x is an empirical figure: the new image 7 ended `done` with a
`back_after` of 531 s against the 600 s watchdog, a thin margin; if a later
tree resets it, raise `WEIGHT` before anything else.

The table above is the 2026-10-10 measurement that set today's split. The
first tree on the new table (`f2a13877`, 2026-10-11, RESULTS.md) measured
the new texture rows: images 20 (`texture.filtering.3d.formats`) and 21
(`texture.filtering.3d.sizes`; both numbers are of that tree, before the
split into 46 images) hang the GPU on their 14th case (a
`linear_mipmap_linear` 3D case each), and the board comes back only through
the RTI reset. Every other image of that tree ended `done` (image 11 with 4
resets). Images 5 and 7 still stop early, for the reasons above.

AM62P board runs need `BOARD_BOOT=uboot-mmc` (or `uboot-tftp`), `BOARD_UART` and `driver/src/fw/pvr_fw_blob.h` in the checkout (never committed). The console must be the main_uart0 port of the bridge: check it with `python3 tools/am62p_netboot_uart.py probe <host:port>` and a raw read that shows bytes before the first image. A stale one-shot image left in `/var/yakogl/` by an interrupted run (`once-img8-*.bin`, ...) makes `board.sh` refuse (rc=3) and blocks `board.sh tree`: `tools/board.sh am62p disarm` clears the U-Boot env, and the file itself may have to be removed by hand (`board-stage.sh am62p common` removes it before a run). A bridge capture timeout (`verdict=scan_failed`, rc 4) is a transport problem: rerun the row. Another session's dEQP tree may hold the board for hours; on the AM62P `TREE_RETRIES` is 0, so wait for a quiet board. A gles3 stage 1 is not the farm's `DEQP_WORK` (GLES2): build one with `tools/cts/cts-build-zephyr.sh MODULE=gles3`.

### The console under a farm console lease (feed bridge)

A console lease's serial MCP server holds the board's serial bridge for the whole agent turn, so `board.sh`'s `probe` / `capture` get `busy` (rc 4) on it. (A farm stage that runs `board-stage.sh` as a command does not hold the bridge and does not hit this.) Use the feed bridge: it serves a growing file as a local serial bridge, and `board.sh` runs unchanged beside the MCP server.

```sh
FEED=$PWD/build/am62p-console                      # any path outside git
python3 tools/am62p_netboot_uart.py feed "$FEED" <free local port> &   # SIGTERM stops it
FEED_PID=$!
export BOARD_UART=127.0.0.1:<free local port>      # board.sh and cts-common.sh read it
# ... run board.sh / cts-common.sh as usual (BOARD_HOST, BOARD_BOOT, ...)
kill "$FEED_PID"
```

While the run goes on, the agent:
1. reads the MCP console as often as it can and appends each piece to `$FEED` unchanged (`printf '%s' "$piece" >> "$FEED"`; whole pieces, no re-ordering);
2. forwards what a client typed: the feed appends keystrokes sent into the bridge to `$FEED.tx`; send the new bytes to the MCP console. Only the typed route (`NETBOOT_ROUTE=typed`, TFTP boot) types; the `uboot-mmc` route of the common set does not, so `$FEED.tx` stays empty.

How the feed behaves (same protocol as the real bridge; the script's docstring, "Feed bridge"): one client at a time (a second gets `busy\r\n` and is closed, so `probe` exits 4; a quiet client is held open, so `probe` exits 0). A client gets only bytes appended after it connected (start `board.sh` first, its capture connects before the reboot, then append); bytes appended while no client is connected are dropped; a file that shrinks is read again from its start. `console.ts` times are the times `capture` received the bytes, as late as the agent's appends: append at least every few seconds, and make no timing claim from such a run. A relay must pass the `\r\n` after `Starting kernel ...` (`board.sh`'s `scan --back` waits for the line ending). SIGTERM ends the feed with exit 0. Host check: `python3 tools/am62p_netboot_uart.py --self-test`. Used on a real board on 2026-10-18 (7 rows, all `verdict=done`). The same works for the Pi 4's bridge.

The AM62P common set by hand, as run on 2026-10-17:
```sh
export $(tools/cts/deps-env.sh am62p | xargs)
mkdir -p driver/src/fw && cp "$PVR_FW_BLOB" driver/src/fw/   # never committed
tools/board.sh am62p disarm
tools/cts/cts-common.sh am62p build build/common/am62p-<sha8>
BOARD_HOST=<lease ssh> BOARD_UART=<main_uart0 bridge host:port> BOARD_BOOT=uboot-mmc \
  tools/cts/cts-common.sh am62p run build/common/am62p-<sha8> records/am62p/common-<sha8>
```

## 3. Reading the numbers

| outcome | what it means |
|---|---|
| `Pass` / `Fail` | what the test decided. |
| `NotSupported` | the test asked for something the implementation does not advertise and skipped itself. **Allowed** in an AOSP mustpass run — it is not a failure, but a large count means a capability is not being advertised. |
| `QualityWarning`, `CompatibilityWarning` | allowed too: right answer, outside a preferred precision or behaviour. |
| `Crash`, `Timeout`, `InternalError`, `ResourceError` | a driver bug. Each one also costs a process restart, so a run with many of them is much slower than its case count suggests. |
| NoResult | a mustpass case the image never began (2d). |
| Lost | a case the image ran whose result line the capture dropped (2d). Not a failure and not NoResult. |

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

## 5. Raspberry Pi 4 (V3D 4.2, Zephyr only)

The Pi 4 boots Raspberry Pi OS by default. Each Zephyr image runs once through
the firmware's tryboot and ends with its PM watchdog reset, which brings the
board back to Linux (`docs/standalone-boot.md`, "Raspberry Pi 4").
The record is read from the Pi's UART (GPIO 14/15), never from the HDMI panel:
`BOARD_UART` is either the serial device wired to it on this host or
`host:port` of a serial bridge (a raw TCP stream of the UART, one client at a
time, as the AM62P's console).

- `tools/board.sh pi4 run <zephyr.bin> <logdir> [name]` (backend tryboot):
  one image, once. It refuses (exit 3, nothing changed on the board) when the
  board does not answer, when `who` there shows a session from another
  machine, when a process on this host holds the UART device (a bridge that
  answers `busy` 12 times, 10 s apart, refuses after staging, with the image
  removed again and no reboot), or when `/boot/firmware` has less than twice
  the image free; it reads the staged image and `tryboot.txt` back before the
  reboot, captures the UART into the run directory's `console.txt` (from a
  bridge: `console.raw` and `console.ts` from `tools/am62p_netboot_uart.py
  capture`, `console.txt` derived from them) until the record's end line, waits for Linux
  (a new boot_id), removes the image and puts the old `tryboot.txt` back.
- `tools/cts/pi4-deqp-images.sh <table> <img-dir>` builds the 29 images of
  `pi4-full-tree.txt` (one `<img-dir>/<LABEL>/` each, never reused);
  `TREE_IMAGE_DIR=<img-dir> tools/board.sh pi4 tree pi4-full-tree.txt <outdir>`
  runs them in file order and tallies: per group with every non-Pass case by
  name, the recoveries per record; `tools/board.sh pi4 diff <a> <b>` is the
  per-case diff of two trees.
- The console arrives through a serial bridge that can stall and drop its
  backlog (a TCP bridge, 2026-10-10: 260 of 1923 cases of an F1 rerun, 34 of
  another, while the image's own summary counted all of them). Such a row
  reads `verdict=done_lossy ... lost=<n>` in `board.sh pi4 run|tally|tree`, and
  the tally lists the lost case numbers as `Lost`, not NoResult. The image ran
  them; the capture does not show their results, so the row is not a result:
  rerun it until the verdict is `done` (`TREE_ROWS=<n>`). `console.ts` has the
  arrival times; a stall shows there as a silent stretch (named, informational,
  in the Lost lines), and the lines dropped after it are not at the hole itself.

Nothing in the scripts names a machine. The environment, with the lab's
values as placeholders (`tools/cts/deps-env.sh` prints the dependency
layout):

```sh
export BOARD_HOST=<user>@<pi4>                 # key login, passwordless sudo (PI4_HOST is read too)
# the Raspberry Pi Debug Probe; its ttyACM number changes when USB re-enumerates
# (ttyACM0 on 2026-09-25, ttyACM2 on 2026-10-03), the by-id name does not
export BOARD_UART=/dev/serial/by-id/usb-Raspberry_Pi_Debug_Probe__CMSIS-DAP__<serial>-if01
# or the probe behind a serial bridge (usb-serial-over-tailscale): its host:port
# export BOARD_UART=<bridge>:<port>
# the 1920x720 bar panel (17 fields, as the board's own tryboot.txt had them)
export BOARD_HDMI_TIMINGS='1920 1 50 44 50 720 1 10 20 50 0 0 0 60 0 99070000 1'
# build
export ZEPHYR_WS=<west workspace>              # Zephyr af17c0c8
export WEST_VENV=$ZEPHYR_WS/.venv ZEPHYR_SDK_INSTALL_DIR=<zephyr-sdk-1.0.1>
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
`driver/docs/rpi4-v3d.md`, "Measured on the board").

A leftover `/boot/firmware/yakogl-*.bin` or a non-empty `tryboot.txt` refuses the next run; `tools/board.sh pi4 status` shows both. `BOARD_HDMI_TIMINGS` must be the lab's 17 fields (never `1920x720`; `board.sh` refuses anything that is not 17 integers, #905). If the cube rows of the GLES2 tree (T3, T4) fail, read `docs/v3d-noseamless-cube.md`.
