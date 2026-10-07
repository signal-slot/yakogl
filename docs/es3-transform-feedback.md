# Transform feedback and rasterizer discard: design

Status: design (plan #3 item tf-design, 2026-10-08). Two items implement it:
**tf-api** (GL state, errors, queries, rasterizer discard) and **tf-capture**
(the data path). Nothing here is built yet. `src/GL_CORE_NOTES.md`
("Transform feedback (design, not built)") has the short version this
document replaces. When tf-api lands, that section should point here.

## 1. What is missing, and what it costs

Today (`src/gl/es3.rs` at 98ab5fae):

- every `GL_MAX_TRANSFORM_FEEDBACK_*` answers 0 (`src/gl/state.rs:1137-1139`);
- `glTransformFeedbackVaryings` with `GL_SEPARATE_ATTRIBS` and count > 0 is
  `GL_INVALID_VALUE`, an INTERLEAVED request fails the next link
  (`es3.rs:431-456`, `program.rs:1647-1654`);
- `glBegin/End/Pause/ResumeTransformFeedback` are always
  `GL_INVALID_OPERATION` (`es3.rs:390-416`);
- `glBindBufferBase/Range(GL_TRANSFORM_FEEDBACK_BUFFER, i, buf != 0)` is
  `GL_INVALID_OPERATION` (`es3.rs:959-961`, `1014-1016`);
- `glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN)` is
  `GL_INVALID_OPERATION` (`es3.rs:285`);
- `GL_RASTERIZER_DISCARD` is recorded and queryable but changes nothing
  (`state.rs:434-436`).

What that costs on the AM62P (main 396d667a, `tools/cts/RESULTS.md` §1r and the
row at line 991):

| bucket | cases | cause |
|---|---|---|
| `glTransformFeedbackVaryings(): ... GL_INVALID_VALUE at gluShaderProgram.cpp` | 413 | the SEPARATE refusal above |
| `Compile failed: '' at es3fTransformFeedbackTests.cpp` | 48 | an empty info log. This is item **tf-infolog**, not this design. |
| `lifetime.*`, no result | 56 | the group's init needs a working TF object |
| `rasterizer_discard.*` "Fail (no message)" | 51 | discard is not honoured |
| `transform_feedback.*` NotSupported | 421 | dEQP reads the limits (0) and skips |

The mustpass's `transform_feedback` group (`tools/cts/gles3-mustpass.txt`) has
882 cases: `basic_types` and `array` (189 each per buffer mode),
`random_full_array_capture` (30 per mode), `interpolation.{smooth,flat,centroid}`
(18 each), and `position` and `point_size` (6 each: points/lines/triangles ×
separate/interleaved). The draw modes are the three base modes only, so strips
and fans are never captured there. `rasterizer_discard` has 51 cases:
`{basic,scissor,fbo}` × (`write_depth_*` × 7 primitive modes,
`write_stencil_*` × 7, `clear_color`, `clear_depth`, `clear_stencil`).
Nine of every 17 cases are about the **clear**, so discard has to cover
glClear too (§5).

## 2. Citations, and what could not be checked

Lines cited as *verified* were read in this tree. The vendored Mesa subset
(`third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`) and the etna_viv
rnndb headers (`driver/src/vivante/hw/*.xml.h`, the same files Mesa ships as
`src/etnaviv/hw/`) are both here. The full Mesa 26.2.1 tree is **not**:
`$CCFARM_DEPS_DIR/mesa-26.2.1-src` exists but is empty, and this worker had no
network access to fetch it. Citations into files outside the vendored subset
therefore give the file and the function, with `:L?` in place of a line.
**tf-capture's first step is to fill those lines in** from a Mesa 26.2.1
checkout, per `third_party/README.md`, and to correct anything below that the
source contradicts. The ES 3.0 section numbers are those of ES 3.0.6. Where a
rule is from memory and has not been checked against the PDF, it says
"(verify)".

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
  `TRIANGLE_FAN_TF` = 22 (`:63-69`). As far as is known (verify in
  `v3dx_draw.c`), Mesa uses those only for V3D < 4.0 and enables capture on
  4.x through the specs packet.

The capture is done by the **coordinate shader** in the binning pass. The data
specs index the coordinate shader's VPM output, so the TF varyings must be
written to the coordinate shader's VPM segment after the position header. Mesa,
not verified here:

- `src/gallium/drivers/v3d/v3d_program.c:L?`
  `v3d_set_transform_feedback_outputs()` builds `so->tf_specs[]` (and
  `tf_specs_psiz[]`) from `pipe_stream_output_info`, and puts the TF outputs
  into the coordinate shader key (`used_outputs`) so the compiler writes them
  to the VPM.
- `src/gallium/drivers/v3d/v3dx_emit.c:L?` (`v3dX(emit_state)`, the
  `V3D_DIRTY_STREAMOUT` block) emits `TRANSFORM_FEEDBACK_SPECS` with the specs
  and one `TRANSFORM_FEEDBACK_BUFFER` per bound target.
- `src/gallium/drivers/v3d/v3dx_draw.c:L?`: `v3d_update_primitive_counters()`
  (the primitive counts are worked out on the CPU from the vertex count and
  the room left in each buffer), and the job flush for a buffer that TF wrote
  and a later draw reads.
- `src/broadcom/compiler/v3d_nir_lower_io.c:L?` and `nir_to_vir.c:L?`: the
  coordinate shader's VPM output with the TF outputs (`num_used_outputs`).

Our V3D compiler produces the coordinate shader itself
(`src/shaderc/online/v3d_abi.rs`, "the linked program's three QPU programs"),
so a hardware path would mean: TF outputs appended to the CS's VPM segment, the
specs and buffer packets in `src/v3d/backend.rs`, and the counters read back
through `Primitive Counts Feedback` or computed on the CPU as Mesa does.

### 3.2 GC7000UL (i.MX8MP): registers exist, but no driver evidence

Verified in the rnndb headers:

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

What could not be established here:

- Whether r6204 has `HWTFB`. Our identify reads `minor_features[0..5]`
  only, because MINOR_FEATURE_6 at 0xDC has no rnndb name
  (`driver/docs/vivante-kernel-rust.md:146-149`). galcore's
  `gc_feature_database.h` (`REG_HWTFB` in the gc7000UL entries the repo
  already cites, `src/backend.rs:238-240`) would answer it.
- Whether Mesa etnaviv drives the block at all. As far as is known it does
  not expose stream output: `src/gallium/drivers/etnaviv/etnaviv_screen.c:L?`
  should set no `max_stream_output_buffers`, and no `VIVS_TFB_*` should be
  written anywhere in `src/gallium/drivers/etnaviv/` (verify both with grep).
  If that holds, nobody has a public encoding for the descriptors, the
  context buffer or the query buffer beyond the field names.

So the GC7000 has a probable hardware path with no reference implementation
behind it.

### 3.3 PowerVR BXS-4-64 (AM62P): no capture path

Mesa's PowerVR driver is Vulkan only (`src/imagination/vulkan/`) and does not
implement `VK_EXT_transform_feedback`. Verify that it is absent from
`pvr_physical_device.c:L?` `pvr_get_device_extensions`. The vertex USC program
writes its outputs into the vertex output buffer that the TA consumes, and from
there they go to the parameter buffer. No stream-out unit is documented. The
remaining option is a vertex-shader variant that stores its outputs to memory
through USC store instructions. Our online USC back end has no memory stores
(`src/shaderc/online/powervr/`), and Mesa's pco would be the reference only
where it lowers SSBO stores in a vertex stage, which ES 3.0 does not need. This
is §6, rejected alternative R3.

## 4. Decision

| GPU | capture | rasterizer discard | PRIMITIVES_WRITTEN |
|---|---|---|---|
| all three, first | **CPU capture in the GL core** (§4.1) | GL core: skip the draw and the clear | counted by the CPU capture |
| V3D 4.2, later, optional | hardware TF through the coordinate shader (§3.1), behind the same GL interface | unchanged | from `Primitive Counts Feedback` or the CPU count |
| GC7000UL | CPU only until the HWTFB bit and an encoding are shown | unchanged | CPU |
| PowerVR | CPU only | unchanged | CPU |

### 4.1 Why CPU capture first, on every GPU

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

- **Speed.** An interpreted IR costs microseconds per vertex on an A53/A72.
  That is fine for dEQP (draws of tens to hundreds of vertices) and slow for a
  particle system of 10^5 vertices per frame. Capture runs only while TF is
  active and not paused, so no other draw pays anything but one flag test. The
  V3D hardware path (§4.3) is the remedy where it matters.
- **Precision.** The captured values are the evaluator's f32, not the GPU's.
  The GPU still rasterises with its own values, so a captured `gl_Position`
  and the drawn one can differ in the last ulp. ES 3.0 does not require them
  to be bit-identical. dEQP compares the buffer with its reference, not with
  the image.
- **Vertex textures.** The evaluator samples only `texref` model textures
  (`eval.rs:131`). Reading real GL texture storage (tiled per back end) is not
  in scope. A captured draw whose vertex shader samples a texture is refused,
  see §5.7. dEQP's TF shaders do not sample.

### 4.2 What the evaluator has to become (tf-capture)

`src/shaderc/online/eval.rs` is `#[cfg(any(test, feature = "host-test"))]`
(`online/mod.rs:99-100`), uses `std::vec::Vec` (`eval.rs:23`) and std float
methods, and refuses textures, derivatives and indexed locals
(`eval.rs:13-20`). For product use:

- **no_std.** Replace `Vec` with slices taken from the compile scratch arena
  (`store::with_scratch`, `store.rs:580`, `SCRATCH_BYTES` 4 MiB at `:87`). The
  value and local counts are known from the `ir::Module`. Build it in every
  build except `report-es2`: `#[cfg(any(test, feature = "host-test",
  not(feature = "report-es2")))]`. The test-only users keep the same API.
- **Transcendentals.** The crate has no libm and no dependencies
  (`Cargo.toml:159`, `fold.rs:11-13`). Add a small no_std math module (sin,
  cos, tan, asin, acos, atan, atan2, exp, exp2, log, log2, pow, sqrt,
  inversesqrt) with an error bound stated against GLSL ES 3.00 §4.5.1 (verify
  the table of required precisions), and keep the host evaluator on std so
  the existing oracles do not move. dEQP's TF shaders use none of these, so
  this is for real applications.
- **Indexed locals** (`out float v[N]` written in a loop with a variable
  index): `random_full_array_capture` and `array` need it. The arena gives
  each local array its storage.
- **Integer and unsigned outputs** are raw 32-bit lanes already (`eval.rs:13`).
  The capture writes the lanes, not floats.
- A `fault` (`Err`) is a refusal (§5.7), never a value.

The vertex shader's **IR is not kept** after compile (only the GPU binary and
reflection live in the keep region, `store.rs:124`). Keeping it for every
program would cost keep-region space that ES 2.0 programs need. Instead:

- at `glLinkProgram`, when the program has ≥ 1 TF varying, copy the linked
  vertex shader's **source** (and its `#version` text, as compiled) into a
  host-heap buffer owned by the program. Free it on relink and on delete. A
  later `glShaderSource` on the shader does not change the linked program
  (ES 3.0 2.11.3), which is why the copy is taken at link;
- at each captured draw, re-run `lower::compile_to_ir` (`lower.rs:563`) into
  scratch and evaluate there. The compile is milliseconds per draw. tf-capture
  measures it on the host (§7.2, C-perf) and may cache the module for the
  duration of one Begin/End if the scratch can be held, but not across ES 2.0
  draws.

The online front end is in every build (`online/mod.rs`: `lower`, `parser`,
`sema` are unconditional), so this works for a program whose GPU binary came
from the offline table too.

### 4.3 V3D hardware path (later, optional)

Only when an application needs the speed, and only after the CPU path passes
the dEQP TF group on the Pi 4 (it is the oracle for the hardware path).
Interface: the back end gets `GlesDraw.tf: Option<&GlesTfState>` (buffer
addresses and sizes, specs, current offsets). It returns the primitives
written, or the GL core keeps counting on the CPU as Mesa does. Constraint: a
spec covers at most 16 consecutive 32-bit values and there are 4 buffers
(§3.1), which covers ES 3.0's 64 interleaved and 4 × 4 separate. Pause/resume
needs `Primitive Counts Feedback` op 2/3 then 8, store and reload of the buffer
state (`v3d_packet.xml:711-720`). Not part of tf-capture.

