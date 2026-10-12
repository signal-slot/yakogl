<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc-msaa-addresses: case 16's 4x stream against Mesa's etnaviv (issue #8)

Date: 2026-10-12. Host only; no board run was made for this record.

## Question

The round-2 board record (`tools/cts/log/board-sessions/imx8mp-gltest-r2.md`,
image `e3efeaad`) shows case 16's 4x half failing check 4 (`x4 inside` got
`0x00000000`, want `0xffff00ff`) in arms 1 and 9, with the 64 KiB scratch
empty after the frame, after the clear alone and after the draw alone. The
status comment on #8 asks two things of the host: (1) is the 4x colour
surface given to the PE and the RS the one the probe reads, and (2) how the
4x setup differs from Mesa's etnaviv.

## 1. The addresses agree

Captured from the host mock stream (`host-test,gpu-vivante,shaderc-online,test-knobs`),
case 16's shape: 64x64 RGBA8 target, `glClear` (0,0,0,1) and one triangle in
one frame, `gc_debug_msaa` 1 and 9. Mock addresses (the board's differ; what
matters is that they are equal):

| register | value (arm 1 = arm 9) | what |
|---|---|---|
| `PE_PIPE_COLOR_ADDR(0)` 0x01460, `(1)` 0x01464 | `0x40012000`, both draws (clear quad and triangle) | the scratch, `g().msaa.gpu` |
| `PE_COLOR_STRIDE` 0x01434 | `0x200` | 512 bytes = 128 samples x 4 |
| `PE_COLOR_FORMAT` 0x0142c | `0x00102f06` | A8R8G8B8, COMPONENTS 0xf, SUPER_TILED_NEW, SUPER_TILED, no OVERWRITE |
| `RS_PIPE_SOURCE_ADDR(0)` 0x016c0 | `0x40012000`, one write | the scratch |
| `RS_PIPE_DEST_ADDR(0)` 0x016e0 | `0x40100000` | the target |
| `TS_MEM_CONFIG` 0x01654, `PE_MEM_CONFIG` 0x014bc | `0`, `0` | no tile status |
| probe `gc_debug_msaa_scratch` | reads `g().msaa.cpu`, 65536 bytes | the CPU view of the same allocation |

The probe reads the allocation the PE and RS are given. The Zephyr GC OSAL
maps every allocation from a Normal non-cacheable carveout
(`src/osal/zephyr_gc.c:150`), so a stale CPU cache cannot hide GPU writes
from the probe. Pinned by the host test
`every_4x_colour_address_is_the_scratch_the_probe_reads`
(`src/vivante/backend_tests.rs`).

So the probe looks in the right place, and the PE writes nothing there. The
resolved target reads `0x00000000` (alpha 0 too), which is what the resolve
makes of an all-zero scratch.

## 2. Our 4x setup against Mesa's

Reference: Mesa 26.2.1 (tag `mesa-26.2.1`, the release `third_party/mesa-26.2.1`
vendors; etnaviv is not vendored, so the files were fetched from
gitlab.freedesktop.org at that tag and read). Core: the i.MX8MP's GC7000UL,
chip 0x7000 rev 0x6204, product 0x70003. Its features, from
`src/etnaviv/hwdb/nxp/gc_feature_database.h` (both entries for rev 0x6204,
ECO 0 and 1): `REG_SmallMSAA` 1, `REG_PEEnhancements3` 1 (which
`etna_hwdb.c:84` maps to `ETNA_FEATURE_SINGLE_BUFFER`), `CACHE128B256BPERLINE`
1, `RS_NEW_BASEADDR` 1, `MSAA_FRAGMENT_OPERATION` 1, `REG_FastClear` 1,
`NumPixelPipes` 1.

What Mesa writes for a 4x RGBA8 render target on this core, register by
register, and ours (arm 1):

