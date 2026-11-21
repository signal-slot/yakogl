<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# VideoCore IV (Raspberry Pi Zero 2 W) host reference

Item 1 of `docs/notes/pre-study-rpi-zero-2w.md` section 8. No board is
involved. Mesa 26.2.1's own `vc4` GL driver runs on the host, on Mesa's
drm-shim, and we record what it would hand the kernel: the binner control
list, the shader records, the uniforms, the BO handle table, the render
fields and the QPU words. A later item golden-tests packet and QPU encoders
against these bytes.

| File | What it is |
|---|---|
| `drm_shim_ref.py` | builds Mesa with the vc4 driver and the shim; draws a case and records it |
| `vc4_submit.py` | reads a record: checks it, prints it, cuts it into raw parts |
| `vc4_shc.py` | QPU words and uniform streams of a vertex and fragment shader pair, as JSON |
| `vc4_selftest.py` | checks the two tools above on the stored data; needs no Mesa |
| `../testdata/vc4/` | the stored reference data for a clear and a triangle |

## Build

```sh
tools/vc4/drm_shim_ref.py build <mesa-26.2.1.tar.xz> <work dir>
```

The tarball is the one `third_party/mesa-26.2.1/UPSTREAM` names (`tarball:`);
the script refuses a file whose sha256 is not that file's `tarball-sha256`.
It extracts the tree into `<work dir>/mesa-26.2.1` (deleted first), builds
`-Dgallium-drivers=vc4` with EGL, GLES 2 and `-Dtools=drm-shim`, and installs
into `<work dir>/inst` (`libvc4_noop_drm_shim.so`, `libEGL.so.1`,
`libGLESv2.so.2`). It needs meson, ninja, a C and a C++ compiler, bison,
flex, python3 (3.12 or later) with mako and pyyaml, libdrm, expat and zlib.

Before the build the script makes three small edits in the extracted tree.
Each stops the build if its place in the source is not found exactly once.
Nothing of Mesa is stored in this repository.

1. **The submit recorder** (`src/broadcom/drm-shim/vc4_noop.c`). See "How the
   bytes are recorded" below.
2. `DRM_VC4_WAIT_SEQNO` and `DRM_VC4_WAIT_BO` answer 0. The driver waits on a
   BO before it maps one that a job used (a texture upload), and nothing ever
   runs on the shim.
3. Mesa's own dump of each uniform word as a draw writes it
   (`vc4_uniforms.c`, in the source under `if (false)`) is switched on with
   `VC4_DEBUG=qpu`. It names the shader by its number instead of its address,
   so the text is the same on every run.

## Run

```sh
tools/vc4/drm_shim_ref.py run <work dir> clear    <out prefix>
tools/vc4/drm_shim_ref.py run <work dir> triangle <out prefix>
```

The cases are those of `tools/v3d/gl_ref.py`, which is used unchanged: an EGL
surfaceless display, a GLES 2 context, a 64x64 RGBA8 framebuffer object, a
blue clear, and for `triangle` one triangle with a colour per vertex. The
script sets `LD_PRELOAD` to the shim and `MESA_LOADER_DRIVER_OVERRIDE=vc4`.
It draws the case twice, once with `VC4_DEBUG=cl` and once with
`VC4_DEBUG=qpu,qir`, and fails unless both draws record the same bytes.

| Output | What it is |
|---|---|
| `<out prefix>.rec.bin` | the record: raw bytes, in the order of the driver's ioctls |
| `<out prefix>.cl.txt` | Mesa's text dump of the binner list (`vc4_cl_dump.c`) |
| `<out prefix>.qpu.txt` | Mesa's dump of each shader (final QIR, QPU words with disassembly) and of the uniform words; not written for `clear`, which compiles no shader |
| `<out prefix>.decode.txt` | `vc4_submit.py decode` of the record |
| `<out prefix>.stdout` | `gl_ref.py`'s two lines (renderer string, `glGetError`) |

## How the bytes are recorded (Q9 of the note)

Mesa's shim README says the vc4 submit ioctl is stubbed. In 26.2.1 it is not
even that: the shim's ioctl table has no entry for `DRM_VC4_SUBMIT_CL`. Run
as Mesa ships it, the shim prints `DRM_SHIM: unhandled driver DRM ioctl 0`,
the ioctl fails, and the driver warns "Draw call returned Success.  Expect
corruption." (seen with the handler taken out again; `glGetError` stays 0).
The arguments still reach the shim, so the bytes can be taken there.

We chose a handler added to the shim, not a second `LD_PRELOAD` and not the
text dump alone:

- The shim is already the one place every vc4 ioctl goes through. A second
  preloaded library would have to interpose `ioctl` under the shim's own
  interposer.
- `VC4_DEBUG=cl` is text, and it is printed before the driver appends the
  last two packets of the list (`INCREMENT_SEMAPHORE`, `FLUSH`, `vc4_job.c`).
  It has no shader records, no uniforms and no handle table.
- The handler sees exactly what the kernel would: `struct drm_vc4_submit_cl`
  and the four blobs it points at, and the QPU words of each shader as the
  driver passes them to `DRM_VC4_CREATE_SHADER_BO`.

The text dumps are kept beside the record for reading.

The record is a sequence of entries: a 4-byte tag, a little-endian `u32`
length, then that many bytes.

| Tag | Bytes |
|---|---|
| `CRBO` | `struct drm_vc4_create_bo` (16 bytes) with the handle the shim gave |
| `SHBO` | `struct drm_vc4_create_shader_bo` (24 bytes, `data` set to 0), then the QPU words |
| `SUBM` | `struct drm_vc4_submit_cl` (176 bytes, the four pointers set to 0), then the binner list, the shader records, the uniforms and the BO handle table, each as long as the struct says |

