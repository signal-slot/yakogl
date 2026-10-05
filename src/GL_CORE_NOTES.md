# GL core notes

- **Vertex array objects** (`gl/vao.rs`, `GL_OES_vertex_array_object`, ES 3.0 core
  names too): a VAO snapshots the vertex-attribute arrays + the ELEMENT_ARRAY_BUFFER
  binding; the ARRAY_BUFFER binding is not VAO state (captured per attribute).

Portable GL ES 2.0 state machine that records into `struct gl_context`
(`GlContext`) and, on draw, resolves to a `struct gles_draw` (`GlesDraw`,
`src/backend.rs`) and hands it to the backend (`g_backend->draw`). The core is Rust
(`src/gl/`, part of the crate `yakogl`), ported file for file from the C core
(`src/gl_X.c` became `src/gl/X.rs`); the C names below (`struct gl_context`,
`gl_ctx`, `gl_lookup_*`) are the exported C ABI, still declared in the C-view headers.

## Files

| File            | Responsibility |
|-----------------|----------------|
| `gl_internal.h`, `gl_context.h` | The C view of the core's internals: the `#[repr(C)]` structs of `gl/context.rs` and the `#[no_mangle]` helpers (object-table lookups, default-surface accessor, render target, per-program GPU cache hooks) for C code that looks inside the library (the on-silicon test suite, the samples' glue). Must be kept in step with the Rust structs (pinned by `abi_layout.rs` / `abi_layout_check.c`, see "Rust conventions"). Not public headers. |
| `gl/mod.rs`     | Module list and the conventions shared by the modules (see "Rust conventions"). |
| `gl/types.rs`, `gl/consts.rs`, `gl/egl_consts.rs` | The Khronos scalar types; the GL (`u32`) and EGL (`i32`) tokens, **generated** by `yakogl-tools gl-consts` from the headers under `include/`. |
| `gl/context.rs`  | Context lifecycle (`gl_context_create`/`_destroy`), the `gl_ctx` active pointer, `gl_set_error`, default color surface, object lookup helpers. |
| `gl/buffer.rs`   | `glGenBuffers`/`glBindBuffer`/`glBufferData`/`glBufferSubData`/`glDeleteBuffers`/`glIsBuffer`/`glGetBufferParameteriv`. GPU-resident storage via `g_osal->gpu_alloc`. |
| `gl/texture.rs`  | `glGenTextures`/`glBindTexture`/`glTexImage2D`/`glTexSubImage2D`/`glCopyTex{,Sub}Image2D`/`glTexParameter{i,f,iv,fv}`/`glGetTexParameter{iv,fv}`/`glIsTexture`/`glActiveTexture`/`glDeleteTextures`. Converts uploads to linear RGBA8888 in GPU mem. |
| `gl/shader.rs`   | `glCreateShader`/`glShaderSource`/`glCompileShader`/`glGetShaderiv`/`glGetShaderInfoLog`/`glGetShaderSource`/`glGetShaderPrecisionFormat`/`glIsShader`/`glDeleteShader`. Calls `g_shaderc->compile`. |
| `gl/program.rs`  | `glCreateProgram`/`glAttachShader`/`glDetachShader`/`glLinkProgram`/`glUseProgram`/`glGetUniformLocation`/`glUniform{1..4}f{v}`/`glUniform1i{v}`/`glUniformMatrix4fv`/`glGetProgramiv`/`glGetProgramInfoLog`/`glDeleteProgram`. Calls `g_shaderc->link`, keeps a packed uniform block. |
| `gl/state.rs`    | `glViewport`/`glClearColor`/`glClearDepthf`/`glClearStencil`/`glClear`/`glEnable`/`glDisable`/`glIsEnabled`/`glDepthFunc`/`glDepthMask`/`glBlendFunc`/`glBlendEquation`/`glCullFace`/`glFrontFace`/`glScissor`/`glStencilFunc`/`glStencilOp`/`glStencil*Separate`/`glDepthRangef`/`glColorMask`/`glGetError`/`glGetString`/`glGetIntegerv`/`glFinish`/`glFlush`. |
| `gl/egl.rs`      | EGL 1.0..1.5 entry points over the single GL context (see the EGL paragraph below). |
| `gl/vao.rs`      | `glGenVertexArraysOES`/`glBindVertexArrayOES`/`glDeleteVertexArraysOES`/`glIsVertexArrayOES`; `eglGetProcAddress` also resolves the ES 3.0 core names to them (see the first note). |
| `gl/vertex.rs`   | `glVertexAttribPointer`/`glEnableVertexAttribArray`/`glDisableVertexAttribArray`/`glVertexAttrib{1..4}f{,v}`/`glGetVertexAttrib{fv,iv,Pointerv}`. |
| `gl/draw.rs`     | `glDrawArrays`/`glDrawElements`: builds `GlesDraw` from the current context; pending-clear plumbing; per-program GPU (USC + uniform) cache. |
| `gl/framebuffer.rs` | `glGenFramebuffers`/`glBindFramebuffer`/`glFramebufferTexture2D`/`glFramebufferRenderbuffer`/`glCheckFramebufferStatus`/`glGetFramebufferAttachmentParameteriv`, the renderbuffer API, `glReadPixels`, and `gl_fb_target()` (the draw target: default surface or the bound FBO's colour attachment). |

## How `gl/draw.rs` builds a `GlesDraw`

1. **Validate**: current program non-zero and linked, else `GL_INVALID_OPERATION`;
   primitive mode mappable, else `GL_INVALID_ENUM`; `count>=0`/`first>=0`.
2. **Target**: `gl_fb_target()` — the bound FBO's colour attachment (a texture's
   linear RGBA8888 store or a colour renderbuffer) or the default window surface
   allocated in `gl_context_create`; an incomplete FBO records
   `GL_INVALID_FRAMEBUFFER_OPERATION`. `gles_draw.flip_y` says whether the target is
   stored top-down (window) or in texel order (FBO), which flips the viewport y,
   the scissor and the face winding. `glClear` is always a whole-target clear:
   the scissor rectangle is not applied to it (a scissored clear must be drawn
   as a quad by the application for now).
