<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL design

YakoGL (夜光貝 yakogai, the shell cut for 螺鈿 raden; "yet another kernel-optional GL")
is a **source-compatible** OpenGL ES implementation that links as **`libyakogl.a`**.
Apps compile against the standard Khronos `<GLES2/gl2.h>` and link this archive.
It started on the PowerVR BXS-4-64 (Rogue, BVNC 36.53.104.796; TI AM62P / Toradex
Verdin) and now also drives the Vivante GC7000UL (i.MX 8M Plus) and the Broadcom V3D 4.2
(Raspberry Pi 4). Some sections below describe the PowerVR first, as it was built.

## Goals / non-goals

- **Source compatibility first.** Standard Khronos headers; the documented GL ES 2.0
  entry points behave per spec for the subset we implement. Apps need no source changes.
- **Portability.** The GL core and the PowerVR backend are OS-agnostic Rust (one
  `no_std` crate with no dependencies, exporting the C ABI; see "The Rust crate"
  below). All OS/platform coupling is behind one interface (`gles_osal.h`). Targets, in priority order:
  **Zephyr** (primary — this is the SafeUI platform), **bare-metal**, then other OSes
  (**Linux**, **PX5**). One `.a` of portable core + one small OSAL `.o` (C) per platform.
- **GLES3-ready.** The API dispatch, state object model, and backend are structured so
  GLES3 (VAOs, UBOs, MRT, instancing, transform feedback, 3D/array textures, sampler
  objects, integer/GLSL-ES-3.00 shaders) can be added without reworking the core. Ship
  GLES2 first; add `libGLES3.a` (superset) later.
- **Non-goals (for now):** full conformance (CTS), EGL window-system integration beyond a
  minimal surface/context, GLSL-ES features the PCO backend can't yet lower.

## Layering

```
  app  ──#include <GLES2/gl2.h>──▶  libyakogl.a
  ┌─────────────────────────────────────────────────────────────┐
  │ 1. API dispatch      gl/*.rs      (gl* entry points, error   │
  │                                    state, arg validation)     │
  │ 2. GL context/state  gl/context.rs (bound objects, caps,      │
  │    + objects         gl/{buffer,texture,shader,program,       │
  │                          framebuffer,vao}.rs)                 │
  │ 3. Draw translation  gl/draw.rs   (current GL state ─▶ one    │
  │                                    resolved draw, backend.rs) │
  │ 4. GPU backend       pvr/*.rs     (VDM/PPP/PDS/frag-cmd gen;  │
  │    (one per build)                 the PVR_* feature work)    │
  │                      vivante/*.rs (GC7000 LOAD_STATE streams) │
  │ 5a. Shader backend   shaderc/*    (GLSL ES ─▶ USC + metadata) │
  │ 5b. OS abstraction   osal/*       (mem, MMU, submit, sync,    │
  │                                    cache, fw, mmio)           │
  └─────────────────────────────────────────────────────────────┘
       5a and 5b are the only pluggable/replaceable layers.
```

Layers 1–4 and 5a are the Rust crate (built by cargo as a static library). Layer 5b
is one small C translation unit per platform, added to the same `libyakogl.a`. Layer 5a
has two designed implementations (see Shaders); the offline one exists.

## The PowerVR backend (layer 4) — already proven on silicon

The hard GPU bring-up is done and silicon-verified (see `[[gpu-gles-coverage]]` /
`docs/`): TA(geom)+3D kick, indexed draws, **multiple primitives & multiple draw calls**,
per-vertex varyings, **texture sampling** (TPU image/sampler state + PDS DOUTD), **uniform
/ constant** (push-const via DOUTD), **scissor** (region clip), **depth test** (ISP), **face
culling** (PPP cullmode). Recipes in hand for **blending** (FS-lowered, passtype
TRANSLUCENT), **render-to-texture** (two ordered kicks, coherent SLC — no flush), **stencil**
(ISPB 2-pass), **multi-varying / perspective** (coeff/DOUTI). These become the backend's
state-driven code paths; the `PVR_*` build knobs collapse into GL-state-driven branches.

