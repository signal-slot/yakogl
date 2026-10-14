<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Transform feedback and rasterizer discard: design

Status: built. The GL half, **tf-api** (state, errors, queries, rasterizer
discard), is `src/gl/tf.rs` (`16ac8641`); the data path, **tf-capture**, is
`src/gl/tf_capture.rs` with the evaluator it runs (`src/shaderc/online/eval.rs`
`run_in`, `src/shaderc/online/fmath.rs`). `src/GL_CORE_NOTES.md` ("Transform
feedback") lists the deviations. What is left is the board run (section 9).

## 1. What it covers

A draw while a transform feedback object is active and not paused is checked
against ES 3.0 2.15.2 (section 5.6) and then **captured on the CPU**: the GL
core lowers the program's vertex shader to IR again, runs it for every vertex
the draw captures, assembles the draw's primitives into the type Begin named,
and writes the captured varyings into the bound buffers. The draw then goes
to the GPU unchanged, or nowhere under `GL_RASTERIZER_DISCARD`. No back end
is involved, so the three GPUs behave the same (section 3 says why none of
them can do it in hardware here).

The dEQP picture. On the AM62P at `c4ccc966` (before the capture), the
mustpass `transform_feedback` group (882 cases) was 675 Fail and 207
NotSupported, and the 56 `lifetime` cases that need a working object gave no
result:

| bucket (AM62P, `c4ccc966`) | cases | cause |
|---|---|---|
| `guardband write: glGetError() returned GL_INVALID_ENUM` | most of the 675 | `glMapBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER)`, a target the image did not accept yet (accepted on main since) |
| `Compile failed: ''` | 185 | the fragment shader refused (`FragmentInputsNotBuilt`); item **tf-infolog** and the online compiler's, not this design |
| `NotSupported (Implementation limits execeeded)` | 207 | GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS was 4 (section 5.1) |
| `lifetime.*`, no result | 56 | the group needs a capture that works |

With the capture, on the host (`tools/cts/host-deqp-run.sh`, PowerVR online
compiler, HOST-ONLY: the Linux OSAL renders nothing, so the image half of
each case compares two black frames; the buffer contents and the query are
real): `functional.transform_feedback` 882 of 882 Pass (675 Pass and 207
NotSupported with the separate limit at 4), and the counts in section 9 for
`lifetime` and `rasterizer_discard`.

The mustpass draws the three base modes only, so strips, loops and fans are
never captured by dEQP; their vertex order is pinned by the host tests
(section 7).

## 2. Citations, and what could not be checked

Every Mesa `file:line` below was read in the Mesa 26.2.1 release tarball
(`https://archive.mesa3d.org/mesa-26.2.1.tar.xz`, the version
`third_party/README.md` pins), or in the subset of it vendored here
(`third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`). The etna_viv rnndb
headers are cited from `driver/src/vivante/hw/*.xml.h`, the same files Mesa
ships as `src/etnaviv/hw/`. The ES 3.0 section numbers are those of ES 3.0.6.
The spec PDF was not available. Where a rule rests on Mesa or dEQP and has not
been checked against the spec text, it says "(verify)". Mesa enforces a
desktop-GL extension's wording in several places, and those can differ from
ES 3.0.

## 3. Per GPU: is there a capture path?

### 3.1 V3D 4.2 (Pi 4): hardware transform feedback exists

Verified in `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`
(`<vcxml gen="4.2" min_ver="42" max_ver="71">`, line 1, so every packet below
applies to 4.2):

- `Transform Feedback Specs` (code 74, `:730-733`): an enable bit and a count
  of 16-bit `Transform Feedback Output Data Spec`s that follow
  (`:750-755`). Each spec has a first VPM vertex value, a count of
  consecutive 32-bit values (minus one, at most 16), an output buffer (0-3)
  and a stream number.
- `Transform Feedback Buffer` (code 73, `:724-728`): the address of each of
  the 4 buffers, their size in 32-bit words, and the buffer number.
- `Flush Transform Feedback Data` (code 75, `:735`),
  `Transform Feedback Flush and Count` (code 31, `:560`), and
  `Wait for transform feedback` (code 14, `:440-442`, binning control list).
- `Primitive Counts Feedback` (code 72, `:700-722`): stores the words written
  to TFB 0-3, primitives generated, primitives written and primitives
  overflowed (`enum Primitive counters`, `:307-315`). It can also store and
  reload the buffer state, which is what pause/resume needs.
- The primitive enum has `*_TF` variants, `POINTS_TF` = 16 to
  `TRIANGLE_FAN_TF` = 22 (`:63-69`). Mesa's 4.2+ draw path leaves
  `prim_tf_enable` at 0 (`src/gallium/drivers/v3d/v3dx_draw.c:1281`, OR-ed
  into `prim.mode` at `:1313-1375`). Capture is switched on by the specs
  packet's enable bit instead.

The capture is done by the **coordinate shader** in the binning pass. The data
specs index the coordinate shader's VPM output, so the TF varyings must be
written to the coordinate shader's VPM segment after its header. In Mesa 26.2.1:

- `src/gallium/drivers/v3d/v3d_program.c:79-175`
  `v3d_set_transform_feedback_outputs()` lists the captured components per
  buffer as VPM slots (padding gaps with `POS.x`, `:105-111`). The first one
  is at VPM offset 6, past the `[X, Y, Z, W, Xs, Ys]` header (`:132`,
  `:138-142`). It packs one spec per run of at most 16 values (`:134-145`).
  It also packs a second set, `tf_specs_psiz`, with every offset one higher,
  for a shader that writes a point size (`:156-169`). There is a hardware
  bug note, GFXH-1559: the first spec must not start at value 8
  (`:147-149`).
- `src/gallium/drivers/v3d/v3dx_emit.c:645-677` emits `TRANSFORM_FEEDBACK_SPECS`
  (enable + specs; the psiz set for points with a per-vertex size,
  `:653-659`). `:679-715` emits one `TRANSFORM_FEEDBACK_BUFFER` per target, at
  `buffer_offset + offset` with the remaining size in words, and clamps the
  offset to the buffer (`:696-701`).
- `src/gallium/drivers/v3d/v3dx_draw.c:202-222` flushes the jobs that read a TF
  buffer before a draw writes it. `:1382-1386` emits
  `TRANSFORM_FEEDBACK_FLUSH_AND_COUNT` after every TF draw ("or the GPU may
  hang"). `:1392-1402` advances each target's offset on the CPU by
  `u_stream_outputs_for_vertices(mode, count)`.
- **Counts come from the GPU**: `src/gallium/drivers/v3d/v3d_context.c:151-175`
  `v3d_update_primitive_counters()` submits the job and waits for the
  primitive counts it writes, and `PIPE_QUERY_PRIMITIVES_EMITTED` (ES's
  PRIMITIVES_WRITTEN) reads `tf_prims_generated` after that
  (`v3d_query_pipe.c:147-155`). Every query end therefore costs a job submit
  and a wait.

Our V3D compiler produces the coordinate shader itself
(`src/shaderc/online/v3d_abi.rs`, "the linked program's three QPU programs"),
so a hardware path would mean: TF outputs appended to the CS's VPM segment, the
specs and buffer packets in `src/v3d/backend.rs`, and the counters read back
through `Primitive Counts Feedback` or computed on the CPU as Mesa does.

### 3.2 GC7000UL (i.MX8MP): this core has no hardware transform feedback

The register headers describe the block:

- a feature bit `chipMinorFeatures6_HWTFB` 0x02000000
  (`driver/src/vivante/hw/common.xml.h:345`);
- a `VIVS_TFB` register block (`driver/src/vivante/hw/state_3d.xml.h:869-914`):
  `TFB_CONFIG` with a `RASTERIZER_DISCARD` bit (`:871-872`), `TFB_COMMAND`
  (`:874`, values ENABLE/DISABLE/RESUME at `:137-139`), `TFB_CONTEXT_BUFFER`,
  `TFB_FLUSH`, `TFB_QUERY_COMMAND`/`TFB_QUERY_BUFFER` (`:876-882`, values at
  `:140-141`), and four `TFB_BUFFER_ADDR/SIZE/STRIDE` plus
  `TFB_DESCRIPTOR_COUNT` (`:884-898`). There are 512 `TFB_DESCRIPTOR` words,
  each with output buffer, input register, component offset and component
  mask (`:900-914`).

Mesa etnaviv **does** drive the block, on cores that have it:

- `src/gallium/drivers/etnaviv/etnaviv_screen.c:268` gives 4 stream-output
  buffers when the core has `ETNA_FEATURE_HWTFB` **or** the `deqp` debug flag
  is set, and 0 otherwise. The flag is "Hacks to run dEQP GLES3 tests"
  (`etnaviv_debug.h:60`, `etnaviv_screen.c:77`). The 64/64 component limits
  are at `:269-270`.
- `etnaviv_state.c:842` `etna_set_stream_output_targets` binds the buffers.
  `:1160-1200` fills `TFB_BUFFER_SIZE/STRIDE/ADDR` per buffer and one
  `TFB_DESCRIPTOR` per captured output. The descriptor's input register is
  the **fragment shader's input register** for that varying (`:1187-1190`,
  `find_register_for_components(fs, output)`), so the hardware captures from
  the VS→FS varying stream.
- `etnaviv_query_acc_xfb.c:11-23` answers `PRIMITIVES_EMITTED` only with
  `HWTFB`. `:32-51` drive `TFB_QUERY_BUFFER/COMMAND` and `TFB_FLUSH`.
- `etnaviv_rasterizer.c:78` maps rasterizer discard to
  `TFB_CONFIG.RASTERIZER_DISCARD`.
- `src/etnaviv/hwdb/etna_hwdb.c:114` takes `HWTFB` from galcore's database.

**The i.MX8MP's core lacks it.** Both `gc7000ULN_v123` entries, chip
0x7000, revision 0x6204, product 0x70003, say `gcFEATURE_BIT_HWTFB 0x0`. The
first entry is at `src/etnaviv/hwdb/nxp/gc_feature_database.h:18839-18843`,
its HWTFB at `:19202`. The second is at `:19733-19737`, its HWTFB at `:20096`.
These are the two entries `src/backend.rs:238-240` already cites for this
board. The older `gc7000UL_6200` entry (`:15263`) says 0 too, at `:15626`.
Mesa's own dEQP-GLES3 runs on this core claim 4 buffers only through the debug
hack.

So the GC7000UL has no capture path. The register block is for other
Vivante cores.

### 3.3 PowerVR BXS-4-64 (AM62P): no capture path we can use

Mesa's PowerVR driver is Vulkan only (`src/imagination/vulkan/`), and its
extension table has no `EXT_transform_feedback`
(`pvr_physical_device.c:122-220`, `pvr_physical_device_get_supported_extensions`).
It also reports `vertexPipelineStoresAndAtomics = false` (`:258`): Mesa does
not let a vertex shader store to memory on this GPU at all. The vertex USC
program writes its outputs into the vertex output buffer that the TA consumes,
and from there they go to the parameter buffer. No stream-out unit is
documented. A vertex-shader variant that stores its outputs to memory would
need USC stores in a vertex stage. Neither Mesa nor our online USC back end
(`src/shaderc/online/powervr/`) does that, so there is no reference
implementation to follow, and a program whose vertex binary came from the
offline table (made by Mesa's `pco_shc`) could never capture. This is §6,
rejected alternatives R2 and R3.

## 4. Decision

| GPU | capture | rasterizer discard | PRIMITIVES_WRITTEN |
|---|---|---|---|
| all three (built) | **CPU capture in the GL core** (§4.1) | GL core: skip the draw and the clear | counted by the CPU capture |
| V3D 4.2, later, optional | hardware TF through the coordinate shader (§3.1), behind the same GL interface | unchanged | counted on the CPU, as now (§4.3) |
| GC7000UL | CPU only: r6204 has no HWTFB (§3.2) | unchanged | CPU |
| PowerVR | CPU only: no stream-out, no VS stores (§3.3) | unchanged | CPU |

### 4.1 Why CPU capture, on every GPU

1. **One implementation for three GPUs**, which can be checked on the host
   against exact expected bytes. A hardware path is board-only to verify on
   the V3D, and unverifiable on the other two.
2. **The dEQP oracle is a CPU computation.** dEQP's transform-feedback
   reference evaluates the outputs from the inputs on the host and compares
   with a tolerance per precision. The front-end oracles already compare our
   IR evaluator with the dEQP reference: values 0 wrong of 13,953 on GLES3
   (`src/GL_CORE_NOTES.md`, "Transform feedback"). A GPU path can only add
   precision differences (lowp/mediump on the USC, the QPU's SFU).
3. **ES 3.0 makes capture simple.** There is no geometry or tessellation
   stage. Indexed draws are an error while TF is active and not paused
   (§5.6), so capture sees `first .. first + count` per instance and never an
   index buffer. Overflow is an error before anything is drawn (§5.6), so
   there are no partial writes. A capture is a loop over
   `instances × count` vertices.
4. **Rasterisation stays on the GPU, unchanged.** The draw goes to the
   back end exactly as without TF, or not at all under discard. No back end
   changes, and no ES 2.0 path changes.

The costs are accepted and stated:

- **Speed.** Measured on the x86 host, release build
  (`tf_capture_tests.rs` `capture_cost`): 1.5 us a vertex for a vertex
  shader of about 20 operations (a mat4 transform, a normalize), and 23 us
  for a one-point draw, which is the lowering and the set-up. An A53/A72 is
  several times slower. That is fine for dEQP (draws of tens to hundreds of
  vertices) and slow for a particle system of 10^5 vertices per frame.
  Capture runs only while TF is active and not paused, so no other draw pays
  anything but one flag test. The V3D hardware path (§4.3) is the remedy
  where it matters.
- **Precision.** The captured values are the evaluator's f32, not the GPU's.
  The GPU still rasterises with its own values, so a captured `gl_Position`
  and the drawn one can differ in the last ulp. ES 3.0 does not require them
  to be bit-identical. dEQP compares the buffer with its reference, not with
  the image.
- **Vertex textures.** The evaluator samples only `texref` model textures
  (`eval.rs:131`). Reading real GL texture storage (tiled per back end) is not
  in scope. A captured draw whose vertex shader samples a texture is refused,
  see §5.7. dEQP's TF shaders do not sample.

### 4.2 The evaluator in the product

`src/shaderc/online/eval.rs` serves two callers with one machine:

- the host oracles (`run`, `run_with_textures`, `run_with_trig_error`,
  `host-test` only): every store collected into an `Outcome`, the host's
  `std` float functions (the oracles' pins were taken with them), texture
  models, and section 5.9's undefined results stop the run;
- the capture (`run_in`, compiled in every build except a `report-es2`
  product build): the working memory is a `Frame` lent from the compile
  scratch (`store::with_scratch`), the stores go through a `Sink`, the
  float functions are `fmath`'s, an undefined result is a value (5.9 says
  undefined, not an error, and a draw must not fail on it), and a loop may
  run 2^20 iterations (ES 3.00 lifts Appendix A) before the draw is refused.
  Faults are a fixed 112-byte text (`Fault`), never a heap string.

`src/shaderc/online/fmath.rs` is the no-libm half: `sqrt`, `trunc`,
`floor`, `ceil` bit for bit the host's (IEEE's correctly rounded square
root; checked on every 997th bit pattern), `exp2` and `log2` within 1 ULP
(f64 series, one rounding; GLSL ES 3.00 4.5.1 asks for 3), `sin` and `cos`
within 2^-23 for |x| < 2^19 (fdlibm's three-piece pi/2 reduction; 4.5.1
leaves their precision undefined).

The lowering never emits an indexed local (`random_full_array_capture` and
`array` capture output arrays written in loops: they lower to selects), so
the evaluator's refusal of one is never reached.

The vertex shader's **IR is not kept** after compile (only the GPU binary and
reflection live in the keep region, `store.rs:124`). Instead, a link with
transform feedback varyings copies the vertex shader's source (as the link
read it, the same text `link_resolve` resolved the names against) into a
heap block owned by the program (`TfProg::src`), freed on relink and on
delete. Each captured draw lowers it again into the scratch and evaluates
there: 23 us for the `capture_cost` shader on the host. The online front end
is in every `shaderc-online` build, so a program whose GPU binary came from
the offline table captures too; its uniforms are moved by name from the
linked stage's layout to the IR's (section 7.2, item 4).

