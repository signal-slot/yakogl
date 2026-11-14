<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 program binaries (G7)

Design only; no code yet. G7 of `docs/notes/v3d-notpass-rest.md`:
`glGetProgramBinary` answers `GL_INVALID_OPERATION` and
`GL_NUM_PROGRAM_BINARY_FORMATS` is 0 (`src/gl/es3.rs` `glGetProgramBinary`,
`glProgramBinary`; `src/gl/state.rs` `get_integerv`). That is a legal ES 3.0
answer, and dEQP reports it as NotSupported. The rows:
`shader_api.program_binary.*` are 14 NotSupported on every board
(`docs/es3/pi4-gles3-fails.tsv`, `docs/es3/am62p-gles3-notpass.tsv`), and
`state_query.shader.program_binary` is 1 Fail on both boards. That Fail
predates 6f7d7772 (see (a)).

Sources:

- The OpenGL ES 3.0.6 specification (November 1, 2019): §2.12.1 (object
  names), §2.12.3 (`UseProgram`, re-linking a program in use), §2.12.4
  (Program Binaries, pp. 53-55), §2.15.2 (transform feedback errors),
  §6.1.12 (`GetProgramiv`), Table 6.18 (Program Object State) and Table 6.29
  (Implementation Dependent Values).
- Mesa 26.2.1, read only: `src/mesa/main/program_binary.c` and
  `src/mesa/main/shaderapi.c`. Our `third_party/mesa-26.2.1` does not carry
  `src/mesa/main`, so these were read from the release tarball named in its
  `UPSTREAM`. Facts are cited below; no code is copied.
- The dEQP sources at VK-GL-CTS 93bca01: `es3fShaderApiTests.cpp`
  (`ProgramBinaryCase` and its subclasses), `es3fShaderStateQueryTests.cpp`
  (`ProgramBinaryCase`) and `es3fNegativeShaderApiTests.cpp`
  (`get_program_binary`, `program_binary`, `program_parameteri`).

## (a) What dEQP checks, and the answers we owe

### `state_query.shader.program_binary` (1 case)

`es3fShaderStateQueryTests.cpp`, `ProgramBinaryCase::test()`:

1. Compile the common vertex and fragment sources, create a program, attach
   both shaders and link. Expect no GL error.
2. `glGetProgramiv(GL_PROGRAM_BINARY_RETRIEVABLE_HINT)` must be `GL_FALSE`
   (the initial value in Table 6.18).
3. `glProgramParameteri(program, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE)`,
   link again, and read the hint back: it must be `GL_TRUE`.
4. `glGetProgramiv(GL_PROGRAM_BINARY_LENGTH)` into a write guard. Only
   *validity* is checked: the query must write the value and raise no error.
   The value itself is not compared, and the case reads neither
   `GL_NUM_PROGRAM_BINARY_FORMATS` nor `GL_PROGRAM_BINARY_FORMATS`.
5. Delete everything. Expect no GL error.

Both board rows predate the fix. The AM62P list is from build ad2b5ed
(2026-10-13) with the message "got invalid value", and the Pi 4 image SQ ran
at 0ff58329 (2026-11-01). On 2026-11-07, commit 6f7d7772 made
`glProgramParameteri` store the hint and `glGetProgramiv` return it
(`tests/es3_program_binary_hint.rs`). Steps 2-4 therefore have their answers
at current main, so this row should already pass on the host. G7 must keep
it passing: once a binary exists, `GL_PROGRAM_BINARY_LENGTH` of a linked
program becomes non-zero, and the case does not care about the value.

**The hint (decision).** §2.12.4 says the hint "will not be in effect until
the next time LinkProgram or ProgramBinary has been called successfully".
§6.1.12 says `GetProgramiv` returns the properties "currently in effect".
Mesa (`shaderapi.c`, `get_programiv`, `GL_PROGRAM_BINARY_RETRIEVABLE_HINT`) returns the value as set, at once, and
so do we today. dEQP links before it reads, so it passes under either
reading. We keep today's behaviour, and the binary does not carry the hint:
every linked program is retrievable whatever the hint says, so it changes
nothing.

### `shader_api.program_binary.*` (14 cases)

