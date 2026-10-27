<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP: the KICK_DIFF instrument at the tip, and NPOT REPEAT on the GC7000

Item `gc-flake-instrument` of plan #20, at main `a63fcc5b`. Host only: no
board was run, and nothing under `src/` or `driver/` changed. Two parts:

1. Is the KICK_DIFF instrument complete for the planned board run (about 40
   interleaved boots each of the base arm and the KICK_DIFF arm, one image,
   read back from the SNVS, no console)?
2. Stage 1 residual risk, "the GC7000's REPEAT on NPOT sizes"
   (`docs/es2-cts-failures.md` section H.2, H.3): what the Vivante back end
   emits for each NPOT wrap case, against the register meaning.

## Part 1: the KICK_DIFF instrument at the tip

### Verdict

**The code is complete for the run; nothing in it needs to change before the
boots.** The path is: library knob (`src/vivante/kick.rs` `diff`,
`src/gl/framebuffer.rs` `gc_kick_diff_arm_target`) → harness
(`driver/tools/pvrlab/gl_test.c`, the block above `read_topdown`) → selector
and record (`driver/samples/zephyr/gc_gl_test/src/main.c` `kick_diff_tag`,
`report_result`) → decoder (`tools/yakogl-tools/src/kickdiffrec.rs`). Each
link was read, and the two host test sets that cover it pass at this tip:

```text
cargo test --no-default-features --features host-test,gpu-vivante,test-knobs --test gc_kick_diff   # 7 passed
cargo test -p yakogl-tools kickdiff                                                                  # 6 passed
```

Two things stop me writing "ready" without a condition. The first is gap G1:
no Zephyr image has been built from this code. The second is a set of
documentation errors (G2 to G4) that would cost a wrong selector or a wrong
reading if the run copied them. Both are below, with the exact lines.

### Selector values

`SEL = GC_RESULT_PERF_SEL (0x1000) | list << 8 | tag`, tag `0xe0 | case`
(`main.c` `kick_diff_tag`; `yakogl_record.h` `YK_RECORD_PERF_SEL`). `board.sh`
writes `0x53450000 | SEL` into SNVS word 1 (`tools/board.sh` `snvs_clear`), so
a selector must fit 16 bits; these do.

| arm | `SEL` | hex | what runs |
|---|---|---|---|
| base | `512` | `0x0200` | entry 0 of `CONFIG_GC_GL_TEST_SWEEP_LISTS` (`1-6,8-10`), no instrument; the run ends with the SWEEP counters record |
| KICK_DIFF, case 10 | `4586` | `0x11ea` | the same list, instrument armed for case 10; the run ends with the case 10 KICK_DIFF page |
| KICK_DIFF, case 2 | `4578` | `0x11e2` | the same list, instrument armed for case 2; the case 2 page |
| case 10 alone (not planned) | `4330` | `0x10ea` | list 0 means "the case alone" |
| case 2 alone (not planned) | `4322` | `0x10e2` | |

The base arm and the KICK_DIFF arms run the same list on the same image; the
selector is the only difference (`main.c` comment at `kick_diff_tag`). Case
numbers 2 and 10 are the only ones the harness arms
(`gl_test.c` `KICK_DIFF_WIRED`); tag `0xe0..0xef` do not meet the case 16
pages `0xf0..0xfd` (`msaa_page_tag`).

### Record fields

Base arm: the SWEEP counters record, kind SWEEP, tag `0xff`
(`gc_result.h` kind SWEEP; `gc-case10-flake.md`, "Board recipe"). With
`first = 0` case `c` is bit `c` of word 3, so case 10 is `0x400` and case 2 is
`0x004`. `594b07ff,00000000,0000ff00,0000077e` is "all nine passed".
Decode: `yakogl-tools record sweep 0 <w1> <w2> <w3>`. A non-zero w1 means lost
GPU work, a reset or a fence timeout: not a reading.

KICK_DIFF arms: a PERF record, w0's low byte the tag (`0xea` or `0xe2`),
w1..w3 as `gl_test_kick_diff_page` packs them (`gl_test.c`, same layout as
`gc-kick-diff.md`, "The page"):