### 4.3 V3D hardware path (later, optional)

Only when an application needs the speed, and only after the CPU path passes
the dEQP TF group on the Pi 4 (it is the oracle for the hardware path).
Interface: the back end gets `GlesDraw.tf: Option<&GlesTfState>` (buffer
addresses and sizes, specs, current offsets). With ES 3.0's no-overflow rule,
the GL core already knows each draw's vertex and primitive counts, so it can
keep advancing offsets and counting on the CPU. Mesa advances its offsets on
the CPU too (`v3dx_draw.c:1392-1402`). That spares the submit-and-wait Mesa
pays for its counts (`v3d_context.c:151-175`). Constraints from §3.1:

- a spec covers at most 16 consecutive values, and there are 4 buffers. That
  covers ES 3.0's 64 interleaved and 4 × 4 separate;
- the specs start at VPM value 6, and 7 with a point size;
- GFXH-1559;
- a `TRANSFORM_FEEDBACK_FLUSH_AND_COUNT` after each TF draw.

Not built.

## 5. GL-layer specification

All of this is ES 3.0 state. §8 says what an ES 2.0 build keeps.

### 5.1 Limits

| pname | value | ES 3.0 minimum (table 6.34, verify) |
|---|---|---|
| `GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS` | 64 | 64 |
| `GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS` | 4 | 4 |
| `GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS` | 64 | 4 |

