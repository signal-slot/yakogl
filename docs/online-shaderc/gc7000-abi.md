<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# The register ABI a generated GC7000 shader must honour

GC7000UL — model `0x7000`, revision `0x6204`, HALTI5 — on the Toradex Verdin
i.MX8M Plus.

`gc7000-isa.md` says how to encode an instruction. This says what the machine
holds when the shader starts and what the driver expects back. A shader that
gets the encodings right and this wrong produces a clean draw and a wrong
picture.

Every fact carries the file and line it came from. Statements that are a
reading of our source rather than a measurement say so; the ones marked
**measured** were measured on the board or against a captured command stream.
This is the sibling of `abi.md`, which covers the PowerVR and is now reference
material only.

**The driver side is fixed.** `src/vivante/` and `driver/src/vivante/` already
implement this contract, and `src/vivante/shader.rs` is the whole of the shader
half. The online compiler's job is to fit it and to report, through `ShcShader`
and its `ShcViv`, the numbers the driver programs the hardware with.

## How this differs from the PowerVR, in one paragraph

There is no firmware, no tiling, no PDS sequencer and no parameter buffer. The
host writes `LOAD_STATE` commands into a buffer, the front end fetches them,
and a draw runs the pipeline against whatever register state is loaded
(`src/vivante/VIVANTE_NOTES.md`). So nothing "loads registers before the shader
runs" the way DOUTI and DOUTD do on Rogue: **the uniforms are command-stream
words written straight into the constant register file**
(`src/vivante/shader.rs:11-13`). And **the GC7000 blends in the pixel engine**,
so a fragment shader is compiled once and has no blend variants — the single
biggest simplification against the PowerVR contract.

## Stages are linked by register number, not by name

This is the central fact of the whole ABI (`src/vivante/shader.rs:5-9`): the
fragment shader reads its varyings from **registers 1..N**, and a `VS_OUTPUT`
map says which vertex-shader output register feeds each one. The GLSL names
exist only so that `link()` can pair them up at link time; the hardware never
sees a name.

`ShcViv` (`src/shaderc/mod.rs:246-276`) is what the compiler must fill:

| field | meaning |
|---|---|
| `in_[]`, `num_in` | inputs: VS attributes, or FS varyings. For a fragment shader the compiler places varyings in registers 1..N, so `num_in` **is** the varying count (`:247-250`) |
| `out[]`, `num_out` | outputs: VS varyings and position, FS colour |
| `vs_reg_of_in[]` | filled by `link()`: the VS output register feeding `in_[i]`, or `SHC_VIV_NO_REG` when the VS writes no such output (`:254-257`) |
| `vs_pos_out_reg` | the register holding `gl_Position` |
| `vs_pointsize_out_reg` | `gl_PointSize`, or `SHC_VIV_NO_REG` |
| `vs_id_in_reg` | where `gl_VertexID`/`gl_InstanceID` arrive |
| `ps_color_out_reg`, `ps_depth_out_reg` | fragment outputs |
| `vs_load_balancing` | the `VS_LOAD_BALANCING` word the compiler chose |
| `input_count_unk8` | the `{VS,PS}_INPUT_COUNT` UNK8 field |
| `const_contents`, `const_data`, `const_count` | the constant register file, one entry per dword, in register order |

Each `ShcVivIo` (`src/shaderc/mod.rs:218-229`) carries the register number, the
`gl_varying_slot`/`gl_vert_attrib`, 1..4 components, the interpolation mode
(0 smooth, 1 noperspective, 2 flat) and the GLSL name in 32 bytes.
`SHC_VIV_MAX_IO` is 16 (`:213`), `SHC_VIV_NO_REG` is `0xff` (`:212`).

## Vertex stage

### Inputs

One hardware input per **vertex element**, not per shader input. An element the
shader does not read is pointed at a temporary register above the ones it uses
(`src/vivante/shader.rs:51-61`) — so the compiler's temp count is not private
information: the driver allocates above it, and a wrong count aliases a live
temporary with an unread attribute.