Backend interface (draft): `pvr_backend_draw(ctx_state) ` consumes a resolved draw
(vertex buffers, index buffer, VS+FS USC + their PDS programs, render state) and emits the
VDM/PPP/PDS/frag-command into GPU memory, then submits via OSAL. Source moves from
`src/pvr_vdm_ppp_gen.c`, `src/pvr_fw_boot.c` (the render path) into `yakogl/pvr/`.
(As built: the backend is `src/pvr/backend.rs`; the OS-free command generators stayed
in the driver repository and are called through FFI, see "The Rust crate" below.)

## Shaders (layer 5a) — the key design decision

`glShaderSource(GLSL ES)` + `glCompileShader` must turn GLSL text into USC at the API
boundary. GLSL→USC = glslang(GLSL→SPIR-V) → `spirv_to_nir` → PCO backend, which we already
run offline (`pco_shc`, silicon-verified via F1). Two backends behind one interface
(`gles_shaderc.h`: `shc_compile(stage, glsl, &usc, &meta)`):

- **`shaderc_offline` (Zephyr / bare-metal):** the PCO+glslang toolchain is too large to run
  on-target. At build time, app shaders are compiled to USC+metadata blobs and embedded; at
  runtime `glShaderSource` hashes the source and looks it up. Fits the SafeUI model (apps are
  known and signed). A build tool (`glsl2usc`, wrapping `pco_shc`) produces the blob table.
- **`shaderc_remote` (Linux host / dev, the cargo feature `shaderc-remote`):** send the GLSL
  over TCP to `tools/shaderc_server.py` on a machine that has the toolchain, and get the same
  USC+metadata back. This is what makes arbitrary-shader source compatibility real today:
  dEQP-GLES2 writes thousands of shaders as it runs, so nothing can be precompiled for it.
  The server runs the very pipeline that generates the blob table, so a shader compiled at
  run time is identical to the one a build would have embedded (`tests/shaderc_remote.rs`).
- **`shaderc_online` (Linux / PX5 / dev):** link the PCO compiler into the process and compile
  in-process. Not written; `shaderc_remote` covers the same need without putting a compiler
  in the image.

Both emit the same **shader ABI** the backend consumes: USC dwords, temp count, the DOUTI
coefficient layout for varyings (w iterator @cf0..3 then VAR0.., per the toolchain), the
DOUTD shared-reg layout for uniforms (push-const → sh0..), vertex attribute → `vi` mapping,
and the `po0..` outputs. This ABI is already characterised (`scratchpad/shader_toolchain.md`).

## OS abstraction (layer 5b) — `gles_osal.h`

Everything platform-specific is one struct of function pointers (or link-time symbols):
GPU-visible memory alloc + GPU MMU map/unmap; firmware load; MMIO/register access; submit
(KCCB write + doorbell); completion wait (poll/IRQ/fence); cache maintenance. Zephyr impl
uses the existing SafeUI driver plumbing; bare-metal a minimal direct impl; Linux a
`/dev`-backed impl; PX5 its own. The GL core never calls an OS API directly.

One thread issues every GL and EGL call of the process; the core keeps no per-thread
state (no TLS in `no_std`). The OSAL's optional `thread_id` lets the core refuse a
call from a second thread while a context is current (`gles_osal.h`, "threads").

## Repository layout