All 14 derive from `ProgramBinaryCase`. `init()` reads
`GL_NUM_PROGRAM_BINARY_FORMATS`, and only if that is > 0 reads
`GL_PROGRAM_BINARY_FORMATS`. `iterate()` returns **NotSupported when the list
is empty**, and that is today's 14. The helpers:

- `getProgramBinary`: `glGetProgramiv(GL_PROGRAM_BINARY_LENGTH)`. If it is
  > 0, call `glGetProgramBinary(program, length, &actual, &format, data)`
  under `GLU_CHECK_CALL` (no error allowed), then
  `TCU_CHECK(actual == length)`. A length of 0 leaves the binary empty, and
  then the format check below fails.
- `loadProgramBinary`: `glProgramBinary(program, format, data, size)` under
  `GLU_CHECK_MSG` (no error allowed). The link status is not read.
- `verifyProgramBinary`: the returned format must be one of the formats from
  `GL_PROGRAM_BINARY_FORMATS`, otherwise Fail "Invalid format".

The cases:

| Case | What it does and checks |
|---|---|
| `simple.get_program_binary_vertex_fragment` | Link `gl_Position = vec4(0.0)` / `o_fragColor = vec4(0.0)` (`#version 300 es`, `layout(location = 0) out`). Get the binary and verify its format. Nothing is loaded and nothing is drawn. |
| `simple.uniform_reset_on_binary_load` | Both stages declare `bool u_boolVar`, `highp int u_intVar`, `highp float u_floatVar`, and the vertex shader also `in highp vec4 a_position`. The link must succeed (`TCU_CHECK_MSG`). Get and verify the binary. `glUseProgram` the program, then set `u_boolVar = GL_TRUE`, `u_intVar` to a random value in [1,1000] and `u_floatVar` to a random value in [1,1000]. Draw a quad with `a_position` and expect no GL error (pixels are not read). Load the saved binary **into the same program, which is current**, with no error. Then `glGetUniformiv` / `glGetUniformfv` at the locations taken before the load must all read 0. |
| `binary_persistence.{detach,reattach,delete,replace,recompile}_shader_{vertex,fragment}`, `binary_persistence.replace_source_{vertex,fragment}` (12) | Link a pair whose outputs are random constants, then save the binary. Then, **without relinking**, change one stage: detach it; detach and attach it again; detach and delete it; replace it with a new uncompiled shader; compile it again; or set a new random source and compile. Get the binary again: format and every byte must be equal ("Program binary changed"). The binary is never loaded in these cases. |

No case uses transform feedback, and no case reads pixels. We still carry
transform feedback state in the binary, because §2.12.4 requires it (see
(b)).

The negative cases are already in the mustpass and pass today; they must
still pass with formats present:

- `negative_api.shader.get_program_binary` (linked `program`, unlinked
  `programInvalid`). `bufSize = GL_PROGRAM_BINARY_LENGTH`.
  `glGetProgramBinary(program, 0, ..)` must give `GL_INVALID_OPERATION`, and,
  if `bufSize > 0`, so must `(program, bufSize - 1, ..)`.
  `glGetProgramBinary(programInvalid, <its length, 0>, ..)` must give
  `GL_INVALID_OPERATION`.
- `negative_api.shader.program_binary`. Take the linked source program's
  binary (no error), then `glProgramBinary(<a shader name>, format, buf, len)`
  must give `GL_INVALID_OPERATION`, and `glProgramBinary(<unlinked program>,
  -1, buf, len)` must give `GL_INVALID_ENUM`. Today `bufSize` is 0 there, so
  `len` is still dEQP's initial -1. With a real format, `len` is the real
  length.
- `negative_api.shader.program_parameteri`. `glProgramParameteri(0, ..)`
  must give `GL_INVALID_VALUE`, an unknown pname `GL_INVALID_ENUM`, and
  value 2 `GL_INVALID_VALUE`. Today `es3.rs` `program_exists` answers
  `GL_INVALID_OPERATION` for every name that is not a program, name 0
  included. This case failed on the Pi 4 at 0ff58329
  (`tools/cts/log/2026-10-31-pi4-gles3-a/M1.console.txt:866`). §2.12.1 makes
  it `GL_INVALID_VALUE` for a name that is neither object, and
  `GL_INVALID_OPERATION` only for a shader name. See (d).

## (b) The format

### One format, one token