| word | bits | meaning |
|---|---|---|
| w1 | 0..7 | verdict: 0 armed, no check made; 1 stable; 2 late (at least one check); `0x80 \| n` not armed, `gc_kick_diff_arm_target` returned `-n` |
| w1 | 8..15 | first late check, from 1 (0 none) |
| w1 | 16..23 | the case's own result: 1 pass, 2 fail, 3 neither |
| w1 | 24..31 | the case's first failing check (`0xff` none, `0xfe` 254 or more) |
| w2 | 0..15, 16..31 | checks made, late checks (each saturates at 0xffff) |
| w3 | all | the most words one check saw change |

Decode: `yakogl-tools record kickdiff <w0> <w1> <w2> <w3>`, or `--nvmem`/`--log`.
Case 10 makes 8 checks (one per read-back), so a clean KICK_DIFF run reads
`checks 8 late 0`.

### Commands

From `tools/cts/imx8mp-gltest-build.sh` and `tools/board.sh` (the Zephyr
environment of `tools/cts/imx8mp-env.sh` set; `BOARD_BOOT=uboot-mmc` as the
board sessions use):

```sh
# one image, built once. -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y is already in the script.
tools/cts/imx8mp-gltest-build.sh kd-$(git rev-parse --short=8 HEAD) 0 \
  '-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10\;10\;2"'          # \; keeps the separators through cmake
IMG=$IMX8MP_OUT/kd-<sha>/zephyr/zephyr.bin

# 40 interleaved rounds; the board is the only variable that drifts, so it drifts for both arms
for i in $(seq 1 40); do
  SEL=512  BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run "$IMG" <logdir> base-$i
  SEL=4586 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run "$IMG" <logdir> kd10-$i
done
# case 2, if the plan wants its own arm:  SEL=4578 ... kd2-$i
```

Each `run` ends with one verdict line carrying `record=<w0>,<w1>,<w2>,<w3>`
(the board has no console; `board.sh` reads the SNVS after Linux is back).
Tally:

```sh
grep -ho 'record=[0-9a-f,]*' <runner outputs of the kd10 runs>  | cut -d= -f2 | tr ',' ' ' | xargs yakogl-tools record kickdiff | sort | uniq -c
grep -ho 'record=[0-9a-f,]*' <runner outputs of the base runs> | cut -d= -f2 | tr ',' ' ' | xargs -L1 -I{} sh -c 'yakogl-tools record sweep 0 $(echo {} | cut -d" " -f2-)'
```

(The first is `gc-kick-diff.md`'s. The second is mine: `record sweep` takes
one record, `first w1 w2 w3`, so it is called once per run; it drops w0. Neither
was run against a board record. `yakogl-tools` is
`target/debug/yakogl-tools`, built with `cargo build -p yakogl-tools`, or
`$YAKOGL_TOOLS` for `board.sh`.)

### What the run reads from it