## 5. GL-layer specification (tf-api)

All of this is ES 3.0 state. §8 says what an ES 2.0 build keeps.

### 5.1 Limits

| pname | value | ES 3.0 minimum (table 6.34, verify) |
|---|---|---|
| `GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS` | 64 | 64 |
| `GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS` | 4 | 4 |
| `GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS` | 4 | 4 |

These are constants in a new `src/gl/tf.rs` (`TF_MAX_INTERLEAVED_COMPONENTS`,
`TF_MAX_SEPARATE_ATTRIBS`, `TF_MAX_SEPARATE_COMPONENTS`). The minimums are
enough because the CPU path has no hardware ceiling, and they match V3D's 4
buffers. **Ordering:** tf-api raises the limits in the same change that makes
Begin work. Until tf-capture lands, a draw with TF active and not paused is
`GL_INVALID_OPERATION` with the log line "transform feedback capture is not
implemented yet (tf-capture)". It must never succeed without capturing.

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
  `ctx->TransformFeedback.CurrentBuffer`, outside the object (verify in
  `src/mesa/main/transformfeedback.c:L?`). The **indexed** bindings, START
  and SIZE are object state.
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
  active one is `GL_INVALID_OPERATION` and deletes **nothing** in that call
  (ES 3.0 2.15.1, verify whether the call stops at the first or deletes the
  others; dEQP `negative_api.transform_feedback.delete_transform_feedbacks`
  and `lifetime.delete_active.transform_feedback` are the referee).
  Releasing an object drops its buffer refs.
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
4. Anything else is a link error: "transform feedback varying 'X' is not an
   output of the vertex shader". Matching is case-sensitive. A struct member
   (`s.m`) is refused with its own message unless ES 3.0 2.11.8 allows it
   (verify. dEQP's mustpass captures no struct members, as far as its
   names show).
