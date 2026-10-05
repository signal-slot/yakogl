# Shader back ends — flow, ABI mapping, and how to add shaders

This directory holds the shader back ends (layer 5a of `yakogl/DESIGN.md`). There
are two, and a build has one of them as `g_shaderc`:

- **offline** (`offline.rs`): app shaders precompiled at build time and looked
  up at run time by the SHA-256 of their GLSL source.
- **online** (`online/`, the cargo feature `shaderc-online`): our own compiler,
  no_std and allocation-free, running in the image; it compiles the shaders an
  application writes as it runs (dEQP-GLES2 does).

## Files

| File | Role |
|---|---|
| `mod.rs` | the shader ABI of `include/gles_shaderc.h` as `#[repr(C)]` (`ShcShader`, `ShcVarying`, `ShcAttrib`, `ShcUniform`, `ShcBackend`); defines `g_shaderc` (initially the offline back end); the table's row types `ShcBlob` (sha, name, stage, `blend_key`, shader; the name is NUL-terminated so `shc_shader_source_name()` can hand C a pointer into the table, and `ShcBlob::source_name()` drops the NUL) and `ShcSource` (name, GLSL text) |
| `offline.rs` | implements `struct shc_backend` (`compile`, `link`, `blend_variant`) as `SHC_OFFLINE`; `shc_offline_register()`; `gles_offline_shader_source()` (`include/gles_offline.h`) |
| `link.rs` | the PowerVR link rule itself (pair the FS inputs with the VS outputs by name, bounds-check them against the VS output vector), shared by both back ends: it is about the `ShcShader` pair, not about where the shaders came from. Only the prefix the log carries differs, so the caller passes it. This is the port's `shc_link.c` |
| `sha256.rs` | self-contained SHA-256 (no dependency) — links on Zephyr/baremetal |
| `blobs.rs` | **generated** by `yakogl-tools glsl2usc` (`tools/glsl2usc.sh`): `SHC_BLOB_TABLE` (USC + metadata per shader and blend variant) and `SHC_SOURCE_TABLE` (the source texts) |

All of it is part of the Rust crate `yakogl`.

### The online back end in an image (M5)

The compiler and the offline table are alternatives, so an image that carries
one drops the other. `CONFIG_YAKOGL_SHADERC_ONLINE` selects it and implies the
crate's `no-shader-table` feature; the region it works in is a static in
`src/osal/zephyr_shaderc_scratch.c`, sized by
`CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES` and lent at boot by `SYS_INIT` so no OSAL
has to remember to do it.

Measured on `gc_gl_test` for `verdin_imx8mp/mimx8ml8/a53`, the same tree built
both ways (2026-09-22, commit b56e3d9 + this change):

| | offline | online |
|---|---|---|
| `zephyr.bin` | 2,647,304 B | 922,888 B (**-1,724,416**, -65 %) |
| RAM | 32,520 KiB | 33,396 KiB (**+876 KiB**) |
| table symbols in the ELF | 1199 | **0** |
| `yakogl_shaderc_scratch` | absent | 0x200000 B in `.bss` |

The RAM figure is the net of three things, not one: the table leaves, 2 MiB of
scratch arrives, and the compiler's code arrives with it. The image shrinks and
the memory grows, which is the trade M5 is.

Booted on the i.MX8MP: the online arm of `gc_gl_test` case 108 alone passed
there on 2026-10-04 (e835c0dc, record `594b010e 006cff01 00000001 00000000`).
The size figures above are still the 2026-09-22 ones.

**`gl_VertexID` / `gl_InstanceID` on the GC7000 (online).** Both arrive in
one register after the attribute registers, the vertex id in `.x` and the
instance id in `.y`, reported as `vs_id_in_reg`; the driver feeds it as one
more vertex input and programs `FE_HALTI5_ID_CONFIG` (`vivante/shader.rs`).
This is Mesa's etnaviv (`etnaviv_nir.c`, `etnaviv_compiler_nir_ra.c`) and
the reference compiler's choice for gl_test's `es300/vid.vert` (register 1
after `pos` in register 0). Pinned by `codegen.rs` `lowered_ops`
(`gl_vertex_id_is_x_of_the_register_after_the_attributes` and the two after
it). Until this the online compiler refused the shader with "this built-in
input is not selected yet" (GitHub issue #11), so gl_test cases 43, 44, 45,
49, 57 and 98 failed in an online image. The PowerVR online selector had it
already (`powervr/select.rs`, `vs_vertex_id_reg1`).

