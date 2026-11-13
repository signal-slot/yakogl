<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Changelog

All notable changes to YakoGL are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The first version is 0.1.0; its section says what the project is and what it has
at that version.

Commit hashes quoted here name commits of the non-public working repository
the project was developed in.

## [Unreleased]

### Results

The newest board results are in `tools/cts/RESULTS.md`; these are the
records at the tip.

- dEQP-GLES2, one whole tree per board, still reads 0 Fail. Re-runs: the
  AM62P's 1x images at `36c4305a` (2026-11-05), where the two
  `flush_finish` timing cases pass without dEQP's warning and the
  combined 1x + ms4 row reads 17,142 (the ms4 row is still from
  `e9901869a`); i.MX8MP 17,318 at `0fbe61a3`; Pi 4 17,126 at `46f93767`.
- glmark2-es2 on the AM62P reads 468: 33 scenes, 0 failed, 0 GL errors
  (2026-11-01, record `99156df8`; shadow 10.62, refract 35.09 and
  terrain 83.06 ms a frame). The README table stays #1196's one-commit
  table.
- `gl_test` on the common set: the AM62P 49 of 50 — case 20 `etc` fails
  its first check alone, a stale expectation, its other 16 checks pass
  (record `99156df8`); the i.MX8MP 49 of 49 run, case 16 skipped, from
  main `f87dde43` (`8737e5a3`).
- `gl_test` on the NanoPC-T6 (its own Linux, panthor): 165 of 256 cases
  pass, twice with identical verdict lines, on main `1a22bd38e`
  (`ba200dda`); the previous record read 161 at `0decb5f7`
  (`98863975`).
