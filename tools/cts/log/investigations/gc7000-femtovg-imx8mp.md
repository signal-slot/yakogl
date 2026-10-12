<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# gl_femtovg on the i.MX8MP (GC7000): not buildable, and what it would take

Written 2026-10-12 from the code at `db3545fa25e2` (the integration tip; `gl_femtovg/` and the board
layer are as on `origin/main`). Host only: it ran nothing on a board and changed no code, config or
table. Each finding is marked **[read]** (read from a file at that commit), **[inferred]** (follows
from what was read; not run) or **[not run]**.

**The build was run** (item femtovg-build-evidence, 2026-10-12): section 2 quotes the observed
commands, their exit status 1 and the errors. An earlier attempt on another node was refused by the
farm's permission layer; the "inferred" first error of that version has been replaced by the
observed one.

## Answer in short

`gl_femtovg` has no i.MX8MP board configuration and its one board configuration drives the PowerVR
and the AM62P display. It cannot be built for the Verdin i.MX8MP from `origin/main` or from PR #129
without source changes. **The offline GC7000 table is not the blocker**: all 36 femtovg rows are in
both tables. The same patched femtovg and glow run on this board inside the `slint` sample.

## Quotable line for the board-session record

> gl_femtovg: nothing ran: not buildable for the GC7000 (west build for verdin_imx8mp exits 1: first
> `FVG_NOSTD_DIR is not set`, then with it `pvr_ddk.h: No such file`; no i.MX8MP board configuration; prj.conf sets
> CONFIG_SAFEUI_GPU_PVR=y and main.c drives the AM62P DSS; driver/docs/one-method.md lists the
> GC7000 femtovg sample as "none"; investigations/gc7000-femtovg-imx8mp.md). Its shaders are the
> 36 femtovg rows that gl_slint compiles, 36 of 36 in the table on main and on PR #129.

## 1. What exists [read]

| Piece | What it is |
|---|---|
| `driver/samples/zephyr/gl_femtovg/` | `CMakeLists.txt`, `Kconfig`, `prj.conf`, `src/main.c` (154 lines), `rust/` (the `fvg_zephyr` staticlib, 98 lines in `src/lib.rs`), `boards/verdin_am62p_am62p54_a53.{conf,overlay}` |
| its board configs | the AM62P only. No `verdin_imx8mp_mimx8ml8_a53.*`, no `rpi_4b.*` |
| `prj.conf` (22 lines) | `CONFIG_SAFEUI_GPU_PVR=y` on its first line, then `YAKOGL`, `LOG`, `FPU`, the 16 MB libc arena and a 256 KiB main stack. The AM62P-only lines (`MBOX_TI_SECURE_PROXY`, `TISCI`, `SOC_AM62PX_A53_FEED_INHERITED_WDT`, `YAKOGL_AM62P_WDT_SECONDS`, `SAFEUI_SN65DSI8X_SELF_START_INIT_PRIORITY`, `PVR_SAMPLE_HOLD_SECONDS`, `PVR_SAMPLE_REBOOT`) are in `boards/verdin_am62p_am62p54_a53.conf` |
| `main.c` | `pvr_sample_show_window()` / `pvr_sample_finish()` / `pvr_sample_window_flags()`; the display part of those (`show_on_dss`, `am62p_dss_*`) is compiled only under `CONFIG_PVR_SAMPLE_SHOW_ON_DSS`, which `depends on SAFEUI_DSS_AM62P` (`driver/zephyr/lib/board/am62p/Kconfig:80-83`) |
| `CMakeLists.txt` | `FATAL_ERROR` unless `FVG_NOSTD_DIR` (or `$FVG_NOSTD_DIR`) holds `fvg_demo/Cargo.toml`; `cargo build --release -Z build-std=core,alloc --target aarch64-unknown-none` |
| `driver/docs/one-method.md:24` | the sample table: femtovg row, `gl_femtovg`, GC7000 column **none**, V3D `v3d_gl_femtovg` |
| `driver/docs/one-method.md:481-483` | Step 4 (not done): "one sample from `gl_femtovg` plus `v3d_gl_femtovg`'s program check. The i.MX8MP gets femtovg for the first time" |
| how the offline images are built | `tools/cts/common-images.sh` knows the rows `glmark2`, `slint`, `slint-view`, `cluster`, `gl_test`. There is **no femtovg row**, and `tools/cts/common-tree.txt` has none. The farm's `build-main-offline.sh` builds `GT:gc_gl_test` and `SL:slint` only |

