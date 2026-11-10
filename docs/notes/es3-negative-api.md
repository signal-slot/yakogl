<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 negative_api: the first wrong error of five dEQP-GLES3 cases

Plan #343, item `an-negapi`, written 2026-11-10 at `dfae7366`. The five cases
fail with "Got invalid error" on the AM62P (`docs/es3/am62p-gles3-notpass.tsv`)
and on the Pi 4 (`docs/es3/pi4-gles3-fails.tsv`, image M1). Two GPUs with two
different back ends fail the same five rows, and every site below is in the
GL core (`src/gl/`): none is back-end code. `blit_framebuffer{,_multisample}`
are RC8 (plan #339) and not part of this note.

## How this was found (HOST-ONLY)

The case sources are VK-GL-CTS `modules/gles3/functional/`
`es3fNegative{Texture,Shader,Buffer}ApiTests.cpp` (a fresh checkout of the CTS
head in the deps cache, not the 93bca01 of `tools/cts/README.md`; these five
bodies are old and stable). Every call and its expected error was replayed in
order through the public GL API in a throw-away host test (the stand-in
back end of `tests/es3_attrib_location.rs`, `gpu-powervr,shaderc-online`,
ES 3.0 report; **not committed**: each fix item writes its own
`tests/es3_*.rs`). dEQP itself was not built or run for this item
(`base-*` items own the HOST-ONLY runs). dEQP stops at nothing: it records
the first wrong error and goes on, so "the first failing call" below is the
first line of the replay that differs; later calls of the same case can
differ too and are listed.

Result of the replay, `want`/`got` being GL errors:

| case | calls that differ |
|---|---|
| `texture.texsubimage2d` | `glTexSubImage2D(TEXTURE_2D,0,0,0,4,4,RGB,FLOAT,0)` after `glTexImage2D(RGBA,UNSIGNED_BYTE)`: want INVALID_OPERATION, got NO_ERROR. Only this call. |
| `texture.texsubimage3d` | `glTexSubImage3D(TEXTURE_3D,0,0,0,0,4,4,4,RGB,FLOAT,0)`: same. Only this call. |
| `shader.uniformui_incompatible_type` | `glUniform4ui(ivec4_f,0,0,0,0)`: want INVALID_OPERATION, got NO_ERROR. Only this call. |
| `shader.uniformuiv_incompatible_type` | `glUniform4uiv(ivec4_f,1,data)`: same. Only this call. |
| `buffer.read_pixels_fbo_format_mismatch` | the `expectError(NO_ERROR)` after `glRenderbufferStorageMultisample(RENDERBUFFER,4,RGBA8,32,32)` + `glFramebufferRenderbuffer` + `glCheckFramebufferStatus` + `glGetIntegerv(SAMPLE_BUFFERS)`: want NO_ERROR, got INVALID_OPERATION. `SAMPLE_BUFFERS` reads 0, so dEQP goes to its "Got invalid value" branch and never reaches `glReadPixels` (the replay's `glReadPixels` there answers INVALID_FRAMEBUFFER_OPERATION: the FBO is incomplete). |

## 1. `negative_api.texture.texsubimage2d` and `texsubimage3d`

Calls and expected errors (`es3fNegativeTextureApiTests.cpp:1459`, `:2291`),
all on a 32x32 (3D: 4x4x4) `GL_RGBA`/`GL_UNSIGNED_BYTE` texture:

| call | expected |
|---|---|
| target `0` (3D: also `GL_TEXTURE_2D` on `glTexSubImage3D`) | INVALID_ENUM |
| format not a format (the 2D case passes `4` there; the 3D one passes `0`) | INVALID_ENUM |
| type `0` with `GL_RGB` | INVALID_ENUM |
| `RGBA`/`UNSIGNED_SHORT_5_6_5`; `RGB`/`..._4_4_4_4`; `RGB`/`..._5_5_5_1` (2D also the first); `RGBA_INTEGER`/`UNSIGNED_INT` | INVALID_OPERATION |
| **`RGB`/`GL_FLOAT`** (ES contexts only, `isES`) | **INVALID_OPERATION** |

Actual: everything up to the last row is right. The last row is accepted.

**Rule.** ES 3.0.6 section 3.8.5 ("Texture Image Specification", for
`TexSubImage*`: "the combination of the internal format of the previously
specified texture array, format and type must be a row of table 3.2,
INVALID_OPERATION otherwise; as the dEQP section text puts it) and table 3.2: the unsized internal formats
`RGBA` and `RGB` take `UNSIGNED_BYTE` and their packed types only. `FLOAT`
pairs with the sized `R32F`, `RG32F`, `RGB32F`, `RGBA32F` (and `RGB9_E5`,
`R11F_G11F_B10F`, `DEPTH_COMPONENT*`), never with an unsized colour format.
(GL_OES_texture_float would allow it; this build does not claim it,
`src/gl/state.rs:1636`.)

**Fix site.** `es3_sub_image_check`, `src/gl/texture.rs:5997`, its `None`
arm at `:6005` (a texture with no row in `es3_formats`, i.e. an unsized or an
8-bit sized one): `only_for_new_formats` (`src/gl/es3_formats.rs:288`) does
not list `GL_FLOAT`, because the ES 2.0 surface unpacks it and that table
means "pairs only the ES 3.0 formats take". The one function serves both
entry points (`glTexSubImage2D` at `:3071`, `tex_sub_image_3d` at `:5489`),
so one edit fixes both rows. Change: in a non-`report-es2` build, a texture
whose `es3_fmt` is `None` refuses `GL_FLOAT` when its base format is an
unsized colour one (check what the texture records of its format first: the
R32F/RGBA32F/depth textures must keep taking it, `src/gl/texture.rs:11937`
uploads `GL_RED`/`GL_FLOAT` and `tests/es3_blit_ds.rs:1015` uploads depth
`GL_FLOAT`). Do not widen it to `GL_HALF_FLOAT_OES`, which the build claims
(`state.rs:110`).

Not in these rows, same rule: `glTexImage2D/3D(RGBA, FLOAT)` into an unsized
internal format (`es3_image_check`, `texture.rs:5913`) is accepted too; no
listed case reaches it, so leave it to the fix item to decide with its test.

**Host test (fix item `fix-texsub`).** New `tests/es3_negative_api_texsub.rs`
(not `report-es2`, no GPU gate; the stand-in back end): the two dEQP
sequences above, plus controls that still pass: `RGB`/`FLOAT` into an
`RGB32F` texture, `RED`/`FLOAT` into `R32F`, `RGBA`/`UNSIGNED_BYTE` into
`RGBA`.

**Files and holders.** `src/gl/texture.rs` (held by #333/#328: wait for
them), possibly `src/gl/es3_formats.rs` (no holder named), the new test.

## 2. `negative_api.shader.uniformui_incompatible_type` and `uniformuiv_incompatible_type`

Calls (`es3fNegativeShaderApiTests.cpp:1308`, `:1424`), program with
`vec4 vec4_v`, `ivec4 ivec4_f`, `uvec4 uvec4_f`, `sampler2D sampler_f`, all
active, `glUseProgram` made:

| call | expected |
|---|---|
| `glUniform{1,2,3}ui[v](uvec4_f ...)` | INVALID_OPERATION (size) |
| `glUniform4ui[v](uvec4_f ...)` | NO_ERROR |
| **`glUniform{1,2,3,4}ui[v](ivec4_f ...)`** | INVALID_OPERATION (the 1, 2, 3 forms are refused today by size; **the 4 form is accepted**) |
| `glUniform{1,2,3,4}ui[v](vec4_v ...)` | INVALID_OPERATION |
| `glUniform1ui[v](sampler_f ...)` | INVALID_OPERATION |

Actual: only `glUniform4ui(ivec4_f)` and `glUniform4uiv(ivec4_f)` differ
(NO_ERROR). Note the size mismatches (`ivec4` with 1 to 3) already pass: the
dEQP sections for `ivec4_f` fail on their fourth call only.

**Rule.** ES 3.0.6 section 2.12.6 ("Uniform Variables"): the `glUniform*ui`
commands load `uint`/`uvecN` (and bool) uniforms, and INVALID_OPERATION is
generated when the command's type or size does not match the uniform's
(as this repository's own comment at `program.rs:2337` already cites).

**Cause and fix site.** A deliberate, documented deviation, one way round:
`setter_accepts`, `src/gl/program.rs:2347`, line `:2353`:
`Setter::Uint(n) => ... Base::Int | Base::Uint | Base::Bool`. Its comment
(`:2337`-`:2346`) explains it: Qt's RHI picks the setter from the SPIR-V type
(`uint`) while the GLSL ES 1.00 text declares the uniform `int` (Qt Quick 3D's
light counts). The unit test `an_int_setter_does_not_load_a_uint_uniform`
(`program.rs`, near `:4040`) pins "unsigned setter still loads an int
uniform". So the fix is a decision, not a missing check: take `Base::Int`
through the unsigned setter **only where the program is not a GLSL ES 3.00
one**, which keeps Qt's ES 1.00 programs working and gives dEQP (all
`#version 300 es`) its error. The program has no version flag today; the
source scan `is_glsl_es300` (`program.rs:1504`, used by `ExplicitAttribs::scan`
at `:1403`) reads it from the shader text, so the link would record it per
program (a `bool` beside `ExplicitAttribs`) and `set_uniform` (`:2536`)
would pass it to `setter_accepts`. Check with the owner of the Qt use
(`docs/notes/`, Qt/femtovg items) that Qt does not run `#version 300 es`
programs through the `ui` setters: if it does, the alternative is a
`report-es2`-style build option and the dEQP rows move only in the
`report-es2=off` image, which is the one these rows come from.

**Host test (`fix-uniformui`).** New `tests/es3_negative_api_uniform.rs`
(needs `shaderc-online`, ES 3.0 report, `gpu-*`: the pattern of
`tests/es3_link_iface.rs`): the two dEQP programs/sequences, plus the Qt
control (an ES 1.00 program with an `int` uniform loaded by `glUniform1ui`
keeps NO_ERROR).

**Files and holders.** `src/gl/program.rs` only (no holder named in plan
#343), the new test. The existing unit test in `program.rs` stays valid.

## 3. `negative_api.buffer.read_pixels_fbo_format_mismatch`

Calls (`es3fNegativeBufferApiTests.cpp:245`): RGBA8 texture on a FBO:
`glReadPixels(RGBA, FLOAT)` INVALID_OPERATION; then RGBA32I and RGBA32UI
textures with `glReadPixels(RGBA, FLOAT)` INVALID_OPERATION (all three pass
today); then `glRenderbufferStorageMultisample(RENDERBUFFER, 4, RGBA8, 32, 32)`
attached to the FBO, `glCheckFramebufferStatus`, `glGetIntegerv(SAMPLE_BUFFERS)`
and `expectError(NO_ERROR)`; if `SAMPLE_BUFFERS` is 0 the case records
"Got invalid value"; else `glReadPixels(RGBA, UNSIGNED_BYTE)` expects
INVALID_OPERATION.

**Cause.** Not a glReadPixels rule. `glRenderbufferStorageMultisample`
(`src/gl/framebuffer.rs:3460`) answers GL_INVALID_OPERATION to any
`samples > 1` (`:3484`: "no multisampled storage exists"), so the case's
`expectError(GL_NO_ERROR)` is the first wrong error, `SAMPLE_BUFFERS` is then
0. This is RC8, the same cause as `blit_framebuffer_multisample`: **plan #339
(multisample renderbuffers) fixes it, there is no separate fix site.**
`glReadPixels` already has the rule the last step needs: ES 3.0.6 section
4.3.2 ("if the read framebuffer's `SAMPLE_BUFFERS` is greater than zero
INVALID_OPERATION"), `src/gl/framebuffer.rs:2827`, via `framebuffer_samples`
(`:897`) which reads `attached_samples` (`:882`); `GL_SAMPLE_BUFFERS` is
answered by `src/gl/state.rs:1191` from the same function.

**What `fix-readpixels` does.** Nothing before #339 lands. After it, rerun
the replay on the main tip: with `max_samples >= 4` a 4-sample RGBA8
renderbuffer must give NO_ERROR, a complete FBO with `SAMPLE_BUFFERS` 1 and
`glReadPixels` INVALID_OPERATION; on a back end with `max_samples` 0 (if one
keeps it) the case ends NotSupported or Fail by #339's RC8 decision, not by
this note. If the rows still fail then, the likely remainder is the
`glReadPixels` path when the *read* framebuffer differs from the bound one
(`as_read`, `framebuffer.rs:2807`), which has no replay here.

**Host test.** `tests/es3_negative_api_readpixels.rs` (after #339; its
`max_samples` field is in every `YakoglBackend {` literal by then, grep
`tests/` and `src/` for them): the three format mismatches (already green,
they pin the rule) and the multisample read.

**Files and holders.** `src/gl/framebuffer.rs` (held by #335 and #339),
`src/gl/state.rs`, `src/backend.rs` (#339). Wait for #339; do not edit around
it.

## Summary for the planner

| case | first wrong call | fix site | holder |
|---|---|---|---|
| texsubimage2d | `RGB`/`GL_FLOAT` into an RGBA8 texture | `texture.rs:6005` (`es3_sub_image_check`, `None` arm) | #333/#328 |
| texsubimage3d | same, 3D | same function (`texture.rs:5489`) | #333/#328 |
| uniformui_incompatible_type | `glUniform4ui(ivec4)` | `program.rs:2353` (`setter_accepts`) plus a per-program GLSL version flag | none named |
| uniformuiv_incompatible_type | `glUniform4uiv(ivec4)` | same | none named |
| read_pixels_fbo_format_mismatch | `glRenderbufferStorageMultisample(4)` error | `framebuffer.rs:3484`, plan #339 | #339, #335 |

Two of the four fix items (`fix-texsub`, `fix-readpixels`) wait on other
plans for their file; `fix-uniformui` can go now. The Pi 4 and AM62P show the
same rows because none of the sites is back-end code; the GC7000UL's
`negative_api` rows were not in the lists given to this item, but the same
sites apply there.