`gl_VertexID` and `gl_InstanceID` share one register: the driver appends
`vs_id_in_reg` to the input map and programs `FE_HALTI5_ID_CONFIG` with the
**component** addresses `reg*4` and `reg*4 + 1` (`:64-74`) — vertex id in `x`,
instance id in `y` of the same register.

### Outputs, and the order is fixed

The `VS_OUTPUT` map is built in exactly this order
(`src/vivante/shader.rs:76-101`):

1. **position**, from `vs_pos_out_reg`;
2. **one entry per fragment-shader varying, in the order the fragment shader
   reads them** — driven by `fs.vs_reg_of_in[i]`, not by the vertex shader's own
   output order;
3. **point size**, if the shader writes one.

So the vertex shader's output register numbering is whatever the compiler
chose, and the *stream* order is dictated by the fragment shader. A vertex
shader is only meaningful against the fragment shader it was linked with.

`VS_OUTPUT_COUNT` counts position plus varyings plus point size. `SH_SPECIALS`
instead wants the point size's **component** offset, which is
`(1 + varyings) * 4`, i.e. computed *before* the point size is counted in
(`:96-103`). That one distinction — a count including the point size everywhere
but an offset excluding it in `SH_SPECIALS` — is the sort of thing that yields
correct geometry with wrong point sizes.

## Fragment stage

`PS_INPUT_COUNT` is **varyings + 1** (`:121-125`): the position counts as an
input. The temp register count is `max(fs.temps, varyings + 1)` (`:127`), so a
fragment shader that declares fewer temps than it has varyings still gets room.

`PS_OUTPUT_REG` takes `ps_color_out_reg`, defaulting to 0 when the shader
declares none (`:122`). One render target; its output is saturated for the
unorm formats (`:129-134`).

Per-varying component counts go to both `PA_VARYING_NUM_COMPONENTS` and
`PS_VARYING_NUM_COMPONENTS` (`:118`, `:135`), and the interpolation modes to
`GL_HALTI5_SHADER_ATTRIBUTES`, one 4-bit lane per *component* rather than per
varying (`:85-90`, `:145`). `GL_VARYING_TOTAL_COMPONENTS` is the total rounded
**up to even** (`:137`).

## Uniforms: the constant register file

Both stages share **one** constant register file. The vertex stage starts at
register 0 and the fragment stage immediately after it
(`src/vivante/shader.rs:262-277`).

### A shader's `SRCn_REG` is stage-relative; the hardware adds the base

**A fragment shader numbers its constant-file reads from 0, within its own
block. The compiler does not add `PS_UNIFORM_BASE`; the hardware does.** This
is the sentence the rest of this section was ambiguous about, and getting it
backwards is invisible until a program has constants in *both* stages, at which
point the only symptom is a wrong colour.

Three independent things say so.

**Mesa's compiler never adds a base.** A uniform load becomes a source with
`.rgroup = ISA_REG_GROUP_UNIFORM_0, .reg = nir_intrinsic_base(intr)`
(`etnaviv_compiler_nir.c:634-641`) — the shader's own uniform index, unmodified.
`SRC_CONST(idx, s)` does the same for a folded constant (`:226`). Grep the whole
compiler for `PS_UNIFORM_BASE` and the only hit is `max_ps_uniforms`
(`:1117`), which is a per-stage *limit* on how many uniforms a fragment shader
may have, not a base added to anything.

**Our driver programs the base as hardware state**, exactly as Mesa does:
`VS_UNIFORM_BASE = 0` and `PS_UNIFORM_BASE = vs_vec4`
(`src/vivante/shader.rs:277-278`, `vs_vec4 = const_count.div_ceil(4)` at
`:275`); Mesa writes `VIVS_PS_UNIFORM_BASE = vs->uniforms.count / 4`
(`etnaviv_emit.c:792`). A register the host programs per draw, that no compiler
adds, can only be for the hardware to apply.

**The offline table agrees, over the whole corpus.** Of the 127 fragment
shaders that read uniforms, **all 127** index `0 ..= const_count/4 - 1` within
their own block and **none** reads beyond it. Under absolute numbering a
fragment shader paired with a vertex stage that has constants would have to
start at that stage's count.