These are constants in `src/gl/tf.rs` (`TF_MAX_INTERLEAVED_COMPONENTS`,
`TF_MAX_SEPARATE_ATTRIBS`, `TF_MAX_SEPARATE_COMPONENTS`). The capture runs on
the CPU and has no hardware ceiling. The separate limit is the interleaved
one, as Mesa's softpipe, llvmpipe and etnaviv report it
(`max_stream_output_separate_components = 64`, `sp_screen.c:284`,
`lp_screen.c:263`, `etnaviv_screen.c:269`): at 4, dEQP's 207
`transform_feedback.{basic_types,array}.separate` cases of a matrix or an
array varying are NotSupported ("Implementation limits execeeded", measured
on the host and the AM62P). A vertex shader has 64 output components, so no
single varying can exceed 64; the link check stays for the rule's sake. The
V3D hardware path (§4.3), if built, takes 16 values a spec and 4 buffers.

### 5.2 Objects and binding points

A transform feedback object (ES 3.0 2.15.1) is **per context**: TF objects are
not shared (verify, ES 3.0 D.1.1 lists the shared kinds). The existing pool is
already per context (`es3.rs:108`, `FEEDBACKS[MAX_CONTEXTS][MAX_FEEDBACKS]`).
It grows from `Es3Name` to:

