<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): a texture coordinate that is not a whole vec2 varying (`TexCoord`)

**Status: design** (plan #336, issue #33; row 17 of `mali-corpus-families.md`).
This note changes no code. It says where `tools/shaders/es100/xlink.frag` is
refused on the tip, what Mesa 26.2.1 emits for it and for minimal variants of
its coordinate (measured, quoted), what already matches, what is missing, in
which files, and which test pins it. Anything not measured is marked
**inferred**. State read: `f5bc25f4`.

Read first: `mali-texture-refusals.md` (the `texcoord` item; its
`TexCoord::Computed` form is landed), `mali-texture-lookups.md` (the
coordinate rule, section 0), `mali-vs-texture-lanes.md` (the vertex half; it
does not touch this refusal).

## How the words were made

Mesa 26.2.1 (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, the tree
`third_party/mesa-26.2.1/UPSTREAM` lists) built out of tree with the meson
options of `tools/mali/drm_shim_ref.sh build`, word for word, LLVM 21, into
`$CONNECTED_DEPS_DIR/mesa-shim` (`inst/lib/x86_64-linux-gnu/libpanfrost_noop_drm_shim.so`);
the draws report `Mali-G610 MC4 (Panfrost)` and `glGetError 0x0`. Each draw is
the default 64x64 triangle draw:

    tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim <vs> <fs> out/<name> [--uniform-values]
    python3 -I tools/mali/pan_ref_diff.py extract out/<name>.trace --draw > out/<name>.expect.json

`xlink` is drawn without `--uniform-values` (gl_ref.py sets float uniforms
only and refuses a struct uniform with it); the variants with it. The variants
use two pass-through vertex shaders, `tc.vert` (`attribute vec4 a_pos; attribute
vec4 a_v; varying vec4 v; ... v = a_v;`) and `tc2.vert` (the same with
`attribute vec2 a_v; varying vec2 v;`), and one fragment shader each, written
`precision <p> float; uniform sampler2D s; uniform vec2 u; varying <T> v; void
main() { gl_FragColor = <body>; }`:

| name | `<T>`, precision | body |
|---|---|---|
| `t_ref_hp` | vec2, highp | `texture2D(s, v)` (the lowered reference) |
| `t_xy4_hp`, `t_xy4_mp` | vec4, highp / mediump | `texture2D(s, v.xy)` |
| `t_zw4_hp`, `t_zw4_mp` | vec4, highp / mediump | `texture2D(s, v.zw)` |
| `t_swz_hp`, `t_swz_mp` | vec2, highp / mediump | `texture2D(s, v.yx)` |
| `t_comp_hp`, `t_comp_mp` | vec2, highp / mediump | `texture2D(s, v * u)` |

"Ours" below was measured with a scratch test, not committed: the shader
compiled by `compile(SHC_STAGE_FRAGMENT, ..)` and passed to
`tests.rs::check_fs_stage` against the draw's `expect.json` (words, work
registers, preload, FAU). The scratch test and the one-line experiment in
step 1 were reverted; the tree is unchanged by this note.

| draw | `fs.fragment` words | sha256 of the words (16 hex digits and a newline each) |
|---|---|---|
| `xlink` | 32 | `d782bd90f6018b740cc541ba30eabe599b9140452459368ea6c0c400c6903f78` |
| `t_ref_hp`, `t_xy4_hp`, `t_zw4_hp`, `t_xy4_mp`, `t_zw4_mp` | 8 (one program) | `ed638f5c52e6de3c256a3f3ccbf635992aaf3af71247224bb8e7b10ea28284ae` |
| `t_swz_hp`, `t_swz_mp` | 10 (one program) | `3e8c7ddae0fe47f3e26dfd1f04accfd1621a876f6638042f04e8c87315abda1f` |
| `t_comp_hp` | 10 | `f45d6b0da154c4bd98f630ba22a43d2b84219020ad3e2aa56c2a6df14bb82cd9` |
| `t_comp_mp` | 14 | `9d74561986498a9300e2d76b5063da71c100eeaeb07fdbf6792c72ae3652d66c` |

All have `work_reg_count` 32 and preload `0x3000000000000000`.

## The source and the refusal on the tip

`tools/shaders/es100/xlink.frag` (highp): `varying vec4 v_uv2; varying float
v_mix;` and a struct uniform `u_fade` (`mat4`, `vec4`, `vec3`, `float`);

    vec4 a = texture2D(u_tex, v_uv2.xy);
    vec4 b = texture2D(u_tex, v_uv2.zw);
    gl_FragColor = mix(a, b, v_mix) * u_fade.opacity;

Verdict (`tools/mali/corpus-verdicts.txt`): `a texture coordinate other than a
vec2 varying read whole or two constants` (`MaliError::TexCoord`). Where, on
the tip:

