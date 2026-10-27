<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the frame-side refusals, Mesa's design per case

The six refusals of issue #1386 (the NanoPC-T6's `gpu-mali` backend) and how
Mesa 26.2.1's panfrost builds each one on v10: the code it runs, the
descriptor fields and program bytes that differ from what `src/mali` packs
today, where the change lands in our tree, and the `gl_test` case that checks
it. The items `frame-query`, `frame-a2c`, `frame-blendconst`, `frame-msaa-zs`,
`frame-msaa-preload` and `link-varying-order` follow it. It is a reading of
Mesa's source; the **pin on the board** blocks were filled in on 2026-10-21 from
the committed Mesa 25.0.7 recordings (`0dce654a8:tools/mali/board-ref/2026-10-20-*`),
and on 2026-10-23 from gl_test cases 208-211 under the same Mesa
(`0dce654a8:tools/mali/board-ref/2026-10-21-mesa-refs3/`); they say where a fact is
still open. A fact that needs the board
is marked **pin on the board**; the method is the project's usual one (Mesa
on the board first through `tools/mali/mesa-board-ref.sh`, our descriptors
and registers pinned to its dump, the prediction written before the runs).

## Citations

Every Mesa citation is `file:line` in the Mesa 26.2.1 tree (`MESA_PAN_TREE`,
`$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`). Short names for the directories:

| Short | Directory |
| --- | --- |
| `G/` | `src/gallium/drivers/panfrost/` |
| `L/` | `src/panfrost/lib/` |
| `C/` | `src/panfrost/compiler/` (`C/bifrost/` the BIR compiler) |
| `X/` | `src/panfrost/genxml/v10.xml` (`X:line`) |
| `Q/` | `src/mesa/main/`, `S/` `src/mesa/state_tracker/` |
| `GL/` | `src/compiler/glsl/` |

Our own code is cited as `src/...:line` at `origin/main` db5701fb. Mesa
25.0.7 is what the board runs (the `board-ref` dumps); `blend.rs` and
`frame.rs` already note where 26.2.1 differs.

## Where each refusal is raised today

| Refusal | Raised at | Line |
| --- | --- | --- |
| occlusion query | `NOT_BUILT_QUERY`, `check()` | `src/mali/backend.rs:166`, `:1100` |
| alpha-to-coverage | `NOT_BUILT_ALPHA_TO_COVERAGE`, `check()` | `backend.rs:167`, `:1101` |
| blend shader | `NOT_BUILT_BLEND_SHADER`, `check_draw()` | `src/mali/frame.rs:176`, `:1826-1829` |
| 4x with depth/stencil | `NOT_BUILT_SAMPLES`, `check_target()` | `frame.rs:182`, `:1417-1420` |
| preload of a 4x target | `NOT_BUILT_PRELOAD_SAMPLES`, `open_frame()` | `frame.rs:146`, `backend.rs:1865-1867` |
| varyings in different orders | `MALI_LAYOUTS_DIFFER`, `link_shaders_mali()` | `src/shaderc/online/mali/link.rs:33`, `:211` |

`check()` runs before any frame state is touched (`backend.rs:1081-1107`), so
a case meets the first of these it contains. `query_begin`, `query_end` and
`query_result` are `None` in the backend table (`backend.rs:2366-2368`).

## What the draw registers and DCD flags are (common ground)

Mesa 26.2.1 builds a v10 draw's per-draw state in `csf_emit_draw_state`
(`G/pan_csf.c:1433-1673`). The parts the six cases move, in the order of the
code:

- `OQ` (IDVS register 46, a 64-bit address): `pan_csf.c:1500-1504`; `X:919`.
- `BLEND_DESC` (50): the blend descriptor array's address with the count in
  the low bits, `pan_csf.c:1508-1509`.
- `DCD0` (57): `pan_csf.c:1544-1636`, the struct `DCD Flags 0` (`X:2118`),
  built field by field below; `DCD1` (58): `pan_csf.c:1638-1646`.

Our `emit_draw` writes the same registers (`src/mali/frame.rs:2566-2644`),
but its `reg` table (`frame.rs:236-283`) has no `OQ`, and `DCD0` is built with
`occlusion_query: DISABLED` and nothing for coverage (`frame.rs:2611-2632`).

`fs_required` (whether the draw runs a fragment shader at all) is
`panfrost_fs_required`, `G/pan_cmdstream.h:199-235`: side effects, **alpha to
coverage** (210-213: "writing the pixel can modify occlusion query results"),
a written colour target, or shader-written depth/stencil. Ours is `mask != 0`
(`frame.rs:2327`); it needs `|| alpha_to_coverage` for the draw with an empty
colour mask.

The early-z modes come from `pan_earlyzs_get` (`pan_csf.c:1585-1593`,
`L/pan_earlyzs.h:62-67`) whose analysis is `L/pan_earlyzs.c:43-125`:
`best_early_mode` (30-37) gives WEAK_EARLY if the depth/stencil test always
passes, else FORCE_EARLY; alpha-to-coverage makes the *update* FORCE_LATE (50);
`late_coverage` (a coverage write, a discard or alpha-to-coverage, 62) plus
"writes Z/S or an occlusion query is on" makes it late too (71). Our
`frame.rs:2610` has the two best-early cases only.

---

## 1. Occlusion queries (case 196)

**Mesa.**

- GL type to gallium: `GL_ANY_SAMPLES_PASSED` is `OCCLUSION_PREDICATE`,
  `_CONSERVATIVE` is `OCCLUSION_PREDICATE_CONSERVATIVE`, `GL_SAMPLES_PASSED`
  is `OCCLUSION_COUNTER` (`Q/queryobj.c:162-171`). ES 3.0 (what `backend.h`
  offers) only has the first two.
- Create: a buffer of `8 * core_id_range` bytes, zero filled
  (`G/pan_context.c:673-718`, size at 689, zeroes at 711-714);
  `core_id_range = util_last_bit64(shader_present)` (`L/pan_props.c:47-55`).
- Begin: zero the buffer again (`pan_context.c:742-764`, 753-758) and make it
  the context's active query (761). End: `ctx->occlusion_query = NULL`
  (798-810). **Nothing is put in the command stream at begin or end**, and no
  batch is ended: the frame goes on.
- Draw: while a query is active, `OQ` gets the buffer's address
  (`pan_csf.c:1500-1504`; the active test is `G/pan_context.h:576-580`), and
  `DCD0.occlusion_query` is COUNTER for a counter, PREDICATE otherwise
  (`pan_csf.c:1577-1583`; enum `X:422-426`: Disabled 0, Predicate 1, Counter 3;
  field `X:2130`, bits 14-15). The flag is set before the `fs_required`
  branch, so a draw with no fragment shader (colour mask 0) counts too.
- Early-z: `writes_zs || has_oq` goes into `pan_earlyzs_get` (1587), which only
  matters for a shader with a discard, a coverage write or alpha-to-coverage
  (`pan_earlyzs.c:71`). The shaders of case 196 (`link_program(gl_test_vs_src,
  gl_test_fs_src)`, which I take to be a plain colour pair) are not those: **DCD0's kill and update modes do not change.**
- Result: flush the batch that writes the buffer, wait for it, then for a
  counter sum the `core_id_range` qwords, for a predicate read **only**
  `result[0]` (`pan_context.c:839-871`, 852-853, 858-866, 868).

