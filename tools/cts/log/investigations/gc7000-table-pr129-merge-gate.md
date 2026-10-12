# GC7000 table, PR #129 + main 2fe4b879: host gate on the re-created merge

One line for the record and the PR comment: `tools/check.sh --jobs 4` on the merge of
419b0159 (PR #129) and 2fe4b879 (main) ran 147 rows: 140 ok, 7 failed, 0 gap; every gpu-vivante clippy and
test row (including the GC7000 round trip under shaderc-online) is ok, no row reproduces
tests/reference/vivante_blobs.rs (check.sh has none), and the 7 red rows are an
`only metadata stub found for rlib` rustc error (4), a missing `fw/pvr_fw_blob.h` (2, PVR_FW_BLOB
not exported to this run) and 2 unnamed failed checks of board_selftest.sh (1); none of them was
run on main, so none is attributed.

## The commit

| | id |
|---|---|
| PR #129 head (origin/work/int-pr31) | `419b0159d2eab3e1b0ed7e8e376441beea4fdc6e`, tree `f9bd713431fb35251f1b5120f20a7c6e25eaa417` |
| main | `2fe4b8798b5faf2807f31dd486296f21045f4abf` |
| the merge gated (re-created here) | `62b4404a9c23ef32bf24a7d2de49119cd1a6231c`, tree `f5f0324a75ffe4b949f01c8e20758ddb039e02f2` |
| board-merge's merge | `63940012a0aea051d3e110f3dd837d3e1a9bccb3`: **not obtainable** |

How the commit was obtained. 63940012 is not on origin and not in this clone;
`git fetch origin ccfarm/01a12f0d-cec7-706f-9402-20bc63f69e6c` found no such ref, and
`fetch_from_farm` of that agent's workspace answered "no online node exports" the path. So the merge was
re-created: `git fetch origin`, `git checkout --detach origin/work/int-pr31`,
`git merge --no-ff 2fe4b879`. It merged without conflicts (auto-merging docs/HANDOFF.md and
src/shaderc/online/select.rs). `git merge-tree --write-tree origin/work/int-pr31 2fe4b879`
gave the same tree, f5f0324a.

Equal to 63940012's tree? **Unknown**: 63940012's tree was not available to compare. By the item's
rule, a re-created merge counts as the same commit under test only if its tree equals
63940012's, or if that tree is unavailable and the merge had no conflicts; the second holds, so
this is taken as the commit under test, with the caveat that it is an assumption. A merge of the same
two parents has no other content, so a different tree could only come from a
conflict resolution by hand or a different merge strategy.

Nothing was pushed. The merge commit 62b4404a lives only in this workspace's object store.

## Commands

```
tools/check.sh --jobs 4                  # full run on 62b4404a, 3864 s
tools/check.sh --changed HEAD~1          # re-run of the failed rows, see below, 1450 s
```

The environment of both runs was the shell's own: `PCO_SHC`, `GLSLANG`, `GLSL2USC_CACHE`,
`GLMARK2_SRC` could not be exported to the run (the tool refused `env VAR=... tools/check.sh`), and
`tools/cts/deps-env.sh --check` says all nine entries are present in the farm's deps directory. The
rows that need PCO_SHC ran (`gen tests/reference/blobs.rs ... (pco_shc)` is ok, not a gap), so
PCO_SHC reached the run; PVR_FW_BLOB evidently did not (see the two am62p rows). check.sh printed
`0 gap(s)`.

The second run used a probe commit on top of the merge (2a309cc3: two inert text files,
tools/precompile/GATE_PROBE.txt and tools/cts/GATE_PROBE.txt) so that `--changed HEAD~1` selects
only the precompile and tools scopes; the probe is not part of the merge and not in this
branch.

## Result of the full run (62b4404a)

```
check.sh: 140 ok, 7 failed, 0 gap(s) (a gap was not run: it is not a pass), 0 skipped (every row), 0 not run (passed on main) in 3864s; test binaries: 594 run, 0 skipped (passed on main)
```

All 140 ok rows (`ok` as check.sh printed them; times left out here):

- `cargo fmt --check`; `build yakogl-capi (no_std)`
- clippy and test, all of: `host-test,gpu-powervr`; `host-test,gpu-powervr,report-es2`;
  **`host-test,gpu-vivante`**; `host-test,gpu-powervr,shaderc-online`;
  **`host-test,gpu-vivante,shaderc-online`** (the GC7000 USC round trip and online_reflection against
  the offline table); `host-test,gpu-powervr,shaderc-online,report-es2`;
  **`host-test,gpu-vivante,shaderc-online,report-es2`**;
  `host-test,gpu-powervr,shaderc-online,precompiled-shaders`;
  **`host-test,gpu-vivante,shaderc-online,precompiled-shaders`**; the two `no-shader-table (no compiler)` pairs
- `gen precompiled tables for the tier configurations`
- `clippy host-test,gpu-vivante,shaderc-online,test-knobs`, its `--lib` (9 knob tests) and
  `tests/gc_kick_diff.rs` (7 tests)
- the v3d rows, the pvr and osal checks, the Mesa reference rows, `cc gl_test.c` (three arms), `gl_test set
  walker count`, `gl_test case 44 probe decode`, `gen gl_test_shaders{,_es100}.h`, the aarch64 `check`
  arms, the tools self-tests, `tools shader-table generator guards`, `tools precompiled.list gl_test pairs ==
  what gl_test.c links`, the artefact checks, `clippy`/`test yakogl-tools`, `yakogl-precompile`
  clippy and tests for both GPUs, `gen src/vivante/internal.rs == yakogl-precompile --internal`,
  `gen tests/reference/blobs.rs == tools/glsl2usc.sh on precompiled.list (pco_shc) [206s]`,
  `test glmark2 corpus ...`, the yakogl-drivers, slint/cluster/triangle checks,
  `zephyr imx8mp gc_gl_test image, -Werror on our C files [53s]`,
  `zephyr rpi4 v3d_gl_test image, -Werror on our C files [46s]`.

The 7 failed rows, as check.sh printed them:

```
FAIL  tools   board.sh dry-run matrix and replays [37s]
      board_selftest: 227 checks, 2 failed (pinned: 227)      (every PASS line shown; the two failed ones are not named, build/check-fail/1.log is empty)
FAIL  zephyr  am62p gl_test image, -Werror on our C files [32s]
      driver/src/pvr_fw_boot.c:304:10: fatal error: fw/pvr_fw_blob.h: No such file or directory
FAIL  zephyr  am62p gl_test image without a compiler (precompiled shaders only), -Werror [87s]
      driver/src/pvr_fw_boot.c:304:10: fatal error: fw/pvr_fw_blob.h: No such file or directory
FAIL  gen     precompiled table gpu-powervr: deterministic, rows == fresh compiles, counts pinned, compiles [98s]
FAIL  gen     precompiled table gpu-vivante: deterministic, rows == fresh compiles, counts pinned, compiles [57s]
FAIL  gen     precompiled table gpu-powervr,report-es2: deterministic, rows == fresh compiles, counts pinned, compiles [68s]
FAIL  gen     precompiled table gpu-vivante,report-es2: deterministic, rows == fresh compiles, counts pinned, compiles [10s]
      each: error: only metadata stub found for `rlib` dependency `yakogl` please provide path to the corresponding .rmeta file with full metadata
```

The four precompiled-table rows are red only in the last step of prove.sh (`rustc ... --emit=metadata
--extern yakogl=<rlib>`). Before it, their output is: GC7000 `made twice, the same 7335933 bytes`;
`each of its 595 rows is what the compiler makes for that row alone`; `gc7000 es2=0 app=all: 595 rows
... 741968 code bytes`, and the pin `rows=595 code_bytes=741968` met; es2=1: 365 rows, 245312 code
bytes, pin met; PowerVR 3433 / 4096960 and 2652 / 2758128, pin met. The Vivante ones
add `unresolved imports super::ShcAttrib ...` in the check file, a consequence of the stub rlib.
PowerVR fails the same way, and the PowerVR table is not a Vivante change, so this reads
as the toolchain's rlib (a metadata-only `yakogl` rlib that cargo named), not as the table. That is a
reading, not a measurement: the same rows were not run on main.

## Re-run of the failed rows

`tools/check.sh --changed HEAD~1` on 62b4404a + the probe: `47 ok, 7 failed, 0 gap(s), 93 skipped`
in 1450 s. The same seven rows failed with the same messages (the board.sh row again `227 checks, 2
failed`). So they are not a one-off of the first run's order or load. The
`gen tests/reference/blobs.rs ... (pco_shc)` row, `test host-test,gpu-vivante,shaderc-online,precompiled-shaders`
and the rest of the scopes were ok again.

## What the gate does not cover

- **No row reproduces tests/reference/vivante_blobs.rs.** check.sh's only reference-table row is
  `gen tests/reference/blobs.rs == tools/glsl2usc.sh on precompiled.list (pco_shc)`, the PowerVR table. The GC7000
  table is generated by tools/vivante (etna_shc), which has no gate row and no env name in
  deps-env.sh; the committed table is exercised by the `gpu-vivante` rows above (round trip,
  reflection, the tier configuration) and checked by `tools shader-table generator guards`
  (`gen_shader_table.py --self-test`), not regenerated. The byte-for-byte reproduction of the 218 rows with
  this farm's etna_shc is the table-repro item's.
- Not a board result: nothing here says the three images behave on the i.MX8MP.

## Not established

- 63940012's tree vs f5f0324a (63940012 unavailable).
- Whether the 7 red rows are red on main 2fe4b879 too (main was not run). The two am62p rows read as
  the unexported PVR_FW_BLOB; the board.sh row's two failed checks were not identified; the four
  precompiled-table rows are red at a step that does not touch the table's contents.