```rust
pub(super) struct TfObject {
    n: Es3Name,                 // used = generated, bound = was ever bound
    active: bool,
    paused: bool,
    prim_mode: GLenum,          // GL_POINTS / GL_LINES / GL_TRIANGLES at Begin
    program: GLuint,            // the program current at Begin (slot)
    bind: [TfBinding; 4],       // indexed GL_TRANSFORM_FEEDBACK_BUFFER points
    written: [u32; 4],          // bytes written since Begin, per binding
}
struct TfBinding { buffer: GLuint /* slot */, offset: GLintptr, size: GLsizeiptr /* 0 = Base */ }
```

- Object 0 is the default object. It always exists and is the initial
  binding. `MAX_FEEDBACKS` goes from 8 to **32** (dEQP lifetime's `gen` and
  `delete` cases create several. 32 × ~80 B × `MAX_CONTEXTS` (2) ≈ 5 KiB of
  .bss). Slot 0 is the default object's state.
- The **generic** `GL_TRANSFORM_FEEDBACK_BUFFER` binding is context state.
  The field `GlContext::bound_transform_feedback_buffer` already exists
  (`gl_context.h:261`, ABI-pinned at offset 23584), and Mesa keeps it as
  `ctx->TransformFeedback.CurrentBuffer`, outside the object
  (`src/mesa/main/transformfeedback.c:188`, `:215`, `:566`). The **indexed**
  bindings, START and SIZE are object state.
- `glBindBufferBase/Range(GL_TRANSFORM_FEEDBACK_BUFFER, i, buf, ...)` sets
  `bind[i]` of the **bound** object and the generic binding. Buffer slots go
  through `names::slot_of`, as `bind_uniform_buffer` does (`es3.rs:969-985`).
  Base records size 0 = "the whole buffer at draw time".
- Queries: `glGetIntegerv(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING)` → generic.
  `glGetIntegeri_v` / `glGetInteger64i_v` with `..._BUFFER_BINDING / START /
  SIZE` → the bound object's `bind[i]`, `GL_INVALID_VALUE` for i ≥ 4. This
  replaces the `Ok(0)` arm at `state.rs:1264-1266`.
  `GL_TRANSFORM_FEEDBACK_ACTIVE / PAUSED` → the bound object (replaces
  `state.rs:1143`). `GL_TRANSFORM_FEEDBACK_BINDING` stays as is.
- **Deleting a buffer** that is bound: the generic binding and the indexed
  bindings of the **currently bound** TF object become 0 (ES 3.0 2.9.1 and
  the container rule of D.1.2, verify). A non-current object keeps its
  reference. The storage then has to stay alive while the object holds it,
  which is the orphan rule `gl/names.rs` already applies to framebuffers. dEQP
  `lifetime.attach.deleted_{name,output}.buffer_transform_feedback` test
  exactly this. The model: a per-buffer `tf_refs` count. `glDeleteBuffers`
  defers the free while it is non-zero, and the name is freed (`glIsBuffer`
  false) at once.
- **Deleting a TF object**: deleting the bound one rebinds 0. Deleting an
  active one is `GL_INVALID_OPERATION`. Mesa walks the list in order and
  returns at the first active object, so the names before it are already
  deleted and the rest are not (`transformfeedback.c:1218-1228`). Do the
  same. The referees are dEQP
  `negative_api.transform_feedback.delete_transform_feedbacks` and
  `lifetime.delete_active.transform_feedback`. Releasing an object drops its
  buffer refs.
- `glBindTransformFeedback` on a name never generated (or deleted) is
  `GL_INVALID_OPERATION`. TF objects need glGen (ES 3.0 2.15.1), and
  `lifetime.bind_no_gen.transform_feedback` expects the refusal. That is
  today's behaviour (`es3.rs:379`). Keep it, and do **not** route TF names
  through `names.rs`.

### 5.3 Program state: varyings

Per program (in `program.rs`'s `ProgramState`, next to `tf_varyings` at
`program.rs:103`):

- **pending** (set by `glTransformFeedbackVaryings`): the names, copied at
  the call into a host-heap buffer, and the buffer mode. They take effect at
  the next link only (ES 3.0 2.11.8).
- **linked** (set by a successful link): `[TfVaryingRow]` with name, GL
  type, array size (1, or N for a whole array), components per element, and
  where to read it: an output's IR slot plus component offset, or a built-in
  (`gl_Position`, `gl_PointSize`). Also the buffer mode, the per-binding
  stride (interleaved: the sum of components × 4; separate: the varying's
  size), and the VS source copy (§4.2). The default before any link is 0
  varyings, `GL_INTERLEAVED_ATTRIBS`.

**Resolution at link** against the vertex stage's reflection
(`reflect::VaryingRow`, `reflect.rs:93-105`: name, `gl_type`, `components`,
`count`):

1. `name` equal to a declared vertex output: the whole variable (an array is
   `count` elements).
2. `name[i]`: element i of an array output. `i` out of range is a link error.
3. `gl_Position` (vec4) and `gl_PointSize` (float).
4. Anything else is a link error. Mesa's message is "Transform feedback
   varying X undeclared." (`src/compiler/glsl/gl_nir_link_varyings.c:2137-2147`).
   A subscript on a non-array is "...requested, but X is not an array."
   (`:1826-1830`). Matching is case-sensitive. A struct member (`s.m`) is
   refused with its own message unless ES 3.0 2.11.8 allows it (verify. As
   far as its names show, dEQP's mustpass captures no struct members).
5. The same variable **and the same array index** twice is a link error,
   "Transform feedback varying X specified more than once." Mesa reads the
   rule that way so that the elements of one array can be captured separately
   (`gl_nir_link_varyings.c:2188-2206`). So `v[0]` and `v[1]` are fine, and
   `v[0]` twice is not.
6. Limits: interleaved, the total components > 64, is a link error. Mesa's
   message is "The MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS limit has
   been exceeded." (`:1952-1959`). Separate, one varying's components > 4 (a
   `mat2` or larger, a `vec2[3]`) gives "Transform feedback varying X exceeds
   MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS." (`:1848-1855`). The count > 4
   for separate is caught earlier, at the call (§5.6).