**Descriptor fields that change.** Register `OQ` (r46:r47) written with the
buffer's GPU address on a counted draw; `DCD0.Occlusion Query` (r57 bits
14-15) 0 to 1 or 3. Nothing in the FBD, tiler context, ZSD, blend descriptor
or resource tables.

**Program bytes.** None. Neither stage's program changes.

**Lands in.**

- `src/mali/frame.rs`: `DrawCall` gets the query (address and counter or
  predicate); `mod reg` gets `OQ = 46` (`MALI_IDVS_SR_OQ`, `X:919`); `emit_draw`
  writes it between `HIGH_DEPTH_CLAMP` and `VARY_SIZE` as Mesa does
  (`pan_csf.c:1494-1506`) and sets `occlusion_query` in `dcd0` (`frame.rs:2626`).
  A new `DrawCall` field means every literal changes: `grep -rl 'DrawCall {'
  src tests` (CLAUDE.md, the 2026-10-14 failure).
- `src/mali/backend.rs`: drop `(d.occlusion_query != 0, NOT_BUILT_QUERY)`
  (`:1100`); fill `query_begin`/`query_end`/`query_result` (`:2366-2368`).
  `begin` zeroes a per-handle block of persistent GPU memory (64 handles,
  `GLES_QUERY_HANDLES`), `result` sums the block (the contract wants the
  samples, or any non-zero value). `OsalMaliGpu` (`src/osal/mali.rs:35-44`) has
  no `shader_present`: either extend the query, or size every block for the 64
  possible core ids (512 bytes) and sum all of it; a block the hardware does
  not touch stays zero.
- Tests: `src/mali/backend_tests.rs:870` (the refusal row) turns into a pass
  row; a frame test pins `r46`, `r57` to a Mesa dump taken with
  `0dce654a8:tools/mali/board-ref/2026-10-19-msaa/msaa.c`'s method (a new
  board record).

**Checked by.** gl_test case **196** `occl` (`gl_test.c:17142-17248`): five
queries, q0 hidden, q1 visible, q2 conservative and empty, q3 one query over a
`glFlush` (two kicks), q4 visible then covered later in the same frame; the
picture is checked too. The case first needs the depth path it already has.

**Pin on the board.** Settled by the record
`0dce654a8:tools/mali/board-ref/2026-10-20-query/` (Mesa 25.0.7 on the T6, `mesa/mesa-occl.pandecode`,
`README.md`, and its excerpt `tools/mali/board-dump/query-2026-10-20.txt` in this tree; the dump's `Occlusion` fields are the OQ
register):

- `r57` and `OQ`: **settled.** A counted draw has `MOVE d46, #<buffer>` between
  r45 and r48 (`mesa-occl.pandecode:716`, q0's buffer `0x7FFFFFFC7000`; q1
  `..6000` at 1365, q3 `..4000` at 2014 and again in frame 2 at 3843, q4 `..3000`
  at 4492) and `MOVE32 r57, #0x16183` (721, 1370, 2019, 3848, 4497) against the
  uncounted draws' `#0x12183` (72, 5145): bits 14-15 = 1, **PREDICATE**, and
  nothing else of DCD0 moves (pandecode 683 `Occlusion query: Disabled` against
  1332 `Predicate`; the register dump shows `Occlusion: 7ffffffc7000` at 1242,
  `Occlusion: 0` at 593). An uncounted draw after a counted one leaves d46 as it
  was (5666 still prints `7ffffffc3000` while 5756 says `Disabled`). Only the
  predicate value is recorded: ES 3.0 has no counter, so the COUNTER value
  (3) stays the source's (`X:422-426`) until a desktop-GL run.
- Accumulation across jobs: **settled for PREDICATE** by gl_test case 210
  `occlacc` under Mesa (`0dce654a8:tools/mali/board-ref/2026-10-21-mesa-refs3/`,
  `mesa/mesa-occlacc.pandecode`, four kicks, one dump file each, the query
  buffers dumped after each). q0 (pass, `glFlush`, pass) is counted in both
  kicks (`MOVE d46, #0x7FFFFFFC7000` at 716 and again at 2437, `r57`
  `#0x16183` at 721 and 2442), and its qword 0 reads **1 after the first kick
  and still 1 after the second** (2407-2409, 5348-5350): the hardware
  **stores** the flag, it does not add one job's pass to the other's. q1
  (pass, flush, hidden) reads 1 after its passing kick and still 1 after the
  kick that passes nothing (5356-5358, 8305-8307): a later job does not clear
  an earlier pass. q2 (hidden twice) stays 0 (8313-8315, 10639-10641). Mesa's
  results are 1, 1, 0 (`mesa/mesa-occlacc.stdout:10-12`, verdict PASS at 16).
  So for a predicate a block holding 1 is the answer whatever the job count;
  COUNTER (3) is still unrecorded (ES 3.0 has none), so whether a counter adds
  across jobs stays the source's reading.
- q4: **settled.** Counted, then covered by a later white quad of the same
  frame: result 1 (`mesa/mesa-occl.stdout:11`, buffer `..3000` = 1 after
  frame 2, `tools/mali/board-dump/query-2026-10-20.txt:1442-1448`). Mesa never ends the frame at
  `glEndQuery`, so the count is taken before the later occluder, and
  `query_end` need not submit the frame.
- Which qword: **settled for PREDICATE.** After each frame the hardware wrote
  `1` into **qword 0** of a buffer and nothing else in its 4 KiB
  (`tools/mali/board-dump/query-2026-10-20.txt:1397-1448`; `README.md` "between nooq and occl"). Mesa
  reads only `[0]`, which is what the hardware writes; summing all is still a
  safe superset. Per-core qwords for COUNTER are not recorded.
- `shader_present`: **settled**, `0x50005` (`README.md` line 3, GPU_ID
  `0xa8670005`, `tools/mali/board-dump/query-2026-10-20.txt` header). That is not "4 or more cores
  contiguous": `core_id_range = util_last_bit64(0x50005)` is **19**, so Mesa's
  buffer is `8 * 19` = 152 bytes, and the 512 bytes (64 ids) the plan above sizes a
  block for cover it.

---

## 2. Alpha to coverage (case 198)

**Mesa.**

- State: only when multisampling is on and the first draw buffer is not an
  integer one (`S/st_atom_blend.c:339-347`; our `GlesDraw.alpha_to_coverage`
  is already "on and the target is multisampled", `backend.h:386`).
  `alpha_to_one` is separate and not asked here.
- Fragment shader: **no change.** Every non-blit fragment shader ends with
  `ATEST coverage, alpha, ATEST_DATUM` (`C/pan_nir_lower_fs_outputs.c:110-117`,
  alpha defaults to 1.0; 144-150; `C/bifrost/bifrost_nir.c:930-940`; emitted at
  `C/bifrost/bifrost_compile.c:1633-1644`, FAU special `va_pack.c:83-84`). The
  datum is the hardware's, fed by the DCD flag. Ours already emits the same
  ATEST (coverage r60, the alpha, `atest_datum`): `select` at
  `src/shaderc/online/mali/nir_fs.rs:1840` and `select_highp` at `:2361`.
