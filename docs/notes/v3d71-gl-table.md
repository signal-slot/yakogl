<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1 GL table: the P9 figures and how a build selects it

Design note of ROADMAP stage 5, step P9 (`docs/notes/pre-study-rpi5.md`,
section 8, the P9 row at line 612): the `src/v3d` back-end table for the
Raspberry Pi 5 (BCM2712 D0, V3D 7.1.10), done when the first six gl_test
cases run on the board. It is written before the code so that the items that
build it (the C job builder, the Rust table, the Zephyr side, the image) can
run side by side. Nothing here is a board measurement; each figure is read
from a file and cited. Two prefixes:

- `M:` is `third_party/mesa-26.2.1/src/broadcom/` (vendored: `cle/`,
  `common/`, `clif/`, `qpu/` only). `X:` is `M:cle/v3d_packet.xml`.
- Mesa's `src/gallium/drivers/v3d/` (`v3dx_rcl.c`, `v3dx_tfu.c`,
  `v3dx_draw.c`) and `common/v3d_tfu.h` are **not** vendored. Where this note
  cites them it cites the line the repository's own notes and C sources already
  took from the Mesa 26.2.1 tree (named in each row), never a fresh reading.

Read first: `docs/notes/pi4-v3d-bringup.md` ("gl_test on the V3D", about
line 322), `driver/docs/rpi5-v3d71-facts.md` (sections 1, 2, 12, 14),
`docs/notes/pre-study-rpi5.md` sections 3, 6 and 9,
`docs/online-shaderc/v3d-backend.md` ("M6: the GL core on the V3D", line 1379)
and `docs/notes/v3d71-backend.md`.

## 1. What exists and what P9 adds

| Piece | State on `origin/main` (8ba6e448) |
|---|---|
| The 4.2 table | `yakogl_backend_v3d` (`src/v3d/backend.rs:7204`), chosen by `g_backend` for `gpu-v3d` (`src/backend.rs:1786-1788`); the module is cfg-gated by `gpu-v3d` (`src/v3d/mod.rs:11`) |
| 4.2 job builder | `driver/src/v3d/v3d_job.c`, `v3d_frame.c` (one generation per translation unit, `V3D_CL_VER` 42 by default: `driver/src/v3d/v3d_frame.h:25-26`, `v3d_cl.h:85-88`) |
| 7.1 packets and frame | `v3d_cl.h` with `-DV3D_CL_VER=71`; the 7.1 triangle frame `driver/src/v3d/v3d_frame71_tri.c` (declared `v3d_frame.h:270-313`); texture state `v3d_tex71.*`; TFU `v3d_tfu71.*` |
| 7.1 compiler | the `*71.rs` files under `src/shaderc/online/v3d/`; host checks `tools/v3d/check71.d/`, `tests/v3d71_*.rs` |
| 7.1 on the board | `v3d_triangle` PASS with Mesa's shaders and ours (`tools/cts/RESULTS.md`, "V3D 7.1: Raspberry Pi 5 on Zephyr") |
| `src/v3d/**` for 7.1 | none: that is P9 |

How the Pi 4 did the same step (`pi4-v3d-bringup.md`, "gl_test on the V3D",
line 322): one `v3d_gl_test` image, the list 5, 58, 59, 66, 67, 102 on one
context, a clear quad (`COLOR_WRITE_MASKS`) for scissored, stencil-masked and
colour-masked clears; **4 of 6** at first, 59 and 67 refused by name. The
cases in `driver/tools/pvrlab/gl_test.c`: 5 scissor (`:2338`), 58 `glClear`
honours the scissor (`:8069`), 59 `glClear` honours the write masks (`:8153`),
66 `glClear` is not bounded by the viewport (`:9197`), 67 what a framebuffer
object does not have (`:8675`), 102 a colour clear under a partial
`glColorMask` (`:22814`); dispatch `:23870`, `:23945-23946`.

## 2. The P9 items, one by one

