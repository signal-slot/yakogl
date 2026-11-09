<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: a 64-bit integer colour target beside another target (design)

Why `draw_one` refuses a draw that has an RGBA16I/UI or RG32I/UI colour
target and any other target, what lifting the refusal takes, in which
order, and what only the board can answer. Design only: nothing here was
run and no code changed. Line numbers are of commit `722494ee` plus the
tip `c6b16f71` (the files below did not move between them); they will
drift, so grep the quoted name when they do.

This is RC1 of `docs/es3-cts-failures.md`. Host dEQP `fragment_out` on
`GPU=vivante` fails 63 cases on it; the board fails the same ones, because
the refusal is the back end's, not the stand-in OSAL's. Related notes:
`docs/es3/int-rt-gc.md` (what is drawn today),
`docs/notes/gc-128bit-render-targets.md` (the RGBA32I/UI plane pairs, which
need the same multi-target shadow), `docs/notes/gc-tiled-targets.md`.

## (1) Why the refusal exists

**The hardware fact.** The PE misplaces 64-bit pixels in a LINEAR colour
surface: it moves 16-byte pixel pairs inside a row (gl_test case 44, record
`got 0x20000004 want 0x00c2073e`; `src/vivante/wide_target.rs:8-15`). Mesa
never renders a linear target on this core, so it never meets it
(`wide_target.rs:17-22`). A 64-bit target therefore must be rendered TILED.

**The cure that exists: a shadow for one target.** `wide_target::wanted`
(`wide_target.rs:189-195`) says yes for a draw with exactly one hardware
target (`single`), a positive stride (`straight`), a CPU view of the
storage and 8 bytes a pixel. The draw then:

- `prepare` (`wide_target.rs:207-273`, called at `backend.rs:2916-2924`)
  finds or allocates a page-aligned tiled copy, padded to 16x4
  (`padded`, `wide_target.rs:115`), and `tile_in`s the linear storage into
  it (`:291-311`) when the storage is the newer copy;
- `emit_pe_state` binds it (`wide_target::bind`, `backend.rs:1677`, marks
  the shadow as written by the open kick, `wide_target.rs:314-320`), gives
  PE_COLOR_STRIDE the padded stride (`backend.rs:1680-1681`), points both
  pipe colour addresses at it (`backend.rs:1734-1735`) and writes
  PE_LOGIC_OP SINGLE_BUFFER(2) (`backend.rs:1755`, `single_buffer`
  `:565-587`), Mesa's value for a tiled target of more than 16 bits;
- the dedup of PE state is off for such a draw (`pe_key`,
  `backend.rs:2022-2024`), since `bind` must run every time;
- when the last kick that wrote it has completed, `retire`
  (`wide_target.rs:324-351`, from `retire_wide_targets`,
  `backend.rs:3263,3295,3322,3350`) untiles it back into the linear
  storage; `released` drops it with its storage (`backend.rs:3357`).

**Why it stops at one target.** Three things.

1. *The count.* `wide_target()` (`backend.rs:560-562`) passes
   `hw_targets(gl).1 == 1` (`hw_targets`, `backend.rs:525-535`). A second
   target is programmed in the 8-RT bank (`backend.rs:1706-1725`):
   address `pe_rt_addr_8_pipe`, `pe_rt_config_8` (stride, format, sRGB),
   per-target colour mask and alpha config. That bank "has no tiled bit of
   its own" (`wide_target.rs:46-47`, `backend.rs:1702`): SINGLE_BUFFER in
   PE_LOGIC_OP switches **every** target of the draw between linear and
   tiled (Mesa `etnaviv_state.c:532-539`, quoted in
   `gc-128bit-render-targets.md` section 4 item 2). So a draw with a 64-bit
   target must tile all its targets, 32-, 16- and 8-bit ones too; a shadow
   for the wide target alone would leave the others written tiled into
   linear storage.