5. The same name twice (or an element and its whole array) is a link error
   (verify the exact rule. Mesa: "specified multiple times",
   `src/compiler/glsl/link_varyings.cpp:L?`, `tfeedback_decl::store`).
6. Limits: interleaved, the total components > 64, is a link error that names
   `GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS`. Separate, one varying's
   components > 4 (a `mat2` or larger, a `vec2[3]`), is a link error that
   names `..._SEPARATE_COMPONENTS`. The count > 4 for separate is caught
   earlier, at the call (§5.6).
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
  `GL_INVALID_OPERATION` (ES 3.0 2.15.2, verify; dEQP
  `negative_api.vertex_array.draw_elements*` expects it in an ES 3.0
  context).

While paused, or with no object active, every draw is as today.

### 5.6 Errors (ES 3.0 2.11.8, 2.15, 2.16, 6.1. Each is a tf-api host test)

The order within a call follows dEQP's `es3fNegative*ApiTests.cpp` sections.
The checks that come first today keep their place.

`glTransformFeedbackVaryings(program, count, varyings, mode)`:
not a program → `not_a_program` (as today). count < 0 → `INVALID_VALUE`.
mode not INTERLEAVED/SEPARATE → `INVALID_ENUM`. SEPARATE and count > 4 →
`INVALID_VALUE`. Count > 0 with `varyings` NULL: record nothing and
`INVALID_VALUE` (no rule in the spec. This is the repo's NULL convention,
as in glDrawBuffers).

`glBindTransformFeedback(target, id)`: target ≠ `GL_TRANSFORM_FEEDBACK` →
`INVALID_ENUM`. The bound object active and not paused → `INVALID_OPERATION`.
id not generated → `INVALID_OPERATION`.

`glDeleteTransformFeedbacks(n, ids)`: n < 0 → `INVALID_VALUE`. Any id active →
`INVALID_OPERATION` (§5.2).