`GL_NUM_PROGRAM_BINARY_FORMATS` is 1, and `GL_PROGRAM_BINARY_FORMATS` is the
one value `GL_PROGRAM_BINARY_FORMAT_YAKOGL`. Table 6.29 allows a minimum of
0, and "OpenGL ES defines no specific binary formats" (§2.12.4). The token is
a `pub const` in `src/gl/program_binary.rs`, not in the generated
`consts.rs`.

Its value is **0x6B47**, inside [0x6000, 0x7FFF]. The Khronos registry
(`registry.khronos.org/OpenGL/docs/enums.html`) reserves that range for
temporary use, and it never assigns a value from it to a shipping enumerant,
so the token cannot collide with a registered one. Before a release, a
vendor block must be requested from the registrar and the constant moved
(see the handoff). Moving it costs one line. An old binary then gets
`GL_INVALID_ENUM`, which an application already handles because §2.12.4
lets any load fail. Mesa's `GL_PROGRAM_BINARY_FORMAT_MESA` (0x875F) is not
reused: it names Mesa's format.

### Header

All fields are little-endian at fixed offsets: 80 bytes, then the payload.

| Offset | Bytes | Field |
|---|---|---|
| 0 | 4 | magic `YKPB` |
| 4 | 2 | format version, 1 |
| 6 | 2 | header bytes, 80 |
| 8 | 4 | payload bytes |
| 12 | 4 | GPU tag: `V3D4`, `PVRB`, `GC7K` or `MALI`, from the `gpu-*` feature |
| 16 | 32 | build identity: the SHA-256 of the identity string below |
| 48 | 32 | the SHA-256 of the payload |
| 80 | n | payload |

The identity string is `yakogl <CARGO_PKG_VERSION> gpu=<v3d|powervr|vivante|mali>
es=3.0 shaderc=<online|precompiled|online+precompiled|offline> fmt=1 id=<option_env!("YAKOGL_BUILD_ID")
or empty>`. It is hashed with `crate::shaderc::sha256::sha256`, which is in
every build. A product image can stamp its own `YAKOGL_BUILD_ID`; no
build.rs change is needed.

A binary whose magic, version, header size, GPU tag, identity, payload size
(it must equal `length - 80`) or payload SHA-256 differs is **rejected, with
link status FALSE and no GL error** (§2.12.4: the load "will fail, setting
the LINK_STATUS of program to FALSE"; "If ProgramBinary fails to load a
binary, no error is generated"). The info log names the field that differed.
Mesa does the same with one 20-byte BLAKE3 of the driver and version plus a
CRC32 of the payload (`program_binary.c`, `program_binary_header`,
`is_program_binary_valid`). We use SHA-256 because we already have it, and
the GPU tag on its own makes the info log of a cross-GPU rejection readable.
These checks are not a security boundary: the parser must stay bounds-safe
on any bytes whatever the hash says (see the payload rules).

### Payload: the link inputs, linked again on load (decision)

The payload holds the link inputs, in order:

```
u32 vs_bytes, vs source (no NUL, none inside)
u32 fs_bytes, fs source (no NUL, none inside)
u8  nbinds (<= MAX_ATTRIB_BINDS); per binding: u8 name_len (1..=31), name, u8 index (< MAX_VERTEX_ATTRIBS)
u32 tf_mode (the request's mode as stored: 0, INTERLEAVED_ATTRIBS or SEPARATE_ATTRIBS)
u32 tf_count, u32 tf_bytes, tf names (NUL-separated; exactly tf_count NULs; the last byte a NUL)
```

The payload must end exactly at `payload bytes`. A short field, a count over
its limit, an interior NUL or trailing bytes is a rejection like a header
mismatch: link status FALSE and no error. This is every input that
`glLinkProgram` reads (`src/gl/program.rs`, `glLinkProgram`): the two
stages' sources (each compiled shader is a pure function of its source, and
the link reads `vs_src` again for the attribute layout, `#version 300 es`
and transform feedback), the `glBindAttribLocation` bindings (`GlProgram::attrib_binds`),
and the transform feedback request (`tf.rs`, `TfProg::pending*`). Fragment
output locations come only from `layout(location)` in the source, because
ES has no `BindFragDataLocation`. The retrievable hint is not a link input
(see (a)).

A load compiles both sources with the build's shader back end
(`shaderc::backend().compile`, as `glCompileShader` does), then runs the
same link with these inputs.