3. **Clear fusion**: `glClear` only *records* a pending clear (color captured at
   call time). The next draw sets `draw->do_clear` + `clear_rgba` and clears the
   flag (one kick, clear-on-tile-load). `glFlush`/`glFinish` emit a clear-only
   kick (`index_count=0`) for a clear with no following draw. The pending
   clear belongs to the context that issued it (`draw::PENDING`, one per
   context slot, outside `GlContext` so its C view does not change): only
   that context's draws and flushes consume it. Making another context
   current carries it out first -- `eglMakeCurrent`, the C API's
   `gl_context_set_current` and `gl_context_create*` alike (EGL 1.5 3.7.3,
   "that context is flushed") -- and `gl_context_destroy` of the current
   context carries it out and finishes the frame before the surface is
   freed, rather than drop a command already issued. Until 2026-09-29 it was
   one record for the process and the C API did not flush it, so context
   B's next draw took context A's clear and A lost it (review R-04).
4. **Shaders**: from the linked program's `rvs`/`rfs`. USC dwords are uploaded to
   GPU mem **once per link** and cached per program (`vs_usc_gpu`/`fs_usc_gpu`).
5. **Attributes**: iterate the VS `shc_attrib` map; for each input whose GL attrib
   index is enabled, emit a `gles_attrib{vi_reg (from shc), components, stride,
   offset, buffer_gpu}`. `stride==0` → tightly packed floats. `glDrawArrays`'
   `first` is folded into each attrib offset (`first*stride`).
6. **Uniforms**: each stage's push-constant block (`gl_program.vs_uniforms` /
   `fs_uniforms`, laid out as `shc_uniform.dword_off` says) is uploaded to a reused
   per-program buffer (VS block first, FS block 16-byte aligned after it);
   `vs_uniform_gpu`/`fs_uniform_gpu` point at each. The VS block reaches the shared
   registers through a VDM PDS_STATE DOUTD program, the FS block through the PPP
   texture-state DMA.
7. **Textures**: every sampler in the FS reflection is wired to the `TEXTURE_2D`
   bound on the unit its `glUniform1i` value names (`gles_texture_bind.unit` is the
   sampler binding = descriptor slot), with filter/wrap flags. Up to 4 samplers
   per draw (`PVR_BACKEND_MAX_SAMPLERS`).
8. **Fixed-function** → backend enums: depth `map_cmp` (GL 0x200..0x207 is in
   `gles_cmp` order), cull `map_cull` (resolves enable+mode+front-face to the
   culled winding), scissor, stencil (`map_stencil_op`; front and back state
   plus `front_cw`, the backend emits the TA's back-face ISP words when they
   differ), colour mask (all-off → the object writes no colour through the
   ISP; any other partial mask is an FS variant dimension, see 9).