7. A declared output that the shader never writes is accepted, and captures
   0 (the value is undefined. 0 is deterministic for tests).

This replaces the "INTERLEAVED fails the next link" rule
(`program.rs:1647-1654`).

Queries:

- `glGetProgramiv`: `GL_TRANSFORM_FEEDBACK_BUFFER_MODE`,
  `GL_TRANSFORM_FEEDBACK_VARYINGS` (the linked count),
  `GL_TRANSFORM_FEEDBACK_VARYING_MAX_LENGTH` (the longest linked name + 1, or
  0 with none).
- `glGetTransformFeedbackVarying(program, index, ...)`: the linked row
  `index`. `size` is the array size, `type` the GL type, and the name is
  truncated to `bufSize - 1` plus NUL. Errors: program is not a program
  object (`not_a_program`: `GL_INVALID_VALUE` for a non-name,
  `GL_INVALID_OPERATION` for a shader), index ≥ the linked count →
  `GL_INVALID_VALUE`, bufSize < 0 → `GL_INVALID_VALUE` (keep today's
  order).

### 5.4 Begin / End / Pause / Resume

| call | effect |
|---|---|
| `glBeginTransformFeedback(mode)` | `active` = true, `paused` = false, `prim_mode` = mode, `program` = current, `written` = 0 for every binding |
| `glPauseTransformFeedback()` | `paused` = true. Draws stop capturing. `written` is kept. |
| `glResumeTransformFeedback()` | `paused` = false. Capture continues at `written`. |
| `glEndTransformFeedback()` | `active` = `paused` = false |

### 5.5 Draws

While the bound object is active and **not paused**:

- `glDrawArrays` / `glDrawArraysInstanced`: validated (§5.6), then captured
  (§7.2), then drawn on the GPU unless discard is on (§5.8).
- `glDrawElements`, `glDrawRangeElements`, `glDrawElementsInstanced`:
  `GL_INVALID_OPERATION` "regardless of mode". Mesa quotes the ES 3.1 text,
  which keeps 3.0's rule, and applies it to every ES 3 context without
  `OES_geometry_shader` (`src/mesa/main/draw_validate.c:510-538`).

While paused, or with no object active, every draw is as today.

### 5.6 Errors (ES 3.0 2.11.8, 2.15, 2.16, 6.1. Each is a tf-api host test)

The order within a call follows dEQP's `es3fNegative*ApiTests.cpp` sections.
The checks that come first today keep their place.

`glTransformFeedbackVaryings(program, count, varyings, mode)`:
not a program → `not_a_program` (as today). count < 0 → `INVALID_VALUE`.
mode not INTERLEAVED/SEPARATE → `INVALID_ENUM`. SEPARATE and count > 4 →
`INVALID_VALUE`. The current TF object active, even if paused →
`INVALID_OPERATION`. Mesa checks that first, from ARB_transform_feedback2
(`transformfeedback.c:925-933`); verify that ES 3.0 2.15 has it. Mesa's
order is: active, mode, count, program (`:929-957`). Today's order (program
first, `es3.rs:440-456`) is what dEQP passed against, so keep it and add the
active check last. Count > 0 with `varyings` NULL: record nothing and
`INVALID_VALUE`. There is no rule in the spec. This is the repo's NULL
convention, as in glDrawBuffers.

`glBindTransformFeedback(target, id)`: target ≠ `GL_TRANSFORM_FEEDBACK` →
`INVALID_ENUM`. The bound object active and not paused → `INVALID_OPERATION`.
id not generated → `INVALID_OPERATION`. This is Mesa's order
(`transformfeedback.c:1185-1196`, `:1160-1165`).

`glDeleteTransformFeedbacks(n, ids)`: n < 0 → `INVALID_VALUE`. Any id active →
`INVALID_OPERATION` (§5.2; `transformfeedback.c:1210-1228`).

`glBeginTransformFeedback(mode)`: No current program → `INVALID_OPERATION`.
The program has 0 linked TF varyings → `INVALID_OPERATION`. mode ∉ {POINTS,
LINES, TRIANGLES} → `INVALID_ENUM`. Already active → `INVALID_OPERATION`. A
binding the program needs has no buffer (interleaved: binding 0; separate:
bindings 0..count-1) → `INVALID_OPERATION`. That is Mesa's order
(`transformfeedback.c:345-396`). Today's code checks the mode first
(`es3.rs:393`). Take dEQP's negative_api section order where they disagree.

`glEndTransformFeedback()`: not active → `INVALID_OPERATION`.
`glPauseTransformFeedback()`: not active, or already paused →
`INVALID_OPERATION`.
`glResumeTransformFeedback()`: not active, or not paused →
`INVALID_OPERATION` (`transformfeedback.c:1327-1330`). Current program ≠
`program` at Begin → `INVALID_OPERATION` (`:1333-1342`, quoting
ARB_transform_feedback2; verify ES 3.0).
`glPauseTransformFeedback()`: Mesa `:1276-1278`.
`glEndTransformFeedback()`: Mesa `:538-539`.

`glUseProgram(p)`: TF active and not paused → `INVALID_OPERATION`
(`src/mesa/main/shaderapi.c:2213-2216`).
`glLinkProgram(p)`: p in use by **any** active TF object, bound or not,
paused or not → `INVALID_OPERATION`. Mesa: `shaderapi.c:1385-1395`, which
walks every object, `transformfeedback.c:59-90`. So `program` at Begin is
per object, and the check walks the pool.
`glProgramBinary`: the same rule as link.

`glBindBufferBase/Range(GL_TRANSFORM_FEEDBACK_BUFFER, i, ...)`: The bound
object active (paused or not) → `INVALID_OPERATION`, before the index check
(`transformfeedback.c:594-600`, `:668-680`). i ≥ 4 → `INVALID_VALUE`. Range:
size ≤ 0 → `INVALID_VALUE`, offset or size not a multiple of 4 →
`INVALID_VALUE`. Today's `offset < 0 || size <= 0` check (`es3.rs:1001`)
stays first.

