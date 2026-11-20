<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gltest-mesa71: Mesa's offline V3D 7.1 programs for the first six gl_test cases

These are the programs that gl_test cases 5, 58, 59, 66, 67 and 102 draw with,
plus the back end's clear quad. Mesa's v3d compiler built them for the
Raspberry Pi 5's V3D 7.1.10. They are the fallback that P9
(`docs/notes/pre-study-rpi5.md` section 8, `docs/notes/v3d71-gl-table.md`)
allows wherever our online compiler (`src/shaderc/online/v3d/*71.rs`) cannot
build a program yet. The Pi 4 started the same way
(`docs/notes/pi4-v3d-bringup.md`, "gl_test on the V3D"). They are reference
binaries, not a product table. `tools/v3d/check71.d/gltest_mesa71.sh`
regenerates them and compares them byte for byte.

## The programs

| name | vertex shader | fragment shader | drawn by | fs / vs / cs instructions | record flags |
| --- | --- | --- | --- | --- | --- |
| `color_uv` | `tools/shaders/gl_test_es100/color_uv.vert` | `tools/shaders/gl_test_es100/color_uv.frag` | `gl_test_vs_src` + `gl_test_fs_src` of `driver/tools/pvrlab/gl_test_shaders_es100.h`, the only program cases 5, 58, 59, 67 and 102 link (`link_program` in `case_scissor`, `case_clearscis`, `case_clearmask`, `case_fbods`, `case_maskclear`) | 12 / 26 / 15 | `0x208002`, 6 varyings |
| `clear` | `src/v3d/backend.rs` `CLEAR_VS` | `src/v3d/backend.rs` `CLEAR_FS` | the back end's clear quad, for a clear the TLB cannot do (the 4.2 table draws scissored, stencil-masked and colour-masked clears with it: `docs/notes/pi4-v3d-bringup.md`) | 6 / 15 / 15 | `0x200002`, 0 varyings |

Case 66 (`case_clearvp`) links no program: it only clears. A V3D build takes
the ES 1.00 shader set (`gl_test.c`: `CONFIG_YAKOGL_SHADERC_ONLINE` or
`CONFIG_YAKOGL_PRECOMPILED_SHADERS` include `gl_test_shaders_es100.h`), and
that header is generated from the `tools/shaders/gl_test_es100/` files used
here. The clear sources are read out of `src/v3d/backend.rs` on every run, so
a change to the back end's clear shaders shows up as a difference.

Every program of the six cases is here. Mesa refused none, so no program needs
a "could not build" line. Not here, because none of the six cases uses them:
the integer clear quad (`CLEAR_VS_INT` + `CLEAR_FS_INT`, for integer colour
targets) and the several-attachment clear quads (`src/v3d/clear_mrt.rs`).

## Files, per program

- `<name>.json`: what `v3d_shc` prints, unchanged. It holds the QPU words of
  the fragment, vertex and coordinate shaders (64-bit words, low word first in
  memory), each stage's uniform stream (`contents` is Mesa's
  `enum quniform_contents`, `data` its argument) and the prog_data the record
  is made from. `tools/v3d/README.md` describes the format; it is the format
  of the 4.2 files in `tools/v3d/tri-shaders/`.
- `<name>.disasm`: `v3d_shc`'s stderr, which is Mesa's per-stage statistics and
  its 7.1 disassembly of the same words. The SPIR-V paths are replaced by
  `<vs.spv>` and `<fs.spv>`. It cannot carry a comment header (the check
  compares it line for line), so its licence is in `<name>.disasm.license`
  (a REUSE sidecar the licence audit reads).