| register | Mesa | ours | same? |
|---|---|---|---|
| `GL_MULTI_SAMPLE_CONFIG` | `MSAA_SAMPLES_4X \| MSAA_ENABLES(0xffff)` = `0xf2` (state.c:505, emit.c:358-362, context.c:771), written before `PS_INPUT_COUNT` | `0xf2`, before the shader state | yes |
| `RA_MULTISAMPLE_UNK00E04`, `UNK00E10[0..2]`, `RA_CENTROID_TABLE[0..11]` | state.c:508-523 | the same words | yes |
| `PS_INPUT_COUNT` | `PS_INPUT_COUNT_MSAA`: varyings + 2 (shader.c:236-238; emit.c:466-469 picks it whenever `msaa_mode`) | varyings + 1 (only bit 1 adds one) | **no** (A) |
| `PS_TEMP_REGISTER_CONTROL` | `MAX2(temps + 1, varyings + 2)` (shader.c:240-241; emit.c:470-473) | `MAX2(temps, varyings + 1)` (only bit 1 adds one) | **no** (A) |
| `PE_LOGIC_OP` UNK24 | 5 on a SMALL_MSAA core (state.c:524-525) | 0 (only bit 1 sets 5) | **no** (C) |
| `PE_LOGIC_OP` SINGLE_BUFFER | 2 for a non-linear 32-bit target on a SINGLE_BUFFER core (state.c:536-539) | 1, the linear value, with a SUPER_TILED colour format | **no** (B) |
| `PE_COLOR_FORMAT` | SUPER_TILED, SUPER_TILED_NEW (CACHE128B256BPERLINE), no OVERWRITE for nr_samples > 1 (state.c:280-296) | the same: `0x00102f06` | yes |
| `PE_COLOR_STRIDE` | `level->stride` of a surface padded to 64x64 at width x 2, height x 2 (resource.c:327-328, 389-392) | `0x200` for 64x64 | yes |
| tile status | an MSAA render target always gets a TS, compressed (resource.c:197-199, `!modifier && nr_samples > 1`) and in 256B mode (resource.c:208, 217); `TS_MEM_CONFIG` COLOR_COMPRESSION, `PE_MEM_CONFIG` COLOR_TS_MODE | none | **no** (D) |
| depth for a 4x draw | SUPER (resource.c:583-590 binds DEPTH_STENCIL too), `PE_DEPTH_CONFIG_SUPER_TILED` (state.c:421, 430) | `0x00040711` (clear) / `0x01040711` (draw): not SUPER_TILED; the PE writes it row-linear under SINGLE_BUFFER 1 (backend.rs, measured on 2026-09-30) | **no** (E) |
| viewport, scissor, clip | in target pixels (state.c:585-590, emit.c:440-457; `msaa_xscale` only sizes the surface and the RS window) | target pixels (arm 1); doubled (arm 9, bit 3) | arm 1 yes, bit 3 **not Mesa's** (F) |
| `PS_MSAA_CONFIG` | `~(ALPHA_TO_COVERAGE_MASK \| ALPHA_TO_COVERAGE)`, from the blend state (blend.c:104-108, emit.c:484) | the same word (context init) | yes; bit 2's sample-mask word is **not Mesa's** (F) |
| resolve: flush and stall | `GL_FLUSH_CACHE` COLOR \| DEPTH, stall RA->PE, `TS_MEM_CONFIG` 0 without a TS (rs.c:848-850, 888-889) | the same flush and stall | yes |
| `RS_CONFIG` | src/dst A8R8G8B8, DOWNSAMPLE_X/Y, SOURCE_TILED, dest linear (rs.c:95-102) | `0x000006e6` | yes |
| `RS_SOURCE_STRIDE` | `stride << 2 \| TILING \| TS_MODE(0) \| SUPER_TILED_NEW` (rs.c:104-110) | `0x88000800` | yes |
| `RS_DEST_STRIDE` | linear stride (rs.c:130-135) | `0x100` | yes |
| `RS_WINDOW_SIZE` | in source samples, 128x128 (rs.c:809-810, 143-144) | `0x00800080` | yes |
| `RS_DITHER`, `RS_CLEAR_CONTROL`, `RS_FILL_VALUE`, `RS_EXTRA_CONFIG` | `0xffffffff` x2, disabled, 0, AA 0 / endian 0 (rs.c:154-162, 919-920) | the same | yes |
| `RS_SINGLE_BUFFER` around `RS_KICKER` | set and cleared on a SINGLE_BUFFER core (rs.c:252-256) | the same | yes |

Outside case 16: `emit_resolve` gives a 16-bit 4x target `RS_FORMAT_A4R4G4B4`
whatever its format; Mesa picks R5G6B5 for RGB565 (`translate_rs_format`).
That is a separate defect (case 16 is RGBA8) and is left for its own item.

## 3. Ranked against the symptom