```
yakogl/
  DESIGN.md                     (this file)
  include/GLES2/{gl2,gl2ext,gl2platform}.h   Khronos (source compat)
  include/GLES3/...             Khronos ES 3.x headers (the ES 3.0 entry points provided)
  include/KHR/khrplatform.h
  include/EGL/{egl,eglplatform}.h  Khronos egl.h + our platform types (native window =
                                a GPU-mappable pixel buffer); src/gl/egl.rs implements it
  include/gles_osal.h           platform interface (portability contract)
  include/gles_shaderc.h        shader-backend interface
  Cargo.toml  src/lib.rs  src/util.rs          the workspace + the crate `yakogl` (no_std rlib)
  capi/                         `yakogl-capi`: the staticlib (panic handler) and its
                                post-processing (assemble.cmake) -> libyakogl.a
  src/gl/{context,buffer,texture,shader,program,state,vertex,vao,draw,framebuffer,egl}.rs
  src/gl/{types,consts,egl_consts}.rs           GL types; tokens generated from include/
                                (yakogl-tools gl-consts)
  src/backend.rs                the backend contract (C view: src/backend.h)
  src/pvr/{backend,blobs,ffi}.rs  PowerVR backend (cargo feature gpu-powervr), its USC blobs, the FFI to the
                                driver's command generators; ffi_layout_check.c pins it
  src/vivante/{backend,shader,texture,cmd,regs,state,kick,clear_shader}.rs
                                the i.MX8MP's GC7000 backend (cargo feature gpu-vivante);
                                VIVANTE_NOTES.md; gc_kick.h is the OSAL's view of the kick
  src/osal/mod.rs               g_osal + osal_set_ops; render.rs the kick descriptor types
  src/osal/{zephyr,zephyr_gc,baremetal,linux}.c the OSALs (C; zephyr_gc is the GC7000's)
  src/shaderc/{mod,offline,link,sha256}.rs      offline shader back end + the shared link rule
  src/shaderc/remote.rs                         the host-compile back end (`shaderc-remote`)
  src/shaderc/{blobs,vivante_blobs}.rs          the generated table of each GPU (one per build)
  src/{gl_internal,gl_context,backend}.h, src/pvr/pvr_backend.h, src/osal/osal_internal.h
                                C view of the crate's #[repr(C)] structs / #[no_mangle] helpers
  tools/yakogl-tools            host tools: glsl2usc (build-time GLSL→USC, wraps pco_shc,
                                emits Rust), usc-blob, femtovg-shaders, gl-consts, abi-layout
  tools/vivante/                the GC7000's offline shader tool and table generator, the
                                and the command-stream diff against Mesa's
  src/abi_layout.rs  src/abi_layout_check.c     generated layout pins (yakogl-tools abi-layout)
  tools/hostharness/            host differential harness (compare.sh, run.sh)
  tests/                        host tests (cargo test --no-default-features --features host-test,gpu-powervr)
  CMakeLists.txt / baremetal.mk / zephyr/       run cargo, assemble libyakogl.a (README "Linking")
```

## Incremental plan (vertical slices, each silicon-tested)

1. **Scaffolding**: Khronos headers, `gles_osal.h`, `gles_shaderc.h`, context skeleton,
   Zephyr OSAL wrapping the proven render path.
2. **Slice A — clear + solid draw**: `glViewport/glClearColor/glClear` + `glDrawArrays` of a
   triangle with a precompiled solid-color program (uses the proven backend). First app
   linking `libyakogl.a` that renders on silicon.
3. **Slice B — attributes + uniforms + varyings**: `glVertexAttribPointer`, `glUniform4f`,
   a GLSL passthrough+interp program (multi-varying backend).
4. **Slice C — textures**: `glTexImage2D` + `glBindTexture` + a sampling program.
5. **Slice D — render state**: `glEnable(DEPTH_TEST/CULL_FACE/SCISSOR_TEST/BLEND)`,
   `glDepthFunc/glCullFace/glScissor/glBlendFunc` mapped onto the backend paths.
6. **Slice E — FBO / render-to-texture**, then stencil, then GLES3 groundwork.

Each slice keeps the standalone `PVR_*` harness working (it stays the silicon test rig)
until the equivalent GL path is verified, so we never lose a known-good reference.

## The backend contract (`src/backend.h`, 2026-09-16)

