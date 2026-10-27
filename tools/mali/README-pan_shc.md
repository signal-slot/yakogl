<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali-G610 (Valhall v10) shader references: the shim driver and pan_shc

The product path for this GPU is our own compiler (`src/shaderc/online/`, a
Valhall back end to come). Everything here is an instrument. There are two,
and each answers its own question:

| Question | Instrument |
|---|---|
| Which bytes does Mesa's GL driver make for this GLSL ES 1.00 program? | **The shim driver**: Mesa 26.2.1's own gallium panfrost driver on the host, on Mesa's drm-shim, no GPU (`drm_shim_ref.sh`, `gl_ref.py`). It reads the ES 1.00 text YakoGL receives and takes the GLSL path the board's driver takes. This is the byte reference. |
| What does the driver need to run those bytes: the FAU (push uniform) and sysval layout, where each uniform is, the varying layout, attribute records, preload and register counts, the IDVS split and `no_psiz` offsets, the early-ZS table, the FS facts? | **`pan_shc`**: Mesa's compiler called directly, with the driver's preparation transcribed, printing all of that as JSON. The trace prints some of the values (the FAU words, "Register allocation", "Preload", each program's VA); `pan_shc` names them (which uniform or sysval a FAU word is, the layouts, the facts the trace has no field for). |

`pan_shc`'s bytes equal the shim driver's except where its SPIR-V front end
differs from Mesa's GLSL path (the gaps below). So its metadata is the metadata
of the shim driver's program only for a region the case list shows as equal.

## pan_shc

`pan_shc.c` is a C program built inside a Mesa 26.2.1 tree, like
`tools/v3d/v3d_shc.c`. C is allowed here only because it is built in Mesa's tree
and calls Mesa's compiler; nothing of it ships. It reads one GL program as two
OpenGL SPIR-V modules and does what Mesa does for a `GL_ARB_gl_spirv` program
and a draw, each pass cited by Mesa file:line in the source:

1. `_mesa_spirv_to_nir` with panfrost's caps (gl_FragCoord, gl_PointCoord and
   gl_FrontFacing stay system values);
2. the linker: `remove_dead_varyings_pre_linking`, `preprocess_shader`,
   `prelink_lowering`, `nir_lower_io_passes` + `nir_opt_varyings_bulk`;
3. st/mesa: the uniform storage (panfrost packs uniforms: one dword per
   component, `PackedDriverUniformStorage`), `st_nir_lower_wpos_ytransform` (the
   upper-left, integer-centre origin), the 64-bit lowering, `st_finalize_nir`,
   `nir_lower_uniforms_to_ubo` (the uniforms become UBO 0) and the variant;
4. the driver: `panfrost_create_shader_state` (`pan_shader.c:500-633`: fragcolor,
   `pan_preprocess_nir`, IO lowering, the VS varying layout), the link (the FS
   takes the VS's layout), then `panfrost_shader_compile` (`:95-239`:
   resource indices, `pan_postprocess_nir`, sysvals into UBO 1, the FAU push
   settings) and `pan_shader_compile` (`bifrost_compile_shader_nir`), and
   `pan_earlyzs_analyze`.

Three gallium passes (`pan_nir_lower_sysvals.c`, `pan_nir_lower_res_indices.c`,
`pan_nir_remove_fragcolor_stores.c`) are compiled from the driver's own files,
copied unmodified; `pan_shc-shim/` stands in for the `pan_context.h` they
include (its sysval and resource-table part, transcribed).

### Build

```sh
tools/mali/build_pan_shc.sh <mesa-26.2.1.tar.xz> <work dir>   # once: extract, configure, build
tools/mali/build_pan_shc.sh --update <work dir>/mesa-26.2.1   # after a change here
```

The tarball is checked against `third_party/mesa-26.2.1/UPSTREAM`. The tree is
configured with `-Dgallium-drivers=panfrost -Dtools=panfrost` and no GL, EGL or
Vulkan: with no GL, Vulkan or CL user, Mesa builds NIR as a stub
(`src/compiler/nir/meson.build:81-95`), so the gallium driver is there only to
make NIR real. The script prints `PAN_SHC=<binary>`.

### Run

```sh
PAN_SHC=<binary> tools/mali/glsl2pan.py vs.vert fs.frag [--gpu-id HEX] [--gpu-variant N] [--prim triangles|lines|points] > out.json 2> disasm.txt
<binary> vs.spv fs.spv [the same options]                  # on SPIR-V you made yourself
```

