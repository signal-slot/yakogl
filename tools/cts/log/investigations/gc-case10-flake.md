<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP: gl_test cases 10 and 2 pass or fail at random (#9, #25)

**Status (2026-10-11): open.** The first part of the board recipe ran twice
(section "Board results"). The failure rate fell from 14 of 30 to about 1 in
10 or less, and neither knob (page alignment, no pool reuse) removes it, so
(b) and (c) are not separated. The second part needs the K1 knob, which does
not exist yet. Tracked as #9 and #25 in `docs/HANDOFF.md`.

Host investigation, 2026-10-08, on integration `fdff1147`. Item `gc-case10`
of plan #2. No code is changed. The open row is tools/cts/RESULTS.md:930
(`gc-nondeterminism`).

## Summary

- **(c) history-dependent pool placement: ruled out for the colour target.**
  Case 10 draws into the harness's default surface. gl_test creates that
  surface once, before the first case, so it has the same address in every
  run, alone or after the 8 regression cases: carveout + `0x6600C0`
  (`0x586600C0` on the Verdin). History can still move the other buffers
  (shader code, vertex buffers, textures). But case 10 also fails as the
  first case after boot (8 of 70). Then no history exists and every
  address is fixed.
- **(b) 64-byte alignment: unlikely to be the trigger.** The target is 64-byte
  aligned and not 256-byte aligned (`0xC0` mod 256), so each 256-byte row
  spans two 256-byte PE cache lines. Mesa's buffers start on a page, but
  Mesa renders to mip levels at 64-byte offsets inside them, so 64 bytes is
  the PE alignment Mesa relies on too. And the address is the same in every
  run, while the failure is random. So (b) can at most be a condition that a timing
  race needs. It cannot be the random part.
- **(a) the linear PE path: the most likely place, as a race.** No placement
  differs between a passing and a failing first-after-boot run, so the cause
  must be timing. Mesa also renders linear window buffers on this core, but
  with tile status and fast clears. Both failing cases have one shape that
  Mesa never sends to this core: a full-surface PE quad clear, then a
  per-draw colour flush,
  then a draw of 1 to 4 pixels that partly writes the same 256-byte lines.
  All of this is in one linear target, in one kick. The lost pixels read back
  as the clear colour. The best fitting story: the flush still writes back
  the clear's dirty lines when the tiny draw's write reaches memory, and the
  older line lands last. **This is not proven.** The host cannot see GPU
  timing.
- **Proposed fix** (not applied; `src/vivante/` is held by `gc-npot-bit`):
  do what etnaviv does. Do not flush the colour cache between two draws to
  the same target in one frame. As a second step, clear with the resolve
  engine (RS fill) and not a PE quad. Each step comes first as an A/B knob,
  not as a product change.
- **Board recipe:** 30 boots with interleaved arms, all from existing knobs.
  It confirms (b) or (c) if either one is the cause. 10 optional boots more
  try to catch case 2's first failing check, which the shape story predicts.
  The (a) sub-hypothesis
  needs one new knob; its recipe is in the second part below.

## What was read, and a limit

Read in this repository: gl_test cases 10 and 2, the gc_gl_test harness, the
GC back end's render-target state, flushes and pool, the Zephyr OSAL, and the
earlier sync audit (src/vivante/VIVANTE_NOTES.md:1152-1269, commit
`8409419`).

**Mesa: only one file was read directly.** `git clone` was refused, and the
web fetch of gitlab.freedesktop.org got its bot wall ("Access Denied"). One
file came through `curl`: `src/gallium/drivers/etnaviv/etnaviv_resource.c`
from Mesa **main** as of 2026-10-08 (commit not recorded). Lines marked
"(main)" are from that file. The other Mesa line numbers are the ones this
repository already recorded from Mesa 26.2.1 (each with the place it is
recorded). Where no line was recorded, the Mesa function is named and marked
"(line not re-checked)". To re-check them, put a Mesa 26.2.1 checkout at
`<deps>/mesa` and read the named functions.

## The facts the host can show

### F1. Case 10's target is the same buffer in every run

- `gl_test_run` creates the harness context with its 64x64 default surface
  before any case runs: driver/tools/pvrlab/gl_test.c:20231-20236.
- The list walker runs every case on that context. It makes a new one only
  after cases 17, 18 and 21 (gl_test.c:20279-20303). Cases 1-6, 8-10 do not
  do that.
- `gl_context_create` allocates the surface: `surface_bytes(64, 64, 4)` =
  16384 bytes, 256-byte pitch (src/gl/context.rs:1561-1580).
- gc_gl_test initialises the render service and then calls `gl_test_run` with
  the selected case or list. Nothing allocates from the pool in between
  (driver/samples/zephyr/gc_gl_test/src/main.c:392, 405-437).

So the single-case run (`SEL=10`) and the list run (`SEL=512`) make the same
allocations up to the default surface.

### F2. Where it lands: carveout + 0x6600C0, 64-byte aligned

The pool is a bump allocator from `GC_OFF_POOL` = 0x60000
(driver/src/vivante/gc_internal.h:55, gc_render.c:516). Nothing is freed
before the surface, so each block starts at the bump, rounded up to its
alignment (gc_render_pool.h:128-152). Every allocation is 64-byte aligned
(src/osal/zephyr_gc.c:67, 130-139). In order:

| allocation | where | size | offset |
|---|---|---|---|
| 4 x frame command buffer | src/vivante/backend.rs:1916 (`GC_CMD_WORDS` 128 Ki words, :77) | 512 KiB each | 0x060000 ... |
| 4 x frame arena | backend.rs:1917 (`GC_ARENA_BYTES`, :78) | 1 MiB each | ... up to 0x660000 |
| clear VS code | backend.rs:1922, 1868-1871 (`usc_dw` 4, src/vivante/internal.rs:55) | 16 -> 64 | 0x660000 |
| clear PS code | the same (internal.rs:118) | 16 -> 64 | 0x660040 |
| black texel | backend.rs:1926 | 4 -> 64 | 0x660080 |
| **default surface** | context.rs:1574 | 16384 | **0x6600C0** |

The Verdin carveout is at 0x58000000
(driver/boards/toradex/verdin_imx8mp/verdin_imx8mp_mimx8ml8_a53.dts:58-59),
and it is identity-mapped (gc_render.c:256). So the target is at GPU address
**0x586600C0** in every run.

With `CONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN=y`, a block of 4 KiB or more is
page-aligned (zephyr_gc.c:130-139). The surface then moves to **0x58661000**.
With `CONFIG_YAKOGL_GC_POOL_NO_REUSE=y` it does **not** move: that knob changes
only what happens after a free (zephyr_gc.c:259-296), and nothing is freed
before the surface.

How this was checked: a throwaway host program (not committed) included the
real `driver/src/vivante/gc_render_pool.h`, called `gc_pool_init(0x60000)`
and then `gc_pool_alloc` with the sizes and alignments above, in this order.
It printed:

    default surface 64x64 RGBA8  size 16384 align   64 -> +0x06600c0 (mod 256 = 0xc0)
    default surface 64x64 RGBA8  size 16384 align 4096 -> +0x0661000 (page-align arm)

The allocation order is read from the code, not traced on a board. No board
record shows this address yet (see the end of the board recipe).

### F3. The pixel engine's view of that target

- Linear, with `PE_LOGIC_OP.SINGLE_BUFFER(1)`. No SUPER_TILED unless the draw
  is multisampled (backend.rs:1471-1484, 1546-1552).
- `OVERWRITE` is set when there is no blending and the colour mask is full.
  That holds for the clear quad and for case 10's points (backend.rs:1478-1480).
- Stride 256 bytes (backend.rs:1485-1501). With the base at `0xC0` mod 256,
  every row covers the last 64 bytes of one 256-byte line and the first
  192 bytes of the next.
- The tile status is off on every draw: `PE_MEM_CONFIG` 0 and `TS_MEM_CONFIG`
  0 (backend.rs:1558-1559).
- Before **every** draw, the clear quad included, the back end flushes
  COLOR|DEPTH|TEXTURE|SHADER_L1, then TEXTUREVS, then does SEMAPHORE+STALL
  RA->PE and `TS_FLUSH_CACHE` (backend.rs:1597-1602, called from :1610).

### F4. The two failing cases have the same shape

- Case 10, frame 5 (gl_test.c:3852-3897): `glClear`, one `glDrawArrays(GL_POINTS)`
  of 1, 3 and 5 px points, `glFinish`, read-back. Check 23 is the 1 px
  point's centre (gl_test.c:3887). It fails with got `0xff000000`, the clear
  colour (VIVANTE_NOTES.md:1154-1157). In frame 4 that pixel was red (the
  line strip starts at (8,8), gl_test.c:3838-3840). So the clear of frame 5
  reached memory, and the point drawn over it later in the same kick did
  not. The KICK_DIFF record agrees: the 1 px and 3 px points (4 rows) are
  missing, and the 5 px point, the last vertex, is there
  (tools/cts/STATUS.md:110).
- Case 2 (gl_test.c:1513-1568) has the same shape four times: `glClear`, then
  a quad of 2x2 px (check 22, gl_test.c:1541), then 1x1 px (check 23,
  :1543), then 1x1 px again (checks 24 and 25, :1561, :1568), each followed
  by `glFinish` and a read-back. No record has said which check of case 2
  fails: the set's record names only the case (RESULTS.md:16948-16975).

## Each hypothesis

### (a) The linear PE target

For:

- Mesa does render into linear targets on this core, but not the way we do.
  Its default layout is tiled or super-tiled (etnaviv_resource.c:658,
  674-683 (main)). A resource bound LINEAR or SCANOUT becomes linear
  (:698-703 (main)), and a linear resource is render-compatible only on a
  core with LINEAR_PE (:104-106 (main); the assert is etnaviv_state.c:268-269,
  recorded at VIVANTE_NOTES.md:1148). So a linear window buffer is a normal
  Mesa path on this core. This corrects VIVANTE_NOTES.md:1204-1207, which
  says Mesa uses linear PE only behind a debug flag.
- But such a Mesa target gets **tile status**. On a FAST_CLEAR core, a
  render-compatible linear target whose stride is a multiple of the TS tile
  size gets a TS buffer (:108-113, 154-181, 605-612 (main)), and with
  CACHE128B256BPERLINE a 128 B or 256 B TS mode (:216-232 (main)). Its clear
  is then a fast clear in the tile status, and the memory is written only
  when the tiles are resolved. Our target has no tile status (F3), and our
  clear is a PE quad that writes every byte (VIVANTE_NOTES.md:1192). Mesa
  also does not flush the colour cache between draws
  (VIVANTE_NOTES.md:1189). We flush before every draw (F3). So the order "PE
  writes the whole surface, colour flush, PE writes a few pixels of the same
  lines, all in one kick" never comes from Mesa on this core. Our two
  failing cases are made of exactly this order (F4).
- The lost writes are at the start of the draw after the flush. The 5 px
  point, the last primitive, survives. A write-back of the clear's lines that
  is still running when the next draw starts writing would lose exactly the
  first writes. Case 2's tiny quads are all "first writes". In a full-screen
  draw after a clear, only the first pixels could be hit, and the checks look
  at the centre, which is drawn later. This fits the fact that only
  tiny-draw checks fail.
- A timing race fits the numbers. With F1 and F2, a first-after-boot run has no
  placement that can differ between pass and fail, and still fails 8 of 70.

Against:

- After the flush we also do SEMAPHORE+STALL RA->PE (F3). This is the stall
  Mesa uses after its own colour flush before an RS kick (`etna_try_rs_blit`,
  etnaviv_rs.c, line not re-checked; VIVANTE_NOTES.md:1192). If "PE idle"
  includes the flush's memory writes, the race cannot happen. Nothing public
  says whether it does.
- The 12-step audit found no missing sync step (VIVANTE_NOTES.md:1166-1198).
  But that audit compares our steps with etnaviv's steps. It cannot find a
  hazard that only our extra steps create (the per-draw flush, the quad clear).

Judgement: **the most likely place.** Within (a), the leading candidate is
the per-draw colour flush between the clear quad and a small draw (call it
a1). Not proven.

### (b) 64-byte target alignment

For:

- The target is 64-byte aligned, at `0xC0` mod 256 (F2). This core's PE cache
  lines are 256 bytes (CACHE128B256BPERLINE, hwdb ChipID 0x7000 rev 0x6204,
  RESULTS.md:8423-8430). Every Mesa target is a GEM buffer from `etna_bo_new`,
  so it is page-aligned (etnaviv_bo.c / the kernel's etnaviv_gem.c, line not
  re-checked; zephyr/Kconfig:166-179 records the same).
- In the a1 story, a row that spans two lines makes the clear's write-back
  and the point's write share a line with the previous row. Alignment may
  change how likely the race is.

Against:

- The address is the same in every run (F1, F2), and the result is not. So
  alignment cannot be the random part.
- Mesa itself renders to 64-byte offsets. It places each mip level at a
  64-byte boundary inside the buffer, "to be able to render to them"
  (`ETNA_PE_ALIGNMENT`, etnaviv_resource.c:351-352 (main)). So 64 bytes is
  the PE alignment Mesa relies on, and level 1 and up of a render target are
  usually not 256-byte aligned there either.
- The 2026-10 color_clear A/B ran the 48-case chunk with the page-align knob.
  Case 10 flipped both with and without the knob (RESULTS.md:8818-8830).
  That was 1 run per arm, so it is weak.

Judgement: **unlikely as the cause.** It may be a precondition. The page arm
of the recipe tests it directly, because it moves this target to 0x58661000.

### (c) History-dependent pool placement

For:

- Placement after the first free depends on the cases before (best fit over
  a free list, gc_render_pool.h:91-126). There is an open reuse defect
  (gc_render.c:281-296). The failure rate does change with history: 14 of 30
  after 8 cases, 8 of 70 alone (VIVANTE_NOTES.md:1154-1156).

Against:

- The colour target does not move with history (F1, F2).
- Alone, after boot, every address is fixed, and the case still fails
  (8/70). So history can at most change the rate. Other things also differ
  after 8 cases: GPU and DRAM timing, which of the 4 frame slots a kick
  uses, and where the case's shaders and vertex data land.

Judgement: **not the cause.** It may only change the rate. The no-reuse arm
tests whether the non-target buffers matter: the vertex staging, the shader
code (GC_HEAP_SHADER, backend.rs:1871) and the index buffer.

### State a previous case can leave

- **TS enabled:** no. Every draw writes `PE_MEM_CONFIG` 0 and `TS_MEM_CONFIG`
  0 (backend.rs:1558-1559).
- **PE cache dirty:** no. Every draw starts with a COLOR|DEPTH flush and an
  RA->PE stall (backend.rs:1597-1602). Every submit ends with the kernel-style
  flush, FE->PE stall and EVENT FROM_PE (VIVANTE_NOTES.md:1194).
- **Stale RS state:** not for case 10. The RS is used only by the RS copies
  (backend.rs:769-790, 826-855), each with its own flush and
  `RS_SINGLE_BUFFER` set and cleared. Case 10 uses no texture, and the clear
  is a quad, so no RS kick runs in it.
- **Stale 3D state:** the whole state block is emitted for every draw
  (backend.rs:1593-1596). The one tracked exception,
  `GL_VARYING_COMPONENT_USE`, is written again after any draw that marked it
  (backend.rs:1900, `component_use_needs_write`).

### One more, on the CPU side (low)

After the fence, the CPU reads the pixels through the Normal non-cacheable
view. `gc_render_wait` polls the volatile `fence_done`. There is no
load-acquire or `dmb` between that read and the pixel reads
(driver/src/vivante/gc_render.c:677-680, 921-931). ARMv8 lets a later load of
Normal memory pass an earlier one. The Cortex-A53 is in-order and a function
call chain lies in between, so this is unlikely. It costs one barrier to
remove. The "late read" step of the second part catches it, together with
"the event came before the write-back".

## Proposed fix (not applied)

`src/vivante/` is held by `gc-npot-bit`. Do this after it merges, each step as
an A/B knob first:

1. **K1, etnaviv's flush rule.** In `emit_flushes` (backend.rs:1597-1602), do
   not flush COLOR|DEPTH before a draw when the frame's previous draw wrote the
   same colour and depth targets and no texture of this draw is one of them.
   Keep the TEXTURE/SHADER_L1 flush and the stalls. This is etnaviv's
   behaviour (VIVANTE_NOTES.md:1189). The host test
   `a_frame_flushes_and_stalls_as_etnaviv_does` (backend_tests.rs:4012-4091)
   pins today's stream. It changes with K1 on.
2. **K2, etnaviv's clear.** Clear a whole unmasked, unscissored target with an
   RS fill, as `etna_blit_clear_color_rs` does. Use etnaviv's flush and
   RA->PE stall before the next draw (VIVANTE_NOTES.md:1192). Keep the quad
   for masked or scissored clears. This is the clear Mesa uses for a target
   with no tile status. Mesa's own linear window buffer goes further: it
   gets tile status and a fast clear (etnaviv_resource.c:104-113, 605-612
   (main)). Tile status for our targets is a larger change and a later step;
   it would also cut the clear's memory writes (the frame-floor item,
   imx8mp-floor, may want it for speed).