- Draw: `DCD0.alpha_to_coverage` (bit 22) = the state; `shader_modifies_coverage`
  (bit 20) = `writes_coverage || can_discard || alpha_to_coverage` ("Unlike
  Bifrost, alpha-to-coverage must be included in this identically-named
  flag", `pan_csf.c:1607-1614`; `X:2135-2137`). `Alpha To Coverage Invert`
  (bit 21) is never set by the gallium driver.
- Early-z: `pan_earlyzs_get(..., alpha_to_coverage, ...)` (`pan_csf.c:1586-1590`)
  makes `zs_update_operation` FORCE_LATE (`pan_earlyzs.c:41-50`, a comment
  there says it is empirical on Valhall); `pixel_kill_operation` stays
  `best_early_mode` (no late kill without a Z/S write).
- Forward pixel kill is off: `pan_allow_forward_pixel_to_kill` requires
  `!alpha_to_coverage` (`G/pan_cmdstream.h:97-112`, line 111), and
  `allow_forward_pixel_to_kill` takes it (`pan_csf.c:1595-1596`).
  `overdraw_alpha0/1` do not depend on it (`pan_cmdstream.h:122-139`).
- The draw runs a fragment shader even with an empty colour mask
  (`pan_cmdstream.h:210-213`).

**Descriptor fields that change** (DCD0, r57): `Allow Forward Pixel To Kill`
(bit 0) 1 to 0; `ZS Update Operation` (bits 4-5) WEAK_EARLY(2) or FORCE_EARLY(0)
to FORCE_LATE(3, `X:370-374`); `Shader Modifies Coverage` (20) and `Alpha To
Coverage` (22) set. Nothing else: the FBD is the 4x one that exists
(`0dce654a8:tools/mali/board-ref/2026-10-19-msaa`). Derived prediction for a 4x draw with the depth
test off and a colour mask of RGB: the same draw's r57 with bit 0 clear (it
already is: the mask reads the destination), bits 4-5 = 3, bits 20 and 22 set.

**Program bytes.** None.

**Lands in.** `frame.rs`: a `DrawCall.alpha_to_coverage` (the literals grep as
in section 1); `dcd0` (`:2610-2632`) takes the early/FPK rule above and the two
bits; `fs_required` (`:2327`) becomes `mask != 0 || alpha_to_coverage`.
`backend.rs`: drop the `check()` row (`:1101`) and pass `d.alpha_to_coverage`
where the `DrawCall` is built (`:1352`, next to `sample_mask`). Unit tests:
`frame_msaa_tests.rs` (add the a2c row beside the 4x one), `backend_tests.rs:871`.

**Checked by.** gl_test case **198** `a2cconst` (`gl_test.c:17250-17410`):
16x16 cells, a green then a red quad over it with the same alphas, alpha to
coverage on and alpha masked, 1024 draws in one frame on a 4x pbuffer, twice:
no green left, the two read-backs equal, the alpha-1 cell red. The case
SKIPs without a 4x config; the 4x config is offered (cases 16 and 145 pass).

The case also does `glClear(GL_DEPTH_BUFFER_BIT)` first (`gl_test.c:17360`),
on the 4x pbuffer whose D24S8 no draw then uses: if the Mali backend opens a
ZS attachment for that clear, `NOT_BUILT_SAMPLES` (section 4) refuses the
frame before a2c is reached. The frame builder lets a 4x frame carry an
attachment no draw uses (`frame.rs:1417-1420`, `uses_zs`), so it should not;
**pin** it with the first run.

**Pin on the board.** The record is
`0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/` (`mesa/mesa-a2cconst.pandecode.xz`,
decompressed here; line numbers are the uncompressed file's;
`mesa/mesa-a2cconst.stdout`, verdict PASS, 0 green pixels in both
pictures, the pictures equal):

- r57 of Mesa's a2c draw: **settled**, `MOVE32 r57, #0x5901BA` on every one of the
  2048 a2c draws (first at line 1395, then 1991, 2587, ...). Against the
  background draw's `#0x801AB` (line 72): `Allow forward pixel to kill` **false**
  (1943), `ZS update operation` **Force Late** (1946, bits 4-5 = 3),
  `Pixel kill operation` Weak Early (1945), `Multisample enable` true (1958),
  `Shader modifies coverage` **true** (1959, bit 20), `Alpha-to-coverage` **true**
  (1961, bit 22), `Alpha-to-coverage Invert` false (1960), `Single-sampled
  lines` false (1953). That is the derivation above, bit for bit; the
  prediction's depth-test-off draw is this one (Weak Early before).
- The fragment program is unchanged, the FBD is the 4x one of the
  1024-draw frame (`Sample Count: 4` 611730, `Effective Tile Size` 1024 and
  `Color Buffer Allocation` 16384, 611733-611740, `Writeback MSAA: Layered`
  611902).
- The depth clear: **settled for Mesa.** The 4x frame's FBD has
  `Has ZS CRC Extension: false` (611749; also 767 for the frame before it): the
  `glClear(GL_DEPTH_BUFFER_BIT)` opens no ZS attachment for a frame whose draws
  do not use depth, and `Pre Frame 0/1: Never` (611716-611717). Ours must also open none (see above); the
  first board run of ours will show whether `NOT_BUILT_SAMPLES` is reached.
- 1024 draws in one frame: **settled for Mesa, open for ours.** Each picture
  is one frame of 1024 `RUN_IDVS` between two `RUN_FRAGMENT` (lines 727 and
  611709; 653902 and 1272847): Mesa never splits it. Whether our frame has room
  for 1024 draws (`NOT_BUILT_FRAME_ROOM`, `frame.rs:192`; if not,
  `backend.rs:2053-2062` opens a second frame from a **4x preload**, section 5,
  where the PVR lost its samples, `gl_test.c:17260-17265`) is ours to measure:
  `panthor-gltest 198` on the first build, the verdict line or
  `NOT_BUILT_FRAME_ROOM`.

---

## 3. Blend constants that need a blend shader (case 8)

Case 8 `blend` (`0dce654a8:tools/mali/board-ref/2026-10-19-blend/README.md`): frames 1-4
pass (over, add, premul, replace); the fifth sets `glBlendColor(0.25, 0.5,
0.75, 1.0)` with `CONSTANT_COLOR`, three different constants, which the fixed
function cannot hold. The same path serves MIN/MAX and `SRC_ALPHA_SATURATE`
(`blend.rs:67-93`).

**Mesa: when.** `panfrost_get_blend` (`G/pan_context.c:163-229`): fixed function
if the equation allows it (`L/pan_blend.c:284-298`, the checks we mirror in
`blend.rs`), the format is blendable, there is no alpha-to-one and the
constants are homogeneous (`pan_blend_is_homogenous_constant`,
`L/pan_blend.c:247-258`; 170-183 of `pan_context.c`); none if the target is
not written (188) or the equation is opaque on arch >= 6 (196); otherwise a
blend shader (200-228). The shader is cached on the format, the source types,
the sample count, the render target and **the equation without the constants**
(`G/pan_blend_cso.c:59-98`, key 65-75): one shader serves every glBlendColor.
(`pan_blend_optimize_equation`, `L/pan_blend.c:136-242`, is called by panvk only;
gallium does not.)

**Mesa: the shader.** `pan_blend_create_shader` (`L/pan_blend.c:797-946`):

1. source colour from `load_blend_input` (837-842): on Valhall the registers
   r0-r3 (`C/bifrost/compiler.h:1225-1240`; two registers when the colour is
   fp16, `bifrost_compile.c:920-946`, our fragment shader writes fp16,
   `mod.rs:471`), the link register r48 (`compiler.h:1241`);
2. destination from `load_tile` (879-898), sample id when the target is 4x;
3. `nir_color_blend` with the equation (900-915), colour mask (917), the
   channels the format lacks dropped (919-921);