| # | 4.2 (what the Pi 4 table does) | 7.1.10 (what the new table does) | Citation |
|---|---|---|---|
| 1 | GL Shader State Record: **36 bytes** (last field, Coordinate Shader Uniforms Address, at byte 32). Carries the flags "Coordinate/Vertex shader has separate input and output VPM blocks" (bits 10, 11) and "Address of default attribute values" (byte 8). The builder sets `address_of_default_attribute_values = j->defaults` | **32 bytes** (last field at byte 28). Those three fields are gone; adds `Never defer FEP depth writes` (bit 20). Each stage's code, NaN, thread and uniforms-address fields sit 4 bytes lower than in 4.2 (Fragment code at bit 67, not 99). "No prim pack" and the others keep their bit numbers | 4.2: `X:1197-1254`; 7.1: `X:1256-1311`; builder: `driver/src/v3d/v3d_job.c:953` |
| 2 | no variant | **Draw-index variant**, `GL Shader State Record Draw Index`, "exclusive to 2712D0 (V3D 7.1.10)": the same 32 bytes and the same address layout as item 1, but the ID flags are renumbered to add `cs_basevertex` (5), `cs_drawindex` (6), `vs_basevertex` (10), `vs_drawindex` (11): vertex/instance/base-instance of the vertex stage move to 7, 8, 9, and every flag from "Fragment shader does Z writes" on moves up (Z writes 12, early-z 13, real pixel centre W 15 ... Never defer FEP depth writes 23; the plain 7.1 record has them at 8, 9, 12 ... 20). Mesa uses it iff `v3d_device_has_draw_index`: `ver > 71 \|\| (ver == 71 && rev >= 10)`. The Pi 5 board is 7.1.10 (IPREV 10), so **the table uses the draw-index record only**; the plain 7.1 record is for 7.1.2 to 7.1.7, which the table refuses (section 4). Our programs read neither `gl_DrawID` nor `gl_BaseVertex`, so the four new bits are 0 (what `v3d_frame71_tri.c` does) | `X:1313-1375` (comment `X:1313`); `M:common/v3d_device_info.h:94-97`; `driver/src/v3d/v3d_frame71_tri.c:296-312`; board identity `driver/docs/rpi5-v3d71-facts.md:539` (IPREV 10, IPIDX 16, compat_rev 2) |
| 3 | A default-attribute-values BO: 16 vec4 slots of `(0,0,0,1.0)` per job, its address in the record | **None.** The record has no field for it, so no BO is allocated, nothing is written and the job has no `defaults` address. Mesa: the BO "is only needed on <= 4.2" | `M:common/v3d_device_info.h:114-121` (`v3d_device_needs_default_attribute_values`: `ver <= 42`); 4.2 builder `driver/src/v3d/v3d_job.c:379-384`, `v3d_frame.c:683`; `v3d_job.h:399` (`defaults`); 7.1 frame already has none: `v3d_frame71_tri.c:296-312` |
| 4 | at most **4** render targets, framebuffer at most **4096** px | **8** render targets, **7680** px. The table's limit answers (`max_render_targets`, `max_framebuffer_size` equivalents of `state.rs`'s refusals) read the 7.1 values; `Blend Cfg` takes an 8-bit RT mask and the Store packet's "Buffer to Store" has render targets 4 to 7. 4.2's builder refuses `rts > 4` (`mrt4-v3d-pvr.md:51`); the 7.1 builder lifts that to 8 only when a case needs it (P9's six cases use one target) | `M:common/v3d_limits.h:44` (`V3D_MAX_RENDER_TARGETS(ver)` = `ver < 71 ? 4 : 8`) and `:63` (`V3D_MAX_FRAMEBUFFER_SIZE(ver)` = `ver < 71 ? 4096u : 7680u`); `M:common/v3d_device_info.c:82-83`; `X:787` (7.1 Blend Cfg), `X:491-525` (Store, values at 7.1) |
| 5 | The 4.2 tile list clears with `CLEAR_TILE_BUFFERS` (two bytes, flags `clear_all_render_targets` and `clear_z_stencil_buffer`), emitted for a clear-only tile list and for the generic one; the Store packet also has a "Clear buffer being stored" bit | **`Clear Render Targets`** (packet 25, **one byte, no fields**) in place of both; the Store's clear bit is not used. Emitted in the first tile only for the initial clear (`do_double_initial_tile_clear`), and in the generic list when `job->clear_tlb`. Z and stencil clear values come from `Tile Rendering Mode Cfg (ZS Clear Values)` (sub-id 1 at 7.1, 2 at 4.2); the colour clear value moves into `Render Target Part1/2/3` | `X:485` (packet), `X:480-483` (4.2's); the store bit stays in the XML, `X:502`; Mesa `v3dx_rcl.c:331-340` and `:646-655` (the 7.1 branch), `:919-923` (ZS values), as read in `docs/notes/v3d71-clear-frame.md:82,130`; `pre-study-rpi5.md:181`; 4.2 builder `v3d_job.c:1296-1298`, `:1443-1445`, `v3d_frame.c:200` |
| 6 | TFU (mipmaps): format and mip count in `ICFG` and `IOA`, output tiled only, TFU registers at 0x400 | TFU at 0x700 with a **new register `IOC`** (0x71c): `DIMTW` bit 0, `NUMMM` at bit 4, output format at bit 12 (raster 0, linear tile 3), stride at bit 16 (padded height in UIF blocks, or raster stride in texels). `ICFG` keeps only the input side and bit 0 = interrupt on completion; `COEF1..3` are always written; raster output works. `IOC` is passed as `args.v71.ioc` | encoders `driver/src/v3d/v3d_tfu71.h:48-53`, `v3d_tfu71.c:171-181`, struct field `v3d_tfu71.h:71`; register `driver/docs/rpi5-v3d71-facts.md:68,70,186`; Mesa `v3dx_tfu.c:165-202` and `v3d_tfu.h:57-67`, as quoted in `v3d_tfu71.c:80-181`; the 4.2 submit has no `IOC` (`driver/src/v3d/v3d_hw.c:541-552`) |

