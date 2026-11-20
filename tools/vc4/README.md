<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# VC4 (Raspberry Pi Zero 2 W) reference instruments

Mesa's own driver for the VideoCore IV (gallium `vc4`, Mesa 26.2.1), built on
the host and run without a GPU, as the reference for our VC4 packet and QPU
work (item 1 of `docs/notes/pre-study-rpi-zero-2w.md`, section 8). The same
method as `tools/v3d/drm_shim_ref.sh`. Nothing here is in a gate row: the gate
hosts have no Mesa tree for vc4. `ref_selftest.py` reads only the stored data
and runs in the gate by its name.

## Build

```sh
tools/vc4/drm_shim_ref.sh build <mesa-26.2.1.tar.xz> <work dir>
```

Extracts the tarball (the one `third_party/mesa-26.2.1/UPSTREAM` names) into
the work directory, adds `vc4_shc.c` and its meson target
(`mesa-vc4_shc.patch`), builds the vc4 driver, EGL, GLES 2 and the drm-shim
(`libvc4_noop_drm_shim.so`) and installs them into `<work dir>/inst`, builds
`vc4_shc`, and compiles the submit recorder into
`<work dir>/inst/libvc4_submit_rec.so`. Needs meson, ninja, a C compiler,
python3 with mako and pyyaml, libdrm and expat, and glslangValidator for
`shc`.

## Run

```sh
tools/vc4/drm_shim_ref.sh run <work dir> clear|triangle|flat <out prefix>
tools/vc4/drm_shim_ref.sh shc <work dir> <vs.vert> <fs.frag> <out prefix> [vc4_shc options]
```

`run` draws one `tools/v3d/gl_ref.py` case on the shim
(`MESA_LOADER_DRIVER_OVERRIDE=vc4`, `VC4_DEBUG=cl,qpu`) with the recorder
loaded in front of the shim. `shc` compiles a GLSL 450 pair to OpenGL SPIR-V
(`glslangValidator -G`) and runs `vc4_shc` on it. The output files are
described in `testdata/README.md`.

## How the bytes are recorded (Q9 of the pre-study)

The shim stubs the submit: `drm-shim/vc4_noop.c` has no `DRM_VC4_SUBMIT_CL`
entry, so the shim answers it `-EINVAL` (its log line `unhandled driver DRM
ioctl 0` is in each `.log`). `VC4_DEBUG=cl` prints the binner list as text,
but not the shader records, the uniforms or the BOs, and not as bytes. So
`vc4_submit_rec.c`, a small `LD_PRELOAD` library placed before the shim,
takes `ioctl` and `mmap`:

- on `DRM_IOCTL_VC4_SUBMIT_CL` it writes `struct drm_vc4_submit_cl` (the four
  user pointers set to 0), the binner list, the shader records, the uniforms
  and the handle table as raw bytes, and every BO the table names, then hands
  the ioctl to the shim unchanged;
- shader BOs: it copies the code from `DRM_IOCTL_VC4_CREATE_SHADER_BO`'s
  `data`, because the shim keeps no copy;
- other BOs: it reads them through Mesa's own mapping (`MMAP_BO` gives the
  offset, the `mmap` of that offset the address), up to the last non-zero
  byte (the vertex upload buffer is 1 MiB).

We chose raw bytes as the primary form because later items compare encoders
byte for byte; the `VC4_DEBUG=cl,qpu` text is kept beside them for reading.

## vc4_shc

`vc4_shc vs.spv fs.spv [--prim triangles|lines|points] [--attr-components
N,N,...] [--color-format rgba8|bgra8|rgb565] [--no-threaded-fs]` prints the
fs, vs and cs QPU words and uniform streams as JSON, and the disassembly on
stderr. It needs no device and no shim. `vc4_shader_ntq` is static in
`vc4_program.c`, so the tool does not call it directly: it fills the part of
a `vc4_context` that the compile reads (one RGBA8 colour buffer, no depth,
blending off, the vertex elements), and calls the driver's public functions
`vc4_program_init`, `create_vs_state`/`create_fs_state` and
`vc4_update_compiled_shaders`. These build the keys and compile through
`vc4_get_compiled_shader` and `vc4_shader_ntq`, as a draw does. The one
kernel call on that path, `vc4_bo_alloc_shader`, is replaced at link time
(`-Wl,--wrap`) by a copy into memory. The front end (SPIR-V to NIR, the
st/mesa passes) is `tools/v3d/v3d_shc.c`'s with vc4's screen caps, and the
NIR options are the driver's own (`vc4_screen_get_compiler_options`).

## What was checked (2026-11-20)

- The build reproduces from a clean work directory (the `build` form, 1020
  build steps, then `vc4_shc`).
- All three cases draw with `glGetError 0x0` and one submit each. A second
  `run` of each case gives byte-identical files (all of them, `.log`
  included). Nothing had to be made stable except the four user pointers;
  BO handles (the shim hands out the lowest free one) and the out syncobj
  repeat.
- Each recorded binner list decodes with `vc4_packet.h`'s opcode and size
  table, ends exactly at `bin_cl_size` and with `FLUSH`, and holds one
  `GL_SHADER_STATE` per shader record (`ref_selftest.py`, 174 checks; a
  flipped opcode makes it fail).
- `vc4_shc`'s fs, vs and cs words for the triangle and flat programs are
  byte for byte the shader BOs the GL driver submitted for the same draw, so
  the SPIR-V front end and the default keys match the GLSL path on these two
  programs.

Not checked: the render control list (the kernel builds it from the submit's
surface fields; the shim has none), any program with textures, discard,
points or lines, and `vc4_shc` on the gl_test corpus (item 2 of the
pre-study). Both fragment shaders came out single-threaded although the
shim reports threaded FS support: Mesa falls back when the shader has no
texture fetch to switch threads on (`vc4_shader_ntq`).