2. *The CPU view.* The shadow copies on the CPU (`tile_in`, `retire`), and
   `Key` is the target's `target_gpu`/`w`/`h`/pitch (`wide_target.rs:197`).
   `GlesDrawTarget` (`src/backend.rs:841-853`) carries only `gpu`, `w`, `h`,
   `format`, `out_shift`: targets 1 to 3 have no CPU view and no pitch,
   only slot 0 has (`GlesDraw::target_cpu`, `src/backend.rs:1269`,
   `target_pitch`, filled `src/gl/draw.rs:2334-2349`; the others at
   `draw.rs:2494-2501`). The shadow's pixel size is the constant `BPP` = 8
   (`wide_target.rs:60`).
3. *The refusal itself.* Without 1 and 2 a mixed draw "would keep the linear
   path", so `draw_one` returns `-ENOTSUP` rather than store garbage
   (`backend.rs:2813-2833`; the test is `n > 1 && GC_INT_RT_64BPP & bit != 0`
   at `:2829`, the set is `GC_INT_RT_64BPP`, `backend.rs:361-365`; the
   rationale is also in the doc comment `:341-347`). The GL core reports
   it as `GL_INVALID_OPERATION` on the flush.

**What else rides on it.** A `glClearBuffer{i,ui}v` of one attachment is
drawn by the clear quad (`emit_clear`, `backend.rs:2384`) whose draw record
carries every attachment of the framebuffer, so it too is a mixed draw and
is refused; `clear_takes` (`backend.rs:499-502`) only zeroes the other
targets' colour mask (`rt_mask`, `:1490`). Two tests already pin that as a
known defect, `#[ignore]`d:
`a_clear_of_a_64_bit_attachment_beside_another_clears_it` and
`a_clear_of_a_32_bit_attachment_beside_a_64_bit_one_clears_it`
(`tests/es3_int_rt_mrt_gc.rs:636-695`); three more pin the refusal
(`a_64_bit_target_beside_a_32_bit_one_is_refused_at_an_unaligned_size`,
`two_64_bit_targets_are_refused`,
`a_refused_mrt_draw_leaves_a_one_target_draw_working`, same file from
`:718`; `tests/es3_int_rt_gc.rs` for the single-target case). RGBA16F is
the same class: it also takes the shadow only alone (`wide_target.rs:46-49`
says its mixed draw "still" runs linear), so a mixed RGBA16F draw is wrong
on the board today and the same change fixes it.

## (2) What a mixed draw needs

**Every target on a shadow.** Per (1.1), once any target is 8 bytes a
pixel, all targets of the draw, 1, 2, 4 or 8 bytes a pixel, are rendered
into 4x4-tiled shadows: SINGLE_BUFFER(2) for the draw, each target's
address at its own shadow, each stride the shadow's. The shadow module
becomes bpp-parametric (`BPP` and `tiled_offset`, `wide_target.rs:60,
128-132`, take the pixel size; `Key` gains it). A draw with no 64-bit
target keeps the linear path untouched (the measured, board-proven one).

**Strides.** Slot 0: PE_COLOR_STRIDE = padded width x bpp
(`wide_target::stride`, `:122-124`). Targets 1 to 3: `rt_config_stride(
rt_pitch)` (`backend.rs:1718`) from `target_pitch` today. For a tiled
target the same quantity is the shadow's row stride; whether RT_CONFIG
wants it as given or as PE_COLOR_STRIDE does (Mesa: only the RS takes a
tiled stride times 4, `wide_target.rs:122`) is a board question (section
5).

**Per-target shadow or direct?** Per-target shadow for all, not "direct
for the narrow ones": a narrow target cannot be left linear while the draw
runs under SINGLE_BUFFER(2). The costs are the CPU tile and untile of each
target once per kick (the shadow already behaves so), which for a
conformance-sized (about 64x64) target is small. The alternative of
splitting a mixed draw into one pass per target is rejected: depth,
stencil, occlusion queries and `gl_FragDepth` are per-draw, so a second
pass would depth-test against the first pass's writes.

**Resolve order.** Unchanged in kind, extended in count. `prepare` runs
for every target before the draw records anything (`backend.rs:2916`);
each target's shadow is tiled in when its storage is the newer copy;
`bind` marks each as written by this kick; `retire` untiles each when the
kick completes. The new rules:

- all shadows of one draw are prepared **together**: the LRU
  (`free_slot`, `wide_target.rs:276-288`) must not evict one it prepared
  for the same draw (stamp it with the draw's clock, which `prepare`
  already does, and never evict an entry of the current draw); `ENTRIES`
  is 8 (`:64`), the most a draw needs is 4 (a 128-bit attachment later
  takes 2 of them, `gc-128bit-render-targets.md`), so `finish()` at
  `:243-252` and `:259-263` stays the only fallback;
- a draw whose targets alias one storage (same texture level attached
  twice) is not served: the key is the storage's address, one shadow per
  address, so the second target would be the first's shadow. The GL core
  already refuses a feedback loop; refuse the alias here by name if it
  gets through;
- a sampler reading a target a kick still holds in a shadow goes through
  `kick_pending` first, as today (`wide_target.rs:36-42`).

**Clears.** A colour clear is a draw of the clear quad over the
framebuffer's targets. `clear_takes` and `rt_mask` (`backend.rs:499,1490`)
already give targets the clear does not take mask 0; with the shadows a
masked-out target is still tiled in and untiled (its pixels survive: the
mask is zero), which is correct but costs a copy. A cheaper variant: do
not give a target a shadow at all when `clear_takes` is false and the
draw is a clear... except that SINGLE_BUFFER(2) tiles it. So the clear
keeps every target in the draw. A depth or stencil clear alone has no
colour target (`target_gpu` 0 uses the dummy, `backend.rs:1686,1731`).

**Partial colour masks.** The mask is the same for every draw buffer
(`gl.color_mask`, `rt_mask`, `backend.rs:1490`). Slot 0 takes it through
PE_COLOR_FORMAT COMPONENTS (`:1666`), slots 1 to 3 through
`pe_halti5_rt_colormask_components` and OVERWRITE only at a full mask
(`:1719-1722`). Both already work for 32-bit targets. For a 64-bit one the
meaning of the four component bits is not known: RGBA16I has four channels
in 64 bits, RG32I has two 32-bit channels in the `G32R32F` format, so
whether its mask is bits 0-1 or bits 0 and 2 is the PE's, not ours. This
is the "random 79" part of the 63 (`fragment_out` random cases use masks):
it cannot be judged on the host. Blend: an integer target is never blended
(`rt_blend`, `backend.rs:1715`), so the alpha-config word of a 64-bit
integer slot is 0; slot 0 takes PE_ALPHA_CONFIG from `blend` as a whole
(`:1644-1657`), which is wrong when RT0 is integer and a float target
beside it wants blending: slot 0 would need the same `rt_blend` rule
(`blend && !gles_surf_is_integer(gl.target_format)`). That is a latent
defect of the 32-bit mixed path too; the step list covers it.

**The CPU view of targets 1 to 3.** `GlesDrawTarget` gains `cpu` and
`pitch` (the draw's `target_cpu` and `target_pitch` for slot 0 are the
only ones today). A field added to this struct means every literal changes:
`grep -rn 'GlesDrawTarget {' src tests` (CLAUDE.md, the 2026-10-14
failure). Known literals: `src/backend.rs:856` (`ZERO`),
`src/gl/draw.rs:2494,2501`, `src/vivante/mipmap.rs:149`,
`src/v3d/backend_tests.rs:3657,3659,3715,3716` (#330's file),
`src/pvr/backend.rs:5236`, `tests/es3_pvr_modf.rs:304`. A way to avoid
touching the V3D and PowerVR literals: a `..GlesDrawTarget::ZERO` in the
new fields, or a second array on `GlesDraw` (`target_cpus: [*mut c_void; 4]`,
`target_pitches: [u32; 4]`) that only the GL core and this back end read.
Prefer the array: `GlesDraw` is built by `GlesDraw::new` (`src/backend.rs:
1400`), one literal.

## (3) Smallest implementation sequence

Each step is one host-tested item. 1 to 3 are free of the GL core and
backend.rs; the rest name their owners. Nothing is switched on until
section 5: the new path ships behind an opt-in knob (`gc_debug_mixed64`,
the style of `gc_debug_msaa`), default off, so the
refusal stays the behaviour.

1. **Shadow for several targets, bpp-parametric** (`src/vivante/
   wide_target.rs`; the item `gc-rt128-shadow` is doing the several-target
   and 128-bit plane part, take it first and do not duplicate). Add the
   pixel size to `Key`/`Entry`; `tiled_offset(x, y, tw, bpp)`; `padded`
   unchanged; `stride(w, bpp)`; `prepare_all(draws)` that prepares up to 4
   keys together without evicting one of the set; `bind(slot)`;
   `retire` per entry's bpp. Host tests (unit, in the module, with the
   stand-in OSAL as `backend_tests.rs:1316` does): tile/untile round-trip
   for bpp 1, 2, 4, 8 at 17x13; two keys prepared together never evict
   each other with all 8 entries full; `retire` leaves a still-pending
   entry alone.
