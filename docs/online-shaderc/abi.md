<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# The register ABI a generated USC program must honour

BVNC 36.53.104.796 — PowerVR BXS-4-64, Toradex Verdin AM62P.

`isa.md` says how to encode an instruction. This says what the machine holds
when the program starts, and what the driver expects it to leave behind. A
program from `src/shaderc/online/` that gets the encodings right and this
wrong produces a clean kick and a wrong picture.

Every fact below carries the file and line it came from. Where a statement is
inference rather than something read out of the tree or measured on silicon, it
says so.

**The driver side is fixed.** The PDS programs are already written, already on
silicon, and are not changing for the online compiler: `driver/src/pvr_pds_render_gen.c`
generates them and `src/pvr/backend.rs` drives it. The online compiler's job is
to produce a program that fits the contract those two already implement, and to
report, through `ShcShader`, the numbers the driver needs to program the PDS.
That reporting duty is as binding as the code generation.

## The two sequencers

A USC program never starts with an empty register file. Before it runs, the
**PDS** — a separate programmable sequencer — executes a short program that
loads registers by issuing `DOUT*` operations. `DOUTI`, `DOUTD`, `DOUTU`,
`DOUTW` and `DDMAD` are **PDS** operations, not USC instructions
(`isa.md`, "EMIT and the DOUTx interface").

| PDS operation | loads | destination |
|---|---|---|
| `DDMAD` | vertex attributes | vertex-input registers (`vi0..`) |
| `DOUTW` | `gl_VertexID`, `gl_InstanceID` | vertex-input registers |
| `DOUTI` | varying interpolation coefficients | coefficient registers (`cf0..`) |
| `DOUTD` | texture descriptors, then uniforms | shared registers (`sh0..`) |
| `DOUTU` | nothing — it *kicks* the USC program | — |

`PDS_DSTDOUT_*` encodings: `DOUTD` 0, `DOUTU` 2, `DOUTI` 4
(`driver/src/pvr_pds_render_gen.c:61-63`, `:675`).

## DOUTU: how the program is entered

`pds_build_doutu_src0()` (`driver/src/pvr_pds_render_gen.c:210-235`) builds the
64-bit task-control word that launches the USC program. Three of its fields are
the compiler's responsibility, because only the compiler knows them:

- **Execution address**, bits 31:2, the program's byte offset **relative to the
  USC heap base**, 4-byte aligned (`:217-219`, mask
  `PDS_DOUTU_SRC0_EXE_OFF_NOTCLRMSK = 0x00000000FFFFFFFC`, `:103-104`). Not an
  absolute address.
- **Temps**, bits 40:35, in **blocks of 4 dwords**, rounded up
  (`:222-226`; `PDS_DOUTU_SRC0_TEMPS_ALIGNSIZE = 4`, `:99`). The count asserts
  into 6 bits (`:224`), so at most 63 blocks = 252 temp dwords. **A program that
  uses more temps than it declares corrupts another task's temps**, because the
  allocation is what separates concurrent tasks — this is the single most
  dangerous number the compiler reports.
- **Sample rate**, bits 34:33 (`:229-230`), and **dual-phase**
  (`PDS_DOUTU_SRC0_DUAL_PHASE_EN`, `:101`, `:233`).

`ShcShader` carries the temp count; the backend passes it through. Getting it
from the compiler's own register allocator, not from a guess, is M3's
precondition.

## Vertex stage

### Inputs: `vi0..`, loaded by DDMAD

One `DDMAD` per vertex stream (`driver/src/pvr_pds_render_gen.c:610-614`). Its
control word (`:600-606`) carries the DMA size in dwords
(`PDS_DDMAD_SRC3_BSIZE_SHIFT`), the destination register
(`st->dst_reg << PDS_DDMAD_SRC3_AO_SHIFT`) and `DEST_UNIFIED_STORE`. The
address is `stride * index + base`, where the index is input register 0 for a
per-vertex stream and input register 1 for a per-instance one (`:611-613`).

