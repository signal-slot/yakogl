<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL design

YakoGL (夜光貝 yakogai, the shell cut for 螺鈿 raden; "yet another kernel-optional GL")
is a **source-compatible** OpenGL ES implementation that links as **`libyakogl.a`**.
Apps compile against the standard Khronos `<GLES2/gl2.h>` and link this archive.
It started on the PowerVR BXS-4-64 (Rogue, BVNC 36.53.104.796; TI AM62P / Toradex
Verdin) and now also drives the Vivante GC7000UL (i.MX 8M Plus) and the Broadcom V3D 4.2
(Raspberry Pi 4). A fourth back end, for the Arm Mali-G610 (RK3588, FriendlyElec
NanoPC-T6), is in progress (`src/mali/`, `driver/docs/nanopc-t6-mali.md`). Some
sections below describe the PowerVR first, as it was built.

## Goals / non-goals

- **Source compatibility first.** Standard Khronos headers; the documented GL ES 2.0
  entry points behave per spec for the subset we implement. Apps need no source changes.
- **Portability.** The GL core and the GPU back ends are OS-agnostic Rust (one
  `no_std` crate with no runtime dependencies, exporting the C ABI; see "The Rust
  crate" below). All OS/platform coupling is behind one interface (`gles_osal.h`).
  Targets, in priority order: **Zephyr** (primary), **bare-metal**, then other OSes
  (**Linux**). One `.a` of portable core + one small OSAL `.o` (C) per platform.
- **dEQP-GLES2.** Zero dEQP-GLES2 failures on the three boards was the first goal
  (`ROADMAP.md`, stage 1), and it is met: one whole tree per board from main
  `e9901869a` (2026-10-15/16) reads 0 Fail over the 17,165-case mustpass list
  (AM62P 17,140 pass, i.MX8MP 17,318, Raspberry Pi 4 17,126). The per-board state
  is `tools/cts/RESULTS.md`; the per-case detail is `docs/es2-cts-failures.md`.
- **GLES3.** The API dispatch, state object model, and backend are structured so
  GLES3 (VAOs, UBOs, MRT, instancing, transform feedback, 3D/array textures, sampler
  objects, integer/GLSL-ES-3.00 shaders) needs no reworking of the core. The ES 3.0
  entry points are already provided (`include/GLES3/`, `src/gl/es3.rs`). The cargo
  crate reports ES 3.0 unless the `report-es2` feature is on; the CMake and Zephyr
  builds default to ES 2.0 (`YAKOGL_REPORT_ES2` ON / `y`), as the dEQP-GLES2 runs need.
- **Non-goal (for now):** EGL window-system integration beyond a minimal
  surface/context.

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
  │                      v3d/*.rs     (V3D 4.2 shader streams)    │
  │                      mali/*.rs    (Valhall, in progress)      │
  │ 5a. Shader backend   shaderc/*    (GLSL ES ─▶ USC + metadata) │
  │ 5b. OS abstraction   osal/*       (mem, MMU, submit, sync,    │
  │                                    cache, fw, mmio)           │
  └─────────────────────────────────────────────────────────────┘
       5a and 5b are the only pluggable/replaceable layers.
```

Layers 1–4 and 5a are the Rust crate (built by cargo as a static library). Layer 5b
is one small C translation unit per platform, added to the same `libyakogl.a`. Layer 5a
has three implementations (see Shaders); the online one is the shipping back end.

## The PowerVR backend (layer 4) — already proven on silicon

The hard GPU bring-up is done and silicon-verified (see `driver/tools/pvrlab/`
and `docs/`): TA(geom)+3D kick, indexed draws, **multiple primitives & multiple draw calls**,
per-vertex varyings, **texture sampling** (TPU image/sampler state + PDS DOUTD), **uniform
/ constant** (push-const via DOUTD), **scissor** (region clip), **depth test** (ISP), **face
culling** (PPP cullmode). Recipes in hand for **blending** (FS-lowered, passtype
TRANSLUCENT), **render-to-texture** (two ordered kicks, coherent SLC — no flush), **stencil**
(ISPB 2-pass), **multi-varying / perspective** (coeff/DOUTI). These become the backend's
state-driven code paths; the `PVR_*` build knobs collapse into GL-state-driven branches.

Backend interface (as built): `int pvr_backend_draw(const struct gles_draw *draw)`
(`src/pvr/pvr_backend.h`) consumes a resolved draw (vertex buffers, index buffer,
VS+FS USC + their PDS programs, render state) and emits the VDM/PPP/PDS/frag-command
into GPU memory, then submits via OSAL. The backend is `src/pvr/backend.rs`; the
OS-free command generators (`pvr_vdm_ppp_gen.c`, `pvr_fw_boot.c`) live in
`driver/src/` and are called through FFI (see "The Rust crate" below).

## Shaders (layer 5a) — the key design decision

`glShaderSource(GLSL ES)` + `glCompileShader` must turn GLSL text into USC at the API
boundary. Mesa's path, GLSL→USC = glslang(GLSL→SPIR-V) → `spirv_to_nir` → PCO backend, runs
only on the host (`pco_shc`), to make the Mesa reference table. Three back ends behind one interface
(`gles_shaderc.h`: `struct shc_backend` — `compile`, `link`, `blend_variant`,
`compile_vs_linked`, `lod_sensitivity`, `sampler_variant` — reached through
`g_shaderc`, whose initial value `src/shaderc/mod.rs` picks from the build's features;
`shc_offline_register()` / `shc_online_register()` switch it):

- **`shaderc_online` (the shipping back end, the cargo feature `shaderc-online`):** our own
  `no_std`, allocation-free GLSL ES compiler in the library (`src/shaderc/online/`), so a
  shader an application writes at run time compiles on the board; dEQP-GLES2 writes
  thousands of them. It is the only shader back end the V3D and Mali builds have.
- **precompiled (`src/shaderc/precompiled.rs`, the cargo feature
  `precompiled-shaders`):** app shaders compiled by our own compiler while the image is
  built (`yakogl-precompile`, `tools/precompile`) and looked up by the compiler's record
  key. In front of the compiler a hit runs no compile; without the compiler
  (`YAKOGL_SHADERC_PRECOMPILED_ONLY`) it is the whole back end and refuses a miss by
  name. Fits a product whose applications are known when the image is built.
- **`shaderc_offline` (`src/shaderc/offline.rs`):** a table compiled on the host, looked
  up by the SHA-256 of the GLSL text. Its only table is the Mesa reference under
  `tests/reference/` (made by `glsl2usc` in `yakogl-tools`, wrapping `pco_shc`), linked
  with `mesa-reference-table` for measurement only: the PCO+glslang toolchain is too
  large to run on-target.

On the PowerVR every back end emits the same **shader ABI** the backend consumes: USC dwords, temp count, the DOUTI
coefficient layout for varyings (w iterator @cf0..3 then VAR0.., per the toolchain), the
DOUTD shared-reg layout for uniforms (push-const → sh0..), vertex attribute → `vi` mapping,
and the `po0..` outputs. This ABI is already characterised (`docs/online-shaderc/abi.md`,
`docs/online-shaderc/gc7000-abi.md`).

## OS abstraction (layer 5b) — `gles_osal.h`

Everything platform-specific is one struct of function pointers (or link-time symbols):
GPU-visible memory alloc + GPU MMU map/unmap; firmware load; MMIO/register access; submit
(KCCB write + doorbell); completion wait (poll/IRQ/fence); cache maintenance. The Zephyr
OSALs use the GPU drivers and board support in `driver/`; the bare-metal OSAL is a skeleton whose
board hooks are `TODO(board)` (open work, #1223); the Linux OSAL is a
host-side impl that mallocs fake GPU memory and completes submits at once, rendering
nothing. The GL core never calls an OS API directly.

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
  include/EGL/{egl,eglext,eglplatform}.h  Khronos egl.h + our platform types (native window =
                                a GPU-mappable pixel buffer); src/gl/egl.rs implements it
  include/gles_osal.h           platform interface (portability contract)
  include/gles_shaderc.h        shader-backend interface
  include/gles_offline.h        the offline shader table's source texts
  Cargo.toml  src/lib.rs  src/util.rs          the workspace + the crate `yakogl` (no_std rlib)
  capi/                         `yakogl-capi`: the staticlib (panic handler) and its
                                post-processing (assemble.cmake) -> libyakogl.a
  src/gl/{context,buffer,texture,shader,program,state,vertex,vao,draw,framebuffer,egl}.rs
  src/gl/{types,consts,egl_consts}.rs           GL types; tokens generated from include/
                                (yakogl-tools gl-consts)
  src/gl/{es3,etc1,names,native_cube}.rs        the ES 3.0 entry points, ETC1, the name
                                pools, the cube-map path
  src/gl/{es3_formats,srgb,blit,tf,tf_capture}.rs  ES 3.0 sized formats, sRGB,
                                glBlitFramebuffer, transform feedback and its CPU capture
  src/backend.rs                the backend contract (C view: src/backend.h)
  src/pvr/{backend,internal,csbgen,ffi}.rs  PowerVR backend (cargo feature gpu-powervr), its own programs (our
                                compiler's, yakogl-precompile --internal), the FFI to the driver's command
                                generators; ffi_layout_check.c pins it
  src/vivante/{backend,shader,texture,cmd,regs,state,kick,internal,ds_cache,tiled_copy}.rs
                                the i.MX8MP's GC7000 backend (cargo feature gpu-vivante);
                                src/vivante/VIVANTE_NOTES.md; src/vivante/gc_kick.h is the
                                OSAL's view of the kick
  src/v3d/{backend,state,texture,kick,ffi}.rs   the Raspberry Pi 4's V3D 4.2 backend (cargo
                                feature gpu-v3d); src/v3d/v3d_kick.h is the OSAL's view of the kick
  src/mali/                     the NanoPC-T6's Mali-G610 backend (cargo feature gpu-mali, in
                                progress; driver/docs/nanopc-t6-mali.md); src/osal/mali.rs its
                                part of the OSAL contract
  src/osal/mod.rs               g_osal + osal_set_ops; src/osal/render.rs the kick descriptor types
  src/osal/{zephyr,zephyr_gc,zephyr_v3d,baremetal,linux}.c the OSALs (C; zephyr_gc is the
                                GC7000's, zephyr_v3d the V3D's); zephyr_shaderc_scratch.c
                                (the compiler's regions) and zephyr_timing.c beside them
  src/shaderc/{mod,offline,link,sha256}.rs      offline shader back end + the shared link rule
  src/shaderc/{precompiled,record_key,packing,es100_twins}.rs  the precompiled back end,
                                its record key, and helpers the back ends share
  src/shaderc/online/                           the in-process compiler (`shaderc-online`)
  tests/reference/{blobs,vivante_blobs}.rs      the Mesa reference tables (measurement-only,
                                `mesa-reference-table`)
  src/{gl_internal,gl_context,backend}.h, src/pvr/pvr_backend.h, src/osal/osal_internal.h
                                C view of the crate's #[repr(C)] structs / #[no_mangle] helpers
  tools/yakogl-tools            host tools: glsl2usc (build-time GLSL→USC, wraps pco_shc,
                                emits Rust), usc-blob, femtovg-shaders, gl-consts,
                                abi-layout, csbgen-bindings, record and more
                                (`cargo run -p yakogl-tools -- help`)
  tools/precompile              `yakogl-precompile`: makes the product shader table
                                (`precompiled-shaders`)
  tools/vivante/                the GC7000's offline shader tool and table generator, and
                                the command-stream diff against Mesa's
  tools/cts/                    the dEQP harness: case lists, image scripts, the newest
                                board results (RESULTS.md, STATUS.md)
  src/abi_layout.rs  src/abi_layout_check.c     generated layout pins (yakogl-tools abi-layout)
  tools/hostharness/            host differential harness (compare.sh, run.sh)
  tests/                        host tests (cargo test --no-default-features --features host-test,gpu-powervr)
  driver/                       the GPU drivers and their board support: the OS-free
                                command generators (src/), board plans (docs/), the pvrlab
                                test rig (tools/), samples; `driver/rust` is a workspace
                                member
  CMakeLists.txt / baremetal.mk / zephyr/       run cargo, assemble libyakogl.a (README "Linking")
```

## The on-silicon reference

The library was built in vertical slices, each tested on silicon (clear and a solid
draw, attributes and varyings, textures, render state, framebuffer objects); the plan
is in git history (`git show 5ad2fa0e:DESIGN.md`, "Incremental plan"). The silicon test rig is
`driver/tools/pvrlab/gl_test.c`, so a known-good reference is never lost.

## The backend contract (`src/backend.h`)

The GL core reaches the GPU only through `struct yakogl_backend`, a function table
(`init`, `draw`, `flush`, `finish`, `finish_previous`, `kick_id`, `kick_pending`,
`heap_base`, `counters`, and the hooks for texture uploads and releases, queries,
depth resolves and mipmap chains; `src/backend.h` has all 21) and the GPU-agnostic types next to it: `struct gles_draw` (the
fully resolved draw), `gles_attrib`, `gles_texture_bind`, the `GLES_PRIM_/CMP_/CULL_/SOP_`
enums, `gles_texfmt`, `gles_surf_format` and the three heaps the core allocates in
(`GLES_HEAP_DATA`, `GLES_HEAP_SHADER`, `GLES_HEAP_PROGRAM`). The contract is defined in
`src/backend.rs` (`YakoglBackend`, `GlesDraw`, ...; `#[repr(C)]`, the C enums as `u32`
aliases) and `src/backend.h` is its C view. `src/backend.rs` also names the
backend of the build (`g_backend`: `yakogl_backend_pvr`, `yakogl_backend_vivante`,
`yakogl_backend_v3d` or `yakogl_backend_mali`, from that GPU's `backend.rs`). Everything PowerVR-specific (the options, the debug hooks,
`pvr_backend_counters`) stays in `src/pvr/pvr_backend.h`; only the on-silicon test
suite and the samples' diagnostics include it. Each further GPU adds `src/<gpu>/` with
its own table and shader back end (`driver/docs/vivante-kernel-rust.md` and
`src/vivante/VIVANTE_NOTES.md` for the GC7000, `driver/docs/rpi4-v3d.md` for the V3D).

Kicks: `kick_id` is the kick being recorded, and `kick_pending(id)` whether a
kick has still to complete. The GL core stamps what a draw renders into and reads
(texture targets, textures, buffers, its staging blocks) with `kick_id()` while it
builds the draw, notes where each stamp lives, and moves them all to `kick_id()`
once `draw` has returned 0 (`gl::draw::draw_stamp`, `restamp_draw`): a draw that
ends the open frame (another target, a full frame) lands in the next kick, and the
contract is that `kick_id()` after a recorded draw names the kick holding it. A
CPU write into an object, a depth texture's resolve and a read-back submit and
wait for the object's stamp while it is pending.

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

- the OSALs `src/osal/{zephyr,zephyr_gc,zephyr_v3d,zephyr_shaderc_scratch,zephyr_timing,baremetal,linux}.c`:
  thin glue over C APIs (Zephyr macros and inlines, board hook pointers, a variadic log),
  none of which stable Rust can define (their move to Rust is open work,
  #1225);
- the driver's OS-free command generators in `driver/src/`
  (`pvr_vdm_ppp_gen.c`, `pvr_pds_render_gen.c`, `pvr_pds_pixel_event_gen.c`), shared
  with the driver's own render path and called through the FFI declared in
  `src/pvr/ffi.rs`;
- `src/pvr/ffi_layout_check.c`, which has no code, only `_Static_assert`s: it pins the
  driver structs, constants and prototypes that `ffi.rs` mirrors, so a driver header
  change fails the build. `src/abi_layout_check.c` (generated, with its Rust twin
  `src/abi_layout.rs`, by `yakogl-tools abi-layout <driver src dir>`) does the same
  for the 38 `#[repr(C)]` structs (590 fields, 2026-10-26) the C headers share with the crate:
  size and every offset are asserted on both sides, so header/Rust drift fails the
  build; re-run the tool when a shared struct changes;
- the headers `src/gl_internal.h`, `src/gl_context.h`, `src/backend.h`,
  `src/pvr/pvr_backend.h`, `src/osal/osal_internal.h`: the C view of the crate's
  `#[repr(C)]` structs and `#[no_mangle]` helpers, used by the on-silicon test suite
  (`driver/tools/pvrlab/gl_test.c`) and the samples. They must be
  kept in step with the Rust structs (enforced by the layout pins above).

Builds: CMake, `baremetal.mk` and the Zephyr module each run cargo for `yakogl-capi`,
make its Rust runtime and compiler_builtins libc/libm symbols local
(`capi/assemble.cmake`) and compile the C objects. CMake and `baremetal.mk` append the
C objects, so `libyakogl.a` is still the single archive; Zephyr links the Rust archive
next to the `zephyr_library` of the C OSAL. With `YAKOGL_RUST_LINKAGE=external` only
the C parts are built and a Rust application links the `yakogl` crate itself. The
default cross target is `aarch64-unknown-none` (hard float: `CONFIG_FPU` /
`CONFIG_FPU_SHARING` on Zephyr). README.md, "Building" and "Linking", has the details.

The port began as a translation of the upstream C library and diverged from it
wherever the C was wrong. That account (the fix round, the syncs to the upstream
commits `a444fe9`, `bfab616` and `55de113`, and what upstream adopted) is in git
history (`git show 5ad2fa0e:DESIGN.md`, "The Rust crate"). Those three commits are
of the old C repository and are not in this one.

Verification on the host: unit tests on the strict mocks of `src/testutil.rs`, in every
feature configuration `tools/check.sh` lists, and the host differential harness
(`tools/hostharness/`, whose README lists what is and is not compared): gl_test.c, API
sweeps and hostile sequences over a recording OSAL (GPU memory hashes, decoded kicks)
and an interposer on the exported entry points, in every platform mode, dev and
release, with AddressSanitizer on the libraries themselves. Nothing is rendered there.
On silicon the library runs on all three boards, built as a Zephyr module: gl_test,
dEQP-GLES2, glmark2 and the Slint example (`examples/slint/`; results in
`tools/cts/RESULTS.md`). The open items are
the GitHub issues of signal-slot/yakogl.