Draws while active and not paused: mode not compatible with `prim_mode`
(POINTS ← POINTS; LINES ← LINES, LINE_LOOP, LINE_STRIP; TRIANGLES ←
TRIANGLES, TRIANGLE_STRIP, TRIANGLE_FAN) → `INVALID_OPERATION`. This is
Mesa's mask, `draw_validate.c:358-374`. An indexed
draw → `INVALID_OPERATION`. Not enough room: for some binding,
`written + vertices × stride > avail` → `INVALID_OPERATION`, nothing captured
and nothing drawn. Here vertices = primitives(mode, count) × verts-per-prim ×
instances, and avail = buffer size − offset (read at draw time), cut to the
range's size for Range (a buffer respecified smaller after the bind cuts it,
as Mesa's `compute_transform_feedback_buffer_sizes` does). A buffer the
draw would write that is mapped → `INVALID_OPERATION` (ES 3.0 2.10.3). An
instanced draw that the back end cannot take in one (a divisor above its
`max_instance_divisor`) is drawn instance by instance (`draw_instanced`);
its room is checked for all the instances before the first one captures. Mode compatibility is checked after today's draw
argument checks (`INVALID_ENUM`/`INVALID_VALUE` first).

`glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, id)`: id 0 or not
generated → `INVALID_OPERATION`. A query already active on this target →
`INVALID_OPERATION`. id active on another target, or created with another
target → `INVALID_OPERATION`.
`glEndQuery(target)`: none active on target → `INVALID_OPERATION`.
`glGetQueryObjectuiv(id, ...)`: id not a query object, or active →
`INVALID_OPERATION`. pname bad → `INVALID_ENUM`.

### 5.7 Queries

ES 3.0 has **one** primitive query, `GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN`
(2.16). `GL_PRIMITIVES_GENERATED` (0x8C87) is not ES 3.0. It comes with ES
3.2 / `EXT_geometry_shader`, so `glBeginQuery(0x8C87)` **stays
`GL_INVALID_ENUM`** (`query_target`, `es3.rs:240-245`). The plan's wording
"PRIMITIVES_WRITTEN and PRIMITIVES_GENERATED queries" is read as the one ES
3.0 query.

- Query objects get a type on their first Begin and keep it. There is one
  active query per target per context. The `Es3Name` query pool becomes
  `{n, target, active, result: u32, available: bool}`. **occlusion-query**
  plugs its counter into the same pool. tf-api builds the generic part and
  leaves `GL_ANY_SAMPLES_PASSED*` refused at Begin, as today, so the two
  items touch disjoint arms.
- PRIMITIVES_WRITTEN adds, for each captured draw, the number of primitives
  whose vertices were written. With ES 3.0's overflow rule that is all of
  the draw's primitives, or none if the draw was refused. The capture is
  synchronous, so `GL_QUERY_RESULT_AVAILABLE` is `GL_TRUE` as soon as
  `glEndQuery` returns, and `GL_QUERY_RESULT` never waits. A query that is
  active while TF is inactive or paused counts 0.
- `glGetQueryiv(target, GL_CURRENT_QUERY)` gives the active id on that
  target, or 0.
- A captured draw that is **refused** (§5.6, or the evaluator refusal below) adds nothing.

**Refusal of a draw the evaluator cannot run.** A vertex shader that samples
a texture (found in the IR before anything is written), an attribute element
outside its buffer, a uniform the shader reads that the linked stage lays
out differently, or any evaluator `Err` makes the draw `GL_INVALID_OPERATION`
with the log line "transform feedback capture: <reason>". Nothing is drawn
or counted; a fault in the middle of a draw (a loop that does not end) may
have written the records before it. ES 3.0 has no such error, so it is a
known deviation, listed in `GL_CORE_NOTES.md`. It is in the repo's practice
of failing loudly (`es3.rs:25-31`) rather than capturing zeros.

### 5.8 Rasterizer discard (ES 3.0 3.1, and 4.2.3 for clears)

With `GL_RASTERIZER_DISCARD` enabled:

- **Draws**: all argument and TF validation and the TF capture happen. Then
  the draw stops in the GL core, before `submit_draw` (`draw.rs:400`). No
  back end sees it, so it writes no colour, depth or stencil, and an occlusion
  query counts no samples. This is the same for all five draw entry points.
- **Clears**: `glClear` and `glClearBuffer{fv,iv,uiv,fi}` validate their
  arguments, then do nothing (no pending clear is recorded, `draw.rs:46-140`).
  This comes from dEQP's `rasterizer_discard.*.clear_{color,depth,stencil}`
  (9 of the 51). Mesa returns on `ctx->RasterDiscard` in `glClear` **after**
  the incomplete-framebuffer error (`src/mesa/main/clear.c:176-183`), and in
  `glClearBuffer*` after their argument checks (`:717-718`, plus the
  per-buffer conditions at `:372`, `:392`, `:485`, `:585`, `:619`). Do the
  same: errors first, then return.
- **Not affected**: `glReadPixels`, `glBlitFramebuffer`, `glCopyTex*`,
  `glGenerateMipmap`, and uploads (verify Blit against ES 3.0 4.3.3. If ES
  3.0 is silent, leave it unaffected as GL does).
- The GC7000's `TFB_CONFIG.RASTERIZER_DISCARD` bit (§3.2; etnaviv sets it at
  `etnaviv_rasterizer.c:78`) is **not** used. It belongs to a unit r6204
  lacks, and the GL core drops the work before the back end, the same on all
  three.

This part needs no capture; tf-api built it, and the AM62P passed the 51
cases at `c4ccc966`.

## 6. Rejected alternatives

- **R1: Hardware TF on all three GPUs.** Not possible. The PowerVR has none
  (§3.3), and the i.MX8MP's GC7000UL lacks HWTFB (§3.2). Only the V3D has
  it (§3.1). A V3D-only hardware path plus a CPU path for the other two would
  be two implementations from the start. Rejected for the first version. The
  V3D path is kept as §4.3.