`glsl2pan.py` rewrites each ES 1.00 stage as GLSL ES 3.10 with explicit
locations and runs `glslangValidator -G` (GLSLANG). It is
`tools/vivante/glsl2vivante.py`'s rewrite with three changes, each to keep what
Mesa sees for the ES 1.00 source: the target is `310 es` (a `460` source keeps
no RelaxedPrecision), gl_FragColor and gl_FragData[0] are `mediump` outputs
(GLSL ES 1.00 section 7.2), and gl_Position is not moved first. Attribute,
varying and uniform locations are the declaration order; varyings are linked by
name.

stdout is the line `pan_shc: build <git describe of this tree> mesa 26.2.1`,
then one JSON object; stderr has the disassembly of each region
(`disassemble_valhall`, with each instruction's bytes). The JSON:

- `gpu`: `gpu_id` (default 0xa8670005, the board's GPU_ID), arch, model,
  `gpu_variant` (`assumed` 0 unless given: the compiler reads it only to pick
  the model for its statistics, `bifrost_compile.c:4038-4045`), `prim`.
- `attributes`: per attribute, its GL location and per slot the attribute
  record index and its Valhall handle (`table 1 << 24 | index`).
- `vs` and `fs`: `work_reg_count`, `preload` (bit n = register n is live at
  entry), `code` (the binary as 64-bit words) and `regions` with their offsets:
  - VS: `position` at 0, `position_no_psiz` at `no_psiz_offset` when the VS
    writes gl_PointSize (packed right after the position program), `varying` at
    `secondary_offset` (packed last), with `secondary_work_reg_count` and
    `secondary_preload`; `idvs`, `writes_point_size`, `needs_extended_fifo`;
    `draw_position_region`, the region a draw of `--prim` runs as its position
    program (the driver packs three program descriptors and a draw that is not
    of points takes the second, which points at `no_psiz_offset`:
    `pan_cmdstream.c:4502-4555`, `pan_cmdstream.h:238-249`);
  - `varying_layout` (VS: the compact layout the FS also uses, per slot its
    location, ALU type, components, section and byte offset) and
    `varying_formats` (FS);
  - `uniforms`: per uniform, its dword in UBO 0 and its size, or a sampler's
    binding, and the state uniform `gl_FbWposYTransform` when the FS reads
    gl_FragCoord;
  - `sysvals`: what UBO 1 holds, one vec4 each (the viewport scale and offset in
    a VS, the blend constants always in an FS);
  - `fau`: every pushed 32-bit word (FAU slot = word / 2): a UBO word with the
    sysval or uniform it is, or a constant the compiler promoted;
  - FS: `reads_*`, `can_discard`, `writes_*`, `can_early_z`, `can_fpk`, the
    `hsr` facts, `blend[rt]` (the type of each colour output and the
    blend-shader return offset), `wait_6`/`wait_7`, `uses_ld_var`,
    `noperspective`, and `earlyzs`, the whole `pan_earlyzs_lut` (update and kill
    per state).

`PAN_SHC_PRINT_NIR=1` prints the NIR each stage hands the driver;
`BIFROST_MESA_DEBUG=shaders` the compiler's own dump.

`--prim`: for GLES 2 the driver's FS key is the same for all three primitives.
Point sprite coordinate replacement is `ctx->Point.CoordReplace`, 0 in GLES 2
(`points.c:235`), and panfrost sets `tgsi_texcoord`, so gl_PointCoord adds
nothing (`st_atom_rasterizer.c:217-223`); `line_smooth` is not GLES state. What
`--prim` changes is `draw_position_region`.

## The shim driver

```sh
tools/mali/drm_shim_ref.sh build <mesa-X.Y.Z.tar.xz> <work dir>
tools/mali/drm_shim_ref.sh run <work dir> vs.vert fs.frag <out prefix> [--prim P] [--size WxH] [--viewport X,Y,W,H] [--depth-range N,F]
tools/mali/pan_ref_diff.py list <out prefix>.trace
tools/mali/pan_ref_diff.py diff out.json disasm.txt <trace> --draw    # pan_shc vs the trace's draw
PAN_SHC=<binary> tools/mali/ref_cases.py <work dir> <out dir>         # the control cases
```

`build` makes gallium panfrost with EGL (surfaceless), GLES 2 and the drm-shim.
`run` draws once with the program through `gl_ref.py` (an FBO, 64x64 unless
`--size` says otherwise, the viewport and depth range of `--viewport` and
`--depth-range` or the whole FBO and 0,1, attributes bound in declaration
order, six vertices, no read-back) with
`BIFROST_MESA_DEBUG=shaders,shaderdb` and a pandecode trace. With
`--uniform-values` it sets every float uniform the two stages declare,
component c of the k-th declared uniform to 16 (k + 1) + c (an integer fp16
holds), so the trace's FAU table names each pushed uniform word by its
value; the Valhall back end's uniform references (`testdata/sel_u_*`) are
drawn so. The shim reports
GPU_ID 0xa8670000 (revision 0; the board's is 5); pan_shc's output for
0xa8670000 and 0xa8670005 is identical except for the `gpu` line (measured on
tri_varying). For Mesa 25.0 the script needs two things 26.2.1 does not:
`MESA_LOADER_DRIVER_OVERRIDE=panthor` (else 25.0's loader takes the host's own
render node's PCI id and loads no driver), and `mesa-glibc-once_flag.patch`
(26.2.1's guard for the glibc that defines once_flag, applied only to a tree
that lacks it; no compiler or driver code).

`pan_ref_diff.py diff --draw` pairs pan_shc's regions with the programs that
the descriptors of the trace's first draw name, and compares the instruction
words, `work_reg_count`, the preload mask, the FAU constants and
`secondary_offset` (the module docstring has the rules). The disassembly text
of two Mesa versions differs (`^r0` / `r0^`, `u1` / `u0.w1`), so only the words
count. `pan_ref_diff.py diff-ref A B` compares two traces or `expect.json` files
with each other, for example a board trace with the 25.0.7 shim's.

## Results (2026-10-18)

The board reference is the board's two programs (`testdata/tri*.vert/.frag`,
verbatim from `tri.c` and `tri_varying.c`) under Debian's Mesa 25.0.7 on the
NanoPC-T6, with traces taken with `PAN_MESA_DEBUG=trace,sync` and
`BIFROST_MESA_DEBUG=shaders`.

| Comparison | Regions | Result |
|---|---|---|
| upstream Mesa 25.0.7 on drm-shim vs the board | tri: position, fragment; tri_varying: position, varying, fragment | 5 of 5 byte-equal (`diff-ref`) |
| pan_shc vs Mesa 26.2.1 on drm-shim | the same 5 | 5 of 5 byte-equal (`diff --expect`, the gate row) |
| pan_shc vs the board | the same 5 | 5 of 5 differ (`diff --draw`) |

Each row is a command on committed data, and `pan_ref_diff.py` compares the
words, `work_reg_count`, the preload mask, the FAU words and
`secondary_offset` (the lines below leave out the per-instruction detail):

```sh
cd tools/mali/testdata
T=../pan_ref_diff.py
# board (Mesa 25.0.7) vs the 25.0.7 shim: board-25.0.7/*.pandecode are the board's traces,
# shim-25.0.7/*.trace the shim driver's (drm_shim_ref.sh run on a Mesa 25.0.7 build)
$T diff-ref board-25.0.7/tri.pandecode shim-25.0.7/tri.trace
```
```text
fs.fragment vs 7fffffffc080: byte-equal, 7 instructions
fs.fragment facts: equal, work_reg_count 32, preload 0x3000000000000000 (fau table printed by neither side, not compared)
vs.position vs 7fffffffc000: byte-equal, 7 instructions
vs.position facts: equal, work_reg_count 32, preload 0x3800000000000000 (6 fau words compared)
pan_ref_diff: 2 region(s) compared, 0 differ or missing
```
and `board-25.0.7/tri_varying.pandecode shim-25.0.7/tri_varying.trace`:
```text
fs.fragment vs 7fffffffc100: byte-equal, 6 instructions
fs.fragment facts: equal, work_reg_count 32, preload 0x3000000000000000 (fau table printed by neither side, not compared)
vs.position vs 7fffffffc000: byte-equal, 7 instructions
vs.position facts: equal, work_reg_count 32, preload 0x3800000000000000 (6 fau words compared)
vs.varying vs 7fffffffc080: byte-equal, 3 instructions
vs.varying facts: equal, work_reg_count 32, preload 0x3800000000000000 (6 fau words compared)
secondary_offset: equal, 128
pan_ref_diff: 3 region(s) compared, 0 differ or missing
```
```sh
# pan_shc (a recorded run, pan_shc-out/) vs the board
$T diff pan_shc-out/tri.json pan_shc-out/tri.disasm board-25.0.7/tri.pandecode --draw
```
```text
fs.fragment vs 7fffffffc080: 3 of 7/7 instructions differ (words sha256 8eb3c005a976)
fs.fragment facts: differ: fau absent in reference
vs.position vs 7fffffffc000: 16 of 19/7 instructions differ (words sha256 41ef03dff988)
vs.position facts: differ: fau word 5 pan_shc 00000004, reference 00000000
pan_ref_diff: 2 region(s) compared, 2 differ or missing
```
and `pan_shc-out/tri_varying.json ... board-25.0.7/tri_varying.pandecode --draw`:
```text
fs.fragment vs 7fffffffc100: 8 of 8/6 instructions differ (words sha256 a2e324f5d090)
fs.fragment facts: differ: fau absent in reference
vs.position vs 7fffffffc000: 16 of 19/7 instructions differ (words sha256 41ef03dff988)
vs.position facts: differ: fau word 5 pan_shc 00000004, reference 00000000
vs.varying vs 7fffffffc080: 11 of 11/3 instructions differ (words sha256 ec221f7f002c)
vs.varying facts: differ: fau word 5 pan_shc 00000004, reference 00000000
secondary_offset differs: pan_shc 256, reference 128
pan_ref_diff: 3 region(s) compared, 4 differ or missing
```

(The board's 25.0.7 trace prints no "Fragment FAU" table; `diff` reports that as
absent, not as equal. `board-25.0.7/*.expect.json` are `extract --draw` of the
board traces; `test_pan_ref_diff.py` checks that.)

So every difference between pan_shc and the board is a change from Mesa 25.0.7
to 26.2.1, and not pan_shc's. The causes, from the two versions' code:

- **VS stores** (both position programs, the varying program). 25.0.7 stores
  each output with one `STORE.i128` / `STORE.i96` from the back end
  (25.0.7: `src/panfrost/compiler/bifrost_compile.c:1194-1196`, `bi_emit_store_vary`). 26.2.1 lowers VS outputs
  in NIR to `store_global` at `lea_buf + offset` (26.2.1:
  `src/panfrost/compiler/pan_nir_lower_vs_outputs.c:95`), whose alignment is
  the builder's default, the first source's bit size over 8: one 32-bit
  component, 4 bytes (26.2.1: `src/compiler/nir/nir_builder_opcodes_h.py:143-145`,
  the generator of `nir_build_store_global`).
  `nir_lower_mem_access_bit_sizes` (26.2.1:
  `src/panfrost/compiler/bifrost/bifrost_nir.c:1009-1017`, with
  `mem_access_size_align_cb` at `:776`) then splits every store into `STORE.i32`, and
  each address after the first is a `SHADDX.s64` by 4, 8 or 12. Those three
  constants are promoted FAU words. tri position: 7 instructions in 25.0.7, 19
  in 26.2.1; tri_varying varying: 3, 11.
- **Constants in the FAU.** 26.2.1 promotes repeated immediates into FAU words
  (26.2.1: `src/panfrost/compiler/bifrost/bifrost_compile.c:4495-4515`, enabled by
  `inputs.fau.promote_immediates`, 26.2.1:
  `src/gallium/drivers/panfrost/pan_shader.c:207`); 25.0.7 has no such pass and keeps them as `IADD_IMM`
  (tri FS: `IADD_IMM 0x3C00` in 25.0.7, `MOV r0, u2.w0` with word 4 =
  0x00003c00 in 26.2.1).
- **ATEST's alpha** (tri FS). 25.0.7's back end takes alpha from the packed
  fp16 colour register (25.0.7: `src/panfrost/compiler/bifrost_compile.c:948-969`: `bi_half(..., true)` at `:963`), so
  it first builds `0x3C000000` in a register. 26.2.1 builds ATEST in NIR
  (26.2.1: `src/panfrost/compiler/pan_nir_lower_fs_outputs.c:138-150`) on the alpha value, which folds to the
  fp32 constant `0x3F800000`, an inline constant of ATEST.
- **The mediump varying load** (tri_varying FS). 25.0.7 loads the mediump vec3
  as fp16 in the load itself (`LD_VAR_BUF_IMM.f16 ... src_f32`;
  25.0.7: `src/panfrost/compiler/bifrost_compile.c:554`, `bi_emit_load_vary`,
  register format F16 for a 16-bit destination at `:577`).
  26.2.1 runs `pan_nir_resize_varying_io`
  (`src/panfrost/compiler/bifrost/bifrost_nir.c:902`) before
  `nir_lower_mediump_io` (`:914`). The VS layout's slot is 32-bit, so the pass
  clears the load's `medium_precision` (26.2.1: `src/panfrost/compiler/pan_nir_resize_varying_io.c:62-82`).
  The load stays `LD_VAR_BUF_IMM.f32` and the conversion is a separate
  `V2F32_TO_V2F16` / `FADD.f32 ... .h0`.

The per-instruction detail is the full output of the commands above.

### Control cases

`testdata/cases.txt`: 73 cases, 208 regions (the 9 below, then the S4-S6 programs of #1384: `s4_*`, `fvg_*`, `fvgtest_*`, then the constant-coordinate and run-time-index programs of #1385 (b): `tex_coord_const`, `vtl_fs`, `uniidx_d`, `uniidx_i`), each pan_shc region against the shim
driver (26.2.1). `ref_cases.py` passes when every region is as recorded. `equal`
means the words and the facts agree. A `differs:<cause>:<n>:<sha256>` pins
the number of differing words and the digest of those words on both sides, so
a gap that closes, and a different difference, both fail until the line is
updated; the facts of a `differs` region are not pinned. Each `differs` closes
with the GLSL front end (the follow-up below).

| Case | What it controls | Result |
|---|---|---|
| tri, tri_varying | the board's programs | all equal |
| uniforms | a vec4 and a float uniform: packed UBO 0 layout, FAU push | equal |
| mat4 | `u_mvp * a_pos` | VS differs: `mat-order` |
| psiz_points, psiz_triangles | gl_PointSize: the position program and its `no_psiz` copy | equal |
| tex_highp | highp FS: sampler2D, a uniform, gl_FragCoord | FS differs: `mediump` |
| tex_mediump, tex_points | the same FS at mediump, triangles and points | FS differs: `mediump` |

## Known gaps of pan_shc (its SPIR-V front end)

- **`mediump`**. Mesa's GLSL path lowers mediump arithmetic to fp16 (the GLSL
  compiler's precision lowering, on because panfrost sets the fp16 caps,
  `pan_screen.c:585-589`). The FS then has `FMA.v2f16` and 16-bit texture
  results. glslang's RelaxedPrecision reaches only the IO variables, and
  spirv_to_nir in the GL environment lowers no ALU. In GLSL ES 1.00,
  gl_FragCoord is mediump too (section 7.2), so a highp FS that reads it
  differs as well (tex_highp). For an FS with any mediump value, the bytes and
  the FAU, register and preload facts come from the shim driver, not from
  pan_shc.
- **`mat-order`**. A matrix times a vector is summed from the first column in
  the GLSL path (`lower_mat_op_to_vec.cpp:165-178`) and from the last in
  spirv_to_nir (`vtn_alu.c:79-90`). The FMA chain has another order, so the
  rounding can differ. The uniform and FAU layout is the same (measured on
  mat4).
- The uniform order follows the declaration order (the GLSL path). spirv_to_nir's
  variable order is different; pan_shc takes the uniforms in location order.
  The uniforms case measured this.

The follow-up that closes both gaps is a GLSL front end for pan_shc (Mesa's
GLSL compiler and `gl_nir_link_glsl` with panfrost's caps), checked with the
same case list.

Not covered: blend shaders (the blend-shader need per GLES 2 blend mode), more
than one colour buffer, line smoothing and user clip planes (not GLES 2),
transform feedback, and anything the case list does not exercise. A score here
says only that nothing the cases cover differs unexpectedly.

## Gate

`tools/check.sh` (scope `mali`, `tools/mali/*` and `tools/vivante/es100.py`) has
two rows:

- **Without a Mesa tree**: `test_pan_ref_diff.py` (11 tests, asserted by
  count). It runs `pan_ref_diff.py` on the committed recorded pan_shc outputs
  (`testdata/pan_shc-out/`) against `testdata/*.expect.json` (exit 0), against
  mutated copies of them (a word, a FAU constant, `work_reg_count`, the preload,
  `secondary_offset`, a missing region: exit 1), on the board's traces
  (`board-25.0.7/`, the first draw only) and the 25.0.7 shim's
  (`shim-25.0.7/`), and parses `ref_cases.py --lint` over the case list.
- **With `MESA_PAN_TREE`** set to a tree from `build_pan_shc.sh`: it rebuilds
  pan_shc from the checkout, requires no compiler warning in pan_shc's sources
  and the build line `pan_shc: build <git describe --always --dirty of this
  checkout> mesa 26.2.1`, runs pan_shc on tri and tri_varying and requires every
  region to be equal to `testdata/*.expect.json` (the shim driver's programs,
  written by `pan_ref_diff.py extract --draw` from `drm_shim_ref.sh run`
  traces): 5 regions compared, 5 planned, 0 differ. Without `MESA_PAN_TREE`
  this row is a gap.

The shim-driver case list (`ref_cases.py`) is not a gate row; it needs the
second Mesa build. A change to `tools/vivante/es100.py`, which glsl2pan.py and
gl_ref.py import, reaches this scope as well as the vivante one.