Only the user-space pointers are changed, because they differ from run to
run. BO handles are small integers the shim gives in order, so they are
stable. `vc4_submit.py extract <file.rec.bin> <out prefix>` writes each part
as a raw file for a test that wants to include one.

`vc4_submit.py` refuses a record whose binner list holds an opcode that is
not in `kernel/vc4_packet.h`, whose list does not end exactly at
`bin_cl_size`, or whose shader records do not end exactly at
`shader_rec_size`. Its opcode and size table is compared with the header of
the built tree on every `run` (`vc4_submit.py check-header`).

## vc4_shc

```sh
tools/vc4/vc4_shc.py <work dir> <vertex.glsl> <fragment.glsl> > out.json
tools/vc4/vc4_shc.py --from <out prefix of an earlier run> > out.json
```

It prints the QPU words and disassembly of the fragment, vertex and
coordinate shaders, and for each the uniform stream: what every word holds
(`constant`, `unif[n]`, `vp_x_scale`, `tex_p0[n]`, ...) and the value the
draw submitted.

**It is not a standalone compiler driver, and there is no `vc4_shc.c`.** The
pair is compiled by the driver itself during one draw on the shim
(`drm_shim_ref.py shaders`: `gl_ref.py`'s triangle with the pair's sources).
The reasons, from Mesa 26.2.1 `src/gallium/drivers/vc4/`:

- `vc4_shader_ntq` is `static` in `vc4_program.c` (line 2180) and no header
  declares it. Its one caller, `vc4_get_compiled_shader`, is static too and
  is reached only from a draw and from the `shaderdb` precompile. v3d's
  compiler is a library with a public entry (`src/broadcom/compiler`);
  vc4's is part of the driver.
- It takes a `struct vc4_context`. The key holds a `vc4_uncompiled_shader`
  that only the context's `create_fs_state` hook makes. The fragment key is
  built from the bound blend, depth-stencil, rasterizer and framebuffer
  state (`vc4_update_compiled_fs`). The vertex key points at the fragment
  shader's input set, which lives in a hash set of the context.
- The input NIR comes from Mesa's GLSL front end and state tracker. vc4 has
  no SPIR-V path, so the front half `tools/v3d/v3d_shc.c` transcribes does
  not apply. A standalone tool would have to copy the state tracker's passes
  and include `vc4_program.c` to reach the static function. Its words would
  then be those of our imitation of the driver, not the driver's.

What the draw gives instead is exactly what Mesa's GL driver compiles for a
GLSL ES 1.00 pair, with the keys of a real draw. The limits: the keys are
those of that one draw (triangles, no blending, no depth, an RGBA8 target,
uniforms at zero, no texture bound), and a pair Mesa's GLSL compiler or
linker refuses gives no output.

The uniform stream order is not the order of the QIR dump: the QPU scheduler
orders the uniforms again and prints nothing (`vc4_qpu_schedule.c`). In the
stored triangle the coordinate shader reads `vp_y_scale` before `vp_x_scale`
while its QIR lists x first. That is why the build switches on Mesa's dump
of the uniform words. `vc4_shc.py` fails unless that dump agrees with the
QIR (the same uniforms), with the record (the same values) and with
`uniforms_size`.

## What was checked (2026-11-21, x86-64 host, Mesa 26.2.1)

- **Clean build.** Two `build` runs into two empty work directories, each
  green: 1020 ninja steps, a work directory of 840 MB.
- **Same bytes.** `run clear` and `run triangle` with both builds, and twice
  with one build: all 10 output files are byte-identical each time. Every
  `run` also compares its own two draws.
- **The list decodes.** `clear`: 21 bytes, 5 packets. `triangle`: 92 bytes,
  16 packets, one shader record of 52 bytes after its 5 handle indices, 8
  uniform words. Every opcode is in `vc4_packet.h` (39 opcodes and sizes
  compared with the header) and each list ends exactly at `bin_cl_size`.
  Mesa's text dump names the same packets at the same offsets, up to the two
  the driver adds after the dump.
- **QPU words.** The words in Mesa's `VC4_DEBUG=qpu` text are the bytes
  recorded from `DRM_VC4_CREATE_SHADER_BO`: 13 (fragment), 25 (vertex) and 23
  (coordinate) for the triangle.
- **`vc4_shc.py` on more shaders.** The 23 pairs of `tools/shaders/es100/`
  that share a file stem: 20 give JSON (488 fragment, 648 vertex and 551
  coordinate words, 463 uniform words), byte-identical with both builds. 3
  are refused by Mesa's GLSL front end and are not programs as paired
  (`deriv`: a compile error; `loop_nested`, `ret_nested`: the fragment
  shader's varying has no match in the vertex shader).
- `python3 tools/vc4/vc4_selftest.py`: 33 checks on the stored data. The
  gate runs it by its name.

## What was not checked

- Nothing ran on a GPU or in a kernel. The record is what the driver hands
  the kernel, before the kernel validates it and before the kernel builds the
  render list (`kernel/vc4_render_cl.c`). The render list is not recorded;
  the record has only the fields the kernel builds it from.
- The contents of the BOs (the vertex data, the texture data) are not
  recorded, only their handles and sizes.
- The shim reports one device: `V3D_IDENT0` `0x2000000`, `IDENT1` 1, with
  branches, ETC1, threaded fragment shaders and fixed render order supported.
  The values a Zero 2 W reports are not known yet.
- `vc4_shc.py` covers the keys of one draw (above). Blending, depth, points,
  lines and bound textures change the shaders and are not covered.
- The recorded build is x86-64. The driver's NEON paths are not built.
- The gate has no row that builds Mesa for vc4. A change to
  `drm_shim_ref.py`'s build or run is checked by hand, with the commands
  above and `tools/testdata/vc4/README.md`.