**Why not store the compiled back-end code.**

1. `ShcShader` (`src/shaderc/mod.rs`) is a set of pointers into compiler
   output, with a GPU-specific half for the V3D and for the GC7000. Some
   variants are compiled later, outside the program. The GC7000
   front-facing variant is keyed by GL state (`reads_front_facing`), and so
   are the blend-key and sampler-key variants. §2.12.4 requires that "If a
   GL implementation needs to recompile ... based on GL state outside the
   program, GetProgramBinary is required to save enough information to allow
   such recompilation". In this design that information is the source.
2. Storing code would need a serializer and a validator per back end:
   PowerVR USC, V3D QPU, GC7000 and Mali, each with its own relocations.
   Four back ends' code, owned by other plans (#350, #325, #335, #355),
   would have to change. The source payload touches the GL core only.
3. A load already costs little. The in-process compiler caches by record key
   (`src/shaderc/record_key.rs`), so loading a program the process has
   compiled before is a cache hit. A precompiled-table build looks the
   source up by the same key, just as the original compile did.
4. Its cost: a load into a fresh process compiles again, so the load does not
   save "online compilation" (the purpose §2.12.4 names). That is allowed:
   the spec requires only that a binary loads under the same configuration
   and then runs correctly. A later format version 2 may add per-back-end
   code as an optional section, with the sources kept as the fallback.
   Version 1 does not.

### When the binary is made (decision)

The binary is **made at the end of every successful link and kept until the
next link, load or delete**. It is not built when `glGetProgramBinary` is
called. The persistence cases need this: after a detach, delete, recompile
or new source without a relink, the bytes must be identical, so the binary
must not read the shader objects afterwards. A successful load keeps
**the loaded bytes themselves**, so a second `glGetProgramBinary` returns
the same binary. `GL_PROGRAM_BINARY_LENGTH` reads the length from the kept
header (80 + payload bytes).

Every link is retrievable whatever the hint says: the persistence cases do
not set the hint. The cost is one heap block (`util::malloc`) per linked
program, about the size of its two sources. `program_binary.rs` keeps one
pointer per `[MAX_CONTEXTS][MAX_PROGRAMS]` slot, in its own `Global`. At the
defaults (2 × 512) that is 8 KiB of BSS on a 64-bit target and 4 KiB on a
32-bit one. The length lives in the block's header, so no length field is
needed. If the allocation fails, the link still succeeds,
`GL_PROGRAM_BINARY_LENGTH` answers 0, and `glGetProgramBinary` answers
`GL_OUT_OF_MEMORY` (see (d)).

A link can only succeed with both stages compiled, and `glShaderSource`
clears `compiled` (`src/gl/shader.rs`). So the source that the link reads
from the shader object is always the source that was compiled.

## (c) What a load does

A load is "an implicit linking operation" (§2.12.4). It follows the
`glLinkProgram` path, so everything a link resets, a load resets:

- **Link status and info log.** Both are set by the load, success or
  failure (§2.12.4). On success the log is the relink's, normally empty. On
  failure it names the rejected field, or carries the compile or link log of
  the relink. "Any information about a previous link or load of that program
  object is lost": `linked = false`, the uniform blocks are freed, the
  location and block tables are cleared, `tf::link_begin` runs and
  `draw::invalidate_program` runs. These are the first steps of
  `glLinkProgram`. The kept binary is dropped. `GL_VALIDATE_STATUS` goes
  back to FALSE, as on a link.
- **Uniforms.** "A successful call to ProgramBinary will reset all uniform
  variables to their initial values, FALSE for booleans and zero for all
  others" (§2.12.4). The relink allocates the blocks zeroed (`calloc` in
  `glLinkProgram`), and `build_locations` sets every sampler unit back to 0
  (the unit test `relink_resets_uniform_values_and_sampler_units`).
  Uniform-block bindings go back to 0, as on a link. GLSL ES has no uniform
  initialisers, so zero is the only initial value.
