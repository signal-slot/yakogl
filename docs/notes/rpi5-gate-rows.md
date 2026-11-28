<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5: the gate's rows (P12 plan)

Plan #384, P12 of `docs/notes/pre-study-rpi5.md` section 8: the Pi 5's rows in
`tools/check.sh` at **0 gaps**. This note lists the rows that exist on the tip
(24995f2e), the rows P9 to P13 still need, and the order in which the open
items add them. All `tools/check.sh:N` citations are lines of that file at
2c4e802f (re-read for plan #404; the other facts are those of 24995f2e); `docs/notes/v3d71-gl-table.md` (cited as `gl-table`) holds the build
selection. Nothing here edits a script: the rows are added by the items named in
section 4, each in the commit that adds what the row runs
(`docs/notes/v3d71-gl-table.md:150`: "one row per feature").

## 1. How a row is selected, and what a gap is

* **Gap.** `gap()` (`check.sh:355-364`) prints the row, counts it and makes the
  gate red. A skipped step is never a pass. Three ways a Pi 5 row gaps:
  1. a missing environment value (`ZEPHYR_WS`, `MESA_V3D_TREE`, `V3D_SHC`,
     `ZEPHYR_SDK_INSTALL_DIR`, `dtc`, a C compiler);
  2. a `needs <file> <how>` row (`check.sh:664`) whose file is absent;
  3. exit 77 of `tools/v3d/check71.sh` (`check.sh:3161-3167`): any script of
     `check71.d` that prints `gap: <names>` and exits 77 makes the **one**
     v3d71 row a gap (`tools/v3d/check71.sh:42-48`), and a failure hides a
     gap (`:36-41`).
* **`--changed <base>`** (the pull request's gate, `gate.yml` header, `CLAUDE.md`)
  maps each path to scopes with `scope_of_path` (`check.sh:185-190`, its map `path_scopes` at `:210-262`); a row runs
  when one of its `scope` words (`check.sh:455`, `wanted` at `:473-492`) is
  reached. A path that no pattern names is `all` (`:260`). Nothing runs
  `--changed` for the nightly: it runs every row, clippy included.
* **Feature sets and scopes.** `config_scopes` (`check.sh:412-417`) maps any
  feature set containing `gpu-v3d` to scope `v3d`. `*gpu-v3d*` also matches
  `gpu-v3d71`, so a `gpu-v3d71` feature-set row needs no change there.
* **Environment.** `tools/setup-gate-host.sh` installs packages and `--check`s
  them; it does not install the workspaces (`setup-gate-host.sh:45-47`:
  `ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC`, `MESA_PAN_TREE`, `MESA_V3D_TREE` are
  "workspaces, not packages"; `tools/cts/deps-env.sh --layout` places them).
  `dtc` is checked at `setup-gate-host.sh:319-327`. `gate.yml` runs the gate on a
  self-hosted runner whose layout comes from `deps-env.sh`; no Pi 5 value is
  special there.
* **No board in the gate.** Nothing in `check.sh` reaches a board: the Pi 5
  board sessions (`tools/board.sh pi5 run`, `tools/cts/board-stage.sh`) run by
  hand, and the gate covers what they use: their images build, their verdict
  parsers and replays pass (`board_selftest`, `cts-selftest`). So "board-stage
  rows" below are **host rows of the board tooling**, never a row that waits for
  a board, and none can gap for lack of one.

## 2. Pi 5 rows that exist on the tip

| # | Row (label, `check.sh` line) | Runs | Env, and why it can gap | `--changed` reach |
|---|---|---|---|---|
| E1 | `run     v3d71 host checks (tools/v3d/check71.sh)`, `:3170-3172` | every `tools/v3d/check71.d/*.sh` (59 scripts on the tip), one line each; `check71.sh` is **bash-only** (`sh` is dash on the farm and exits 2) | **Yes, one cause measured.** A script exits 77 for a missing input: no C compiler; a `MESA_V3D_TREE` that has no `build-v3d` (`gltest_mesa71.sh:55-57`); a `V3D_SHC` that is not an executable (`:63-65`); no aarch64 C compiler (`xbuild71.sh:137`, named `ZEPHYR_SDK_INSTALL_DIR`). One 77 makes the whole row a gap; a failure outranks it. **Measured 2026-11-24** on a stock farm machine, `MESA_V3D_TREE` and `ZEPHYR_WS` unset, `bash tools/v3d/check71.sh` (or run directly): exit 77 after about 55 s, `check71: 58 ok, 0 failed, 1 gap(s)`; the only gap is `xbuild71`. `gltest_mesa71` and `gltest_mesa71_tex` pass on the vendored reference build, so neither `MESA_V3D_TREE` nor `V3D_SHC` is needed for 0 gaps (unset is right; a tree without `build-v3d` is the gap). `check71_selftest.sh` passes (5 of 5); `common/tri_check` passes (47 checks) and runs in the gate as `check71.d/tri_check.sh` | scope `v3d71`: `src/shaderc/online/v3d/*`, `driver/src/v3d/*`, `tools/v3d/*` (`:229`). **Not** reached by `src/v3d/*` (`:233`, scope `v3d` only), nor by `driver/zephyr/*` or `src/osal/*` (the scripts that read `src/osal/zephyr_v3d71.c` or `driver/zephyr/lib/board/rpi5` then run only when `tools/v3d/*` or an `all` path moves) |
| E2 | `tests/v3d71_*.rs` (48 files), in the row `test    $V3D_CONFIG (every tests/ target)`, `:1903-1905` | `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --tests` (`V3D_CONFIG`, `:1859`); the 7.1 tests carry that set in their headers (e.g. `tests/v3d71_compose.rs`, `v3d71_texfill.rs`) | No. Cargo only. Four tests read `V3D_SHC` or `MESA_V3D_TREE` and then skip *inside* the test (`v3d71_verify_corpus.rs`, `v3d71_mesa_corpus.rs`, `v3d71_mesa_shapes.rs`, `v3d71_shapes.rs`): the cargo row stays green, the same input is checked by E1 | `tests/v3d71_*` matches `tests/*` (`:258`) → scope `tests`, narrowed by `check_tests_reach.py` to the targets a path reaches (`:431-444`); `src/v3d/*` → scope `v3d` (the whole row). Files a test *reads* are in `TEST_TREE_PATHS` (`:1111-1138`, the 7.1 entries `:1133-1137`: `tools/v3d/pi5`, `tools/v3d/gltest-shaders71`, `driver/docs/rpi5-v3d71-facts.md`, `docs/notes/pre-study-rpi5.md`, `src/v3d/zskeep71.rs`, `backend.rs`, `ffi.rs`, `select.rs`). A new tree a test reads **must be added there** (the tree key row, `tools/check_tree_key_scan.py`, fails on a path outside it) |
| E3 | `clippy  $V3D_CONFIG` and `test    $V3D_CONFIG (--lib ...)`, `:1895-1897` | clippy and the lib tests, which include the unit tests of `src/v3d/*71.rs` once those modules are in `src/v3d/mod.rs` (today only `#[path]`-included by `tests/`) | No. Clippy is left to the nightly in a PR gate (`CLAUDE.md`); `ci71fix` (`eq_op` in `tests/v3d71_compose.rs`) exists because this row fails there | `v3d` |
| E4 | `zephyr  rpi_5 probe images (tools/probes/rpi_5/*)`, `:5044-5046` via `zephyr_rpi5_probe_images` (`:4579-4589`) | one `zephyr_werror_image rpi_5 <probe> rpi_5-<name>` (`:4533-4571`: `west build -p always -b rpi_5 ... -DYAKOGL_WERROR=ON`) for **every** directory under `tools/probes/rpi_5/` with a `CMakeLists.txt` and a `prj.conf`: today `rpi5_clear`, `rpi5_dispregs`, `rpi5_dtb`, `rpi5_hello`, `rpi5_ident`, `rpi5_mbox`, `rpi5_scanout16`, `rpi5_scanout32`, `rpi5_tex`, `rpi5_tfu`, `rpi5_triangle`, `rpi5_triangle_ours`, `rpi5_wdog` (13 on the tip; `common/` has neither file) | **Yes.** The whole `if` is guarded by `ZEPHYR_WS` with a `west` workspace (`:4917-4918`); else one gap `zephyr images: no west workspace at ZEPHYR_WS` (`:5054`). **Not measured on 2026-11-24** (it needs `ZEPHYR_WS`, which was unset); with it unset the row is that one gap. The probes' host selftests pass (same date): dispregs 37, mbox 70, scanout16 frame16 70 / hold_check 62 / verdicts, scanout32 io_filter 51 / swap_check 151 / verdicts 147. An image is built one after another in one row, so one probe that does not build fails the row. The comment at `:4575-4578` lists five probes: it is stale, the loop is not | `scope drivers samples` (`:5044`): `tools/probes/*` and `driver/samples/*` → `samples` (`:253`), `driver/*` → `drivers` (`:254`). A probe's own C is in `tools/probes/`; the library it links (`src/osal/zephyr_v3d71.c`, `src/*`) is `all` |
| E5 | `test    rpi_5 fdt_walk host test`, `:5060-5065` | `sh tools/probes/rpi_5/common/fdt/test/run.sh`, ASan/UBSan, needs `dtc` (`fdt_walk_test`, `:4595-4601`) | **Yes**: no `dtc` → `gap` (`:5064`); a `SKIPPED` line is a failure (`:4599-4600`). **Measured 2026-11-24:** with `dtc` on the machine `common/fdt/test/run.sh` passes (`fdt_walk: 528 checks, 0 failed`); `dtc` is the only input this row needs, and it was present | `drivers samples` |
| E6 | `tools   board.sh dry-run matrix and replays`, `:4258` (`tools/board_selftest.sh`: pi5 `run` against `pi5-fake` and the pi5 tryboot sequence, `board_selftest.sh:482-499`; the probe verdict parser, `:46`) | host only, `no_board_env` (`:383-394`) strips `PI5_*`, `BOARD_*`, `ZEPHYR_*` from the environment | No | `tools` (`:259`: `tools/board.sh`, `board_selftest.sh`, `tools/cts/*`) |
| E7 | `tools   cts-selftest.sh (... images-selftest included)`, `:3631`; its images check asserts that `deqp-tree-images.sh rpi_5` is refused with rc 2 (`tools/cts/images-selftest.sh:380-381`) | host only | No. **The line is a pin that P13 must change** (a Pi 5 dEQP arm) | `tools` |
| E8 | `tools/v3d/pi5/*.clif` and the `gltest-shaders71`, `gltest-mesa71` data | no row of their own: read by E1 and E2 | as E1/E2 | `tools/v3d/*` → `v3d v3d71` (`:229`) |

What is **not** in the gate on the tip: any `gpu-v3d71` feature row (the feature
exists since `9f286b5e` — `Cargo.toml:109` — but `check.sh` has no row that
uses it); an `rpi_5` **gl_test** image row (the board files `rpi_5.conf` and
`rpi_5.overlay` exist since 1bb98759 and the image links `-Werror` since
`85afd0cf`, but `check.sh` has no row that builds it); any Pi 5 stage of
`board-stage.sh` (`board-stage.sh:67`, `:175`, `:182`: `pi4` is the only Pi
board). The dEQP image arm for `rpi_5` exists since `e4001734`.

The Pi 4 row to copy is `check.sh:4955-4956`:
`scope v3d gltest drivers samples` /
`step "zephyr  rpi4 gl_test image, -Werror on our C files" zephyr_werror_image rpi_4b gl_test rpi4`.

## 3. Rows P9 to P13 need

Feature sets and commands are as `docs/notes/v3d71-gl-table.md` section 4 decided
(`gpu-v3d71 = ["gpu-v3d"]`, next to `Cargo.toml:96`; Kconfig
`CONFIG_YAKOGL_GPU_V3D71`, which `zephyr/CMakeLists.txt` already reads, and
`src/osal/zephyr_v3d71.c` already builds).

| # | Row (proposed label) | Feature set or command | Env, can it gap, why | `--changed` mapping | Added by |
|---|---|---|---|---|---|
| N1 | `clippy  $V3D71_CONFIG` | `V3D71_CONFIG="host-test,gpu-v3d71,shaderc-online,no-shader-table"`; `cargo clippy --no-default-features --features "$V3D71_CONFIG" --all-targets` next to `:1895-1896` | No (cargo). PR gate leaves it out; the nightly runs it | `config_scopes` gives `v3d` already (`:414`); add `scope v3d local` like `:1895`. **Add `src/v3d/*71*.rs`, `ffi71*` and `tests/v3d71_*` to the `v3d71cfg` reach** only if a separate scope word is wanted; otherwise `v3d` suffices and costs a second v3d build on every Pi 4 change (~the row's time, see `:1895-1897`) | `backend71` (in the commit that adds the feature to `Cargo.toml`) |
| N2 | `test    $V3D71_CONFIG (--lib)` | `cargo_test --libtest --no-fail-fast --no-default-features --features "$V3D71_CONFIG" --lib`, like `v3d_lib_tests` (`:1862-1897`) with its own ignored list file (`build/check/v3d71-ignored.txt`) | No | `v3d`; the unit tests of `src/v3d/*71.rs` and `backend71.rs` | `backend71` |
| N3 | `test    $V3D71_CONFIG (tests/v3d71_backend.rs, ...)` | one `cargo_test --test <name>` per new integration test that needs `gpu-v3d71` (the existing `tests/v3d71_*.rs` need only `gpu-v3d` and stay in E2). Named rows, as `:1919-1989` do, each with `tests_row`/`--test` so `check_tests_reach.py` narrows them | No. Tests that need `V3D_SHC`/`MESA_V3D_TREE` skip inside the test and their input is E1's | tests: `tests` narrowed (`:255-258`); `src/v3d/*` → `v3d` | `backend71`, `cases71`, `mip71` (each adds its own target in its own commit) |
| N4 | `test    $V3D71_CONFIG (every tests/ target)` | `cargo_test --no-fail-fast --no-default-features --features "$V3D71_CONFIG" --tests`, as `:1903-1905`: runs `v3d71_*.rs` on the 7.1 selection too | No | `scope v3d tests` | `backend71` |
| N5 | `clippy  yakogl-capi $V3D71_IMAGE_CONFIG (aarch64-unknown-none, as the rpi_5 gl_test image links it)` and the TARGET_CONFIGS and knob additions | exact text in 3.1 | No (cargo; inside the existing `rustup target` block) | `config_scopes` maps `gpu-v3d71` to `v3d` (`*gpu-v3d*`, `:487`); the new step adds `gltest` | `gltest71` (merged); the rows wait for `check.sh`'s holder |
| N6 | `zephyr  rpi5 gl_test image, -Werror on our C files` | exact text in 3.2 | **Yes**, `ZEPHYR_WS`, the same single gap as E4 | `scope v3d gltest drivers samples` | `gltest71` (merged); waits for `check.sh`'s holder |
| N7 | `zephyr  rpi_5 probe images`: **no new row** | new probes land in E4's loop automatically (`:4579-4589`). `texprobe71`'s `rpi5_tex` and any later probe need a `CMakeLists.txt` and `prj.conf` and nothing else | as E4 | `tools/probes/*` → `samples` | `texprobe71` (and every later probe), no check.sh edit |
| N8 | `tools   board.sh pi5 gl_test verdicts (rpi5_gl_test replay)` | extend `board_selftest.sh` (so row `:4258` carries it): replays of a recorded `gl_test` image's `v3d_gl_test` record end and verdict lines on `pi5-fake` (`boardprep71` did the record-end part; `texverdict71` adds `v3d_tex`/`v3d_cube`). The verdict reader is `tools/cts/pi5-gltest-verdicts.sh` (six lines, a FAIL named by its `draw refused:` line, exit 1 on a missing case, an unnamed FAIL or no record end); until the first board record it is proven on a synthetic console by `pi5-gltest-verdicts-selftest.sh`, a `tools` row of its own (`tools/check_selftests.py`) | No (host replay) | `tools` | `boardprep71` (done), `texverdict71`, `board71` for the six cases' recorded replay |
| N9 | `tools   board-stage.sh pi5 common` self-test | a `board-stage.sh pi5 common` arm (the Pi 5 `layout`/`key` pair, `board-stage.sh:182`, usage `:175`) with its dry-run checked in `tools/cts/cts-selftest.sh` or `tools/board_selftest.sh`, the way `pi4 common` is (`images-selftest.sh:456-465` loop `am62p imx8mp rpi_4b`: add `rpi_5`) | No. The stage needs `PI5_*` values only at run time; the dry run needs none | `tools` | `stage71` |
| N10 | `docs / tools` rows touched by results | `tools/cts/RESULTS.md` Pi 5 row and `STATUS.md` line: the `ctsdoc`-style lints reach `tools` and `docs`; no new row | No | `docs`, `tools` | `board71`, `board71-tex` (their commit message and RESULTS row; not check.sh) |
| N11 | P12 gate: the v3d71 row (E1) at 0 gaps | not a new row: E1 must pass with an aarch64 C compiler on the runner (see section 5) | **Yes**: that compiler only (measured 2026-11-24). `MESA_V3D_TREE` and `V3D_SHC` are **not** needed: unset `MESA_V3D_TREE` means the vendored reference build, and `gltest_mesa71*` pass on it; set to a tree without `build-v3d` it is a gap, so leave it unset or point it at a built tree | `v3d71` | the P12 closer (no item yet; see section 5) |
| N12 | P13: `dEQP-GLES2` and `glmark2` rows for the Pi 5 | image builders for `rpi_5` (`deqp-tree-images.sh rpi_5`, `images-selftest.sh:380-381` and `:456-465`) and a `board-stage.sh pi5 deqp\|ms4` stage | No in the gate (host rows only); real runs are board sessions | `tools` | P13 items (`deqp71plan` designs them) |

### 3.1 and 3.2: N5 and N6 as `tools/check.sh` rows

Read at `2dd734313308cc1781ad2889ac790b71bced9618` (line numbers are of that
file). Text to paste, in the style of the neighbouring rows. Nothing here was
run (docs only).

**What is already true.** `driver/samples/zephyr/gl_test/boards/rpi_5.conf`
sets `CONFIG_YAKOGL_GPU_V3D71=y`, so the image needs **no** `-DCONFIG_...` on
the west line. The gl_test Kconfig selects `YAKOGL_GL_TEST_RUNNER` and
`YAKOGL_TEST_KNOBS` (`gl_test/Kconfig:14` and `:27`), which `zephyr/CMakeLists.txt`
turns into the crate features `gl-test-runner` and `test-knobs`
(`capi/yakogl_capi.cmake`). The image's library is therefore
`yakogl-capi` with `osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table,test-knobs,gl-test-runner`.
The tip has **no** deqp or glmark2 image row for `rpi_4b` either (only the
`rpi4 gl_test` row at `:5501`), so none is proposed for `rpi_5`; N6 is the one
image row.

**3.1 (N5)**, three additions, all in the cargo part:

1. `TARGET_CONFIGS` (`:3915`), a fifth line, so the existing loop at `:3933`
   (`clippy  $f (aarch64-unknown-none, no host-test)`, scope from
   `config_scopes`, therefore `v3d`) covers the 7.1 library:

```bash
  "osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table"
```

2. After the `yakogl-gltest-runner` row (`:3938-3939`), inside the same
   `if rustup target list ...` block:

```bash
  # The library as the rpi_5 gl_test image links it (Kconfig GL_TEST_RUNNER and
  # GL_TEST_KNOBS select these two features): the 7.1 table behind both.
  V3D71_IMAGE_CONFIG="osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table,test-knobs,gl-test-runner"
  scope v3d gltest
  step "clippy  yakogl-capi $V3D71_IMAGE_CONFIG (aarch64-unknown-none, the rpi_5 gl_test image's library)" \
    cargo clippy -p yakogl-capi --no-default-features --features "$V3D71_IMAGE_CONFIG" \
      --target aarch64-unknown-none --lib
```

3. The knob lists. `gpu-v3d71 = ["gpu-v3d"]` (`Cargo.toml:109`), so the 7.1
   archive carries the same `src/v3d` knobs as the 4.2 one: `KNOB_V3D_SYMBOLS`
   and `KNOB_V3D_PRODUCT` (`:4337-4338`) apply unchanged and need no new
   variable. One line joins `check_knobs_absent` (`:4673-4677`), after the
   `v3d` line (`:4676`), so a knob or a `gl_test_*` symbol in the 7.1 product
   archive fails the row:

```bash
  knob_sets v3d71 '^(v3d_debug_|gl_test_)' "$KNOB_V3D_SYMBOLS" "$KNOB_V3D_PRODUCT" || return 1
```

   (`knob_sets` builds `osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table`
   with and without `test-knobs`, `:4652-4659`. If the first run shows a 7.1
   difference from the 4.2 set, the diff the row prints is the expected set to
   paste.) The row's key (`:4350`) already holds both variables.

**3.2 (N6)**, after the Pi 4 row (`:5501`), in the `ZEPHYR_WS` block, before
the Pi 4 examples (`scope drivers samples` rows follow them). The scope line
is already `scope v3d gltest drivers samples` there, so only the step is new:

```bash
    step "zephyr  rpi5 gl_test image, -Werror on our C files" zephyr_werror_image rpi_5 gl_test rpi5
```

It keeps `zephyr_werror_image`'s `yakogl_zephyr_present` check (`:5079-5092`):
the image has `CONFIG_YAKOGL=y` and `CONFIG_YAKOGL_BOARD=y`, so the hook must be
linked. No `needs`: the Pi 5 image has no firmware blob.

**What still waits for `mod71open`** (the recording library): neither row.
Both only build and link, which the tip does (`gltest71` is merged). What waits
is the board verdict (the six cases record nothing until then) and any N3 row
for tests that need `gpu-v3d71`'s recorded draws.


## 4. Order of the open items

1. **`backend71`** (done as `feat71`, `9f286b5e`: `gpu-v3d71 = ["gpu-v3d"]` is
   in `Cargo.toml` (`:109`) and `table71` is the `gpu-v3d71` build's
   `g_backend`; the zephyr 7.1 library build passes the feature to cargo
   (`79c8e4f2`). The N1, N2 and N4 rows themselves are still open, they wait
   for `check.sh`'s holder): adds `gpu-v3d71 = ["gpu-v3d"]` to `Cargo.toml` and
   `V3D71_CONFIG` plus N1, N2, N4 (and N3/N5 for its own tests) in the same
   commit, because a new feature joins `check.sh`'s lists then
   (`CLAUDE.md`, "Common failures"). Its `lib.rs` "exactly one GPU feature" and
   `shaderc-online` rules are in `gl-table:92-93`. It also adds `ffi71`'s
   layout check as a row-less cargo test. The cargo rows have no env; the first
   ones to go red are the 4.2 rows if `src/v3d/mod.rs` changes, so the hook is a
   `mod` line or a cfg only (the plan's rule). No N3 row is needed for the 7.1
   tests merged so far: every `tests/v3d71_*.rs` header names the 4.2
   `V3D_CONFIG` feature set, so E2 runs them.
2. **`gltest71`** (done: the `V3D71` branch and `main_rpi5.c` `dec352ac`, the
   image links `-Werror` `85afd0cf`): N6 (the rpi_5 gl_test image, `-Werror`) and N5. Both need
   `ZEPHYR_WS`. N6 can only be green after `backend71` (the Kconfig help names
   the feature, `gl-table:114-118`) and `werrdry71` (the trial build) listed what
   to fix — both happened (`9f286b5e`, `85afd0cf`); the board `.conf`/`.overlay`
   have merged (1bb98759), so the only thing left for N5 and N6 is `check.sh`
   itself (its holder has the row texts).
3. **`stage71`**: N9 (and N8's `board-stage` side). Reaches `tools` only, so a
   PR of it runs no cargo or Zephyr row: it can land alone.
4. **`board71`** (after 1 to 3; **ran 2026-11-27, `d2dbaa7e`**: all six cases
   FAIL for the one named cause of the product table recording nothing —
   `docs/notes/rpi5-deqp-glmark2.md` section 2 — so it produced no pass to
   record; its replay of the recorded console is in `board_selftest.sh`, the
   row `:4258`'s file, and a re-run waits for a product-form 7.1 table): the
   board session; RESULTS and STATUS rows
   (N10). Adds no row.
5. **`mip71`, `cases71`, `board71-tex`, `texprobe71`, `texverdict71`**: N3 rows
   for their tests; N7 for the probe; N8 for `texverdict71`.
6. **P12 closer** (no item yet): N11, the aarch64 compiler on the runner (and `ZEPHYR_WS` for E4), and a
   check that every Pi 5 row of sections 2 and 3 prints `ok` with `0 gap(s)` in
   one nightly. See section 5.
7. **P13**: N12.

## 5. What 0 gaps still needs (decisions for the planner)

* **Measured 2026-11-24** on a stock farm machine with `MESA_V3D_TREE` and
  `ZEPHYR_WS` unset, per Pi 5 row input:

  | Input | Result | What closes it |
  |---|---|---|
  | `check71.sh` (E1) | gap (exit 77, 58 ok, 0 failed, 1 gap; only `xbuild71`) | an aarch64 C compiler, an environment step on the runner (below) |
  | `gltest_mesa71`, `gltest_mesa71_tex` | pass on the vendored reference build | nothing; keep `MESA_V3D_TREE` unset or pointing at a built tree |
  | `check71_selftest.sh`, `common/tri_check` | pass (5 of 5; 47 checks) | nothing |
  | `common/fdt/test/run.sh` (E5) | pass with `dtc` (528 checks, 0 failed) | nothing on this machine; `dtc` must stay installed |
  | Pi 5 probe host selftests | pass (dispregs 37, mbox 70, scanout16 70 / 62 / verdicts, scanout32 51 / 151 / 147) | nothing |
  | Zephyr `rpi_5` probe images (E4) | not measured; one gap without `ZEPHYR_WS` | an environment step: a west workspace at `ZEPHYR_WS` (`tools/cts/deps-env.sh --layout`) |
  | `setup-gate-host.sh --check` | reports `aarch64-cc`, `GLMARK2_SRC` and `pvr_fw_blob.h` missing | only `aarch64-cc` and `dtc` matter for the Pi 5 rows; `GLMARK2_SRC` and `pvr_fw_blob.h` belong to other boards and P13 |

* **The aarch64 compiler.** `xbuild71.sh` (`:46-53`, `:114-138`) takes
  `aarch64-zephyr-elf-gcc` from `ZEPHYR_SDK_INSTALL_DIR`, else
  `clang --target=aarch64-none-elf` if it finds a C library for the target,
  else `aarch64-linux-gnu-gcc`. It must accept `-mcpu=cortex-a72` and
  `-mcpu=cortex-a76`. Stock clang without a target C library is not enough.
  Installing `gcc-aarch64-linux-gnu` or the Zephyr SDK on the runner closes the
  one measured E1 gap: an environment step, no repository change.
* **`--check` lines.** `setup-gate-host.sh --check` does not name the aarch64
  compiler for `xbuild71`, `dtc` for E5, `V3D_SHC` or `ZEPHYR_SDK_INSTALL_DIR`
  as Pi 5 inputs. Item `p12-hostcheck` adds the lines (the compiler and `dtc`
  matter; `V3D_SHC` is optional, see above). It is a script edit, not part of
  this note.
* **`check71.sh` is bash-only.** `sh tools/v3d/check71.sh` exits 2 under dash.
  Run it directly or with `bash`; a doc or script that says `sh check71.sh` is
  wrong. A run takes about 55 s, mostly the Mesa-based scripts.
* **One row, many gaps.** E1 collapses every check71 gap into one row, and its
  exit code is 77 for any one of them. To read which, use the printed `gap:`
  names (`check71.sh:42-48`). A row per input would be clearer but changes the
  row count the nightly's `seen` step compares; leave E1 as is.
* **Scope reach.** `src/v3d/*71.rs` reaches `v3d` but not `v3d71`; a change
  there does not re-run E1, whose scripts read `src/osal/zephyr_v3d71.c` and the C
  builders, not `src/v3d` (`ffi71_layout_check.c` is under `src/v3d/`: its check
  is run from `tests/v3d71_ffi.rs`, E2). If a check71.d script starts reading
  `src/v3d/*71*`, add `src/v3d/*71*` to `:229` in that commit.
* **Stale comment.** `check.sh:4575-4578` lists five probes; the loop takes
  thirteen. A comment-only fix for whoever next touches `:4579`.
* **`-Werror` of N6.** Not yet established on the tip for gl_test (`werrdry71`
  trial listed what to fix; the probes are the E4 evidence, not measured on
  2026-11-24).
* **PR gate versus nightly.** On a PR only the rows the diff reaches run, so a
  Pi 5 change that reaches only `scope v3d71` leaves N6 unrun and vice versa; the
  nightly runs all. A PR that adds N6 reaches `gltest drivers samples v3d`
  itself, so its first run shows the row.