- glmark2-es2 on the NanoPC-T6 ran all 33 scenes on the board's own
  Armbian: the first record (main `da65355e`, `08ae7a8e`) and the second
  session (main `a57d8104`, `4bf0e7ab`) read the same 33 outcomes —
  desktop shadow clean at 0.96 ms a frame (1042 FPS), its picture a
  match; 31 of the 32 failing scenes stop at the online compiler's
  `glCompileShader` refusal (#33) and terrain aborts in glmark2 itself
  after its refusals, so there is no score yet.
- Z4 — one frame submitted from our own kernel side on Zephyr, the
  `t6_submit` probe — reads PASS on the NanoPC-T6: 42 of 42 predicted
  record lines, the target read back 2048 red and 2048 blue of 4096
  pixels, 26 ms in all (the image of main `ef86672bf`; a kexec
  development run, the U-Boot sign-off form not run, `14ea040a`).
- dEQP-GLES3 on the Pi 4: rows 0-15, sixteen images, from main
  `b1b5b845`: 8,350 mustpass cases all with a result, 7,346 pass, 558
  fail, 446 not supported, 0 GPU recoveries (`d498d5ad`).
- dEQP-GLES2 on the NanoPC-T6 (Mali-G610, panthor, the board's own
  Armbian), the whole tree in two sessions, kept as slices under
  `tools/cts/log/` (not yet a RESULTS.md row): S1 at main `a50d87b5`,
  5,141 cases, 2,659 Pass, 2,445 Fail, 34 NotSupported, 3 QualityWarning
  (`8fd2e909`); S2 at main `1bd2074f`, 4,872 cases, 1,245 Pass, 3,626
  Fail, 1 ResourceError (`4ecefc7b`). Both ran with the same boot before
  and after every chunk.
- The full gate reads 222 ok, 0 failed, 0 gap(s) at `c66bea9f`
  (`4946e879`); green means that one commit and is re-run on the commit
  to publish.

### Added

- V3D: ES 3.0 texture and QPU work for the Raspberry Pi 4: 3D textures and
  2D-array textures (the layout and slice offsets in `v3d_tex.c`, uploaded
  at their `texture.rs` offsets; the by-name draw refusal of the two
  types was lifted at `8ae0a44b`), `texelFetch` and
  the `*Offset` look-ups in the QPU compiler, swizzle and wrap R taken from
  `GlesTextureBind`, and an integer clear program for a scissored or masked
  clear of an integer colour target. Also the V3D 7.1 (Raspberry Pi 5)
  QPU model, encoder and decoder, host-side only (#210).
- V3D: a `GLES_SURF_RGB10_A2` render surface with its TLB words, and the GL
  core's side of `GL_RGB10_A2` (own surface, bits, CPU mipmap chain). The
  V3D reports the format: `GLES_TEXFMT_RGB10A2` joined
  `V3D_SAMPLED_TEXFMTS`, so a `GL_RGB10_A2` texture, renderbuffer and
  attachment is the 10/10/10/2 class there (`03f4bbd9`). The TFU has no
  10-bit input yet, so the mipmap chain stays on the CPU; PowerVR and
  GC7000 keep the RGBA8 fallback (`docs/notes/v3d-rgb10a2.md`).
- GC7000: texture-format words for the remaining ES 3.0 classes, checked
  against Mesa; RGBA32F/UI/I as two G32R32 planes with a companion slot and
  the compiler's 128-bit look-up keys (kept inert,
  `docs/notes/gc-128bit-texture-sampling.md`); the four 64-bit integer
  render targets (RGBA16I/UI, RG32I/UI) drawn through
  a tiled shadow, 17 of the 19 integer surfaces (RGBA32I/UI stay refused).
- PowerVR: end-of-tile programs for four colour targets (`eot4`, and
  `eot4_tb`..`eot4_tb3` with the last one to three targets from tile
  buffers, each spilled emit patched with its own buffer's address), three
  tile buffers, and four PBE pairs in the render kick (344 bytes, `frag_pr`
  at 232 and 112 bytes). Draws still write at most three targets
  (`PVR_RENDER_MAX_RT`); the capacity is `PVR_RENDER_EOT_MAX_RT` (RC14).
- PowerVR: the groundwork of the firmware's partial render (#2, #3), all
  off by default: the render kick's zeroed `frag_pr` block
  (`PvrRenderFragPr`, 96 bytes), the SPM scratch size and PBE store words
  (`src/pvr/spm.rs`), the reserved PR end-of-tile PDS data slot, the
  partial-render stall decoders (`pr_diag.rs`) and the debug trace groups.
  `gl_test` case 217 `forcedpr` is the reproducer
  (`docs/notes/pvr-render-pool.md`, `docs/notes/pvr-pr-zs.md`).
- PowerVR: an integer render target variant packs several colour outputs
  (the two outputs of `modf`); more than four output registers are refused
  by name.
- Mali (`gpu-mali`, NanoPC-T6, #32, #33): the GLSL ES compiler reaches many
  more shader forms (integer and bool values, run-time loops and arrays,
  several samplers, 2D-array and 3D textures, mixed fp16/fp32 code,
  control flow kept as branches or flattened into selects, fragment outputs
  past location 0), and the frame builder draws into several colour targets
  and keeps a split 4x frame's samples. Each is checked byte for byte
  against Mesa's drm-shim output.
- NanoPC-T6 bring-up, not on Zephyr's GPU path yet: the MCU firmware's
  section list, CSF interface and boot over a host model (Z3); the
  `board::nanopc_t6_fw` probe with a `#[repr(C)]` record and its C entry
  `yd_t6_fw_up` (declared in `yakogl_drivers.h`); and Z4, the CSG/CS
  start, the scheduler's submit sequence, ring, doorbell and sync-object
  wait, and the tiler heap's creation, growth and chunk return, transcribed
  from panthor and run over host models; Z4 also read PASS on the board
  (Results). The model firmware acknowledges CS 0, sizes CSG 0's suspend
  buffers and publishes the output extract as the board's does
  (`6abf4e8c`, `5ec69246`, `068e65be`).
  `driver/tools/kexec-zephyr/t6-verdict.sh` judges a board `uart.log`.
- V3D: the QPU's shadow look-ups end to end, pinned to Mesa's ES 3.0
  look-up forms (`06bd7bc5`); `textureGrad`, vertex-stage 32-bit look-ups
  and the 3D offset's r lane (`79b98f34`); per-RT 4x sample buffers and
  typed 4x targets in the job builder (`4bcb55c3`); the RGB10_A2 job
  words, clamped clear, renderbuffer and MRT-beside-RGBA8 tests
  (`6770479e`); the V3D 7.1 clear frame in the frame builder with
  `frame_check` at 7.1 (`5fec9352`); and the Pi 5 identity probe
  `rpi5_ident` (`4b1f68ea`).
- Mali (`gpu-mali`): a shadow sampler carries GL's compare function,
  flipped as Mesa does (`af3d1bcb`); every instance divisor is one draw,
  NPOT divisors as Mesa encodes them (`d7731961`); textures stored as
  rgba16f and r16f sample (`520bc394`); `GL_MAX_COLOR_ATTACHMENTS` is 4
  (`4cf8532b`); the transcendentals reach Valhall ops and run-time index
  ladders rewrite into Mesa's binary search (`3d937fe5`, `ee9d01e0`); a
  vertex `M * vec4(p, 1.0)`, `cross()` and `P * M * v` are Mesa's fadd
  and FMAs byte for byte and a vertex `fract()` and `inversesqrt()` are
  Mesa's (`7810e44a`, `fc6522a3`, `9ed7b294`); mediump `reflect`, `length`,
  `normalize` and `fract` of a varying's lane compile byte-equal to Mesa
  26.2.1 (`e0574847`, `9794b4a7`, `8ea24220`;
  `docs/notes/mali-fs-vector-builtins.md` tracks the rest); and
  `gl_test` cases 218 and 221 run through the GL core on the mock
  (`68b9cdf2`). Each is checked against Mesa's drm-shim output.
- GC7000: the hardware per-stream instance divisor (V1),
  `max_instance_divisor` 0xffff, its divisor words pinned per stream
  (`6c4759ea`, `308dcf69`); the X2 run is recorded as not measured
  (`46969bd7`). The compressed sRGB ETC2 classes are offered
  (`f2884e0d`).
- PowerVR: `mrt_layout`, the four-target colour layout and its
  end-of-tile program (`a47bcb77`); the vertex program adds a per-draw
  base to `gl_InstanceID` (`a489b1b8`,
  `docs/notes/instanceid-per-draw.md`); a texture swizzle composes
  into the texture state (`2df189df`); and the pvrlab `gl_test` wires
  the ES 3.0 flat-line, `drawrandom` and precision cases 230-242
  (`99f6aaf6`).
- GL core: `msaa_resolve` resolves a rectangle on the stored surface
  layout (`c1288b0c`); the ES 3.0 multisample renderbuffer rules are
  pure functions in `src/gl/msaa.rs` (`230caff1`), a shared
  per-attachment sample store keys by (id, gen) (`d560e749`), and
  `gl_test` case 223 `msaarbo` draws and resolves a 4x renderbuffer FBO
  by a blit (`080b6f9e`); the `msaa-fbo` cargo feature and
  `YAKOGL_MSAA_FBO` exist and have no effect yet (`234b9d4a`).
- STM32MP257F-DK: `gc_fence` and `gc_clear` for the DK's A35, the GPU
  powered through `stm32mp2_gpu.h` (`31e07d90`); `board-stm32mp2.sh`, a
  one-shot runner off its own U-Boot (`41645a3c`); and `ffi::stm32mp2`,
  the C ABI over OP-TEE's SMC watchdog and PSCI reset (`98e268dd`).
- NanoPC-T6: `tools/board.sh` runs the board, kexec-dev through
  `nanopc-t6.sh` (`10347727`); the GPU power-on starts a powered-down
  NPLL and runs from U-Boot's starting state (`eca6e91d`), and `gpu_up`
  applies the GPU QoS policy after the power-on (`54c8f991`); the Z1
  (t6_gpu), Z2 (t6_mmu) and Z3 (t6_fw) probe images read PASS on the
  board (`358401ed`, `1bac1956`, `7aa7e8cb`); the `t6_submit` probe
  prints the Z4a..Z4k record lines `t6-verdict.sh` judges, Z4j and Z4k
  measuring when the firmware publishes the extract and counting the
  drain's acknowledges (`0357ae87`, `b559d36b`, `371b517d`), and Z4
  reads PASS on the board (Results); the Zephyr images' window is
  256 MiB, tied in the dts, `nanopc-t6.sh` and the U-Boot script, with
  no jump while the GIC tables are unknown (`e1264cd1`);
  `driver/tools/uboot-t6/t6-zephyr.cmd`, the draft sign-off route that
  ext4loads a Zephyr image and runs `go`, uses only v2026.07 commands
  and compiles with `mkimage` (`59fee21b`, `e0433194`).
- RK3588: the GPU clock set stays inside its CLKSEL and OPP range
  (`8e415597`) and `QosPolicy` with `qos_apply` drives the four GPU QoS
  blocks (`483a3037`).
- Tools: the redaction scan of `ed76d276`, private classes with masked
  counts, is Part 4 of `docs/release/redaction-audit.md`
  (`tools/release/redaction-scan.py`, `76143e93`); the NanoPC-T6 glmark2
  suite (`tools/cts/mali-glmark2-run.sh` per scene over ssh with a fault
  timeout, `tools/cts/mali-glmark2-preflight.sh`,
  `tools/cts/glmark2-mali-tally.py`) has a host end-to-end self-test in
  the gate (`a6dd0f28`, `a1fa12c5`, `92b93c23`, `48a4d1bf`, `c1b45dee`,
  `e7bb2b3e`); `tools/cts/mali-slice-redact.py` checks dEQP slices for
  addresses, host names and serials (`2e9a9bda`); the gate takes its
  inputs from a dependency layout, `tools/cts/deps.toml`, with LZ4 from
  a pinned source build and `$LZ4` honoured by `am62p-mkboot.py`
  (`8c046e65`, `570cd8c6`); `kexec_zephyr` refuses a file larger than
  64 MiB instead of loading it cut short, and its board self-test
  compares the loaders' window with the dts node (`2ded94b7`,
  `e1264cd1`).
- GL core: the ETC2 and EAC blocks of the ten ES 3.0 compressed formats
  decode on the CPU at upload (`4e539da8`, `src/gl/texture.rs`; the
  design and dEQP's EAC tolerance in `docs/notes/etc2-eac.md`), and
  `gl_test` case 20's compressed-format checks run through the GL core on
  the host (`a8f0ea12`). R32I/UI and RG32I/UI have rows in
  `integer_texture_surface` (`6c5b004f`) and the GC7000's sampled set
  (`b5e04b14`).
- GC7000: tiled 32-bit render targets with early depth behind
  `gc_tiled_rt` (W1-W5, off by default; `9cdeff6d`,
  `docs/notes/gc-tiled-wire.md`); `GLMARK2_GC_TILED_RT` and
  `GL_TEST_GC_TILED_RT` switch it on from glmark2 and `gl_test`
  (`98e847be`); the layout and depth-dump decode of the TS probe
  are pure functions with a host test (`1cdf3d1b`, `b383d647`,
  `docs/notes/gc-ts-probe.md`); and `gc_triangle`, a sample that draws
  one triangle and checks its three apexes (`3bf0cebf`).
- GL core: `src/msaa_hw/`, the module tree of the Valhall 4x FBO
  descriptor and writeback resolve words, pure and unit-tested
  (`fb51ce43`).
- Mali (`gpu-mali`): the -0.0 rules of compares and negated-literal
  products follow Mesa's algebraic rules or refuse by name
  (`721b9df9`..`7b24cebe`), the uniform words pushed are Mesa's loads
  (`125186a5`), and `driver/rust/src/board/nanopc_t6_osal.rs` is a
  Zephyr OSAL over our own kernel side (K1, `4d9c670c`).
- NanoPC-T6: the RK3588 display power domains (`6aefdd49`),
  `ip::dw_hdmi_qp` (`e6f889c2`) and `ip::hdptx` (`6f022b17`) in
  `driver/rust/src/ip/`, with the design in
  `driver/docs/nanopc-t6-display.md`; the Z1, Z3 and Z4 records name the
  GPU QoS policy (`cc5f1750`, `b619b3fa`).
- STM32MP257F-DK: the display chain from LTDC to the ADV7535's HDMI, as
  host-tested IP blocks (`ip::stm32_ltdc`, `ip::dw_mipi_dsi`,
  `ip::stm32_dsi_phy`, `ip::stm32_i2c`, `ip::adv7535`,
  `ip::stm32_gpio`), host models (`model_mp25_display.rs` and
  `model_mp25_i2c.rs` in `driver/rust/src/`), the C ABI `ffi::mp25_display` and
  `ffi::mp25_i2c`, and the Zephyr drivers over them; the
  `mp2_display` probe, `tools/probes/stm32mp257f_dk/display_check/`
  judging its colour bars from a KVM screenshot, and the gl_test,
  glmark2 and dEQP builds for the DK (`cb69bd9b`, `fb94dafe`,
  `b505bdff`). The board runs are still to do; the design is in
  `docs/notes/stm32mp257f-dk-display-bringup.md` and
  `docs/notes/stm32mp257f-dk-bringup.md`.
- STM32MP257F-DK: `driver/tools/uboot-mp2/` runs the DK's typed U-Boot
  one-shot scripts, with a QEMU test on U-Boot v2023.10
  (`90d16804`, `de328573`); `tools/board-stm32mp2.sh` undoes its run on a
  signal, never writes `bootcmd` (`7f285c49`) and keeps a hold after
  the result line (`ac7e0988`).
- Tools: `tools/cts/es2-tree-gate.py` reads a whole dEQP-GLES2 tree's
  stage-1 fields against `es2-accepted.txt` (`99c8882c`);
  `tools/cts/deqp-limits-tally.py` tallies the limit, state-query, UBO
  and varying cases of host-deqp runs (`5554bb62`);
  `tools/cts/x2-scene-table.py` prints the X2 slow-scene rows from
  records (`edab7cf3`); `tools/cts/host-deqp-run.sh --compare` reads indented
  result lines and refuses an empty side (`8816ebb4`, `615c5ba4`).
- Tests: flat shading on V3D and GC7000UL, `tests/es3_flatshade_guard.rs`
  (`bab1374f`); `tests/es3_float_target.rs` pins that a float colour
  target is NotSupported or works, never an incomplete FBO
  (`33e48bc1`, `docs/notes/es3-float-targets.md`); PowerVR int/uint
  outputs from ALU results, conversions and MRT beside a float output;
  and the V3D compiler's uniform block reads past dword 1536
  (`b8c3f49b`).

### Changed

- PowerVR: a depth texture a kick rendered into is sampled by the TPU from
  the ISP's depth buffer (X8U24, Mesa's design), not resolved into R32F
  texels on the CPU first (`docs/notes/pvr-depth-sample.md`, #40). glmark2
  shadow on the AM62P goes from 353 to 10.6 ms a frame and refract from
  376 to 37.0 ms (score 461 to 467); gl_test case 215 `shadowmap` is the
  reproducer (355 to 27.8 ms a frame for its 1600x1200 shadow map).
- PowerVR: a render of a size whose two HWRT datasets are both in flight
  takes a second pair, up to four, instead of waiting for the first kick
  (#42). A mipmap chain that draws its large levels on the GPU and filters
  the tail on the CPU exists as an opt-in (`mip_cpu_tail`, off): on the
  AM62P it was slower than drawing every level on the GPU
  (`docs/notes/am62p-pvr-mipmap-kicks.md`); `gl_test` case 216 `mipchains`
  prints its kicks a frame.
- Build: `capi/assemble.cmake` and the external linkage handle the
  `__rustc::rust_eh_personality` name that recent nightlies reference from
  `compiler_builtins`: the references are read off the archive and renamed
  to `yakogl_rust_eh_personality`, and the build fails if one is left.
  `libyakogl.a` links with `--whole-archive` again on such a nightly.
- Vivante: a render-target change no longer ends the GC kick
  (`e299852c`).
- GL core: `GL_MAX_SAMPLES` answers from `src/gl/msaa.rs`, the back-end
  max fixed at 0 (`2830b218`); `glUniform*ui` on a non-uint uniform of a
  GLSL ES 3.00 program is `GL_INVALID_OPERATION` (`bc200f21`); and
  `glTexSubImage2D/3D` refuse `GL_FLOAT` into a texture with no float
  format (`bece11ec`).
- Vivante: a program that writes `gl_FragDepth` turns early Z off through
  the early-depth rule (`b3f42d14`); dedup's PE room holds the tiled
  target's TS words (`be0c1527`).
- V3D: `texture.rs` pins the sampler compare state of all eight GL
  functions and the non-depth bypass (`753dedbf`).

### Fixed

- V3D: occlusion counters sit at a 1024-byte group each (V3D's TLB counter
  groups), not a 64-byte stride; `gl_test` 196 `occl` read 0 for every
  query on the Raspberry Pi 4.
- GC7000: a user uniform block member is read with a `load` from the
  block's copy in memory, not from the 320-register constant file, when
  its register holds only members; large uniform blocks now link.
- GC7000: `ETC2_RGB` is sampled as Mesa's `EXT_NONE` format, so the T, H
  and planar modes of ETC2 decode (ETC1 blocks decode as before).
- V3D: refusals of legal GL state answer `GL_INVALID_OPERATION`, not
  `-EINVAL` (`b7c1484e`).
- PowerVR: flat varyings take the last (provoking) vertex (`7b39917c`).
- GL core: `texture_has_no_alpha` covers the integer base formats
  (`50b9b6e3`).
- GC7000: a draw whose sampler slots pass slot 31 is refused by name
  instead of writing past the 32-slot array (`b1e22a8d`).
- GL core: `glDeleteBuffers` frees the name at once, so binding it again
  creates a new buffer, and a transform feedback or vertex array object
  that held the deleted buffer keeps its storage (`da0eede3`,
  `docs/notes/es3-tf-lifetime.md`).

## [0.1.0] - 2026-10-27

YakoGL is an OpenGL ES 2.0 and EGL 1.5 library for embedded GPUs that runs
without an operating-system GPU driver. It drives the GPU directly, from
Zephyr or bare metal. It is a `no_std` Rust crate with a C ABI.

### Results

The newest board results are in `tools/cts/RESULTS.md`; this list is the
state on 2026-10-26.

- dEQP-GLES2, the 17,165-case mustpass list, from one main commit,
  `e9901869a` (2026-10-15/16), reads 0 failures on the three boards:
  AM62P 17,140 pass, 18 not supported, 7 warnings (the 1x images with the
  16 `multisample.*` cases of the 4x image); i.MX8MP 17,318 pass,
  34 not supported, 19 quality warnings; Raspberry Pi 4 17,126 pass,
  34 not supported, 5 warnings. Later runs read the same counts: the AM62P
  1x images at `25cce714`, the i.MX8MP at `d0c5dc48`, the Pi 4 at `46f93767`.
  The accepted not-supported cases are listed in `docs/es2-cts-failures.md`,
  section C.
- glmark2-es2 at 800x600 runs all 33 scenes on every board, with 0 failed
  scenes and 0 GL errors. Scores from main `445c5653` (2026-10-20): AM62P
  461, Raspberry Pi 4 193, i.MX8MP 42. The Pi 4 read 195 at `c4af4917`
  (2026-10-22). A new table from one main commit is issue #1196.
- dEQP-GLES3 (OpenGL ES 3.0) is partial: the AM62P's whole tree read
  36,552 of 42,810 pass (2026-10-14), the Pi 4 ran 9 of its 91 images, the
  i.MX8MP has not run it (`ROADMAP.md`, stage 4).

### Added

#### Library
- OpenGL ES 2.0 and EGL 1.5 entry points, built against the Khronos headers
  in `include/`. Applications are ordinary GL ES 2.0 clients.
- The C, CMake and Zephyr builds report ES 2.0 by default
  (`YAKOGL_REPORT_ES2`); the cargo feature `report-es2` does the same for a
  Rust build.
- One GPU back end per build, chosen by one Cargo feature: `gpu-powervr`
  (Imagination PowerVR BXS-4-64, TI AM62P), `gpu-vivante` (Vivante GC7000UL,
  NXP i.MX 8M Plus), `gpu-v3d` (Broadcom V3D 4.2, Raspberry Pi 4) and
  `gpu-mali` (Arm Mali-G610, RK3588; in progress, see below).
- The OS layer behind one interface (`include/gles_osal.h`): a Zephyr OSAL
  per GPU, a bare-metal OSAL whose board hooks are still to be written
  (#1223), and a Linux OSAL for host tests with no GPU.
- EGL window surfaces are double-buffered by default (`EGL_BACK_BUFFER`) on
  a platform that can show a buffer; `EGL_SINGLE_BUFFER` renders in place.
  Pbuffer surfaces are supported too.
- `libyakogl.a` can sit next to other Rust static libraries in one image:
  the Rust runtime and the compiler_builtins libc and libm symbols are made
  local (`capi/assemble.cmake`).
- The ES 3.0 entry points (`include/GLES3/`, `src/gl/es3.rs`), reported when
  `YAKOGL_REPORT_ES2` is off. Transform feedback, uniform blocks, sized and
  integer formats, occlusion queries, `glBlitFramebuffer`, sRGB, buffer
  mapping and pixel buffers are tested on the host for the three GPUs; the
  board state is above.

#### Shader compilers
- Our own `no_std` GLSL ES compiler runs on the board (`src/shaderc/online/`,
  the default), so an application can compile shaders at run time.
- A precompiled tier: the shaders of known applications compiled by the same
  compiler while the image is built (`yakogl-precompile`), in front of the
  compiler or, in an image without the compiler, alone.
- A table of Mesa-compiled shaders for measurement images only
  (`mesa-reference-table`); no product build links it.

#### Boards, examples and samples
- Zephyr board support for the Toradex Verdin AM62P, the Toradex Verdin
  iMX8M Plus and the Raspberry Pi 4 (`driver/`, `zephyr/`). Each board boots
  from its own eMMC or SD card with no host (`docs/standalone-boot.md`).
- Examples: `examples/triangle`, `examples/triangle-rust` and
  `examples/slint` (a Slint UI drawn through YakoGL).
- The test and benchmark images: `gl_test`, dEQP-GLES2 and dEQP-GLES3, and
  glmark2-es2 (`driver/samples/zephyr/`, `tools/cts/`).
- The PowerVR firmware is not in the repository. `driver/tools/mkfwblob.sh`
  turns the firmware file from TI's SDK into a header that is never
  committed.

#### Tools and tests
- `tools/check.sh`, the gate the CI runs (`.github/workflows/gate.yml`):
  the formatter, clippy and the host tests in every feature configuration,
  and the Zephyr image builds.
- Host unit tests on strict mocks of the OS layer and the GPU
  (`src/testutil.rs`), and a host differential harness over a recording OS
  layer (`tools/hostharness/`).
- `tools/board.sh` runs an image on a board and reads its result.
- A licence and SPDX audit (`tools/license_audit.py`).

#### Documentation
- `README.md`, `DESIGN.md`, `ROADMAP.md`, `CONTRIBUTING.md`, `SECURITY.md`
  and a guide to building the benchmark images (`docs/release/building.md`).

### Known limits

- OpenGL ES 3.0 is not complete on any board (stage 4 of `ROADMAP.md`).
- The `gpu-mali` back end runs only on the NanoPC-T6's own Linux, through the
  panthor kernel driver, not on Zephyr yet (#26). Its newest `gl_test`
  sweep reads 98 of 256 cases PASS and 0 FAIL (2026-10-24).
- Slow glmark2 scenes: terrain on the AM62P (91 ms a frame; shadow and
  refract are 10.6 and 37.1 ms since #40), terrain, shadow and refract on
  the i.MX8MP (4, 2 and 2 FPS), terrain on the Pi 4 (162 ms).
- One thread issues every GL and EGL call; the library does not lock
  (`include/gles_osal.h`, "threads").
- Some display and GPU bring-up files were written after GPL-only Linux
  drivers and are still in that form; `docs/source-form.md` lists them.

[Unreleased]: https://github.com/signal-slot/yakogl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/signal-slot/yakogl/releases/tag/v0.1.0