2. **A CPU view and pitch for every target** (`src/backend.rs`:
   `GlesDraw::target_cpus`, `target_pitches`, `GlesDraw::new`;
   `src/gl/draw.rs:2490-2505`, where `targets[]` is filled; `src/gl/
   draw.rs` is held by other plans, so this step waits for the file or
   becomes an issue for its owner). Host test: `src/gl/draw.rs` has a test
   next to `:13393-13416` that checks `target_cpu` of slot 0 is the
   level-0 storage; add the same for slots 1 to 3 of an MRT framebuffer,
   a mipmap level and a layer included. This is inert (nothing reads it).
3. **The back end programs every target on its shadow** (`src/vivante/
   backend.rs`: `wide_target(gl)` returns true for a draw that has a 64-bit
   target whatever `hw_targets(gl).1`; `emit_pe_state`: slot 0 and the
   8-RT bank at `:1706-1725` take the shadow's address and stride per
   target; `pe_key` `:2022` unchanged; the `draw_one` check `:2829` drops
   `n > 1 &&` when the knob is on; `prepare` call `:2916`). Small and
   separate from #333/#328's changes: one function `wide_targets(gl) ->
   ([Option<u64>;4])`. Host tests in `tests/es3_int_rt_mrt_gc.rs` (the
   stand-in GPU of its header writes a known pattern into the shadow the
   stream names): turn the three refusal tests into draw tests with the
   knob on (words: SINGLE_BUFFER 2, per-target `PE_RT_ADDR_8_PIPE`
   inside the shadows, `PE_RT_CONFIG_8` stride, `PE_COLOR_STRIDE`),
   RGBA8UI+RGBA16UI, RGBA16UI+RGBA16UI, RG32I+R32F, three targets with
   the wide one at each position, and a read-back of each from
   `glReadPixels` through `retire`. The refusal stays when the knob is off
   (existing tests unchanged, run under both).
4. **Clears** (`src/vivante/backend.rs` `emit_clear`: no change expected
   once 3 lands; the work is the tests). Un-`#[ignore]` the two clear
   tests of `tests/es3_int_rt_mrt_gc.rs:636-695` with the knob on; add a
   masked clear (`glColorMask`, then `glClearBuffer`) and a
   `glClear(GL_COLOR_BUFFER_BIT)` with a 32-bit attachment beside a 64-bit
   one, asserting the other attachment's pixels survive (the pattern of
   the stand-in GPU is checked to be untouched where the mask is 0).
5. **Slot-0 blend rule** (`src/vivante/backend.rs` `emit_pe_state`,
   `:1501,1644-1670`): slot 0 takes `blend && !integer(target_format)` as
   slots 1 to 3 do (`:1715`). Host test in `backend_tests.rs`: RT0
   RGBA8UI, RT1 RGBA8 with blending on: PE_ALPHA_CONFIG has no blend
   enable for RT0, the RT1 alpha config does. Independent of 3; also
   fixes the 32-bit mixed case, so it can go first.
6. **Mask words for 64-bit targets** (same function, `:1666,1719-1722`):
   the component mask of an RG32 target and an RGBA16 target. The words
   are a hypothesis until the board; the host test pins what the code
   writes for masks 0x1, 0x3, 0x5, 0xf per format and states the
   hypothesis in a comment. Do not enable by default (section 5).
7. **RGBA16F** joins (`wide_target.rs:46-49`): the same condition now
   admits it in a mixed draw; test with an RGBA16F + RGBA8 framebuffer.
   Gated by the same knob.
