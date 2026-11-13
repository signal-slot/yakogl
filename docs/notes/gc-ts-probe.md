<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: the tile-status probe with the MMU off, and the depth-layout dump — designed for one X2 session

Plan #355, item ts-probe-design (the board probe `gt-tiled-probe` of
`gc-tiled-targets.md` step 0, parts (ii) and (iii)). **Design only: no code was
changed and no board was run.** It gives the two gl_test cases word by word,
what each outcome reads as, what each outcome means for W7 to W9 and W12 of
`gc-tiled-wire.md`, the risk, and the items that build and run it.

Why it is needed. W1 to W5 of `gc-tiled-wire.md` program no tile status (TS),
so session A cannot say whether the TS works on this board. W7 to W9 (TS
buffers, the colour and depth fast clears, the TS-sourced resolve) are gated on
question (iii). W12 (depth textures tiled, a TS-aware CPU resolve) is gated on
question (ii). Mesa never runs the configuration: the back end turns the GPU
MMU off (`driver/rust/src/gpu/vivante/hw.rs:15-16`), and Mesa's FAST_CLEAR gate
counts MC20 (`etnaviv_screen.c:986-992`). `gc-tiled-targets.md` "Risks" says
so.

Sources: `src/vivante/ts.rs` (every TS word and size, from Mesa 26.2.1),
`src/vivante/rs_clear.rs` (the RS fill and the stall/flush shape),
`src/vivante/layout.rs` (the tiled address maps), `src/vivante/regs.rs`,
`tools/vivante/testdata/ts-clear.golden` (Mesa's stream for a TS-cleared
target, `cs_golden` mode 29, README section "A tile-status fast clear"),
`gc-depth-sample.md` ("This back end's surface"), `driver/samples/zephyr/gl_test`
(`main_imx8mp.c`, the case table in `driver/tools/pvrlab/gl_test.c`). Own code
is cited at `7c2af19d`; `main_imx8mp.c` and `regs.rs` are being edited by
items knob-plumb and w1-regs, so line numbers there can drift.

## Answer in short

- **One question per case.** Case 243 `tsprobe` answers (iii): does an RS fill
  of a TS buffer land at the address the back end gives it, and does an RS
  resolve with the TS bound read that TS and return the clear value for every
  tile it marks cleared? Case 244 `dlayout` answers (ii): where does the PE
  put each pixel of a D24S8 surface under SINGLE_BUFFER(2), with and without
  `PE_DEPTH_CONFIG` SUPER_TILED?
- **Neither case draws a picture, and (iii) needs no PE draw at all.** It is an
  RS-only stream into small allocations of the case's own. What it leaves
  unanswered is the PE writing through a TS (W7's `PE_MEM_CONFIG` bits with a
  draw); that is W7's own board check, not this probe's.
- **Each result is a PERF page of the result record, one arm per boot.** Arms
  run from the least to the most risky. A hang stops the session: no retry.
- **The expected values come from the host.** The stream is checked against
  `ts-clear.golden`, the readback against `ts.rs` and `layout.rs`.
- **Cost.** About 250 lines of C and 300 of Rust for the cases, one host item
  before one X2 session of at most 16 short boots (one boot when arm 0 hangs).

## The shared frame of both cases

### Selector, case numbers and pages

- Case numbers: the next free ones at `7c2af19d` (the case table ends at 242):
  **243 `tsprobe`**, **244 `dlayout`**. Take the next free numbers if another
  item has used them; the rest of this note says 243 and 244.
- One image: `IMX8MP_BOARD=x2 tools/cts/imx8mp-gltest-build.sh <name> 243` with
  `GL_TEST_CASES="243,244"` (the sample's `CONFIG_GL_TEST_KNOBS=y`, so
  `CONFIG_YAKOGL_TEST_KNOBS=y`; the hooks below exist only with the crate's
  `test-knobs` feature) and `CONFIG_GC_BOARD_WDOG_SELF_ARM=y`. The image must
  carry `gc_wdog_arm: armed` (`strings`), as every X2 image of plan #328 does.
- The arm comes from the record selector, as case 16's x4 probes and the
  KICK_DIFF pages do (`main_imx8mp.c` `msaa_page_tag`, `kick_diff_tag`;
  `gc_result.h`, "kind PERF"): selector `GC_RESULT_PERF_SEL | tag` (`0x1000 |
  tag`) runs the case alone with that arm and ends the run with that page as a
  PERF record, w0's low byte = the tag, w1 to w3 the page's words. Without a
  selector the case runs arm 0 and ends with its CASE record. **Tags
  `0xd0..0xdf`** (the tags in use at `7c2af19d` are `0xe0..0xef` and
  `0xf0..0xfd`); the host item greps the tree for a clash before taking them.