- **Attribute and output assignments.** "All vertex shader input and
  fragment shader output assignments that were in effect when the program
  was linked before saving are restored". The relink uses the **binary's**
  bindings, not the program's current `attrib_binds`. Even so, the program's
  own `glBindAttribLocation` bindings are left as they were, for both
  success and failure: "The failure does not alter other program state not
  affected by linking such as the attached shaders, and the vertex attribute
  location bindings". The load therefore saves `attrib_binds` and
  `num_attrib_binds`, puts the binary's bindings in, links, and puts the
  program's own bindings back. The transform feedback request is handled the
  same way: the binary's request is used for this link, and the program's
  pending request (`TfProg::pending*`) is left untouched.
- **Attached shaders.** These are untouched. The sources are compiled into
  temporary `ShcShader`s that do not belong to a shader object, the link
  retains its own copies (`shaderc::retain_shader`), and the load then
  releases the temporaries (`shaderc::release_shader`).
- **Program in use** (§2.12.3). "If LinkProgram or ProgramBinary
  successfully re-links a program object that was already in use ... the
  generated executable code will be installed as part of the current
  rendering state". `c.cur_program` keeps the name and the draw path reads
  the relinked program, as after `glLinkProgram`, so nothing extra is
  needed. That is what `uniform_reset_on_binary_load` exercises. A *failed*
  load of the program in use behaves like a failed `glLinkProgram` of it
  today: the executable is gone. §2.12.3 asks that it stays until the next
  `UseProgram`. That deviation belongs to `glLinkProgram`, not to G7, and
  is reported for the planner rather than fixed here.
- **The hint** is untouched (see (a)). `GL_DELETE_STATUS` is untouched.

## (d) Errors

The rule for every program-name argument is §2.12.1: `GL_INVALID_VALUE` for
a name that is neither a shader nor a program, `GL_INVALID_OPERATION` for a
shader name. `program.rs` already has this rule as `not_a_program`. The
three calls of §2.12.4 (`glProgramParameteri`, `glGetProgramBinary`,
`glProgramBinary`) switch from `es3.rs` `program_exists` (always
`GL_INVALID_OPERATION`) to `not_a_program`. That fixes
`negative_api.shader.program_parameteri` (see (a)).
`negative_api.shader.program_binary` passes a shader name, so it stays
`GL_INVALID_OPERATION`. `program_exists` stays as it is for its other
callers. A command that raises an error has no other effect (§2.5).

`glGetProgramBinary(program, bufSize, length, binaryFormat, binary)`, checked
in this order:

