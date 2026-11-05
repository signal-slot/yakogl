<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pco_shc: the offline shader compiler harness

`yakogl-tools glsl2usc` (tools/glsl2usc.sh) compiles GLSL with glslang to SPIR-V and hands that to
`pco_shc`, a small stand-alone driver of Mesa's PowerVR compiler (PCO). It is
not part of upstream Mesa: the sources here are dropped into a Mesa 26.2.1
tree and built with its build system.

Files (dropped into the tarball tree; the Mesa files themselves are only
ever changed by the two patches):

- `pco_shc.c`            -> `src/imagination/vulkan/pco_shc.c` (the harness)
- `capture_usc.c`        -> `src/imagination/vulkan/` (prints one of Mesa's own
  USC programs -- end-of-tile, clear-attach, a precompiled usclib program --
  for the driver's blobs, see below)
- `capture_smp_fs.c`     -> `src/imagination/vulkan/` (a texture-sample FS)
- `capture_spm_load.c`   -> `src/imagination/vulkan/` (prints Mesa's SPM
  background-load shader of the partial render with its register metadata,
  see below)
- `meson-vulkan.patch`   adds the four executables to
  `src/imagination/vulkan/meson.build`
- `usclib-tex.cl.patch`  `src/imagination/pco/usclib/tex.cl`: textureSize()
  reads the STRIDE image layout (the reason is in the patch)
- `gen_usc_blobs.py`, `usc_blobs.txt`: the driver's USC blobs (below)
- `gen_spm_load.py`, `spm_load/`: the partial render's SPM load shader (below)

Build with `build_pco_shc.sh` (tarball check, the patches, the meson options,
the usclib check; the packages it needs are in its header; about a minute on
8 threads):

    tools/pco_shc/build_pco_shc.sh mesa-26.2.1.tar.xz [<work dir>]

It builds `pco_shc`, `capture_usc` and `capture_spm_load` (all three in
`<work dir>/mesa-26.2.1/build/src/imagination/vulkan/`); `capture_smp_fs` is
one more `ninja -C <work dir>/mesa-26.2.1/build src/imagination/vulkan/capture_smp_fs`
away. Always in a fresh build directory (an old `build` once compiled MRT
shaders to nothing): the script deletes and re-extracts the tree. On a gate
host the build is the `PCO_SHC` entry of `tools/cts/deps.toml`
(`tools/cts/deps-env.sh --build PCO_SHC`), which copies `pco_shc` and
`capture_spm_load` (the `PCO_CAPTURE` entry) into the dependency layout.

## The driver's USC blobs

`driver/src/pvr_usc_{eot,clear_fs,clear_fs2,vs_passthrough}_blob.h` are
generator output, in two steps:

    # needs the Mesa build above; writes tools/pco_shc/usc_blobs.txt
    tools/pco_shc/gen_usc_blobs.py capture <mesa>/build/src/imagination/vulkan/capture_usc
    # needs nothing; writes the four headers from usc_blobs.txt
    tools/pco_shc/gen_usc_blobs.py render

`usc_blobs.txt` lists, per program, the `capture_usc` arguments (which of
Mesa's generators, with which props) and what it printed: the temps count
and the machine-code words. `tools/check.sh` runs `render --check` (the
headers are the rendering of the list, and the clear-attach words are the
listed ones, in Mesa's copy `tests/reference/pvr_internal.rs` and in the
backend's, our encoder's, `src/pvr/internal.rs`); `capture --check` against
a build proves the list is what this Mesa's pco makes. Last full run
(2026-10-01): every program reproduced, usc_blobs.txt sha256
f9a0dc0adff79149a4b4cb713628631082c086ab63b5ec7555f6228e154701fc, identical
with and without usclib-tex.cl.patch (none of these programs samples).

## The partial render's SPM load shader

`spm_load/spm_load.hex` and `spm_load/spm_load.meta.txt` are generator
output too: the USC program the resumed render of a partial render runs as
its background object, which samples the tiles the PR's end-of-tile program
stored to the SPM scratch back into the output registers
(`docs/notes/pvr-render-pool.md`, "Design: the firmware's partial render",
part 3; the shared-register words it reads are
`docs/notes/pvr-pr-spm-const.md`). Mesa's Vulkan driver generates it with
pco at device init (`pvr_uscgen_spm_load`, `src/imagination/vulkan/pvr_usc.c`,
called from `pvr_arch_spm.c`); the target has no pco, so
`capture_spm_load.c` calls that generator once on the host for the AM62P
driver's one configuration, `pvr_spm_load_props { 1 output dword (RGBA8),
0 tile buffers, 1 sample }` on the BXS-4-64 (shader index 0 of Mesa's set
of 20), and prints the machine code with the metadata the PDS program that
kicks it needs (temps, the shared-register slots of the sampler and image
state, the position coefficients, the output register, entry and size; the
keys are explained in the meta file's header):

    # needs the Mesa build above; writes tools/pco_shc/spm_load/
    tools/pco_shc/gen_spm_load.py capture <mesa>/build/src/imagination/vulkan/capture_spm_load
    # needs nothing: decodes the committed words and checks the metadata
    tools/pco_shc/gen_spm_load.py check

`tools/check.sh` runs `check` always (`yakogl-tools usc-dis`, the online
compiler's decoder: every byte in an instruction group, one end on a lone
`mbyp` to pixout0 with `olchk`, one 2D texture sample reading its image
state from sh4 and its sampler from sh0, a `wdf`, the temps and shared
registers the code names within the counts, the size and the metadata
consistent; the documentation decoder `tools/isa-extract/usc_decode.py`
cannot tell this shader's `fmad`/`movc` and `tst` forms apart, having no
`pco_map.py` half, so it is not used), and `capture --check` against the capture
program of a pco build when it has one (`PCO_CAPTURE`, or the one beside
`PCO_SHC` in a Mesa build tree); without one that row is a gap, never a
pass. The committed files are the 2026-11-03 capture (Mesa 26.2.1, the
build of `tools/cts/deps.toml`, LLVM/clang 21.1.8), reproduced byte for
byte by the 2026-11-04 build of the `PCO_CAPTURE` entry on another Ubuntu
host (gcc 15.2.0; this shader uses no usclib, so the LLVM that compiles
the usclib does not reach its words); the words are Mesa's pco output for
Mesa's own source (MIT, NOTICE 1), and the file header names the Mesa
version, the inputs and the device. The Rust table entry the
backend binds, and the PDS program that kicks the shader, are not here
(driver issue #2, items bgload-pds and pr-frame-colour).

Point `PCO_SHC` at `build/src/imagination/vulkan/pco_shc` when running
`tools/glsl2usc.sh`. Every table regeneration must use the same harness build
on every machine: the offline table is keyed by the GLSL text, and the
GLSL2USC_CACHE key includes the harness binary's size and mtime.

Harness features beyond a plain SPIR-V -> USC run: usclib setup (texelFetch,
textureSize, arrays), gl_VertexID/gl_InstanceID system values, PCO_SHC_OUT
(fragment output class: rgba8 / f16x4 / f16 / f32 for float render targets),
PCO_SHC_BLEND (blend state lowered into the FS), and a push-constant range
sized from the block when a load has a dynamic offset (uniform arrays indexed
by a loop counter).

## How the harness compiles, and how to run it by hand

The route is GLSL -> SPIR-V (glslang) -> NIR -> PCO, the same pipeline Mesa's
PowerVR Vulkan driver runs (`src/imagination/vulkan/pvr_arch_pipeline.c`):
`spirv_to_nir` with `pco_spirv_options()`, the common lowering, then
`pco_preprocess_nir`, `pco_lower_nir`, `pco_postprocess_nir`,
`pco_trans_nir`, `pco_process_ir` and `pco_encode_ir`. The driver's
register-allocation helpers (`pvr_alloc_vs_attribs`, `pvr_alloc_vs_varyings`,
`pvr_alloc_fs_varyings`, the push-constant allocation) are copied into the
harness, so the vertex-input, coefficient and shared-register layout is the
real driver's.

One shader:

    glslangValidator -V -S frag shader.frag -o shader.spv
    pco_shc fs shader.spv > shader.bin     # USC bytes on stdout, metadata on stderr
    PCO_SHC_NOBIN=1 PCO_DEBUG_PRINT=fs,vs,internal pco_shc fs shader.spv   # disassembly

Two code generation modes:

- **lean** (the default): the NIR is marked `internal`, so the back end skips
  the run-time-gated Vulkan fragment epilogue (alpha-to-coverage, colour-write
  enable, alpha test, the ISP feedback a full FS reads from a per-draw meta
  word) and the VS implicit point-size and clip/cull outputs.
- **full** (`PCO_SHC_FULL=1`): the full Vulkan pipeline output; the FS reads
  a meta control word from a shared register. The harness forces full mode
  for any FS that uses `discard`, because the coverage kill is only emitted
  on that path (it prints a note on stderr).

The other switches: `PCO_SHC_OUT` (the fragment output class),
`PCO_SHC_BLEND` and `PCO_SHC_COLORMASK` (blend state lowered into the FS),
`PCO_SHC_NOBIN` (no machine code on stdout). The register model the metadata
describes is `driver/docs/gpu-recipes/shader_toolchain.md`, section 2.

