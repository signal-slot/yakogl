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
- `meson-vulkan.patch`   adds the three executables to
  `src/imagination/vulkan/meson.build`
- `usclib-tex.cl.patch`  `src/imagination/pco/usclib/tex.cl`: textureSize()
  reads the STRIDE image layout (the reason is in the patch)
- `gen_usc_blobs.py`, `usc_blobs.txt`: the driver's USC blobs (below)

Build with `build_pco_shc.sh` (tarball check, the patches, the meson options,
the usclib check; the packages it needs are in its header; about a minute on
8 threads):

    tools/pco_shc/build_pco_shc.sh mesa-26.2.1.tar.xz [<work dir>]

It builds `pco_shc` only; `capture_usc` and `capture_smp_fs` are one more
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
headers are the rendering of the list, and the clear-attach words
`src/pvr/blobs.rs` carries are the listed ones); `capture --check` against
a build proves the list is what this Mesa's pco makes. Last full run
(2026-10-01): every program reproduced, usc_blobs.txt sha256
f9a0dc0adff79149a4b4cb713628631082c086ab63b5ec7555f6228e154701fc, identical
with and without usclib-tex.cl.patch (none of these programs samples).

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
