<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_FragDepth: the contract on the GC7000UL, V3D and PowerVR back ends

Written from `origin/main` at `de737051`, early-Z and V3D test
paragraphs refreshed at `007ac36e`. Function and constant names are
the address; line numbers drift. Nothing here is measured on a board: the
dEQP-GLES3 `shaders.fragdepth.write` (8) and `.compare` (7) cases were
refused at compile on every GPU, and this note says what each back end now
compiles and which state it sets, and what is still open.

## 1. The front end

`gl_FragDepth` lowers to an `IoKind::Depth` output of one lane
(`src/shaderc/online/lower.rs`, the `(Stage::Fragment, "gl_FragDepth")`
entry; the IR slot is `IoSlot::FragDepth`, `ir.rs`). The eight dEQP writers
are pinned as such by `tests/es3_shaderc_repro.rs`
`fragdepth_writes_lower_to_a_depth_output`. Its doc comment still says the
three back ends refuse the output; that is stale since the commits below
(plan #338 holds the file).

## 2. GC7000UL (`70a2050a`, `5fa91ff6`)

Compiler, done:

- `select.rs` lowers `StoreOutput { slot: FragDepth }`: `Fmax` against 0,
  then `Fmin` against 1 (the value is clamped to [0, 1], ES 3.00 7.2), the
  `Fmin` being the last instruction and the only write of the depth
  register, lane `.z` alone (`FS_DEPTH_REG` = `FS_FRAGCOORD_REG` = r0,
  `FS_DEPTH_WRITE_MASK` 0b0100). Mesa etnaviv pins the depth output to the
  same register.
- `Selected::ps_depth_out_reg` is `Some(FS_DEPTH_REG)` when a depth store
  was selected (`select.rs`, `ps_depth_out_reg: s.writes_depth.then_some(..)`);
  `abi.rs` writes it to the shader record as `viv.ps_depth_out_reg`
  (`SHC_VIV_NO_REG` when absent). `codegen.rs` sets `ShcShader::writes_depth`
  from it, the flag the PowerVR program carries too.
- `gl_FragCoord` shares r0 with the depth. A shader that does both reads
  `gl_FragCoord` from a copy of r0 made before the first instruction
  (`select.rs`, "gl_FragDepth and gl_FragCoord share r0"), in a loop too.

Early Z, done (`b3f42d14`): `src/vivante/early_z.rs` holds the rule as a
pure function (`early_z`, :131, from Mesa's `etna_update_zsa`);
`EarlyZInput::writes_z` ("the fragment shader writes depth") turns the early
test and the early write off. `draw_early_z(writes_z)` (:186) is the rule for
a linear target. `src/vivante/backend.rs` calls it with
`d.fs.viv.ps_depth_out_reg != SHC_VIV_NO_REG` in `emit_pe_state` (:1559,
only the `GC_PE_DEPTH_EARLY_Z` bit of `pe_bits`) and `emit_ra_state` (:1833,
the whole `GC_RA_EARLY_DEPTH` word, with a `debug_assert_eq!` against the
old constant). Every colour surface the back end renders to is linear, which
the rule denies for every program, so the emitted words are the same with and
without a depth write: `RA_EARLY_DEPTH` 0x15000030 (WRITE_DISABLE, no
TEST_ENABLE) and no `GC_PE_DEPTH_EARLY_Z`; a depth-writing program differs
from the others only on a tiled target, where the rule gives the same words
but with early test and early write off (below). The back end emits no PE
"depth from the pixel shader" word; the depth comes from the
`ps_depth_out_reg` in the shader record (whether the hardware needs a
further bit is open: Mesa etnaviv is not read into this tree for it).

Tests: `tests/es3_fragdepth_gc.rs` pins the build, r0 as the depth register,
the write to `r0.z` alone by the last instruction, the clamp, the
`gl_FragCoord` copy and the dEQP shaders. For the words:
`early_z_is_off_for_a_depth_writing_program_and_on_for_one_that_is_not`
derives `writes_z` from the compiled record and pins the rule on a tiled
target (the writer: early test and write off, `RA_EARLY_DEPTH` 0x15000030,
no PE bits; a program without a depth write keeps them, e.g. 0x05000031 with
PE bits 0x01010000); `the_back_end_words_say_early_z_off_for_both_programs`
pins what `draw_early_z` gives the back end;
`a_linear_target_keeps_early_z_off_for_both_programs` pins the linear case.
`early_depth_is_never_enabled_and_the_rasteriser_never_writes_z`
(`src/vivante/backend_tests.rs:1750`) pins the words of a submitted draw.
The tests call the rule and the submitted words, not a tiled draw through the
back end.

## 3. V3D (`8a5da05e`, `de737051`)

- A shader that writes `gl_FragDepth` writes its own depth to the TLB,
  per pixel: `mov tlbu, <depth>` with `TLB_Z_PER_PIXEL` (0xffffff8c; Mesa's
  `emit_frag_end`, nir_to_vir.c), after the last thread switch and before
  the colour writes, in place of the pass-through Z from the FEP
  (`src/shaderc/online/v3d/select.rs`, `select_fragment`). The depth is not
  clamped by the compiler, as in Mesa; the clamp is left to the hardware and
  is a board measurement (fbo.depth write_clamp).
- `disable_ez = discard || rt_mask == 0 || depth.is_some()` and
  `writes_z = disable_ez || reads_z`, so a depth write sets both
  (`select.rs`, same function). `chain.rs` carries them into the program
  data (`FsProg::writes_z`, `disable_ez`; `ShcV3dFs` in `src/shaderc/mod.rs`,
  offsets pinned in `src/abi_layout.rs`).
- `prepare.rs` handles the depth as a value carried through control flow
  (a conditional write, a loop-carried `gl_FragDepth += x`); `de737051`
  makes a uniform `break` check the depth phi so a depth written before it
  leaves with the pass.
- Where the words land: `src/v3d/backend.rs` (`programs`) copies them into
  `V3dFsProg` (`ffi.rs`), and the C driver puts them in the shader state
  record, `fragment_shader_does_z_writes = fs->writes_z` and
  `turn_off_early_z_test = fs->disable_ez` (`driver/src/v3d/v3d_job.c` and
  `v3d_frame.c`, "emit_shader_state_record"). The wiring of the shader record
  was there before `8a5da05e`; the compiler now fills the fields. Config-word
  and job-level early-Z handling beyond the record is in
  `docs/notes/v3d-early-z.md`; `record` in `backend.rs` marks `z_written`
  from the GL depth state (`state::zs_use`), not from the program.
- Tests: `tests/es3_fragdepth_v3d.rs` builds and links the eight dEQP
  writers, pins one per-pixel TLBU depth write and its config word before the
  colour, and runs the QPU interpreters for the written value (constant,
  uniform, varying, conditional, loop-carried, before a uniform or a
  divergent `break`, with a discard).
  `tests/es3_fragdepth_v3d_state.rs` compiles the writer, hands its program
  data to the real C job builder and pins the shader record: "fragment shader
  does Z writes" (bit 8) and "turn off early Z test" (bit 9) set for the
  writer and clear for a control, the two records differing in those bits
  alone (`the_depth_writers_shader_record_says_writes_z_and_early_z_off`,
  `the_two_records_differ_in_the_two_bits_alone`); the CFG_BITS early-Z bits
  and the RCL early-Z-disable stay as for every draw
  (`cfg_bits_keep_early_z_off_for_both_programs`,
  `the_rcl_common_packet_disables_early_z_for_both_programs`: the job builder
  never turns early Z on). The Rust `programs` (`src/v3d/backend.rs:3683`)
  copies `writes_z` and `disable_ez` and has no test of its own.
  `tests/es3_fragdepth_v3d_sim.rs` pins the written value in the QPU
  interpreter (a constant as its f32 bits, a computed depth per lane, no
  clamp by the compiler, the shader's own clamp, a one-arm write, a
  discarded lane). `tests/es3_fragdepth_cases.rs` builds every dEQP shape on
  both back ends (`every_fragdepth_shape_builds_with_writes_z_and_disable_ez`
  for V3D). The interpreters have no depth buffer, so the depth test by the
  written value (`fragdepth.compare.*`) is not modelled and is open.

## 4. PowerVR (`7bfac242`; plan #325 `fragdepth`)

On main, with a test:

- The chain clamps the depth (`clamp_depth_output`, `powervr/chain.rs`: an
  `fadd` with `sat`) and `assemble.rs` emits `depthf` from the result, after
  the kill test when the shader discards. `BuiltFragment::depth_out` becomes
  `ShcShader::writes_depth`.
- `src/pvr/backend.rs`: a program with `writes_depth` gets pass type
  `PVR_PPP_PASSTYPE_DEPTH_FEEDBACK`, ahead of the discard and opaque
  choices. A draw with `writes_depth` to a framebuffer without a colour
  attachment is refused by name (`d.target_gpu == 0`), there being no
  depth-feedback variant of the z-only frame.
- Refused at compile: a depth with an integer render target or with several
  sRGB attachments. Builds: depth with a discard, beside several colours,
  in a blend variant.
- Tests: `tests/es3_pvr_fragdepth.rs`; `src/pvr/backend_tests.rs` for the two
  draw rules.

## 5. GL core

It does not need to tell a back end the program writes depth. `src/gl/` has
no reference to `writes_depth`, `writes_z` or `ps_depth_out_reg`; each back
end reads the fact from the compiled program it already receives: PowerVR and
GC7000UL `ShcShader::writes_depth` (the GC also `viv.ps_depth_out_reg`), V3D
`ShcV3dFs::writes_z` / `disable_ez`. The GC back end's early-depth decision
is the one place that needs it (section 2), and it takes it from the
`GcDraw` shader it already has. Nothing in the GL state (`depth_test`,
`depth_write`) changes: those still gate the test and the write, the
program's depth only replaces the interpolated value that is tested.

## Pending, in one list

- A gate row for the fragdepth tests: `tools/check.sh` has one only for
  `tests/es3_pvr_fragdepth.rs`; `es3_fragdepth_gc`, `es3_fragdepth_v3d`,
  `es3_fragdepth_v3d_state`, `es3_fragdepth_v3d_sim` and
  `es3_fragdepth_cases` are not in it (items `gc-fragdepth-row`,
  `v3d-fragdepth-row`).
- GC7000UL on a tiled colour target: the back end renders to linear targets
  only, so the writer's early-Z-off words are pinned through the rule, not
  through a draw.
- V3D: the depth test by the written value in the host model.
- Board verdicts for all three: the i.MX8MP, Pi 4 and AM62P trees.
