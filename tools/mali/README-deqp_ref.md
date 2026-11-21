<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# deqp_ref.sh: a Mesa 26.2.1 reference from a shader pair

`tools/mali/deqp_ref.sh` makes one reference of the `gm2_*` kind
(`tools/mali/testdata/gm2_*`: `.vert`, `.frag` and `.expect.json` with `trace`,
`secondary_offset` and `regions`) from a GLSL ES 1.00 pair. The instruments are
the two of `README-pan_shc.md`; read its "The shim driver" and "Known gaps".

```sh
tools/mali/deqp_ref.sh <in.vert> <in.frag> <out-prefix> [--instrument pan_shc|shim] [--prim triangles|points]
# the dEQP shapes pair, the reference beside the other testdata:
tools/mali/deqp_ref.sh tools/mali/testdata/deqp_shapes/<shape>.vert \
    tools/mali/testdata/deqp_shapes/<shape>.frag tools/mali/testdata/deqp_<family>_<shape>
```

It copies the sources to `<out-prefix>.vert` and `.frag` and writes
`<out-prefix>.expect.json`. Nothing else is left behind (traces and pan_shc
output are made in a temporary directory).

- **shim** (the `gm2_*` method): `drm_shim_ref.sh run ... --prim P
  --uniform-values`, then `pan_ref_diff.py extract <trace> --draw`. The trace
  field is `<basename of out-prefix>.trace`.
- **pan_shc**: `glsl2pan.py --prim P`, then the regions, `work_reg_count`,
  preload and FAU constants of `pan_ref_diff.py`'s `read_pan_shc` in the same
  JSON layout. `va` is `(pan_shc)`, `text` is empty, a FAU word the driver
  fills is `null` and `trace` is `<basename>.pan_shc.json`. Its position region
  is the one a draw of `--prim` runs (`draw_position_region`).
- **default**: shim when the fragment shader (comments left out) has a
  `mediump` or `lowp` or reads `gl_FragCoord`; pan_shc otherwise. Mesa's GLSL
  path lowers those to fp16 and pan_shc's SPIR-V path does not
  (`README-pan_shc.md`, "Known gaps"), so pan_shc's bytes are the reference
  only for a pair with none. When in doubt pass `--instrument shim`: it is the
  byte reference.

## Environment

`MESA_PAN_TREE` (default `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`) and
`PAN_SHC` (default `$MESA_PAN_TREE/build-pan/src/panfrost/pan_shc/pan_shc`)
for the pan_shc instrument; `MESA_SHIM_WORK` (default
`$CONNECTED_DEPS_DIR/mesa-shim`) for the shim. A missing one is built the
documented way: `build_pan_shc.sh` and `drm_shim_ref.sh build`, from the
tarball of `third_party/mesa-26.2.1/UPSTREAM` that the script fetches once into
`$CONNECTED_DEPS_DIR/mesa-tarball/` and checks against its sha256. Both builds
take tens of minutes the first time. Both need what `build_pan_shc.sh` lists
(clang and LLVM of one version, `LLVM_DIR`, meson, ninja, mako, ...);
`GLSLANG` is the glslangValidator for pan_shc.

## Self-check: two existing references, byte for byte

A highp vertex shader (its fragment shader is a constant mediump colour) and a
mediump fragment shader, both with the shim:

```sh
mkdir -p "$TMPDIR/ref"
for n in gm2_cf_ifelse_vs gm2_cf_ifelse_fs; do
  bash tools/mali/deqp_ref.sh tools/mali/testdata/$n.vert tools/mali/testdata/$n.frag "$TMPDIR/ref/$n"
  cmp tools/mali/testdata/$n.expect.json "$TMPDIR/ref/$n.expect.json"
  cmp tools/mali/testdata/$n.vert "$TMPDIR/ref/$n.vert"
done
```

`cmp` prints nothing when the files are equal.

## Struct uniforms

`gl_ref.py --uniform-values` (the shim instrument passes it) sets a uniform
of a named struct type member by member, through the locations dEQP's
`uniform_api` cases look up with `glGetUniformLocation`:

| Declaration | Members set, in this order |
|---|---|
| `struct S { mediump float m0; mediump vec4 m1; }; uniform S u_var;` | `u_var.m0`, `u_var.m1` |
| an array member, `mediump float m2[3];` | `u_var.m2[0]`, `u_var.m2[1]`, `u_var.m2[2]` |
| an array of structs, `uniform S u_var[3];` | `u_var[0].m0`, `u_var[0].m1`, `u_var[1].m0`, ... |
| a nested struct, `T m1;` with `T { ...; U ms2[2]; }` | `u_var.m1.ms0`, ..., `u_var.m1.ms2[0].mss0`, ... |

Depth first, in declaration order (`uniform_leaves()`). Each member takes the
next value index k (component c gets 16 (k + 1) + c, an int member the
integer, a bool member true where c is even), so a uniform declared after a
struct moves up by the struct's member count less one; a program without a
struct is numbered as before (`uniform_settings()`). The run prints each
member's location (`gl_ref: uniform vec4 u_var[1].m1 at 3 = 64 65 66 67`); a
location of -1 means Mesa dropped the member. Not set: a sampler member (the
run stops with "does not set a sampler2D uniform (u_var.m0)"), a struct
defined inline in the uniform declaration (`uniform struct { ... } u;`, not
seen as a uniform at all), and, as before, a plain array uniform (`uniform
vec4 u[3];`, its location lookup is -1, so it stays 0).

Mesa uploads a struct's mediump float members as fp32 FAU words (16.0 is
`41800000`), ints as their integer and bools as `ffffffff` / `00000000`
(`tools/mali/testdata/deqp_fclane2_*.expect.json`). The struct references
drawn before this (`deqp_unif2_*struct*`, `deqp_uniarray_struct*`,
`deqp_unitype_vsapi_struct`) have every uniform 0; `tests/mali_ref`'s
`differences` still assumes that for any program with a struct-member row,
so a byte-compare of a `deqp_fclane2_*` reference needs it to compute the
values above first. The tests: `python3 -I tools/mali/test_gl_ref.py`.