Items 1 to 3 are one change in the builder: the shader-state step emits
`GL_SHADER_STATE_RECORD_DRAW_INDEX`, reserves 32 bytes, 32-byte aligned
(`v3d_frame71_tri.c:290-293`), and never calls the default-attribute reserve.
Item 5 is one packet and the clear values moved; items 4 and 6 are limit
constants and a submit struct. The six gl_test cases need 1 to 5; 6 is P10's
mipmap path and is written down here because the table has one 7.1 submit
type for both.

## 3. What P10 needs from the 7.1 table (texture state)

Not built in P9; recorded so the table's shape leaves room for it.

| Item | 4.2 | 7.1 | Citation |
|---|---|---|---|
| TMU Config Parameter 2 | `LOD Query` bit 24, bits 25-31 pad | adds `DIM query` (25) and `Write conversion` (26), pad shrinks to 5 bits. `textureSize` and `textureQueryLod` use the new bits; the uniform words 44-47 rule of `notes/v3d-texture-size-words.md` is unchanged in intent | `X:1560-1574` (4.2), `X:1575-1590` (7.1) |
| Texture Shader State | `sRGB` (bit 3), `Reverse Standard Border Color` (5), array stride 26 bits at 32 | `Transfer func` 3 bits at bit 2 (none, sRGB, PQ, HLG, BT.1886 variants), `Reverse` kept, **`R/B swap` at bit 32**, array stride **24 bits at 33** (64-byte units), chroma pointers | 4.2: `X:1592-1632`; 7.1: `X:1633-1707` (`:1625`, `:1627` the 4.2/shared bits; `:1691-1692` stride and swap; `:1697-1699` transfer function) |
| Revision split | one layout | **7.1.2 has no R/B swap bit** and its stride starts at bit 32; Mesa uses the 7.1.5 definition for all. The table therefore refuses revisions below 7.1.5 (section 4) rather than carrying two layouts | `X:1686-1690` |
| Driver structs | `v3d_tex.*` | `v3d_tex71.h:82-102` (`transfer_func`, `r_b_swap`, `reverse`, `array_stride_64`), unsupported-state refusals `v3d_tex71.h:117` | |
| Texture data formats | no BSTC | new values 46 to 63 (BSTC, `rgba10x6`, BT.1886); not needed before ES 3 compressed formats | `pre-study-rpi5.md:193` (`X:271-284`) |
| Mip chain | TFU 4.2 | TFU with `IOC` (item 6). Whether this Pi 5's TFU works at all is unproven: its identity reads `TFU: no` (`tools/v3d/pi5/v3d_ident.txt:3`), section 6 | section 2, 6 |
| Cube map (U8) | non-seamless on 4.2 (`docs/v3d-noseamless-cube.md`) | seamless filtering is the first cube case; U8 stays open until the board answers | `pre-study-rpi5.md` U8 |