- **R1b: Use the GC7000 TFB block anyway**, as Mesa's `deqp` debug flag
  claims it (`etnaviv_screen.c:268`). The vendor's database says this core
  does not have the unit. Programming it would be undefined behaviour on
  silicon that the vendor says lacks it. Rejected.
- **R2: GPU vertex pass that writes to a scratch buffer by `gl_VertexID`,
  then CPU primitive assembly.** It would avoid the evaluator's precision and
  speed costs, but needs a memory-store vertex shader on each GPU. On the
  PowerVR, Mesa does not allow vertex-stage stores at all
  (`pvr_physical_device.c:258`). Our compilers emit no USC stores and no QPU
  TMU writes in a VS, and there would be GC7000 stores too. That is three new
  code generator features to save CPU time that dEQP does not need. Rejected
  for now. It is the natural successor if a PowerVR or GC7000 application
  needs fast TF.
- **R3: PowerVR vertex-shader variant that stores to the TF buffer directly**
  (the USC's memory store at address = base + f(vertex index, primitive
  unrolling) × stride, the base and stride as per-draw shared registers).
  Strips and fans write one vertex into several primitives, so the shader
  would need the unrolled index map (or the GL core a list-mode index
  expansion of every strip and fan draw), and a program whose vertex binary
  came from the offline table could never capture. It inherits all of R2's
  cost plus that, and no part of it can be checked off silicon: Mesa never
  issues a vertex-stage store on this GPU, so there is no reference
  encoding or ordering against the TA to compare with. Rejected; the CPU
  capture passes the whole dEQP group on the host.
- **R4: Keep the IR of every program alive for capture.** It costs
  keep-region space (`KEEP_BYTES` 512 KiB, `store.rs:124`) for every program,
  ES 2.0 ones included, to save a re-lower on the rare TF draw. Rejected in
  favour of the per-program source copy (§4.2), which costs only programs with
  TF varyings.
- **R5: Answer the queries and accept Begin without capturing** ("plausible
  zero"). It would turn the 413 + 421 cases into wrong-buffer failures, hide
  the gap, and contradict `es3.rs:25-31`. Rejected. Until the capture was
  built, tf-api refused such a draw with `INVALID_OPERATION`.
- **R6: Expose TF only in builds for the V3D.** dEQP's TF and lifetime groups
  would then stay failed on two of the three boards, for no saving: the CPU
  path is GPU-independent. Rejected.
- **R7: Report discard to the back ends** (cull everything, or the GC7000's
  `TFB_CONFIG` bit, which etnaviv sets at `etnaviv_rasterizer.c:78` but which
  belongs to the unit r6204 lacks). The draw would still pay the back end's
  setup, and three back ends would change where one GL-core early return does
  the same. Rejected.

## 7. Items and host tests

`src/gl/tf.rs` holds the object state, the varying resolution and the draw
rules, `src/gl/tf_capture.rs` the capture; their tests are
`src/gl/tf_tests.rs`, `src/gl/tf_capture_tests.rs` (on `testutil::TestGl`,
which records draws, cache operations and pending kicks and reads what the
GPU sees of a buffer) and `tests/es3_tf_api.rs` (through the C ABI, with a
stand-in back end). Edits to `es3.rs`, `state.rs`, `program.rs`, `draw.rs`
and `buffer.rs` are the hooks.

### 7.1 tf-api (built, `16ac8641`)

The `TfObject` pool (§5.2), indexed bindings and the buffer-delete orphan
rule, varying recording and link resolution (§5.3), Begin/End/Pause/Resume
with every error of §5.6, the query object model, the limits (§5.1) and
rasterizer discard for draws and clears (§5.8). Tests: `tf_tests.rs` (17,
one of them `es2_answers_are_unchanged` under `report-es2`) and
`es3_tf_api.rs` (8).

### 7.2 tf-capture (built)

1. The evaluator in product builds (§4.2): `eval::run_in` over a `Frame`
   and a `Sink`, `fmath` for the float functions, compiled in every build
   but a `report-es2` product build.
2. The vertex shader's source kept at link (`TfProg::src`, §4.2).
3. Vertex inputs per vertex: each attribute column the IR loads, from the
   generic attribute the linked stage's location maps to
   (`attrib_app_index`), through `draw::attrib_element` (the draw's
   conversion, `attrib_row`; `cache_invd` before a CPU read, the OSAL bounce
   for a buffer with no CPU view, client arrays as the draw trusts them);
   the glVertexAttrib* value of a disabled array; element
   `instance / divisor` of an instanced array; `gl_VertexID` = first + i,
   `gl_InstanceID` = the instance.
4. Uniforms: `program::fill_uniform_blocks` and the gl_DepthRange rows
   first, as a draw fills them, then each IR uniform's dwords from the
   linked vertex stage's row of the same name (the same layout when the
   stage was compiled online; a row missing or of another size is refused
   only if the shader reads it).
5. Primitive assembly, Gallium's decomposition with the last vertex
   provoking (`draw_decompose_tmp.h:79-160`, Mesa 26.2.1), the order
   `draw.rs`'s `restart_to_lists` uses too: LINE_STRIP (i, i+1), LINE_LOOP
   adds (n-1, 0), TRIANGLE_STRIP even (i, i+1, i+2) and odd (i+1, i, i+2),
   TRIANGLE_FAN (0, i+1, i+2). Incomplete primitives are dropped; instances
   loop outermost. Each vertex is evaluated once per instance, a strip's
   and a fan's shared vertices read from a four-entry cache.
6. Write-out: interleaved, the varyings of a vertex in the order they were
   named, 4-byte components packed at `offset + written[0]`; separate,
   varying i to binding i. Matrices column-major, arrays element by element,
   integers as their bits; a lane the shader never defined is 0. Before the
   first write each target goes through `buffer::writable` (copy on write
   when a pending kick still reads it, factored out of glBufferSubData), and
   every record through `buffer::write_at` (copied and flushed). Then each
   binding's `written` advances and the active PRIMITIVES_WRITTEN query
   counts the draw's primitives.
7. The interim refusal of tf-api is gone; the draw goes to the GPU as
   usual, or nowhere under discard.

Host tests, `src/gl/tf_capture_tests.rs` (bytes as the GPU sees them, every
one checked flushed):