`glBeginTransformFeedback(mode)`: mode ∉ {POINTS, LINES, TRIANGLES} →
`INVALID_ENUM`. Already active → `INVALID_OPERATION`. No current program, or
the program has 0 linked TF varyings → `INVALID_OPERATION`. A binding the
program needs has no buffer (interleaved: binding 0; separate: bindings
0..count-1) → `INVALID_OPERATION`.

`glEndTransformFeedback()`: not active → `INVALID_OPERATION`.
`glPauseTransformFeedback()`: not active, or already paused →
`INVALID_OPERATION`.
`glResumeTransformFeedback()`: not active, or not paused →
`INVALID_OPERATION`. Current program ≠ `program` at Begin →
`INVALID_OPERATION` (verify that this is in ES 3.0 and not only 3.1+).

`glUseProgram(p)`: TF active and not paused → `INVALID_OPERATION`.
`glLinkProgram(p)`: p in use by an active TF object (the current program,
active, paused or not) → `INVALID_OPERATION` (verify "paused or not").
`glProgramBinary`: the same rule as link.

`glBindBufferBase/Range(GL_TRANSFORM_FEEDBACK_BUFFER, i, ...)`: i ≥ 4 →
`INVALID_VALUE`. Range: size ≤ 0 → `INVALID_VALUE`, offset or size not a
multiple of 4 → `INVALID_VALUE`. The bound object active (paused or not) →
`INVALID_OPERATION`. Today's `offset < 0 || size <= 0` check (`es3.rs:1001`)
stays first.

Draws while active and not paused: mode not compatible with `prim_mode`
(POINTS ← POINTS; LINES ← LINES, LINE_LOOP, LINE_STRIP; TRIANGLES ←
TRIANGLES, TRIANGLE_STRIP, TRIANGLE_FAN) → `INVALID_OPERATION`. An indexed
draw → `INVALID_OPERATION`. Not enough room: for some binding,
`written + vertices × stride > avail` → `INVALID_OPERATION`, nothing captured
and nothing drawn. Here vertices = primitives(mode, count) × verts-per-prim ×
instances, and avail = the range size (Range), or buffer size − offset (Base,
read at draw time). Mode compatibility is checked after today's draw
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
a texture, or any evaluator `Err`, makes the draw `GL_INVALID_OPERATION`. Log
line: "transform feedback capture: <evaluator reason>". Nothing is drawn,
captured or counted. ES 3.0 has no such error, so it is a known deviation,
listed in `GL_CORE_NOTES.md`. It is in the repo's practice of failing loudly
(`es3.rs:25-31`) rather than capturing zeros.

### 5.8 Rasterizer discard (ES 3.0 3.1, and 4.2.3 for clears)

With `GL_RASTERIZER_DISCARD` enabled:

- **Draws**: all argument and TF validation and the TF capture happen. Then
  the draw stops in the GL core, before `submit_draw` (`draw.rs:400`). No
  back end sees it, so it writes no colour, depth or stencil, and an occlusion
  query counts no samples. This is the same for all five draw entry points.
- **Clears**: `glClear` and `glClearBuffer{fv,iv,uiv,fi}` validate their
  arguments, then do nothing (no pending clear is recorded, `draw.rs:46-140`).
  This comes from dEQP's `rasterizer_discard.*.clear_{color,depth,stencil}`
  (9 of the 51). Mesa returns early on `ctx->RasterDiscard` in
  `src/mesa/main/clear.c:L?` (`clear()` and the `ClearBuffer*` paths, verify).
- **Not affected**: `glReadPixels`, `glBlitFramebuffer`, `glCopyTex*`,
  `glGenerateMipmap`, and uploads (verify Blit against ES 3.0 4.3.3. If ES
  3.0 is silent, leave it unaffected as GL does).
- The GC7000's `TFB_CONFIG.RASTERIZER_DISCARD` bit (§3.2) is **not** used.
  The GL core drops the work before the back end, the same on all three.

This part needs no capture, so tf-api ships it and the 51 cases can pass
before tf-capture.

## 6. Rejected alternatives