## 4. Build selection (decision)

**A cargo feature `gpu-v3d71 = ["gpu-v3d"]`, next to `gpu-v3d` (`Cargo.toml:96`),
and a Zephyr option `CONFIG_YAKOGL_GPU_V3D71`.** The revision is chosen when
the image is built, not by a run-time branch inside the 4.2 table. Pieces:

- **Rust.** New files `src/v3d/ffi71.rs` (the 7.1 FFI to the C job builder) and
  `src/v3d/backend71.rs` with the table **`yakogl_backend_v3d71`**. Because the
  feature implies `gpu-v3d`, `src/v3d/mod.rs:11`'s gate, `src/lib.rs:151-157`'s
  "exactly one GPU feature" check, `src/lib.rs:164-170`'s `shaderc-online` rule
  and the `gpu-v3d` ABI asserts in `src/abi_layout.rs` (`:184`, `:485`, `:708`
  and so on) stay true and unchanged. `state.rs`'s gallium translation and its
  refusals are reused unchanged: the same refusal strings come out on both
  boards.
- **Selection.** Wherever `gpu-v3d`'s table is chosen, a cfg picks the 7.1 one:
  `src/backend.rs:1786-1788` (`g_backend`) becomes two lines,
  `all(feature = "gpu-v3d", not(feature = "gpu-v3d71"))` for
  `yakogl_backend_v3d` and `feature = "gpu-v3d71"` for `yakogl_backend_v3d71`.
  Each hook is a mod line or one cfg, in its own small commit
  (`src/v3d/mod.rs`, `Cargo.toml`, `src/backend.rs`); every 4.2 answer and
  pinned 4.2 fixture stays byte-identical.
- **C.** `driver/src/v3d/v3d_job71.{c,h}`, built with `V3D_CL_VER 71`
  (`v3d_cl.h:85-88`), next to `v3d_frame71_tri.c`. No `#if` inside `v3d_job.c`.
- **Zephyr.** `CONFIG_YAKOGL_GPU_V3D71` beside `CONFIG_YAKOGL_GPU_V3D`
  (`zephyr/Kconfig:64`; the CMake switch is `zephyr/CMakeLists.txt:98-103`,
  `:207`), `src/osal/zephyr_v3d71.c` beside `zephyr_v3d.c` (whose init runs
  `v3d_identify`, `:219`). The 4.2 `zephyr_v3d.c` and its Kconfig dependants
  (`depends on YAKOGL_GPU_V3D`, e.g. `zephyr/Kconfig:243-366`) are not edited:
  the 7.1 symbol selects what it needs itself.