The GL core reaches the GPU only through `struct yakogl_backend`, a function table
(`init`, `draw`, `flush`, `finish`, `finish_previous`, `kick_id`, `kick_pending`,
`heap_base`, `counters`) and the GPU-agnostic types next to it: `struct gles_draw` (the
fully resolved draw), `gles_attrib`, `gles_texture_bind`, the `GLES_PRIM_/CMP_/CULL_/SOP_`
enums, `gles_texfmt`, `gles_surf_format` and the three heaps the core allocates in
(`GLES_HEAP_DATA`, `GLES_HEAP_SHADER`, `GLES_HEAP_PROGRAM`). The contract is defined in
`src/backend.rs` (`YakoglBackend`, `GlesDraw`, ...; `#[repr(C)]`, the C enums as `u32`
aliases) and `src/backend.h` is its C view. `src/backend.rs` also names the
backend of the build (`g_backend`, the PowerVR table `yakogl_backend_pvr` from
`src/pvr/backend.rs`). Everything PowerVR-specific (the options, the debug hooks,
`pvr_backend_counters`) stays in `src/pvr/pvr_backend.h`; only the on-silicon test
suite and the samples' diagnostics include it. A second GPU adds `src/<gpu>/` with its
own table and a shader back end (`docs/imx8mp-backend-plan.md`).

## The Rust crate (2026-09)

The GL/EGL core, the PowerVR backend and the offline shader back end were rewritten from
C to Rust: the crate `yakogl` at the repository root (`no_std`, no dependencies,
an `rlib` without a panic handler; exactly one of `gpu-powervr` | `gpu-vivante` |
`gpu-v3d` picks the GPU, `osal-linux` | `osal-zephyr` | `osal-baremetal` pick the
compile-time default of `g_osal`, `host-test` for host tests only). The static library for C is the crate `yakogl-capi` (capi/), which owns the
panic handler. The C ABI is the product and did not
change: `gl*`, `egl*`, `g_osal`, `g_backend`, `g_shaderc` and the structs in `include/`
are `extern "C"` / `#[repr(C)]`.

What deliberately stays C:

- the three OSALs `src/osal/{zephyr,baremetal,linux}.c`: thin glue over C APIs (Zephyr
  macros and inlines, board hook pointers, a variadic log), none of which stable Rust can
  define;
- the driver's OS-free command generators in the `powervr-bxs-am62p` repository
  (`pvr_vdm_ppp_gen.c`, `pvr_pds_render_gen.c`, `pvr_pds_pixel_event_gen.c`), shared
  with the driver's own render path and called through the FFI declared in
  `src/pvr/ffi.rs`;
- `src/pvr/ffi_layout_check.c`, which has no code, only `_Static_assert`s: it pins the
  driver structs, constants and prototypes that `ffi.rs` mirrors, so a driver header
  change fails the build. `src/abi_layout_check.c` (generated, with its Rust twin
  `src/abi_layout.rs`, by `yakogl-tools abi-layout <driver src dir>`) does the same
  for the 26 `#[repr(C)]` structs (365 fields) the C headers share with the crate:
  size and every offset are asserted on both sides, so header/Rust drift fails the
  build; re-run the tool when a shared struct changes;
- the headers `src/gl_internal.h`, `src/gl_context.h`, `src/backend.h`,
  `src/pvr/pvr_backend.h`, `src/osal/osal_internal.h`: the C view of the crate's
  `#[repr(C)]` structs and `#[no_mangle]` helpers, used by the on-silicon test suite
  (`tools/pvrlab/gl_test.c` in the driver repository) and the samples. They must be
  kept in step with the Rust structs (enforced by the layout pins above).

Builds: CMake, `baremetal.mk` and the Zephyr module each run cargo for `yakogl-capi`,
make its Rust runtime and compiler_builtins libc/libm symbols local
(`capi/assemble.cmake`) and compile the C objects. CMake and `baremetal.mk` append the
C objects, so `libyakogl.a` is still the single archive; Zephyr links the Rust archive
next to the `zephyr_library` of the C OSAL. With `YAKOGL_RUST_LINKAGE=external` only
the C parts are built and a Rust application links the `yakogl` crate itself. The
default cross target is `aarch64-unknown-none` (hard float: `CONFIG_FPU` /
`CONFIG_FPU_SHARING` on Zephyr). README.md, "Building" and "Linking", has the details.