4. `blend` with the 64-bit opaque internal descriptor of the format
   (`pan_blend_get_internal_desc`, `L/pan_blend.c:975-1001`: mode OPAQUE,
   `num_comps`, `rt`, memory format; it is an immediate in the program), only
   if the colour changed (923-931);
5. `blend_return`: `BRANCHZI` on r48 on arch >= 9 (`bifrost_compile.c:1711-1721`).

It computes in the type of the destination, fp16 for an RGBA8 target
(`dest_type`, 826-828). Registers r0-r15 only (`C/bifrost/bi_ra.c:431-437`).
Constants: `load_blend_const_color_rgba` becomes a load from the sysval UBO
at offset 0 (`G/pan_blend_cso.c:41-57`), and the **fragment shader always
pushes those four words first**: `bi_pick_blend_constants`
(`C/bifrost/bi_opt_push_ubo.c:92-114`, called at 155-158) maps sysval slot 0
(`G/pan_nir_lower_sysvals.c:258-262`) to FAU words 0-3, "so that they can be
accessed from a consistent location from the blend shader". The blend shader
runs inside the fragment shader's thread, so it reads the draw's FAU. Our
fragment shaders already push them first (`src/shaderc/online/mali/fau.rs:27-29,
95-115`) and the frame fills them from `blend_color` (`frame.rs:2167`).

**Mesa: the call.** The fragment shader's `BLEND` is followed by the lowering
`va_lower_blend` (`C/bifrost/valhall/va_pack.c:1160-1202`, run for
non-blend shaders only, 1212-1213): write r48 the return address, `BRANCHZI`
to the blend descriptor's word 1 (the shader's pc) if non-zero, else fall
through to the fixed-function `BLEND`. **The fragment program is the same
whichever way the draw blends**; our `pack::lower_blend` is this
(`src/shaderc/online/mali/pack.rs:517-538`, run for fragment programs at
`mod.rs:764`), after the `BLEND` that `nir_fs.rs` emits at `:1850-1860`
(`select`) and `:2375-2385` (`select_highp`).

**Descriptor fields that change** (the `Blend` descriptor, `X:1045-1054`, per
target; `panfrost_emit_blend`, `G/pan_cmdstream.c:286-427`):

- `Internal` (words 2-3): mode `Shader` (0, `X:1015-1020`) and `Blend Shader.PC`
  (`X:1022-1024`: 28 bits at word 1 bit 4, `shr(4)`), the shader's address
  low 32 bits (`cfg.shader.pc = (uint32_t)`, `pan_cmdstream.c:374-377`); the
  address is 16-byte aligned. v10 has no `return_value` (that is arch <= 7,
  378-383).
- `Blend Constant` (word 0 bits 16-31): **not written** with a shader (336-337
  needs `!blend_shaders[i]`): 0. `Equation` (word 1): not written (348-354): 0.
- `Load Destination` (bit 0) still `info.load_dest` (332); `Round to FB
  precision` (333) unchanged; `Enable` stays true.
- `DCD0.Evaluate per-sample` (bit 11) set when multisampling with a blend
  shader (`pan_csf.c:1568-1570`).
- `DCD0.Allow Forward Pixel To Kill` already follows `load_dest` (ours,
  `frame.rs:2614`).

**Program bytes.** A new program: the blend shader, one per (format, samples,
equation), Valhall words, in the same code arena as the draw's programs. Its
address must have the fragment shader's top 32 bits (`pan_cmdstream.c:365-372`
asserts it for arch <= 7 only; the field is 32 bits on v10): **pin on the
board**, or just allocate it in the same arena. The fragment and vertex
programs do not change.

**Lands in.**

- A blend-shader generator in `src/shaderc/online/mali/` (a new module, the
  program from the equation; there is no Mesa tool for it, `tools/mali/
  README-pan_shc.md:309` lists blend shaders as "not covered"): the reference
  bytes come from the drm-shim arm (`tools/mali/drm_shim_ref.sh run`, with
  `gl_ref.py` taught a blend mode), one reference per equation class (ADD with
  distinct constants; MIN, MAX; SATURATE).
- `src/mali/blend.rs`: the discriminator of `panfrost_get_blend` (it has the
  equation checks and `constant()`); `frame.rs`: the `Blend` descriptor branch
  (`:2343-2397`) writes the shader mode, `check_draw` drops
  `NOT_BUILT_BLEND_SHADER` (`:1826-1829`), `dcd0.evaluate_per_sample`. A
  per-frame blend-shader cache next to `ProgramCache` (`frame.rs:684-760`).
- Tests: `frame_blend_tests.rs:210` (the refusal row) turns into a descriptor
  pin; `backend_tests.rs:864`.

**Checked by.** gl_test case **8** `blend`, frame 5, and the board reference
`0dce654a8:tools/mali/board-ref/2026-10-19-blend/blend.c` with a `const-distinct` mode added.

**Pin on the board.** The record is `0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/`
(`mesa-blend.pandecode`, `mesa-blend.stdout`: 7 frames, verdict PASS, frame 5
`constant colour: inside (32,40) = 0x800080bf` ok; `mesa-blend-rb05.pam`). The
fifth frame is the dump's fifth shader pair (`Shader ... 7fffffffc000` at
9398 and 10046), two draws: the opaque background (`MOVE32 r57, #0x121AB` at
9036, `Mode: Opaque` 9583) and the blended quad (r57 `#0x121AA` at 9684).

- The blend descriptor (r50 = `0x7FFFFFFD07B1`, i.e. `0x7FFFFFFD07B0` with the
  count 1 in the low bits, line 9681): decoded at 10207-10240 and as raw words in
  the `7ffffffd0000` buffer, row `0007B0` (line 11117): `01 02 00 00 | 00 00 00 00 |
  00 00 00 00 | 00 70 FC FF`, i.e. word0 `0x00000201` (Load Destination = bit 0, Enable
  = bit 9, `Constant: 0`), word1 `0` (Equation: INVALID, `Color Mask: 0`, not
  written), word2 `0` (**Mode: Shader**), word3 **`0xFFFC7000`** (`PC:
  4294733824`, line 10233). The fixed-function frames have the same word0 and
  their word1 equation (`0007B0` at 2153: `01 02 00 00 03 35 50 F0`), so only
  words 1-3 move, as the section says. Correction to the field description
  above: the recorded PC is the shader address's **low 32 bits as is**, not
  `shr(4)` of a 28-bit field (`0x7FFFFFFC7000` & `0xFFFFFFFF` = `0xFFFC7000`).
- **DCD0** of the blended draw is `0x121AA`: the same as the fixed-function
  frames' blended draw (e.g. line 720), `Allow forward pixel to kill` false
  (load_dest), `Evaluate per-sample` false (10293, 1x), `Multisample enable`
  false.