The structural argument is the one that does not depend on which pairs happen
to be in the table: blobs are looked up **by source hash, per stage**, so any
fragment shader can pair with any vertex shader. If `SRCn_REG` were absolute, a
fragment shader compiled against a vertex stage with 4 constants would be wrong
against one with 8, and the blob could not be shared. It is shared.

So an online compiler emits `SRCn_REG = slot_dword / 4`, component
`slot_dword % 4`, and does nothing else.

One thing that reads like a contradiction and is not: the two *windows* are
distinct (VS `0x34000` mirror, PS `0x36000`) and the driver **also** adds the
vertex stage's size to the PS window's byte address when writing
(`src/vivante/shader.rs:290`), matching Mesa (`etnaviv_uniforms.c:141-142`). The
base is therefore applied twice in different senses — once by the host choosing
where to write, once by the hardware resolving a shader-side register number.
Both are correct and neither is the compiler's business.

The compiler does not hand over a uniform block; it hands over a **per-dword
description of the constant file**, `const_contents[]` + `const_data[]`, in
register order (`src/shaderc/mod.rs:271-275`). The driver resolves each dword at
draw time (`src/vivante/shader.rs:201-244`). `ShcVivConst`
(`src/shaderc/mod.rs:191-209`):

| value | contents | resolved to |
|---|---|---|
| 0 | `UNUSED` | zero |
| 1 | `LITERAL` | `const_data[i]` verbatim — a constant the compiler folded in |
| 2 | `UNIFORM` | `block[const_data[i]]`, a dword of the stage's uniform block |
| 3, 4 | `TEXRECT_SCALE_X/Y` | from the bound texture |
| 5, 6, 7 | `TEXTURE_WIDTH/HEIGHT/DEPTH` | from the bound texture |
| 8, 9, 10 | `SAMPLER_LOD_MIN/MAX/BIAS` | `LOD_MAX` is `mip_levels - 1` when mipmapping, else 0 |
| 11 | `UBO_ADDR` | — |

This is a genuinely different contract from the PowerVR's "DMA a prefix of the
uniform block into `sh0..`". **The compiler owns the layout of the constant
file** and tells the driver what every dword means, including that some dwords
are not uniforms at all but properties of a bound texture. An online compiler
must emit this table; it cannot simply report a block size.

**The fragment stage's base is a register number, and a register is a vec4.**
The vertex stage's dword count is therefore rounded **up** to a whole register
(`div_ceil(4)`, `:275`, with `VS_UNIFORM_BASE` 0 and `PS_UNIFORM_BASE` that
count at `:277-278`). The comment above it records why it must be: Mesa divides by
four without rounding because its count is already a vec4 multiple, ours is a
raw dword count out of the table, and a vertex shader with six dwords of
constants would truncate to one register and leave the fragment stage's window
two dwords under its values.

### Ordering: auto-steering, and it is not negotiable

**Measured**, against a captured stream rather than read from anyone's source
(`src/vivante/shader.rs:174-192`). At HALTI5 the driver does **not** say which
stage a constant-file write is for. The hardware steers each write to whichever
stage the shader state written just before it belongs to — Mesa calls this
auto-steering and marks the sequence "do not change ordering unless necessary".

So the command stream must be, in this order: both stages' icache blocks, both
`UNIFORM_BASE` registers, the constants, and **only then** the two prefetches.
A prefetch placed between the fragment stage's shader block and its constants
steers the file at the vertex stage — `VS_ICACHE_PREFETCH` is a vertex-stage
register — and the fragment shader then reads zeros. The evidence is the stream
Mesa produces for this part, captured on the host against its drm-shim
(`tools/vivante/cs_golden.c`, decoded by `cs_decode.py`).

This constrains the *driver*, not the compiler, but an online compiler that
ever emits its own state has to know it.

## Shader code

Both shaders' code sits in GPU memory and the shader processor fetches it
through its instruction cache (`src/vivante/shader.rs:11-12`). Per stage
(`:156-172`): `NEWRANGE_LOW` 0, `HALTI5_RANGE_HIGH` the instruction count,
`INST_ADDR` the GPU address, `SH_CONFIG` with RTNE rounding, icache enabled,
and `ICACHE_COUNT` = **count − 1**, the last instruction's index rather than the
count.