- **`nir_fs.rs:4531-4532`**, `walk_nodes`, the plain `ir::Op::Tex` arm. The
  first look-up's lanes are lane 0 and 1 of one input, the arm takes it for
  `TexCoord::Varying(v)`, and refuses unless the declared varying has exactly
  two components (`.. .map(|d| d.components) != Some(2)`). `v_uv2` has four.
  This is the first refusal; the walk stops here, so the second look-up (lanes
  2 and 3, which the arm would send to `TexCoord::Computed` at `:4540`) is
  never reached.
- The other `TexCoord` sites do not meet this shader. `:6213` (mediump
  `select`: a computed coordinate whose two lanes differ) and `:6018`
  (`coord_lane`: a mediump computed lane that is neither a uniform lane nor a
  constant) are the **mediump** computed forms; `:4325`, `:4380`, `:4416`,
  `:4453`, `:4458`, `:4497` are the cube, fetch, array and 3D arms; `:4541`
  an undefined lane; `:5456` is `layout`'s check that a `Varying` coordinate is
  lanes 0 and 1 of one load. The two `TexCoordShared` sites (`:6290`, `:6303`)
  are not reached by a shader whose colour does not read the coordinate lanes.
- The pinned unit tests are all mediump: `tests.rs:1157` (`v.yx`), `:1159`
  (`v * 2.0`), `:1160` (`vec4 v`, `v.xy`) expect `TexCoord`.

## What Mesa emits (measured)

### `xlink.frag`: both halves of one vec4 varying

`xlink.expect.json`, `fs.fragment`, 32 words. The lines that matter, the rest
being `mix` and the blend:

```
LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store @r0:r1:r2:r3, r61^, index:0x0
MOV.i32 r4, u2.w1
MOV.i32.wait0 r5, 0x0                                  ; the handle pair
TEX_SINGLE.slot1.skip.reserved.32.2d.computed @r4:r5:r6:r7, @r0:r1, [r4^:r5^]
MOV.i32 r0, r2^                                        ; .zw copied to a fresh pair
MOV.i32 r1, r3^
MOV.i32 r2, u2.w1
MOV.i32 r3, 0x0
TEX_SINGLE.slot2.skip.reserved.32.2d.computed.wait12 @r0:r1:r2:r3, @r0:r1, [r2^:r3^]
FADD.f32.discard r0, r0^, r4.neg                       ; mix: b - a
FADD.f32 r1, r1^, r5.neg
FADD.f32 r2, r2^, r6.neg
FADD.f32 r3, r3^, r7.neg
LD_VAR_BUF_IMM.f32.slot0.src_flat32.center.store.wait0 @r8, 0x0, index:0x10   ; v_mix
FMA.f32 r0, r8, r0^, r4^                               ; ... then * u_fade.opacity (u2.w0)
```

- **The varying is one `.v4` load** into `r0:r1:r2:r3`, because both halves are
  read. The first look-up's coordinate is the load's own `r0:r1`, with no copy.
- **The `.zw` pair is copied** (`MOV r0, r2; MOV r1, r3`) although `r2:r3` are
  consecutive: the staging is a `COLLECT` of two values, and a collect whose
  sources are not the leading registers of their vector is moved
  (`bi_lower_vector`, as for a computed coordinate,
  `mali-texture-refusals.md`, section (d)). The second look-up's destination
  `@r0:r1:r2:r3` overlaps its staging `@r0:r1`.
- Each look-up has its own handle pair (`MOV r4, u2.w1; MOV r5, 0` and
  `MOV r2, u2.w1; MOV r3, 0`): `u2.w1` is the texture/sampler handle word
  (FAU `20001800`), `u2.w0` the `opacity` uniform.
- `.32`: the texel is 32-bit, the shader being highp throughout.
- `v_mix` is `src_flat32` (`LD_VAR_BUF_IMM ... src_flat32 ... @r8, 0x0`):
  `xlink.vert` writes `u_fade.scale.z`, a uniform alone, so the link makes it
  a flat varying (`mali-varying-constants.md`).

### The coordinate variants (highp)

`t_xy4_hp`, `t_zw4_hp` and `t_ref_hp` are **the same eight words**:

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store @r0:r1, r61^, index:0x0
MOV.i32 r2, u2.w0
MOV.i32.wait0 r3, 0x0
TEX_SINGLE.slot1.skip.reserved.16.2d.computed.wait0126 @r0:r1, @r0:r1, [r2^:r3^]
ATEST.wait @r60, r60^, r1.h1, atest_datum.w0
BLEND.slot2.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
IADD_IMM.i32 r48, 0x0, #0x0
BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

