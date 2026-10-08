<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Offline shader table: the 8 skipped GL tests and the GC7000 table lag (#22)

Item `es2-offline-table-diag`, 2026-10-09, integration tip `334d4e6`. Docs only; no
source changed. Every claim is marked **[measured]** (a command run in this session) or
**[inferred]** (read from code or docs, not run).

## 0. Two different "tables"

The task text speaks of one table. The tree has two, and the answers differ.

| Table | File | Made by | Committed? |
|---|---|---|---|
| Mesa reference table (the "offline" back end, `shaderc::offline`) | PowerVR `tests/reference/blobs.rs`, GC7000 `tests/reference/vivante_blobs.rs` | external Mesa tools (`pco_shc`, `etna_shc`) via `tools/glsl2usc.sh`, `tools/vivante/gen_shader_table.py` | yes **[measured]** (files exist) |
| Precompiled table (`precompiled-shaders`, `src/shaderc/precompiled.rs`) | `build/precompiled/*.rs` | our compiler, `yakogl-precompile` (`tools/precompile`) | **no**: "into build/ (never committed)" (`tools/precompile/Cargo.toml:5-8`) |

`no-shader-table` removes the Mesa reference table (`Cargo.toml:157`, `src/lib.rs:119-136`).
So there is **no committed precompiled GC7000 table to diff against**; section 3 says what
was measured instead.

## 1. The 8 tests skipped without a table

They are `#[cfg_attr(feature = "no-shader-table", ignore = ...)]` tests **[measured]**:
`cargo test --lib --no-default-features --features host-test,gpu-powervr,shaderc-online,no-shader-table,report-es2`
lists exactly these 8 as `ignored, offline-table ...` (plus 8 more ignored for another reason, below).
The same attribute lines are in `src/gl/shader.rs` and `src/gl/program.rs`.

| # | Test | Skip line | Why skipped | Runs online today? |
|---|---|---|---|---|
| 1 | `gl::shader::tests::compile_of_an_unknown_source_keeps_the_log_and_raises_no_gl_error` | `src/gl/shader.rs:1256` | Pins the table contract: an unknown text *compiles* (status true, `missing` set, "not in offline table" logged); `glLinkProgram` refuses it. | No. The online compiler refuses the text at `glCompileShader` (compile status false). Online twin: `an_invalid_source_is_refused_at_compile_time_by_the_online_compiler` **[inferred from the comment at `shader.rs:1250-1255`]**. |
| 2 | `gl::shader::tests::compile_looks_the_source_up_for_the_shaders_own_stage` | `src/gl/shader.rs:1289` | Same contract: a vertex text must not be found by a fragment shader's lookup. | No. Needs an online equivalent (the stage-mismatch check is by the lexer/stage, not a lookup). |
| 3 | `gl::shader::tests::recompile_after_fixing_the_source_clears_status_and_log` | `src/gl/shader.rs:1328` | Same contract: recompile after an unknown text clears status and log. | Partly: the "bad then good" sequence could be driven with a text the online compiler refuses (a syntax error); not written. |
| 4 | `gl::shader::tests::a_shader_the_table_lacks_compiles_and_its_text_is_logged_in_indexed_chunks` | `src/gl/shader.rs:1490` | The "not in offline table" log line, chunked by index. | No: the log is a table-miss feature. The online path logs the compiler's message instead; its chunking is a separate, unpinned behaviour **[inferred]**. |
| 5 | `gl::shader::tests::a_program_with_a_shader_the_table_lacks_does_not_link` | `src/gl/shader.rs:1545` | Link refuses a `missing` stage. | No: online, the compile itself fails, so no program reaches link. |
| 6 | `gl::program::tests::get_active_uniform_enumerates_one_entry_per_struct_array_element` | `src/gl/program.rs:5271` | Feeds `light_array_reflection()`, the table's flattened per-member rows; `build_locations` only flattens rows of an offline stage (`!shaderc::is_online`). | The online shape of the same dEQP contract is pinned by `nested_struct_array_uniforms_reach_the_shader_as_deqp_sets_them` (`program.rs:5266-5268`, comment). Test 6 itself needs a hand-built reflection tagged "offline": possible with a stage flag in `fake_program`, not done. |
| 7 | `gl::program::tests::struct_array_uniform_locations_follow_the_gl_naming_rules` | `src/gl/program.rs:5324` | Same. | Same. |
| 8 | `gl::program::tests::a_write_to_a_struct_members_array_stops_at_that_array` | `src/gl/program.rs:5353` | Same. | Same. |

The skip is by `ignore`, not by an early return, so it shows in the count. `tools/check.sh`
lists ignores only in the V3D row (`v3d_lib_tests`, `build/check/v3d-ignored.txt`,
`tools/check.sh:264-300`) **[measured: read]**. I could not run that V3D row here: the
`gpu-v3d` build of the library was killed (exit 137) on this machine, so the V3D ignore
file was not produced **[measured: the run was killed]**.

### Measured results (PowerVR, host)

| Configuration | Result |
|---|---|
| `host-test,gpu-powervr` (with the Mesa table) | 1889 passed, 0 failed, 0 ignored **[measured]** |
| `host-test,gpu-powervr,shaderc-online,no-shader-table` (no `report-es2`) | 2546 passed, 18 failed, 8 ignored **[measured]** |
| same `+report-es2` | 2456 passed, 27 failed, 16 ignored **[measured]** |

Reading the failures **[measured: panic text; the cause is inferred]**:

- The 18 failures in the first no-table row: tests that name an ES 3.00 shader
  (`83a830e25f56fd85.vert`, `0119aeb276db508c.frag`) fail with
  "`sampler2DArray`: a GLSL ES 3.00 type, and this source is ES 1.00". Those two Qt3D sources
  use an ES 3.00 type and are judged as ES 1.00 by the online compiler **[inferred: the sources were not read]**.
  Others fail on assertions (`program.rs:6351`, `6475`) that expect table reflection sizes.
- The 9 extra failures with `report-es2`, and 8 ignored `GLSL ES 3.00 test shader (cube.vert)`,
  are the known "report-es2 refuses ES 3.00" tests.
- This is why `tools/check.sh:84-98` makes the no-table PowerVR and GC7000 rows BUILD-ONLY and why
  the comment there says "297 tests". The current number is 18 (27 with `report-es2`) **[measured]**,
  so the comment is out of date. The gate only runs the no-table suite for V3D.

So today the no-table path is **not covered** for PowerVR or GC7000 in the host gate:
"8 skipped" understates it. The 8 are the visible part; 18 to 27 more fail and are
not run at all.

## 2. What would make the 8 run online (inferred)

1. Tests 1–5: write online counterparts in `src/gl/shader.rs` next to
   `an_invalid_source_is_refused_at_compile_time_by_the_online_compiler`, and keep the 5 as the
   offline contract under `#[cfg(not(feature = "no-shader-table"))]` instead of `ignore`
   (an `ignore` still counts as a pass in a summary). Missing: a pinned online message and
   status for each of the five behaviours.
2. Tests 6–8: `fake_program(&gl, light_array_reflection(), ...)` needs a way to mark its stages offline
   in a no-table build. Missing: that flag in `testutil` (`src/testutil.rs`), or run them against the
   precompiled tier (`precompiled-shaders` stages are offline-shaped?). **[inferred; `is_online`
   for a tier hit was not read]**.
3. Make the PowerVR/GC7000 no-table rows testable: fix the 18/27 failures first (they come from
   `testutil::shader_source` mapping names to files whose text differs from the table's, `src/testutil.rs:1440-1480`).

## 3. GC7000 table lag (#22)

What the docs say **[measured: grep]**: `docs/HANDOFF.md:95` "The GC7000 table still lags on `matattr`: issue #22, not
fixed by `0fe778f6`"; `docs/es2-cts-failures.md:397` "Only matters for images that use the precompiled tier.
The dEQP images use the compiler. No CTS case is known to hit it."; `tools/cts/STATUS.md:21` unchanged.
The origin is `0fe778f6`, which re-keyed the **PowerVR** Mesa table and added `matattr`'s 20 rows; the
GC7000 table was not given the same treatment.

### Measured

1. **Precompiled GC7000 table, regenerated twice** into `build/scratch/` (ignored dir, outside git):
   `cargo run -q -p yakogl-precompile --features gpu-vivante -- --app gl_test_es100 -o build/scratch/t{1,2}.rs`
   → 111 rows, 32512 code bytes, 731131 bytes. `cmp t1.rs t2.rs`: **identical**. No nondeterminism.
2. **Diff against the committed table: not possible**; no precompiled table is committed (section 0).
   Shaders whose binary differs: **0 of 111 between two generations; N/A against a committed copy**.
3. **Committed GC7000 Mesa table, matattr**: `tests/reference/vivante_blobs.rs` has rows for
   `es100/matattr.frag`, `matattr2.vert`, `matattr3.vert`, `matattr4.vert`; their SHA-256 equal the current
   `sha256sum tools/shaders/es100/matattr*` for all four. So the *keys* are current; the matattr rows are
   there **[measured]**. HANDOFF's "lags on matattr" is therefore not a missing-row problem. What is lagging
   is the code in those rows, or the claim is stale; I cannot tell which without `etna_shc`, which is not
   on this machine (`gen_shader_table.py` died at `glslangValidator`) **[measured]**. Treat the HANDOFF line as unverified.
4. **Real, measured lag of the GC7000 Mesa table**: `python3 -I tools/vivante/check_gl_test_blobs.py` (exit 1) reports
   the table has 325 rows and **77 of gl_test's 123 shaders have no row**, in both `gl_test_shaders.h` and
   `gl_test_shaders_es100.h` (e.g. `gl_test_offarm*`, `uniidx_*`, `deriv`, `es300_*`, `varyings15`), plus 7
   pass-through shaders with an empty program. `glsl2usc.sh inputs: 587 sources, 0 stale, 32 excused` (the PowerVR side
   is clean). This script is **not called from `tools/check.sh`** (`grep` finds no call) **[measured]**.

### Classification of differences

- Compiler changes since generation: none measurable for the precompiled table (it has no committed copy
  and `tools/check.sh` regenerates it each run, `tools/check.sh:122-131`, so it cannot be stale in the gate **[inferred from the script]**).
- Nondeterminism: none (item 1).
- The Mesa GC7000 table lags **the source list**, not the compiler: 77 gl_test shaders have no row (item 4).

### Could a dEQP-GLES2 case on i.MX8MP see different code from the table?

- The dEQP image uses `CONFIG_YAKOGL_SHADERC_ONLINE=y` (`driver/samples/zephyr/deqp/prj.conf:6`) and the
  i.MX8MP dEQP row is `host-test,gpu-vivante,shaderc-online,report-es2` **[measured: read]**; it has no table.
  dEQP shaders are application-written text; a table keyed by SHA-256 of fixed sources would not hit them
  anyway **[inferred]**. So: **no, not through the table**. Only if a dEQP image enabled `precompiled-shaders`, and then a hit
  runs code from the same compiler (tier tests assert "a hit is what the compiler makes", `check.sh` comment) **[inferred; not run]**.
- Per board: AM62P and Pi 4: none (the V3D has no table at all, `Cargo.toml:81-84`; the AM62P dEQP image is online too, **[inferred]**).
  i.MX8MP: none. The lag matters for gl_test / Slint / femtovg images that use the Mesa or precompiled table (stage 2 benchmarks), not for stage 1.

## 4. Fix proposal (inferred; not implemented)

1. `tools/check.sh`: add a step (full mode, next to `tools   shader-table generator guards`, line 930)
   `python3 tools/vivante/check_gl_test_blobs.py`. It exits 1 on 77 rows now, so first either add the 77 shaders
   to the GC7000 table (`gen_shader_table.py --append`, needs `etna_shc`) or list them as excused in
   `tools/shaders/known-uncompilable.txt` with provenance. Without that, the step is red on day one.
2. Staleness check for the precompiled tier: already regenerated per run (`tier_tables()`, `check.sh:~122`).
   Add the same for any image that commits a table: `yakogl-precompile ... -o build/x.rs` then `cmp` against the committed copy,
   failing on difference (the run in section 3 shows output is deterministic, so `cmp` is a sound check).
3. Cover the no-table path: in `src/gl/shader.rs` and `src/gl/program.rs` replace the 8 `cfg_attr(..., ignore)` with
   `#[cfg(...)]` pairs plus online twins (section 2); in `src/testutil.rs` `shader_source` (the `SOURCES` list)
   and `reset_for_tests` make the no-table PowerVR/GC7000 rows pass, then move
   `host-test,gpu-{powervr,vivante},shaderc-online,no-shader-table` from `BUILD_ONLY_CONFIGS` (`check.sh:92-95`) to `CONFIGS`
   with a `MIN_TESTS` floor like `V3D_MIN_TESTS`. Print ignores for every row, not only V3D.
4. Update `docs/HANDOFF.md:95` once someone with `etna_shc` confirms whether the matattr rows differ from a fresh compile.

## 5. Not done

- V3D no-table ignore list (build killed), the tier test run with the regenerated GC7000 table (the
  environment variable form was refused by the sandbox), and a fresh `etna_shc` comparison (tool absent).