- **R1: Hardware TF on all three GPUs first.** The PowerVR has none (§3.3).
  The GC7000's needs an undocumented encoding and an unread feature bit
  (§3.2). Only the V3D's is documented (§3.1). Three paths, two of them
  unverifiable, against one host-testable path: rejected for the first
  version. The V3D path is kept as §4.3.
- **R2: GPU vertex pass that writes to a scratch buffer by `gl_VertexID`,
  then CPU primitive assembly.** It would avoid the evaluator's precision and
  speed costs, but needs a memory-store vertex shader on each GPU: USC
  stores (none in our back end), QPU TMU writes (V3D 4.2 has them, but our
  compiler emits none for a VS), and GC7000 stores. That is three new code
  generator features to save CPU time that dEQP does not need. Rejected for
  now. It is the natural successor if a PowerVR or GC7000 application needs
  fast TF.
- **R3: PowerVR vertex-shader variant that stores to the TF buffer directly**
  (address = f(vertex index, primitive unrolling)). Strips and fans write one
  vertex into several primitives, so the shader would need the unrolled
  index map. It inherits all of R2's cost plus that. Rejected.
- **R4: Keep the IR of every program alive for capture.** It costs
  keep-region space (`KEEP_BYTES` 512 KiB, `store.rs:124`) for every program,
  ES 2.0 ones included, to save a re-lower on the rare TF draw. Rejected in
  favour of the per-program source copy (§4.2), which costs only programs with
  TF varyings.