A `late` verdict on a failing run is the finding. A KICK_DIFF arm that still
fails with `stable` means the write never arrived in 2 ms, or 2 ms was too
short. The instrument sits before the case's own read, so a late write is
seen *and* is in memory when the case checks: the KICK_DIFF arm can pass.
Compare verdicts across arms, not the KICK_DIFF arm's failure rate against
the base arm's (`gc-kick-diff.md`, "Reading it"). The base arm's rate is the
control: the last measurement is case 10 failing 4 of 32 (#9), so 40 base boots expect about 5 failures.

### Gaps

None of these is fixed here.

- **G1. The Zephyr arm has never been built.** `gc-kick-diff.md`, "What was
  checked": "gc_gl_test's `main.c` and gl_test.c's Zephyr arm were only
  syntax-checked against stub Zephyr headers, so the first image build is still
  owed". The host side is checked: `gl_test.c` compiles with
  `-DYAKOGL_TEST_KNOBS=1` in `tools/check.sh` (the gate's host rows, line
  1766-1769), and the library knob has host tests. A compile error in
  `main.c`, or the `k_busy_wait` call in `gl_test.c`, would only show at the
  image build. Cost if it fails: the build step of the board session, before
  any boot. Needed: build the image first and read `rc=`.
- **G2. The selector comments in `gc-kick-diff.md` say "list 0" for
  `0x1000 + 0x100 + 0xea`.** That value is list 1 (`0x11ea`). List 0 is
  `0x10ea`, the case alone. The *numbers* (4586, 4578) and the call at
  `main.c:518-521` are right and make the same run as `SEL=512`; only the
  comment on the command is wrong. Use the table above.
- **G3. "Case 10 is bit 9 of the pass bits" (same file, "How to run") is wrong:
  it is bit 10** (`0x400`). `s_sweep_pass` bit `i` is case `FIRST + i`
  (`main.c:210`), `FIRST` is 0, and the word table in `gc-case10-flake.md`
  (`0x77e` pass, `0x37e` case 10 failed) agrees with bit 10. Reading bit 9
  would report case 9 (stencil) as the flaky one.
- **G4. The sample output in `gc-kick-diff.md` shows "second read 10000 us
  later"**, while `gl_test.c` arms `KICK_DIFF_DELAY_US 2000` and the file's own
  "Call site" paragraph says 2 ms. The console line is not visible on this
  board anyway (no console); only the page is. Ignore the 10000.
- **G5. There is no loop script for the interleaving.** `board.sh run` is one
  boot. The loop above is the whole of it, and `imx8mp-gltest-r2` must write
  it (and the tally) into its record. Not a defect of the instrument.
- **G6. A boot that hangs or resets before the case ends leaves no page.** The
  SNVS then holds the SWEEP record (the `report_result` sweep path returns
  without a page: `main.c` "the sweep's own record stays the verdict") or the
  watchdog's record. `yakogl-tools record kickdiff` says "no PERF record with a
  KICK_DIFF tag" for it. Count such boots as their own row in each arm, with
  their `resets=`; a hang in the KICK_DIFF arm that the base arm does not have
  would be the instrument's own effect (2 ms waits, a `glFinish`).
- **G7. Design limit, not a gap in the build: `stable` cannot tell a right
  write from a lost one.** The case's own pass bits in w1 (bits 16..23) do. It
  is stated in the code and in `gc-kick-diff.md`; repeated here because the
  tally has to be read with both fields.

### Can the 40+40 run start

Yes, once the image has been built (G1). The selector values, the page layout
and the decoder agree across `main.c`, `gl_test.c`, `kickdiffrec.rs` and
`board.sh`. This is a reading of the code, not a board result: the
instrument has not read a real GC7000 target.

## Part 2: NPOT REPEAT on the GC7000, host test

`GL_OES_texture_npot` is listed on all three GPUs, so dEQP-GLES2 expects
REPEAT and MIRRORED_REPEAT on NPOT textures to work (section H.2/H.3). The
i.MX8MP tree passed all 18 cases at `334d4e65` and `32778b57` (section H.4).
`tests/es2_npot.rs` checks that the GL core calls those textures complete.
`tests/gc_npot_repeat.rs` checks the other half on the host: what the Vivante
back end puts in the command stream.

Run it with:

```text
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_npot_repeat
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_npot_repeat
cargo test ... --test gc_npot_repeat -- --ignored      # the BUG test; it fails today
```

Result: 17 passed, 1 ignored (the BUG test, which fails when run), in both
builds and serially.

### How it measures

The real GL core, the real back end (`gc_draw`, `gc_flush`) and the in-process
compiler for the quad's two programs. Only the OSAL is a stand-in: host memory
and a copy of every submitted command stream. The test follows the CTS's GL
calls (`es2fTextureCompletenessTests.cpp`, as `tests/es2_npot.rs` does), draws
the quad, decodes the LOAD_STATE writes, takes the descriptor that
`NTE_DESCRIPTOR_ADDR(0)` names, and reads the texels that its level addresses
point at.

### The cases and what is pinned

Every NPOT wrap case of H.3 has a test of the same name as in
`tests/es2_npot.rs`. The seven "expect black" size cases are in too: they set
REPEAT, and the question is what the sampler sees when the GL core calls the
texture incomplete.

| dEQP case (`functional.texture.completeness.` ...) | test | texture | sampler fields asserted |
|---|---|---|---|
| `2d.npot_t_repeat` | `texture_completeness_2d_npot_t_repeat` | 127x127, 1 level, NEAREST | U CLAMP_TO_EDGE, V REPEAT |
| `2d.npot_s_repeat` | `..._2d_npot_s_repeat` | same | U REPEAT, V CLAMP_TO_EDGE |
| `2d.npot_all_repeat` | `..._2d_npot_all_repeat` | same | U, V REPEAT |
| `2d.npot_mirrored_repeat` | `..._2d_npot_mirrored_repeat` | same | U, V MIRRORED_REPEAT |
| `2d.repeat_width_npot` | `..._2d_repeat_width_npot` | 127x128 | U, V REPEAT |
| `2d.repeat_height_npot` | `..._2d_repeat_height_npot` | 128x127 | U, V REPEAT |
| `2d.npot_size` | `..._2d_npot_size` | 255x255, 8 levels (255..1), NEAREST_MIPMAP_NEAREST | U, V REPEAT; MIN, MIP, MAG NEAREST; MAXLOD 7 |
| `cube.npot_t_repeat` | `..._cube_npot_t_repeat` | 6 x 127x127 on the default cube texture | U CLAMP_TO_EDGE, V REPEAT |
| `cube.npot_s_repeat` | `..._cube_npot_s_repeat` | same | U REPEAT, V CLAMP_TO_EDGE |
| `cube.npot_all_repeat` | `..._cube_npot_all_repeat` | same | U, V REPEAT |
| `cube.npot_mirrored_repeat` | `..._cube_npot_mirrored_repeat` | same | U, V MIRRORED_REPEAT |
| `2d.npot_size_level_0`, `npot_size_level_1` | `..._2d_npot_size_level_0`, `_level_1` | incomplete | the 1x1 black stand-in, clamped |
| `cube.npot_size_level_0`, `_level_1`, `_level_0_pos_x`, `_level_1_neg_x` | `..._cube_npot_size_*` | incomplete | same |

`negative_api.texture.generatemipmap_npot_wdt_hgt` is the 18th H.3 case. It
returns before any GL call when the extension is listed, so it has no wrap and
no draw; `tests/es2_npot.rs` has it.

Every complete case asserts, each with the register field in its failure text:

| register / word | field | value asserted | source of the meaning |
|---|---|---|---|
| `NTE_DESCRIPTOR_SAMP_CTRL0` | UWRAP bits 0-2, VWRAP 3-5 | the GL wrap as `TEXTURE_WRAPMODE` (REPEAT 0, MIRRORED_REPEAT 1, CLAMP_TO_EDGE 2) | `state_3d.xml.h:1953-1958`, `common_3d.xml.h:108-111` |
| same | WWRAP 6-8 | equal to UWRAP | `texture.rs` `samp_wwrap(uwrap)`; a 2D or cube lookup has no third coordinate |
| same | MIN 9-10, MIP 11-12, MAG 13-14 | `TEXTURE_FILTER` (NONE 0, NEAREST 1) | `state_3d.xml.h:1962-1970`, `common_3d.xml.h:99-102` |
| same | every other bit | only UNK21 (bit 21) and INT_FILTER (bit 23) | `state_3d.xml.h:1979,1981`; Mesa `etnaviv_texture_desc.c:105-116,319-322` |
| `SAMP_CTRL1`, `TX_CTRL`, `SAMP_LOD_MINMAX`, `SAMP_LOD_BIAS`, `SAMP_ANISOTROPY` | whole word | `0x2`, 0, `0x0fff`, 0, 0 | `regs.rs` (Mesa's constant UNK1; no tile status; GL's default LOD range saturated) |
| `TEXDESC_CONFIG0` | TYPE bits 0-2, FORMAT 13-17, ADDRESSING_MODE 20-21 | 2D (2) or cube (5); A8B8G8R8 (9); LINEAR (3) for a plain 2D texture, TILED (0) for a cube | `common_3d.xml.h:103-107,131-132`; `tiled_copy.rs` `wanted` |
| `TEXDESC_SIZE` | width, height | the texture's own size (127, not the padded 128) | `texdesc_3d.xml.h:80`; Mesa `etnaviv_texture_desc.c:202-203` |
| `TEXDESC_LOG_SIZE_EXT` | 16 bits each | Mesa's `(int)(log2f(n) * 256)`, exactly (`texture.rs` `log2_fixp88`) | Mesa `etnaviv_util.h:57`, `etnaviv_texture_desc.c:200-201` |
| `TEXDESC_BASELOD` | BASELOD bits 0-3, MAXLOD 8-11 | 0, levels - 1 | Mesa `texdesc_3d.xml.h:141-152` |
| `TEXDESC_CONFIG1`, `CONFIG2`, `VOLUME`, `3D_CONFIG` | whole word | RGBA swizzle, no TEXTURE_ARRAY, no HALIGN; `0x30000`; log2 depth 0; depth 1 | `regs.rs`; Mesa `etnaviv_texture_desc.c:172-194` |
| `TEXDESC_LINEAR_STRIDE`, `SLICE` | whole word | stride covers the row; for the cube copy `align4(127) * 4` = 512 and slice `128 * 128 * 4` | `tiled_copy.rs` |
| all other descriptor words | | 0 | there is no NPOT field in the HALTI5 descriptor |
| texel memory | | every texel of every level and face reads back at `LOD_ADDR + y * stride + 4x` (2D), or in its 4x4 tile at `LOD_ADDR + face * slice` (cube) | `tiled_copy.rs` `tiled_offset` |

The last two rows matter most for REPEAT. The texture unit wraps at SIZE, so
SIZE must say 127; and a texel that the descriptor does not reach would be
sampled from the wrong row. The texel check proves the descriptor and the GL
core's storage (and, for a cube, the back end's tiled copy) agree for every
texel, for each of 127x127, 127x128, 128x127, the 255 chain, and the six 127
cube faces.

