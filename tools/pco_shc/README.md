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
- `capture_spm_load.c`   -> `src/imagination/vulkan/` (prints the partial
  render's background-object load shader, Mesa's `pvr_uscgen_spm_load`,
  with its metadata; see below)
- `meson-vulkan.patch`   adds the four executables to
  `src/imagination/vulkan/meson.build`
- `usclib-tex.cl.patch`  `src/imagination/pco/usclib/tex.cl`: textureSize()
  reads the STRIDE image layout (the reason is in the patch)
- `gen_usc_blobs.py`, `usc_blobs.txt`: the driver's USC blobs (below)
- `gen_spm_load.py`, `spm_load/`: the partial render's load shader (below)

Build with `build_pco_shc.sh` (tarball check, the patches, the meson options,
the usclib check; the packages it needs are in its header; about a minute on
8 threads):

    tools/pco_shc/build_pco_shc.sh mesa-26.2.1.tar.xz [<work dir>]

It builds `pco_shc` and `capture_spm_load`; `capture_usc` and
`capture_smp_fs` are one more
`ninja -C <work dir>/mesa-26.2.1/build src/imagination/vulkan/capture_usc`
away. Always in a fresh build directory (an old `build` once compiled MRT
shaders to nothing): the script deletes and re-extracts the tree.

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

## The partial render's background-object load shader

`spm_load/spm_load.hex` and `spm_load/spm_load.meta.txt` are generator
output too: Mesa's SPM load shader, `pvr_uscgen_spm_load()`
(`src/imagination/vulkan/pvr_usc.c`, MIT), as Mesa 26.2.1's pco compiles it
for the BXS-4-64, with `struct pvr_spm_load_props { output_reg_count 1,
tile_buffer_count 0, is_multisampled false }` (the backend's one
configuration: an RGBA8 colour target, one sample, no tile buffers;
`pvr_uscgen_spm_load_index` 0). A render that resumes after a partial render
runs it as the background object (`pds_pr_bgnd` of the 3D command): it
samples the tile the partial render stored to the SPM scratch and writes the
raw dword to pixel output 0, so the resumed render starts from the stored
colour (docs/notes/pvr-render-pool.md, "Design: the firmware's partial
render", part 3). Mesa generates it at device creation
(`pvr_arch_spm.c` `pvr_arch_device_init_spm_load_state`); we have no pco on
the target, so it is generated once here. The committed words are a table
entry in a later change; nothing under `src/` reads these files.

    # needs the Mesa build above (capture_spm_load); writes tools/pco_shc/spm_load/
    tools/pco_shc/gen_spm_load.py capture <mesa>/build/src/imagination/vulkan/capture_spm_load
    # needs no Mesa: yakogl-tools usc-dis decodes the words and the metadata is checked against them
    tools/pco_shc/gen_spm_load.py check

`spm_load.meta.txt` is what the compiler says about the program, one
`key value` line each (`capture_spm_load.c` prints them): `usc_bytes`,
`entry_offset`, `temps`, the register counts, the colour output (`fs_out`:
`FRAG_RESULT_DATA0` in po0, `PIPE_FORMAT_R32_UINT`), and the shared-register
slots the program reads, `enum pvr_spm_load_data`
(`src/imagination/common/pvr_iface.h`): the sampler state at
`sh[smp_state_sh..+texstate_dwords)` = sh0..3, the scratch texture's state at
`sh[tex_state_sh..+texstate_dwords)` = sh4..7, `consts_dwords` = 8 dwords in
all, which the PDS program DMAs before the kick. Captured 2026-10-29 on
Gentoo (gcc 16.2.1_p20260926, LLVM/clang 23.1.2): 50 dwords, 16
instruction groups, temps 6, no coefficients, no usclib call; the Ubuntu
build (2026-10-31, gcc 15.2.0, LLVM/clang 21, `deps-env.sh --build
PCO_SPM_LOAD_CAPTURE`) reproduces both files byte for byte.

What the PDS and state side has to supply, read from Mesa's
`pvr_arch_spm_init_bgobj_state` (`pvr_arch_spm.c`, MIT), for the change
that builds the background-object program:

- sh0..3, the sampler (`TEXSTATE_SAMPLER_WORD0`): `non_normalized_coords`,
  u and v `CLAMP_TO_EDGE`, min and mag filter `POINT`, `minlod` and `maxlod`
  `CLAMP_MIN`, `dadjust` `DADJUST_ZERO_UINT`; word 1 all zero.
- sh4..7, the scratch as a texture (`pvr_spm_setup_texture_state_words`):
  `VK_FORMAT_R32_UINT` for one dword, linear layout, a 2D view of the
  framebuffer's width and height, one mip level, the frame's sample count,
  `stride` = the width rounded up to
  `ROGUE_CR_PBE_WORD0_MRT0_LINESTRIDE_ALIGNMENT`, at the scratch's device
  address; `PVR_TEXTURE_STATE_STORAGE`.
- The kick: DOUTU of the program at its heap address with `temps` from the
  metadata and `SAMPLE_RATE_INSTANCE` (the program does no sample shading).
- `CR_PDS_BGRND3_SIZEINFO` and the exact constant dwords for 800x600
  RGBA8: docs/notes/pvr-pr-bgload.md (section 4) and
  docs/notes/pvr-pr-spm-const.md. Mesa passes the constants size in bytes
  where the unit is dwords (`usc_sharedsize` 2); the design transcribes 8
  dwords (`usc_sharedsize` 1), board-checked, so this file does not repeat
  the words.

The two rows of `tools/check.sh`: `gen_spm_load.py check` (every pvr run;
no Mesa) and `gen_spm_load.py capture --check` against `capture_spm_load`
(the full run). The program is `tools/cts/deps.toml`'s
`PCO_SPM_LOAD_CAPTURE`, built beside `pco_shc` in the dependency layout
(`tools/cts/deps-env.sh --build PCO_SPM_LOAD_CAPTURE`; a host prepared
before the entry shows it `missing` in `deps-env.sh --check`); without the
variable the row looks beside `PCO_SHC` (this README's build directory,
or the layout's `PCO_SHC.src` tree). Without `PCO_SHC`, or with a `pco_shc`
and no `capture_spm_load`, the row is a gap naming the build, never a
pass.

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

