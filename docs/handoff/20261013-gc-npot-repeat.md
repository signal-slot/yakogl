<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc-flake-instrument: KICK_DIFF confirmed at the tip, GC7000 NPOT REPEAT host test, 2026-10-13, main a63fcc5b

Details: `docs/investigations/gc-npot-repeat.md`. No file under `src/` or
`driver/` changed.

## Open work

- i.MX8MP case 10 / case 2 flake (#9, #25): the KICK_DIFF instrument
  (`src/vivante/kick.rs`, `gl_test.c`, `gc_gl_test/src/main.c`) was read
  end to end at main `a63fcc5b` and is complete for the planned run (about 40
  interleaved boots each of the base arm and the KICK_DIFF arm, one image, read
  from the SNVS). Selectors: base `SEL=512`; KICK_DIFF case 10 `SEL=4586`
  (`0x11ea`); case 2 `SEL=4578` (`0x11e2`). Pages decode with
  `yakogl-tools record kickdiff`. Seven gaps are listed there (G1 to G7); none
  needs a code change.
- Stage 1 residual risk "the GC7000's REPEAT on NPOT sizes"
  (`docs/es2-cts-failures.md` H.2/H.3): `tests/gc_npot_repeat.rs` pins the
  texture descriptor and sampler registers the Vivante back end emits for the
  17 NPOT wrap and size cases, against the rnndb headers and Mesa 26.2.1.
  All 17 pass. Nothing is wrong at the CTS sizes. Wrap, size, level count and
  texel addresses are as the register meaning says.
- New defect, low severity: `TEXDESC_LOG_SIZE_EXT` is a linear approximation
  of log2 (`src/vivante/texture.rs` `log2_fixp88`); Mesa writes the exact
  value. The CTS sizes are within one unit; a size like 90 is 0.086 octave low.
  Pinned by the ignored test
  `bug_log_size_ext_is_a_linear_approximation_of_log2`. A later item may fix
  it in one function; the fix removes the `ignore`.

## Known defects

- Opened (no issue number yet): `TEXDESC_LOG_SIZE_EXT` for NPOT sizes, above.
- Corrections to `tools/cts/log/investigations/gc-kick-diff.md`, not made here
  (outside this item's three files): the two commands'
  comment "list 0" is list 1 (`0x11ea`); case 10 is bit 10 of the sweep pass
  word, not bit 9; the sample output's "10000 us" is 2000 us.

## Next board session

`imx8mp-gltest-r2` can run its #9/#25 part as it is, with one condition: build
the image first and read `rc=`. The Zephyr arm of `gc_gl_test` and `gl_test.c`
has never been built from this code (`gc-kick-diff.md`, "What was checked"), so
a compile error shows at the build, not at a boot. Use the selector values
above, not the comments in `gc-kick-diff.md`. The loop that interleaves the two
arms is the session's to write. Image recipe:
`tools/cts/imx8mp-gltest-build.sh kd-<sha8> 0 '-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10\;10\;2"'`.
No board check is needed for the NPOT result; the next session may add a texel
read-back across a REPEAT seam if it wants more than the solid-colour CTS cases
show (investigation, "What it cannot see").