8. **GL-core side and the flip** (`src/gl/framebuffer.rs`, `src/gl/draw.rs`
   only if a name list needs it; none is known: the refusal is the back
   end's). After the board check: the knob defaults to on and the refusal
   code and its tests go; `docs/es3/int-rt-gc.md` and RC1 of
   `docs/es3-cts-failures.md` (the item `gc-docs`), then a board run of
   `dEQP-GLES3.functional.fragment_out.*` and `fbo.completeness`.

## (4) Host tests that pin it, summarised

| Step | Where | Pins |
|---|---|---|
| 1 | `src/vivante/wide_target.rs` unit tests | tiled round-trip at bpp 1/2/4/8, co-prepared keys never evict each other |
| 2 | `src/gl/draw.rs` test | slot 1 to 3 CPU view and pitch |
| 3 | `tests/es3_int_rt_mrt_gc.rs` | the words per target, the read-back of each, the knob off = today's refusal |
| 4 | same, un-ignored | single-attachment clears beside a 64-bit target |
| 5 | `src/vivante/backend_tests.rs` | RT0 integer is not blended |
| 6 | same | the mask words (hypothesis) |
| 7 | same | RGBA16F beside RGBA8 |

The stand-in GPU of `es3_int_rt_mrt_gc.rs` models the 4x4 tiling, so the
host tests check the shadow's layout, the padding and the copy-back, not
that the PE stores the right bits. Host dEQP image comparisons on
`GPU=vivante` are zeros (the Linux OSAL never runs the command stream), so
the dEQP count that can move on the host is refusals and completeness
only: the 63 `fragment_out` failures should become "image comparison
failed" (no longer refused), which is the success measure for steps 3 and
4, not a pass.

## (5) What needs the board before it is turned on

Opt-in until all of these are answered on an i.MX8MP (gl_test case,
built behind the knob, in `driver/samples/zephyr/gl_test/src`, which
`#328` holds):

1. **Does a tiled MRT slot honour the 4x4 tiling under SINGLE_BUFFER(2)?**
   The 8-RT bank has no tiled bit (`wide_target.rs:46`). Draw a 17x13
   RGBA8UI + RGBA16UI framebuffer with a constant in each, read both back
   through the shadows. Moved 16-byte pairs is gl_test 44's fault again.
2. **The stride word of slot 1 to 3.** `rt_config_stride` of the
   shadow's padded row (as given), or the PE_COLOR_STRIDE way. The probe
   draws a gradient and finds which one lands straight rows.
3. **Narrow targets in a tiled draw.** An RGBA8 and an R8UI target beside a
   64-bit one under SINGLE_BUFFER(2): the same bit-exact read-back, since
   the narrow ones were never measured tiled in the 8-RT bank.
4. **Depth and stencil with SINGLE_BUFFER(2).** The depth surface's layout
   under (2) is unmeasured (`wide_target.rs:49-53`, logged once at
   `backend.rs:2921`); a mixed draw with a depth test (dEQP
   `fragment_out` has none, `fbo.depth` combinations do) needs the answer
   or stays refused.
5. **The component mask of an RG32 and an RGBA16 slot** (step 6): masks
   0x1, 0x2, 0x5, 0xf, each read back; the shadow must keep the masked
   channels.
6. **`UIF32` keeps 32 bits in an MRT slot** (the open question of
   `gc-128bit-render-targets.md`), the same probe as 1 with values above
   2^24 and negative ones; it also gates the RGBA32I/UI planes.
7. The dEQP rerun on the board: `fragment_out` 63 fewer failures and the
   `fbo.completeness` renderable cases unchanged or better.

Until 1 to 6 hold, the knob stays off and the draw is refused by name as
today. A failure in 1 to 3 closes this design: the refusal stays and the
finding goes to RC1.

## Sources

Mesa etnaviv (MIT) facts are those of `gc-128bit-render-targets.md`
(`etnaviv_state.c:532-539` SINGLE_BUFFER, `etnaviv_blend.c:150-200` per-RT
masks and blend); only `third_party/mesa-26.2.1/UPSTREAM` and its
`include`/`src` headers are vendored, the driver sources were not
re-read for this note. No GPL kernel source was read.