- **Identity check at init.** The 7.1 table's init reads the hub identity
  (`v3d_identify`, `driver/src/v3d/v3d_hw.c:260-275`: `HUB_IDENT1` TVER and REV
  give `ver = 10*TVER + REV`; `HUB_IDENT3` IPREV 15:8 and compat_rev 23:16:
  `rpi5-v3d71-facts.md:41-60`) and **refuses by name** unless `ver == 71`
  **and** `IPREV >= 10`: the message states what it read ("V3D 7.1.x ... this
  table is 7.1.10 (BCM2712 D0) only" or "V3D 4.2: build gpu-v3d"). A 7.1.5 to
  7.1.7 part would use the plain 7.1 record (item 2) and 7.1.2 a different
  texture layout (section 3), so they get a named refusal, not a guess.
  `v3d_identity` as of `v3d_hw.c:268-272` holds `ver`; the 7.1 side adds the
  `IPREV` and compat_rev words it reads from `hub_ident[3]` (`:263`), in new
  7.1 code, not in `v3d_hw.c`.

Why a feature and not a run-time revision answer:

1. **4.2 files untouched.** A run-time choice would put `if rev == 71` into
   `backend.rs` (7,000+ lines), `v3d_job.c` and `zephyr_v3d.c`, which plans
   #376, #382 and #383 are editing, and each such branch is a place a Pi 4
   answer can move.
2. **A 7.1 image never carries a 4.2 decision.** The 4.2 table's limits (4
   targets, 4096 px), its 36-byte record and its default-attribute BO are not
   linked into the 7.1 image, so no code path can take them on 7.1 hardware.
   The reverse holds too.
3. **Pinned fixtures stay valid.** `tests/reference/*.rs`, `tests/fixtures/`
   and `driver/src/v3d/v3d_job.c`'s byte results are compared by the gate;
   they cannot change if the 4.2 code is not touched.
4. **The hardware is not probed twice.** The Pi 4 and the Pi 5 are different
   Zephyr boards (`rpi_4b`, `rpi_5`) with different register maps and
   interrupts (`rpi5-v3d71-facts.md:470-486`); a binary has one of them.

Cost: two tables to keep in step. `state.rs` is shared, so only the job and
submit layers are duplicated, and `tools/check.sh` gets one row per feature.

## 5. Expected results for the six cases

The Pi 4's run, and the refusal strings the shared code produces:

| Case | Pi 4 (`pi4-v3d-bringup.md:322-335`) | Expected on 7.1 and why |
|---|---|---|
| 5 scissor | PASS | pass; scissored draw through the 7.1 job (items 1 to 3, 5) |
| 58 clear with scissor | PASS | pass; the clear quad (a draw) |
| 59 clear with write masks | **refused**: "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in" (`src/v3d/backend.rs:3791`; the host pin `backend_tests.rs:6276`) | **refused, the same string**: `state.rs`/the shared decision, not the 7.1 builder. Stencil kept between jobs is a separate item |
| 66 clear not bounded by viewport | PASS | pass |
| 67 FBO limits | **refused**: "a colour format with fewer than 8 alpha bits" (`src/v3d/state.rs:969`, `backend_tests.rs:3359`: the GL_RGB frame) | **refused, the same string** |
| 102 clear under partial colour mask | PASS (maskclear 0 of 4096 wrong) | pass; the clear quad under a partial mask, and its program compiles (below) |

Two host answers bear on this table, both pinned in the tree:

- **Programs.** Both programs of the six cases compile with our 7.1 online
  compiler, so no case needs Mesa's offline 7.1 binaries: `color_uv` (the
  draws of 5, 58, 59, 67 and 102; 66 links none of its own) and `clear` (the
  float clear quad of all six) are committed as
  `tools/v3d/gltest-shaders71/ours_color_uv.h` and `ours_clear.h` (prefixes
  `OURS71_COLOR_UV_*` and `OURS71_CLEAR_*`), re-compiled byte for byte,
  decoded, verified and run on the 7.1 simulator by
  `tests/v3d71_gltest_shaders.rs`; a program the compiler cannot build would
  be pinned there as a named refusal and there is none
  (`tests/v3d71_gltest_shaders.rs:28-30,63-65`). The clear program's fragment
  shader reads `u_color` as dwords 0 to 3 of its uniform block
  (`OURS71_CLEAR_UNIFORM_U_COLOR_WORD` 0, "frag, 4 dwords": `ours_clear.h:49`;
  the simulator resolves exactly those reads, `tests/v3d71_gltest_shaders.rs:570-585`).
