<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# VideoCore IV reference data

What Mesa 26.2.1's `vc4` GL driver submits for a clear and for one triangle,
taken on the host on Mesa's drm-shim (`tools/vc4/README.md`). No GPU and no
kernel driver took part. A packet or QPU encoder for this GPU can be tested
against these bytes; `tools/vc4/vc4_selftest.py` checks that they still read.

Made on 2026-11-21 with these commands, from the repository root, after
`tools/vc4/drm_shim_ref.py build <mesa-26.2.1.tar.xz> <work dir>`:

```sh
tools/vc4/drm_shim_ref.py run <work dir> clear    tools/testdata/vc4/clear
tools/vc4/drm_shim_ref.py run <work dir> triangle tools/testdata/vc4/triangle
tools/vc4/vc4_shc.py --from tools/testdata/vc4/triangle > tools/testdata/vc4/triangle.shc.json
```

The draws are `tools/v3d/gl_ref.py clear` and `triangle`: a 64x64 RGBA8
framebuffer object, a clear to blue, and for `triangle` one triangle with a
colour per vertex (interpolated, not flat-shaded).

| File | Made by | What it is |
|---|---|---|
| `<case>.rec.bin` | the recorder in the shim | raw bytes: each BO the driver created, each shader's QPU words, then `struct drm_vc4_submit_cl` with the binner list, the shader records, the uniforms and the BO handle table. The layout is in `tools/vc4/README.md` |
| `<case>.cl.txt` | Mesa, `VC4_DEBUG=cl` | Mesa's text dump of the binner list. It is printed before the driver adds the last two packets (`INCREMENT_SEMAPHORE`, `FLUSH`), which are in `.rec.bin` only |
| `triangle.qpu.txt` | Mesa, `VC4_DEBUG=qpu,qir` | for each of the three shaders the QIR before and after optimisation and the QPU words with their disassembly; then the uniform words as the draw wrote them (`uniform <shader>/<word>: <value> <what>`). `clear` compiles no shader and has no such file |
| `<case>.decode.txt` | `tools/vc4/vc4_submit.py decode <case>.rec.bin` | the record as text: the struct's fields, the list packet by packet, the shader record, the uniform words |
| `<case>.stdout` | `tools/v3d/gl_ref.py` | the renderer string (`VC4 V3D 2.1`) and `glGetError` |
| `triangle.shc.json` | `tools/vc4/vc4_shc.py --from` | the fragment, vertex and coordinate shaders' QPU words and uniform streams |
| `*.license` | by hand | the licence of the file beside it |

The numbers, for a first look:

| | `clear` | `triangle` |
|---|---|---|
| `.rec.bin` | 233 bytes | 1032 bytes |
| binner list | 21 bytes, 5 packets | 92 bytes, 16 packets |
| shader records | none | 1: 5 handle indices and 52 bytes |
| uniforms | none | 8 words (vertex 4, coordinate 4) |
| QPU words | none | fragment 13, vertex 25, coordinate 23 |
| BO handle table | 1 | 5 |

What is the same on every run, and what was made so:

- The four user-space pointers of `struct drm_vc4_submit_cl` (`bin_cl`,
  `shader_rec`, `uniforms`, `bo_handles`) and the `data` pointer of
  `struct drm_vc4_create_shader_bo` differ from run to run. The recorder
  stores them as 0. Nothing else is changed.
- BO handles are the shim's, given in order from 1, so the handle table and
  the handle indices are stable.
- Mesa's dump of the uniform words names the shader by a pointer. The build
  changes that to the shader's number.
- The addresses in the shader record and the list are offsets into BOs. The
  kernel would turn them into bus addresses; here they stay offsets.

To regenerate, run the three commands and compare with `git diff`. A second
run and a second clean build gave the same bytes for every file (2026-11-21).
