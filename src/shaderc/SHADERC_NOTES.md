<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Shader back ends — flow, ABI mapping, and how to add shaders

This directory holds the shader back ends (layer 5a of `DESIGN.md`). There
are three kinds, and a build starts with one of them as `g_shaderc` (`mod.rs`
picks it from the features and the build's `mesa_reference` cfg):

- **online** (`online/`, the cargo feature `shaderc-online`): our own compiler,
  no_std and allocation-free, running in the image; it compiles the shaders an
  application writes as it runs (dEQP-GLES2 does). It has one back end per GPU
  (`online/powervr`, `online/vivante`, `online/v3d`, `online/mali`) behind one
  front end; the designs are in `docs/online-shaderc/`. The default.
- **precompiled** (`precompiled.rs`, the cargo feature `precompiled-shaders`):
  shaders our own compiler compiled while the image was built
  (`yakogl-precompile`), found by the compiler's record key (`record_key.rs`).
  In an image with the compiler they are a tier in front of it; without one
  (`SHC_PRECOMPILED`) they are the whole back end and a miss is refused by
  name.
- **offline** (`offline.rs`): a table compiled on the host, looked up at run
  time by the SHA-256 of the GLSL source. Its table is the Mesa reference
  (`tests/reference/`: `blobs.rs` for the PowerVR, `vivante_blobs.rs` for the
  GC7000), which only the host tests and a measurement build link
  (`mesa-reference-table`). The rest of this file, from "Runtime flow" on, is
  about that table and the PowerVR-side tooling that makes it.

## Files

| File | Role |
|---|---|
| `mod.rs` | the shader ABI of `include/gles_shaderc.h` as `#[repr(C)]` (`ShcShader`, `ShcVarying`, `ShcAttrib`, `ShcUniform`, `ShcBackend`, and the link-time attribute map `ShcAttribMap` / `ShcAttribBinding`); defines `g_shaderc` (the offline back end in a build with the Mesa reference, else the compiler, else the precompiled back end); the table's row types `ShcBlob` (sha256, name, stage, `blend_key`, shader; the name is NUL-terminated so `shc_shader_source_name()` can hand C a pointer into the table, and `ShcBlob::source_name()` drops the NUL) and `ShcSource` (name, GLSL text) |
| `offline.rs` | implements `struct shc_backend` (`compile`, `link`, `blend_variant`) as `SHC_OFFLINE`; `shc_offline_register()`; `gles_offline_shader_source()` (`include/gles_offline.h`) |
| `link.rs` | the link rule itself (pair the FS inputs with the VS outputs by name, bounds-check them against the VS output vector; `by_register` for the GC7000), shared by all back ends: it is about the `ShcShader` pair, not about where the shaders came from. Only the prefix the log carries differs, so the caller passes it |
| `sha256.rs` | self-contained SHA-256 (no dependency) — links on Zephyr/baremetal |
| `precompiled.rs` | the precompiled back end (`SHC_PRECOMPILED`) and the tier's look-up in front of the compiler |
| `record_key.rs` | the record key: the source's SHA-256 chained with everything else a compile reads; how the compiler's cache and the precompiled table find a compiled shader |
| `packing.rs` | the resource budget of a GLSL ES 1.00 program: the packing algorithm of GLSL ES 1.00 Appendix A, section 7 |
| `es100_twins.rs` | host test (`cfg(test, mesa_reference)`): `tools/shaders/gl_test_es100/`'s ES 1.00 twins of gl_test's GLSL 450 shaders are the same programs |
| `tests/reference/blobs.rs`, `vivante_blobs.rs` | **generated**. `blobs.rs` by `yakogl-tools glsl2usc` (`tools/glsl2usc.sh`): `SHC_BLOB_TABLE` (USC + metadata per shader and blend variant) and `SHC_SOURCE_TABLE` (the source texts); `vivante_blobs.rs` by `tools/vivante/gen_shader_table.py`. The Mesa reference, `shaderc::mesa_reference`: compiled into the host tests and into a build with `mesa-reference-table`, never into another product build |

All of it is part of the Rust crate `yakogl`.

### The Valhall back end (`online/mali/`, `gpu-mali`)

Mesa 26.2.1's Valhall pipeline, transcribed pass by pass, so that the bytes a
shader compiles to are the bytes Mesa makes for it (`online/mali/mod.rs` has
the module list and the order; milestone L3 of
`driver/docs/nanopc-t6-mali.md`). In order: `nir` (a vertex shader) and
`nir_fs` (a fragment shader) build Mesa's NIR-level graph, `select` and `emit`
select it to BIR (`bir`), `opt` and `fau` run the passes after selection (the
push constants and every constant's form), `sched` is the pre-RA pressure
scheduler, `ra` is LCRA in the split register file, `flow` places message
slots, waits and last-use marks, and `pack` writes the words (the
blend-shader call included, `lower_blend`) and the 128-byte padding. `blend*`
and `motion` carry the blend shaders and the control-flow motion. A vertex
shader yields two programs from one source (Mesa's IDVS split).

Evidence: the programs are compared word for word with the shim driver's
(Mesa 26.2.1 on drm-shim; `tools/mali/testdata/*.expect.json`, drawn by
`tools/mali/gl_ref.py`), and the board's Mesa 25.0.7 differs by the causes
`tools/mali/README-pan_shc.md` names and nothing else. Some pixel results are
checked on the board (`tools/mali/board-ref`).

What is not lowered is refused by name with a "... is not lowered yet" message
(`MaliError`, listed in `MaliError::GAPS`): it is `-ENOTSUP`, never `-EINVAL`,
because the shader is not at fault. Each gap is reached by an ES 1.00 source
in `online/mali/tests.rs`, or is unreachable from ES 1.00 source today and has
its reason in `unreachable_from_es100`. Some combinations are refused rather
than referenced because Mesa's own output there depends on something we do
not reproduce (for example `ColourZeroHalf`, a varying lane paired with 0.0;
`UboFallback`, a push word past Mesa's limit; `Rebalance`, a sum GLSL's
`do_rebalance_tree` reassociates). The generic varying layout is Mesa's link
(`select.rs` `generic_layout`); what `nir_opt_varyings` changes across the
stages is refused by name (`VaryingNotFromAttribute`, `VaryingDuplicate`).

The program link (`shaderc::online::mali::link::link_shaders_mali`) pairs the
two stages' varyings by name and links only when each sits at the same byte
offset in both records' layouts (`MaliShader::varyings`); different offsets
are refused as a gap (`MALI_LAYOUTS_DIFFER`). The online cache takes
`ShcShader::priv_` for its own record and keeps the code generator's pointer
beside it (`online::backend::backend_priv`), which `MaliShader::of` follows.
The frame builder (`src/mali/`) fills the push words from the draw's GL
uniform storage (a fragment shader's as fp16, `frame::f32_to_f16`); on this
back end the front end's matrix product is Mesa's GLSL order
(`MAT_TIMES_VEC_ORDER`, `lower.rs` `mat_times_column`). The record's
sampler binding is `MaliShader::sampler`; the frame builder's descriptors are
pinned to the board's Mesa in `src/mali/frame_tex_tests.rs`.

### The online back end in an image

An image with the compiler links no Mesa table: `CONFIG_YAKOGL_SHADERC_ONLINE`
implies the crate's `no-shader-table` feature. The region the compiler works in
is a static in `src/osal/zephyr_shaderc_scratch.c`, sized by
`CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES` and lent at boot by `SYS_INIT` so no OSAL
has to remember to do it. The trade is that the image shrinks and the memory
grows; `tools/precompile/measure-image.sh` measures both for an image.

**The PowerVR driver's own programs.** `src/pvr/internal.rs` is generated by
`yakogl-precompile --internal` (`tools/precompile`) from the `internal powervr`
lines of `tools/shaders/precompiled.list`: the programs the back end binds
itself, made by our own compiler, except the VDM-sync vertex program, the
passthrough vertex shader and the end-of-tile programs, which our encoder
builds (`assemble::build_vs_nop`, `build_vs_passthrough`, `build_eot`; lines
`vdm_sync`, `vs_passthrough`, `eot1..3`). Mesa 26.2.1's pco programs for the
same jobs are host-test reference material (`tests/reference/pvr_internal.rs`,
`tests/internal_programs.rs`). The C driver's headers
`driver/src/pvr_usc_*_blob.h` are rendered by `tools/pco_shc/gen_usc_blobs.py`
from `tools/pco_shc/usc_blobs.txt` (pco's captured output; `--check` also
compares the clear-attach words with `src/pvr/internal.rs`). Still pco's: the
tile-buffer end-of-tile programs (`eot*_tb*`), which load the tile buffer from
memory (64-bit address immediates, `ld.regbl`, `wdf`) and have no encoder
builder, and `fs_nop`, which nothing binds.

**`gl_VertexID` / `gl_InstanceID` on the GC7000 (online).** Both arrive in
one register after the attribute registers, the vertex id in `.x` and the
instance id in `.y`, reported as `vs_id_in_reg`; the driver feeds it as one
more vertex input and programs `FE_HALTI5_ID_CONFIG` (`src/vivante/shader.rs`).
This is Mesa's etnaviv and the reference compiler's choice for gl_test's
`tools/shaders/es300/vid.vert` (register 1 after `pos` in register 0). Pinned by
`online/codegen.rs` `lowered_ops`
(`gl_vertex_id_is_x_of_the_register_after_the_attributes` and the two after
it). The PowerVR online selector reports the same ids as `vs_vertex_id_reg1` /
`vs_instance_id_reg1` (`online/powervr/chain.rs`).

**ES 3.00 look-ups on the GC7000 (online).** `texelFetch` on a 2D, 3D or
2D array sampler is one `txf` (level as a float in SRC1, 0x1100 in SRC2),
`texture()` / `textureLod()` on a 3D or 2D array sampler the 2D `texld` /
`texldl` with a three-lane coordinate, and `textureSize(s, 0)` reads the
`TEXTURE_WIDTH` / `_HEIGHT` / `_DEPTH` constants the driver fills for the
sampler's unit. All three are Mesa's etnaviv and the reference compiler's
choices for gl_test's `tools/shaders/es300/array.frag`, `tools/shaders/es300/vol.frag`
and `tools/shaders/es300/texsize.frag` (cases 42, 48, 46), pinned by `online/codegen.rs` `lowered_ops`
(`texel_fetch_is_the_reference_txf` and the two after it). Not selected,
refused by name: `textureSize` at another level, shadow samplers, integer
results, gradients, offsets. `es300/texsize.frag` declares
`precision mediump sampler2DArray;`: GLSL ES 3.00 4.5.4 gives that sampler no
default precision.

**Derivatives and `%` on the GC7000 (online).** `dFdx` / `dFdy` are `dsx` /
`dsy` with the source in SRC0 and SRC2, as the reference table and Mesa's
etnaviv emit them; `dFdy`'s source is first scaled by the per-draw y scale
`gl_FragCoord` takes (Mesa's nir_lower_wpos_ytransform `lower_ddy`).
Integer `%` is the exact unsigned remainder of the magnitudes, negated for a
negative dividend (Mesa's nir_lower_idiv); the machine's `imod` is never
emitted, because no reference program contains one.

## Runtime flow (offline)

```
  glShaderSource(text) + glCompileShader
        └─▶ g_shaderc->compile(stage, text, &out, log, sz)
               sha256(text) → search SHC_BLOB_TABLE by {stage, digest}
               hit  → *out = blob.shader  (out->usc points at the embedded array), return 0
               miss → helpful log ("… not in offline table; add it to glsl2usc …"), -ENOENT
                      (src/gl/shader.rs then reports COMPILE_STATUS true and flags the
                       shader `missing`: the application compiles its other stage too,
                       both texts are logged as `shsrc` chunks, and glLinkProgram refuses
                       the pair)
  glLinkProgram
        └─▶ g_shaderc->link(vs, fs, log, sz)
               validate FS input varyings fit the VS output vector → 0 / -EINVAL
```

`compile` matches on **both** stage and digest, so the same source text used for a
VS and an FS never cross-matches. The embedded USC arrays have static lifetime, so
the `out->usc` pointer stays valid after `compile` returns.

## The GLSL ES front end (`tools/yakogl-tools/src/es100.rs`)

Before any of this project's own translation runs, a GLSL ES source goes through
the front end an OpenGL ES 2.0 implementation owes it. It lives in one place,
`es100.rs`, and both the offline table generator (`glsl2usc::frontend_text`,
called once per shader by `resolve_shader`) and the batch service
(`yakogl-tools shader-check`) go through
it, so the two cannot disagree about whether a shader compiles.

| Step | What runs | Why |
|---|---|---|
| validate | `glslangValidator -S vert\|frag <limits> <file>` on the **application's own bytes** | no `-V`/`-G`: those compile for SPIR-V, which applies GLSL 450's rules and refuses `#version 100` outright. Plain `-S` is glslang's ES front end, and a file with no `#version` defaults to ES 100 there, as the ES specification says it must |
| preprocess | `glslangValidator -E -S vert\|frag` | the rewriting in `glsl.rs` is regular expressions over text, so it must never see a declaration inside a false `#if` or a macro it has not expanded. glslang leaves a blank line where a directive was, so line numbers still count the application's source. `#pragma` lines are blanked afterwards: an unrecognised one is ignored by the language and its argument need not even be ASCII |
| translate | `glsl::translate` on the **preprocessed** text | unchanged, except that it now works on live code only |
| link | `glslangValidator -l <limits> <vert> <frag>`, then two rules of our own | see below |

Two things that do not change: the table key stays the SHA-256 of the file's
original bytes (the application still passes byte-identical source), and the
reflection in `resolve_shader` reads exactly the text that was compiled — a
reflection of different text would disagree with the binary.

The limits glslang is given are this implementation's own: `es100::limits_conf()`
builds them from `src/gl/context.rs` and `src/backend.rs`, i.e. from the numbers
`glGetIntegerv(GL_MAX_*)` answers, on top of `glslangValidator -c` (a
configuration file *replaces* glslang's table rather than amending it, so the
defaults have to be the starting point or glslang cannot parse its own built-ins).

**Diagnostics.** The info log is glslang's own text with the file-name echo
removed, so it reads `ERROR: 0:5: '=' : cannot convert ...`: string 0, line 5 of
what the application passed to `glShaderSource`.

### What it does to the committed table

Translating the *preprocessed* text instead of the raw text changes what the
compiler sees: macro expansion before the rewriting lets glslang fold what it
could not fold before, so the code of the entries it touches gets shorter. A
rebuild measured on 2026-09-22 (`tools/glsl2usc.sh`, `pco_shc` from
`mesa-26.2.1`) changed a few hundred entries, every one smaller, none added or
removed. The table is keyed by the SHA-256 of the application's source, which
this work does not touch, so the committed table keeps working as it is. Its
rule is that a shader is verified on silicon, not by its size: rebuild it with
a board run (`gl_test`, the Slint and FemtoVG samples) in the same change.

**The validation means different things to its two callers**
(`glsl2usc::Strictness`). To an application's `glCompileShader` — the path
`yakogl-tools shader-check` takes — the GLSL ES
rules are the contract: a text they reject fails to compile, with the log.
That is what dEQP's `shaders.*.invalid.*` cases check, and until the split
existed those two cases compiled on the board because the generator's lenient
policy was the only one. To the offline table generator the same failure is a
note, for the reason below.

Some shaders in `tools/shaders/` are **not valid GLSL ES**, which the
validation says out loud (`glsl2usc: <name>: not valid GLSL ES (compiled
anyway): ...`) without refusing them: the Qt Quick 3D sweep's harvested shaders
that declare `#version 100` and then use a `sampler2DArray` or a `struct`
uniform block (ES 1.00 has neither). The generator names them and compiles them
as before. Refusing them would delete working rows from the table and stop
those scenes rendering, to satisfy a rule about the *run-time* path;
`shader-check`, which is that path, does refuse them.

### The link rules

`glslangValidator -l` catches what is common to GLSL and GLSL ES: a varying or
uniform whose *type* differs between the stages, and a missing entry point. Two
GL ES 2.0 section 2.10.4 rules it does not apply are in `es100.rs`:

- **a varying the fragment shader uses and the vertex shader never declares**
  (`unmatched_varying`). glslang models an unmatched fragment input as legal,
  which GLSL 4.50 is. Merely *declaring* one is fine — dEQP's
  `shaders.linkage.varying_1` expects that to link — so the uses are counted,
  not the declaration.
- **a uniform of the same name declared at a different precision in the two
  stages** (`uniform_precision_conflict`), struct members included and nested
  structs flattened to their leaves. Only `float`, `int` and the samplers carry
  a precision in GLSL ES; `bool`, `bvec*` and a struct name do not, and treating
  one as if it did invents a conflict out of the stages' differing defaults.

Also here, because glslang does not apply it: **`#extension` must precede any
non-preprocessor token** (GLSL ES 1.00 section 3.4).

**A matrix vertex attribute is split into columns.** `attribute mat4 xf;` is
legal GLSL ES, and GL already says a `matN` attribute occupies N consecutive
locations, one per column. Nothing after the rewriting could take the matrix
itself: `pco_shc` asserts on a matrix input (`pco_nir_pvfio.c:909: lower_pvi:
Assertion 'attrib' failed`) and `struct shc_attrib` holds one location with one
to four components. So the rewriting emits the columns GL was describing all
along:

```
layout(location = 1) in vec4 pvr_mat_xf_col0;    // ...col1, col2, col3
mat4 xf;                                          // file scope, no initialiser
void main() {
    gl_Position = dEQP_Position;                  // the position stays vtxin0
    xf = mat4(pvr_mat_xf_col0, pvr_mat_xf_col1, pvr_mat_xf_col2, pvr_mat_xf_col3);
    ...
```

Three details that are not free choices:

- the matrix variable is at **file scope, declared and not initialised**. An
  initialiser there would not be a constant expression, which GLSL forbids,
  and a local in `main()` would be invisible to a function declared above it
  (dEQP's cases pass the matrix to one).
- the assembly goes **after** a `gl_Position` that `hoist_gl_position` has
  already moved to the front, because `pco_shc` hands out vtxin registers in
  order of first use and the position has to be vtxin0 — unless the position
  statement is itself what reads the matrix, in which case the assembly has to
  come first.
- `reflect_attribs` maps `pvr_mat_<name>_col<k>` back to `<name>` with the
  **matrix** `gl_type`, at every one of the N locations. GL says
  `glGetAttribLocation` of a matrix attribute is the first of them and the
  application binds each column itself, so a lookup by name finding the lowest
  is the answer GL asks for.

The GL core follows, in `src/gl/program.rs`, from one idea: **an attribute is
the reflection row at its lowest location, and rows above it with the same
name are its further columns** (`attrib_base_location`, `attrib_column`,
`attrib_is_whole`). Every attribute of every other type is one row, for which
all three are the identity. That makes

- `glGetAttribLocation` return the lowest of the N locations — by
  construction, rather than by `pco_shc` happening to emit its metadata in
  ascending location order;
- `glBindAttribLocation("xf", 5)` put the columns at 5..5+N-1, each row taking
  the bound index plus its own column. Giving them all the bound index made
  them collide with each other and fail the link, so a bound matrix attribute
  did not work at all before this;
- `GL_ACTIVE_ATTRIBUTES` and `glGetActiveAttrib` count and enumerate one
  `matN`, not N columns.

A binding that would push a column past the vertex inputs (a `mat4` at index
14 of 16) is a link error naming the attribute, which it was not before.

On silicon this is `gl_test` case 63 (`matattr`, `driver/tools/pvrlab/gl_test.c`):
a `mat2`, a `mat3` and a `mat4` attribute, each drawn as a full-screen quad
whose colour is the matrix's **first row**, one element per channel, so a
column that reaches the wrong vertex-input register comes back as the wrong
colour rather than as a plausible picture; the rest of the matrix is added
into the position so that no element is dead. Each width also checks
`glGetAttribLocation`, `GL_ACTIVE_ATTRIBUTES` and `glGetActiveAttrib`.

Measured: `mat2`/`mat3`/`mat4` compile through the real `pco_shc`, and dEQP's
`shaders.functions` passes through the whole pipeline with `--codegen`. No
shader in `tools/shaders/` has a matrix attribute, so the offline table is
untouched.

**The resource budget** (GLSL ES 1.00 revision 17, Appendix A section 7
"Counting of Varyings and Uniforms"; OpenGL ES 2.0.25 sections 2.10.4 and
3.8.1). The appendix's packing algorithm is implemented once, as it is
written, in `src/shaderc/packing.rs`: a grid of 4 columns and as many rows as
the limit the GL reports, the variables ordered mat4, mat2, vec4, mat3, vec3,
vec2, float and largest array first, one row per array element, mat2 taking
two full rows, the vec2 switch to the highest rows, and the float columns
chosen by least space left. Its tests are the appendix's own example (the
printed table, cell for cell) and its "always fails" cases. A plain row count
would have refused `varying float a; varying float b;`, which packs into one
row; this does not. Each class is fed from where its types are known:

- Uniforms, every back end: the GL link (`src/gl/program.rs`
  `check_uniform_packing`) packs each stage's uniform reflection
  (`gl_type`, `array_len`, a table's flattened struct-array rows as one
  variable per outer element) against `MAX_VERTEX_UNIFORM_VECTORS` /
  `MAX_FRAGMENT_UNIFORM_VECTORS` (`src/gl/context.rs`, which `glGetIntegerv` and
  the shader's `gl_Max*UniformVectors` read too). Samplers take texture units,
  not rows (`check_sampler_counts`). Literal constants are not counted: the
  algorithm is a minimum, and counting fewer only accepts more.
- Varyings, online back end (all three GPUs): `link_online` asks
  `reflect::varying_budget` for the varyings statically used in both stages
  plus the fragment built-ins the fragment shader references, against
  `MAX_VARYING_VECTORS`.
- Attributes are not in the appendix: 2.10.4 counts generic attribute
  locations, which the link's attribute layout does (`attrib_layout`,
  aliasing allowed: dEQP `bind_aliasing.max_cond_*` binds twice
  `MAX_VERTEX_ATTRIBS` and must link).

Each refusal is one line in the info log and in glog: "link error: the vertex
shader's uniforms do not fit MAX_VERTEX_UNIFORM_VECTORS (Appendix A packing)",
"... the program's varyings do not fit MAX_VARYING_VECTORS ...",
"... the vertex shader's attributes do not fit MAX_VERTEX_ATTRIBS
(OpenGL ES 2.0.25 section 2.10.4): ..."

What is still not covered, and why:

- Varyings, offline back end: the table's `shc_varying` holds a component
  count per location and no type or array length, and a mat2 and a vec2[2]
  read the same there. Adding the fields changes the table ABI and needs the
  table regenerated, which needs a board run. The table holds only this
  project's own shaders.
- ES 3.00 pairs: GLSL ES 3.00 section 11 counts outputs and inputs apart and
  keeps flat and smooth values in different rows. `packing.rs` takes the ES 3.00
  rule for the non-square matrices; the ES 3.0 link rules are ES 3.0 work
  (`ROADMAP.md`, stage 4).
- The limit is the promise, not the proof that each GPU can carry it:
  the GC7000 link (`link::by_register`) still gives every varying its own
  register, so it refuses more floats than its registers hold even when they
  pack into fewer rows, and its code generator refuses a vertex shader with 16
  outputs before any link (`Abi(LoadBalancingTempsUnmeasured)`, pinned by
  `a_vertex_shader_with_sixteen_outputs_is_refused_by_the_gc7000_code_generator`,
  so the grid-filling varying link tests run on the other GPUs only); and the
  PowerVR online vertex shader has room for a fixed number of output-write
  groups (`online/powervr/assemble.rs` `build_output_writes`), so eight vec4
  varyings each written from constants are refused at compile ("more
  output-write groups than the caller made room for"), although they fit
  `MAX_VARYING_VECTORS`.

## Measuring it: `tools/es100_oracle.py`

dEQP ships its `shaders.*` tests as data, and each case states whether the
program must fail to compile, fail to link, or build. That is an oracle a host
can consult without a GPU. `tools/es100_oracle.py` parses
`<VK-GL-CTS>/data/gles2/shaders/*.test` the way dEQP's own parser does (the same
lexer, the same `${...}` specialization, the same expansion of a `both` case
into `_vertex` and `_fragment`), asks `yakogl-tools shader-check` for a verdict
on each, and reports how many expectations were met.

```sh
tools/es100_oracle.py                 # every sub-case, about three minutes
tools/es100_oracle.py --legacy        # the pipeline as it was before the front end
tools/es100_oracle.py linkage --show-mismatches 20
```

Measured on 2026-09-22, glslang 16.4.0, no `pco_shc`:

| | expectations met |
|---|---|
| before the ES front end (`--legacy`) | 2605 / 2793 — 93.3 % |
| after | 2787 / 2793 — 99.8 % |
| after, with `--codegen` (`pco_shc` too) | 2787 / 2793 — the same six |

The last row is the useful one for a board run: over the whole gles2 shader
set, **`pco_shc` refuses nothing the front end accepts**. Every remaining
mismatch is decided before the machine-code back end is reached, so a case
that builds here builds on silicon.

Which files moved, and by how much, is `tools/es100_oracle.py`'s per-file
output; the matrix-attribute lowering made `conversions`, `functions` and
`linkage` pass completely.

The six that remain are one named thing, and it is not ours:

- **glslang refusing what GLSL ES 1.00 allows**, not this pipeline:
  `#define __VALUE__` (glslang makes a name with consecutive underscores an
  error below ES 3.00; the ES 1.00 specification only reserves it),
  `__FILE__` (glslang wants `GL_GOOGLE_cpp_style_line_directive`), and
  `defined` reached through a macro expansion. They need a change in glslang or
  a different ES front end.
**Fixed since**: `shaders.scoping.valid.local_variable_hides_function_parameter`.
GLSL ES 1.00 makes a function's parameters and its body's outermost
declarations two scopes, so a local may shadow a parameter; GLSL 4.50 makes
them one scope and calls it a redefinition. One more pair of braces around the
body restores the two scopes and changes nothing else, because a function body
is self-contained. `rescope_shadowed_params` applies it only to a function
that actually re-declares one of its own parameters — verified over all 574
shaders in `tools/shaders/`, none of which is touched, so no row of the
offline table moves.

## Metadata → `struct shc_shader` mapping

`glsl2usc` parses `pco_shc`'s stderr metadata into `struct shc_shader`
(`include/gles_shaderc.h`). The mapping:

| `struct shc_shader` field | Source in pco_shc metadata |
|---|---|
| `usc`, `usc_dw` | raw USC dwords on stdout (little-endian) |
| `temps` | `temps=` |
| `num_outputs` (FS) | the sum of every `fs_out[…]: (po\|tilebuf).start=… count=` (a G-buffer shader writes several) |
| `num_tile_buffers`, `tile_buffer_sh` (FS) | `tile_buffers (driver-supplied addresses): sh.start=… buffers=…`: the outputs that did not fit the pixel's four output registers are stored to tile buffers whose addresses the driver writes into two shared registers each |
| `uses_w` / `uses_discard` (FS) | `uses: … w= … discard=` |
| `varyings[]` (FS) | each `fs_in[slot=32+n VARn]: … count=<dwords>` → `location=slot-32`, `components=count/4` |
| `num_attribs`, `attribs[]` (VS) | each `attrib[loc=.. gen=n]: vtxin.start=r count=c` → `location=n`, `vi_reg=r`, `components=c`, `format=0` (f32); `name`/`gl_type` from the GLSL `layout(location = n) in T name;` text |
| `vs_output_size` (VS) | `vtxouts(vs_output_size)=` |
| `vs_psize_off` (VS) | `vs_out[slot=12 ?]: vtxout.start=` (VARYING_SLOT_PSIZ, allocated after the user varyings; `SHC_NO_PSIZE` when the shader does not write `gl_PointSize`) → OUTPUT_SEL.psprite_size_pres |
| `fs_front_face_off1` (FS) | `front_face_op: sh.start=` **plus one** (0 = the shader does not read `gl_FrontFacing`). The backend writes the op (0 keep the ISP's facing, 1 swap it, 2 always front) into that shared register per draw. |
| `fragcoord_ytf_off1` (FS) | always 0 from pco_shc: pco's programs read `gl_FragCoord.y` unchanged. The online compiler sets it **one-based** to the shared register of a (scale, offset) float pair after the uniform block; the backend writes (-1, target height) when the draw's `flip_y` is set and (1, 0) otherwise, and the program computes y' = y * scale + offset. |
| `uniform_dwords` | `push_consts (uniform/DOUTD): … count=<dwords>` (0 if absent). Capped at the prefix the compiler loads when a driver-supplied word follows it in the shared registers (the discard meta word, the front-face op). |
| `uniform_sh_dwords` | the same `push_consts count` before the block is widened to the std430 layout: the prefix the compiled shader reads. `PVR_BACKEND_UNIFORM_PREFIX` in `src/pvr/backend.rs` can DMA only that much, and is **off**: upstream found the count is not a bound on what a shader reads (a Quick 3D post-processing vertex shader reports 64 of 100 dwords and takes its transform from dword 64). |
| `uniforms[]`, `num_uniforms` | **the GLSL text**, not pco_shc: the members of the `layout(push_constant) uniform X { … };` block at std430 dword offsets (checked against `push_consts count`), plus `layout(set, binding) uniform sampler2D` declarations with their binding (`dword_off = SHC_UNIFORM_NO_BLOCK`). Instance-named blocks are rejected (members must be global, as GL names them). |

### Coefficient / vtxout layout the numbers imply

- **FS coefficient (DOUTI) layout** is *derived* by the backend from
  `uses_w` + the varying list, per the toolchain rule: if `uses_w`, the `1/w`
  iterator takes `cf0..3` first, then user varyings in ascending `location`, each
  scalar component consuming one 4-dword coefficient set. `struct shc_varying`
  therefore does **not** store a `cf` offset — it is reconstructed downstream.
- **`shc_varying.vtxout_off`** is the dword offset of the varying in the VS output
  vector. pco_shc's FS metadata does not report it (it is a VS concept), so
  glsl2usc reconstructs the natural packed layout: `gl_Position` occupies dwords
  0..3, then varyings in ascending location. This matches the VS `vs_out`
  offsets pco_shc reports for the paired VS (verified: `color_uv.vert` emits
  VAR0@4, VAR1@8, exactly what `color_uv.frag` expects).

## `link()` validation and its limit

`link()` is `link.rs`, which both back ends call. It checks every FS varying lands
inside the VS output vector:
`VS_POS_DWORDS (4) ≤ vtxout_off` and `vtxout_off + components ≤ vs_output_size`.

It **cannot** do exact per-location component matching against the VS's individual
outputs, because `struct shc_shader`'s vertex section stores only `vs_output_size`
— not the list of VS output varyings (location/components/offset). If a
`vs_varyings[]`/`num_vs_varyings` pair were added to the header, `link()` could
match FS inputs to VS outputs slot-for-slot. See the **Header notes** below.

## Attribute registers decided at link from the bound, active attributes

**The failure it solves.** dEQP-GLES2 `attribute_location.bind_aliasing.max_cond_*`
and `max_inactive_*` declare more attributes than `GL_MAX_VERTEX_ATTRIBS`.
That is legal: `glBindAttribLocation` aliases two of them onto one location,
and only one of each aliased pair is used on any path. ES 2.0 lets the link
fail only if more than the maximum locations are *active* after binding. A
back end that places registers at `glCompileShader` refuses such shaders
before any binding is known:

- PowerVR online: `abi::vtxin_reg` gives the front end's attribute location
  `loc` the four registers `4 * loc ..`, one location per declared attribute,
  and the record's `vi_reg` names them; the register file holds sixteen
  locations (`chain::LIMITS.vtx_in`), so an aliased seventeenth does not
  fit (`Usc(Alloc(OutOfRegisters { file: VtxIn, .. }))`). The file's size is our
  own stand-in (16 locations x 4 components), not a hardware figure: Mesa's
  rogue/pco sources give no vertex-input register maximum, only a per-shader
  count (Mesa's `pco_data.h`, `vtxins`).
- GC7000: `Abi(TooManyIo { .. })` (`online/select.rs`). The 16 is the front
  end's vertex ELEMENTS (`GC_MAX_VERTEX_ELEMENTS`, `src/vivante/state.rs`): an
  element writes one whole vec4 temporary, so 16 active GL locations is the
  hardware limit, no component packing exists, and a matrix is one element per
  column. Two aliased attributes compiled to two input registers cannot both be
  fed from one element, so on the GC aliasing needs the code to name one
  register.

**The design, as built.** Each attribute's registers are decided at link, from
the attributes that are bound and active:

1. `glCompileShader` checks and lowers as usual; the record carries
   `layout_at_link` and a `declared` list (`ShcAttribBinding`: name, columns,
   `gl_type` as declared, `active` = the shader statically reads it).
2. `glLinkProgram` (`src/gl/program.rs` `attrib_layout`, `link_attrib_layout`)
   knows the bindings (`glBindAttribLocation`, else the automatic assignment)
   and places only active entries, per *GL location*: two attributes bound to
   one location share that location's registers, and an attribute the program
   never reads takes no location and aliases nothing. More than
   `GL_MAX_VERTEX_ATTRIBS` active locations is the link error ES 2.0 specifies.
3. The layout reaches the code by recompiling the vertex shader at link with the
   location map (`ShcAttribMap`) as an input, through
   `ShcBackend::compile_vs_linked` (the online compiler is in-process and the
   source is kept), and only when the compile left its inputs to the link or
   the bindings alias or move a location; every other program keeps its
   compile-time layout. Patching placeholder operands afterwards was rejected:
   on the GC7000 allocation has already placed two registers, and merging them
   afterwards can clobber a live temporary. Nothing in the map is back-end
   specific: each back end places its own inputs and reports them as before
   (`abi::attribs_placed` ranks the PowerVR's registers among the read
   locations; `attribs_read` says which are read).
4. The draw fetches each GL attribute once per location into that location's
   registers, as `gles_attrib.vi_reg` describes; aliases then read the same
   registers.

The table path is unchanged: pco's precompiled programs never alias.

**What the GL core answers from.** A relinked record has one row per active
location, so two names on one location share one row, and the second name's
type exists nowhere in the record; hence `declared` carries `gl_type` and
`active`. The linked program keeps its own copy of the active entries.
`GL_ACTIVE_ATTRIBUTES`, `glGetActiveAttrib`, `glGetAttribLocation` and
`GL_ACTIVE_ATTRIBUTE_MAX_LENGTH` answer from that copy, and a name the record
has no row for is still listed at the location the link decided. A map the GL
core builds from rows (bindings that alias, nothing deferred) fills both fields
from the rows itself. A `declared` list with `gl_type` 0 is from a back end
that predates the fields: every entry counts as active, and the rows answer
the queries as before. Which `bind_aliasing` cases still fail on a board is in
`tools/cts/RESULTS.md`.

## Header notes / mismatches (nothing was modified in the shared headers)

The generated table and `offline.rs` match `struct shc_shader` / `shc_varying` /
`shc_attrib` exactly. Two things are worth recording:

1. **No VS output-varying list.** The VS section has `vs_output_size` but not the
   per-varying VS outputs, so `link()` validation is a bounds check, not a
   location/component equality check (above).
2. **Samplers are descriptors, not uniforms.** `pco_shc` (our harness in the Mesa
   tree) gives every `layout(set, binding) uniform sampler2D` a
   `pvr_combined_image_sampler_descriptor` slot in the shared registers, in
   binding order, before the push-constant block: binding b at `sh[20b .. 20b+19]`
   = image words (4), image meta (4: layer size, buffer elems, z slice, pack info
   `0x140` for RGBA8 UNORM), sampler words (4), sampler meta (4: compare op),
   gather sampler words (4). The backend DMAs exactly that (`TEX_DESC_DWORDS`) and
   places the FS uniform block after it. `shc_uniform` carries the binding, and
   `glsl2usc` checks the compiler's `desc[...]` metadata against this layout.
   `interp` in `shc_varying` is `0` (smooth) for every pco_shc-compiled varying:
   pco_shc reports interpolation per-class totals, not per-varying. (Historical
   note: the hand-written `texture.frag` sidecar had no RHW divide and needed npc
   iteration; the backend still forces npc whenever `uses_w = 0`, because a
   perspective iterator without the W plane gave `v = min(u, v)` on silicon.)
3. **Blending is in the FS.** This GPU has no blend unit: `nir_lower_blend`
   rewrites the colour store into a fetch of the on-chip pixel output plus the
   blend arithmetic (`uses_fbfetch`). So the table holds, per fragment shader,
   the plain binary (`blend_key = 0`) and one variant per `--blend` spec given to
   `tools/glsl2usc.sh` (`SHC_BLEND_KEY`, `include/gles_shaderc.h`). `g_shaderc->blend_variant()`
   finds the variant for the draw's blend state; when it cannot (not
   precompiled, the online compiler refused it, it does not link or fit) the
   GL core REFUSES the draw, logs the cause once and counts it
   (`yakogl_draws_refused`); it used to draw unblended, a picture of another
   blend state (the PowerVR's subtract variants, 2026-09-24). The key's bits 26..29 are
   the channels `glColorMask` turns off (`SHC_BLEND_KEY_MASK_OFF`, also FS
   lowered). A variant with a CONSTANT_* factor reads the blend colour from
   shared registers right after its push constants: `blend_consts_off` is that
   dword offset and `uniform_dwords` includes the 4 dwords.

4. **GLSL ES 1.00 is the input language of the table.** `glsl2usc`
   rewrites it to GLSL 450 for the compiler (see `tools/README.md`); the
   table also still takes 450 sources with explicit locations. Varyings
   carry their GLSL names (`shc_varying.name`) and `offline_link()` matches
   fragment inputs to vertex outputs by name, patching `vtxout_off` in the
   program's FS copy; unnamed (explicit-location) varyings keep the
   location-based layout.

### The table and this host's `pco_shc` can disagree

Rebuilt with a `pco_shc` whose usclib lacks `tools/pco_shc/usclib-tex.cl.patch`,
many rows of the table differ: every one is a *fragment* shader and the rebuilt
USC is always the shorter (plain shaders and the float render-target and
`one,inv_src_alpha` variants). It is **not** a stale table. The committed
table is the silicon-verified one; the usclib is the difference, and a host
that cannot regenerate it (its Mesa build graph needs an LLVM it does not
have) links a stale `pco_usclib.spv` in which
`usclib_tex_state_base_level` unpacks no WORD0 and has neither a compare nor a
branch. `tools/README.md`, "Building the pco_shc that reproduces the table",
has the evidence. **Do not regenerate `tests/reference/blobs.rs` with such a
`pco_shc`**: that would drop the patch and bring back the `textureSize()` bug
on mipmapped linear textures it exists to fix.

## Regenerating `tests/reference/blobs.rs`

```sh
tools/glsl2usc.sh           # from tools/shaders/*  (deterministic)
```

See `tools/README.md` for adding shaders and for the prebuilt-sidecar route (no
committed shader needs one any more; `texture.frag` used to).

Writes to `GLSL2USC_CACHE` are not atomic: an interrupted regeneration leaves
zero-length cache files that the next run serves as valid shaders with no code. Delete
the cache directory if a regeneration was killed.

Host tests (`tests/sha256.rs`, `tests/blob_table.rs`;
`cargo test --no-default-features --features host-test,gpu-powervr`): SHA-256 known answers, every
source text in `SHC_SOURCE_TABLE` hashes to a blob row of the same name, the offline
back end finds every source, and the table can be dumped as JSON
(`YAKOGL_DUMP_BLOBS=out.json`) in the format of the dumper used when the table was
converted from the C library's shader table, so the two can be diffed.

## Current table

`tools/glsl2usc.sh` compiles the `src` lines of `tools/shaders/precompiled.list` — the
starter set below, the FemtoVG and `fvgtest_*` sources, and the Qt Quick, Qt Quick 3D and
ES 3.00 sets — once plain and once per blend/mask/render-target variant the list's `blend`
and `for` lines name (`SHC_SOURCE_TABLE`, `SHC_BLOB_TABLE`; a fragment shader
has one row plain and one per `--blend` spec). `yakogl-tools glsl2usc --list <files>`
prints each shader's stage and SHA-256 without emitting the table. Qt shaders
`pco_shc` cannot build are left out rather than failing the whole run
(`--skip-failures`), so the entry count is a little below the file count.

One thing the port does differently from the C library here: `uniform_dwords` stays what
the shader reads (it is the upload size handed to the backend), but `src/gl/program.rs` sizes
the program's uniform block to `max(uniform_dwords, max(dword_off + dwords))` over the
block's members. The reflection can place a uniform past `uniform_dwords` — every FemtoVG
discard fragment shader does — and sizing the block to `uniform_dwords` alone would make
`glUniform4fv` of the last member a silent no-op and `glGetUniformfv` read past the
block.

### The compiler the committed table was built with

`pco_shc` is built from `tools/pco_shc/pco_shc.c` inside a Mesa 26.2.1 tree, following
`tools/pco_shc/README.md`; the build is reproducible (configuring and compiling
twice gives the same binary byte for byte).

The table carries one deliberate deviation from what an unpatched Mesa tree
builds: `tools/pco_shc/usclib-tex.cl.patch`, which `tools/pco_shc/README.md` tells
you to apply. It makes `textureSize()` and the base-level helper skip the
`baselevel` field for a STRIDE (linear) image, whose word 1 holds `num_mip_levels`
there instead. YakoGL samples every texture as STRIDE, so without the patch
`textureSize()` halves every size once per mip level the texture declares.

### The starter set

| Shader | Stage | key metadata |
|---|---|---|
| `uniform_color.frag` | FS | uniform_dwords=4, outputs=1 |
| `passthrough.vert` | VS | 2 attribs |
| `interp_color.frag` | FS | 1 varying (VAR0 vec4 @4), uses_w=1 |
| `color_uv.vert` | VS | 3 attribs |
| `color_uv.frag` | FS | 2 varyings (VAR0 vec4 @4, VAR1 vec2 @8), uses_w=1 |
| `mvp.vert` | VS | 1 attrib, uniform_dwords=16 (mat4) |
| `texture.frag` | FS | 1 varying (VAR0 vec2 @4), uses_w=1; `u_tex` sampler binding 0 -> combined descriptor at sh0..19 |
| `uv.vert` | VS | 2 attribs (pos vi0..3, uv vi4..5); pairs with `texture.frag` |
| `texture2.frag` | FS | two samplers: binding 0 at sh0..19, binding 1 at sh20..39; 0.5 * tex0 + 0.25 * tex1 |
| `points.vert` | VS | pos, colour, per-vertex `gl_PointSize` (vtxout 8); pairs with `interp_color.frag` |

All ten are compiled by the real `pco_shc`. The `blend` lines of
`tools/shaders/precompiled.list` give every one of these fragment shaders its
blend variants.