### What it found

- **NPOT bits.** There are none to set. NPOT support on this part is the chip
  feature `chipMinorFeatures1_NON_POWER_OF_TWO` (`common.xml.h:181`), and the
  descriptor has no NPOT field. The check that matters is that no stray bit is
  set, and none is. Whether the GC7000UL has that feature bit is the open
  question of H.1: the repository records `features` and `minor0` but not
  `minor_features[1]`. The board passes the 18 cases, so the answer in practice
  is yes. The next board session can record `minor_features[1]` if it wants the
  fact rather than the inference.
- **A 127 or 255 texture is described correctly** for all the wrap pairs above:
  the wrap fields are the GL ones, SIZE is the real size, the level count is
  right, and the texels are where the descriptor says. Nothing wrong was found
  for the 18 cases.
- **`TEXDESC_LOG_SIZE_EXT` is Mesa's exact log2.** `texture.rs` `log2_fixp88`
  computes `floor(log2(n) * 256)` bit by bit (no libm on the target) and equals
  Mesa's f32 form for every n up to 16384. It was a straight line between 2^k
  and 2^(k+1), 0.086 octave low at 90 (0x668 against 0x67d); the CTS sizes 127
  and 255 were within one unit. The same function writes `TEXDESC_VOLUME`, so
  a depth that is not a power of two (3D textures, 2D arrays) moved too: 3 is
  now 0x195, was 0x180. The test `log_size_ext_is_the_exact_log2` asserts
  Mesa's value for 90x70. What the texture unit does with the field is not
  known.
- **Observation, not asserted as a defect.** An incomplete texture is replaced
  by a 1x1 black `TYPE 2D` descriptor even for a `samplerCube` program (the
  seven "expect black" cases). The i.MX8MP tree passes the four cube ones, so
  the sampler's type does not break it; the test only records the type.

### What it cannot see

The GPU. Whether the texture unit wraps NPOT coordinates at SIZE (127) and not
at the storage width (128) is the board's answer, and the board passes the 18
cases (H.4), which sample a solid-colour texture and so would pass a wrap at
the wrong size. The first test that would tell is a texture whose texels differ
at the wrap seam, read back as pixels (a gl_test case, or glmark2's terrain
scene). That is a new board check, not part of this item.

Mesa references are to Mesa 26.2.1 (`mesa-src` in the shared dependency
cache), the same version `regs.rs` cites.
