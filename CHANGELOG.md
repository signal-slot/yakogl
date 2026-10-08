# Changelog

All notable changes to YakoGL are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
There is no released version yet; everything below is unreleased.

## [Unreleased]

### Added

#### GL core
- OpenGL ES 2.0 and EGL for embedded GPUs without an OS GPU driver (afb16335).
- `GL_OES_texture_npot` in ES 2.0 builds; non-power-of-two textures get mip chains (2673197d).
- Draw-step timing: each step of a draw is timed and the frame's slowest is reported (b0a9c0c8, 068cb628).

#### PowerVR (TI AM62P)
- Predicated `if` arms, as pco does, and an optional branched `if` where skipping an arm pays (afa12e9b, 97ba03d4, 5f4d3677).
- Fused multiply-add and varying-product fusion in the online compiler (18972dcb, 39bd6983).
- Non-power-of-two 2D textures get real mip chains, laid out at their power-of-two extent as Mesa's PowerVR driver does; this addresses the aliasing seen in glmark2 terrain's minified far field (cc788590).

#### Vivante GC7000 (NXP i.MX8MP)
- Host reproduction and board recipe for the 4x MSAA defect (6bddfc53).
- Host-side root-cause tests for the nondeterministic missing colour writes (84094196).

#### V3D (Raspberry Pi 4)
- Non-seamless cube-map sampling in ES 2.0 builds, with a design note and host tests (ec5ba4fb, c3e4146d).

#### Shader compiler
- In-process compiler is the default back end (84b5761e).
- Precompiled-shader tier in front of the compiler, plus a Kconfig option for images without the compiler (17605c36, 17e7d34c).
- Generator that precompiles the shader tables with our own compiler (68563f54, 05f8f749).
- GLSL ES 3.00 coverage of the online compiler, measured over the dEQP-GLES3 shader corpus; `report-es2` builds still refuse `#version 300 es` (4ab482e5).

#### Tools and CTS
- ES2 CTS failure inventory per board, with dEQP image recipes (8e406f79, e6e5f148).
- Host harness: glmark2's default set runs on the host for PowerVR and Vivante, with shaders and GL errors captured (69a14e24).
- Host harness: the V3D back end runs on osal-linux, with a glmark2 run for the Pi 4 (505ac0eb, e7c314ac).
- Host harness: the scen and hostile program sets run on the V3D build too, with fail-injection offsets per GPU (93a36d33).
- Host replay of glmark2's V3D allocations against the 96 MiB carve-out (ac0c2544).
- glmark2's own shaders compile on all three back ends, checked as a standalone cargo test (abf8c700).
- Provenance record for the femtovg shader corpus variants, checked byte-exact against upstream femtovg 0.26.0 (2563c275, 8d092785).
- `check.sh` builds one Zephyr `gl_test` image per GPU with `-Werror` on our C files (990dbd32).
- Many new `gl_test` cases that measure GPU cost (for example 184-195).

#### Boards and samples
- AGL cluster joins the common set: built, run and tallied on every board (c7c80ba1).
- Per-board benchmark inventory and baselines under `docs/`.
- A release building guide lists every input of the benchmark images and the build line per board (`docs/release/building.md`, f3da7e9e).
- CONTRIBUTING.md and SECURITY.md (b4ec4273).

### Changed

#### GL core
- Log lines a draw can reach are logged once per cause, with repeats counted on the close line (a138f73f).
- Depth/stencil buffers: 32 slots on every GPU, memory is the real bound, least recently used is given up first (2b219c5f).

#### PowerVR (TI AM62P)
- Large GENERAL blocks are taken from the end of the carve-out (541c9d3b).

#### Vivante GC7000 (NXP i.MX8MP)
- Debug knobs leave the product; MRT, vertex-stage tiled copy and INT_FILTER are the design (260711b9, 72d16864).
- Display runs at Linux's 69.3 MHz (c1f0e2a6).

#### Shader compiler
- The Mesa reference moves out of product code into tests (dea95deb).

#### Boards and samples
- Pi 4 glmark2 build uses a 96 MiB carve-out (ca4074a0).
- Website shows one front page with the current state (48866111).

### Fixed

#### GL core
- Refuse a `DEPTH_COMPONENT24` renderbuffer; ES 2.0 has no `MAX_DRAW_BUFFERS` (4e9397c6).
- ES 2.0 CTS: the failures recorded on the three boards have host-side fixes; the confirming board runs are pending (`docs/es2-cts-failures.md`).

#### PowerVR (TI AM62P)
- glmark2 refract's 7.6 MB depth target now fits the pool (541c9d3b).
- An attribute read only in code the unroller drops keeps its registers (c6aa6ba1).

#### Vivante GC7000 (NXP i.MX8MP)
- Constant register padding lanes are unused instead of uniform dwords (ba88a0da).

#### V3D (Raspberry Pi 4)
- Cube maps sample correctly without seamless filtering (ec5ba4fb).

#### Tools and CTS
- Host-harness interposer clamps a negative `glGetAttachedShaders` count (7ad89534).
- Cluster `check_ui` finds the generated file under cargo's new build layout (d7d09a29).
- Archive copy is made writable before `objcopy` (c68dba8e).
- `gl_test` case 171: the raw-memory check follows the window's byte order; the GC7000 stores RGBA8888 as BGRA (41f56eb7).

#### Boards and samples
- glmark2 terrain no longer samples black with non-power-of-two textures (2673197d).
- Pi 4: glmark2 refract and terrain fit V3D memory (ca4074a0).