The symptom: no write reaches the scratch, not from the clear quad and not
from the triangle; no hang, no reset, no fault; the resolve runs.

1. **(A) PS input count and temporaries one short in MSAA mode.** Mesa never
   draws a 4x target without `PS_INPUT_COUNT_MSAA` / `..._TEMP_..._MSAA`;
   etnaviv's comment says the multisample config "clobbers PS.INPUT_COUNT"
   (emit.c:354-356), so the hardware wires one more fragment input in 4x
   mode. With the count one short, the coverage input can read as empty and
   the PE writes no sample, from either draw, without a hang. This is the only
   difference that touches both draws through the one thing they share (the
   PS input setup), and it is in bit 1, which the board has never run (arms
   11 and 13 waited on arm 9 passing).
2. **(B) SINGLE_BUFFER 1 with a SUPER_TILED colour format.** Mesa never emits
   this pair. 1 is the linear-surface mode (measured: under it the PE writes
   depth row-linear); a linear write would still leave the scratch non-empty,
   so (B) explains the symptom only if this mode drops 4x writes outright.
   New bit 4 (16) writes Mesa's 2.
3. **(C) PE_LOGIC_OP UNK24 = 5.** Unnamed; Mesa sets it for every 4x target on
   a SMALL_MSAA core. In bit 1 with (A).
4. **(D) No tile status.** Mesa always pairs an MSAA target with a compressed
   TS. Nothing says 4x needs one; the 1x path writes without one. Not a knob:
   it needs a TS buffer and its clear, so it is the next step only if (A)-(C)
   leave the scratch empty.
5. **(E) Depth not supertiled.** Case 16 tests depth ALWAYS and does not write
   it; a depth read the PE cannot place is unlikely to drop colour writes.
6. **(F) Bits 2 and 3 are not Mesa's.** Arm 9 (bit 3) reads the same as arm 1.
   The board arms below leave both out.
7. **The resolve** matches Mesa register by register and only reads the
   scratch, so it cannot leave the scratch empty.

Nothing on the host decides between (A), (B) and (C): the mocks do not
execute a kick. The board does, with three arms of one image build each.

## The board A/B

`CONFIG_GC_GL_TEST_MSAA_BITS` now takes 1..31. Arms (value of
`gc_debug_msaa`):

| arm | bits | adds over arm 1 |
|---|---|---|
| 1 | 0 | nothing (the known failure; re-run as the control of this session) |
| 3 | 0, 1 | (A) + (C) |
| 17 | 0, 4 | (B) |
| 19 | 0, 1, 4 | (A) + (B) + (C): Mesa's whole 4x state on this core, short of the TS |

Build and run, as in `gc-msaa-record.md` (the case record, then pages
0xf0..0xf4; the map pages 0xf8..0xfd only for an arm whose scratch is not
empty):

```sh
W=-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
for bits in 1 3 17 19; do
  tools/cts/imx8mp-gltest-build.sh msaa$bits-$HEAD8 0 $W -DCONFIG_GC_GL_TEST_MSAA=y -DCONFIG_GC_GL_TEST_MSAA_BITS=$bits
done
for bits in 1 3 17 19; do
  img=$IMX8MP_OUT/msaa$bits-$HEAD8/zephyr/zephyr.bin
  SEL=16 tools/board.sh imx8mp run $img <rec>/msaa msaa$bits-case
  for tag in f0 f1 f2 f3 f4; do
    SEL=$((0x1000 + 0x$tag)) tools/board.sh imx8mp run $img <rec>/msaa msaa$bits-p$tag
  done
done
yakogl-tools record msaa <the record= words of one arm's page runs>
```

What each outcome says:

| seen | reading |
|---|---|
| arm 3 passes (inside `0xffff00ff`, outside `0xff000000`, edge `0xff800080` +-2) | (A)/(C) is the cause; make bit 1 the 4x design |
| arm 3 still empty, arm 17 or 19 passes | (B) (with (A)/(C) for 19) |
| arm 3 or 19: scratch non-empty, target wrong | the PE now writes; read the maps (pages 0xf8..0xfd) for tiling or scale |
| all four empty | (D) tile status is next |
| any arm: fault, reset or fence timeout | stop; the record is then FAULT/UNFINISHED, `dmesg` on the Linux side |

Every row records the image's build line and the recovery count, as
CLAUDE.md asks.
