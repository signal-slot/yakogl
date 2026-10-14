<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc-const-array: the GC7000UL builds the 128-element constant array index, 2026-10-14, branch ccfarm/01a137e0 (from 7f1f4c27)

Only `src/shaderc/online/vivante/alloc.rs` (`sink_definitions`) changed, plus
`tests/es3_gc_const_array.rs` and the GC7000 code-byte pins in
`tests/fixtures/precompile_counts.txt`.

This is the redo of pull request #586 (branch ccfarm/01a136ad), whose gate
failed on the two rows `gen precompiled table gpu-vivante[,report-es2]`: the
change shrinks the GC7000 precompiled tables, and #586 did not re-pin them.
The code is the same as #586's.

## State of main

- dEQP-GLES3 `.test` corpus (`tests/es3_shaderc_corpus.rs`, VK-GL-CTS
  `93bca018`): GC7000UL 3,453 / 10 → 3,453 / 8 of 3,461 expected-build
  programs build, the same as the PowerVR and the V3D (both unchanged, 3,453 /
  8). `large_constant_arrays.indexing.{float,vec4}_128_vertex` now build.

## Open work

- **es3-shaderc**, "Work that closes the table" item 4 (the GC7000UL's
  128-element constant array) is done, and not the way the item said. The
  item assumed a missing feature (relative addressing into the constant
  file). The cause was register pressure made by a scheduling pass:
  - The front end reads `array[i]` as the chain `acc = (i == k) ? c_k : acc`
    (`lower.rs` `select_among`). Selection emits it compare, select, compare,
    select, which needs 3 temporaries for any length.
  - `alloc::sink_definitions` moved each select past the next compare. The
    sweep runs backwards and never comes back to the compare it passed, so
    all 127 compares ended up before all selects, and their 127 results were
    live at once: 35 and 34 temporaries. `abi.rs` refuses a vertex shader
    with one output above 21 (`Abi(LoadBalancingTempsUnmeasured)`).
  - The pass now does not move an instruction past a sibling (another
    operand of the same reader) when the move keeps two or more of its own
    sources alive. Both programs need 3 temporaries.
  - Over the 809 shaders under `tools/shaders` and `tests` on the GC7000:
    no compile verdict changed, 240 programs' code changed, 26 need fewer
    temporaries (`tmparr_const.{vert,frag}` 18 → 9, the femtovg gradients
    9 → 7, eight Qt3D particle vertex shaders down 1), and 4 need one more
    (`qt/RectangularGlow.frag` 4 → 5, `qt/shapestroke_wireframe{,.batchable}.vert`
    5 → 6 and 6 → 7, `qt3d/environmentmap.vert` 5 → 6). The pass comment
    records the two simpler rules that were measured and rejected.
  - The precompiled GC7000 tables (`tools/shaders/precompiled.list`): same
    rows, 80 code bytes less in each (`es2=0 app=all` 741,968 → 741,888,
    `es2=1 app=all` 245,312 → 245,232; the `femtovg` and `gl_test_es100`
    sublists are byte for byte the same). 223 rows (102 in the `es2=1`
    table) are reordered; 4 are shorter, all Qt Quick Shapes stroke vertex
    shaders: `qt/shapestroke.vert` and `qt/shapestroke.batchable.vert` 98 →
    97 and 100 → 99 instructions, `qt/shapestroke_expanding.vert` 124 → 122,
    `qt/shapestroke_expanding.batchable.vert` 126 → 125.
    `src/vivante/internal.rs` is unchanged (`--internal --verify`).
- Why not the address register: `src/vivante/VIVANTE_NOTES.md` (gl_test 108)
  measured that `movar` then `u[base][a.x]` reads element 0 on the i.MX8MP
  whatever the index, and nobody knows why. Mesa etnaviv (main, 26.3.0-devel)
  does not use a0 for a constant array either: it reads constant data from
  memory with `load` (`etnaviv_compiler_nir.c`, `load_constant` from
  `ETNA_UNIFORM_CONSTANT_DATA_ADDR`). If an array ever needs memory, that is
  the path to copy: a constant-data upload in `src/vivante/**`, like the
  `SHC_VIV_CONST_UBO_ADDR` uniform-block copy.
- The constant-file budget checks are unchanged: the elements are still
  placed by `select.rs` `const_reg`, which refuses a file that does not fit.
- Possible follow-up, not done: a stricter version of the rule (never move
  an instruction that keeps two sources alive past a sibling, instead of
  stopping at the sibling) also builds 5 more Qt3D vertex shaders
  (`tools/shaders/qt3d/{0d972b456ed599d1,56105a9f4a2e536b,928dc4e0891ab537,a9805e68997ed2ee,c667c97598a82c1b}.vert`,
  refused today by the same temporaries guard). But it raises 17 others, by
  up to 3 (particle vertex shaders 17 → 19, against the limit of 21). A real
  fix is a scheduler that tracks register pressure, not another rule in this
  pass.

## Next board session

No board check is needed to accept this result. The pass now moves some
instructions a shorter distance, never past a point the old pass could reach,
so every order it makes keeps the pass's correctness conditions. Selection,
the instructions and the allocator are unchanged. A board run of
`dEQP-GLES3.functional.shaders.large_constant_arrays.*` on the i.MX8MP would
confirm the two newly built programs draw. The interpreter in `tests/common`
says they compute every element (`tests/es3_gc_const_array.rs`
`every_element_reads_back`).