How an offline-table image of the i.MX8MP is built (the recipe of the two images that exist, from the
farm's `build-main-offline.sh` and `common-images.sh`): `west build -p always -b
verdin_imx8mp/mimx8ml8/a53 -d <dir> <repo>/driver/samples/zephyr/<sample> -- -DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh $ZEPHYR_WS)
-DYAKOGL_DIR=<repo> -DCONFIG_YAKOGL_SHADERC_ONLINE=n -DCONFIG_YAKOGL_SHADERC_OFFLINE=y
-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`, plus `-DSLINT_NOSTD_DIR` and `-DNOSTD_PATCH_DIR` for `slint`.
Those are the "Vivante / offline-table settings".

## 2. The build attempt

Command (the recipe above, with the sample swapped; no `FVG_NOSTD_DIR`, as the other two images are
built). It is kept on the farm as `$CCFARM_DEPS_DIR/yakogl/build-femtovg-probe.sh`:

```
export PATH=$WEST_VENV/bin:$PATH ZEPHYR_BASE=$ZEPHYR_WS/zephyr
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
cd "$ZEPHYR_WS" && west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d <out> \
  <repo>/driver/samples/zephyr/gl_femtovg -- \
  -DZEPHYR_MODULES="$MODS" -DYAKOGL_DIR=<repo> \
  -DCONFIG_YAKOGL_SHADERC_ONLINE=n -DCONFIG_YAKOGL_SHADERC_OFFLINE=y \
  -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
```

Environment the paths of the earlier i.MX8MP build log (`imx8mp-out/main-502032bf-offline/GT.log`)
show: `ZEPHYR_WS=/home/tasuku/org/zephyrproject/zephyrproject` (Zephyr 4.4.99),
`WEST_VENV=/home/tasuku/org/zephyrproject/.venv`,
`ZEPHYR_SDK_INSTALL_DIR=/home/tasuku/org/zephyrproject/zephyr-sdk-1.0.1`.

**Result: observed, 2026-10-12, exit status 1, no image.** Run on the farm node that has the
Zephyr workspace (Zephyr 4.4.99, SDK 1.0.1, GCC 14.3.0), with the script above and
`<repo>` = this tree (`a9c0191`), out dir under `/tmp`. The first run's last lines, verbatim and
trimmed (the Kconfig warnings come first and do not stop the configure):

```
warning: SAFEUI_GPU_PVR (defined at <repo>/driver/zephyr/Kconfig:5) was assigned the value 'y' but
got the value 'n'. Check these unsatisfied dependencies: DT_HAS_IMG_IMG_ROGUE_ENABLED (=n).

warning: GC_BOARD_WDOG_SELF_ARM (defined at <repo>/driver/zephyr/lib/board/imx8mp/Kconfig:188) was
assigned the value 'y' but got the value 'n'. Check these unsatisfied dependencies:
(GC_SAMPLE_WDOG_BASE = 0x30280000) (=n), YAKOGL_GC_BOARD (=n).
...
-- YakoGL: GPU=powervr shaderc=offline no-table=OFF test-knobs=OFF linkage=staticlib rust-target=aarch64-unknown-none cpu='cortex-a53'
CMake Error at CMakeLists.txt:31 (message):
  gl_femtovg: FVG_NOSTD_DIR is not set.  Point it at the no_std FemtoVG
  workspace (fvg_demo + patched femtovg/glow): -DFVG_NOSTD_DIR=<path> or
  export FVG_NOSTD_DIR=<path>.

-- Configuring incomplete, errors occurred!
rc=1
```

**The first real error is `FVG_NOSTD_DIR is not set`** (`CMakeLists.txt:31`), exit status 1. The
`SAFEUI_GPU_PVR` assignment is only a Kconfig *warning* here (the earlier inference that it aborts
the configure was wrong), and the configure also shows `GPU=powervr`: the image would use the PowerVR
backend, as `prj.conf` leaves the `YAKOGL_GPU` choice at its default. `GC_BOARD_WDOG_SELF_ARM=y` from
the recipe is dropped for the same reason (`YAKOGL_GC_BOARD=n`).

**Second run, to see the next error**: the same command plus
`-DFVG_NOSTD_DIR=<NOSTD_PATCH_DIR>` (the farm's no_std workspace). Configure passes; the build
reaches 129 of 175 steps and stops, exit status 1, no image:

```
FAILED: [code=1] modules/yakogl/CMakeFiles/yakogl.dir/<repo>/src/osal/zephyr.c.obj
In file included from <repo>/src/osal/zephyr.c:43:
<repo>/driver/src/pvr_drv.h:26:10: fatal error: pvr_ddk.h: No such file or directory
   26 | #include "pvr_ddk.h"