1. The name is not a program: §2.12.1, as above.
2. `LINK_STATUS` is FALSE: `GL_INVALID_OPERATION` (§2.12.4: "its program
   binary length is zero, and a call to GetProgramBinary will generate an
   INVALID_OPERATION error").
3. `bufSize` is less than the binary's length, **a negative `bufSize`
   included**: `GL_INVALID_OPERATION` (§2.12.4: "If bufSize is less than the
   number of bytes in the program binary"). Mesa raises
   `GL_INVALID_VALUE` for a negative `bufSize` first (`shaderapi.c`,
   `_mesa_GetProgramBinary`). We do not. §2.12.4 names only the size
   error, and a negative size is less than any length. `es3.rs` has pinned
   `GL_INVALID_OPERATION` for `bufSize` -1 since 2026-09-24
   (`the_program_binary_calls_refuse_...`, which answered `GL_INVALID_VALUE`
   before).
4. The program is linked but no binary was kept (an allocation failed at
   link): `GL_OUT_OF_MEMORY`.
5. Otherwise copy the binary, write `*length` (if not null) and
   `*binaryFormat` (if not null).

Steps 1-4 write `*length = 0` when `length` is not null. Mesa does the same,
and today's unit test pins it. With `binary` null and a non-zero length,
nothing is copied and no error is raised: the GL contract was broken, and we
still must not crash.

`glProgramBinary(program, binaryFormat, binary, length)`, checked in this
order:

1. The name is not a program: §2.12.1.
2. `length < 0`: `GL_INVALID_VALUE` (§2.3.1, negative `sizei`). This is
   today's order and Mesa's.
3. `binaryFormat` is not `GL_PROGRAM_BINARY_FORMAT_YAKOGL`:
   `GL_INVALID_ENUM`, and **nothing else changes**. Mesa also sets the link
   status to FALSE here. We follow §2.5 instead (an error has no side
   effect). dEQP does not read the link status after this error.
4. The program is used by an active transform feedback object, bound or not,
   paused or not: `GL_INVALID_OPERATION` (§2.15.2 names ProgramBinary
   explicitly). This is `tf::program_in_use`, the same check as in
   `glLinkProgram`.
5. Otherwise the load runs (see (c)). Anything wrong with the bytes gives
   link status FALSE, an info log and **no GL error**. That covers `length`
   below 80, a null `binary` with a non-zero length, any header mismatch, a
   malformed payload, a failed compile of either source, or a failed relink.

`glGetProgramiv(GL_PROGRAM_BINARY_LENGTH)` answers the kept binary's length
when linked, and 0 when not linked or nothing was kept (Table 6.18, initial
value 0; Mesa answers the same). `glGetIntegerv` (and the other `glGet*`
forms through `get_integerv`) answers `GL_NUM_PROGRAM_BINARY_FORMATS` with 1
and `GL_PROGRAM_BINARY_FORMATS` with one value, the token.

## (e) Module split, and every function the implementation changes

New: **`src/gl/program_binary.rs`**, with all of the logic.

- `pub const GL_PROGRAM_BINARY_FORMAT_YAKOGL: GLenum = 0x6B47;`
  `pub(super) const NUM_FORMATS: GLint` (1, or 0 under `REPORT_ES2`).
- `fn identity() -> [u8; 32]`, `fn gpu_tag() -> [u8; 4]`.
- `pub(super) struct LinkSnapshot<'a> { vs_src: &[u8], fs_src: &[u8], binds: &[GlAttribBind], tf: TfRequest<'a> }`.
- `pub(super) fn capture(group, program, &LinkSnapshot)`: serialize and keep
  (on allocation failure keep nothing).
- `pub(super) fn forget(group, program)`: free the kept block.
- `pub(super) fn length(group, program) -> GLint`.
- `pub(super) unsafe fn get(c, program, bufSize, length, binaryFormat, binary)`:
  steps 2-5 of `glGetProgramBinary`.
- `pub(super) unsafe fn load(c, program, binary, length)`: step 5 of
  `glProgramBinary`. It parses and validates into a `Parsed` view, which
  holds slices into the caller's bytes and never touches GL state. It
  copies the two sources into NUL-terminated heap strings, compiles both,
  swaps in the bindings, calls `program::link_with`, restores the bindings,
  releases the temporaries and, on success, keeps a copy of the bytes.
- `fn parse(bytes) -> Result<Parsed, &'static str>`: pure, so the unit tests
  can feed it truncated or corrupted bytes. The error string goes to the
  info log.
- Its own `#[cfg(test)] mod tests` for `parse` and the round trip of the
  serializer, and `reset_for_tests`.

`src/gl/program.rs` (owned by #343; keep these edits small):

| Function | Change |
|---|---|
| `glLinkProgram` | Split. The entry point keeps its error checks (program name, `tf::program_in_use`), then gathers the inputs from the attached shader objects (the missing and compiled checks, `vs_src`, `fs_src`, `rvs`, `rfs`, the pending TF request) and calls `link_with`. After a success outside `REPORT_ES2` it calls `program_binary::capture`. |
| new `pub(super) unsafe fn link_with(c, program, inputs: &LinkInputs) -> bool` | The body of today's `glLinkProgram` from `st.validated[..] = false` to `p.linked = true`, unchanged except that it reads `vs_src`, `rvs`, `rfs` and the TF request from `inputs` instead of the shader objects, and calls `program_binary::forget` where it clears the old link. `LinkInputs { vs_src: *const c_char, fs_src: *const c_char, rvs: ShcShader, rfs: ShcShader, tf: TfRequest }`. |
| `destroy_program` | Calls `program_binary::forget`. |
| `glCreateProgram` | Calls `program_binary::forget` beside the line that clears `binary_retrievable` (defensive: `destroy_program` already frees the block). |
| `glGetProgramiv` | `GL_PROGRAM_BINARY_LENGTH => program_binary::length(..)` instead of `0`. The `REPORT_ES2` arm stays first and unchanged. The comment changes. |
| `ProgState::binary_retrievable` (doc comment only) and `set_binary_retrievable_hint` | Unchanged in behaviour. Only the comment "no binary format exists" is corrected. |

`src/gl/state.rs` (owned by #359; two match arms):

| Function | Change |
|---|---|
| `get_integerv` | `GL_NUM_PROGRAM_BINARY_FORMATS => program_binary::NUM_FORMATS`. `GL_PROGRAM_BINARY_FORMATS` writes the token into `out[0]` and returns 1 (today it returns 0 values). |
| `es3_only_pname` | Unchanged: both pnames stay ES 3.0-only, which is what keeps the `report-es2` answer (see (f)). |

`src/gl/es3.rs`:

| Function | Change |
|---|---|
| module doc (lines 16-19) | "the program-binary calls (there are no binary formats ...)" becomes a pointer to `program_binary`. |
| `glProgramParameteri` | Name check through `program::not_a_program` (see (d)). The comment "there are no binary formats" goes. |
| `glGetProgramBinary` | Under `REPORT_ES2`: today's body. Otherwise the name check, then `program_binary::get`. |
| `glProgramBinary` | Under `REPORT_ES2`: today's body. Otherwise the name check, `length < 0`, format, `tf::program_in_use`, then `program_binary::load`. |
| test `program_binary_length_is_zero_and_frag_data_needs_a_link` | The binary half is rewritten: the length is > 0, `bufSize` 0 and length - 1 give `GL_INVALID_OPERATION`, and the full length succeeds. The `glGetFragDataLocation` half stays. |
| test `the_program_binary_calls_refuse_because_there_are_no_formats` | Becomes `#[cfg(feature = "report-es2")]` and keeps today's assertions. An ES 3.0 twin pins (d), with `p + 1` giving `GL_INVALID_VALUE`. |
| test `the_program_binary_hint_is_accepted_and_stored` | `glProgramParameteri(p + 1, ..)` becomes `GL_INVALID_VALUE` (§2.12.1). |
| `program_exists` | Unchanged; it keeps its other callers. |

`src/gl/tf.rs` (owned by #343; accessors only):

| Function | Change |
|---|---|
| new `pub(super) struct TfRequest<'a> { names: &'a [u8], count: u32, mode: GLenum }` and `pub(super) fn pending(group, program) -> TfRequest<'static>` | The request as `set_varyings` stored it, for `glLinkProgram`'s inputs and for `capture`. |
| `link_resolve` | Takes the request as an argument, `link_resolve(group, program, vs_src, req: &TfRequest, say)`, instead of reading `p.pending*`. It sets `p.mode` from `req.mode`. The only caller is `link_with`. |

`src/gl/mod.rs` (owned by #361): one line, `mod program_binary;`.

`src/gl/context.rs`: no change. `GlProgram::attrib_binds` and
`num_attrib_binds` are `pub` already.

## (f) `report-es2` builds answer as today

With `YAKOGL_REPORT_ES2` / the `report-es2` feature, nothing observable
changes, and `crate::REPORT_ES2` (a `const`) removes the new paths at
compile time:

- `glGetIntegerv(GL_NUM_PROGRAM_BINARY_FORMATS / GL_PROGRAM_BINARY_FORMATS)`
  gives `GL_INVALID_ENUM`. The `REPORT_ES2 && es3_only_pname` check at the
  top of `get_integerv` catches it before any match arm (pinned by `tests/es2_error_rules_state.rs` and
  `tests/es2_report_guard.rs`).
- `glGetProgramiv(GL_PROGRAM_BINARY_LENGTH / _RETRIEVABLE_HINT)` gives
  `GL_INVALID_ENUM`. Its `REPORT_ES2` arm comes first.
- `glGetProgramBinary`: `*length = 0`, `GL_INVALID_OPERATION`, for every
  `bufSize`. `glProgramBinary`: today's checks, then `GL_INVALID_ENUM`.
  `glProgramParameteri`: today's checks, including `program_exists` (the
  name rule is not changed under `REPORT_ES2`, so the ES 2.0 answers stay
  byte for byte).
- `glLinkProgram` does not call `capture`, so an ES 2.0 image never
  allocates a binary.
- `GL_EXTENSIONS` claims no `GL_OES_get_program_binary`, in either build.

## (g) The host test `tests/es3_program_binary.rs`

Header: `#![cfg(all(feature = "host-test", feature = "shaderc-online",
not(feature = "report-es2"), any(feature = "gpu-powervr", feature =
"gpu-vivante", feature = "gpu-v3d")))]`. It uses the stand-in back end of
`tests/es3_program_binary_hint.rs` (no GPU draws). Run lines:

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_program_binary
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_program_binary
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_program_binary
```

The `report-es2` half is pinned by the `es3.rs` unit test under
`cfg(feature = "report-es2")` (see (e)) and by the existing
`es2_error_rules_state` and `es2_report_guard`. Mali is not a row: its
compiler refuses every program.

The cases (one `#[test]` each):

1. **Formats.** `GL_NUM_PROGRAM_BINARY_FORMATS` is 1, and
   `GL_PROGRAM_BINARY_FORMATS` is `[0x6B47]` through `glGetIntegerv`,
   `glGetInteger64v` and `glGetBooleanv` (one value each). No error.
2. **Length and round trip.** An unlinked program has length 0. A linked one
   has length > 80, and `glGetProgramBinary(len)` writes `len` bytes with
   the format 0x6B47. Load into a new program: `GL_LINK_STATUS` TRUE, no
   error, the same `GL_ACTIVE_UNIFORMS` / `GL_ACTIVE_ATTRIBUTES` and the
   same uniform and attribute locations, and `glGetProgramBinary` of the new
   program returns the same bytes.
3. **dEQP `get_program_binary_vertex_fragment`.** The dEQP sources: get and
   verify the format.
4. **dEQP `uniform_reset_on_binary_load`.** The dEQP sources: use the
   program, set bool, int and float uniforms, draw, then load into the same
   current program. `glGetUniformiv` / `fv` read 0, no error, the program is
   still current and draws without error.
5. **Persistence.** The six dEQP variants on each stage: identical bytes
   without a relink. After `replace_source` followed by a relink, the bytes
   differ.
6. **Attribute bindings.** Link with `glBindAttribLocation(a_position, 3)`
   and save. In the target program bind `a_position` to 5, then load:
   `glGetAttribLocation` is 3. Then `glLinkProgram` the target with its own
   attached shaders: 5. (The load restored the binary's assignment and kept
   the program's own binding.)
7. **Transform feedback.** A program linked with two varyings in
   `GL_SEPARATE_ATTRIBS`, loaded into a fresh program:
   `GL_TRANSFORM_FEEDBACK_VARYINGS` is 2, the mode matches, and
   `glGetTransformFeedbackVarying` gives the same names and types. The
   target's own pending request (set to one other varying before the load)
   is the one a later `glLinkProgram` uses.
8. **Rejections, no GL error.** Flip one byte in each of: magic, version,
   header size, GPU tag, identity, payload hash, and the payload (the hash
   catches it). Also a truncated `length`, `length` 0, a null `binary`, and
   a payload with trailing bytes (rehashed so only the parser catches it).
   Each gives `GL_LINK_STATUS` FALSE, `glGetError` `GL_NO_ERROR`, a
   non-empty info log, `GL_PROGRAM_BINARY_LENGTH` 0 and `glUseProgram`
   `GL_INVALID_OPERATION`. The attached shaders and bindings are unchanged.
9. **Errors (d).** `glGetProgramBinary`: name 0 → `GL_INVALID_VALUE`; a
   shader name → `GL_INVALID_OPERATION`; unlinked → `GL_INVALID_OPERATION`;
   `bufSize` 0, length - 1 and -1 → `GL_INVALID_OPERATION` with
   `*length` 0. `glProgramBinary`: a shader name → `GL_INVALID_OPERATION`;
   `length` -1 → `GL_INVALID_VALUE`; format -1 and 0 → `GL_INVALID_ENUM`,
   and a linked target **stays linked**; the program of an active (also a
   paused) transform feedback object → `GL_INVALID_OPERATION`.
   `glProgramParameteri(0, ..)` → `GL_INVALID_VALUE`.
10. **dEQP negative sequences.** `get_program_binary` and `program_binary`
    as in (a), step by step.
11. **Hint and state query.** The `state_query.shader.program_binary`
    sequence: the hint is FALSE, then TRUE after a set and a relink, and
    `GL_PROGRAM_BINARY_LENGTH` is written and > 0. A load does not change
    the hint.

After the implementation, the item `g7-host-verify` runs the host dEQP
groups `shader_api.program_binary.*`, `state_query.shader.program_binary` and
`negative_api.shader.{get_program_binary,program_binary,program_parameteri}`
on the three GPUs.