| design name | test | pins |
|---|---|---|
| C-basic | `points_capture_the_inputs_interleaved` | the inputs exactly, the guard, the draw drawn, 6 primitives |
| C-types, C-array | `separate_buffers_take_every_type` | ivec2, uint, mat2 column-major, `arr[1]` of a loop-written array, a uniform, `gl_VertexID` from `first` |
| C-builtins, C-separate-room | `the_smallest_separate_buffer_decides_and_point_size_is_captured` | `gl_PointSize`; the smaller buffer refuses the draw |
| C-modes | `each_draw_mode_captures_its_decomposed_primitives` | the seven modes' vertex orders, incomplete primitives, the counts |
| C-instanced | `instanced_draws_capture_instance_by_instance` | a divisor-2 array and `gl_InstanceID`, one draw and instance by instance |
| C-pause, C-offset, C-overflow | `pause_offset_and_overflow` | a range at 16 inside guards, pause/resume back to back, a refused draw writes and counts nothing, an exact fit |
| (respecified buffer) | `a_range_is_cut_to_the_buffer_it_names_now` | the range cut to the buffer |
| C-discard | `discard_captures_without_drawing` | captured, not drawn |
| C-hazard | `a_buffer_a_pending_kick_reads_is_copied_first` | fresh storage, the old bytes unchanged |
| C-refuse | `a_vertex_shader_that_samples_is_refused` | the reason logged, nothing written, drawn or counted |
| C-feedback-loop | `a_captured_buffer_feeds_the_next_capture` | a captured buffer as a VBO, a uniform block |
| (ES 1.00) | `an_es100_program_captures_its_varyings` | gl_test 203's pair, `glBindAttribLocation`, a VBO |
| C-perf | `capture_cost` (ignored, by hand) | 1.5 us a vertex, 23 us a draw (x86 host, release) |

`tests/es3_tf_api.rs` `capture_writes_the_bound_buffer` reads one capture
back through `glMapBufferRange`, as dEQP does. `fmath`'s own tests hold its
bounds. C-oracle is dEQP itself: on the host (`tools/cts/host-deqp-run.sh`
with `CTS` a VK-GL-CTS 93bca01 copy carrying the yakogl target,
`DEQP_DISABLE_VK_VIDEO_TESTS=ON`, `GPU=powervr MODULE=gles3`), HOST-ONLY:

| filter | cases | Pass | Fail | note |
|---|---|---|---|---|
| `functional.transform_feedback` | 882 | 882 | 0 | 675 and 207 NotSupported with the separate limit at 4 |
| `functional.lifetime` | 56 | 56 | 0 | |
| `negative_api` / `state_query` / `implementation_limits` with `transform_feedback` in the name | 50 | 50 | 0 | |
| `functional.rasterizer_discard` | 56 | 5 | 51 | render verdicts: meaningless on the host (the AM62P passed the 51 at `c4ccc966`) |
| `functional.buffer.*` with a transform feedback target | 96 | 21 | 75 | render verdicts, as every target's (`buffer.copy.basic`: 56 of 56 fail on the host, pass on the AM62P) |

## 8. The ES 2.0 boundary

`report-es2` (`crate::REPORT_ES2`, `lib.rs:85`) builds answer as they do today,
byte for byte:

- `GL_MAX_TRANSFORM_FEEDBACK_*` stay 0. tf-api writes them as
  `if crate::REPORT_ES2 { 0 } else { 64 / 4 / 64 }`, the same pattern as
  `GL_MAX_DRAW_BUFFERS` (`state.rs:1124-1128`). Whether they should be
  `GL_INVALID_ENUM` instead, as `GL_MAJOR_VERSION` is (`state.rs:1086`), is
  not decided here. Changing them is an ES 2.0 change and out of scope.
- `glBeginTransformFeedback`, `glBindBufferBase/Range(TF, buf ≠ 0)`,
  `glBeginQuery(PRIMITIVES_WRITTEN)` and the SEPARATE `glTransformFeedbackVaryings`
  keep today's refusals. The new code is reached only behind
  `!crate::REPORT_ES2`. An ES 2.0 application never calls these: they are ES
  3.0 entry points that the library exports so a conformance run can resolve
  them (`es3.rs:6-11`).
- `GL_RASTERIZER_DISCARD` stays recorded-only: `glEnable` accepts it and
  `glIsEnabled` answers it as today (`state.rs:436`, `:549`, test
  `es3_caps_are_recorded_and_queryable` at `state.rs:3333`), but the draw and
  clear early returns are `if !crate::REPORT_ES2 && c.rasterizer_discard`.
- The draw path's added cost in every build is one test of "TF active and not
  paused" and one of discard. Neither is reached by an ES 2.0 draw with
  default state. The evaluator's product half (`run_in`), `fmath` and the
  capture are not compiled into a `report-es2` product build (§4.2); a
  `report-es2` host-test build compiles them, unreached.
- `GlContext`'s C layout does not change: all new state lives in Rust-only
  statics (`tf.rs`), as the ES 3.0 pools do today (`es3.rs:33-35`).
  `abi_layout.rs` and `abi_layout_check.c` stay as they are.
  `bound_transform_feedback_buffer` and `rasterizer_discard` are used where
  they are.
- Every tf-* change runs `tools/check.sh` with its CONFIGS, the
  `report-es2` rows and `V3D_ES2_CONFIG` included (`check.sh:56-68`, `:326`).
  The T-es2/C-es2 tests pin the list above.

## 9. The board

gl_test case 203 (`tfcap`, `driver/tools/pvrlab/gl_test.c`) is the
silicon check, the same on the three boards (the capture is the GL core's):
gl_test's ES 1.00 pair with `gl_Position` and `v_color` captured,
interleaved from two triangles (records, guard, PRIMITIVES_WRITTEN 2, the
window drawn), separate from a 4-vertex strip (0 1 2, 2 1 3), and under
rasterizer discard (captured, nothing drawn); read back with
`glMapBufferRange`. Then the AM62P dEQP-GLES3 rows that hold the groups
(`tools/cts/am62p-gles3-tree.txt` line 12, `functional.lifetime` with
`negative_api`, `occlusion_query`, `pbo` and the rest of its image; line 32,
`functional.transform_feedback` with the `ubo` groups of its image;
`TREE_ROWS=4,24` for `tools/board.sh am62p tree`).