So: **attributes arrive in vertex-input registers at the dword offsets the
compiler asked for**, one dword per attribute dword, packed in ascending
location (`driver/docs/gpu-recipes/shader_toolchain.md`, "Attribute/output
rule"). The compiler reports them in `ShcShader::attribs` / `num_attribs`
(`include/gles_shaderc.h:238-240`).

`max_vtxin_dw` (`:598-599`, `:637`, `:641`) is the high-water mark the driver
turns into `vdm_usc_unified_size_units` (`:653`) — the unified-store allocation
for the task. A compiler that reads a `vi` register no DDMAD wrote gets
whatever the previous task left there.

`gl_VertexID` and `gl_InstanceID` are **not** automatic. They are `DOUTW`ed
from the IR0:IR1 pair into registers the compiler names (`:620-643`), and the
compiler names them through `vs_vertex_id_reg1` / `vs_instance_id_reg1`, which
are **the register plus one, with 0 meaning "the shader reads neither"**
(`include/gles_shaderc.h:241-248`). The off-by-one is deliberate — it lets a
table row written before the fields existed read as "none" rather than as
"register 0". A compiler that reports a bare register number here will make
every shader look like it wants `gl_VertexID` in `vi0`.

### Outputs: the vertex output store, written by UVSW

The program writes its outputs with the backend `uvsw_write` family and ends
with an emit (`isa.md`, "UVSW: how a vertex program emits"). `gl_Position`
occupies dwords 0..3 of the output vector; user varyings follow
(`shader_toolchain.md`, "Attribute/output rule").

The compiler reports `vs_output_size`, the number of dwords emitted, and
`vs_psize_off`, the dword holding `gl_PointSize` or `SHC_NO_PSIZE`
(`include/gles_shaderc.h:239-240`).

**Measured**: `driver/src/pvr_usc_vs_passthrough_blob.h` is exactly this and
nothing else — `uvsw_write_imm` to output 0 with `rpt=3`, then
`uvsw_write_emit_endtask_imm` to output 3 (`isa.md`, verification section).
It is the smallest correct vertex program on this core and the right model for
M3's passthrough.

## Fragment stage

### Varyings: `cf0..`, loaded by DOUTI

`pvr_pds_render_coeff_program()` (`driver/src/pvr_pds_render_gen.c:677-748`)
emits one iterator per varying, each a 64-bit data word plus one `DOUTI` code
word.

**Layout rule, and it is not negotiable** (`:658-672`, `:702-733`):

1. If the fragment shader needs perspective correction it reads `1/w`, so the
   **W plane iterator is allocated first**, at `cf0..3` (`:702-707`). Its
   descriptor is `GOURAUD << 24` with no perspective bit and offset 0.
2. Then the user varyings, in the order given, each taking
   **`components` coefficient sets of 4 dwords each** (`:733`, `set += cv->components`).

So a smooth-shaded fragment shader's first user varying starts at `cf4`, not
`cf0`. `ShcShader` reports `uses_w`; the compiler must set it to match what the
program actually reads, because the driver allocates from it.

The per-iterator descriptor (`:727-729`):

    shade << 24 | (components - 1) << 18 | persp << 16 | off << 8 | off

with `shade` = 3 (`GOURAUD`) for `smooth` and `noperspective`, 0 (`FLAT_V0`)
for `flat`; `persp` = 1 only for `smooth` (`:717-720`). `LAST` is bit 63 of the
final iterator's word (`:735`).

The `off` field is the trap. It is the varying's index **into what the
tessellation engine actually stored**, in half-dword units:

    pos_stored = 2 + (uses_z ? 1 : 0) + (uses_w ? 1 : 0)      # :690
    off = (vtxout_off - 4 + pos_stored - 2) * 2               # :726

The TE always stores x and y, and stores z and w **only when they are unused**
by the iteration (`:667-671`). So the same varying's coefficient offset
**changes with whether the fragment shader reads `1/w`** — a fragment shader
edited from `flat` to `smooth` moves every subsequent varying. The compiler
supplies `vtxout_off` (the varying's dword in the VS output vector, which must
be ≥ 4, `:714`) and the driver does this arithmetic. A compiler that
pre-applies any part of it will be wrong for one of the two cases.

Longer treatment: `driver/docs/gpu-recipes/multivarying_coeff_recipe.md`.

### Outputs: `po0..`

Fragment colour goes to the pixel-output registers, `po0..`
(`shader_toolchain.md`, "Register model"), reachable as special registers
32..35 and 164..167 (`isa.md`, "Special registers"). The default target is
`R8G8B8A8_UNORM` packed into one `po0` dword.

**Measured**: `driver/src/pvr_usc_clear_fs_blob.h` is one group, phase p0, a
`sngl`/`byp` — a move from `sh0` to `po0` — with `end=1`
(`src/pvr/blobs.rs:127-144`, and `isa.md`'s decode). The two-dword variant is
the same group twice, `end` on the second only. That is the shape of a solid-
colour fragment shader on this core, and it is M3's other half.

### End of tile

The EOT program is kicked by the PDS pixel-event program
(`driver/src/pvr_pds_pixel_event_gen.c:10-20`, a port of
`pvr_pds_generate_pixel_event`), whose `DOUTU` carries the EOT shader's
execution address **relative to the USC heap base**, 4-byte aligned
(`:153-158`) — the same convention as any other program — and its temp count
(`:158-160`, typically 0).

**Measured**: `driver/src/pvr_usc_eot_blob.h` is one group, one backend
`emitpix` with `freep=1`, `end=1` (`isa.md`). The online compiler does not need
to generate EOT for M3; the existing blob stays.

## Shared registers: `sh0..`, loaded by DOUTD

This is the layout most easily got wrong, because two different things share
one register file and the order between them is fixed.

**Texture descriptors come first, then the uniform block.**

Each sampler the stage declares needs one
`pvr_combined_image_sampler_descriptor` of **20 dwords**
(`src/pvr/backend.rs:441`, `TEX_DESC_DWORDS = 20`), packed contiguously in
binding order at `sh0`, `sh20`, ... (`src/pvr/backend.rs:2410-2414`). Within a
descriptor: image 0..3, image meta 4..7, sampler 8..11, sampler meta 12..15,
gather sampler 16..19 unused (`src/pvr/backend.rs:2418-2420`).

The uniform block is DMAed **after** them, at `shared_dw` past the descriptors
(`src/pvr/backend.rs:2974-2977`). The comment there records why this matters:
the layout is "the exact layout the compiler assigns, which puts the shader's
uniform block after them, so a short block would have the shader read image
state out of its own uniforms" (`src/pvr/backend.rs:2411-2414`).

Two consequences the online compiler must honour:

- **The compiler compacts the bindings it uses; the driver then allocates one
  descriptor per reported sampler, in order.** These are two halves of one rule,
  not a conflict — checked, because at first reading they look like one.
  The compiler emits a `shc_uniform` row per sampler the program uses, marked
  `dword_off = SHC_UNIFORM_NO_BLOCK` and carrying its descriptor binding
  (`include/gles_shaderc.h:103-109`; `src/shaderc/mod.rs:106`, `:150`). A
  program that uses only binding 3 therefore reports one row, and its descriptor
  lands at `sh0` — that is gl_test case 24, "sampler compaction: a used non-zero
  binding lands at sh0" (`driver/tools/pvrlab/gl_test.c:45`), and
  `src/pvr/backend_tests.rs:1565` asserts "one descriptor at sh0".
  The driver counts exactly those rows (`stage_sampler_count`,
  `src/pvr/backend.rs:2403-2405`) and gives each one a 20-dword slot in
  reported order. "Bound or not" (`src/pvr/backend.rs:2400-2402`) is about the
  *application*: a sampler the program declares but the app left without a
  texture still occupies its slot and is pointed at a 1×1 black border texture,
  so a stray sample cannot fault. **So the online compiler must report samplers
  by used binding, compacted, in ascending binding order**, and the descriptor
  index a program reads is its position in that reported list — not its GL
  binding number.
- **The uniform block is a contiguous prefix.** `uniform_dwords` is the block's
  size; `uniform_sh_dwords` is the prefix the compiled program actually reads,
  and the driver DMAs only that much (`include/gles_shaderc.h:253-262`), because
  shared registers limit how many USC tasks run at once. 0 means "the whole
  block". A compiler reporting a prefix shorter than it reads will read stale
  registers.

DOUTD's control word (`driver/src/pvr_pds_render_gen.c:473-477`) carries the
dword count, the destination offset (`PDS_DOUTD_SRC1_AO_SHIFT = 13`, `:89`),
`DEST_COMMON_STORE` (`:90`) and `LAST_EN` on the final kick (`:92`).

A `layout(push_constant)` block lands at `sh[start..]` with start 0, and
`load_push_constant` at byte offset *O* reads `sh[start + O/4]`
(`shader_toolchain.md`, "Uniform rule").

## The blend-lowering contract

**On this GPU blending and colour masks are lowered into the fragment shader**
(`tools/glsl2usc.sh:115`; `driver/docs/gpu-recipes/blending_recipe.md`). There
is no fixed-function blend unit to program. This is a hard requirement on the
online compiler, not an optimisation: a fragment shader compiled without its
blend state produces unblended output on a draw that asked for blending.

The contract, from `src/shaderc/mod.rs`:

- A fragment binary is compiled **for one blend state**, identified by
  `blend_key` (`:363`, `:476-477`), built by `shc_blend_key(src_rgb, dst_rgb,
  src_a, dst_a, eq_rgb, eq_a)` (`:68`), with `shc_blend_key_mask_off()` folding
  in disabled colour channels (`:79`) and `shc_blend_key_rt()` the render-target
  class (`:87`). 0 means "no blending".
- `SHC_BLEND_KEY_OPAQUE` is `(ONE, ZERO, ONE, ZERO, ADD, ADD)` (`:97`).
- The backend asks for a variant through `blend_variant(fs, blend_key, out)`
  (`:444`). The online compiler must implement that entry point: given a
  compiled FS and a blend key, produce the variant. The offline table instead
  holds every variant precompiled — that is exactly the storage the online
  compiler exists to delete (M5).
- `blend_consts_off` (`:365`) is the dword offset in the uniform block where the
  variant reads `glBlendColor`'s rgba for the `CONSTANT_*` factors, with
  `SHC_NO_BLEND_CONSTS` meaning the variant reads none, and those 4 dwords are
  **included in `uniform_dwords`** (`include/gles_shaderc.h:270-275`).
  An omitted `blend_consts_off` read as dword 0 is one of the
  absent-means-something-specific failures this project has already paid for;
  the sentinel is not optional.

`uses_fbfetch` (`include/gles_shaderc.h:267`) marks a variant that reads the
pixel output, which forces the TRANSLUCENT pass type.

## What M3 needs, and only that

For the first triangle through the online path — a passthrough VS and a solid
FS, gl_test cases 0 and 1 — the surface is small:

| duty | where |
|---|---|
| temps, honestly counted | DOUTU, above |
| attributes in `vi` at the declared offsets | DDMAD, above |
| `uvsw_write` × n then emit + endtask | vertex outputs, above |
| `vs_output_size`, `vs_psize_off = SHC_NO_PSIZE` | `ShcShader` |
| one move to `po0`, `end=1` | fragment outputs, above |
| `uses_w = 0`, no varyings, no samplers | no DOUTI, no descriptors |
| `uniform_dwords`, `uniform_sh_dwords` | shared registers, above |
| `blend_key = 0` | blend contract, above |

Neither program touches coefficient registers, sampler descriptors or blend
variants, which is why M3 is reachable before those parts of this document are
verified.

## Open, and to be closed before it is relied on

- Nothing in this document has been checked by generating a program from it and
  running it. That is M3, and until then every "must" here is a reading of the
  driver's source, not a measurement. The parts marked **Measured** are the
  exception: those are decodes of programs this driver has run on silicon.
- The `f16_offset` half of the DOUTI descriptor is written with the same value
  as `f32_offset` (`driver/src/pvr_pds_render_gen.c:727-728`). Whether that is
  correct for an f16 varying, or merely harmless because nothing emits one, is
  not established.