- **R5: Answer the queries and accept Begin without capturing** ("plausible
  zero"). It would turn the 413 + 421 cases into wrong-buffer failures, hide
  the gap, and contradict `es3.rs:25-31`. Rejected. §5.1's interim
  `INVALID_OPERATION` at the draw is the honest version.
- **R6: Expose TF only in builds for the V3D.** dEQP's TF and lifetime groups
  would then stay failed on two of the three boards, for no saving: the CPU
  path is GPU-independent. Rejected.
- **R7: Report discard to the back ends** (cull everything, the GC7000's
  `TFB_CONFIG` bit). The draw would still pay the back end's setup, and three
  back ends would change where one GL-core early return does the same.
  Rejected.

## 7. Items and host tests

Test modules are new files, so plan #2's files are not touched: `src/gl/tf.rs`
holds the TF object state, the varying resolution and the capture, and
`src/gl/tf_tests.rs` its unit tests on `testutil::TestGl` (`testutil.rs:1484`:
it records draws, cache ops and pending kicks, and reads buffer bytes). Edits
to `es3.rs`, `state.rs`, `program.rs`, `draw.rs` and `buffer.rs` are the hooks
only.

### 7.1 tf-api (GL core: state, errors, queries, discard)

Work:

1. `TfObject` pool (§5.2), `MAX_FEEDBACKS` 32, the default object, bind,
   delete and is.
2. Indexed TF bindings in `glBindBufferBase/Range`, the indexed and plain
   queries, and the buffer-delete orphan rule.
3. Varying recording, link resolution, link errors, `glGetProgramiv` and
   `glGetTransformFeedbackVarying` (§5.3).
4. Begin/End/Pause/Resume and every error in §5.6, plus the
   UseProgram/LinkProgram/BindBuffer interlocks.
5. The query object model and PRIMITIVES_WRITTEN (counting stub: 0 until
   tf-capture, which is right because draws are refused until then).
6. The limits 64/4/4, with the interim draw refusal (§5.1).
7. Rasterizer discard in the draw and the clears (§5.8).

Host tests (each a `#[test]` in `src/gl/tf_tests.rs`, all
`#[cfg(not(feature = "report-es2"))]` except T-es2):

- T-limits: the three limits answer 64/4/4.
- T-object: gen/bind/is/delete. Bind of a non-generated name →
  INVALID_OPERATION. Deleting the bound object rebinds 0. Deleting an active
  one → INVALID_OPERATION, and it survives.
- T-bind: Base/Range on indices 0..3, the queries
  BINDING/START/SIZE per index and per object (bind two objects, check
  isolation), i = 4 → INVALID_VALUE, offset 2 → INVALID_VALUE, bind while
  active (and while paused) → INVALID_OPERATION.
- T-orphan: buffer bound to a non-current object, deleted. `glIsBuffer`
  false, the object's binding still answers the old name, and the storage is
  freed only when the object lets go (the mock's `live_allocs`).
- T-varyings: one table-driven test per §5.3 resolution rule (whole, element,
  `gl_Position`, `gl_PointSize`, unknown, duplicate, 65 interleaved
  components, `mat2` separate) on the link status and the info log text.
  `glGetTransformFeedbackVarying` rows and the three `glGetProgramiv` values.
  A relink with new varyings replaces them, a failed relink keeps none
  (verify ES 3.0 2.11.3's rule for a failed link).
- T-begin: every Begin/End/Pause/Resume error of §5.6 in dEQP's order.
  ACTIVE/PAUSED read back after each step.
- T-interlock: UseProgram while active (error) and while paused (allowed).
  LinkProgram of the current program while active. BindTransformFeedback
  while active.
- T-draw-guard: while active, DrawElements → INVALID_OPERATION and DrawArrays
  with an incompatible mode → INVALID_OPERATION. Before tf-capture: a
  compatible DrawArrays → INVALID_OPERATION with the interim log line, and no
  draw recorded.
- T-query: PRIMITIVES_WRITTEN Begin/End, `GL_CURRENT_QUERY`, result
  available at once. Two Begins → INVALID_OPERATION. 0x8C87 →
  INVALID_ENUM. ANY_SAMPLES_PASSED still refused (occlusion-query's).
- T-discard-draw: with discard on, each of the 5 draw calls records no
  backend draw (`gl.draws()` empty) and sets no error. Its argument errors
  still fire. With discard off, the draw is recorded.
- T-discard-clear: `glClear` (each bit) and `glClearBuffer*` under discard
  leave no pending clear and change no pixel of the default surface. Without
  discard they do.
- T-es2 (only under `report-es2`): §8's list.

### 7.2 tf-capture (the data path)

Work:

1. The evaluator in product builds (§4.2: no_std, arena, indexed locals,
   math), behind `not(report-es2)`.
2. The VS source copy at link (§4.2).
3. Vertex inputs per vertex: attribute fetch and conversion through
   `draw.rs`'s existing helpers (`attrib_row`, `convert_attrib`,
   `draw.rs:854-968`), with `cache_invd` before a CPU read and the OSAL
   bounce for a buffer with no CPU mapping. Client arrays on the default VAO.
   Generic attribute defaults for disabled arrays. `gl_VertexID` = first + i
   and `gl_InstanceID`, with divisors.
4. Uniforms: the same dwords the draw uploads for the vertex stage, flattened
   uniform blocks included (the draw path's builder, called once per captured
   draw).
5. Primitive assembly in the draw mode, unrolled into `prim_mode`
   primitives. Proposed vertex orders: LINE_STRIP (i, i+1), LINE_LOOP adds
   (n-1, 0), TRIANGLE_STRIP even (i, i+1, i+2) and odd (i+1, i, i+2),
   TRIANGLE_FAN (0, i+1, i+2). **Verify each against dEQP's reference in
   `modules/gles3/functional/es3fTransformFeedbackTests.cpp`** (the cts-host
   checkout, VK-GL-CTS 93bca01) and against ES 3.0 2.15.2 before the tests
   pin them. The mustpass draws base modes only (§1), so the strip and fan
   orders are pinned by our host tests, not by dEQP. Incomplete primitives
   are dropped. Instances loop outermost.
6. Write-out: interleaved, every varying of a vertex in declaration order,
   tightly packed 4-byte components at `offset + written[0]`. Separate,
   varying i to binding i. Matrices column-major, arrays element by element.
   Integers as raw bits. Writes go through a helper factored out of
   `glBufferSubData` (`buffer.rs:405-440`) so a pending kick still reading the
   buffer gets copy-on-write and the written range is cache-flushed. Then
   advance `written`, and add the primitive count to the object and the
   active PRIMITIVES_WRITTEN query.
7. Remove §5.1's interim refusal. Then the GPU draw as usual, or none under
   discard.

Host tests (`src/gl/tf_tests.rs`, plus `tests/es3_tf_capture.rs` for
shader-level cases):

- C-basic: a pass-through VS (`out vec4 v = a;`), 6 vertices, POINTS,
  interleaved. The buffer bytes equal the inputs exactly, and
  PRIMITIVES_WRITTEN = 6.
- C-types: every ES 3.00 output type dEQP's `basic_types` uses (float/vec,
  int/ivec, uint/uvec, mat2..mat4, mat2x3.., and lowp/mediump/highp), both
  modes. Expected bytes are computed in the test from the inputs.
- C-array: whole arrays, single elements (`v[1]`), and a loop-indexed
  output array (the indexed-locals case).
- C-builtins: `gl_Position` and `gl_PointSize` captured, interleaved with a
  user varying.
- C-modes: each of the 7 draw modes into its compatible begin mode, small
  counts including incomplete primitives. The vertex order of item 5 is
  pinned by the expected bytes.
- C-instanced: `glDrawArraysInstanced` with a divisor-1 attribute and
  `gl_InstanceID`.
- C-pause: draw, pause, draw (not captured, still drawn), resume, draw. The
  buffer holds the first and third draws back to back, and the query counts
  only those.
- C-offset: `glBindBufferRange` at offset 16, plus a guard pattern before and
  after the range, untouched (dEQP checks guards).
- C-overflow: a draw that does not fit → INVALID_OPERATION, the buffer is
  untouched, no draw is recorded, and the query is unchanged. The draw that
  exactly fits succeeds.
- C-separate-room: separate mode with bindings of different sizes. The
  smallest decides the overflow.
- C-discard: capture with discard on writes the buffer and records no draw.
- C-hazard: the TF buffer is still read by a pending kick
  (`set_kick_pending`). The capture goes to fresh storage, the pending kick's
  bytes are unchanged, and the cache flush covers the written range
  (`cache_ops`).
- C-refuse: a VS that samples a texture → INVALID_OPERATION with the
  evaluator's reason in the log. Nothing written or drawn.
- C-feedback-loop: capture into buffer A, then draw from A as a vertex
  buffer. The recorded draw's vertex data are the captured values.
- C-oracle: for the dEQP TF shader texts in the repo's corpus (when
  es3-shaderc-coverage has them; otherwise the shaders of item 5's
  dEQP source), the captured values against an independent std-f32
  computation in the test.
- C-perf (`#[ignore]`, run by hand): 10^4 vertices of a 20-ALU VS through
  capture. Print µs/vertex and the re-lower time per draw for the record.
- C-es2: the report-es2 run of the whole file compiles out the capture, and
  T-es2 still holds.

dEQP on the host: `tools/cts/host-deqp-run.sh MODULE=gles3` (items cts-host
and host-gles3-run) over `dEQP-GLES3.functional.transform_feedback.*`,
`rasterizer_discard.*`, `lifetime.*`, `negative_api.transform_feedback*` and
`state_query.*transform_feedback*`. Expected outcome after tf-capture: the
48 empty-log compile failures are tf-infolog's. Everything else in the 882 +
51 + 56 should run. The run's failures are the input for a follow-up.

## 8. The ES 2.0 boundary

`report-es2` (`crate::REPORT_ES2`, `lib.rs:85`) builds answer as they do today,
byte for byte:

- `GL_MAX_TRANSFORM_FEEDBACK_*` stay 0. tf-api writes them as
  `if crate::REPORT_ES2 { 0 } else { 64 / 4 / 4 }`, the same pattern as
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
  default state. The evaluator, the math module and the source copy are not
  compiled into `report-es2` builds (§4.2).
- `GlContext`'s C layout does not change: all new state lives in Rust-only
  statics (`tf.rs`), as the ES 3.0 pools do today (`es3.rs:33-35`).
  `abi_layout.rs` and `abi_layout_check.c` stay as they are.
  `bound_transform_feedback_buffer` and `rasterizer_discard` are used where
  they are.
- Every tf-* change runs `tools/check.sh` with its CONFIGS, the
  `report-es2` rows and `V3D_ES2_CONFIG` included (`check.sh:56-68`, `:326`).
  The T-es2/C-es2 tests pin the list above.