3. A `dmb ishld` (or a load-acquire) after `fence_done` is seen in
   `gc_render_wait` (driver/src/vivante/, not held). This is cheap and right
   either way.

If K1 makes case 10 and case 2 stable, make K1 the default and remove the knob.

## Board recipe

### First part: one session, 30 boots, existing knobs only

This is the HANDOFF i.MX8MP step 5, with three
changes: the arms are interleaved, the reading of the no-reuse arm is fixed
(it does not move the target, F2), and a fourth list catches case 2's check.

Two corrections from the board sessions are in the commands below. The `;`
inside the list is written `\;`: with a bare `;` CMake reads the value as a
list and drops the separators, and the image got
`CONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10102"`. And every image needs
`-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`: `imx8mp-gltest-build.sh` does not set it,
and without the `gc_wdog_arm: armed` marker `tools/board.sh` refuses the image
(rc=3, "would not arm WDOG1").

```sh
# a detached checkout of the integration tip, with ZEPHYR_WORKSPACE,
# ZEPHYR_VENV, ZEPHYR_SDK_INSTALL_DIR and IMX8MP_OUT exported
# (tools/cts/imx8mp-env.sh), BOARD_BOOT=uboot-mmc
LIST='-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10\;10\;2"'  # SEL 512, 513, 514
W=-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
tools/cts/imx8mp-gltest-build.sh gc10-base   0 "$LIST" $W
tools/cts/imx8mp-gltest-build.sh gc10-page   0 "$LIST" $W -DCONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN=y
tools/cts/imx8mp-gltest-build.sh gc10-reuse0 0 "$LIST" $W -DCONFIG_YAKOGL_GC_POOL_NO_REUSE=y
for i in 1 2 3 4 5 6 7 8 9 10; do
  for arm in base page reuse0; do          # interleaved: drift hits every arm alike
    SEL=512 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
      "$IMX8MP_OUT/gc10-$arm/zephyr/zephyr.bin" build/gc10 "$arm-$i"
  done
done
```

