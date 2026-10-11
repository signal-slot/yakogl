<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D back end: two dated sections of its design record

Status (2026-10-11): finished record, moved from
`docs/online-shaderc/v3d-backend.md`. The M9 survey of Slint's Bevy demo is on
hold (the goal's showcase is Slint), and nothing of it was implemented. The
slow-job section reads the E runs on `8c8df5d1`; the OSAL timeouts it led to
are in the code (`src/osal/zephyr_v3d.c`). The rest of the design record stays
in `docs/online-shaderc/v3d-backend.md`.

### M9: Slint's Bevy demo on the Pi 4 (survey, no code)

Status 2026-09-24 (v3d-slint): a survey of what the goal's showcase, Slint's Bevy demo
on Zephyr, would need from us. Nothing is implemented. What is measured says so; the
rest is read from the source and marked as such.

#### Which demo

`slint/examples/bevy` (the Slint checkout at 526ba2cc82) has three variants, all on
Bevy 0.19:

| variant | Slint renders with | GPU users |
|---|---|---|
| `bevy-hosts-slint` | the software renderer, into a CPU buffer that becomes a Bevy texture | Bevy only |
| `bevy-hosts-slint-gpu` | Skia over wgpu 29, straight into a Bevy texture | Bevy and Skia |
| `slint-hosts-bevy` | Skia over wgpu 29; Bevy renders into a Slint texture | Bevy and Skia; plus reqwest, smol, gltf, png, jpeg |

`bevy-hosts-slint` is the one to aim at: Slint needs no GPU at all there (its
software renderer already runs on MCUs), so the GPU work is Bevy's alone, and
there is no Skia (a large C++ library) to port. The other two add Skia over wgpu.

#### The platform: std, before any GL

Bevy renders through wgpu, and on a GL device wgpu's `gles` backend (wgpu-hal).

- **wgpu-hal's GLES backend needs `std` on every target except wasm32**
  (wgpu-hal 29.0.4, `build.rs:14-22`, `gles_with_std`; `src/lib.rs:245-248`,
  `extern crate std`). It uses `std::sync::LazyLock` and loads EGL with
  `libloading` (`src/gles/egl.rs:3, 27-30`); khronos-egl's `static` feature is the
  alternative to dynamic loading.
- **Measured** (a throwaway crate, `wgpu = 29.0.4` with only `gles`, `cargo check
  --offline -Z build-std=core,alloc --target aarch64-unknown-none`): the build
  stops before wgpu-hal is reached, at three dependencies that are std-only:
  `hexf-parse`, `once_cell` and `parking_lot_core` ("std is required by ... because
  it does not declare #![no_std]").
- **rustc has no Zephyr target**, with or without std: the nightly's target list
  (1.98.0, 2026-07-01) has NuttX std targets (`aarch64-unknown-nuttx` and others)
  and `aarch64-unknown-none`, nothing for Zephyr. Our Rust on Zephyr is `no_std` +
  `alloc` (`gl_slint`, `gl_femtovg`).
- **What is `no_std` already**: naga 29 (`#![no_std]`, `glsl-out` a feature),
  wgpu-core 29 (`std` a default feature), wgpu-hal's crate root, and Bevy 0.19's
  core crates: bevy_ecs, bevy_app, bevy_tasks, bevy_platform and bevy_math (each
  `no_std` with `std` as a default feature). bevy_render and bevy_pbr are not in
  this machine's registry, so whether their `no_std` story exists is unmeasured.

So there are three routes, and the choice is the user's (it decides the size of
everything else):

1. **A Rust `std` for Zephyr**: a target and a std port over Zephyr's POSIX layer,
   as NuttX has. Largest, and outside this repository. It does not build every
   crate unchanged either: see "Correction" at the end of (a) below.
2. **Bevy and wgpu without std**: patch wgpu-hal's GLES backend to build `no_std`
   (the `gles_with_std` cfg, LazyLock, parking_lot, a static EGL), and replace the
   std-only dependencies above. Then measure bevy_render/bevy_pbr for the same.
   Forks to carry, like the femtovg/glow/Slint no_std patches in `slint/nostd`.
3. **Measure first**: extend the probe above to bevy_render + bevy_pbr and list
   every std blocker, before choosing. Cheap (one `cargo check` per route), and it
   turns this section's biggest unknown into a number.

#### What wgpu asks of the GL

Read from wgpu-hal 29.0.4 `src/gles/adapter.rs`:

- **OpenGL ES 3.0 or newer**, refused below (`adapter.rs:257-262`). YakoGL reports
  "OpenGL ES 3.0 YakoGL" on the Pi 4 (the Slint board runs print it).
- Everything past 3.0 is optional and switched off by version or extension: compute
  and storage buffers (3.1), indirect draws (3.1), base vertex (3.2), cube arrays,
  geometry shaders, image load/store (`adapter.rs:315-490`). This is the WebGL2
  profile Bevy supports ("downlevel").
- On any ES context wgpu assumes ETC2 textures are supported (`adapter.rs:531`):
  only a problem if an asset uses them.
- The ES 3.0 API wgpu relies on (from its GLES backend's use of glow, not counted
  per call): vertex array objects, uniform buffers (`glBindBufferRange`,
  `glUniformBlockBinding`), sampler objects, `glTexStorage2D/3D`, `glBlitFramebuffer`
  and `glReadPixels` for copies, fence sync (`glFenceSync`, `glClientWaitSync`),
  `glMapBufferRange`, instanced draws, `glVertexAttribIPointer`, `glDrawBuffers`,
  `glClearBuffer*`, `glInvalidateFramebuffer`, sRGB formats. Which of these the GL
  core implements is for the GL core's owners to confirm; the draw-level gaps below
  are the V3D's.

#### What the V3D path refuses today that a Bevy frame would reach

At origin/main 6eba4302 (`src/v3d/state.rs` `refusal`, `src/v3d/backend.rs`, and the
compiler's V3D refusals in `src/shaderc/online/v3d/`). The right column is read from
our source; the middle column is Bevy's and naga's documented behaviour, **not read
from their source here** (bevy_render and bevy_pbr are not in this machine's
registry), so each row is a candidate for the measurement below, not a finding:

| need | Bevy / wgpu | V3D today |
|---|---|---|
| instanced draws | Bevy batches meshes by instance; naga's `instance_index` | refused: "instanced draws are not implemented" |
| multisampling | Bevy's default `Msaa::Sample4` | refused; the app can set `Msaa::Off` |
| float colour targets | HDR cameras (`Rgba16Float`) | refused; the app can keep `hdr: false` |
| more than one colour target | prepasses, deferred | refused; off by default in Bevy |
| non-float vertex attributes | `glVertexAttribIPointer` (indices, flags) | refused: "an attribute format other than float32" |
| 3D, array, cube textures | tonemapping LUTs (3D), shadow maps (2D arrays), environment maps (cube) | "a texture look-up other than 2D" |
| shadow look-ups | shadow maps (`sampler2DArrayShadow`) | "a shadow look-up" |
| `textureSize`, explicit LOD, offsets, integer look-ups | naga's output for PBR and post-processing | "TexSize", "... an explicit level or gradients", "... with an offset", "an integer texture look-up" |
| flat varyings | naga emits `flat` for integer varyings | "flat varying" |
| long uniform arrays | clustered lights in uniform buffers (WebGL2 path) | "a uniform array too long to select from (no ldunifa yet)" |
| texture look-ups in the vertex shader | skinning, morph targets | "a texture look-up in the vertex shader" |

Several can be avoided by the app (MSAA off, HDR off, no shadows, no tonemapping
LUT, unlit or simple materials), which is how a first frame would be reached; the
demo's cube, quad and cow with lights would still need instancing, uniform blocks
and textures in the vertex path. Which exact shaders it compiles is unmeasured:
Bevy composes its WGSL at run time (naga_oil, per pipeline key) and wgpu translates
it with naga to GLSL ES 3.00.

#### How to measure the shaders, without the board

The femtovg route again: get the demo's GLSL ES 3.00 as wgpu would hand it to the
GL, and compile every program through our compiler on the host, pinned like
`femtovg_outcomes_are_pinned`. Two ways to get the text: run the demo on the host
against a recording GL layer (as `slint/host/slint_gl_rec.c` does for Slint;
no rendering, only the shader sources and the call stream), or compose Bevy's WGSL
with naga_oil for the demo's pipeline keys and translate it with naga's `glsl-out`
(`Version::Embedded { version: 300, is_webgl: false }`) offline. The first also
gives the draw count, the textures and the uniform-buffer sizes per frame, as the
Slint recorder did.

#### Order, if the user chooses to go on

1. Route 3: the std measurement for bevy_render and bevy_pbr (a probe crate).
2. The shader measurement above: the demo's programs through our compiler, a
   pinned list of first refusals.
3. With the user's choice of route 1 or 2: the platform work, then the V3D gaps the
   measurements rank first (instancing, uniform blocks and textures beyond 2D are
   the likely top three).

#### Measured: the std blockers and the demo's shaders (2026-09-24 evening)

**(a) What needs std.** A probe crate, `bevy = 0.19.1` with `default-features = false`
and `bevy_core_pipeline`, `bevy_pbr`, `bevy_gltf`, `png` (the demo's render subset,
no windowing), `cargo check --keep-going -Z build-std=core,alloc --target
aarch64-unknown-none`: 46 crates fail to compile. Each failing crate was checked for
`no_std` support in its crate root and for the std facilities its source names
(`std::thread`, `time`, `fs`, `net`, `io`, `env`, `process`, `std::os`/`libc`; a
mention, not a proven call path), and `cargo tree -i` names who pulls it in.

| class | crates | who pulls them in | what they use |
|---|---|---|---|
| Bevy's render stack, std by design (no `no_std` in the crate root, and `features = ["std"]` on its Bevy dependencies) | bevy_render, bevy_pbr, bevy_core_pipeline, bevy_image, bevy_shader | the demo itself | everything below |
| wgpu-hal's GLES backend (from its source: `gles_with_std` on every non-wasm target) | wgpu-hal | wgpu | LazyLock, libloading for EGL |
| std-only dependencies (no `no_std` at all), 8 | hexf-parse; termcolor | naga (WGSL hex floats); codespan-reporting, from naga and naga_oil | parsing; io, env |
| | parking, polling | async-io, from bevy_asset | thread, time, net, io, process, os |
| | parking_lot_core | parking_lot, from wgpu-core | thread, time, os |
| | pxfm | moxcms, from image, from bevy_image | (pure maths, no `no_std` declared) |
| | thread_local | bevy_utils | thread |
| | thiserror 1.0.69 | not found by `cargo tree -i` for this target | io |
| no_std-capable, with `std` switched on by a dependent, 38 | arrayvec, async-task, base64, crossbeam-utils, either, errno, fastrand, foldhash, futures-io, getrandom, log, memchr, once_cell, rand, regex-syntax, serde_core, spin, num-traits, ... | mostly Bevy's own `std` feature (bevy_platform, bevy_ecs, bevy_math defaults) and `std` features of log, rand, once_cell, serde | thread (11 of them), time, io, os, fs, net |

What it costs: route 2 (Bevy and wgpu without std) is a fork of most of Bevy's render
stack (five crates std by design) plus wgpu-hal's GLES backend, the eight std-only
crates and 38 feature fixes. At run time, route 1 (a Rust std for Zephyr) needs
from Zephyr: threads and thread parking, time, `poll` for async-io, randomness for
getrandom, and a filesystem only if assets are loaded from files.

**Correction (2026-09-25, the route-3 survey).** This section said route 1 builds
all of it unchanged and is the realistic route. Both are wrong:

- wgpu-core turns on its GLES backend only for Windows, Linux, Android and FreeBSD
  (plus Apple and the web targets; wgpu-core 29.0.4 `Cargo.toml:211`). A Zephyr
  target gets no GL backend. wgpu-hal loads EGL with `dlopen` on every native
  target; its static-EGL path is Emscripten-only (`src/gles/egl.rs:720-738`). So
  both routes need a wgpu patch.
- The `libc` crate (0.2.189) has no Zephyr module, `polling` picks its backend from
  a fixed list of `target_os` values, and rustc has no Zephyr target (NuttX and
  ESP-IDF have std targets).
- The crates above are only the lowest layer. `cargo check --keep-going` never
  reaches a crate whose dependency failed.

The survey classifies the whole graph instead, with
`driver/samples/zephyr/slint/host/bevy_std_class.py`, over the example's own
dependencies (bevy-hosts-slint, plus wgpu's `gles`) for aarch64-unknown-none:

| set | crates | no_std as built | std only by a feature | no `no_std` mode | of those, naming nothing past `alloc` |
|---|---|---|---|---|---|
| the whole example | 294 | 92 | 136 | 66 | 17 |
| bevy_render, bevy_core_pipeline, wgpu, naga | 171 | 43 | 95 | 33 | 8 |
| wgpu alone | 43 | 20 | 19 | 4 | 1 |
| Slint | 86 | 50 | 33 | 3 | 0 |

The 66 crates with no `no_std` mode, by the heaviest std facility their code names:
threads 15, files 15, io 7, process/env 6, other OS 3, net 2, time 1, and 17 that
name nothing past `alloc`. Bevy's own render code uses little std. bevy_render has
4 `env::var`, one thread (the pipelined-rendering plugin), 2 `Mutex` and one
channel, and uses files only on Windows. Nine Bevy render crates name no `std::`
path at all. The real OS users on the render path are the asset I/O and the task
pool. bevy_tasks has a `no_std` mode. bevy_asset only seems to: it declares
`#![no_std]` and then an unconditional `extern crate std;` (see "Measured: the
render set with the declarations patched" below). The survey's report
(`.claude/worktrees/report-bevy-std-survey.md`) recommends route 2 on these
numbers; the compile count below says in which form.

**(b) The demo's shaders.** `driver/samples/zephyr/slint/host/bevy_gl_rec`
(5786c40a) runs the demo's scene headless on wgpu's GLES backend over a recording GL
that answers as YakoGL does on the Pi 4. Two things stop it before any shader
reaches us, both Bevy-side or GL-core facts, not V3D back-end ones:

1. **GL_MAX_UNIFORM_BLOCK_SIZE.** YakoGL reports 6144 bytes; wgpu then refuses
   Bevy's `mesh_view_bind_group` (binding 8, 16,320 bytes) and
   `skinned_mesh_bind_group` (16,384): no frame is drawn. OpenGL ES 3.0.6's
   minimum is 16384 (Table 6.33). The GL core also reports five other limits below
   ES 3.0's minimums (Tables 6.28-6.33, read from the Khronos PDF):

   | limit | ES 3.0 minimum | YakoGL reports |
   |---|---|---|
   | MAX_UNIFORM_BLOCK_SIZE | 16384 | 6144 |
   | MAX_DRAW_BUFFERS | 4 | 1 |
   | MAX_SAMPLES | 4 | 1 |
   | MAX_COLOR_ATTACHMENTS | 4 | 3 |
   | MAX_VERTEX_TEXTURE_IMAGE_UNITS | 16 | 8 (16 on the V3D since 2026-09-26) |
   | MAX_COMBINED_TEXTURE_IMAGE_UNITS | 32 | 16 |

   They are the GL core's to decide (and some are the V3D's real limits); they are
   listed because a client that trusts "OpenGL ES 3.0" relies on them.
2. **Bevy's GLES 3.0 profile is wasm-only.** Bevy 0.19 sets
   `NO_ARRAY_TEXTURES_SUPPORT`, `NO_CUBE_ARRAY_TEXTURES_SUPPORT` and
   `SIXTEEN_BYTE_ALIGNMENT` and the matching bind-group layouts only for wasm32
   (bevy_render `pipeline_cache.rs:237-242` and 64 more lines), so on a native ES
   3.0 target the PBR shaders do not compose ("Capability CUBE_ARRAY_TEXTURES is
   required"). `bevy-0.19.1-gles3-native.patch` (65 lines, four crates) makes it
   follow the `webgl` feature; a Zephyr build of Bevy needs the same.

With the uniform limit at 16384 and the patch, the frame is drawn: per frame 7 draws
(none instanced), 34,911 vertices, 6 programs, depth/stencil state on 6 draws,
blending on 1, 9 framebuffer binds, 32 uniform-buffer binds, texture bindings 70 2D,
14 3D (the tonemapping LUT), 7 cube and 7 2D-array, and one 1,440,000-byte texture
upload (the 600x600 Slint UI texture). Nine programs are linked. Through our online
compiler on the host (6eba4302's compiler, the product's compile and link entry points,
a local test not committed), **0 of 9 link**; each stage's first refusal:

| program | vertex shader | fragment shader | link |
|---|---|---|---|
| 000, 006 (lit PBR, ~1,700-line FS) | `Abi(TooManyUniforms { got: 279 })` | "compiler scratch buffer exhausted" | - |
| 001, 005, 008 (mesh passes) | `Abi(TooManyUniforms { got: 279 })` | compiles | - |
| 002 (tonemapping) | compiles | "a struct member that is a matrix is not lowered yet" | - |
| 003 | compiles | compiles | `NotYet("TexSize")` |
| 004 | compiles | compiles | `NotYet("an integer texture look-up")` |
| 007 | compiles | compiles | `NotYet("gl_VertexID")` |

Bucketed by message: 5 x `TooManyUniforms { got: 279 }` (the shader's uniform table is
counted in a `u8`, `v3d_abi.rs:163`, and Bevy's mesh vertex shader reflects 279 rows),
2 x scratch exhausted (the 2 MiB compiler scratch, the image's default
`CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES`), 1 each of the struct-member matrix lowering,
TexSize, the integer texture look-up (texelFetch) and gl_VertexID. These are first
stops only. The GLSL also uses `uint` heavily, flat varyings, `gl_InstanceID`,
`sampler2DShadow`, `samplerCubeShadow`, `sampler3D`, `textureLod` and `textureSize`,
so more gaps stand behind these.

#### Measured: the render set with the declarations patched (2026-09-25)

Status: Bevy is on hold (the user, 2026-09-25: the V3D's ES 3.0 and the CTS come
first). This is the measurement to resume from.

The probe depends on bevy_render, bevy_core_pipeline, wgpu 29.0.4 (`gles`, `wgsl`),
wgpu-hal (`gles`) and naga 29.0.4 (`wgsl-in`, `glsl-out`), all with default
features off. It is checked with `cargo check --keep-going -Z build-std=core,alloc
--target aarch64-unknown-none`. The errors are counted per crate from cargo's JSON
messages.

**Patched before counting (declarations and features only):**

- The features in the manifests of all 157 crates the target builds (707 dependency
  entries). Every `std` feature is off, and so is every feature that reaches one,
  in the crate or through a dependency (bevy_utils' `parallel` =
  `bevy_platform/std`; bevy_tasks' `async_executor`). Default features go off when
  they reach std, and the other defaults are listed instead. `alloc` is added where
  a crate offers it. What a crate itself asks for without std is on: glam `libm`,
  weak-table `ahash`, once_cell `critical-section`.
- The 34 crates with no `no_std` mode get `#![no_std]`, and `std` becomes a facade
  crate. The facade is core and alloc under std's paths, plus std's prelude
  (nightly `prelude_import`), so every std path that core or alloc has still
  resolves. bevy_asset's unconditional `extern crate std;` becomes the facade.
- The GLES target gates for `target_os = "none"`:
  - wgpu's and wgpu-core's build scripts;
  - the target keys of wgpu-core and `wgpu-core-deps-windows-linux-android`;
  - wgpu-hal: no `gles_with_std`, and a static EGL through the existing Emscripten
    branch;
  - naga: the build-script alias that turns std on for `wgsl-in`.
- Out of the manifests, so that their use counts in the crate that uses them:
  - bevy_asset's async-fs, async-io, async-broadcast and blocking;
  - wgpu-hal's libloading;
  - bevy_log's tracing-log.
- Target choices: the non-GLES wgpu backends and RenderDoc off; getrandom's custom
  backend; blake3's C code built with the Zephyr SDK's `aarch64-zephyr-elf-gcc`.

With only these, 153 of 162 crates check clean. The errors left in the other 9 are
real code needs.

**Leaf fixes.** cargo does not check a crate whose dependency failed. So each
failing leaf got the smallest real fix, and its errors were counted first:

| crate (errors) | std need | fix |
|---|---|---|
| pxfm 147, weak-table 3, hexf-parse 2 | float methods (`sqrt`, `round`, `ceil`, `floor`, `round_ties_even`, `powf`) | a `FloatMath` trait over nightly `core::fN::math` |
| guillotiere 10 | `println!`, `std::io::Write` in debug and SVG-dump functions | those three functions left out |
| parking_lot_core 9 | thread parking, `Instant`, `thread_local!` | parking_lot replaced by a 13-line shim over spin's lock_api locks |
| glow 8 | C types from `std::os::raw`, `HashSet`, `catch_unwind` | `core::ffi`, hashbrown, a direct call |
| khronos-egl 2 | C types from libc (none for this target) | `core::ffi` |
| naga 1 | `std::path::Path` in the WGSL error text | `AsRef<str>` |
| wgpu-hal (GLES) 20 | `LazyLock`; `vec!`/`format!`; Wayland code on every native target (4); EGL 1.5 upcast on a static EGL (4); the timed context lock (2); `catch_unwind` | spin's `Lazy`, imports, a stub `wayland_sys`, the Emscripten branch, `try_lock()`, a direct call |
| wgpu-core 5 | `once_cell::sync` (a feature, above); `log2`, `round` | libm |

**Result:** 166 crates check clean. They include naga, wgpu-types, wgpu-core,
wgpu-hal's GLES backend, glow and khronos-egl, and Bevy's core crates (bevy_ecs,
bevy_app, bevy_reflect, bevy_math, bevy_tasks, bevy_utils, bevy_platform,
bevy_transform, bevy_window, bevy_time, bevy_color, bevy_diagnostic). 5 crates
have errors:

| crate | errors | what they are |
|---|---|---|
| **bevy_asset** | 110 | `std::io` 53 (its readers are futures-io's `AsyncRead`/`AsyncWrite`, which exist only with std), channels 21 (crossbeam-channel, async-broadcast), `std::fs` 17, `std::path` 13, `env` 3, `Instant` 2 |
| naga_oil | 49 | std `HashMap`/`HashSet` (7 imports), `LazyLock` (2), and what cascades from them |
| moxcms | 11 | image's colour management: `SystemTime`, `std::io::Read`/`Cursor`, runtime CPU-feature detection, `rem_euclid` |
| bevy_log | 6 | `LogPlugin`: environment variables, stderr, tracing-subscriber's formatter |
| wgpu | 1 | wgpu's build script turns std on for `Send + Sync` ("TODO: Remove this when an alternative Mutex implementation is available for `no_std`"). With that off: 889 E0277 `Send`/`Sync` errors from that one cause |

Not reached, because they sit on top of the crates above: bevy_render,
bevy_core_pipeline, bevy_image, bevy_shader, bevy_mesh, bevy_camera,
bevy_material, bevy_light and image. Their own counts are unmeasured.

**What it means:**

- wgpu's GLES stack without std is small: 47 real errors, fixed in about 70 lines
  plus the lock shim. One design item is left: wgpu's `Send + Sync` without std,
  which is wgpu's own TODO. Bevy's render resources must be `Send + Sync`, so wgpu
  needs a no-std Mutex. On a single-core image, a critical-section or spin lock
  does.
- bevy_asset is the wall. Its I/O, paths, files and channels are std, and every Bevy
  render crate depends on it.
- **The route when Bevy resumes: an in-repo `std` facade.** It is the facade this
  measurement used: core + alloc + std's prelude. It gets grown with the pieces
  bevy_asset, naga_oil and bevy_log need (an io and path subset, channels, a lock,
  time), plus the wgpu GLES patches above. The 34 std-only crates built through it
  with no change beyond the header. It needs no rustc target and no Zephyr std port
  (route 1 would still need the same wgpu gates). It does need nightly
  (`prelude_import`, `core_float_math`). The first step is to make bevy_asset build
  on it, and then count bevy_render and bevy_core_pipeline.

**The probe** stays in the v3d-slint worktree's `build/bevy_nostd_decl/`, not in
the sample. It needs a nightly toolchain and `-Z build-std`, and it vendors about
300 crates. Its parts:

- `probe/`: the crate;
- `patch.py`: the declaration and feature patcher;
- `leaf.py`: the leaf fixes above;
- `std_facade/`: the facade;
- `shim/parking_lot/`: the lock shim;
- `count.py`: counts the errors per crate;
- the runs: `run4.json` is declarations only; `run10.json` is the final state.

To re-run: `patch.py <dir>`, then `leaf.py <dir>`, then the cargo check above,
with `CC_aarch64_unknown_none` set to the SDK's gcc and
`RUSTFLAGS='--cfg getrandom_backend="custom"'`.

### Slow and hung jobs (the E runs on 8c8df5d1)

The OSAL waited 1 s per stage, then marked the job done and submitted the
next one with no reset. In the E runs a failing struct.uniform case took
about 1 s, and later cases passed, so the timed-out jobs were not hung:
they ended later. Two defects followed from that:
- `v3d_wait` cleared every status bit it saw, so a late job's done bit
  could be taken as the next job's end. Now a wait clears only the bits it
  waits for (and OUTOMEM), and each submit clears its own stage's done bit
  before starting (tools/v3d/hw_mock_test.c t_stale, 5 checks; revert
  check: clearing every bit fails it).
- A timed-out job's held frees were given back while the job may still
  have been running.
Now each stage waits up to JOB_HANG_US (4 s), timed:
- A stage that ends after JOB_SLOW_MS (1 s) is slow: its result stands,
  and it is counted and printed with its time.
- A stage that does not end (timed out, the overflow pool dry, or an MMU
  fault) marks the GPU hung. No later job is submitted (each is refused
  with -EIO), and no free is given back, since the hung job may still use
  it.
The record says "slow jobs: N (the slowest M ms); MMU faults: F; hung:
none | job J, K later jobs not submitted, L KiB not given back". This
replaces the old line that assumed every job after a timeout also times
out.

**Recovery (batch 24).** A stage that does not end now resets the GPU,
recorded first, as Linux's v3d_reset does on the 2711 (v3d_gem.c:124-146;
the node's PM reset line, bcm2711.dtsi:627, so bcm2835_reset_reset: power
off, power on, bcm2835-power.c:538-566; facts only):
- interrupts quiesced;
- v3d_power_off: ASB slave then master stopped (REQ_STOP, ACK rises), clock
  off, PM_V3DRSTN asserted (bcm2835_asb_power_off, :342-371);
- v3d_power_on;
- the caches' state;
- the MMU on the same page table;
- interrupts cleared.
The job's frame is lost. What it held comes back, since the GPU is idle,
and later jobs are submitted. Every verdict names the count
(osal_v3d_recoveries, "recoveries: N" in the record): a run that
recovered is never a clean pass. Only a reset that fails marks the GPU
hung. hw_mock_test t_reset pins the sequence (the stop order, a master
that never stops, the reset's order and page table, a failing reset
stopping before the MMU). Whether a job after a reset draws right is the
board's question: gl_test 129's control faults today, so its later
passes are that test.

**Code guards.** Before a job runs a program, the backend checks each stage's
code against the compiler's contract (v3d-m5, 2026-09-24):
- the program-end THRSW is on the third-to-last word, and neither delay
  slot switches again;
- every word decodes (the online compiler's own decoder);
- exactly as many words consume a uniform as the stage's stream holds:
  ldunif, ldunifrf, wrtmuc, a TLBU write, and a branch that moves the
  stream. The compiler emits no ldunifa, so nothing else reads it.
A fault is refused by name per stage and reason (for example "the vertex
shader's uniform stream is not as long as its code reads") and counted in
v3d_refused_draws. A program that passed is remembered by its code's
address, length and a checksum of its words (32 entries), so each program
is decoded once, and code that changes under the same address is checked
again. Decoding costs about 45 ns a word on the host: per draw it would
cost too much.

**The MMU-fault record.** The struct.uniform runs on 8c8df5d1 faulted
(MMU_PTI) at VA 0x8080000, past the carve-out, after other sampler cases
had run, and not alone. The whole carve-out stays mapped, so a freed copy
cannot cause a page-table fault: something handed the GPU a wild
address. On a job that ends with -EFAULT the backend prints:
- each texture record against the bytes it packed (a record overwritten
  after packing has a garbage base);
- each program's code on the GPU against the CPU copy it checked (the GPU
  copy overwritten: the QPU ran garbage).
A canary follows every tiled image: 64 bytes in the per-job arena, the
slack in a kept copy. A tiling that writes past its image is counted and
logged with the texture's shape.
