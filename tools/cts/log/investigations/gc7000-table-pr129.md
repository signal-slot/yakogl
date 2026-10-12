<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# The GC7000 table of PR #129: the row diff and the table misses each board app should see

Written 2026-10-12 on the host, before any board run. No board was used. Each finding is marked
**[measured]** (a program read the git objects and printed the number) or **[inferred]** (read from
code or text, not run).

## Commits

| Ref | Commit |
|---|---|
| `origin/main` (fetched 2026-10-12) | `2fe4b8798b5f` Merge pull request #141 from signal-slot/work/issue-127 |
| `origin/work/int-pr31`, PR #129 head | `419b0159d2ea` GC7000 table rows (#22): land the integration result on main |
| PR commits since the merge base | `7898ab09` 218 rows appended, the stale texsize row replaced; `d6951f1c` review fixes (pins, `gl_PointCoord` names, constant branch condition); `419b0159` |
| merge base of the two | `136659c2` Merge pull request #100 (work/ci-step6), 2026-10-12 03:38 +09:00 |

Both refs were read with `git show <ref>:<path>`; nothing was checked out (see "Host tests").

`tests/reference/vivante_blobs.rs` was **not touched on main** after `136659c2` [measured:
`git log 136659c..origin/main -- tests/reference/vivante_blobs.rs` is empty, and the file is
64,261 lines on both]. So the table of a main + PR merge is the PR's table byte for byte, and the
A/B on the board differs by this table only, for the offline-table apps.