- **A vec4 varying of which one half is read is loaded as a `.v2`**
  (`pan_nir_resize_varying_io`, trimming the unread components). The vertex
  program stores only the read pair into the varying buffer (`t_zw4_hp`'s
  `vs.varying` stores `r2` and `r3`, the attribute's `.zw`, at buffer offsets
  0 and 4). So `.zw` alone and `.xy` alone are byte-identical in the fragment
  program and equal to a whole vec2 varying's.
- `t_swz_hp` (`v.yx`, 10 words): `LD_VAR_BUF_IMM ... .v2 @r0:r1`, then
  `MOV.i32 r2, r1^; MOV.i32 r3, r0^; MOV.i32 r0, u2.w0; MOV.i32 r1, 0x0;
  TEX_SINGLE ... .16.2d.computed.wait0126 @r0:r1, @r2:r3, [r0^:r1^]`: the
  swapped pair is two moves into the staging, the handle after it.
- `t_comp_hp` (`v * u`, 10 words): `LD_VAR_BUF_IMM ... .v2 @r0:r1; FMA.f32 r0,
  r0^, u2.w0, 0x0.neg; FMA.f32 r1, r1^, u2.w1, 0x0.neg; MOV.i32 r2, u3.w0;
  MOV.i32 r3, 0x0; TEX_SINGLE ... @r0:r1, @r0:r1, [r2^:r3^]`: the product is
  computed in place, so the value pair is already the staging and no copy is
  made (FAU `u2.w0 = 42000000`, `u2.w1 = 42040000` are the two `u` lanes,
  `u3.w0` the handle).
- `.16` in these: their texel goes straight to a mediump `gl_FragColor`, so the
  look-up is narrowed (the `.32` of `xlink` is the highp `mix`).

### The coordinate variants (mediump)