- The blend shader: its address is **`0x7FFFFFFC7000`**, a 4 KiB buffer of its own
  (`Buffer: memory_7ffffffc7000`, line 11251), the one 4 KiB page below the
  command-stream buffer `7ffffffc8000` (11199), not in the 4 KiB arena of the
  other programs (`7fffffffc000`, vertex `..c000`, fragment `..c100`, line 10123).
  It is 152 bytes (19 words, `0x00-0x97`; the rest of the page is zero), not
  disassembled by pandecode, dumped at lines 11253-11262:

  ```
  C0 00 D0 0E 00 C2 10 01
  C0 00 00 00 FF C3 10 01
  43 3C 02 33 04 84 78 08
  45 D9 C0 0E 04 C3 B3 00
  41 C0 00 28 13 C1 A5 00
  40 C0 00 28 13 C0 A5 00
  C0 00 3A 00 3A C5 10 01
  01 05 43 08 00 C3 B3 00
  04 45 C0 0A 04 C5 B3 00
  00 44 00 3C 00 C4 A5 00
  40 D9 45 0C 00 C0 B3 00
  43 41 05 0C 00 C1 A1 00
  DA 44 C0 32 04 C3 B3 00
  40 43 05 00 00 C0 A1 00
  C0 19 00 00 00 C3 10 01
  43 00 00 00 00 C4 91 00
  42 00 00 00 00 C5 91 00
  44 00 3C 73 04 40 7F 00
  70 70 00 00 00 C1 2F 78
  ```

  Its tail has the shape of the fragment program's own (10128-10130): the
  word at `0x88` ends `04 40 7F` like the fragment's `BLEND.slot2.v4.f16.end`
  (`f0 00 3c b3 04 40 7f 78`), and the last word ends `C1 2F 78` like its
  `BRANCHZI.eq.absolute` (`c0 f1 00 00 10 c1 2f 00`) with the `.end` bit set;
  that is a reading of the opcode bytes, not a disassembly. The fragment program
  is the fixed-function one: it sets `r48` to 0 (`IADD_IMM.i32 r48, 0x0, #0x0`,
  10129) and branches on `blend_descriptor_0.w1` (10130), so a non-zero word 3
  diverts it to the blend shader. The bytes are the board's Mesa **25.0.7**;
  the 26.2.1 compiler may differ, and the pixels are the proof (frame 5
  read-back).
- The 4 GiB rule: **settled as "not contradicted".** The blend shader
  (`0x7FFFFFFC7000`) and the fragment program (`0x7FFFFFFFC100`) share the top
  32 bits (`0x7FFF`); the descriptor holds only the low 32. Nothing in the
  record places a blend shader in another window, so the rule stays "allocate it
  in the draw's code window"; no run was made to break it.