compilation terminated.
ninja: build stopped: subcommand failed.
rc=1
```

That is the PowerVR backend (`-DYAKOGL_GPU_POWERVR`) asking for the PowerVR DDK header on a board
that has none. So without the board configuration of section 3 the sample does not build for the
GC7000 even with `FVG_NOSTD_DIR` given.

## 3. What it would take

| Blocker | Missing? | Size [inferred] |
|---|---|---|
| **Board config** | Yes. `boards/verdin_imx8mp_mimx8ml8_a53.conf` and a 4-line `.overlay` (copy of `slint`'s, which sets `SAFEUI_GPU_VIVANTE`, `YAKOGL_GPU_VIVANTE`, `FPU_SHARING=n`, `UART_CONSOLE=n`, `SAFEUI_DISPLAY_IMX8MP`, `PM_CPU_OPS_PSCI`), and `SAFEUI_GPU_PVR=y` moves from `prj.conf` (22 lines) into the AM62P board file; `YAKOGL_GPU_VIVANTE=y` comes with the i.MX8MP one. Its `Kconfig` sources `../common/Kconfig`; the i.MX8MP samples source `../gc_common/Kconfig`. | about 30 lines of config, no logic |
| **Vivante backend code** | **No.** `YAKOGL_GPU_VIVANTE` is the backend `gc_gl_test` and `slint` already run on this board, and femtovg's calls are the same GL ES 2.0 calls (Slint's renderer is femtovg). | 0 |
| **The sample's board glue** | Yes. `main.c` calls the AM62P-only `pvr_sample_*` glue. On the i.MX8MP `show_on_dss` compiles to a no-op returning 0, so a built image would draw into a buffer and show nothing, print through `LOG_INF` (no UART on the Mallow) and write **no SNVS record**, so no verdict could reach `tools/board.sh imx8mp`. It needs the board layer API of `slint` (`board_window`, `board_show`, `board_result`, `board_progress`, `yakogl_board.h:141-221`) and a pass/fail from its four read-back points. | `main.c` rewritten, about 100 lines (`slint`'s `main.c` is 299 lines for a larger job) |
| **Shaders missing from the table** | **No.** See the list below: 36 of 36 on main and on PR #129. | 0 rows |
| **femtovg's no_std deps** | Not missing. `FVG_NOSTD_DIR` wants `fvg_demo/` + patched `femtovg/` + `glow/`. The farm's `NOSTD_PATCH_DIR` (`$CCFARM_DEPS_DIR/yakogl/NOSTD_PATCH_DIR`) holds exactly `Cargo.toml`, `femtovg/`, `fvg_demo/`, `glow/`, and `fvg_demo` depends on `../femtovg` and `../glow`. So `-DFVG_NOSTD_DIR=$NOSTD_PATCH_DIR` is the value (observed: the configure passes with it, section 2). The staticlib crate is target-generic (`aarch64-unknown-none`, `aligned_alloc` from Zephyr's libc heap). | an option on the command line, 0 lines |
| **Build recipe** | Yes. A `femtovg` row in `common-images.sh` / `common-tree.txt` (a `-DFVG_NOSTD_DIR` argument), and the farm's `build-main-offline.sh` row. | about 10 lines |
| **A run** | Yes. A host check that the image's SNVS record carries a verdict; `board.sh imx8mp` reads the record, not a console. | with the glue above |

Total: a day of porting, almost all of it in `main.c` and the config split. Step 4 of
`one-method.md` already plans it (one `femtovg` sample for the three boards, replacing `gl_femtovg`
and `v3d_gl_femtovg`), and is the right place, not this PR.

### Which femtovg shaders would need table rows: none

From `tools/cts/log/investigations/gc7000-table-pr129.md` (branch `ccfarm/01a12f0e-27b3-7647-90ad-0944dc6633d1`,
`2e34771`) and re-counted here [read]: `tests/reference/vivante_blobs.rs` holds **36 `femtovg/` rows** (`grep -o
'name: "femtovg/[^"]*"' | sort -u | wc -l` = 36). They are the files of `tools/shaders/femtovg/`, the
texts femtovg 0.26.0 plus the no_std patch passes to `glShaderSource` (`PROVENANCE.md`):

- `main.vert` (1);
- `fill_color`, `fill_gradient`, `fill_gradient_conic`, `fill_image`, `fill_image_gradient`,
  `fill_image_gradient_conic`, each as `.frag`, `.glyph.frag`, `.glyph.nodiscard.frag`,
  `.nodiscard.frag`, `.nodiscard.noscissor.frag` (30);
- `filter_image.frag`, `filter_image.nodiscard.frag`, `stencil.frag`, `stencil.nodiscard.frag`,
  `texture_copy_unclipped.frag` (5).

The six hand-written `fvgtest_{green,ftcoord,inner,cover,discard,discard2}.frag` probes of the
`gl_femtovg` demo also have rows. PR #129 adds no `femtovg/` row (its 218 rows are `es100/` 97, `qt/` 85,
`es300/` 18, `qt3d/` 16, `armselect` 1, `gl_test_es100/` 1), and none is looked up by femtovg. So a
`gl_femtovg` image would see the same table result on main and on the PR, and the A/B would be
vacuous for it. The `slint` sample's A/B carries the same 36 texts.

## 4. What this does to the record

- `gl_femtovg` is its own outcome: **nothing ran**. It is not "passed" and not "failed", and it does
  not count in the A/B.
- The femtovg texts are still covered on the board by the `slint` image (same fork, same 36 rows),
  which the session runs on both arms.
- Nothing here needs a code change inside PR #129, and nothing here argues against committing its
  table: the question the board run answers for femtovg is already answered by the table lookup
  (36 of 36 on both sides) and by what `slint` does.

## Commits this rests on

| Ref | Commit |
|---|---|
| integration tip, the tree read | `db3545fa25e2` check.sh: a failed row keeps its whole output |
| `origin/main` side of the A/B (images of `main-images-head`) | `502032bf` |
| PR #129 head, as measured by the earlier analysis | `419b0159d2ea` |
| the earlier analysis, read for the table counts | `2e34771` (`gc7000-table-pr129.md`) |