9. **Blending** is not fixed-function on this GPU: the blend state
   (`glBlendFuncSeparate`/`glBlendEquationSeparate`) becomes an `SHC_BLEND_KEY`,
   and the draw uses the fragment-shader variant compiled for it
   (`g_shaderc->blend_variant`, cached with its USC upload per program in
   `prog_gpu.fs_var[]`). Blend off, or ONE/ZERO, uses the plain FS. A state with
   no variant (not precompiled, refused by the online compiler, unlinkable,
   out of memory) refuses the draw: no GL error exists for it, so it is
   counted (`yakogl_draws_refused`, read as a difference), logged once per
   cause, and a pending glClear stays pending for the next draw. The V3D and
   the GC7000 blend in fixed function and ask for no blend variant (the
   GC7000's winding twin still refuses when missing). The key also carries
   the disabled `glColorMask` channels (`SHC_BLEND_KEY_MASK_OFF`), and a variant
   that reads the blend colour (`shc_shader.blend_consts_off`) gets
   `glBlendColor` appended to its uniform block at upload time.
9. **Indices**: bound `ELEMENT_ARRAY_BUFFER` → `buf.gpu + offset`; client-side
   arrays widened (u8→u16) / copied (u16) into a reused scratch buffer.
   `glDrawArrays` → `index_buf_gpu=0` (sequential).

## Fully implemented vs stubbed

**Fully implemented (real behavior):** all of buffers, textures (RGBA/RGB/
LUMINANCE/LUMINANCE_ALPHA/ALPHA + 565/4444/5551 uploads), shaders, program link +
uniforms (float vecs, `mat4` -- with transpose outside a `report-es2` build,
which ES 2.0 has not got -- int/sampler), all render state in the
task list, `glGetError`/`glGetString`/`glGetIntegerv`, and the full draw path
(`glDrawArrays`, `glDrawElements` for u16 buffers and u8/u16 client arrays).

**Stubbed / limited (clear TODO + error where applicable):**
- `glBindAttribLocation` cannot move a compiled input; it is honoured as a
  remap (`gl_program.attrib_app_index[]`): the VS input keeps its GLSL location
  and `gl/draw.rs` sources it from the app's index. Two inputs on one index is a
  link error.
- A program whose active samplers exceed a stage's texture-unit limit, or the
  combined one, fails to link ("One descriptor per sampler the shader declares").
- Index types: u16 indices in a VBO are used in place; u8/u32 (VBO or client
  memory) are widened into a u16 scratch buffer, so an index above 65535 is
  `GL_INVALID_OPERATION` (the vertex-fetch index path is 16-bit). A *non-indexed*
  draw past 65535 vertices is split into pieces instead (up to 64, so ~4.2M
  vertices); what the backend still cannot record it reports, and `gl/draw.rs`
  turns any negative `draw()` result into `GL_OUT_OF_MEMORY` rather than dropping
  the draw silently as the C does; the one exception is `-ENOTSUP` (legal GL
  state the backend cannot render yet, e.g. alpha-to-coverage on the V3D), which
  is `GL_INVALID_OPERATION` so a harness fails that case instead of ending its
  run (dEQP treats `GL_OUT_OF_MEMORY` as fatal).
- Client-side vertex arrays are uploaded per draw into per-attribute scratch
  buffers as packed float32 (BYTE/SHORT/FIXED/HALF_FLOAT/2_10_10_10_REV and
  normalisation converted on the CPU; `glDrawElements` scans the indices for the
  range). A VBO attribute of any of those types is converted the same way, into a
  scratch buffer, because the vertex DMA only copies dwords; a float array at least
  as wide as the shader input is the one case fetched in place.
- `glDeleteBuffers` resets every binding of the deleted buffer in the
  current context to zero (ES 2.0.25 2.9): the targets, the indexed uniform
  buffer points, and each vertex attribute's buffer binding of the bound
  vertex array object (`c.attribs`; the attribute was left out until
  2026-09-29, review R-24). An attribute of an unbound VAO keeps the name
  (ES 3.0 D.1.2), but the store is freed at once: ES 3.0's "the object stays
  alive while a container refers to it" (D.1.3) is not modelled. An enabled
  array with a zero binding is a client array; a client pointer below 4096
  is taken for an offset into a buffer that is not bound and the draw is
  `GL_INVALID_OPERATION` with a log line, as a small `indices` pointer is,
  instead of a read of page 0.
- All seven GLES2 primitive modes are native VDM topologies; lines honour
  `glLineWidth` (up to 16 px, ISPA), points are sprites sized by the VS's
  `gl_PointSize` output (a VS that does not write it draws points of undefined
  size; the backend logs once).
- `glClear` carries the write masks and the scissor box, as GL says. A buffer
  whose mask is entirely off is not cleared, an empty scissor box clears
  nothing, and a box narrower than the target -- or a stencil clear with a
  partial write mask -- becomes `GlesDraw::clear_quad`, a full-screen quad the
  backend draws in sequence with the frame's other objects rather than the
  frame's background object. A colour mask with some channels on and some off
  travels in `GlesDraw::clear_color_mask` (the mask at the glClear, not the
  next draw's) and is the backend's to apply: the GC7000 in its pixel
  engine's component mask (behind `gc_debug_masked_clear`), the PowerVR
  (behind `pvr_debug_masked_clear`, default off until the AM62P has measured
  it) -- which has no per-channel write mask; Mesa's pvr lowers one into the
  fragment shader too -- by drawing the clear with the masked clear program
  for the mask (`PVR_USC_CLEAR_M*`, clear.frag through pco with the mask lowered;
  RGBA8 and RGBA16F), turning a partially masked background clear, alone or
  fused into a draw, into that quad before the draw. Under the same switch any
  fragment shader that reads the pixel (`uses_fbfetch`) runs TRANSLUCENT, as
  in Mesa, never OPAQUE: hidden surface removal would leave it a pixel the
  object behind never reached.
- `glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)` is a pending clear
  like the colour one (`gles_draw.do_clear_depth/do_clear_stencil` + values).
  Colour, depth and stencil are all preserved across kicks: a frame that does
  not start with a colour clear begins with an internal full-screen quad whose
  FS fetches the target's previous texels (`fbload.frag`); the same quad writes
  the depth/stencil clear values into every pixel when those are cleared; and
  the ISP stores the depth/stencil of every kick to a per-target buffer
  (`gles_draw.ds_persist/ds_id`: the window surface, or the FBO's depth/stencil
  renderbuffers) and loads it back when the next kick does not clear it. So
  the backend can split long frames (draw count / arena limits) and apps may
  draw incrementally without glClear. `glDepthRangef` is the viewport z
  transform.
- `glStencil*Separate`: both faces render with their own state (triangles
  only; lines and points use the front state, as GL specifies).
- Mipmaps: a power-of-two texture owns its whole chain (levels consecutive,
  each tightly packed; the layout the TPU walks with `mipmaps_present`), so
  `glTexImage2D(level > 0)` and `glGenerateMipmap` (CPU box filter) fill it and
  the `*_MIPMAP_*` min filters select levels on the GPU. NPOT textures have one
  level (GLES2 rule); `glGenerateMipmap` on them is `GL_INVALID_OPERATION` in a
  `report-es2` build and a no-op otherwise (ES 3.0 dropped the rule).
  `glGenerateMipmap` of a cube map needs all six faces at one size
  (`GlTexture::faces_set`), in both versions.
  Cube maps (six faces, one `glTexImage2D` call each), 2D arrays and 3D
  textures (slices tiled in a grid, sampled through a `sampler3D` emulation)
  exist as of upstream `c1e6084`; the draw resolver flattens them onto the
  stacked-2D layout `GlesTextureBind` describes.
  PowerVR, default off: the native cube (`gl/native_cube.rs`,
  `shaderc::pvr_native_cube`, read when a shader compiles). A program the
  online compiler built with it on samples the TPU's own cube (`smp` 3d) and
  the draw binds a twiddled face-major copy of the cube as TEXTYPE_CUBE,
  copied again when the storage, its content or a render into it changes.
  The Linux harness turns it on with `YAKOGL_PVR_NATIVE_CUBE=1` (the pvrgpu
  OSAL logs `pvr_native_cube=` at init). Not yet measured on silicon.
- Texture completeness (GL ES 2.0 3.7.10) is resolved in `gl/draw.rs`
  (`texture_is_complete`) and reaches the backend as
  `GlesTextureBind::incomplete`, which binds a 1x1 opaque black texel instead
  of the texture: a mipmap minification filter without the whole chain, a cube
  map without all six faces at one size, and -- only in a `report-es2` build --
  a non-power-of-two texture that is not clamped on both axes and filtered
  without mipmaps. Texture object 0 is incomplete too, so a sampler with
  nothing bound reads the same (0,0,0,1).
- All three wrap modes, per axis: `GlesTextureBind::wrap_s` / `wrap_t` carry
  `GlesWrap` (repeat, mirrored repeat, clamp-to-edge) and each backend maps
  them to its own field. A one-texel axis always clamps.
- Compressed textures: `GL_ETC1_RGB8_OES`, `GL_COMPRESSED_RGB8_ETC2` and
  `GL_COMPRESSED_RGBA8_ETC2_EAC` through `glCompressedTexImage2D` /
  `glCompressedTexSubImage2D` (whole blocks), decoded by the TPU from the
  twiddled block layout (`docs/gpu-recipes/mesa_texture_layout.md`); one
  level per texture, no PVRTC (its TPU format is not documented in Mesa).
- Blend states and partial colour masks are limited to what `glsl2usc.sh`
  precompiled (the `blend` lines of `tools/shaders/precompiled.list`,
  `SPEC[:MASK]`); CONSTANT_* factors work when their variant exists
  (`const_color,inv_const_color` is in the default list).
- Attribute arrays with a packed type (BYTE/UNSIGNED_BYTE/SHORT/UNSIGNED_SHORT,
  normalised or not), or narrower than the shader input (a vec3 array for a
  vec4 input reads w = 1 as GL specifies), are converted to float32 of the
  input's width at draw time, in a VBO or not: the vertex fetch DMAs dwords
  and the VS binaries expect float32. This costs a CPU pass over the drawn
  range per draw; float arrays at least as wide as the input are fetched in
  place (only the input's dwords). GLSL ES 1.00 shaders are accepted (the
  offline table's front-end rewrites them; varyings link by name, case 19).
- `glCullFace(GL_FRONT_AND_BACK)` treated as no-cull (no "cull-all" backend enum).

Every GL ES 2.0 entry point exists (`tools/pvrlab/gl_test.c` case 14 covers the
late additions). Accepted but not modelled by the backend:
`glSampleCoverage` / alpha-to-coverage (the sample mask
is not written by the FS; MSAA itself comes from the EGL config, `EGL_SAMPLES 4`,
and `GL_SAMPLES`/`GL_SAMPLE_BUFFERS` report it), `GL_DITHER`,
`glHint`. No shader binary format is exposed (`glShaderBinary` is
`GL_INVALID_ENUM`); the compressed texture formats are the ETC1/ETC2 set
listed above, any other is `GL_INVALID_ENUM`.
`glCopyTex*Image2D` read the framebuffer back through `glReadPixels` (CPU copy;
`glReadPixels` also converts the BGRA scan-out surface to RGBA and expands RGB565). Disabled
attribute arrays read the `glVertexAttrib*` constant through a zero-stride
vertex fetch. `glPolygonOffset` drives the ISP depth-bias table (`STATE_ISPDBSC`
per object, `ISP_DBIAS_BASE` per kick; case 3 frames 4/5).

EGL (`gl/egl.rs`, `include/EGL/egl.h` + the platform header
`include/EGL/eglplatform.h`): one display (the OSAL; the native display is
the OSAL init argument, NULL = platform default), three configs (RGBA8888,
D24S8, window + pbuffer, ES2; the second adds 4x MSAA, the third is RGB565),
`GL_MAX_CONTEXTS`
(default 2) contexts, up to 4 surfaces and 8 images. A
native window is `struct gles_native_window` (physical address, size,
tightly packed rows, RGBA8888, XRGB8888 or RGB565) that eglCreateWindowSurface maps
for the GPU; pbuffers are render-pool memory. Surfaces are single-buffered:
`eglSwapBuffers` finishes the frame in place. `eglInitialize` brings the OSAL
and backend up (both accept a repeat). `eglGetProcAddress` resolves every
GL ES 2.0 and EGL entry point. Fence syncs are signaled on creation (every
flush completes its render). Contexts: the GL core is a pool of
`GL_MAX_CONTEXTS` contexts (about 400 KB of static state each: object
tables plus the per-context program/shader caches keyed by
`gl_context.index`); `eglMakeCurrent` flushes the previous context's frame
and swaps `gl_ctx`; contexts do not share objects (`share_context` is
`EGL_BAD_MATCH`). Images (`EGL_KHR_image_base`, `EGL_KHR_gl_texture_2D_image`,
`EGL_KHR_image_pixmap` with a `struct gles_native_window` as the "pixmap",
`GL_OES_EGL_image`): an image is a w x h RGBA8888 (or XRGB8888 / RGB565 native)
buffer; from a GL texture it takes over the texture's storage and outlives
the texture, and `glEGLImageTargetTexture2DOES` /
`glEGLImageTargetRenderbufferStorageOES` bind it as external storage in any
context. Not provided: pixmap surfaces (case 15: pbuffer path, 16: MSAA,
17: two contexts, 18: images; `gl_panel=1` uses a window surface over the
LVDS scan-out buffer).

FBO/renderbuffer semantics (`gl/framebuffer.rs`): rendering into a texture writes
the texture's linear RGBA8888 store directly (the PBE and TPU agree on the layout),
so no copy is needed and the texture samples correctly afterwards; the backend
starts a new kick when the target changes, which orders the passes. Depth/stencil
renderbuffers own no memory themselves; the backend keeps a D24S8 buffer for
what an FBO binds (per renderbuffer pair, or per object on the PowerVR with
`pvr_split_zs`; the GC7000 follows each object's depth and stencil plane by
plane, `vivante/ds_cache.rs`). Those contents are GL's: a renderbuffer's image
persists across framebuffer switches (ES 2.0 4.4.3), so a backend never gives
a buffer that holds them to another target (`src/ds_retention.rs`). The GL
core tells it when they are gone (`backend::attachment_gone`, in the form of
`GlesDraw::depth_id`): when the object is freed (`free_renderbuffer`,
`free_texture`, the share group's end), when its storage is specified again
(`context::respecified`), and when a surface is destroyed. A whole clear with
no draw after it leaves one value, which the backend keeps without a buffer.
Each backend keeps `ds_retention::DS_SLOTS` (32) buffers, so that GPU memory,
not the slot count, is what runs out. With every one holding contents, or no
memory for the buffer, a draw that needs another is `GL_OUT_OF_MEMORY` with one
line naming it (gl_test 162); a buffer that held nothing given to another
target is counted on the exit line ("ds given up"). `glReadPixels` accepts only
RGBA/UNSIGNED_BYTE (`GL_IMPLEMENTATION_COLOR_READ_*`) and flushes the frame first.

Stack budget: the GL core runs on kernel/RTOS thread stacks (16 KiB in the Linux
harness, less on Zephyr). `glTexImage2D` converts through a 256-byte chunk buffer;
the `GlesDraw` records (~1 KB each) are static state in `gl/draw.rs`, not stack, and
the backend keeps its PPP/PDS scratch (~2 KB) in its static state too (`Scratch` in
`pvr/backend.rs`). Keep new frames small.

## Conformance: the version reported, and what dEQP still finds

The library reports **OpenGL ES 3.0** by default, because Qt's RHI takes the
context version from `glGetString(GL_VERSION)` and turns on the features Qt
Quick 3D needs at 3.x. A conformance build reports **ES 2.0** instead: the
crate feature `report-es2` (CMake `-DYAKOGL_REPORT_ES2=ON`, Kconfig
`CONFIG_YAKOGL_REPORT_ES2`, `make REPORT_ES2=1`), read through
`crate::REPORT_ES2`. The version is not a label on its own, so the whole
answer moves with it:

| | default | `report-es2` |
|---|---|---|
| `GL_VERSION` | `OpenGL ES 3.0 YakoGL` | `OpenGL ES 2.0 YakoGL` |
| `GL_SHADING_LANGUAGE_VERSION` | `OpenGL ES GLSL ES 3.00` | `OpenGL ES GLSL ES 1.00` |
| `GL_MAJOR_VERSION`, `GL_MINOR_VERSION`, `GL_NUM_EXTENSIONS` | 3, 0, the count | `GL_INVALID_ENUM` |
| `glUniformMatrix*fv(transpose = GL_TRUE)` | transposes | `GL_INVALID_VALUE` |
| `glGenerateMipmap` of an NPOT texture | keeps the base level | `GL_INVALID_OPERATION` |
| an NPOT texture that repeats or mipmaps | sampled | incomplete: (0,0,0,1) |

`GL_EXTENSIONS` is the same either way and lists only what is really here:
EGL images, the ETC1/ETC2 formats the texture path decodes, vertex array
objects, `GL_OES_fbo_render_mipmap` (a level attachment renders into that
level, which is how Qt prefilters a light probe), and
`GL_EXT_texture_format_BGRA8888`.

BGRA is a format at the API surface only. `glTexImage2D`, `glTexSubImage2D`
and `glCopyTexImage2D` take `GL_BGRA_EXT` as format and as unsized internal
format, `convert_row` swaps red and blue once on upload, and the store is
ordinary RGBA8 -- so sampling, rendering into it and reading it back all see
the colour the application meant and know nothing about the external order.
That is also why such a texture is a colour attachment like any other
(`is_color_format`), while renderbuffer storage still refuses it: the
extension adds a texture format, not renderbuffer storage, which GL names
with a sized format. `GL_EXT_read_format_bgra` is a **separate** extension
for `glReadPixels` and is not claimed, so
`GL_IMPLEMENTATION_COLOR_READ_FORMAT` stays `GL_RGBA`, which GL allows.

`tests/negative_api` runs dEQP-GLES2's 246 `functional.negative_api` cases
against this build on a host (`tests/README.md`); those check only which error
each bad call raises, so they need no GPU. **222 of 246 pass** with
`report-es2` (221 without). All 24 that remain build a program with
`glu::ShaderProgram` first: without a run-time shader compiler `glUseProgram`
leaves `GL_INVALID_OPERATION` pending and every later check in the case reads
that instead of its own answer. They are waiting on the compiler, not on an
error enum.

The `state_query` group needs no GPU either, and was read against this core
case by case. Five defects came out of it, all fixed, each with a test naming
the case:

- the stencil masks widened as SIGNED through `glGetFloatv` /
  `glGetInteger64v`, so the all-ones default read as -1.0;
- `GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE` answered 0 instead of the
  attached face, which the attachment had recorded correctly all along;
- `GL_VALIDATE_STATUS` mirrored the link status, so a program that had never
  been validated said GL_TRUE. It is the answer of a command that was run;
- `GL_ACTIVE_UNIFORM_MAX_LENGTH` and `GL_ACTIVE_ATTRIBUTE_MAX_LENGTH` answered
  buffer capacities rather than the longest name the program has.

Everything else that group asks for was already right: no pname is missing,
wrongly defaulted, wrongly counted, or wrongly converted between the boolean,
integer and float views. The cases still out of reach all need a program that
compiles and links, like the negative_api remainder.

### What is not ES 3.0 conformant

The default build answers ES 3.0 to `GL_VERSION`, and EGL lets an
application create an ES 3 context (`EGL_RENDERABLE_TYPE` has
`EGL_OPENGL_ES3_BIT`). It does not claim that such a context is conformant:
`EGL_CONFORMANT` is `EGL_OPENGL_ES2_BIT` only (EGL 1.5 3.4: the mask says
whether a context "will pass the required conformance tests for that API").
Until 2026-09-29 it named ES 3 as well (review R-13). What an ES 3.0
context lacks, each checked in the code:

- `glBlitFramebuffer` is always `GL_INVALID_OPERATION` (`gl/framebuffer.rs`).
- Transform feedback: every `GL_MAX_TRANSFORM_FEEDBACK_*` is 0, ES 3.0
  table 6.34 asks for at least 64 / 4 / 4 ("Transform feedback" below).
- `GL_MAX_SAMPLES` is 1; ES 3.0 table 6.35 asks for at least 4.
- A depth or stencil attachment of another size than the colour ones is
  `GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS` in both builds (`check_status`);
  ES 3.0 4.4.4.2 says that token cannot be generated. Only the colour
  attachments follow the ES 3.0 rule.
- A draw whose draw buffer 0 is `GL_NONE` while a later one is written is
  refused by every backend, and a framebuffer object with no colour
  attachment 0 is `GL_FRAMEBUFFER_UNSUPPORTED` ("Draw buffers" below).

## Clipping

A draw is clipped against the near and far planes. GL ES 2.0 2.11 clips
against all six planes of the view volume, and ES 2.0 has no depth clamp --
that is `GL_EXT_depth_clamp`, which this library does not advertise -- so
there is no conformant configuration in which the clipper is off. The
PowerVR backend's TA clip mode was `NONE` on every draw, because selecting
FRONT_REAR was a bring-up option and every option defaults to 0; the option
now reads `PVR_BACKEND_OPT_NO_DEPTH_CLIP`, so the default (0, clip) is the
conformant one and a rig that wants the raw behaviour asks for it.

What is checked on the host stops at the command stream. What the pixels end
up as is `driver/tools/pvrlab/gl_test.c`, in this repository since the merge,
and the four cases these changes need are there: **58 clearscis** (a scissored
clear covers the box and nothing else, and lands after a draw that came before
it), **59 clearmask** (a fully masked colour or depth clear does nothing; a
stencil clear changes only the bits the write mask allows), **60 wrapmode**
(the three wrap modes, three probes per mode per axis) and **61 texincomp**
(an incomplete texture, and a sampler with nothing bound, read (0,0,0,1)).
They are in the regression set, case 7. **None has run on silicon yet.**

## Header gaps

1. *(resolved)* `struct shc_shader.uniforms[]` (`struct shc_uniform`: name, block
   dword offset, dwords, GL type, array length, sampler binding) is generated by
   `yakogl-tools glsl2usc` from the GLSL `layout(push_constant)` block text (std430)
   and the sampler declarations. `gl/program.rs` builds one location table per
   program from both stages at link time (`glGetActiveUniform` works).
2. *(resolved)* `struct shc_attrib` carries `name` and `gl_type` (from the GLSL
   `layout(location) in` declarations), so `glGetAttribLocation`,
   `glBindAttribLocation` (remap, see above) and `glGetActiveAttrib` work.
3. *(partly resolved)* `gles_draw.clear_depth` carries the depth clear; a stencil
   clear value is still missing.
4. *(resolved)* blend and per-channel colour masks live in the FS variant;
   stencil ops/masks (both faces, `front_cw`) and `color_write` (all-off) are
   in `gles_draw`. Still missing there: depth range.
5. *(resolved)* `bound_texture_2d[GL_CAP_COMBINED_TEXTURE_UNITS]` is per unit; each
   fragment-shader sampler binding reads the unit its `glUniform1i` named
   (`gl_program.fs_sampler_unit[]`), and the backend DMAs one 20-dword
   descriptor per binding (a black 1x1 for bindings without a texture).
6. *(resolved 2026-09-24)* **Macro collision:** `gl_context.h` defined
   `GL_MAX_VERTEX_ATTRIBS` as the array size `16`, shadowing the Khronos query
   token `0x8869`, and `gl_internal.h` `#undef`ed the token to make room. Every C
   file including `gl_internal.h` therefore sent pname `0x10` for
   `glGetIntegerv(GL_MAX_VERTEX_ATTRIBS)`: gl_test's `reset_state` got
   GL_INVALID_ENUM and 0 attributes, disabled none, and case 22 read a previous
   case's attribute 0 as enabled on the default VAO. The cap is now
   `GL_CAP_VERTEX_ATTRIBS`, like the other `GL_CAP_*` sizes, and the token is the
   Khronos one everywhere.

## Colour attachments and the pixel output registers

A pixel has **four** output registers on this core (BXS-4-64 has no
`eight_output_registers`), and a 64-bit source -- an RGBA16F attachment, which
takes two -- must start at an even one. So a fragment shader that writes three
RGBA16F attachments (Qt Quick 3D's `userpasses` G-buffer) wants six registers
and does not fit.

The compiler (`tools/pco_shc/pco_shc.c`) lays the outputs out with that budget
and puts the ones that do not fit in **tile buffers**: the shader stores them to
memory, addressed from two shared registers per buffer (the table records
`num_tile_buffers` and `tile_buffer_sh`), and the driver's end-of-tile program
loads each one back into the output registers before emitting that attachment
(`pvr_usc_eot_tb_n_words[]`, the buffer address patched into the program at
upload). `src/pvr/backend.rs` writes the addresses into the shared registers
with the draw's last DMA kick -- after the uniform block, which a shader with a
trimmed block would otherwise overwrite.

## Draw buffers

`glDrawBuffers` is ES 3.0 (4.2.1); ES 2.0 has no such entry point and this
library does not advertise `GL_EXT_draw_buffers`, so under `report-es2` the
entry keeps its ES 3.0 rules for whoever resolves it anyway.

- Its errors, in dEQP-GLES3's order: n < 0, then n > 0 with a NULL array
  (`GL_INVALID_VALUE`: ES 3.0 names no error for it, but there is nothing to
  carry out; it was accepted until 2026-09-29, review R-22), then an enum
  that is no draw buffer, then the window given anything but one `GL_BACK`
  or `GL_NONE`, then n past `GL_MAX_DRAW_BUFFERS`, then buffer i other than
  `GL_COLOR_ATTACHMENTi` or `GL_NONE`. A refused call changes nothing.
- The default framebuffer keeps its draw buffer per context
  (`framebuffer::DEFAULT_DRAW_NONE`, outside `GlContext` so its C view does
  not change). `GL_NONE` writes and clears no colour: the draw and the
  clear take a colour mask of zero, so depth and stencil still happen.
- On a framebuffer object the whole table is recorded, the buffers past n
  set to `GL_NONE`; n = 0 sets every one to `GL_NONE`. `num_draw_buf` 0
  means glDrawBuffers was never called on the object, and then output i
  writes attachment i (gl_test 55 es3mrt draws two attachments that way). ES 3.0
  says the initial value past buffer 0 is `GL_NONE`; that difference is
  kept, and `GL_DRAW_BUFFERi` answers what is written.
- A draw's colour targets are built from the table (`draw::resolve_targets`):
  slot i is draw buffer i, which writes attachment i or nothing (ES 3.0 makes
  buffer i `GL_COLOR_ATTACHMENTi` or `GL_NONE`, so a fragment output keeps its
  location and a `GL_NONE` slot keeps its place with `gpu` 0). Slot 0 was
  always attachment 0 until 2026-09-29 (review R-09): with
  `{GL_NONE, GL_COLOR_ATTACHMENT1}` output 0 overwrote attachment 0. Now
  slot 0 gets `gpu` 0 there, while `target_gpu` stays attachment 0, the
  frame's own target. All three backends always store slot 0 (the PowerVR's
  end-of-tile emit, the V3D's RT 0, the GC7000's hardware target 0), so each
  refuses such a draw by name (`draw buffer 0 is GL_NONE while a later draw
  buffer is written`, -ENOTSUP, so `GL_INVALID_OPERATION`); none draws into
  the wrong image. When no draw buffer writes at all, the draw and the clear
  take a zero colour mask instead, which every backend honours.
- Colour attachments of different sizes: `report-es2` makes such a
  framebuffer `GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS` (ES 2.0.25 4.4.5, "All
  attached images have the same width and height"). ES 3.0 4.4.4.2 cannot
  generate that token, and 4.4.1 defines rendering "only within the largest
  area that can fit in all of the attachments", so the default build
  calls it complete and each draw target carries its own size. The PowerVR
  clips every PBE target to its own size; the V3D refuses unequal sizes by
  name; the GC7000's MRT path (behind `gc_debug_mrt`) clips once, to RT 0
  (`clip_rect`), so it refuses a later target smaller than RT 0 by name (`a
  colour attachment past the first is smaller than the first`, review R-02)
  and draws one at least as large. Its single SE scissor could clip to the
  intersection; that is not done.

## One descriptor per sampler the shader declares

The compiler packs a 20-dword combined image+sampler descriptor per **used**
sampler at sh0.., in binding order, and puts the shader's uniform block right
after them. The descriptor block the backend DMAs must therefore have exactly
that many slots: a short block leaves the shader reading image state out of its
own uniforms, which is a texture fetch from a nonsense address and a GPU fault
(a BIF read by the TCU). Qt Quick 3D material shaders declare a dozen samplers
and more, so `GLES_MAX_TEX_BINDS` is 16. The vertex stage has descriptors of
its own (`GLES_MAX_VS_TEX_BINDS`, 16 slots in `GlesDraw`; the units a GPU
takes are `GLES_VS_TEX_UNITS`: 16 on every GPU, from Mesa's v3d, pvr and
etnaviv; the GC7000's was 8 until 2026-10-01 and `gc_debug_vs_units16` = 0 still
selects it, and what a process answers is `backend::vs_tex_units()`): it lays its shared registers out the same way, descriptors
first and the uniform block behind them.

The link counts each stage's active samplers (one per sampler and per element
of a sampler array, `ShcShader::sampler_bindings`, the same list the draw wires)
against `GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS` (`GLES_VS_TEX_UNITS`),
`GL_MAX_TEXTURE_IMAGE_UNITS` and `GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS` (the sum
of the two stages: ES 2.0.25 2.10.5 counts a unit both stages use twice), and
fails with "link error: the vertex shader uses N active samplers; this
implementation exposes M (GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS)" or its fragment
or combined twin (`program.rs` `check_sampler_counts`, ES 2.0.25 2.10.4). The
draw used to wire the first `GLES_VS_TEX_UNITS` and leave the rest unwired; on
the AM62P a 16-sampler vertex shader hung the GPU.

The texture units themselves are GL state, and there are 32 of them on every
GPU (`CAP_COMBINED_TEXTURE_UNITS`: glActiveTexture's range, a sampler
uniform's range, the per-unit binding arrays, and
`GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS`, ES 3.0's minimum). The draw gives a
backend compacted per-stage bindings (`GlesTextureBind::unit` is the sampler
binding, not the GL unit), so no backend table is indexed by a unit. A
fragment shader samples at most `CAP_TEXTURE_UNITS` (16,
`GL_MAX_TEXTURE_IMAGE_UNITS`, one binding per slot of `GLES_MAX_TEX_BINDS`):

| GPU     | vertex (`GLES_VS_TEX_UNITS`) | fragment | combined |
|---------|------------------------------|----------|----------|
| V3D     | 16                           | 16       | 32       |
| PowerVR | 16                           | 16       | 32       |
| GC7000  | 16 (8 with `gc_debug_vs_units16` = 0) | 16 | 32  |

So the combined rule of the link never refuses a program the two stage rules
accepted; it stays because the spec states it. The combined limit was 16 (the
same constant as the fragment one) until 2026-09-29, and dEQP-GLES2
`uniform_api` refused 8 cases that need 18, 20 or 28 combined units.

The other per-program limits are checked at link too, each against the one
constant its `glGetIntegerv` query answers with (`gl/context.rs`):
`GL_MAX_VERTEX_UNIFORM_VECTORS` and `GL_MAX_FRAGMENT_UNIFORM_VECTORS` (384
each, `MAX_UNIFORM_DWORDS / 4`) by the GLSL ES 1.00 Appendix A packing of
each stage's uniforms (`program.rs` `check_uniform_packing`),
`GL_MAX_VARYING_VECTORS` (8; 15 on the GC7000) by the same packing of the
varyings, in the shader back end's link, and `GL_MAX_VERTEX_ATTRIBS` (16) in
whole locations by the attribute layout. The algorithm is `shaderc/packing.rs`;
`src/shaderc/SHADERC_NOTES.md` ("The resource budget") says which back end
feeds which class and what is not covered.

## Object names past the pools

ES 2.0 lets `glBind*` create an object under any name the application picks,
generated or not (3.7.13, 4.4.1). The objects live in fixed pools indexed by
name (`MAX_TEXTURES` 512, `MAX_BUFFERS` 512, `MAX_RENDERBUFFERS` 64,
`MAX_FRAMEBUFFERS` 256). dEQP-GLES2 `lifetime.bind_no_gen.{buffer,framebuffer,
renderbuffer,texture}` bind a random `uint32` and require `GL_NO_ERROR` and
`glIs*` true afterwards (`glsLifetimeTests.cpp` `testBindNoGen`). Until
`gl/names.rs`, a name at or past the pool was refused with `GL_INVALID_VALUE`,
and all four failed for that alone.

`gl/names.rs` translates at the API boundary only. Everything inside the core
still stores and compares table slots.

- A name at or past the pool (a "far name") borrows a free slot, taken from
  the top of the pool down, so glGen* still hands out the names it would hand
  out without it. A table of `FAR_NAMES` (16) entries per type records
  (name, slot): per share group for buffers, textures and renderbuffers, per
  context slot for framebuffers. They are Rust-only statics, so the C view of
  `GlObjects` does not change. Cost: 16 x 8 bytes per table.
- In: bind, delete, glIs*, the attach calls, glBindBufferBase/Range, the EGL
  image sources and the C `gl_lookup_*` go through `names::slot_of` (or
  `names::bind_slot` for a bind). Out: the binding queries,
  `VERTEX_ATTRIB_ARRAY_BUFFER_BINDING`, the indexed `UNIFORM_BUFFER_BINDING`
  and the attachment's `OBJECT_NAME` go through `names::name_of`.
- Refusals, all `GL_OUT_OF_MEMORY` with a log line: a 17th far name of one
  type; no free slot; a small name whose slot a far name holds (it cannot get
  its own object while the far name lives). The last one never happens to
  generated names (their slots are `used`), only to an application that binds
  both small ungenerated names and far ones.
- A deleted far name that a framebuffer still holds (the orphan rule) is
  known by its slot number afterwards; an in-pool orphan is known by its old
  name, which is the same number.
- An earlier plan (this section until the change) kept the application's
  name in every stored binding and resolved it at every table index, about
  465 sites. The boundary form touches about 45 and leaves the draw path
  alone.

## Transform feedback (design, not built)

Today only the names and the binding exist (`gl/es3.rs`): every
`MAX_TRANSFORM_FEEDBACK_*` is 0, `glBeginTransformFeedback` is
`GL_INVALID_OPERATION`, an INTERLEAVED varyings request fails the next link.
What that costs, from the GLES3 host run at 01de3104: negative_api 16,
state_query 20, the whole `transform_feedback` group (882 cases, only 2
refused at compile, so they would run), and `functional.lifetime`, whose
init needs a transform feedback object to work.

Neither back end has a capture path. The PowerVR's USC writes vertex outputs
to the parameter buffer only (Mesa's pvr driver does not expose transform
feedback either), and the GC7000's stream-out is undocumented here. So the
plan is to capture in the GL core, on the CPU, and leave rasterisation as it is:

- **State first, no capture**: transform feedback objects with their indexed
  `GL_TRANSFORM_FEEDBACK_BUFFER` bindings (`glBindBufferBase/Range` on that
  target), active/paused, the primitive mode, the varyings recorded at
  `glTransformFeedbackVaryings` and resolved at link against the vertex
  stage's reflected outputs, `glGetTransformFeedbackVarying`, the limits
  (4 separate components x 4, 64 interleaved), and every ES 3.0 error of
  2.15.1-2. That alone moves the negative_api and state_query cases and
  unblocks the lifetime group's init.
- **Capture**: at a draw with transform feedback active, the GL core runs the
  vertex stage's IR on the CPU for the draw's vertices (an evaluator like
  `shaderc/online/eval.rs`, today test-only and `std`, made `no_std` over the
  host heap), assembles primitives in the begin mode (strips and fans unrolled,
  as 2.15.2 says), and writes the captured varyings into the bound buffers
  at their offsets; the `TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN` and
  `PRIMITIVES_GENERATED` queries count them. The draw then goes to the GPU
  as usual, or not at all under `GL_RASTERIZER_DISCARD` (already recorded).
- What the evaluator must add for that: textures in the vertex stage (it
  refuses them), indexed locals, and speed -- a CPU pass per captured vertex
  is fine for dEQP's few hundred vertices and wrong for a real workload, which
  is why capture stays off the draw path when no feedback is active.
- The captured values are the evaluator's, not the GPU's: a precision
  difference between the two would show as a dEQP transform_feedback
  mismatch. The front-end oracles already compare the evaluator with the
  reference (values 0 wrong of 13,953, GLES3), which is the reason to think
  it is close enough; it is unmeasured against either GPU.

## Design invariants

- **Single current context** (`gl_ctx`), static storage (the context pool lives in
  `.bss`); no malloc for the context or object tables. The host heap (the platform's
  `malloc`/`free`, declared in `src/util.rs`) is used only for `glShaderSource`'s
  source copy and draw-time staging in `gl/draw.rs`. Object handles are 1-based
  indices; handle 0 is the reserved/invalid GL name.
- **GL error semantics:** `GlContext::set_error` (C: `gl_set_error_at`) keeps the first error; `glGetError`
  returns and clears it. Bad handles and unbound-program-on-draw →
  `GL_INVALID_OPERATION`.
- The C core was smoke-tested end-to-end against stub OSAL/shaderc/backend (clear
  fusion, attribute resolution, uniforms, textures, indexed + client-side draws, cull
  mapping, error paths) and run on silicon. The Rust core is checked on the host only,
  two ways. **Unit tests on strict mocks** live next to the code and run with
  `cargo test --no-default-features --features host-test,gpu-powervr` (1350 crate tests plus the 6
  in `tests/`: SHA-256 known answers and the shader table's invariants; none ignored).
  The mocks in `src/testutil.rs` model a non-coherent platform and fail the test on a
  double free, a cache range outside its allocation, a read of memory the library never
  wrote, a draw whose inputs change before its kick completes, and they can withhold the
  CPU view; read that module's documentation before writing a test. **The host
  differential harness** (`tools/hostharness/`, whose README lists what is and is not
  compared) runs gl_test.c, API sweeps and hostile sequences over a recording OSAL (all
  17 operations, GPU memory hashes, decoded kicks) and an interposer on all 222 exported
  entry points, in twelve platform modes, dev and release, with AddressSanitizer on the
  library itself; `compare.sh` repeats the check for any two builds. Nothing is
  rendered, and the Rust library has not been run on silicon.
- **The core is no longer bug-compatible with the C.** It follows upstream C `a444fe9`,
  and a fix round (`git log 669279f..e235fab`) deliberately diverges where the C is
  wrong: a `glIs*` name that nothing ever bound is not an object (ES 3.0 6.1.10; upstream
  `4490ced` adopted this and the driver's `gl_test` case 22 now expects it); immutable storage after `glTexStorage*`;
  `glTexParameteri` value validation; `glReadPixels` distinguishing `INVALID_OPERATION`
  from `INVALID_ENUM`; the ES 3.0 buffer-mapping rules and query pnames; `glUniform*v`
  size and count checks and `glGetUniform*v` conversions; deferred `glDeleteProgram` /
  `glDeleteShader`; `glClear` rejecting undefined bits; `glViewport` clamped to
  `GL_MAX_VIEWPORT_DIMS`; `GL_MIN`/`GL_MAX` blend equations; the ES 3.0 vertex formats;
  a client-array pointer while a VAO is bound being `INVALID_OPERATION`. It also fixes
  memory-safety defects the C shares (uniform blocks sized to the reflected layout and
  not only to `uniform_dwords`, EGLImage storage with an owner, allocations sized at the
  pitch the GPU writes, checked application offsets, attachment format checks, fresh
  storage cleared before the application can see it) and reports draws the backend
  cannot record instead of dropping them. Every one of those defects is still open
  upstream. Application pointers are never turned into typed Rust slices or references:
  the ABI does not guarantee their alignment, so they are read and written unaligned or
  copied as bytes.

## Rust conventions

- **Single-threaded by contract.** One thread issues every GL and EGL call of the
  process, as with the C library; it may make any number of contexts current in turn.
  The EGL error, bound API and current context are process-wide, not per thread (no
  TLS in `no_std`), and nothing locks the GL tables for a second thread. Where the
  OSAL names threads (`thread_id`), the thread that makes a context current owns the
  library until it releases it, and a call from another thread is refused before it
  touches state (`context::foreign_thread`: `current()` answers None to it,
  `egl_entry!` returns the EGL failure value, its `glGetError` / `eglGetError` give
  `GL_INVALID_OPERATION` / `EGL_BAD_ACCESS`, one log line). The check is one atomic
  load while nothing is current and one `thread_id` call per entry while a context
  is (about 3 ns per call on the host). The online compiler's re-entrancy flag is a
  separate guard. Global state lives in `util::Global<T>` cells (an `UnsafeCell`
  marked `Sync` on that contract; `Global::zeroed()` places the value in `.bss` and
  is used only with types whose all-zero pattern is valid) instead of scattered
  `static mut`.
- **The context is taken once.** An entry point calls `context::current()` once and
  passes `&mut GlContext` down; helpers never fetch it again, so no two live
  references to the context exist. Object lookups are methods
  (`GlContext::buffer(h)` etc. return `None` for 0 / out of range / free); errors are
  recorded with `c.set_error(GL_INVALID_..., "glEntryPoint")`, which also names the
  entry point in the log.
- **No `alloc`, no global allocator.** The crate is `no_std` with no dependencies
  and links into C and Rust applications alike; the few variable-sized host
  allocations go through the platform's `malloc`/`free` (`util::malloc`, `util::CBuf`).
- **C ABI.** Everything a C caller can see is `extern "C"` / `#[repr(C)]` /
  `#[no_mangle]` and mirrors the headers. C enums that cross the ABI are integer
  aliases with constants (`pub type GlesCmp = u32;`), so any value a C caller passes
  is representable. The layout of every shared `#[repr(C)]` struct (26 structs, 365
  fields: size and each offset) is pinned on both sides by `src/abi_layout.rs` and
  `src/abi_layout_check.c`, generated by `yakogl-tools abi-layout <driver src dir>`;
  a change to a header or a Rust struct that is not mirrored fails the build. Re-run
  the tool when a shared struct changes.
- **Logging** goes through `glog!("fmt {}", args)`: the line is formatted with
  `core::fmt` into a 256-byte buffer (longer lines are truncated) and handed to the
  OSAL as `g_osal->log("%s", line)`; the variadic `log` is never called with any
  other format from Rust.
