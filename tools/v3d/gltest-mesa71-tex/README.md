<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gltest-mesa71-tex: Mesa's offline V3D 7.1 programs for gl_test's texture, mipmap and cube cases

The programs that the first gl_test cases which sample a 2D texture, call
`glGenerateMipmap` and sample a cube map draw with, built by Mesa's v3d
compiler for the Raspberry Pi 5's V3D 7.1.10, as `tools/v3d/gltest-mesa71/`
does for the six P9 programs (read its README for the file formats, the Mesa
build and the record). This is the fallback for P10 (`mip71`, `board71-tex`)
wherever our online compiler cannot build a program yet. They are reference
binaries, not a product table.
`tools/v3d/check71.d/gltest_mesa71_tex.sh` regenerates them and compares them
byte for byte.

## The cases

The selection rule: in `driver/tools/pvrlab/gl_test.c`, in dispatch order (the
`switch (c)` at line 23841, which is the order of its `case` labels), the
first case that samples a 2D texture filled by `glTexImage2D`, the first that
calls `glGenerateMipmap`, and the first that samples a `GL_TEXTURE_CUBE_MAP`.

| kind | case | `gl_test.c` | what it does |
| --- | --- | --- | --- |
| 2D texture by `glTexImage2D` | 2 `case_texture` | 1819 (dispatch 23844); `glTexImage2D` 1866 (8x8 RGBA), 1905-1907 (a 4/2/1 chain), 1994 | `link_program(gl_test_uv_vs_src, gl_test_texture_fs_src)` at 1825, then `gl_test_texture2_fs_src` at 1982 |
| `glGenerateMipmap` | 2 `case_texture` | `glGenerateMipmap` at 1958 (4x4 base, half red, half blue, level 2 purple) | the same two programs: no earlier case calls it |
| `GL_TEXTURE_CUBE_MAP` | 41 `case_es3cube` | 6520 (dispatch 23904); `glBindTexture` 6535, six 8x8 faces at 6538, NEAREST, CLAMP_TO_EDGE | `link_program(gl_test_es300_cube_vs_src, gl_test_es300_cube_fs_src)` at 6531 |

The first two kinds are one case: case 2 comes before every other case that
touches a texture, and it does both. So there are three distinct programs and
two cases. Cube case 41 is the first in dispatch order, but it is an
ES 3.00 one (`#version 300 es`, `samplerCube`, `textureLod`): its shader pair is
in `gl_test_shaders_es100.h` too, from `tools/shaders/es300/cube.{vert,frag}`,
because that header holds the ES 3.00 shaders next to the ES 1.00 twins. The
cube case that Mesa's 7.1 reference `tools/v3d/pi5/gl_cube` follows is case 115
(`case_texcube`, `gl_test_texcube_{vs,fs}_src`, `textureCube` with `u_major`,
`u_sx`, `u_ty`); it is not first in dispatch order, so it is not built here.

## The programs

| name | vertex shader | fragment shader | linked by | fs / vs / cs instructions | record flags |
| --- | --- | --- | --- | --- | --- |
| `texture` | `tools/shaders/gl_test_es100/uv.vert` | `tools/shaders/gl_test_es100/texture.frag` (`gl_test_texture_fs_src`) | case 2 (`gl_test.c:1825`) | 9 / 19 / 15 | `0x208002`, 2 varyings |
| `texture2` | `tools/shaders/gl_test_es100/uv.vert` | `tools/shaders/gl_test_es100/texture2.frag` (`gl_test_texture2_fs_src`) | case 2 (`gl_test.c:1982`) | 23 / 19 / 15 | `0x208002`, 2 varyings |
| `cube` | `tools/shaders/es300/cube.vert` | `tools/shaders/es300/cube.frag` (`gl_test_es300_cube_fs_src`) | case 41 (`gl_test.c:6531`) | 15 / 17 / 12 | `0x208002`, 2 varyings |

All three are 4-thread programs. Mesa refused none, so no program needs a
"could not build" line. The flag word is the 24-bit word of the first record
dword as in `gltest-mesa71` (the varyings count, byte 3, is 2: `0x02208002`).

Uniform streams (words per stage, fs / vs / cs): `texture` 2 / 14 / 8,
`texture2` 4 / 14 / 8, `cube` 4 / 15 / 10. The sampler words are not
`QUNIFORM_UNIFORM`: a texture's TMU config words are `QUNIFORM_TMU_CONFIG_P0`
(40) and `QUNIFORM_TMU_CONFIG_P1` (41) entries of the stream, and none of the
three programs asks for a texture-size word; the header's `#define`s name the
contents each stream uses (these are what `texuni71` fills). A
sampler appears in the header's "GL uniforms" comment as
`sampler, <stage> stage, binding N (no uniform words)` (the `u_tex`, `u_tex0`
and `u_tex1` of cases 2's twins at bindings 0 and 1, `cube` at 0), because
`v3d_shc` gives it a binding and no slot. `cube`'s `axis` is slot 0 in each
stage's own storage.

## Files, how they are made

Per program: `<name>.json` (v3d_shc's stdout, unchanged), `<name>.disasm`
(its stderr: Mesa's statistics and 7.1 disassembly; licence in
`<name>.disasm.license`) and `<name>.h` (C: code words, uniform streams, FS
inputs, vpm config, the `GL Shader State Record Draw Index` field values and
packed 32 bytes, the attribute read counts: `texture` and `texture2` have two
attributes, `a_pos` and `a_uv`; `cube` has one, `pos`).

- `gen.py` here loads `tools/v3d/gltest-mesa71/gen.py` (it is called, not
  copied: its compile, record, attribute and header code are used as they are)
  and gives it this directory's programs. Its only changes to the output are
  the header comment's generator and script names, and the sampler line
  above (the original assumes every uniform name has a slot).
- `check71.c` is `gltest-mesa71/check71.c` with the three headers (the check
  of the packed record and of the disassembly; it must be a copy because it
  includes the headers by name).
- `record71.c` is the one of `tools/v3d/gltest-mesa71/`, built by the check
  script from there.

Mesa and its identity are those of `gltest-mesa71`: Mesa 26.2.1,
`v3d_shc --device 71 --rev 10 --compat-rev 2 --vpm-size 65536` (7.1.10.2, 64 KiB
VPM). The committed files were made with glslang 16.2.0, gcc 15.2.0 and meson
1.10.1 on x86_64 Linux; a different glslang can change the SPIR-V and so the
code.

## Regenerate and check

```sh
V3D_SHC=<mesa-26.2.1>/build-v3d/src/broadcom/v3d_shc tools/v3d/check71.d/gltest_mesa71_tex.sh --write
V3D_SHC=<mesa-26.2.1>/build-v3d/src/broadcom/v3d_shc tools/v3d/check71.d/gltest_mesa71_tex.sh
# or: MESA_V3D_TREE=<mesa-26.2.1> tools/v3d/check71.d/gltest_mesa71_tex.sh
sh tools/v3d/check71.d/gltest_mesa71_tex.sh   # no compiler: the first half only
```

It runs under `sh` and `bash`. The first half needs no compiler (each header
compiles as C11 with `-Wall -Wextra -Werror`; Mesa's 7.1 packer and QPU
disassembler on the committed files; `gen.py --recheck`; every committed file
belongs to a program of `gen.py`). The second half needs `v3d_shc` and compiles
every program again and compares every file both ways, as `gltest_mesa71.sh`
does (without its control program: the control is `gltest-mesa71`'s). Without
a compiler the last line says `regeneration not run`; that is not a gap.