`t_xy4_mp` and `t_zw4_mp` are the eight words above again; `t_swz_mp` is
`t_swz_hp`'s ten. `t_comp_mp` is the mediump computed pair (14 words):

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0
V2F32_TO_V2F16 r0, r0^, r1^
FMA.v2f16 r0, r0^, u2.w0, 0x0.neg                      ; v * u in fp16 (u2.w0 = 50205000: two halves)
F16_TO_F32 r1, r0.h0
F16_TO_F32 r0, r0^.h1
MOV.i32 r2, r1^
MOV.i32 r3, r0^
MOV.i32 r0, u2.w1
MOV.i32 r1, 0x0
TEX_SINGLE.slot1.skip.reserved.16.2d.computed.wait0126 @r0:r1, @r2:r3, [r0^:r1^]
```

(Case 121's `uniidx_h` is the other mediump computed coordinate and has its own
reference, `case121.expect.json`; see `mali-texture-refusals.md`.)

## What the tip does with them (measured)

| shader | verdict on the tip | against Mesa |
|---|---|---|
| `t_ref_hp`, `t_zw4_hp`, `t_swz_hp`, `t_comp_hp` | compiles | **byte-equal** (words, registers, preload, FAU) |
| `t_xy4_hp` | `TexCoord` at `nir_fs.rs:4532` | (equal once the guard is relaxed, below) |
| `xlink.frag` | `TexCoord` at `nir_fs.rs:4532` | (30 of 32 words equal once relaxed, below) |
| `t_xy4_mp` | `TexCoord` at `:4532` | not tried |
| `t_zw4_mp`, `t_swz_mp` | `TexCoord` at `:6213` | not tried |
| `t_comp_mp` | `TexCoord` (`coord_lane`, `:6018`) | not tried |

So for a highp shader the computed-coordinate machinery (`TexCoord::Computed`
for lanes 2 and 3, the swapped pair, the product) is **already right**. The
only highp refusal is the plain `Varying` form's two-component guard.

## The lowering

1. **`nir_fs.rs:4531`, accept a wider varying.** The `Varying(v)` form requires
   the declared varying to have exactly two components. Replace the test with
   `>= 2`. Measured with exactly that one-token edit (`!= Some(2)` to
   `< Some(2)`, reverted): `t_xy4_hp` is **byte-equal**, and `xlink.frag`'s
   fragment program differs from `xlink.expect.json` in exactly two words, both
   of which belong to the link step, not to the coordinate:
   - word 0, `LD_VAR_BUF_IMM ... r61` against Mesa's `r61^` (`0x...3d` against
     `0x...7d`): ours does not mark the first load's `r61` as discarded because
     our `v_mix` load, further down, still reads it; in Mesa `v_mix` is
     `src_flat32` and reads no `r61`;
   - word 13, our `v_mix` load is the smooth `LD_VAR_BUF_IMM ... src_f32 @r8,
     r61^` where Mesa's is `src_flat32 @r8, 0x0`.
   That is the fragment half compiled alone: `compile_fs_for_vs` takes the
   vertex shader's constant lanes (`vs_constants`) to know `v_mix` is flat, and
   `vs_constants(xlink.vert)` stops at `ConstantFold` (row 10, `mali-vs-constant-products.md`,
   `mali-constant-products.md`). **Inferred:** both words close when row 10 lands,
   because the same link step already produces `src_flat32` for other
   uniform-only varyings (`varyconst_*` references). Not measured, since the
   vertex half cannot compile yet.
2. **`layout`, `nir_fs.rs:5450-5460`.** Nothing to change for `xlink`. The
   `Varying` form's check that the coordinate is lanes 0,1 of one load
   (`:5456`, the loop over `look_ups`) stays; the `.zw` look-up is
   `Computed` with raw input lanes and the load keeps all four lanes because
   both halves are read (measured: word 0 is the `.v4` load).
3. **`t_zw4_hp` and `t_xy4_hp` as the single-look-up forms.** Already equal;
   when only one half is read the load is `.v2` (the `layout` rule "a load
   holds the lanes read, and those alone"). A vec4 varying with the coordinate
   in `.xy` and its `.zw` read as colour (`TexCoordShared`) has no reference;
   leave the refusal.
4. **Mediump.** `t_xy4_mp`/`t_zw4_mp`/`t_swz_mp` Mesa words are those of the
   highp ones (a raw fp32 load feeds the staging, no conversion): they need
   the same relaxation at `:4532` and, for `.zw` and `.yx`, the `Computed`
   path in the mediump `select` to accept two **different raw varying lanes**
   (`:6213` refuses `nd.src[0] != nd.src[1]` because it only knows case 121's
   one-lane form). **Inferred** (not measured on our side): the mediump
   `select` has the load, handle and staging machinery, so the change is a
   second `Computed` shape beside `coord_lane`. `t_comp_mp` needs fp16
   products with `V2F32_TO_V2F16` and two `F16_TO_F32`, all already ISA rows
   (`F16_TO_F32` is `Alu::F2f32`, `emit.rs`), but the sequence order is
   unmeasured on our side. No corpus shader needs the mediump shapes; do them
   only when a dEQP/glmark2 case does.
5. **A second look-up (`xlink`).** The highp path does not refuse a second
   `Computed` look-up (`TexCount` at `nir_fs.rs:6203` is the **mediump**
   `select`, and `:7708` the cube one); the experiment's 30 equal words include
   both `TEX_SINGLE`s and the `MOV r0, r2; MOV r1, r3` copy. Nothing to add.

## Files, test, what it moves

- Code: `src/shaderc/online/mali/nir_fs.rs`, the one condition at `:4531` (step
  1). Nothing in `bir.rs`, `emit.rs`, `pack.rs`, `ra.rs`, `src/mali`.
- Testdata to add **by the item that lands the change** (not this note):
  `tools/mali/testdata/xlink.{vert,frag,expect.json}` (the shader pair above,
  drawn without `--uniform-values`) and `tex_xy4.{frag,expect.json}`,
  `tex_zw4`, `tex_swz`, `tex_comp` (the variants), with the two commands in
  "How the words were made". The drawing is `tools/mali/drm_shim_ref.sh run`
  with a vertex shader that matches the layout; pair the `xlink` one with
  `xlink.vert`.
- Host test, in `src/shaderc/online/mali/tests.rs` beside
  `texmore_*`/`case121`: byte-equal checks (`check_fs_stage`) of the four
  variants against their `expect.json` (the first, `t_xy4_hp`, is the one the
  change turns from `TexCoord` to equal; the other three are equal today and
  pin the Computed forms), and `check_linked` of `xlink` once row 10 lands. Keep
  the three mediump `TexCoord` pins at `tests.rs:1157-1160` until step 4.
- `tools/mali/corpus-verdicts.sh --write`: `xlink.frag` moves from `TexCoord`
  to `ok`. `xlink.vert` stays `ConstantFold` (row 10) and `UniformType`-like
  struct-uniform reads need nothing more on the fragment side (`u_fade.opacity`
  is a plain `u2.w0` in the measured words).
- Cases moved: corpus `xlink.frag` only. No gl_test case and no dEQP-GLES2 /
  glmark2 shader is known to read half of a vec4 varying as a coordinate
  (**inferred** from the corpus tally in `mali-corpus-families.md`, which counts
  the family at one source). The program `xlink` itself runs once row 10 also
  lands, which is a vertex-side item.

## Not covered / open

- The vertex shader `xlink.vert` (row 10, constant product) and its struct
  uniform `u_fade` reads. The fragment half reads the struct uniform fine
  (`opacity` is `u2.w0`).
- The `.v2` trimming of a partly read vec4 varying also governs which lanes the
  vertex shader stores; it is the "partial varying read" of plan item
  `link-partial` (case 19), not new here.
- Mediump sequences of step 4.