Where the port stands against the C: it follows upstream C **55de113** with the driver
**0e1d975**, and it is **not** bug-compatible with that C any more. A fix round of 32
commits (`git log 669279f..e235fab`) diverges from the C wherever the C is wrong —
memory-safety defects reachable through the public API (uniform blocks sized to
`uniform_dwords` while the reflection reaches past them, EGLImage storage with no owner,
allocations sized `w * h * bpp` while the GPU works at the padded pitch, vertex and index
ranges checked with wrapping arithmetic or not at all, any texture accepted as an RGBA8
colour attachment), silent data loss (unrecordable draws dropped without a word, GPU pool
memory handed out with the previous user's contents) and ES 3.0 / EGL 1.5 conformance
gaps. Each divergence is named in the commit that makes it and pinned by a regression
test. Upstream has since adopted seven of them (the sync to `a444fe9`: the uniform
arrays, the EGLImage's storage lifetime, the GPU's pitch, the vertex and index ranges,
an unrecordable draw, cleared storage and `glIs*`) and, with the sync to `bfab616`, most
of the ES 3.0 conformance gaps as well: upstream's own dEQP round added the whole ES 3.0
entry point surface, the default texture objects, the eight buffer binding points and the
error state EGL 1.5 3.1 asks for. The attachment-format check and the full per-frame
tables are still open in upstream C. The sync to `bfab616` added two divergences of its
own (the range check of an in-place vertex fetch, and the alpha of an RGBA8888 target
read back from a pixel engine that stores it B,G,R,A); the sync to `55de113` adds none
in the library, and three in the tools and the notes: two harness modes renumbered off
the numbers the new texture modes took, the table checker's upstream ref made
configurable, and comments this round made false. The claim is therefore
byte-identical to the C library *up to those divergences*, and identical for the GPU
command stream wherever no fix applies — not byte-identical outright.

Verification, all of it on the host: unit tests on the strict mocks of `src/testutil.rs`
(`cargo test --no-default-features --features host-test,gpu-powervr`: 1427 crate tests plus 6
integration tests, none ignored, and 1341 in the `gpu-vivante` build; `cargo test -p
yakogl-tools`: 34), and the host
differential harness (`tools/hostharness/`, whose README lists what is and is not
compared): gl_test.c, API sweeps and hostile sequences over a recording OSAL (all 17
operations, GPU memory hashes, decoded kicks) and an interposer on all 222 exported entry
points (return values, output parameters, read-backs), in twelve platform modes, dev and
release, with AddressSanitizer on the libraries themselves. Against the C at `c1e6084`
the pre-fix port was identical in the release profile over all 1344 runs but the 12 where
the C aborts on an ASan heap-buffer-overflow read in `glGetUniformfv`; the fix round then
took that build's 14 ASan reports, 14 panics and 8 `HH-ERROR` runs to zero. See
README.md, "Status", for the current report numbers. Nothing is rendered, and **the Rust
library's PowerVR half has run on silicon**: built as a Zephyr module for the
Verdin AM62P and run through `tools/pvrlab/gl_test.c` case 7 on 2026-09-22, it
passes 34 of 34 cases with no fence timeout, where upstream C `55de113` on the
same board and driver passes 31 and fails 3. README.md, "On silicon", has the
conditions that run needs. Open items: the driver repository's Linux
kernel-module harness (`tools/pvrlab`) compiled YakoGL's `.c` files directly through
symlinks and Kbuild and has to link the cargo-built static library instead (not done);
the GC7000 backend is ported but has never run on an i.MX8MP from this crate, and its
own open questions (`src/vivante/VIVANTE_NOTES.md`) are the C's;
gl_test case 22 fails on purpose (`glIsVertexArrayOES` follows ES 3.0 6.1.10, the test
asserts the older behaviour); three hardware-facing register choices are inherited from
the C and unproven (`ISP_CTL_DBIAS_IS_INT` overwritten before the kick, 2x MSAA
tiles-in-flight, the 10-bit ZLS extent fields); and the Zephyr module build has never
been run.