- `<name>.h`: the same as C for a Zephyr image or a host check. It has the
  code arrays, the uniform streams with their contents numbers (the names are
  Mesa's, read from its `v3d_compiler.h`), the GL uniforms' slots, the FS
  inputs and interpolation flags, and the shader state record:
  - the record is Mesa's `GL Shader State Record Draw Index`, the BCM2712 D0
    layout that Mesa emits when `v3d_device_has_draw_index()` holds (7.1 with
    rev >= 10; the Pi 5 is 7.1.10.2). `record71.c` fills it the way
    `emit_shader_state_record()` does (`src/gallium/drivers/v3d/v3dx_draw.c:477-546`),
    with As, Ve and Vc from Mesa's own `v3d_compute_vpm_config()`, and packs it
    with Mesa's own generated 7.1 packer. Every field is a `..._REC_<FIELD>`
    macro. `..._rec[8]` is the packed 32 bytes with the three code and three
    uniform addresses at 0, and `..._REC_FLAGS` is the 24-bit flag word (the
    first dword without byte 3, which is the varyings count). That is the same
    word as `tools/v3d/tri-shaders71`'s `MESA71_TRI_REC_FLAGS`.
  - the draw state that the record takes from outside the program is fixed to
    what these cases do: a triangle draw (no point size in the shaded vertex
    data) and no geometry shader. Mesa leaves `enable_sample_rate_shading`
    clear in the draw-index record whatever the program asks for.
  - the vertex attribute records give, per attribute, the number of values the
    coordinate shader and the vertex shader read (`vattr_sizes`), with Mesa's
    GFXH-930 and GFXH-1602 dummy reads applied as in `v3dx_draw.c:802-847`,
    one element per attribute record. Stride, type and address belong to the
    draw (`gl_test.c` `cuv_attribs`: `a_pos` vec4, `a_color` vec4, `a_uv`
    vec2, stride 40).

The uniform streams are Mesa's, unchanged. A `QUNIFORM_UNIFORM` word's data is
`slot * 4 + component` in the GL uniform storage (`clear`'s `u_color` is
slot 0, so its four words are 0..3). The viewport words (`QUNIFORM_VIEWPORT_*`)
come from the draw's viewport, and `QUNIFORM_CONSTANT` words are used as they
are.

## The Mesa build

Mesa 26.2.1, from `https://archive.mesa3d.org/mesa-26.2.1.tar.xz` (sha256
`c47e81bddc4760360a41ac3c5acec38acb81f9d750ecef47e7f3adc7021a4442`, the
tarball `third_party/mesa-26.2.1/UPSTREAM` names). It is set up as in
`tools/v3d/README.md`, "Set up once": `tools/v3d/v3d_shc.c` copied in,
`tools/v3d/mesa-v3d_shc.patch` applied, `meson setup build-v3d ...`, and
`ninja -C build-v3d src/broadcom/v3d_shc`. The vendored
`third_party/mesa-26.2.1` cannot be used: it has no `src/compiler`.

`v3d_shc` runs with `--device 71 --rev 10 --compat-rev 2 --vpm-size 65536`.
That is the Pi 5's identity as the board reported it (7.1.10.2 and a 64 KiB
VPM: `tools/v3d/pi5/v3d_ident.txt`, `driver/docs/rpi5-v3d71-facts.md`). The
revision matters because it selects the unpack ops in the compiler and the
draw-index record. The SPIR-V is made the way `tools/v3d/glsl2v3d.py` makes it,
through its `translate()` and then `glslangValidator -G`. The committed files
were made with glslang 16.2.0, gcc 15.2.0 and meson 1.10.1 on x86_64 Linux. A
different glslang can change the SPIR-V, and with it the code.

## The control

The same run compiles `tools/shaders/gl_test_es100/passthrough.vert` +
`interp_color.frag` (into the build directory only). It then requires the
result to equal `tools/v3d/tri-shaders71/mesa_passthrough_interp_color.json`
and `.h`, which were read off the CLIF dump of the Pi 5's own Mesa (26.2.2,
`tools/v3d/pi5/triangle.clif`). The comparison covers every QPU word, the
uniform streams, all prog_data, the attributes, and 31 record values (the
flag word, VPM sizes, As/Ve, threading, final-section and NaN bits, VCM cache
size). So this route (SPIR-V through `v3d_shc` at 7.1.10.2, then `record71`)
gives the same words and record as Mesa's GL driver on the board for that
program.

## Regenerate and check

```sh
# with a Mesa tree set up as above
V3D_SHC=<mesa-26.2.1>/build-v3d/src/broadcom/v3d_shc tools/v3d/check71.d/gltest_mesa71.sh --write
V3D_SHC=<mesa-26.2.1>/build-v3d/src/broadcom/v3d_shc tools/v3d/check71.d/gltest_mesa71.sh
# or: MESA_V3D_TREE=<mesa-26.2.1> tools/v3d/check71.d/gltest_mesa71.sh
```

The check has two halves.

The first needs no compiler and always runs, against the vendored Mesa
reference build (`tools/v3d/mesa_ref.sh`) or `MESA_V3D_TREE`'s `build-v3d`, as
the other 7.1 checks do:

- each header compiles as C11 with `-Wall -Wextra -Werror`;
- `check71.c` packs each header's `..._REC_<FIELD>` values with Mesa's own 7.1
  packer and must get the header's `..._rec[8]` and `..._REC_FLAGS`. It
  disassembles the header's code words with Mesa's own 7.1 QPU disassembler
  (7.1.10.2), in `v3d_shc`'s format, and the lines must equal `<name>.disasm`
  without its `v3d:` statistics lines;
- `gen.py --recheck` writes each header again from its committed JSON, with the
  `record71` values and the `QUNIFORM_*` names read back from the header, and
  the result must equal the committed header byte for byte. The record values
  that `record71.c` copies from prog_data (VPM segment sizes, threading, final
  thread section, Z writes, varyings count, ...) must equal the JSON's;
- every committed `.json`, `.disasm` and `.h` belongs to a program of `gen.py`.

The second needs Mesa's compiler (`V3D_SHC`, or `MESA_V3D_TREE` with
`build-v3d/src/broadcom/v3d_shc` built). It builds `record71` with
`v3d_shc.c`'s own compile command (from the build's `compile_commands.json`)
and links it like `v3d_shc`. It then runs `gen.py` into
`build/v3d71/gltest_mesa71/out` and compares every file in both directions, so
a committed file that no program makes is also a failure, and it checks the
control. The last line is then
`gltest_mesa71: N programs compiled by Mesa at 7.1.10.2 and checked against its 7.1 packer and disassembler, and the control, F failed`.

The vendored reference has no compiler, and the gate host has no `v3d_shc`.
Without one the first half still runs and its last line says
`regeneration not run`. That is not a gap, because the committed files are
still checked against Mesa's packer and disassembler (as `tfu71.sh` does without
its Mesa tree). `MESA_V3D_TREE` without `build-v3d`, and a `V3D_SHC` that is not
an executable inside a Mesa build directory, are gaps naming them. `--write`
without a compiler exits 2.