**ES 3.00 look-ups on the GC7000 (online).** `texelFetch` on a 2D, 3D or
2D array sampler is one `txf` (level as a float in SRC1, 0x1100 in SRC2),
`texture()` / `textureLod()` on a 3D or 2D array sampler the 2D `texld` /
`texldl` with a three-lane coordinate, and `textureSize(s, 0)` reads the
`TEXTURE_WIDTH` / `_HEIGHT` / `_DEPTH` constants the driver fills for the
sampler's unit. All three are Mesa's etnaviv and the reference compiler's
choices for gl_test's `es300/array.frag`, `vol.frag` and `texsize.frag`
(cases 42, 48, 46), pinned by `codegen.rs` `lowered_ops`
(`texel_fetch_is_the_reference_txf` and the two after it). Not selected,
refused by name: `textureSize` at another level, shadow samplers, integer
results, gradients, offsets. `es300/texsize.frag` declared a
`sampler2DArray` with no precision, which GLSL ES 3.00 4.5.4 does not
allow; it now says `precision mediump sampler2DArray;`.

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
                      (gl/shader.rs then reports COMPILE_STATUS true and flags the
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
what the application passed to `glShaderSource`. Before this, an error's line
number referred to the rewritten GLSL 450 and pointed at a line the application
never wrote.

### What it does to the committed table — measured, not yet on silicon

Translating the *preprocessed* text instead of the raw text changes what the
compiler sees, so `blobs.rs` is due a rebuild. It was run on 2026-09-22
(`tools/glsl2usc.sh`, `pco_shc` from `mesa-26.2.1`) and diffed against the
committed table:

| | |
|---|---|
| entries | 2042 before, 2042 after; none added, none removed |
| USC changed | 300 entries |
| direction | all 300 **smaller** (10 to 108 dwords), none larger; 7,750 dwords less in total |

Macro expansion before the rewriting lets glslang fold what it could not fold
before, so the code gets shorter. **The rebuilt table is not committed**: 300
shader binaries none of which has run on the board, and the rule here is that a
shader is verified on silicon, not by its size. Nothing needs it in the
meantime — the table is keyed by the SHA-256 of the application's source, which
this work does not touch, so the committed table keeps working exactly as it
did. Rebuild it with a board run (`gl_test`, the Slint and FemtoVG samples) in
the same change.

**The validation means different things to its two callers**
(`glsl2usc::Strictness`). To an application's `glCompileShader` — the path
`yakogl-tools shader-check` takes — the GLSL ES
rules are the contract: a text they reject fails to compile, with the log.
That is what dEQP's `shaders.*.invalid.*` cases check, and until the split
existed those two cases compiled on the board because the generator's lenient
policy was the only one. To the offline table generator the same failure is a
note, for the reason below.

Two kinds of shader in `tools/shaders/` are **not valid GLSL ES**, which the
validation says out loud (`glsl2usc: <name>: not valid GLSL ES (compiled
anyway): ...`) without refusing them:

- the Qt Quick 3D sweep's harvested shaders that declare `#version 100` and
  then use a `sampler2DArray` or a `struct` uniform block — ES 1.00 has
  neither, and no ES 1.00 compiler would take them;
- `es300/texsize.frag`, which declares a `sampler2DArray` with no default
  precision for it (GLSL ES 3.00 gives `sampler2D` a default in the fragment
  stage and the array sampler none).

The generator names them and compiles them as before. Refusing them would
delete working rows from the table and stop those scenes rendering, to satisfy
a rule about the *run-time* path; `shader-check`, which is that path, does
refuse them.

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

On silicon this is `gl_test` case 63 (`matattr`, `driver/tools/pvrlab/`):
a `mat2`, a `mat3` and a `mat4` attribute, each drawn as a full-screen quad
whose colour is the matrix's **first row**, one element per channel, so a
column that reaches the wrong vertex-input register comes back as the wrong
colour rather than as a plausible picture; the rest of the matrix is added
into the position so that no element is dead. Each width also checks
`glGetAttribLocation`, `GL_ACTIVE_ATTRIBUTES` and `glGetActiveAttrib`. It is
**not in the regression set** (case 7) until the offline table is rebuilt with
its four shaders, because until then the table does not have them and the
case cannot link.

Measured: `mat2`/`mat3`/`mat4` compile through the real `pco_shc`
(`usc_dw` 24 / 38 / 58 for dEQP's `datatypes` shader), and
`shaders.functions` is 271/271 through the whole pipeline with `--codegen`.
No shader in `tools/shaders/` has a matrix attribute, so the offline table is
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

- Uniforms, every back end: the GL link (`gl/program.rs`
  `check_uniform_packing`) packs each stage's uniform reflection
  (`gl_type`, `array_len`, a table's flattened struct-array rows as one
  variable per outer element) against `MAX_VERTEX_UNIFORM_VECTORS` /
  `MAX_FRAGMENT_UNIFORM_VECTORS` (`gl/context.rs`, which `glGetIntegerv` and
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
shader's uniforms do not fit MAX_VERTEX_UNIFORM_VECTORS = 384 (Appendix A
packing)", "... the program's varyings do not fit MAX_VARYING_VECTORS = 8 ...",
"... the vertex shader's attributes do not fit MAX_VERTEX_ATTRIBS = 16
(OpenGL ES 2.0.25 section 2.10.4): ...".

What is still not covered, and why:

- Varyings, offline back end: the table's `shc_varying` holds a component
  count per location and no type or array length, and a mat2 and a vec2[2]
  read the same there. Adding the fields changes the table ABI and needs the
  table regenerated, which needs a board run. The table holds only this
  project's own shaders.
- ES 3.00 pairs: GLSL ES 3.00 section 11 counts outputs and inputs apart and
  keeps flat and smooth values in different rows; not implemented (ES 3.0 work
  is on hold).
- The limit is the promise, not the proof that each GPU can carry it:
  the GC7000 link (`link::by_register`) still gives every varying its own
  register, so more than 15 varyings are refused even when they pack into 15
  rows, and its code generator refuses a vertex shader with 16 outputs before
  any link (`Abi(LoadBalancingTempsUnmeasured)`, pinned by
  `a_vertex_shader_with_sixteen_outputs_is_refused_by_the_gc7000_code_generator`,
  so the four grid-filling varying link tests run on the PowerVR and V3D rows only);
  and the PowerVR online vertex shader has room for 32 output-write
  groups (`powervr/assemble.rs` `build_shader`), so eight vec4 varyings each
  written from constants are refused at compile ("more output-write groups
  than the caller made room for"), although they fit `MAX_VARYING_VECTORS`.

## Measuring it: `tools/es100_oracle.py`

dEQP ships its `shaders.*` tests as data, and each case states whether the
program must fail to compile, fail to link, or build. That is an oracle a host
can consult without a GPU. `tools/es100_oracle.py` parses
`<VK-GL-CTS>/data/gles2/shaders/*.test` the way dEQP's own parser does (the same
lexer, the same `${...}` specialization, the same expansion of a `both` case
into `_vertex` and `_fragment`), asks `yakogl-tools shader-check` for a verdict
on each, and reports how many expectations were met.

```sh
tools/es100_oracle.py                 # all 2,793 sub-cases, ~3 minutes
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

Per file, the ones that moved: `reserved_operators` 0 → 26, `qualification_order`
18 → 34, `preprocessor` 460 → 526, `linkage` 44 → 55, `constants` 71 → 81,
`functions` 249 → 259, `scoping` 78 → 80, `keywords` 186 → 188, `declarations`
5 → 8, `invalid_texture_functions` 3 → 4.

Per file after the matrix-attribute lowering: `conversions` 524/524,
`functions` 271/271, `linkage` 58/58.

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

## Attribute registers decided at link from the bound, active attributes (design, not built)

**The failure.** dEQP-GLES2 `attribute_location.bind_aliasing.max_cond_*` and
`max_inactive_*` (7 attribute types each, 14 cases) declare more attributes than
`GL_MAX_VERTEX_ATTRIBS` (16). That is legal: `glBindAttribLocation` aliases two of
them onto one location, and only one of each aliased pair is used on any path. ES 2.0
lets the link fail only if more than the maximum locations are *active* after
binding. Both back ends refuse these shaders at `glCompileShader`, before any binding
is known:

- PowerVR online: `Usc(Alloc(OutOfRegisters { file: VtxIn, needed: 65 }))` (AM62P
  host pre-run, f86625e0). `abi::vtxin_reg` gives the front end's attribute location
  `loc` the four registers `4 * loc ..`, one location per declared attribute, and the
  record's `vi_reg` names them. Sixteen declared attributes fill the 64 registers
  (`chain::LIMITS.vtx_in`); the aliased seventeenth needs register 64.
- GC7000: `Abi(TooManyIo { got: 32, max: 16 })` (i.MX8MP rows). There the 16 is
  the front end's vertex ELEMENTS (`GC_MAX_VERTEX_ELEMENTS`, shader.rs VS_INPUT): an
  element writes one whole vec4 temporary, so 16 active GL locations is the hardware
  limit, no component packing exists, and a matrix is one element per column. Two
  aliased attributes compiled to two input registers cannot both be fed from one
  element, so on the GC aliasing needs the CODE to name one register (gc-backend's
  review).

**What is not the fix.** The 64 is our own stand-in (16 locations × 4 components),
not a hardware figure: Mesa's rogue/pco sources give no vertex-input register maximum,
only a per-shader count (`pco_data.h` `vtxins`). Raising it guesses at the hardware.
Packing attributes by component count, as pco does, saves the float and vec2 variants
but not vec4 or the matrices (17 × 4 = 68 registers).

**The design.** Decide each attribute's registers at link, from the attributes that
are bound and active:

1. `glCompileShader` checks and lowers as today, but a vertex shader's vertex-input
   layout is not final: selection reads each declared attribute from a *placeholder*
   register, and the record lists the placeholders per attribute.
2. `glLinkProgram` knows the bindings (`glBindAttribLocation`, else the automatic
   assignment) and which attributes the program actually reads (`attribs_read`). It
   assigns registers per *GL location*: two attributes bound to one location share
   that location's registers, and an attribute the program never reads gets none.
   More than `GL_MAX_VERTEX_ATTRIBS` active locations is the link error ES 2.0
   specifies.
3. The layout reaches the code by (a): recompile the vertex shader at link with the
   location map as an input (the online compiler is in-process and the source is
   kept), and only when binding actually aliases or more than
   `GL_MAX_VERTEX_ATTRIBS` attributes are declared; every other program keeps its
   compile-time layout. (b), patching placeholder operands through a relocation
   table, was rejected in review: on the GC7000 allocation has already placed two
   registers, and merging them afterwards can clobber a live temporary.
   The map the compile gets, per active location: the attribute names bound to it
   and its column count. Nothing back-end specific: each back end places its own
   inputs and reports them as today.
4. The draw fetches each GL attribute once per location into that location's
   registers, as `gles_attrib.vi_reg` already describes; aliases then read the same
   registers.

The table path is unchanged: pco's precompiled programs never alias. The GC7000's
attribute registers get the same link-time assignment in its own back end; the
location map and the link rule are shared GL-core code (`link.rs`).

**Unread attributes take no register -- and that does not settle `max_inactive_*`.**
An attribute the program never reads takes no input register on either back end (the
GC7000's selection drops it; the PowerVR's registers are ranked among the read
locations since 2026-09-25, `abi::attribs_placed`). But measured with dEQP's own
shaders on the host, `max_inactive_*` is still refused ("VtxIn needed 65"): its
"inactive" attributes are read in code the front end keeps. All 14 cases need the
link-time layout above.

**What the GL core answers from (frontend4, 2026-09-25).** A relinked record
has one row per active location, so two names on one location share one row, and
the second name's type exists nowhere in the record. So every `shc_attrib_binding`
of a `layout_at_link` compile's `declared` list carries `gl_type` (as declared,
e.g. `GL_FLOAT_MAT2`) and `active` (1 = the shader statically reads it; loads in
branches the front end keeps count). `glLinkProgram` places only active entries.
An inactive one takes no location and aliases nothing, bound or not. The linked
program then keeps its own copy of the active entries. `GL_ACTIVE_ATTRIBUTES`,
`glGetActiveAttrib`, `glGetAttribLocation` and `GL_ACTIVE_ATTRIBUTE_MAX_LENGTH`
answer from that copy, and a name the record has no row for is still listed at
the location the link decided. A map the GL core builds from rows (bindings that
alias, nothing deferred) fills both fields from the rows itself. A `declared`
list with `gl_type` 0 is from a back end that predates the fields: every entry
counts as active, and the rows answer the queries as before.

**Status.** Written 2026-09-25 by usc-backend, reviewed by gc-backend (approved,
(a) chosen, the GC7000's facts above). Nothing is built yet. Its priority is below the native cube, depth/stencil and clear
work: 14 cases per board.

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
   `glsl2usc.sh` (`SHC_BLEND_KEY`, gles_shaderc.h). `g_shaderc->blend_variant()`
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

### The 307: `blobs.rs` and this host's `pco_shc` disagree

Rebuilt with this host's `pco_shc`, 307 rows of the table differ. Every one
is a *fragment* shader, and the rebuilt USC is always the shorter: 62 are plain shaders (`blend 0x0`), the rest are the float
render-target variants (`@f16x4`, `@f16`, `@f32` = keys `0x4…`, `0x8…`,
`0xc0088621`) and `one,inv_src_alpha` (`0x98661`). Spot-checked by hand:
`es300/array.frag @f32` is 272 dwords in the table and 262 from
`yakogl-tools glsl2usc --blend replace@f32` run here;
`qt3d/db6a67a14bb451a6.frag` plain is 6256 in the table and 6218 rebuilt.

This was chased down and it is **not** a stale table. The committed table is the
silicon-verified one; what this host is missing is
`tools/pco_shc/usclib-tex.cl.patch` *in the built usclib*. It is in the Mesa
source tree, but the `pco_usclib.spv` both local builds link does not have it
(disassembled: `usclib_tex_state_base_level` unpacks no WORD0 and has neither a
compare nor a branch), and that host cannot regenerate the usclib — its Mesa
build graph needs an LLVM it does not have, so `build_pco_shc.sh` only
recompiles `pco_shc.c` and relinks. `tools/README.md`, "The pco_shc this host
has", records the full evidence. **Do not regenerate `blobs.rs` to close the
gap**: that would drop the patch and bring back the `textureSize()` bug on
mipmapped linear textures it exists to fix.

## Regenerating `blobs.rs`

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
converted from `shader_blobs.c`, so the two can be diffed.

## Current table

`tools/glsl2usc.sh` compiles every GLSL file under `tools/shaders/` — the starter set
below, the FemtoVG and `fvgtest_*` sources, and the Qt Quick, Qt Quick 3D and ES 3.00
sets — once plain and once per blend/mask/render-target variant its `BLENDS` and `--for`
lists name. The committed `blobs.rs` holds **541 source texts and 2268 table entries**
(`SHC_SOURCE_TABLE`, `SHC_BLOB_TABLE`; a fragment shader typically has 18 rows, one plain
and one per `--blend` spec). `yakogl-tools glsl2usc --list <files>` prints each shader's
stage and SHA-256 without emitting the table. Qt shaders `pco_shc` cannot build are left
out rather than failing the whole run (`--skip-failures`), so the entry count is a little
below the file count.

One thing the port does differently from the C library here: `uniform_dwords` stays what
the shader reads (it is the upload size handed to the backend), but `gl/program.rs` sizes
the program's uniform block to `max(uniform_dwords, max(dword_off + dwords))` over the
block's members. The reflection can place a uniform past `uniform_dwords` — every FemtoVG
discard fragment shader does — and sizing the block to `uniform_dwords` alone made
`glUniform4fv` of the last member a silent no-op and `glGetUniformfv` read past the
block. Upstream C still does the latter.

### The compiler the committed table was built with

`pco_shc` is built from `tools/pco_shc/pco_shc.c` inside a Mesa 26.2.1 tree, following
`tools/pco_shc/README.md`. The committed table was generated with a binary built from
exactly those sources (LLVM 23.1.1, gcc 16.2.0, `-Dvulkan-drivers=imagination
-Dgallium-drivers= -Dplatforms=`); the build is reproducible — configuring and
compiling twice gives the same binary byte for byte.

Against upstream's own `shader_blobs.c` at `a444fe9` (unchanged since `e6c449c`, as are
`tools/`) the two tables hold the same 2042
entries, in the same order, keyed by the same source SHAs, with the same 533 source
texts. **1744 entries are equal field for field and 298 differ, in the USC alone**
(and in `temps` for the two `texsize.frag` variants). Every wiring field — outputs,
tile buffers, varyings, attributes, uniforms, shared-register offsets — matches.

The 298 are exactly the entries of the 62 shaders that inline a usclib routine
(`samplerCube`, `sampler2DArray`, `sampler3D`, `textureSize`, `texelFetch`), and the
cause is `tools/pco_shc/usclib-tex.cl.patch`, which this repository carries and its
README tells you to apply: it makes `textureSize()` and the base-level helper skip the
`baselevel` field for a STRIDE (linear) image, whose word 1 holds `num_mip_levels`
there instead. YakoGL samples every texture as STRIDE, so without the patch
`textureSize()` halves every size once per mip level the texture declares. The extra
`textype` test costs 10 to 62 dwords of USC in those shaders, which is the whole
difference: **rebuilt with the patch reverted and nothing else changed, the
regenerated table is identical to upstream's `shader_blobs.c` in all 2042 entries and
all 533 source texts.** Upstream's committed table was therefore produced from a Mesa
tree without upstream's own documented patch (it was added in `1ce44f3`, before the
last regeneration in `86c9ca1`), so upstream's `textureSize()` on a mipmapped linear
texture is wrong on silicon and the port's is not. This is the port's one deliberate
deviation from the upstream table, and it is the same one the previous table had.

Measured against the port's previous table (`2263348`, built with a `pco_shc` from the
revision of `pco_shc.c` before `99f6c1d`, which configured only `FRAG_RESULT_DATA0`),
the rebuild changed 175 entries and no others — every one of them towards upstream,
none away from it:

| what changed | entries | why |
|---|---|---|
| `num_outputs`, and the USC with it | 175 | each fragment output now gets its own pixel output registers, so a G-buffer shader reports two, three or four instead of one |
| `num_tile_buffers`, `tile_buffer_sh` | 2 | `0b4df72d7cd2d1fd.frag` (3 × RGBA16F → 1 spilled) and `lightmapuvraster_300.frag` (4 × RGBA16F → 2 spilled) now mark what does not fit the pixel's four output registers |
| `num_varyings`, `varyings`, `uniform_sh_dwords` | 10 | those same two shaders keep the varyings and the uniform block that feed the outputs the old compile had dead-coded away |



### The starter set

| Shader | Stage | USC dw | temps | key metadata |
|---|---|---|---|---|
| `uniform_color.frag` | FS | 16 | 5 | uniform_dwords=4, outputs=1 |
| `passthrough.vert` | VS | 8 | 0 | 2 attribs, vs_output_size=8 |
| `interp_color.frag` | FS | 24 | 5 | 1 varying (VAR0 vec4 @4), uses_w=1 |
| `color_uv.vert` | VS | 10 | 0 | 3 attribs, vs_output_size=10 |
| `color_uv.frag` | FS | 52 | 10 | 2 varyings (VAR0 vec4 @4, VAR1 vec2 @8), uses_w=1 |
| `mvp.vert` | VS | 92 | 16 | 1 attrib, uniform_dwords=16 (mat4) |
| `texture.frag` | FS | 56 | 32 | 1 varying (VAR0 vec2 @4), uses_w=1; `u_tex` sampler binding 0 -> combined descriptor at sh0..19 |
| `uv.vert` | VS | 8 | 0 | 2 attribs (pos vi0..3, uv vi4..5), vs_output_size=6; pairs with `texture.frag` |
| `texture2.frag` | FS | 112 | 22 | two samplers: binding 0 at sh0..19, binding 1 at sh20..39; 0.5 * tex0 + 0.25 * tex1 |
| `points.vert` | VS | 20 | 2 | pos, colour, per-vertex `gl_PointSize` (vtxout 8), vs_output_size=9; pairs with `interp_color.frag` |

All ten are compiled by the real `pco_shc`. The `BLENDS` list in `tools/glsl2usc.sh`
has grown to seventeen specs (the six original ones, `zero,one` for the all-channels-off
mask a discarding shader needs, and the FemtoVG composite operations), so each of these
fragment shaders carries eighteen rows, not six.