- `SEL` is the runner's way to hand the selector over (`tools/board.sh x2 run`;
  the dtpu session ran `SEL=0`). The probe does not change `tools/board.sh`
  or `board-stage.sh` (plan #355: those are ordered after #351).
- The CASE record alone is not enough. The dtpu session found that a CASE
  record keeps only the first failing check (`gc-depth-sample.md`, "The X2
  session of 2026-11-11"). The pages carry the whole answer, and the CASE
  record keeps PASS (the arm ran to the end, whatever it found) or FAIL (a
  step could not run).

### Progress, hang and watchdog (plan #323's rules)

- The case writes `gc_result_progress(0x200 | arm << 4 | step)` before each
  kick (`gc_result.h:426`; step 1 = after set-up, 2 = the control resolve,
  3 = the TS fill, 4 = the TS-sourced resolve, 5 = the pattern resolve, 6 = the
  readback). A record the CPU cannot overwrite after a wedge is then the last
  one written: the watchdog's reset leaves it, and it names the arm and the
  kick.
- The image's supervisor thread (`main_imx8mp.c` `supervisor`) feeds the
  WDOG1 self-arm with `gc_sample_service_wdog` and resets the SoC after
  `CONFIG_GC_GL_TEST_DEADLINE_SECONDS`. The probe takes under a second a boot,
  so the image sets the deadline to **60**. The supervisor then writes CASE
  `UNFINISHED` with the case number and stage; if the CPU is wedged too (a bus
  hang) the hardware watchdog resets and the last PROGRESS stays. Both are a
  verdict `hang`.
- A kick that does not complete shows as a fence timeout (2 s, `s_hang_case`
  in `main_imx8mp.c`). The case checks `gc_render_counters` after every wait;
  on a timeout it writes its page with outcome `HANG` (below) and **runs no
  further kick** of that boot.
- `NETBOOT_NO_WATCHDOG=1` is never set. If the board does not come back within
  the runner's limit: stop, release the board, report that it needs a person,
  no retry (the rule of plans #323 and #328).
- **The picture value.** The cases leave the window as found (no picture, no
  `PICTURE` or `FINGERPRINT` record). The value the page carries to compare
  two boots and two commits is a CRC-32 of the 16 KiB readback (the case's
  `crc32` over `D`), so a rerun is compared by number, not by eye.

### The hook behind the case (test-knobs)

The gl_test case calls two `extern "C"` functions, both `#[cfg(feature =
"test-knobs")]` in a new `src/vivante/ts_probe.rs`, in the style of
`gc_debug_msaa_scratch` (`src/vivante/backend.rs`):

- `gc_debug_ts_probe(arm: u32, out: *mut u32) -> i32`: runs the arm's steps
  and fills `out[0..3]` with the page's words; returns 0 when it ran to the
  end, a negative number for a step that could not run.
- `gc_debug_depth_dump(mode: u32, out: *mut u32) -> i32`: draws case 244's
  ramp under the layout `mode` and fills the page words.

A new Cargo feature or knob joins `tools/check.sh`'s lists in the same commit
(CLAUDE.md). The probe is not a knob of the GL core: it submits through the
back end's own kick and fence wait (the path of `emit_rs_clear`'s frame), so a
hang surfaces as the fence timeout above and not as a CPU spin.

## Part (iii): case 243 `tsprobe`

### Memory

All from the back end's allocator (the carve-out the GC already uses; no
cache maintenance, as plan #328 settled for the carve-out). `A` is an
allocation of `size + 2 * 0x1000` bytes with the object in the middle, so a
base written wrongly lands in the guard and is seen.

| object | size | alignment | start contents |
|---|---|---|---|
| `S` the colour surface, 64x64 A8R8G8B8 | 16384 (`ts::padded_dims(64, 64, false)` = 64x64; the same for SUPER_TILED) | 0x1000 | `B(i) = 0xa5000000 \| i` for dword i of the allocation, written by the CPU |
| `T` the TS buffer | 256 (`ts::ts_size(64, 64, 4, st)`, both layouts) | 0x1000 | zero ("not cleared") |
| `D` the linear destination | 16384 | 0x1000 | `0xdeadbeef` |
| guards, 0x1000 each side of each | | | `0xa5a5a5a5` |

`C = 0xff4080bf`, Mesa's clear word for (0.25, 0.5, 0.75, 1.0) in A8R8G8B8
(`ts::color_clear_words(Argb8888, ..)`, golden `TS_COLOR_CLEAR_VALUE`). No
pixel of `B` equals `C`.

### Arms (one per boot, in this order)

| tag | arm | surface | TS mode | `TS_MEM_CONFIG` in the resolve | why this order |
|---|---|---|---|---|---|
| `0xd0` | 0 | TILED | 128B (mode 0) | `0x00000002` (`COLOR_FAST_CLEAR`) | what W7/W8 would program, no compression; the PE's TILED write is proven (gl_test 44) |
| `0xd1` | 1 | TILED | 128B | `0x00000022` (+ `COLOR_AUTO_DISABLE`) with `TS_COLOR_AUTO_DISABLE_COUNT` = `ts::auto_disable_count(64, 64)` = 256 | W8's exact words (`etnaviv_rs.c:382-387`) |
| `0xd2` | 2 | TILED | 256B (mode 1: `RS_SOURCE_STRIDE` bit 29) | `0x00000382` (Mesa's resolve word: fast clear, compression bit 7, format 3) | the shim row's mode and compression (W8, "Compression, an open point") |
| `0xd3` | 3 | SUPER_TILED | 128B | `0x00000002` | the super-tiled RS read has not worked here (#8); last, and its control tells the two failures apart |

Arm 0 answers the gate. Arms 1 to 3 are asked only when arm 0 gave a clean
answer (below), and each is its own boot. An arm that hangs ends the session.

### The steps of one arm

Every step is one kick, waited on, with `gc_result_progress` before it. `S`,
`T`, `D` are the GPU addresses of the objects (the allocator's `gpu`; with the
MMU off they are the addresses the back end already hands every other
register). All streams are `LOAD_STATE` of one register unless a count is
shown; `hdr(reg, n)` = `0x08000000 | n << 16 | reg >> 2` (`regs.rs`
`load_state`); an even-count load is followed by `NOP` = `0x18000000`
(`rs_clear::PAD`).

**Step 2: K0, the control resolve (no TS).** Cleans up the question "does the
RS read this surface at all?" before any TS word is blamed.

| word | register | value |
|---|---|---|
| `0x08010e03` `0x00000003` | `GL_FLUSH_CACHE` | COLOR \| DEPTH |
| `0x08010e02` `0x00000705`, `0x08010f00` `0x00000705` | `GL_SEMAPHORE_TOKEN`, `GL_STALL_TOKEN` | RA → PE |
| `0x08010594` `0x00000001` | `TS_FLUSH_CACHE` | 1 |
| `0x08010595` `0x00000000` | `TS_MEM_CONFIG` | 0 (TS off) |
| `0x08010581` | `RS_CONFIG` | `0x00000686 \| 0` for TILED arms; the source is tiled, A8R8G8B8 both ways, no `SWAP_RB` (a bit copy: `D` gets `S`'s dwords as they are) |
| `0x08010583` | `RS_SOURCE_STRIDE` | TILED: `0x80000400` (`TILING`, stride 64 × 4 B × 4); SUPER_TILED (arm 3): `0x88000400` (+ `SUPER_TILED_NEW`); TS mode bit 29 = 0 |
| `0x08010585` | `RS_DEST_STRIDE` | `0x00000100` |
| `0x080105b0` | `RS_PIPE_SOURCE_ADDR[0]` | `S` |
| `0x080105b8` | `RS_PIPE_DEST_ADDR[0]` | `D` |
| `0x080205c0` + pad | `RS_PIPE_OFFSET[0..1]` | 0, 0 |
| `0x08010588` | `RS_WINDOW_SIZE` | `0x00400040` (64 × 64) |
| `0x0802058c` + pad | `RS_DITHER[0..1]` | `0xffffffff` ×2 |
| `0x0805058f` | `RS_CLEAR_CONTROL`, then the four fill values | 0 ×5 |
| `0x080105a8` | `RS_EXTRA_CONFIG` | 0 |
| `0x080105ae`, `0x08010580`, `0x080105ae` | `RS_SINGLE_BUFFER` 1, `RS_KICKER` `0xbeebbeeb`, `RS_SINGLE_BUFFER` 0 | |

This block (from `RS_CONFIG` to the last `RS_SINGLE_BUFFER`) is exactly
`ts-clear.golden`'s resolve block (the last RS block of the golden) apart from
the values named in the "golden diff" below, and it is the block
`gc-tiled-wire.md` W2's one emitter writes. `D` is then read by the CPU.

**Step 3: K1, the TS fill.** `ts::ts_clear_job(ts::ts_layer_stride(16384,
mode))` = stride `0x40`, 16 wide, `align(256 / 0x40, 4)` = 4 high, fill
`0x11111111`, `RS_CLEAR_CONTROL` = `ts::rs_clear_control(0xffff)` =
`0x0001ffff`.

| word | register | value |
|---|---|---|
| `0x08010e03` `0x00000003`, the stall pair (as above) | flush and stall | |
| `0x08010594` `0x00000001` | `TS_FLUSH_CACHE` | |
| `0x08010581` | `RS_CONFIG` | `0x00004606` (A8R8G8B8 → A8R8G8B8, `DEST_TILED`) |
| `0x08010583` | `RS_SOURCE_STRIDE` | 0 |
| `0x08010585` | `RS_DEST_STRIDE` | `0x00000100` (`0x40 × 4`) |
| `0x080105b8` | `RS_PIPE_DEST_ADDR[0]` | `T` |
| `0x080205c0` + pad | `RS_PIPE_OFFSET[0..1]` | 0, 0 |
| `0x08010588` | `RS_WINDOW_SIZE` | `0x00040010` (16 × 4) |
| `0x0802058c` + pad | `RS_DITHER[0..1]` | `0xffffffff` ×2 |
| `0x0805058f` | `RS_CLEAR_CONTROL`, 4 fill values | `0x0001ffff`, `0x11111111`, 0, 0, 0 |
| `0x080105a8`, then the three `RS_SINGLE_BUFFER`/`RS_KICKER` loads | | `RS_EXTRA_CONFIG` 0, as K0 |

This is the golden's first RS fill block (the colour TS fill) with `T` for
its address. The CPU then reads `T`.

**Step 4: K2, the TS-sourced resolve.** `D` is first set to `0xdeadbeef`
again. Stream as K0 with the TS words in the middle:

| word | register | value |
|---|---|---|
| (K0's flush and stall) | | |
| `0x08010594` `0x00000001` | `TS_FLUSH_CACHE` | |
| `0x08010595` | `TS_MEM_CONFIG` | the arm's word (table above) |
| `0x08010596` | `TS_COLOR_STATUS_BASE` | `T` |
| `0x08010597` | `TS_COLOR_SURFACE_BASE` | `S` |
| `0x08010598` | `TS_COLOR_CLEAR_VALUE` | `C` |
| `0x080105af` | `TS_COLOR_CLEAR_VALUE_EXT` | `C` |
| arm 1 only: `0x0801059d` | `TS_COLOR_AUTO_DISABLE_COUNT` | 256 |
| (K0's RS block) | | `RS_SOURCE_STRIDE` bit 29 = 1 for arm 2 (`0xa0000400 \| ..`) |

**Step 5: K3, the pattern resolve.** The CPU writes `T` as 64 bytes of `0x01`
(low nibble 1 = cleared, high nibble 0 = not cleared) and 192 bytes of 0, `D`
is reset, and K2's stream is submitted again. This reads which TS nibble
covers which tile, and in which order, with one more kick.

### The golden diff

The test of the host item compares the generated words with `ts-clear.golden`.
The golden is the shim's GC7000UL row: a 64x64 super-tiled colour surface with
compression (mode 1, 256B). The words that differ on purpose, and nothing else
may:

| word | golden | the probe |
|---|---|---|
| the addresses (`RS_PIPE_*_ADDR`, `TS_COLOR_*_BASE`) | `0xffff6000` ... | `S`, `T`, `D` |
| fill block: all | | equal, including `0x00040010` and `0x11111111` |
| `TS_MEM_CONFIG` of the resolve | `0x00000382` | arms 0, 1, 3: `0x2`, `0x22`, `0x2`; arm 2: `0x382` |
| `RS_CONFIG` of the resolve | `0x20000686` | `0x00000686` (no `SWAP_RB`: the clear word is the pixel as stored) |
| `RS_SOURCE_STRIDE` of the resolve | `0xa8000400` | TILED `0x80000400` (arm 2 `0xa0000400`); SUPER_TILED `0x88000400` |
| `RS_DEST_STRIDE`, `RS_WINDOW_SIZE`, `RS_EXTRA_CONFIG`, clear control | `0x100`, `0x00400040`, 0, 0 | equal |

**A point for W2 and W8 found while reading the golden.** `gc-tiled-wire.md`
(W8 and the register list in W1) names `RS_EXTRA_CONFIG` `TS_MODE` (bit 24) for
the TS-sourced resolve. Mesa's stream puts the source's TS mode in
`RS_SOURCE_STRIDE` bit 29 (`0xa8000400`, `state_3d.xml.h:1371-1373`) and
leaves `RS_EXTRA_CONFIG` at 0. The golden is the authority, so the probe
follows it; it is the same bit 0 in mode 128B, so arms 0, 1 and 3 do not
depend on the answer. Arm 2 does. W2's emitter should follow the golden.

### Expected readback and the outcome code

`detile(B)` is `layout::layout_to_linear` of `S`'s dwords (TILED, or
SUPER_TILED with `SUPER_ORDER` `Drm`; arm 3 also computes it with
`SuperOrder::Interleaved`, `layout.rs`'s second candidate). Page word w1's low
nibble is the **first** line below that holds:

| code | name | readback | reads as |
|---|---|---|---|
| `F` | `HANG` | a fence timeout in any step (no later kick runs) | the TS words (or the RS with them) wedge the GPU |
| `1` | `GUARD` | a guard dword changed, or `S` changed in K1/K2 | an address or window is wrong, a write left its object; the session stops after the page |
| `2` | `NO_CONTROL` | K0's `D` ≠ `detile(B)` | the RS does not read this surface layout here (arm 3: the #8 super-tiled read again, independent of the TS). The arm says nothing about the TS |
| `3` | `FILL_MISS` | after K1, `T[0..64]` ≠ `0x11111111` | the RS fill did not land at `T` with the MMU off. **This is the physical-address question** |
| `4` | `YES` | K2's `D` = `C` in all 4096 dwords, `S` unchanged | the RS reads the TS at `T`, honours the cleared code, and returns the register's clear value |
| `5` | `IGNORED` | K2's `D` = `detile(B)` | the RS reads the surface and ignores the TS (the register, the mode word or the enable has no effect in this configuration) |
| `6` | `NO_WRITE` | K2's `D` is still `0xdeadbeef` | the resolve with TS bound did not write |
| `7` | `MIXED` | otherwise | partial; the counts and the first difference are in the page, see below |

K3 does not enter the code; its words go in the next page's data, and are
read only when K2 was `YES`.

Page words (the record's w1 to w3, `gl_test_tsprobe_page(tag, w)`):

| tag | w1 | w2 | w3 |
|---|---|---|---|
| `0xd0..0xd3` | `code` \| `arm << 4` \| `steps done << 8` \| fence timeouts `<< 16` | CRC-32 of K2's `D` (or of the last `D` read) | `count(D == C) << 16 \| count(D == detile(B))`, both of 4096 |
| `0xd4..0xd7` | K3 (arm = tag − `0xd4`; absent unless K2 was `YES`): the row-0 map: bit k = 1 when dword `8k` of row 0 of `D` is `C` (k = 0..7) \| `nibble_order << 8` (1 = low nibble first) | CRC-32 of K3's `D` | `count(D == C) << 16 \| count(D == detile(B))` |
| `0xd8..0xdb` | arm = tag − `0xd8`: the first dword of K2's `D` that is neither `C` nor `detile(B)`: index `<< 16 \| low half of the value read` (`0xffffffff` when there is none) | the value expected there (`C`) | the first index of `T` that is not `0x11111111` after K1 (`0xffff` when none) `<< 16 \| ` count of changed guard dwords (low 16 bits) |

Redaction: the records under `records/x2/` hold no addresses, UART paths or
credentials (CLAUDE.md, "Do not touch"). The pages carry indices, counts and
values only; addresses are never in a page.

**What K3 expects.** In TILED with a 64-wide surface, a 128B granule is two
4x4 tiles, 8 pixels by 4 rows (`layout::offset`). Low nibble first, the cleared
granules are the even ones: in row 0, pixels `x` in `[0,8)`, `[16,24)`,
`[32,40)`, `[48,56)` are `C` and the others are `B`, so the row-0 map is
`0b01010101` (`0x55`) with `nibble_order` 1. High nibble first gives `0xaa`.
In arm 2 (256B granules) the runs are 16 wide, map bits `0x33`/`0xcc` over
8-pixel steps. For arm 3 the expected map is computed from
`layout::offset_in(SuperTiled, order, ..)` for both orders, and the page says
which order matched (`nibble_order` bit 1 = `Interleaved`); neither means the
map is neither candidate, and the CRC is kept for the host to decode.

### What each outcome means for W7 to W9 and W12

| arm 0 | then | W7 | W8 | W9 | W12 |
|---|---|---|---|---|---|
| `YES` | arms 1 to 3 asked, each once | go, 128B mode, TILED | go: fast clear and TS-sourced resolve as `gc-tiled-wire.md` writes them (with the `RS_SOURCE_STRIDE` correction above) | goes with W8 and W3; depth needs its own fast-clear check in W9's board check (the probe's colour answer is not the depth answer: `TS_DEPTH_*` use the same RS and the same registers' pattern, so it is likely, not proven) | W12b (the TS-aware resolve) is possible; W12a still needs (ii) |
| `YES` and arm 1 `YES` | | `AUTO_DISABLE` goes in W8 as written | | | |
| `YES` and arm 1 not `YES` | | | W8 programs no `AUTO_DISABLE` bits or count | | |
| arm 0 `YES`, arm 2 `YES` and arm 0 not | | W7 uses mode 256B and the compression bits | W8's "open point" is settled for the shim row | | |
| `IGNORED` | arm 2 (Mesa's word) is the one retry; arms 1 and 3 are skipped | stays off | stays off | stays off | W12b stays off, the CPU depth resolve stays (`gc-depth-sample.md` settles whether it can go another way) |
| `NO_WRITE` or `FILL_MISS` | arms 1 to 3 skipped | stays off: the TS cannot be used with the MMU off in the form Mesa uses | stays off | stays off | W12b stays off |
| `NO_CONTROL` | the arm says nothing; re-run only the TILED arm | | | | |
| `GUARD` | stop: the page says the address is wrong; do not ask more | design bug in the case (an allocation or window), host item | | | |
| `HANG` | stop, release, report that the board needs a person | stays off; "TS with the MMU off wedges the GPU" goes to `gc-tiled-wire.md` as a finding | | | |

W6 (the knob on by default) and W10/W11 (direct sampling of tiled targets) do
not depend on (iii). The sampler-side TS (`TS_SAMPLER_*`, W10 and later) is a
separate question that (iii) does not answer even on `YES`: the texture unit
reading a TS with the MMU off needs its own check in W10's board check.

## Part (ii): case 244 `dlayout`, the D24S8 surface under SINGLE_BUFFER(2)

`gc-depth-sample.md` established that today's PE writes the D24S8 surface
row-linear, with `PE_LOGIC_OP` SINGLE_BUFFER(1) and no
`GC_PE_DEPTH_SUPER_TILED`. Mesa's depth surfaces under SINGLE_BUFFER(2) are
super-tiled. No board has shown what the PE writes in the second case, or in
the pair "SINGLE_BUFFER(2), no SUPER_TILED".

### The draw

A 64x64 depth-only draw into a framebuffer with a D24S8 renderbuffer and a
dummy colour target (Mesa's, `etnaviv_state.c:397-407`; `ensure_no_colour_surface`,
`backend.rs:785-797` is the back end's dummy), `GL_ALWAYS`, depth writes on,
depth cleared to 1.0 by a quad first. One quad (two triangles in one plane)
covering the target, with window depth `z(x, y) = (x + 64 y) / 8192` over the
target, so `z` stays in `[0, 0.5)`. Every pixel then has a **unique** depth,
and neighbouring pixels differ by at least `2^-13` of the range (2048 units
of 2^24), far above the interpolator's error (below 2^-20).

The case decodes a surface dword `d` by `z24 = d >> 8` and matches it to the
pixel whose expected value is nearest: `expect(x, y)` is the plane at the
pixel centre, `((x + 0.5) + 64 (y + 0.5)) / 8192`, rounded to 24 bits; a match
is within 1024 units. The decode is a pure function in the host test, so the
board decodes what the host verified.

### Layout modes (one per boot)

| tag (page A) | mode | `PE_LOGIC_OP` SINGLE_BUFFER | `PE_DEPTH_CONFIG` SUPER_TILED (`0x04000000`) | question |
|---|---|---|---|---|
| `0xc0` | 0 | 2 | off | what does the PE write under SB(2) alone? |
| `0xc1` | 1 | 2 | on | Mesa's pair: is it `layout::offset(SuperTiled)` and which order? |
| `0xc2` | 2 | 1 | on | gc-depth-sample's "(E) not tried under SINGLE_BUFFER 1" |
| `0xc3` | 3 | 1 | off | control: today's draw, must read row-linear (`stride = align(w, 64) * 4`) |

Mode 3 runs first: if the control does not read as today's layout, the
decoding is wrong and the other three pages mean nothing. The hook is `gc_debug_depth_dump(mode, out)`; it needs a
new test knob that overrides the two fields for a depth-only draw
(`single_buffer()` and the `PE_DEPTH_CONFIG` writer, `backend.rs:573-598`,
`:1500-1512`), default off. The hook reads the surface through the CPU view
after `finish` (the carve-out is non-cacheable; `gc-depth-sample.md`).

### The page

For each mode the case locates, by the decoding above, 12 reference pixels
and records each one's **dword offset in the surface** (16 bits): the pixels
(1,0), (2,0), (4,0), (8,0), (0,1), (0,4), (0,8), (16,0), (0,16), (32,0),
(0,32), (63,63). 12 sixteen-bit values are 6 words. Pages A and B each
spend one word on the code and on the CRC, so they have only 3 free words
between them (A w2, A w3, B w2); 12-bit packing would not fit (4.5 words
against 3). The dump is therefore **three pages a mode**, each page three
words (w1 to w3). Offsets are dword offsets, 16 bits, `0xffff` when the
reference was not found:

| tag | w1 | w2 | w3 |
|---|---|---|---|
| page A, `0xc0 + m` | `code` \| `mode << 4` \| fence timeouts `<< 8` \| `matched << 16`, where `matched` is how many of the 4096 ids were found exactly once | (1,0) \| (2,0) `<< 16` | (4,0) \| (8,0) `<< 16` |
| page B, `0xc4 + m` | CRC-32 of the 16 KiB surface | (0,1) \| (0,4) `<< 16` | the surface's first dword that is not the clear value: index `<< 16`, its z byte |
| page C, `0xc8 + m` | (0,8) \| (16,0) `<< 16` | (0,16) \| (32,0) `<< 16` | (0,32) \| (63,63) `<< 16` |

The 12 references live at: (1,0) and (2,0) in page A w2 (low, high);
(4,0) and (8,0) in A w3; (0,1) and (0,4) in B w2; (0,8) and (16,0) in
C w1; (0,16) and (32,0) in C w2; (0,32) and (63,63) in C w3.

What the Rust twin must match (the later `src/vivante/layout.rs` dump
decode): the same reference order, dword offsets, and `0xffff` for not
found. Offset 2k is the low half and 2k+1 the high half of word k; words
0 and 1 are A w2 and w3, word 2 is B w2, words 3 to 5 are C w1 to w3.

Tags `0xc0..0xcb` are case 244's (page C is `0xc8..0xcb`), `0xd0..0xd7`
are case 243's; the host item greps for a clash before taking them.
The codes below are the low nibble of page A's w1.

| code | name | reads as |
|---|---|---|
| `F` | `HANG` | as part (iii) |
| `1` | `NO_WRITE` | the surface is still the clear value everywhere (the PE wrote nowhere the CPU can see) |
| `2` | `ROW_LINEAR` | every id found at `y * stride/4 + x` with `stride = align(64, 64) * 4`: today's layout |
| `3` | `TILED4` | every id at `layout::offset(Tiled, ..)` |
| `4` | `SUPER_DRM` | every id at `layout::offset_in(SuperTiled, Drm, ..)` |
| `5` | `SUPER_INTERLEAVED` | every id at `layout::offset_in(SuperTiled, Interleaved, ..)` |
| `6` | `OTHER` | each id found once but at none of the four maps; the 12 offsets and the CRC let the host derive the map |
| `7` | `PARTIAL` | `matched` < 4096 (ids missing or twice): the PE wrote less, or overlapped |

### What each outcome means

- **Mode 3 not `ROW_LINEAR`**: the decoding or the hook is wrong; repair on
  the host, run again.
- **Mode 0 (SB(2), no SUPER_TILED)**: `ROW_LINEAR` means SB(2) alone changes
  nothing for the depth write, and a PE-private or sampled depth surface may
  stay linear under a tiled colour target (W3 has nothing more to do for
  layout; the SB(2) of W4's draws costs W5 nothing). `TILED4` or another map
  means a draw into a tiled shadow needs the depth in that layout, and
  `ds_cache::Surface` must carry it (W3 already carries a layout).
- **Mode 1 (Mesa's pair)**: `SUPER_DRM` or `SUPER_INTERLEAVED` is the layout
  `layout.rs` must use for every super-tiled surface (it settles
  `layout::SUPER_ORDER`, and the `SuperTiled` colour arm 3 of case 243 reads
  against it), and makes W12a possible: `resolve_depth` and `seed_depth` read
  through `layout::offset_in`. `OTHER` or `PARTIAL`: W12a stays off and the
  12 offsets go to a layout item; the CPU depth resolve keeps the row-linear
  surface.
- **Mode 2**: only matters when mode 1 is `NO_WRITE` or `PARTIAL`: it says
  whether SINGLE_BUFFER(2) is what the super-tiled depth write needs.
- **For W5 (early depth into a tiled shadow)**: this case does not answer
  whether the RA's early depth agrees with a super-tiled depth surface under
  SB(2). Session A answers that, with the knob (`gc-tiled-wire.md`,
  "What this note does not settle").
- **For `gc-depth-sample.md`**: it is the second half of the missing check:
  route 1 ("render depth SUPER_TILED to match Mesa") needs mode 1 to read as
  a known map, and then the TPU read of that map is a case 222 variant.

## The risk

- **A hang.** TS registers with the MMU off may wedge the GPU or the bus.
  Cover: the fence wait gives up after 2 s and the case stops; the supervisor
  thread resets the SoC at the 60 s deadline; the WDOG1 self-arm (`gc_wdog_arm:
  armed`) resets the SoC if the CPU is wedged. Nothing is installed or
  persisted; the board is left as found. A hang ends the session and is a
  finding for `gc-tiled-wire.md`, not a thing to retry. The board has
  survived raw RS jobs before (`gc_clear`, the 4x scratch probes).
- **A wrong address.** An RS job writes to the address it is given. The
  windows are fixed small (the TS fill is 256 bytes, the resolve 16 KiB), the
  objects sit in the guarded allocations above, and nothing is computed from
  runtime data. A write into a guard is the `GUARD` code, and the session
  stops on it.
- **A misread outcome.** The two controls (K0 of every arm, mode 3 of case 244)
  exist so that "the RS cannot read a super-tiled surface" or "the decoding is
  wrong" is never read as "the TS does not work". The TS-fill check (code 3)
  separates "the fill did not land" from "the resolve did not honour it".
- **What the probe does not prove.** It proves the RS fill and the RS read of
  the TS on tiles it marks cleared. It does not prove the PE updating the TS
  on a draw into a cleared tile (needed by W8), the TS compression, or the
  sampler's TS (W10). Each is in the board check of the item that needs it.
- **Cost.** The host item is about 4 h (see below), the X2 session an hour
  of board time with the images built in advance.

## The items to build and run it

1. **ts-probe-host** (host; after w1-regs and rs-interp; `src/vivante/ts_probe.rs`, a
   line in `mod.rs`, `tests/gc_ts_probe.rs`, `tools/check.sh`'s feature list;
   about 3 h).
   - `ts_probe.rs`: the stream builders of K0 to K3 as pure functions of
     (`S`, `T`, `D`, arm) over `ts.rs`, `rs_clear.rs`'s `Coalesce` and the
     W1 registers; the `test-knobs` hooks; the classification as a pure
     function of the readbacks.
   - Tests: the stream against `ts-clear.golden` with exactly the diff above;
     the classification on synthetic readbacks (one per outcome code); K3's
     expected map against `layout.rs` (a unit test on the CPU functions); the
     RS interpreter of item rs-interp, extended with Mesa's TS semantics, runs
     K1 and K2 and gives `YES` (self-consistency of the design, not a claim
     about the board).
   - Only the tests it adds: `cargo test --test gc_ts_probe` with the header's
     feature sets, `cargo fmt --check`.
2. **depth-layout-host** (host; independent of 1; `src/vivante/ts_probe.rs` or
   a sibling, the one test knob in `backend.rs` kept to a few lines because
   #335 and #310 edit that file; about 2 h).
   - The override knob for SINGLE_BUFFER and `PE_DEPTH_CONFIG` SUPER_TILED of
     a depth-only draw; `gc_debug_depth_dump`; the ramp and its decode.
   - Tests: the four modes' streams differ in exactly those two fields;
     decode of a surface filled through `layout::offset_in` returns the
     matching code for each of the four maps.
3. **probe-cases-build** (after 1 and 2 and after knob-plumb has merged, since it
   edits `gl_test/src/main_imx8mp.c` and `Kconfig`; `driver/tools/pvrlab/gl_test.c`
   cases 243 and 244, their names in the case table, `gl_test_tsprobe_page`,
   the selector decoding in `main_imx8mp.c`, `GC_GL_TEST_DEADLINE_SECONDS`
   60 in the image's config; about 2 h). Item x2-gltest-build (ready) is the
   model for the host build check (`west build` for `armadillo_x2`; `strings`
   for `gc_wdog_arm: armed`; `tools/board.sh --dry-run x2 run`).
4. **x2-ts-probe-session** (one X2 session, `target:x2` and `target:x2-console`;
   the console only reads, and no character is sent to it).
   - First, reachability from the leased worker (the cause of the 2026-11-11
     stop: ping by name, ssh with the persisted key), before any jump.
   - The image built from an `origin/main` commit with items 1 to 3, carrying
     `gc_wdog_arm: armed`; `tools/board.sh x2 run` (kexec-dev), `SEL` =
     `0x1000 | tag`. Order: `0xd0`; if its code is `YES`, `0xd4` (the K3 page);
     then `0xd2`, `0xd1`, `0xd3` as the table above allows; then case 244:
     `0xc3` (the control), `0xc0`, `0xc1`, `0xc2`, and the B page of every
     mode (`0xc4..0xc7`) and its C page
     (`0xc8..0xcb`). The whole session is at most 16 boots; a `HANG` or
     `GUARD` stops it. Never `NETBOOT_NO_WATCHDOG=1`; no eMMC boot partition, no U-Boot environment,
     no swupdate; nothing installed or persisted.
   - The record goes under `records/x2/<YYYYMMDD-HHMM>-tsprobe-<sha8>/`
     (gitignored, redacted). The session adds a "Result" section to this note
     with the commit, the `board.sh` verdict lines, the record words of every
     page, and the decoded outcome per arm and per mode, and gives the comment
     texts for the issues. X2 figures never go into `tools/cts/RESULTS.md`,
     `STATUS.md` or `README.md`.
5. **w7-gate-update** (docs; after 4): `gc-tiled-wire.md` W7 to W9 and W12 get
   their gate answered (go, go with the arm-2 mode, or stay off), "What this
   note does not settle" loses its two bullets, and W2 is told the
   `RS_SOURCE_STRIDE` TS-mode correction. This note's design is not edited
   by item 5: the Result section is.

Order: 1 and 2 in parallel, then 3, then 4, then 5. Nothing here touches
`tools/board.sh`, `board-stage.sh` or any default; none of it moves #351's
stage-1 commit.