Each run ends with the counters record (kind SWEEP, tag 0xff;
driver/zephyr/lib/board/imx8mp/gc_result.h:195-214). The `record=` words on
the verdict line:

| words | meaning |
|---|---|
| `594b07ff,00000000,0000ff00,0000077e` | all 9 cases passed |
| `594b07ff,00000000,0000ff00,0000037e` | case 10 failed (#9) |
| `594b07ff,00000000,0000ff00,0000077a` | case 2 failed (#25) |
| `594b07ff,00000000,0000ff00,0000037a` | both failed |
| w1 not `00000000` | lost work, a reset or a fence timeout: not a reading, run it again |
| w2 low 16 bits not `ff00` | a case left a GL error unchecked: note it; the w3 reading still holds |

Expected rate on base: about 5 of 10 runs fail case 10 (14/30 before). If the
rate does not change, an arm has 0 of 10 with chance 0.53^10 = 0.2 %.

| base | page | reuse0 | reading | next |
|---|---|---|---|---|
| ~5/10 | 0/10 | ~5/10 | (b): alignment is the cause, or the precondition | make 4 KiB alignment the default for allocations of 4 KiB or more; run the second part on page to see if a1 remains |
| ~5/10 | ~5/10 | 0/10 | (c) through a non-target buffer (shaders, vertex data) | the reuse defect above `gc_render_pool_reset`; the target is not the cause |
| ~5/10 | 0/10 | 0/10 | both change the rate | fix (b) first, then re-measure reuse |
| ~5/10 | ~5/10 | ~5/10 | **predicted**: neither; (a) is left | the second part |
| 0/10 | - | - | the rate fell in this session | run 20 more base before reading the others |

Case 2's first failing check (optional, 10 more boots on base). Run one at a
time, `SEL=2 ... gc10-base ... c2-$i`. The record is kind CASE
(gc_result.h:157-167):

- pass: `594b01xx,0002ff01,<checks made>,00000000`;
- predicted failure: `594b01xx,0002NN02,ff000000,<want>` with NN = `16`, `17`,
  `18` or `19` (checks 22-25, the tiny quads, gl_test.c:1541-1568), got the
  clear colour.

Alone after boot, case 2 may fail rarely (its rate alone is unknown), so 0 of
10 says nothing. A failure at check 1-21 contradicts the "tiny draw after a
clear" shape, and then a1 is less likely.

### Second part: a1, after K1 exists

This needs the K1 knob (proposed fix 1). It does not exist yet. With K1 as a
Kconfig switch, for example `CONFIG_YAKOGL_GC_FLUSH_AS_ETNAVIV` (the name is
free), build one more image and interleave it with base:

```sh
tools/cts/imx8mp-gltest-build.sh gc10-k1 0 "$LIST" $W -DCONFIG_YAKOGL_GC_FLUSH_AS_ETNAVIV=y
for i in $(seq 1 10); do for arm in base k1; do
  SEL=512 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
    "$IMX8MP_OUT/gc10-$arm/zephyr/zephyr.bin" build/gc10 "$arm-k1-$i"
done; done
```

- k1 0/10 with `...,0000077e` and base about 5/10: **a1 confirmed.** Make K1
  the default.
- k1 about 5/10: a1 is wrong. Next is the late-read instrument: read the
  missed pixel again about 10 ms after the fence, with a `dmb` first. If
  the pixel is then there, the event or the CPU read came too early. If it
  is never there, the PE lost the write.

F2 (the target address) is derived on the host only. No gc_gl_test record
carries the address today. The dEQP sample's `CONFIG_DEQP_SNVS_ADDRESS`
(driver/samples/zephyr/deqp/Kconfig:113-125) records the low 12 bits of the
default target's address, but that is the dEQP surface, a different buffer.
If K1 is added, add the same field to gc_gl_test's counters record from
`yakogl_debug_default_target_gpu()` (src/gl/context.rs:1128-1137). Expected
low 12 bits: `0c0` on base and reuse0, `000` on page.

## Board results

All on the Verdin i.MX8M Plus, `BOARD_BOOT=uboot-mmc`, integration images
(not landing proofs). Every run below had w1 = `00000000` (no lost work, no
reset, no fence timeout), so every one is a reading. The records were kept
on the workers, not committed. The session notes were
`tools/cts/log/board-sessions/imx8mp-plan1.md` and `imx8mp-gc-case10.md`;
git history keeps them.

| session | image | arm and list | case 10 (#9) failed | case 2 (#25) failed |
|---|---|---|---|---|
| 2026-10-08, plan #1 step 5 | `b20e4d11` | base, `1-6,8-10` | 3 / 40 (`base-20`, `base-23`, `base-35`) | 1 / 40 (`base-13`) |
| | | page | 1 / 10 (`page-3`) | 0 / 10 |
| | | reuse0 | 1 / 10 (`reuse0-4`) | 1 / 10 (`reuse0-6`) |
| 2026-10-08, this recipe | `0175bac1` | base, interleaved | 0 / 10 | 0 / 10 |
| | | page | 1 / 10 (`page-9`, `0000037e`) | 0 / 10 |
| | | reuse0 | 0 / 10 | 0 / 10 |
| | | base, case 2 alone (`SEL=2`) | - | 0 / 10 (each `594b010e,0002ff01,0000001d,00000000`, 29 checks) |

The plan #1 session ran its base arm 30 more times when the first 10 gave
0 / 10 (the table's last row), so its base is 40 boots in all. The earlier
measurement was 14 / 30.

The default gl_test image (48 cases, the record names only the first
failure) also shows case 10:

| image | runs | first failure |
|---|---|---|
| `b20e4d11` (2026-10-08) | 3 | case 16 in all three (#8's skip, counted as a failure by the set walker then) |
| `334d4e65` (2026-10-09) | 3 | case 16 once; case 10 twice (45 / 48) |
| `32778b57` (2026-10-10, main) | 3 | case 10 once (45 / 48); case 16 twice (46 / 48) |

Reading: the rate fell a lot since 14 / 30, but the flake is still there on
every arm. At these counts the arms do not separate, so (b) and (c) are
neither confirmed nor ruled out by the board. The closest row of the table
is "neither; (a) is left", but at a rate far below the one that row assumes.
The fall itself needs a cause: a re-measure, or a bisect, against the commit
the 14 / 30 was taken at. One change against the earlier measurement is the
watchdog self-arm option in the image; it is a candidate.

## Handoff for the items after this one

- Do not spend board time on (c) for the colour target. It is the same
  buffer at the same address in every gl_test run (F1, F2).
- The first part needs only knobs that exist on integration. The second
  part needs K1 in src/vivante/backend.rs `emit_flushes`, after `gc-npot-bit`
  merges.
- Mesa line numbers here come from what the repository already recorded
  for Mesa 26.2.1. Check the ones marked "(line not re-checked)" when a Mesa
  checkout is at hand.