- The blend shader at 4x and the sample id: **settled** by gl_test case 211
  `msaablend` under Mesa (`0dce654a8:tools/mali/board-ref/2026-10-21-mesa-refs3/`, mode
  `msaablend`: an RGBA8 texture behind a 4x view, the backend's 4x target;
  `mesa/mesa-msaablend.pandecode`). The blended draw's descriptor is `Mode:
  Shader`, `PC: 4294750208` (0xFFFCB000, lines 1267-1269; raw row `0007B0` at
  2184: word0 `0x201`, words 1-2 zero, word3 `0xFFFCB000`, the 1x shape), and
  its DCD0 has **`Evaluate per-sample: true`** (1329) with `Multisample enable:
  true` (1335), `r57` `#0x809AA` (720). The 4x blend shader (`memory_
  7ffffffcb000`, 2287-2297; disassembled in `blend-shaders.disasm.txt:26-44`
  beside the 1x one of case 8 at `:6-24`) is **not** the 1x program with the
  sample added to `BLEND`: it reads the sample id from r61 (`MOV.i32 r2, r61`,
  `RSHIFT_AND.i32 r2, r2, 16, 0x1F`, then `+ 0xFF000000`, `:26-29`), passes
  that word as the pixel operand of `LD_TILE` (`:31`; the 1x shader passes
  the constant `0xFF000000`, `:7-8`) and ends with
  **`ST_TILE.v4.f16`** of the blended colour at the same pixel word, coverage
  r60 and conversion word `0xED000` (`:43`) instead of the 1x shader's
  `BLEND ... [r4:r5]` with the internal descriptor (`:23`). Both are 19 words.
  The picture: over red `0x800080bf`, over black `0x80008000`, the half-covered
  diagonal **`0x80008060`** (each sample blended with its own destination;
  `mesa/mesa-msaablend.stdout:8-11`, PASS). Mesa's own 4x pbuffer
  (`msaablend-pb4`) builds the same 19 words (`mesa/mesa-msaablend-pb4.pandecode:
  4593-4604`) and the same picture (both read-backs fnv1a `5638abc5`).

---

## 4. 4x with depth/stencil

**Mesa.** `EXT_multisampled_render_to_texture` is how panfrost does what
`backend.h:221-224` describes (the 4x tile buffer averaged into single-sampled
memory): `caps->surface_sample_count = true` (`G/pan_screen.c:726`), and the
view of a 1x image is made with `nr_samples = surf->nr_samples` (4)
(`G/pan_job.c:537-538` colour, `:571` depth, `:592-593`, `:609` stencil). The
mode of each part is `mali_sampling_mode` (`L/pan_desc.c:26-48`): the image has
1 sample and the view 4, so **AVERAGE**. For depth and stencil it is written
in the ZS/CRC extension: `cfg.zs.msaa` and `cfg.s.msaa` (`pan_desc.c:501-557`,
518-521); the colour target takes the same function (1033). The whole
depth/stencil attachment is otherwise as for 1x (`pan_emit_linear_zs_attachment`,
`pan_desc.c:356-373`).

Tile size: `pan_select_tile_size` (`pan_desc.c:606-649`) takes the smaller of the
colour and the depth budgets. Depth is 4 bytes a sample times the **view's**
samples (`pan_zsbuf_bytes_per_pixel`, `L/pan_desc.h:241-258`), the budget
`optimal_z_tib_size` (`G/pan_job.c:473`) is half the model's `z_size`
(`L/pan_props.h:61-72`), G610: 16384 (`src/panfrost/model/pan_model.c:88`), so
8192. A 4x frame with depth: 8192 >> log2(16) = 512 against the colour's 16384
>> 4 = 1024, so **the effective tile is 512, the colour allocation `ALIGN(16 *
512, 1024)` = 8192** (`pan_desc.c:617-625`, 643); a 1x frame with depth is 2048
against 1024, unchanged (the existing 1x ZS dumps). Ours (`frame.rs:1428-1432`)
ignores depth. These two numbers are read from the code; the board's Mesa
25.0.7 gives 1024 and 16384 instead (see "Pin on the board" below).

The draw side is the 4x colour-only draw we have (`msaa` reference: DCD0
multisample enable, sample mask) plus the depth state; nothing else changes.
The FBD's sample count, pattern and tiler context are already 4x.

**Descriptor fields that change** (vs. a 4x frame with no depth, and vs. a 1x
frame with depth): the ZS/CRC extension's `ZS Target.MSAA` (`X:1724-1733`,
bits 8-9, word 0) SINGLE to AVERAGE (ours `frame.rs:1664`, `msaa:
Msaa::SINGLE`; the enum is `X:405-411`); the FBD's `Effective tile size` and
`Color buffer allocation` (512 and 8192 on the T6). The `S Target.MSAA`
(`X:1754`) stays SINGLE while the stencil is the packed D24S8's (no separate
view, as `frame.rs:1648-1650` already assumes).

**Program bytes.** None.

**Lands in.** `frame.rs`: `check_target` accepts `samples == 4` with ZS
(`:1417-1420`; `NOT_BUILT_SAMPLES` text, `:178-183`); `tile_size()` takes the
depth into account (it is called from the FBD and from `tiler_context`'s rule
at `:2721`); the `ZsTarget` at `:1664` takes `msaa` from the target. The
preload of such a frame is section 5. `backend.rs`: nothing at `check()`; the
refusal lives in the frame builder. Tests: `frame_msaa_tests.rs` (a 4x depth
row against a new Mesa dump), `backend_tests.rs:849` (flips from refused).

**Checked by.** No gl_test case tests depth or stencil on a 4x surface today:
cases 16 and 145 do not enable the tests (a read of their code), case 198
only clears depth (`gl_test.c:17360`), and I found no other with an
`EGL_SAMPLES 4` config. The issue's list has none to name here, so
`frame-msaa-zs` needs one: case 16's triangle with a depth test and two
overlapping triangles at different depths, on the 4x pbuffer, which also needs
a Mesa reference in the board-ref method (`msaa.c` with a depth attachment
under `glFramebufferTexture2DMultisampleEXT`).

**Pin on the board.** Settled by the record `0dce654a8:tools/mali/board-ref/2026-10-20-msaa-zs/`
(`README.md`, "Mesa's reference"; `mesa/mesa-msrtt.pandecode` the depth frame,
`mesa/mesa-msrtts.pandecode` the stencil frame; Mesa 25.0.7 on the T6):

- The tile and allocation numbers: **the board's Mesa 25.0.7 gives 1024 and
  16384, not 512 and 8192.** `Sample Count: 4` (msrtt 730), `Effective Tile
  Size: 1024` (733), `Color Buffer Allocation: 16384` (740); the same in
  msrtts (1381, 1388). `README.md`: "25.0.7 has no z-budget rule for the
  view's samples; 26.2.1's would give 512 and 8192". So the paragraph above is
  26.2.1's reading and is **not** what to pack against this board's Mesa:
  `tile_size()` keeps ignoring the depth, pinned to these dumps. A move to
  26.2.1's rule needs a board run of that Mesa.
- What AVERAGE writes: **settled.** `ZS MSAA: Average` (msrtt 869, msrtts
  1517), `Writeback MSAA: Average` (924, 1572); the read-backs equal the
  single-sample picture (depth edge `0xff008080`, stencil edge `0xff808000`,
  `README.md` table: msrtt 6e52a585, msrtts 25387b85, both PASS).
- Packed D24S8 versus a separate stencil: **settled, one packed view.** `S MSAA:
  Single` (msrtt 873) with `S Clear: 0` and no S part; the stencil frame
  (msrtts) is the same single ZS view with `Z Clear: 0.000000` (1402). The
  stencil byte itself is not read back; what the record shows is the stencil
  frame's picture: the edge pixel is the half-covered `0xff808000` and the
  read-back equals the 4x pbuffer's (`README.md` table, msrtts and pb4
  25387b85). Mesa 25.0.7 reports a D24S8 texture
  attached with `glFramebufferTexture2DMultisampleEXT(..., 4)` on both DEPTH
  and STENCIL as incomplete (`README.md`), so the two frames are separate
  attachments of the same resource.

---

## 5. Preload of a 4x target

**Mesa.** `GENX(pan_preload_fb)` (`G/pan_fb_preload.c:1298-1336`) builds the pre
frame DCDs from the **images' sample count**, not the view's:
`pan_preload_is_ms` (162-179) and the shader key's `samples` (585-609) call
`pan_image_view_get_nr_samples`, which reads `image->props.nr_samples`
(`L/pan_image.h:115-124`). A 4x view of a 1x image (the multisampled render to
texture, section 4) is therefore **not multisampled** to the preload: the
program is the single-sampled one, the texture a plain 2D one, the DCD's
`multisample_enable` and `evaluate_per_sample` are `ms` = false (1144-1145),
`sample_mask` 0xFFFF (1143), and the pre-frame modes are the 1x ones (colour
INTERSECT 1247-1250, depth/stencil EARLY_ZS_ALWAYS on v10, 1228-1230). That is
exactly what `colour_preload_dcd` and `zs_preload_dcd` pack for a 1x target
(`frame.rs:1992-2150`). The multisampled shader (`txf_ms` with the sample id,
`pan_fb_preload.c:460-496`) is for a real 4x image, which the GL core never
makes (`backend.h:221-224`: "FBOs are 1").

Which targets are preloaded is `panfrost_batch_to_fb_info`
(`G/pan_job.c:543-548` colour, `:629-651` depth/stencil): not cleared and read
or drawn over a valid image. The FBD's `sample_count` stays 4
(`pan_desc.c:1421-1442`, asserts at 1309, 1315); the render target's
`writeback_msaa` AVERAGE and the clean-tile rules are those of the frame
without a preload.

**Descriptor fields that change.** Against a 1x frame with a preload: none in
the preload's DCDs. Against today's 4x frame (cleared): `FBD.Pre Frame 0` (and
`Pre Frame 1` with depth) INTERSECT (EARLY_ZS_ALWAYS), the DCD array,
`RT.Clean Tile Write Enable` off, `RT.Clear` zero (as `end_frame`,
`frame.rs:1593-1745`, does for 1x). No field is new; the 4x frame only stops
refusing.

**Program bytes.** None: the existing colour and Z/S preload programs
(`src/shaderc/online/mali/preload.rs`, `backend.rs:1845-1853`).

**Lands in.** Drop `backend.rs:1865-1867` and `frame::NOT_BUILT_PRELOAD_SAMPLES`
(`frame.rs:146`); `colour_preload_dcd` keeps
`sample_count: 1` in the texture. For depth/stencil see section 4. The ZS
preload of a 4x frame has no check of its own: it is the `NOT_BUILT_SAMPLES`
of section 4.

**Checked by.** `backend_tests.rs:880` (the "no clear on a 4x target" row)
and a split 4x frame: case **198** when its 1024 draws overflow one frame
(section 2) is the first case that needs it; whether that happens on our side
is open (section 2: Mesa's frame holds all 1024 draws). A direct control is Mesa's own: draw, flush, draw over it, on
`msaa.c`'s msrtt mode (no `glClear` the second time).

The board case is gl_test **208 msaapre**: on the 4x pbuffer (8-bit colour, no
depth or stencil used), a cleared frame with case 16's red lower-right
triangle, `glFinish`, then a frame with no clear drawing a green lower-left
triangle (x + y < 0) over it; both frames are read back. Its first frame is
case 16's (PASS on the T6); its second draw opens a frame with no clear on a
4-sample target, which `open_frame` refuses after the stride check
(`backend.rs:1902-1912`), so today it prints

    panthor-gltest: verdict case 208 msaapre: REFUSED draw: mali frame: a preload of a multisampled target is not built yet

(`frame.rs:146`; the mock's "no clear on a 4x target" row,
`backend_tests.rs:894`). After the fix it reads `PASS` exactly when the
reloaded pixel lands in all four samples: the first frame's red (56, 12),
its half-red diagonal (40, 40, `0xff000080`) and black (12, 56) kept, the
green's edge half over red (40, 23, `0xff008080`) and half over black (23,
40, `0xff008000`). A reload into one sample shows at those pixels, which is
the pin below.

**Pin on the board.** This is the open question of the whole design: what a
single-sampled preload draw does to a **4-sample tile buffer**. If the
hardware writes the pixel's value into all four samples (which Mesa relies
on, since it builds nothing else for this case), the 4x preload is free. If it
writes only sample 0, the fallback is a deviation from Mesa that no source
shows: the same DCD with `multisample_enable` and `evaluate_per_sample` on and
the 1x texture, so the shader runs per sample and each reads the same texel;
that must be measured, not assumed. Also the reloaded colour is the *averaged*
image, so a draw split across frames differs from an unsplit one at the
edges; Mesa has the same property.

**Settled: the hardware writes the single-sampled preload into all four
samples.** gl_test case 208 `msaapre` under Mesa
(`0dce654a8:tools/mali/board-ref/2026-10-21-mesa-refs3/`, mode `msaapre`: an RGBA8
texture behind a 4x view, the backend's 4x target;
`mesa/mesa-msaapre.pandecode`, two frames). The second frame's FBD reads `Pre
Frame 0: Intersect`, `Pre Frame 1: Never` (2276-2277), `Frame Shader DCDs:
0x7ffffffd4800` (2283), `Sample Count: 4` (2290), tile 1024 and allocation
16384 as the cleared frame (2293, 2300), `Writeback MSAA: Average` (2735),
`Clean Pixel Write Enable: false` (2739; true in the cleared frame, 906). Its
preload DCD is the single-sampled one predicted above: `Evaluate per-sample:
false` (2589), `Multisample enable: false` (2595), `Sample mask: 65535`
(2600), the texture `Dimension: 2D`, `Sample count: 1` (2517, 2528). The
pixels: every check of the case is ok (`mesa/mesa-msaapre.stdout:8-20`,
verdict PASS), among them the first frame's half-red diagonal kept at
`0xff000080` and the second triangle's edge half green over red
`0xff008080` and over black `0xff008000`: a reload into one sample would
leave the other three at their cleared value and change those pixels. The
read-backs equal Mesa's own 4x pbuffer's (`msaapre-pb4`, fnv1a `066327c5` and
`d65d3105` both), whose image is multisampled in memory and so reloads with
a per-sample DCD and a 4-sample texture
(`mesa/mesa-msaapre-pb4.pandecode:7764-7842`): the two routes draw the same
picture. So the 4x preload needs no deviation from Mesa: the 1x preload DCDs
on the 4x FBD. The Z/S part (a 4x depth view reloaded) is not a gl_test case;
`0dce654a8:tools/mali/board-ref/2026-10-23-msaa-preload/` (mode `msrttz`) holds Mesa's
descriptors for it.

---

## 6. Varyings declared in different orders in the VS and the FS

**Mesa.** The location of a varying is decided **by the vertex shader's
output list** and the fragment shader follows it.

- GLSL link: each vertex output is paired with the fragment input of its name
  (`GL/gl_nir_link_varyings.c:3602-3610`), the pairs are sorted by packing class
  (interpolation, centroid, sample...), then packing order (scalar, vec2,
  vec3, vec4), then **the producer's original index**
  (`gl_nir_link_varyings.c:2490-2505`, `2653-2709`, `2829-2861`), and both
  variables get the same `VARYING_SLOT_VAR0 + n` (`:3032-3056`). The fragment
  shader's declaration order is not an input.
- Layout: the vertex shader's layout is built once, from the shader, at
  creation: `pan_varying_collect_formats` (slot index = the variable's
  `driver_location`, `C/pan_nir_collect_varyings.c:187`, 338-410) then
  `pan_build_varying_layout_compact` (412-448): offsets in slot-index order, each
  aligned to the next power of two of its size (433-436), `generic_size_B`
  the total (`G/pan_shader.c:583-591`).
- Link: the fragment shader's layout **is** the vertex shader's:
  `fs->vs_varying_layout = vs->vs_varying_layout` (`pan_shader.c:659-690`,
  681-682), and the fragment shader's default variant is not compiled until
  then (611-617), so its `LD_VAR_BUF_IMM` offsets are the vertex shader's
  offsets by construction.
- The draw: `VARY_SIZE` is the vertex layout's `generic_size_B`
  (`pan_csf.c:1506-1507`). Nothing else in the descriptors depends on the order.

The board confirms the producer decides: `0dce654a8:tools/mali/board-ref/2026-10-19-packed-varyings/`
`mesa/` (`pack.c`, modes `fwd` and `rev`): a `STORE` writes `(a_t.xy, a_pos.zw)`
or its halves swapped with the vertex declaration order, and both draw the
same pixel.

**Descriptor fields that change.** None. `VARY_SIZE` is the vertex layout's
size, which the order does not change (the same set of outputs).

**Program bytes.** The **fragment** program's `LD_VAR_BUF_IMM` byte offsets
change (immediates in the instruction words); the vertex program's stores and
its varying variant are already the vertex shader's own layout. Today our
fragment shader is laid out in its own declaration order and the link
compares offsets (`link.rs:496-505`): equal passes, unequal is refused.

**Lands in.** Two ways, the first being Mesa's:

1. Compile the fragment shader for the vertex shader's layout: the link
   rebuilds the FS record with the VS offsets (the lowering has the reads as
   `Section::Generic` slots, `MaliShader::varyings`; `compile_vs_for_reads`,
   `src/shaderc/online/mali/mod.rs:1105`, is the existing example of one stage
   rebuilt for the pair). Needs the fragment shader's IR or source kept past
   its compile (the `keep` arena, `mod.rs:1079-1099`).
2. Patch the offsets in the finished fragment program: not Mesa's way (it
   compiles once, for the layout), and the offsets are immediates inside packed
   instruction words; not recommended.

The change is in `link_shaders_mali` (`src/shaderc/online/mali/link.rs`; it was in
`src/shaderc/link.rs` before #1387): `MALI_LAYOUTS_DIFFER` is
replaced by the rebuild and kept only for what the rebuild cannot do;
`MALI_PARTIAL_READ` (components read in part) is the neighbouring gap and shares
the machinery. Whatever is rebuilt, the layout rule is the producer's: the
vertex shader's slot order, power-of-two alignment (`pan_nir_collect_varyings.c:
433-436`).

**Checked by.** No gl_test case today pairs the stages in different orders;
the reference is the `pack.c` `rev` mode above, which `panthor-gl` can draw
(`--varying`-style mode) and `0dce654a8:tools/mali/board-ref/2026-10-19-packed-varyings/README.md`
says is "refused by name at the link (MALI_LAYOUTS_DIFFER)". The item needs a
`panthor-gl` mode or a gl_test case for it; none exists to name.

The board case is gl_test **209 varyorder**: one vertex shader writes `vec4
v_a`, `vec2 v_uv`, `vec4 v_b` (in that order, each from an attribute in a
buffer), two fragment shaders compute `v_a * vec4(v_uv, 1.0, 1.0) + v_b`
(highp, no uniform, no texture), one declaring the varyings in the vertex
order (top half) and one in the reverse order (bottom half); both halves
must read (64, 191, 191, 255). The two vec4s are needed: our layout
(`generic_layout`, `select.rs:105-136`) sorts by packing class (vec4, vec2,
scalar, vec3) and keeps the declaration order only within a class, so a vec4
and a vec2 swapped lay out alike (color_uv's pair with its fragment
declarations swapped links on the host; case 19's textured pair is such a
swap too, and is refused earlier). The reverse order puts `v_b` at 0 in the fragment layout and at 16
in the vertex one; on the host's Mali back end the first program links and
the second's link log is the line below, so today the case prints

    panthor-gltest: verdict case 209 varyorder: REFUSED link: online shaderc: a Mali link whose stages place a varying at different offsets is not lowered yet ('v_b')

(`MALI_LAYOUTS_DIFFER`, `src/shaderc/online/mali/link.rs:20`). After the
fix (the fragment shader compiled for the vertex layout) it reads `PASS`,
both halves the same pixel.

**Pin on the board.** The varying layout of case 209 under Mesa: **settled**
(`0dce654a8:tools/mali/board-ref/2026-10-21-mesa-refs3/`, mode `varyorder`,
`mesa/mesa-varyorder.pandecode`, one frame, both programs). The vertex
shader's varying variant stores at offsets 0 and 16 (two `STORE.i128`) and 32
(`STORE.i64`) in both programs (525-533 and 1233-1241), `Varying allocation:
48` for both draws (654, 1362). The fragment program of the reverse order
reads with the **same three words** as the vertex order's,
`LD_VAR_BUF_IMM` at `index:0x10` (v4), `0x20` (v2) and `0x0` (v4) (562-564
against 1270-1272, byte for byte): Mesa compiles the fragment shader for the
vertex layout, and the fragment declaration order leaves no trace in the
program. The two vec4s come first and the vec2 last, so the packing-class sort
applies on this screen (panfrost does not disable varying packing). Both
halves read `0xffbfbf40` (`mesa/mesa-varyorder.stdout:8-10`, PASS). Ours
reads PASS too since `link-varying-order`
(`0dce654a8:tools/mali/board-ref/2026-10-23-varying-order/README.md:104`).

## 7. A frame's data past its first chunk (cases 197 and 56, #1385 (b))

**Ours, before.** The frame's `data` arena (client vertex arrays, staged
indices, texel row copies) was one fixed 256 KiB region of the slot
(`DATA_BYTES`). Case 197 (vtxgrid) stages 516,128 bytes for one draw, so the
draw was `-ENOMEM` ("mali arena 'data' is full: 516128 bytes aligned to 16 do
not fit in the 262144 left", `0dce654a8:tools/mali/board-ref/2026-10-23-tex-coord/`);
splitting the frame cannot help a draw larger than an empty one.

**Mesa.** User vertex arrays go to the stream uploader, 1 MiB buffers that
grow to `align(MAX2(default_size, min_size), 4096)`
(`src/gallium/auxiliary/util/u_upload_mgr.c:93-95`, `:181`); the batch pool
takes a new BO of `ALIGN_POT(MAX2(slab_size, sz), 4096)` when a block does
not fit (`G/pan_mempool.c:162-164`). Nothing is flushed. In the board dump
(`0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-vtxgrid.pandecode.xz`,
lines 134-149) the draw's two attribute Buffers are at `0x7fffffec6000` and
`0x7ffffff44020` of one 1 MiB BO, apart from the batch's descriptor BO.

**Ours, now.** The data region is chunk 0 of a `Pool`
(`src/mali/arena.rs`): a block that does not fit goes to the next chunk that
holds it; when none does, the backend (`grow_data`, `src/mali/backend.rs`)
adds a chunk of `MAX2(DATA_BYTES, block + align - 1)` bytes rounded to 4 KiB
and the draw is made again. Blocks never move, so every address in a
descriptor stays valid; the grown chunks belong to the slot (kept for its
later frames, freed at shutdown or replaced while they hold nothing of the
open frame, which the GPU does not read). A frame whose data fits chunk 0 is
built byte for byte as before. Up to 8 chunks are grown per slot; past that
the frame ends and the draw goes on in the next (`no_room`), as for the
other arenas.

---

## Order and dependencies

- `frame-query`, `frame-a2c` and `frame-blendconst` are independent of each
  other and of the MSAA items. All three add a `DrawCall` field or branch, so
  they conflict in `frame.rs` `emit_draw` and in the `DrawCall` literals:
  land them one at a time, each against the current main.
- `frame-msaa-zs` and `frame-msaa-preload` share `check_target`/`tile_size` and
  the 4x frame. The cleared 4x frame with depth comes first; the preload of a
  4x frame (colour, then Z/S) after it, and after the pin of the open question in
  section 5, which can be taken with a colour-only 4x frame before any depth.
- `frame-a2c`'s case 198 needs `frame-msaa-preload` if its frame splits, and
  `frame-msaa-zs` if its depth clear opens an attachment on the 4x frame.
- `link-varying-order` follows #1387 (the file moves).
- None of the six changes a vertex program, a fragment program (but for the
  link rebuild) or the FBD of a 1x frame.

## Facts I could not settle from source alone

Collected from the **pin on the board** lines, now against the committed
records (`0dce654a8:tools/mali/board-ref/2026-10-20-query/`, `-mesa-refs/mesa/`,
`-msaa-zs/`, and `2026-10-21-mesa-refs3/` (`R3/` below: gl_test cases 208-211
under Mesa); citations in each section's pin block):

| Fact | State | Where |
| --- | --- | --- |
| OQ register value, DCD0 value | answered: `d46` = the query buffer, `r57` `0x16183` (PREDICATE) | section 1, `mesa-occl.pandecode:716,721` |
| entry addressing, which qword | answered: qword 0 only, `shader_present` `0x50005` (range 19) | section 1, `tools/mali/board-dump/query-2026-10-20.txt:1391-1448` |
| q4's ordering | answered: result 1, the count is not taken after the later occluder | section 1, `mesa-occl.stdout:11` |
| accumulation across jobs | answered for PREDICATE (case 210): qword 0 stays 1 after a second passing kick (stored, not added) and is not cleared by a later kick that passes nothing; COUNTER unrecorded | section 1, `R3/mesa/mesa-occlacc.pandecode:2407-2409,5348-5350,8305-8307` |
| a2c r57 | answered: `0x5901BA`, FPK off, ZS update Force Late, coverage and a2c bits | section 2, `mesa-a2cconst.pandecode.xz:1395,1943-1961` |
| a2c depth clear / 1024 draws | answered for Mesa (no ZS extension; one frame of 1024); ours is open: first `panthor-gltest 198` | section 2 |
| blend shader bytes, address, descriptor | answered: 152 bytes at `0x7FFFFFFC7000`, word 3 `0xFFFC7000`, mode Shader | section 3, `mesa-blend.pandecode:11251-11262,11117,10231-10233` |
| blend shader's 4 GiB window | not contradicted (same top 32 bits as the fragment program), not tested | section 3 |
| blend shader `BLEND` at 4x with the sample id | answered (case 211): `Evaluate per-sample` set; the 4x shader takes the sample id from r61 into the `LD_TILE` pixel word and ends in `ST_TILE`, not `BLEND`; the diagonal `0x80008060` | section 3, `R3/mesa/mesa-msaablend.pandecode:1267-1269,1329`, `R3/blend-shaders.disasm.txt:26-44` |
| depth/stencil AVERAGE, tile, allocation | answered: AVERAGE, one packed ZS view, tile **1024**, allocation **16384** on 25.0.7 (not 512/8192) | section 4, `mesa-msrtt.pandecode:733,740,869,873` |
| single-sampled preload on a 4-sample tile | answered (case 208): `Pre Frame 0: Intersect`, the 1x preload DCD (multisample and per-sample off, 1-sample 2D texture) on the 4x FBD, and every edge pixel right: the reload lands in all four samples | section 5, `R3/mesa/mesa-msaapre.pandecode:2276,2589-2600,2528`, `R3/mesa/mesa-msaapre.stdout:8-20` |
| varying layout (case 209) | answered: both orders' fragment programs read with the same `LD_VAR_BUF_IMM` words (offsets 16, 32, 0), the vertex layout 0/16/32, allocation 48 | section 6, `R3/mesa/mesa-varyorder.pandecode:525-533,562-564,1270-1272` |
