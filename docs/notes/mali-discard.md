<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): `discard`, the frame side and the compiler side

Date 2026-10-22, tip `bc17288cb`. Plan #152 item `notes-discard`, for step S6
of issue #1384 (`gh` was not run; the scope is `driver/docs/nanopc-t6-mali.md`,
"The general lowering (approved 2026-10-19)"). It is a reading of Mesa
26.2.1's source and of our tree; host research only, no file under `src/`,
`tools/` or `driver/` changed, no board run. What only the board can settle is
marked **pin on the board** and collected in section 6.

The compiler side of `discard` (demote to `DISCARD`, the `FCMP` fusion, DCE
and CSE) is in `docs/notes/mali-lowering-map.md` (section 4 row `discard`,
sections 5 and 6); this note cites it and adds what that map lacks: the
frame side (what a discarding program changes in the draw), the exact
helper-invocation and termination semantics on v10, and the board cases.
The early-z table and `shader_modifies_coverage` are first described for
alpha-to-coverage in `docs/notes/mali-frame-refusals.md` ("common ground" and
section 2); here they are read for `can_discard`.

## Citations

Mesa citations are `file:line` in the Mesa 26.2.1 tree (`MESA_PAN_TREE`,
`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, the tree the Mali gate uses).
Short names:

| Short | Directory |
| --- | --- |
| `G/` | `src/gallium/drivers/panfrost/` |
| `L/` | `src/panfrost/lib/` |
| `C/` | `src/panfrost/compiler/` (`B/` = `C/bifrost/`) |
| `X/` | `src/panfrost/genxml/v10.xml` |
| `N/` | `src/compiler/nir/` |

Our files are cited as `src/...:line` at `bc17288cb`. Mesa 25.0.7 is what the
board runs (the `0dce654a8:tools/mali/board-ref` recordings); no 25.0.7 tree was
available, so every Mesa fact below is 26.2.1's and "same on the board's
25.0.7" is open (section 6).

## Summary: what S6 changes

| Where | Today | For a program that discards | Mesa |
| --- | --- | --- | --- |
| program record | no bit (`ShaderMeta::can_discard` is hard-coded `false`, `src/mali/program.rs:147`) | a `can_discard` bit in `MaliShader` and `StageMeta`, handed to `ShaderMeta` | `info.fs.can_discard = uses_discard`, `C/pan_compiler.c:255` |
| `check_draw` | refuses `can_discard` (`src/mali/frame.rs:1832-1835`) | stop refusing it | n/a |
| `fs_required` | `mask != 0` (`frame.rs:2363`) | `mask != 0 \|\| can_discard` | `G/pan_cmdstream.h:205-208` (`sidefx`) |
| DCD0 bit 20 `shader_modifies_coverage` | not set (`frame.rs:2656-2682`) | set | `G/pan_csf.c:1610-1612` |
| DCD0 bit 0 `allow_forward_pixel_to_kill` | `!load_dest \|\| !fs_required` (`frame.rs:2659`) | cleared | `C/pan_compiler.c:269-271`, `G/pan_cmdstream.h:110` |
| DCD0 bits 4-5 `zs_update_operation` | `early` (`frame.rs:2655`, `:2662`) | FORCE_LATE when the draw writes depth or stencil, or an occlusion query counts it; else `early` | `L/pan_earlyzs.c:61-76` |
| DCD0 bits 2-3 `pixel_kill_operation` | `early` | `early`, unchanged | `L/pan_earlyzs.c:78-81`, `:111-112` |
| blend descriptor, DCD1 | tied to `fs_required` (`frame.rs:2379`, `:2685`) | a draw with colour mask 0 now runs the shader but keeps its blend descriptor OFF | `G/pan_cmdstream.c:296-305`, `G/pan_csf.c:1641-1646` |
| shader binary | `Discard` refused (`src/shaderc/online/mali/nir.rs:937`, `nir_fs.rs:916`) | `DISCARD.f32` (section 4) | `B/bifrost_compile.c:1979-1991` |

The program-info bit is one boolean. Unlike the S4 gl_FragCoord step, which
needed no new bit (section 3.3), discard needs the frame to know.

## 1. What `can_discard` is

- NIR: `uses_discard` is set by gathering `demote`, `demote_if`, `terminate`
  and `terminate_if` in a fragment shader (`N/nir_gather_info.c:482-493`; also
  `demote_samples`, `:897`). It is cleared and recomputed with the other
  gathered bits (`:1118`).
- panfrost's shader info (`pan_shader_update_info`):
  `fs.can_discard = uses_discard` (`C/pan_compiler.c:255`);
  `fs.sidefx = writes_memory || uses_discard` (`:261`);
  `fs.can_early_z = !sidefx && !writes_depth && !writes_stencil && !writes_coverage`
  (`:264-266`, a Midgard/Bifrost JM notion, see below);
  `fs.can_fpk = !writes_depth && !writes_stencil && !writes_coverage &&
  !can_discard && !outputs_read` (`:269-271`).
- So one `discard` changes **three** derived facts: `sidefx` (the draw needs a
  fragment shader even with no colour write), `can_fpk` (forward pixel kill is
  off), and `late_coverage` in the early-z analysis (section 2.2). It does not
  change `writes_global`, `allow_forward_pixel_to_be_killed`, the preload
  mask, the SPD's "requires helper threads" (`G/pan_cmdstream.c:4518-4519`,
  from `contains_barrier`, `C/pan_compiler.c:276`, which discard does not set)
  or any resource table.

## 2. The frame side, as Mesa 26.2.1 builds it

### 2.1 Where the fields are filled on v10

- Gallium (what the board's Mesa runs): `csf_emit_draw_state`,
  `G/pan_csf.c:1433-1673`. `fs_required` is computed at `:1441-1442`; when it
  is false the fragment shader registers are zeroed (`:1457-1464`). `DCD0` is
  built at `:1544-1636` in the `if (fs_required)` branch at `:1585`
  and the `else` at `:1615`. The struct is `DCD Flags 0`, `X:2118-2141`
  (bit 0 Allow Forward Pixel To Kill, bit 1 ... To Be Killed, bits 2-3 Pixel
  Kill Operation, bits 4-5 ZS Update Operation, bits 7-8 Overdraw Alpha0/1,
  bit 20 Shader Modifies Coverage, bit 22 Alpha To Coverage); the enum is
  `X:370-374` (Force Early 0, Weak Early 2, Force Late 3).
- The early-z choice is a lookup computed once per fragment program:
  `prog->earlyzs = pan_earlyzs_analyze(&prog->info, dev->arch)`
  (`G/pan_shader.c:389`), read at draw time through
  `pan_earlyzs_get(lut, writes_zs_or_oq, alpha_to_coverage, zs_always_passes,
  zs_read)` (`L/pan_earlyzs.h:61-68`, a `[2][2][2][3]` table).
- panvk's CSF path does the same with the same helpers and one extra
  condition: `shader_modifies_coverage = writes_coverage || can_discard ||
  alpha_to_coverage` (`src/panfrost/vulkan/csf/panvk_vX_cmd_draw.c:2143-2144`),
  `cfg.shader_modifies_coverage` at `:2214`, `pan_earlyzs_get` at `:2198-2203`,
  FPK at `:2180-2182`. (Its `hsr_*` fields at `:2282-2296` are v13 and not in
  `X`.)
- Not v10 and not used here: the JM path (`G/pan_jm.c:586-592`, `:626-627`);
  the pre-Valhall renderer state (`G/pan_cmdstream.c:500-540`, where
  `force_ez_with_discard`, `shader_contains_discard` and
  `shader_reads_tilebuffer` are Bifrost renderer-state bits;
  `L/pan_shader.h:152-153`).

### 2.2 The early-z analysis for a discarding shader

`analyze` (`L/pan_earlyzs.c:31-115`), read for arch 10 (`should_force_early_update`
is `arch >= 11`, so false, `:125`), no `early_fragment_tests`, no
`writes_global`, the ZS tile buffer not read, a shader that discards and
writes no depth, stencil or coverage:

- `late_update = shader_writes_zs || alpha_to_coverage` is false (`:49-50`);
  `late_kill = shader_writes_zs` is false (`:51`).
- `late_coverage = writes_coverage || can_discard || alpha_to_coverage` is
  **true** (`:61-62`; "Discards are implemented as coverage mask updates and
  must be considered", `:57-58`).
- `late_update |= late_coverage && writes_zs_or_oq` (`:76`): "Late coverage mask
  updates may affect the value written to the depth/stencil buffer (if a pixel
  is discarded entirely)... they do not affect depth/stencil testing"
  (`:64-71`), "occlusion queries with late coverage also require a late
  update" (`:72-75`).
- `late_kill |= writes_global` (`:81`) stays false: a discard does not delay
  the *kill*.
- Result (`:107-114`): `kill = best_early_mode(zs_always_passes, false)` =
  WEAK_EARLY if the depth/stencil test always passes, else FORCE_EARLY
  (`:17-24`), exactly what our `early` is today; `update = FORCE_LATE` if
  `writes_zs_or_oq`, else the same `best_early_mode`.

So for the state our draws can have:

| depth/stencil written, or an OQ counting | `zs_always_passes` | `pixel_kill_operation` | `zs_update_operation` |
| --- | --- | --- | --- |
| no | yes | WEAK_EARLY | WEAK_EARLY |
| no | no | FORCE_EARLY | FORCE_EARLY |
| yes | yes | WEAK_EARLY | FORCE_LATE |
| yes | no | FORCE_EARLY | FORCE_LATE |

(The first two rows are what a non-discarding shader gets in every row; the
`or_oq` term is the OQ of case 196, section 2.4.) `writes_zs` and
`zs_always_passes` are the ZSA CSO's (`G/pan_cmdstream.c:4290-4291`):
`writes_zs = util_writes_depth_stencil` = depth written (test on, write mask
on, func not NEVER) or either stencil face writing (enabled, write mask
non-zero, some of fail/zfail/zpass not KEEP) (`src/gallium/auxiliary/util/
u_inlines.h:961-982`); `zs_always_passes` is `pipe_zs_always_passes`,
`G/pan_cmdstream.c:4207-4220`.

The stencil case is the one femtovg's strokes meet: the first pass of
`StencilStroke` increments the stencil where the AA fringe is *not* discarded
(case 38, section 5); a discarded fragment must not increment, which is why
the update is late and the stencil write waits for the coverage the discards
leave. The kill (the early depth/stencil *test*) is unaffected: a discarded
fragment may still be killed early by the test, its test never depends on the
discard.

### 2.3 What else a discard moves in the draw

- `fs_required` (`G/pan_cmdstream.h:199-235`): `sidefx` returns true first
  (`:205-208`, "This includes use of discard, which can affect the results of
  an occlusion query"). A draw with colour mask 0 and no depth write from the
  shader **still runs the fragment shader** if it can discard. That is the
  colour-masked stencil pass of case 38. The consequences in `csf_emit_draw_state`
  follow the `fs_required` branches: the fragment SRT/FAU/SPD registers are
  written (`:1457-1459`), the secondary (varying) shader is used
  (`:1443`), DCD0 takes the `fs_required` branch (`:1585`), DCD1's
  `render_target_mask` is `(outputs_written >> FRAG_RESULT_DATA0) &
  fb_rt_mask` (`:1641-1646`), so 1 for a shader that writes colour 0, even
  when the draw's write mask is 0.
- The blend descriptor does **not** follow `fs_required`; it follows the
  blend state: `panfrost_emit_blend` packs `enable = false`, mode OFF for a
  target whose `info[i].enabled` is false (`G/pan_cmdstream.c:296-305`), and
  `enabled` is the colour write mask being non-zero. So a mask-0 draw with a
  discard has the shader, the RT mask 1 and a *disabled* blend descriptor.
  Our `emit_draw` ties both to one flag (`frame.rs:2379`: `let bd = if
  fs_required {..} else {"Disable blending"}`; `frame.rs:2685`: the RT mask
  from `fs_required`): S6 splits `fs_required` from "blend enabled".
- Forward pixel kill: `pan_allow_forward_pixel_to_kill` is
  `can_fpk && !(rt_mask & ~rt_written) && !alpha_to_coverage &&
  !blend_reads_dest` (`G/pan_cmdstream.h:98-112`); `can_fpk` false for a
  discard (`C/pan_compiler.c:269-271`), so DCD0 bit 0 is **false** for every
  discarding draw whatever the blend. `allow_forward_pixel_to_be_killed` is
  `!writes_global` (`pan_csf.c:1597`), true.
- Overdraw alpha: `panfrost_overdraw_alpha` is about the blend equation
  (`G/pan_cmdstream.h:122-139`), not the shader; unchanged.
- `shader_modifies_coverage` (bit 20): `writes_coverage || can_discard ||
  alpha_to_coverage` (`G/pan_csf.c:1607-1612`; comment "Unlike Bifrost,
  alpha-to-coverage must be included in this identically-named flag").
- The depth-only `else` branch (no fragment shader, `:1615-1635`) is what our
  mask-0 draws build today: FORCE_EARLY both, FPK on. A discarding shader
  must not take it.
- Nothing else in the frame depends on it: no framebuffer descriptor field,
  tiler context, ZSD or blend descriptor field is derived from `can_discard`
  (the only users of the info bit on this path are `pan_csf.c:1610-1611`,
  `pan_earlyzs.c:62`, `pan_compiler.c:255,261,270` and
  `pan_cmdstream.h`'s `sidefx`; `pan_fb.h:546`'s `can_discard` is the
  framebuffer-shader ops' own).

### 2.4 Interaction with depth/stencil writes, blending, occlusion queries, MSAA

- **Depth/stencil writes**: the update becomes late (table in 2.2). The
  `writes_zs` term must be Mesa's (`util_writes_depth_stencil`), not "the test
  is on": our `DepthStencilState` (`frame.rs:985-992`) holds `depth_write`
  (GL's mask only when the test is on) and two `StencilFace`s with `write_mask`
  and the three ops (`frame.rs:953-961`; an off face is `StencilFace::off`:
  mask 0, KEEP). The depth func NEVER exclusion (`u_inlines.h:970-974`) has
  no counterpart in `uses_attachment` (`frame.rs:1009-1011`).
- **Blending**: no field depends on blending except FPK (above, off anyway)
  and the blend-shader refusal that exists today (`NOT_BUILT_BLEND_SHADER`,
  `frame.rs:1860`). The fixed-function blend is applied to the surviving
  coverage by the hardware; Mesa has no blend-side code for discard.
- **Occlusion queries**: `has_oq` joins `writes_zs` in the lookup key
  (`pan_csf.c:1587`). A counting draw of a discarding shader therefore has a
  late update; a counting draw of a non-discarding shader does not (section
  2.2 of this note; `frame.rs:2670-2676` documents the non-discard case, whose
  comment "which `check_draw` refused" is the one that stops being true).
- **Alpha-to-coverage** (#1386 `frame-a2c`) feeds the same two terms
  (`late_update`, `late_coverage`, `shader_modifies_coverage`) with the same
  formulas; the two items must share one computation. A discard with a2c is
  just both.
- **MSAA**: `multisample_enable` and `evaluate_per_sample` are independent of
  `can_discard` (`pan_csf.c:1561`, `:1568-1570`). A discard on a 4x target
  clears the coverage bits of that fragment's samples; the sample mask in DCD1
  is untouched. Whether a 4x discard draw is right on the board is unpinned
  (section 6).

## 3. Where our back end sets these fields today

### 3.1 The refusal

`ShaderMeta` (`src/mali/frame.rs:523-`) carries `can_discard`, `writes_depth`,
`writes_stencil`, `writes_coverage`, `writes_global` ("Only none is built",
`:559-565`). `program.rs:134-151` builds it with all five hard-coded `false`
(`:147-151`) because the compiler refuses every one of them
(`MaliError::Discard`, `src/shaderc/online/mali/mod.rs:174`, text `:358`;
raised at `nir.rs:937` and `nir_fs.rs:916`). `check_draw` refuses a program
that has any of the five (`frame.rs:1832-1835`, `NOT_BUILT_FS_EFFECTS`,
`:140-141`).

### 3.2 The DCD0 and what S6 touches

`emit_draw` (`frame.rs:2298-`) builds DCD0 at `:2656-2682`:

| Field | Today | Why | S6 |
| --- | --- | --- | --- |
| `allow_forward_pixel_to_kill` | `!load_dest \|\| !fs_required` (`:2659`) | blend reads the destination (`pan_cmdstream.h:98-112`) | `&& !can_discard` (always false for a discard) |
| `allow_forward_pixel_to_be_killed` | true (`:2660`) | `!writes_global` | unchanged |
| `pixel_kill_operation` | `early` (`:2661`) | `best_early_mode` (`:2655`) | unchanged (2.2) |
| `zs_update_operation` | `early` (`:2662`) | same | FORCE_LATE if (depth/stencil written \|\| `occlusion_query.is_some()`) && `can_discard`, else `early` |
| `shader_modifies_coverage` | default `false` (`..Default::default()`, `:2681`) | no coverage effects built | `can_discard` (`writes_coverage` and a2c join later) |
| `occlusion_query` | PREDICATE or DISABLED (`:2676`) | case 196 | unchanged |

and the draw-level `fs_required` (`:2363`, `let fs_required = mask != 0`),
which gates `blending` (`:2364`), `load_dest` (`:2376`), the blend descriptor
(`:2379`, else `:2415-2431`), `fs_state` (`:2471`), the secondary program
(`:2576`) and the RT mask (`:2685`). Its comment says "`check_draw` refused
every side effect": that stops being true. The change is
`fs_required = mask != 0 || fs.meta.can_discard`, with a separate
`colour_written = mask != 0` kept for the blend descriptor (`bd`, `blending`,
`load_dest`) and `rt_mask` taking `fs_required` as Mesa does (2.3).

**Exactly which field must change** for a program that discards, in order of
risk: (1) `shader_modifies_coverage` set; (2) `allow_forward_pixel_to_kill`
cleared; (3) `zs_update_operation` FORCE_LATE when depth/stencil is written or
an OQ counts; (4) `fs_required` true for a mask-0 draw. Nothing else in DCD0,
DCD1, the FBD or the descriptors moves. (1) and (2) are bits of the flags word
for every discarding draw; (3) only moves for draws with zs writes or an OQ;
(4) only for mask 0.

The preload programs (`frame.rs:2085-2088`, `:2165-2168`) are not user
shaders and are unaffected.

### 3.3 The program-info bit, compared with S4's

S4's `gl_FragCoord` (`a52638c5`) added no field the frame reads: the preload
of r59 is the shader's own `preload` mask, which already travels from
`MaliShader::preload_r48_r63` (`src/shaderc/online/mali/mod.rs:539`) through
`StageMeta.preload` (`src/mali/program.rs:111`, `:131`) into the SPD. The
Mesa reference of every discard program has the same fragment preload as a
non-discarding one: `0x3000000000000000` (r60, r61: the coverage mask and its
companion that ATEST reads) in `fvgtest_discard2`, `fvgtest_discard` and
`fvgtest_green` alike (`tools/mali/testdata/*.expect.json`, fragment
`preload`).

Discard needs a new bit because `can_discard` is an input of the *draw*, not
of the shader's registers. The path is the one `helper_threads` already
takes: `MaliShader.helper_threads` (`mod.rs:533`, set where the compiler knows,
`:959`, `:1095`) -> `StageMeta.helper_threads` (`program.rs:122`) ->
`ShaderMeta.requires_helper_threads` (`program.rs:148`). A `can_discard: bool`
beside it, set when the lowering emits `DISCARD`, read at `program.rs:147`
instead of `false`. `ShaderMeta` already has `can_discard`, so its literals in
`src/mali/frame_*_tests.rs` stay; `MaliShader` and `StageMeta` literals gain the
new field (CLAUDE.md: `grep -rl 'MaliShader {' src tests` before the push).

## 4. The compiler side: what the lowering map lacks

### 4.1 Discard is demote on v10, and there is no `halt`

- GLSL `discard` is `nir_discard_if(cond)` / `nir_discard()`
  (`src/compiler/glsl/glsl_to_nir.cpp:812-825`), which build `demote_if` /
  `demote` when the compiler option `discard_is_demote` is set and
  `terminate_if` / `terminate` otherwise (`N/nir_builder.h:2367-2383`;
  option `N/nir_shader_compiler_options.h:864-867`). Mesa's options for arch
  9 and 10 set it (`B/bifrost_compile.h:151`, selected at
  `C/pan_compiler.c:93-97`).
- `nir_lower_terminate_to_demote` (`B/bifrost_nir.c:406-416`) therefore finds
  no `terminate` in a GLSL ES shader. It matters only for SPIR-V; the lowering
  map's row ("`nir_lower_terminate_to_demote`") is true but the S6 author need
  not implement it, and **need not implement a `halt`**: it would turn
  `terminate_if(c)` into `demote_if(c); if (c) halt` and delete everything
  after an unconditional `terminate` (`N/nir_lower_terminate_to_demote.c:
  10-62`, rationale `:96-104`). With `discard_is_demote` the code after a
  `discard` still exists in NIR and runs for the lane.
- NIR semantics of `demote`: the invocation continues as a *helper* (its
  stores and the colour are dropped; derivatives still see it). For a
  fragment of femtovg this changes nothing visible except which lanes the
  implicit-LOD look-ups see, section 4.3.

### 4.2 `DISCARD.f32` only flags the thread

- ISA text (`B/valhall/ISA.xml:829-840`): "Evaluates the given condition, and
  if it passes, discards the current fragment and terminates the thread." It
  has no destination and one `.cmp` (`unused`, `unit="CVT"`).
- Mesa's own comment is more precise and is what the passes implement:
  "The DISCARD instruction just flags the thread as discarded, but the actual
  termination only happens when all threads in the quad are discarded, or
  when an instruction with a .discard flow is encountered (Valhall)"
  (`B/bifrost_nir.c:406-410`). **Answer to "ends the thread or only masks the
  lanes": it masks the lane (flags it) and the thread keeps executing the
  instructions that follow, including ATEST and BLEND, until a `.discard`
  flow bit or the end of the quad.** The hardware then gives a flagged lane
  no coverage: ATEST reads the cumulative coverage (r60, preloaded;
  `load_cumulative_coverage_pan`, `B/bifrost_compile.c:1622-1624`;
  `C/pan_nir_lower_fs_outputs.c:122-150`) and the blend is a no-op for a lane
  that is not covered.
- The `.discard` flow (`VA_FLOW_DISCARD`) is a different thing: it
  terminates *helper* invocations (lanes that are only there for
  derivatives) and is placed by the helper analysis (4.3), not by `DISCARD`.
  In the references it shows as a suffix: `DISCARD.f32.lt.discard r0^, u4.w0`
  in `tools/mali/testdata/fvgtest_discard2.expect.json:120` is `DISCARD.f32`
  carrying the program's start-of-shader flow discard, merged onto it by
  `merge_discard` (`B/valhall/va_merge_flow.c:182-222`, "Discard may be moved
  down" `:17`; our `flow.rs:293`), not a second kind of discard.
- Opcode forms: `demote_if` is `DISCARD.b32` (`B/bifrost_compile.c:1979-1987`,
  a 16-bit condition is read as its low half); `demote` is
  `DISCARD.f32 0, 0 .eq` (`:1989-1991`), an unconditional discard. The
  fusion `DISCARD.b32(FCMP.f32 x, y) -> DISCARD.f32(x, y, cmpf)` and the
  lowering of a leftover `DISCARD.b32` to `DISCARD.f32 x, 0 .ne` are the
  lowering map's rows (`B/bi_opt_mod_props.c:108-134`, `:189-198`, `:501-518`),
  not repeated here. femtovg's `strokeAlpha < strokeThr` becomes one
  `DISCARD.f32 .lt` (both references show it: `fvgtest_discard.expect.json`
  has `DISCARD.f32.lt r0, u10.w0`, the compare of the product against
  `frag[10].y`).

### 4.3 Side effects, scheduling, helper analysis

- `DISCARD` has a side effect without a destination: `bi_side_effects` keeps it
  through DCE (`B/bir.c:175-192`); it is **never CSE'd** (`B/bi_opt_cse.c:
  117-124`, only `DISCARD_F32` is named there, the `.b32` having been
  lowered by then, section 6 of the lowering map).
- The pre-RA scheduler (`B/bi_pressure_schedule.c`, our `sched.rs`): a
  `DISCARD.f32` is serialised against ATEST and the coverage chain
  (`:136-139`) and against memory loads and stores (`:141-149`). In the
  fragment shaders here, with no memory writes, the practical rule is: a
  `DISCARD` stays before the `ATEST` and keeps its order relative to other
  `DISCARD`s.
- `va_insert_flow_control_nops` (`B/valhall/va_insert_flow.c:387-506`):
  ATEST is waited for slots 0, 1, 2 and 6 before it and slot 0 after it
  (`:443-446`; comment `:419-426`: "For discarded thread handling, ATEST must be
  serialized against all other asynchronous instructions"), unchanged by a
  discard shader, already ported (`flow.rs:10-21`). A fragment shader that
  needs no helper invocations discards helpers at the *start* of the program
  (`:497-505`); one that does (an implicit-LOD look-up) discards them after
  the last such instruction (`:470-480`) and at the start of blocks entered
  from a block that still needs them (`va_discard_before_block`, `:346-368`).
  That analysis does not look at `DISCARD` at all: `bi_instr_uses_helpers`
  (`B/bi_helper_invocations.c:66-97`) is texture, derivative and `WMASK`
  instructions only.
- **Is a discarded lane a helper for later look-ups?** After `DISCARD` the
  lane is flagged but still executes (4.2). An implicit-LOD `TEX` later in
  the program reads derivatives from its quad neighbours, discarded or not, so
  a firing discard does not change the LOD of the surviving lanes. This is
  the demote semantics NIR wants and what the references encode:
  `filter_image` has computed-LOD look-ups after its (never-fired) discard.
  Whether the hardware keeps a *flagged* lane's registers valid for the
  neighbours' derivatives on silicon is **pin on the board** (section 6).

### 4.4 Non-uniform control flow after a discard

What Mesa does is nothing special: the `DISCARD` is an ordinary instruction
in the block, the lane stays alive, and `if` bodies after it run as usual for
the lanes that take them (reconverge flow bits as for any branch,
`va_insert_flow.c:490-493`). The shim reference of the full-body discard
program shows it: in `tools/mali/testdata/fvgtest_discard.expect.json` the
`DISCARD.f32.lt r0, u10.w0` (line 202) sits in the entry block and the two
scissor `BRANCHZ.eq.reconverge` follow it, with no branch around the rest of
the program for the discarded lanes; the ATEST/BLEND tail (`ATEST.wait0`,
`BLEND ... .end`) is reached by every lane. A discard inside an `if` is a
`DISCARD` in that block; the peephole never flattens an `if` that contains
one (lowering map section 1, `N/nir_opt_peephole_select.c:227-232`), so it is
a real block, the S5 block machinery. GLSL IR's own comment says that after
its discard lowering "all the discards will be immediately followed by a
return" (`glsl_to_nir.cpp:814-819`); with `demote` there is no `halt` and
nothing for the back end to see, so the shape the S6 emit sees is a
`demote_if` (the case in both references) or an `if` around a `demote`.
Whether a conditional `discard` ends up as one or the other for the 14 femtovg
programs is **measured by the shim reference**, not by this note (section 6).

## 5. Which programs discard, and which case checks them

**femtovg** (`tools/shaders/femtovg/`): the discard is one line,
`if (strokeAlpha < strokeThr) discard;` (`fill_color.frag:200`, line 200 in
all 15 `.frag` files that are not `.nodiscard`), inside `#ifdef EDGE_AA` / `#if SELECT_SHADER != 6`
/ `#ifndef NO_DISCARD`. Every `.frag` file that is not `.nodiscard` has it
except `texture_copy_unclipped` (`SELECT_SHADER 6`, no discard; the map has the
same finding): 14 programs, `fill_color` and its `.glyph`, `fill_gradient`
(+glyph), `fill_gradient_conic` (+glyph), `fill_image` (+glyph),
`fill_image_gradient` (+glyph), `fill_image_gradient_conic` (+glyph),
`stencil`, `filter_image`. Their `.nodiscard` siblings and
`texture_copy_unclipped` do not discard and are S4/S5 programs. The renderer
picks the discard variant only for the second stroke pass
(`strokeThr >= 0`; the comment at `:195-199`). In the lowering map's Part 2 the
discard column says `yes` for the rows it classifies (`fill_color`, `stencil`,
`filter_image`, `fvgtest_discard`, `fvgtest_discard2`); the other eleven are
the same line and the same S6 class, plus whatever else they need (texture,
`gl_FragCoord`).

**fvgtest** (`tools/shaders/fvgtest_*.frag`): `fvgtest_discard2` is the
smallest (one `if (ftcoord.y < -100.0) discard;` then `gl_FragColor =
innerCol`; reference `fvgtest_discard2.expect.json`, 8 fragment words, the
first `DISCARD.f32` reference); `fvgtest_discard` is the stroke and scissor body plus the
femtovg discard line (`fvgtest_discard.frag:34`) (reference `fvgtest_discard.expect.json`). The `expect.json` files for
both exist and are the shim's output; the host byte-equality test is S6's
acceptance.

**Board cases** (`driver/tools/pvrlab/gl_test.c`; the Mali sweep of
2026-10-22, `0dce654a8:tools/mali/board-ref/2026-10-22-s4-fragcoord/ours/
06-panthor-gltest--sweep-2.log`: all five REFUSED today, the first three with
"control flow (if, a loop, a return before the end) is not lowered yet"):

| Case | What it checks | State it needs | Today |
| --- | --- | --- | --- |
| 32 `fvgdis` | `fvgtest_discard`, strokeThr -1 so the discard never fires: "the discard USC is fine" | colour on, no depth | REFUSED (control flow) |
| 33 `fvgdis2` | `fvgtest_discard2`: a never-taken discard must not kill fragments | colour on | REFUSED (control flow) |
| 38 `fvgstencil` | femtovg's stencil-stroke sequence with a firing discard: pass (1) stencil INCR with colour and strokeThr 0.998, (2) EQUAL 0 with strokeThr -1, (3) a **colour-masked** pass with a discard-capable shader that zeroes the stencil (gl_test.c:5822-5840, the cluster bug) | stencil write + discard (late update, 2.2) and mask 0 + discard (`fs_required` by `sidefx`, 2.3) | REFUSED (control flow) |
| 73 `branchlane` | a discard behind a test of lane y and one behind lane w (an `if` containing a `discard`) | colour on | REFUSED (arithmetic other than `+ - *`) |
| 81 `branchquad` | two discards behind tests of lanes y and w of a varying, every quadrant takes its own branches | colour on | REFUSED (arithmetic) |

Case 38 is the only board check of the two frame-side facts that cost a
field (late stencil update; a mask-0 draw that runs a discard shader) and
the only one that exercises a *firing* discard together with depth/stencil.
Cases 32 and 33 never fire; 73 and 81 fire per lane. The gl_test cases for
`discard` with a bounded loop are item `gltest-cases3` (not written yet).
No Mali board record of any discard program exists
(`git grep DISCARD 0dce654a8 -- tools/mali/board-ref`: none): Mesa's DCD0 for these draws has
not been dumped.

## 6. Open: facts that need a board pin

Each is a prediction from the source, to be written before the run
(`tools/mali/mesa-board-ref.sh`, then our descriptor pinned to its dump, as
`docs/notes/mali-frame-refusals.md` does for a2c).

1. **r57 of a discarding draw on the board's Mesa 25.0.7.** Prediction: the
   non-discard value of the same draw with bit 0 cleared, bit 20 set
   (`+0x100000`), and bits 4-5 = 3 only when depth/stencil is written or an OQ
   counts. The a2c recording shows the same bits for a2c (`0x5901BA`:
   FPK off, bit 20 and 22, update Force Late, `mesa-a2cconst.pandecode.xz:1395`
   per `mali-frame-refusals.md`) and 25.0.7 is not 26.2.1's `pan_earlyzs.c`
   (`blend.rs` and `frame.rs` already note where the two differ). **Run**: the
   case-38 draws and `fvgtest_discard2`, one frame each, under
   `PAN_MESA_DEBUG=dump`.
2. **Mask-0 draw with a discard shader**: that Mesa runs the fragment shader
   (SPD non-zero), packs RT mask 1, the blend descriptor OFF (2.3). From the
   same dump (pass (3) of case 38).
3. **Late update on silicon**: that a firing discard in the stencil INCR pass
   does not increment the stencil on the T6. Case 38 pass 1 checks it; no
   Mali run exists (case REFUSED today).
4. **Discarded lanes as helpers** (4.3): a texture look-up with computed LOD
   after a firing discard, derivatives from a discarded neighbour. Not needed
   for femtovg's programs (their discard fires before the look-up only on
   fringe pixels, and the colour of the discarded lanes is dropped), so do
   not gate S6 on it. A one-case probe if ever needed.
5. **Conditional form** (4.4): the NIR shape of femtovg's `if (a < b) discard;`
   after Mesa's pass loop for each of the 14 programs (`demote_if` fused to
   one `DISCARD.f32` as in both references, or an `if`-wrapped `demote`):
   read the shim's NIR or the `expect.json`; for the other 12 programs no
   reference with a discard exists yet (the `shim-refs` item produced the
   discard/discard2 pair and the `.nodiscard` family).
6. **4x with a discard**: coverage bits of a discarded fragment on a 4x
   target (DCD1 sample mask untouched). No case; add one only if S6 meets it.

## 7. Checklist for the S6 author

1. `MaliShader.can_discard` and the same on `StageMeta`; `program.rs:147`
   reads it (3.3).
2. `check_draw`: drop `fs.can_discard` from the refusal at `frame.rs:1833`; the
   other four stay refused (`writes_depth`, `writes_stencil`,
   `writes_coverage`, `writes_global`).
3. `emit_draw`: `fs_required = mask != 0 || can_discard`; keep the blend
   descriptor, `blending`, `load_dest` on `mask != 0`; the RT mask and
   `fs_state` on `fs_required`; DCD0 as in 3.2. One function for the early-z
   inputs shared with `frame-a2c` (`late_coverage`, `writes_zs_or_oq`,
   `shader_modifies_coverage`).
4. Unit tests: the descriptor words for the four rows of the table in 2.2 and
   the mask-0 draw, against the bit predictions of section 6 item 1 (no Mesa
   recording to compare yet).
5. The compiler: `demote_if` -> `DISCARD.b32`, `demote` -> `DISCARD.f32 0,0
   .eq`, the fusion and the leftover lowering (lowering map section 4,
   section 6), `bi_side_effects`, no CSE, the scheduling rule of 4.3; the
   program's `can_discard` set from the emit. The byte-equal gate is
   `fvgtest_discard2.expect.json` first, then `fvgtest_discard`.
6. A discard inside an `if` needs S5's blocks (4.4).
7. Update `tools/cts/RESULTS.md` and #1384 with the board cases 32, 33, 38
   (and 73, 81 once their arithmetic is lowered).