PR #129 changes code outside the table in three places only [measured, `git diff --stat 136659c
origin/work/int-pr31 -- src driver zephyr Cargo.toml include`]: `src/shaderc/es100_twins.rs` (a pin),
`src/shaderc/online/vivante/roundtrip.rs` (pins) and `src/shaderc/online/select.rs` (a constant `if`
condition reads `.xxxx`). The last one is the in-process compiler. A `mesa-reference-table` image without
`shaderc-online` does not run it [inferred from `zephyr/Kconfig`: the choice
`YAKOGL_SHADERC_OFFLINE` / `_ONLINE`]. The PR also carries tests, the generator
(`tools/vivante/`) and the record `tools/cts/log/investigations/gc7000-table-rows.md`.
(A plain `git diff origin/main origin/work/int-pr31` shows 84 files, because main moved on; use
`136659c..` for what the PR itself changes.)

## 1. The row diff

Method [measured]: both files were parsed into rows (`ShcBlob { ... }`), keyed by (name, blend_key),
and each row's text compared with its `CODE_n` / `CC_n` / `CD_n` / `UNI_n` arrays inlined, so a row
that only moved to another array number does not count as changed.

| | main | PR #129 |
|---|---|---|
| Rows (`SHC_BLOB_TABLE`) | 330 | 548 |
| Distinct lookup keys (blend_key 0, stage + sha256) | 325 | 543 |

- **Added: 218 rows. Removed: 0. Changed: 1.** (178 fragment, 40 vertex; none with a blend key, so no
  new `@front_cw` twin.)
- The one changed row is `es300/texsize.frag`: its `sha256` key moved
  (`dbda06a6...` to `c2f6426a...`) because the source text had gained a precision line, and the
  stored source text moved with it. Its instruction words, wiring and every other field are
  identical. This is why one key leaves and one enters: 325 + 219 - 1 = 543.
- Where the 218 rows come from (source directory under `tools/shaders/`):

| Directory | Added rows |
|---|---|
| `es100/` | 97 |
| `qt/` | 85 |
| `es300/` | 18 |
| `qt3d/` | 16 |
| root (`armselect.frag`) | 1 |
| `gl_test_es100/` | 1 |

- Who needs them [measured]: of the 219 new keys, **61 are texts of `gl_test_shaders.h`** (the default
  set; 45 `es100/`, 15 `es300/` including the re-keyed texsize, 1 root). **157 serve none of the three
  board apps** (`qt/` 85, `es100/` 52, `qt3d/` 16, `es300/` 4: Qt and dEQP-style sources that no board app
  here compiles). **The PR adds no `femtovg/` row**: the 36 `femtovg/` rows are the same on both sides.

### etna_shc builds recorded in each version [measured, header comments of the table]

| Version | Block | `etna_shc` sha256 (first 12) | glslang |
|---|---|---|---|
| main and PR | original 325 rows | not recorded in the file (the generator README says this host's `etna_shc` does not reproduce them) | - |
| main and PR | `--append`, 6 `femtovg/*.nodiscard.noscissor` rows | `e224bebc0531` | 11:16.4.0 |
| main and PR | `--replace`, `es300/dynidx.frag`, `qt3d/e0880dcd5582d4de.frag` | `a22662e5bd4e` | 11:16.4.0 |
| PR only | `--append`, the 218 rows | `1d52afefcb16` (the PR record says: built from `tools/vivante/etna_shc.c`, Mesa 26.2.1) | 11:16.4.0 |
| PR only | `--replace`, `es300/texsize.frag` | `1d52afefcb16` | 11:16.4.0 |
| PR only | `--replace`, 4 `gl_PointCoord` rows (`es100/pointcoord`, `es100/pointcoordvary`, `qt/imageparticle_coloredpoint`, `qt/imageparticle_simplepoint`) | `1d52afefcb16` | 11:16.4.0 |

The last block replaced rows that the same PR had appended, so against main it shows only as part of
the 218. **The 218 rows come from an `etna_shc` that did not build the first 325, and none of them was
run on a board** (the PR's own record says so). That is what the board run is for.

## 2. Issue #22 and `matattr`

**PR #129 adds no `matattr` rows, and it does not need to.** The four `matattr` rows
(`es100/matattr.frag`, `matattr2.vert`, `matattr3.vert`, `matattr4.vert`) are in the table of main and
of the PR, byte for byte [measured: same four names in both, and no `matattr` row in the 218].
The four gl_test sources (`gl_test_matattr_fs_src`, `_matattr2_vs_src`, `_matattr3_`, `_matattr4_`) hash
to a key that both tables hold [measured]. So the HANDOFF line "the GC7000 table still lags on
`matattr`" (the older wording of the issue and of earlier HANDOFF text) is out of date; the HANDOFF on
`origin/main` already says "the four `matattr` keys are current". gl_test case 63 (`matattr`) has no table miss on either arm.

**What does lag is the rest of gl_test's source list, and the PR closes about two thirds of it, not all.**
Counting the 192 sources of `driver/tools/pvrlab/gl_test_shaders.h` against the table [measured]:

| | main | PR #129 |
|---|---|---|
| gl_test sources with no row | **146** (equals HANDOFF's "146 of 192") | **85** (the PR record: 3 `uniidx_a/b/c` + 82 listed out) |
| sources that gain a row | - | 61 |

The 85 that remain are not a table that lags: 3 are refused by the generator's layout check
(`gl_test_uniidx_a/b/c_fs_src`), and the other 82 (`tools/shaders/gltest/` 69 and `offarm/` 13, by the
PR record) have role `-` in `tools/shaders/precompiled.list`, so no generator reads them [inferred
from the list and the PR record; the 85 itself is measured]. Making `gltest` a `gc7000` directory is
the open step HANDOFF already names. So **issue #22 is not closed by this PR**; the lag the issue
names (matattr) was never the cause, and the 61 new gl_test rows only reach cases that the default
`gc_gl_test` run does not run (section 4).

## 3. What the apps compile

The offline back end matches the **SHA-256 of the exact text given to `glShaderSource`** plus the stage;
a text with no row logs `offline shaderc: <stage> shader not in offline table (sha256=...)` and the
compile fails with ENOENT (`src/shaderc/offline.rs`). Only rows with blend key 0 are matched by text.

### Which images exist on the i.MX8MP

This matters for the board items. On `origin/main` [measured, `git grep`/`git show`]:

| Name in the plan | What exists for the GC7000 (i.MX8MP) | Offline table build |
|---|---|---|
| `gc_gl_test` | `driver/samples/zephyr/gc_gl_test`; its `prj.conf` sets `CONFIG_YAKOGL_SHADERC_ONLINE=y` | needs `YAKOGL_SHADERC_OFFLINE=y` and ONLINE off (a Kconfig `choice`) |
| `gl_slint` | **no sample of that name for it.** `gl_slint/` holds only `README.md`, `rust/` and `ui` (a crate for the SafeUI Supervisor, no CMakeLists). The i.MX8MP Slint image is the sample `driver/samples/zephyr/slint` with `boards/verdin_imx8mp_mimx8ml8_a53.conf`; `gc_slint` was removed. `prj.conf` sets `SHADERC_ONLINE=y`. | same override needed |
| `gl_femtovg` | **no i.MX8MP / GC7000 build.** `gl_femtovg` has only `boards/verdin_am62p_am62p54_a53.*`, `prj.conf` sets `CONFIG_SAFEUI_GPU_PVR=y`, `main.c` calls `pvr_sample_*`, and it needs `-DFVG_NOSTD_DIR`. `driver/docs/one-method.md` lists the GC7000 femtovg sample as "**none**" (step 4 would make one). | none |

So **`gl_femtovg` cannot be built for the Verdin i.MX8MP from main or from the PR**, and `gl_slint`
means the `slint` sample [inferred from the file list; no build was tried here]. The `slint` sample draws
through the same patched femtovg, so its shader set is the femtovg set below.

### gc_gl_test: the default set

Which header: `gl_test.c` includes `gl_test_shaders_es100.h` if `GL_TEST_ES100`,
`CONFIG_YAKOGL_SHADERC_ONLINE` or `CONFIG_YAKOGL_PRECOMPILED_SHADERS` is set, else the **default
`gl_test_shaders.h`** (GLSL 450 / 300 es texts). An offline-table image has none of the three, so it
runs the default header [inferred from the `#if` in `gl_test.c` and `CMakeLists.txt`]. The two header files
are identical on main and in the PR [measured: no diff].

"Default set" is case 7, the regression set: `order[]` of `gl_test_run()`, 50 case numbers. In a
table build `shader_skip_reason()` skips 181, 182, 183, 188, 189, 190, 191, 192, 197 and 199
("its own sources: a table build has no rows for them") [measured from the source], so **40 cases run** and 10 are
skipped by name (they count as skipped, not failed).

Method [measured]: each of the 192 header texts was hashed and looked up in both tables. For each of
the 40 cases the shaders were found by following the case function (`run_case` switch) and the
functions it calls to the `gl_test_*_src` names it uses (a textual call closure over `gl_test.c`).
Two limits [inferred]: a source that a case builds at run time with `snprintf` is not a header name and
is not seen; and the closure is textual, not a run, so a name that is mentioned but not compiled would be
counted.

| Arm | Cases of case 7 that run | Shaders they use | Table hits | Table misses |
|---|---|---|---|---|
| main (table of `136659c2`) | 40 | 27 | 26 | **1**: `gl_test_es300_texsize_fs_src` (case 46 `es3size`) |
| PR #129 | 40 | 27 | 27 | **0** |

So **the only table difference the default `gc_gl_test` run can see is case 46** (`es3size`): on main its
fragment shader is not in the offline table (the stale key), on the PR it is. The other 39 cases use rows
that are in both tables, and those rows are byte identical [measured, section 1]. The 218 new rows are not
reached by case 7.

The wider gl_test case list (what a `GC_GL_TEST_CASE=<n>` run or a sweep of 48 numbers reaches) [measured,
same method, all 166 cases that a table build does not skip by name]:

- Cases with at least one table miss: **66 on main, 23 on the PR.**
- 43 cases go from a miss to no miss with the PR: 46, 68-84, 86, 87, 89, 91, 97, 98, 100, 101, 104-106, 112-114,
  116, 117, 119-122, 128, 138, 142, 148, 163. (Case 108 `uniidx` goes from 4 misses to 3.)
- Still missing on the PR: 108, 110, 111, 115, 123, 124, 125, 129-131, 133-135, 149, 150, 162, 166-169, 193,
  200, 201. The shaders are `uniidx_a/b/c` (generator refuses), and `tools/shaders/gltest/` / `offarm/` texts (role `-`).
  Four sites build GLSL at run time and no table can name them (HANDOFF: cases 149, 150, 166).
- Case 63 `matattr` is clean on both arms.

### gl_slint and gl_femtovg (femtovg's corpus)

Slint adds no GLSL of its own; femtovg 0.26.0 with the no_std patch builds `#version 100` texts from `main-fs.glsl`
/ `main-vs.glsl` in 35 fragment variants plus `main.vert` (`tools/shaders/femtovg/PROVENANCE.md`, which records
a byte-exact check of the 36 files against the real crate). The `slint` sample has no `glShaderSource` call
(`git grep` of `driver/samples/zephyr/slint` and `gl_slint`: none). The `femtovg` application in
`precompiled.list` names exactly these 36 files.

| Arm | femtovg files hashed | Table hits | Table misses |
|---|---|---|---|
| main | 36 (35 `.frag` + `main.vert`) | 36 | **0** [measured] |
| PR #129 | 36 | 36 | **0** [measured] |

The six hand-written `fvgtest_*.frag` probes of `gl_femtovg`'s demo (`green`, `ftcoord`, `inner`, `cover`,
`discard`, `discard2`) also have rows on both arms [measured]. That the running app compiles **only** these texts is
[inferred]: it follows from the PROVENANCE check and from the absence of any shader text in the sample, and
is not a run. A shader type that femtovg builds at run time and that is not among the 36 would be a miss on
both arms.

**Prediction from this: gl_slint and gl_femtovg see the same table result on both arms (0 misses), because the PR
adds no `femtovg/` row.** Any A/B difference they show is not a table-lookup difference; the only
bytes that differ are rows that these apps never look up.

## 4. Host tests [measured]

Command (the row `host-test,gpu-vivante` of `tools/check.sh`, line 89), run in a detached worktree of each ref,
with its own target directory:
`cargo test --lib --no-default-features --features host-test,gpu-vivante`

| Arm | Commit | passed | failed | ignored |
|---|---|---|---|---|
| main | `2fe4b8798b5f` | 1865 | 0 | 0 |
| PR #129 | `419b0159d2ea` | 1864 | 0 | 0 |

Both runs exited 0 (`test result: ok`, about 17 s of test time each). The one test fewer on the PR side is a
count difference between two different code bases (the PR is based on `136659c2`, main has moved on); it is
not a failure and was not traced to a test. Only the lib tests of this one feature set were run: not
`tools/check.sh`, not the integration test targets (`stack_survey`, `online_reflection`, ...), and not the merge of
the two (the `board-merge` item runs the gate rows the merge reaches). The PR record's `1797 passed` is from its
older tip `e984a4c` and is not comparable.

## 5. What the board run should show (falsifiable)

All of this holds only for images built with the offline table (`CONFIG_YAKOGL_SHADERC_OFFLINE=y`,
ONLINE off), with the PR table in the "PR" arm and main's table in the "main" arm.

1. **gc_gl_test, case 7, main arm:** the log prints `offline shaderc: fragment shader not in offline table`
   with the sha256 of `gl_test_es300_texsize_fs_src` for case 46 and for no other case, and case 46 fails at
   shader compile or link. **PR arm:** no `not in offline table` line at all in the run. If the PR arm prints one,
   or the main arm prints one for any case other than 46, this analysis is wrong.
2. **Case 7, both arms:** 10 cases skipped by name (181, 182, 183, 188-192, 197, 199) and 40 run. The counts
   of `all: N of M cases failed` differ between the arms by at most case 46's verdict. Any other case that
   differs is not a table-lookup difference (all other rows are byte-identical), and would point at the
   build (merge) or at the board, not at PR #129's rows.
3. **Case 46 on the PR arm** uses a row whose code words are those of the old `texsize` row. Its verdict is
   not predicted here: it was never run with the new key, and only a run decides it.
4. **Slint (`slint` sample) and femtovg:** no `not in offline table` line on either arm, and no verdict difference
   caused by the table. If a miss appears on both arms, the app compiled a text outside the 36 femtovg files.
5. **gl_femtovg: nothing can run** on the i.MX8MP from these trees (no board configuration, a PowerVR
   sample). The record should say "not buildable for the GC7000" as its own outcome, not "passed" or "failed".
6. **The 218 new rows are not exercised by these three runs** except texsize. To exercise the rows that
   matter to gl_test, a sweep or single-case runs of the 43 cases in section 3 are needed (case 63 and
   others already clean); that is outside "the whole default set".

## What this analysis did not do

No board access, no merge build, no full gate (only the lib tests of section 4), no GitHub comment and no push. The table
diff, the hit/miss lookups and the per-case source mapping were computed by a short Python program that reads
the git objects of both refs (not kept in the repository). It reproduces two numbers the repository states
independently (HANDOFF: 146 of 192 gl_test sources without a row on main; the PR record: 3 + 82 without a row
after the PR), which is the check on the lookup itself.