The instruction count is `usc_dw / 4` (`:157`) — the same `usc` pointer and
`usc_dw` dword count the PowerVR side uses, four dwords per 128-bit
instruction. A shader whose `usc_dw` is not a multiple of four is malformed and
`gc7000-isa.md`'s decoder reports it as such.

## What an online compiler must produce, and only that

| duty | where |
|---|---|
| 128-bit instructions, `usc_dw` a multiple of 4 | shader code, above |
| `temps`, honestly counted | the driver allocates unread attributes above it |
| `in_`/`out`/`num_in`/`num_out` with registers, components, interp, names | `ShcViv` |
| `vs_pos_out_reg`, and `vs_pointsize_out_reg` or `SHC_VIV_NO_REG` | vertex outputs |
| `ps_color_out_reg` | fragment outputs |
| `const_contents[]`/`const_data[]`/`const_count` | the constant file |
| `vs_load_balancing`, `input_count_unk8` | passed through verbatim |
| **no blend variants** | the pixel engine blends |

`vs_reg_of_in[]` is filled by `link()`, not by the compiler.

## Open, and to be closed before it is relied on

- **A fragment uniform does not reach its shader — DOES NOT REPRODUCE on the
  offline path at current main (2026-09-22).** The claim below was measured on
  2026-09-21 and steered a day of work; it is kept, with its status, because a
  measurement nobody re-checks becomes a claim, and this one had.

  **What the control says** (cts-runner, captures in
  `build/imx8mp/case1-control/`, whole and unfiltered):

      offline pair @ HEAD 9329c68, 32/32 pool   case 1 uniform: PASS (exit 0)
      offline pair @ HEAD 9329c68, 2/4 pool     case 1 uniform: PASS
      board binary @ Sep 21 23:32 vintage       case 1 uniform: PASS
      remote pair  @ a51616c                    VOID - link fails before any draw

  The first row is the one that matters: **same generation as the failing
  remote pair, same board, minutes apart, only the shader path differs**. So
  the earlier "it passes on the old binary" was not a vintage artefact, and
  passing at both context-pool sizes rules the pool out too. All ten checks
  pass, including the surface-kept and redraw-with-its-program pair that the
  defect would have broken, and `active uniforms 2; [0] = u_mvp` is reported.

  **What is still open.** The remote (run-time shader) path cannot be measured
  yet: it dies at link on `push_constant` before any draw, a regression in the
  new GC7000 link check. Until that run exists, "the defect is gone" and "the
  defect is specific to the run-time shader path" are not distinguished. Either
  way it is **not** `emit_uniforms` — see the six eliminations below, which
  stand regardless.

  **The earlier reading, for the record.** Measured 2026-09-21
  (`src/vivante/shader.rs:280-290`): gl_test case 1 failed while a vertex
  uniform in the same draw arrived correctly, and it was not the window address
  — writing the fragment values through the vertex window at the
  `PS_UNIFORM_BASE` offset was tried on silicon and case 1 still failed.

  **For M3**: a literal folded into the constant file
  (`SHC_VIV_CONST_LITERAL`) remains the safe choice for the first triangle
  because it does not touch the uniform path at all, but it is no longer
  known to be *necessary*. gles2-core should not plan around a uniform working
  on the online path until the remote capture exists.

  **An instrument mistake of mine belongs here too.** The first run of the
  instrument was reported by cts-runner as void, correctly: my prediction said
  "line absent → the block arrived", and the absent branch is only sound *if a
  draw occurred*. A link failure produces the same silence as a working uniform
  path. The instrument was fine; the prediction had no representation for
  "nothing was measured". Any two-way prediction handed to a runner must state
  the precondition that makes its negative branch mean anything.

  **The upload path has now been read against Mesa's line by line, and it is
  not the cause.** Six candidates eliminated, all host-side, no board time:

  | candidate | why it is not it |
  |---|---|
  | Missing `SH_CONTROL` steering before the PS constants | Mesa gates that on `need_steering = screen->info->halti < 5` (`etnaviv_emit.c:695`), so on HALTI5 Mesa does **not** steer either. Our comment is right. |
  | The two window bases swapped | Mesa sets `vs_uniforms_offset = VIVS_SH_HALTI5_UNIFORMS_MIRROR(0)` and `ps_uniforms_offset = VIVS_SH_HALTI5_UNIFORMS(0)` (`etnaviv_screen.c:949-950`), which are `0x34000` and `0x36000` (`src/etnaviv/hw/state_3d.xml.h:2038,2042`). Ours are the same two values (`src/vivante/regs.rs:749,751`). |
  | The PS byte offset | Mesa adds `vs->uniforms.count * 4` (`etnaviv_uniforms.c:141-142`); we add `ceil(count/4) * 16` (`src/vivante/shader.rs:275`). Those differ only when `count % 4 != 0`, and **all 307 shaders in `vivante_blobs.rs` have `const_count ≡ 0 (mod 4)`** — checked, histogram `{0: 307}`. Byte-identical on everything we ship. |
  | A units mix-up in `LOAD_STATE` | `load_state()` takes a **byte** address and does `reg >> 2` (`src/vivante/regs.rs:19-21`), exactly Mesa's `base >> 2` (`etnaviv_uniforms.c:148`). |
  | Emission order | ours is shader state, code, uniforms, prefetch (`src/vivante/backend.rs:987-997`), matching Mesa's HALTI5 sequence (`etnaviv_emit.c:777-808`). |
  | The missing post-prefetch stall | Mesa stalls RA against PE after the two prefetches (`etnaviv_emit.c:805-809`); we do the same (`src/vivante/backend.rs:998`). |

  **So the next suspect is upstream of the upload: a block that never
  arrived.** `d.fs_uniforms` is filled only when `gl.fs_uniform_dwords != 0`
  (`src/vivante/backend.rs:1461-1464`) and otherwise stays the empty default
  (`src/vivante/state.rs:100`), while `emit_stage_uniforms` is driven by the
  shader's `const_count`, not by the block — so a short or empty block made
  every `SHC_VIV_CONST_UNIFORM` dword resolve to 0 through an `unwrap_or(0)`,
  which is indistinguishable from a uniform whose value really is zero. That is
  the absent-means-something-specific failure this project keeps paying for, and
  it matches the symptom exactly: the vertex block arrives, the fragment one
  does not, and nothing says so.

  **Prediction, on record before the board run.** `const_value` now logs
  `"{stage} uniform dword N missing, block holds M (reading 0)"` instead of
  returning a silent zero (`src/vivante/shader.rs:203-224`). On the first
  i.MX8MP run of gl_test case 1, either:
  - **the line appears for the fragment stage** — the block never reached the
    draw, the defect is in the GL core's fragment uniform plumbing or in
    `fs_uniform_dwords`, and the fix is there, not in `emit_uniforms`; or
  - **no line appears** — the block arrived and the values were written to the
    right registers, so the remaining suspects are the compiled fragment
    shader's own constant-register indices (its `const_contents` row) and the
    `PS_UNIFORM_BASE` the hardware actually reads.

  One run distinguishes those, which is why the instrument goes first. This is
  behind the kernel-tree gate (`build/imx8mp-ktree`) that cts-runner is
  restoring.
- `vs_load_balancing`, `input_count_unk8`, `VS_HALTI5_UNK008A0` (`:112`) and
  `SH_SPECIALS_BASE` are values carried through from the offline table or
  copied from Mesa. What they mean is not established, and an online compiler
  choosing them freshly rather than copying is an unmeasured change.
- `src/vivante/VIVANTE_NOTES.md` opens with "Nothing here has run on
  hardware". **That line is stale** — the Vivante driver, `gc_triangle` and the
  userspace `/dev/gcgpu` pair have since run on the board (task #35), and the
  fragment-uniform defect above was itself measured there. Read the NOTES for
  the encodings and the task list for what has run; do not take that sentence
  as current.
- Nothing in this document has been checked by generating a shader from it and
  running it. That is M3.