- **Job builder.** `driver/src/v3d/v3d_job71.c` covers what the six shapes
  put on the wire: the scissored clip window (`:408-418`), the 7.1 1/64-pixel
  viewport (`:475-488`), `COLOR_WRITE_MASKS` (`:538-542`), four and eight
  render targets (four and eight in the compared shapes; the builder takes 1
  to `V3D_JOB71_MAX_RTS` = 8, `v3d_job71.h:66`, job71.sh's `multi` builds 8,
  and P9's six cases use one), and D16/D24S8 with Z
  and with a separate stencil (`v3d_job71.c:203-212`; loads and stores
  `:882-925`). Its one gap: a D24S8 buffer loaded or stored Z-only or
  stencil-only picks its buffer from `zs_buffer()` (`:838-844`), which
  answers Z or ZSTENCIL from the format and the presence of a separate
  stencil, not from the load/store side; a separate change to `v3d_job71`
  and `tools/v3d/check71.d/job71.sh` is fixing that. It reaches only the
  stencil frames of 59 and 67, which the shared refusal refuses anyway (the
  two rows above).

Expectation: **4 of 6 pass, 59 and 67 refused by the same names**, as the Pi 4
started. Further refusals are allowed only with a name: a program the 7.1
online compiler cannot emit yet falls back to Mesa's offline 7.1 binaries (as
the Pi 4 started, `v3d_triangle` "Mesa's shaders" before "ours") and a case
that needs a 7.1 packet the builder lacks (an 8-target frame, a non-raster
store) is refused as `V3D 7.1: <what>` from `backend71.rs`, never silently
drawn through the 4.2 path. The result goes to `tools/cts/RESULTS.md`'s Pi 5
row after the board session, not before.

## 6. Open

- `IOC` and the mip path run only in P10; the encoders are checked on the
  host, not on the board. Whether the Pi 5 has a usable TFU is itself open:
  its identity reads `TFU: no` (`tools/v3d/pi5/v3d_ident.txt:3`), HUB_IDENT1
  bit 17 (`WITH_TFU`, `tools/v3d/ident71.py:17,100`), clear in the Pi 5's
  `0x00081117` (`tools/v3d/pi5/v3d_regs.txt:3`) and set in the Pi 4's
  `0x000e1124` where the TFU works (`driver/docs/rpi4-v3d.md:161`). The
  repository does not say what the bit means on 7.x: Linux and Mesa carry a
  full 7.1 TFU submit path (`driver/docs/rpi5-v3d71-facts.md:179,186`) and
  our 4.2 `v3d_tfu_run` refuses the BCM2712 only because its job's register
  block is the 0x400 one, not because the TFU is absent
  (`driver/src/v3d/v3d_hw.c:530-533`). So the TFU is unproven on the Pi 5
  until the `tools/probes/rpi_5/rpi5_tfu` probe (one 7.1 TFU mip-chain job,
  being added) runs on the board: `glGenerateMipmap` on 7.1 goes through the
  TFU if that passes and is otherwise refused by name.
  `tools/cts/RESULTS.md` has no TFU row today, and none is claimed here.
- Seamless cube filtering (U8) and the transfer-function column of section 3
  need the board.

Answered on the host since this section was written: the 7.1 online compiler
does emit the clear quad's program, and `color_uv` with it, so 58, 59, 66
and 102 need no Mesa offline binaries —
`tools/v3d/gltest-shaders71/ours_clear.h` and `ours_color_uv.h`, pinned and
simulated by `tests/v3d71_gltest_shaders.rs`; section 5 carries it case by
case.
