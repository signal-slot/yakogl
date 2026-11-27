<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: vec3 varyings the link packs into one slot (plan #367, VARYING_LOAD)

Four glmark2 fragment shaders have the same form behind their first
refusals. Each is a mediump shader whose fp16 arithmetic reads two or more
`vec3` varyings:

- bump fs `77c5`, line 46: `NormalEye - height_factor * dh.x * TangentEye -
  height_factor * dh.y * BitangentEye`.
- terrain fs `4d26`, line 74: `mat3 tbn = mat3(vTangent, vBinormal,
  vNormal)`, then `tbn * normalTex`.
- ideas fs `c565` (line 42 and 44) and `1aca` (line 69 and 77):
  `normalize(vertex_normal)` and then `dot(reflection, eye_direction)`.

The compiler refused them with "a fragment varying load or conversion of a
form no reference draws" (`MaliError::FsVaryingLoad`). The refusal came from
`conversion_order` in `src/shaderc/online/mali/nir_fs.rs`: a load whose fp16
trees read lanes of two variables. `compact_varyings` lays the second
`vec3` out in the first one's spare lane: `v_e.x` goes to slot 0's w and
`v_e.yz` to slot 1. A `vec3` beside a `vec4` does not straddle a slot, so it
was not refused. The lowering is proto-varying-load's, applied on main in
plan #367 lower-train-2.

## What Mesa does (gm2_vload_1..6)

Mesa 26.2.1 on the drm-shim (`tools/mali/deqp_ref.sh --instrument shim`;
the cases are original reductions, not glmark2 text). gm2_vload_1 is
`vec4(v_n + v_e, 1.0)`:

```
LD_VAR_BUF_IMM.f32.slot0.v4 ... @r0:r1:r2:r3     v_n.xyz, v_e.x
V2F32_TO_V2F16.discard r0, r0^, r1^              (v_n.x, v_n.y)
V2F32_TO_V2F16 r1, r2^, r3^                      (v_n.z, v_e.x): two variables, one pair
LD_VAR_BUF_IMM.f32.slot1.v2 ... @r2:r3           v_e.yz
V2F32_TO_V2F16 r2, r2^, r3^                      (v_e.y, v_e.z)
MKVEC.v2i16 r3, r1.h1, r2.h0                     (v_e.x, v_e.y) rebuilt
FADD.v2f16 r0, r0^, r3^
FADD.v2f16 r1, r1^.h00, r2^.h11                  v_n.z + v_e.z
```

Each varying is its own `f2fmp` in GLSL's `lower_precision`. After
`nir_lower_alu_width`, `nir_opt_vectorize` pairs the scalar conversions of
the load they share in lane order, whichever variable each lane belongs to.
That is the pairing `conversion_order` already computed, so the lowering
removes the guard for this form.

One difference is where a conversion is placed. gm2_vload_5 is `v_n + v_t +
v_b` over three slots:

- Slot 1 holds `v_t.yz` and `v_b.xy`.
- Its first pair, all of `v_t`, is converted at the load.
- Its second pair, all of `v_b`, is converted where `v_b` is first read.
  That is after `v_n + v_t` and just before slot 2's load (`V2F32_TO_V2F16
  r2, r4^, r5^`, then `LD_VAR_BUF_IMM.f32.slot2`).

So a conversion whose lanes all belong to a variable read later than its
load's place stands at that variable's first read. A pair of mixed
variables, such as `(v_n.z, v_t.x)`, stands at the earlier one, the load.

gm2_vload_3 is ideas vs `51b2`'s layout: a `vec3`, a `vec4` and a `vec3`.
Mesa lays the whole `vec4` out at slot 0 and the two `vec3`s after it.
gm2_vload_4 is the dot product of two `vec3`s. Both follow the same rule.

gm2_vload_6 has four `vec3`s, terrain fs `4d26`'s set. The fourth, `v_v`,
starts at slot 2's odd lane 1, so its `(x, y)` lies across two pair
conversions: the high half of `(v_n.z, v_v.x)` and the low half of `(v_v.y,
v_v.z)`. Mesa joins them with `MKVEC.v2i16 r3, r3^.h1, r2.h0`.

## The lowering

In `conversion_order`, a load whose trees read lanes of two or more
variables is taken when every such variable is a mediump float `vec3` or
`vec4` of its own (not a matrix column) in a mediump shader
(`Fs::var_comps`, filled where the layout reads the declarations).
Otherwise it stays `FsVaryingLoad`, because those shapes have references
that differ:

- deqp_varyload_rnd57 and basic33 are scalar float varyings. Mesa's link
  folds and merges them further.
- op_common_mod_fs_highp has a highp `vec4` beside.

A conversion whose owner is read after its load's place goes to
`Conversions::later` and becomes a `CONV` event at the owner's first read.
It comes before anything else with that key, so before the next slot's
load. Every other event's rank in `select` moves up by one, so their order
among themselves is unchanged.

In `vectorise`, an odd lane and the next lane of one load, both of one
mediump `vec3` or `vec4`, are a `Leaf::Lanes` of the two pair conversions
(gm2_vload_6). Before this change that operand was ColourForm unless the link
kept the load whole. `packed_whole` holds it to the measured layout: the
vector keeps every declared lane in consecutive scalar slots, it starts
mid-slot, and each varying beside it in that slot is whole too. In
gm2_faupage_2 the link drops a lane of a `vec4`, which also gives an odd
start. That shape stays ColourForm, its pinned cause.

`tests/mali_varying_load.rs` checks the result:

- gm2_vload_1, 3, 4, 5 and 6 are byte-equal.
- gm2_vload_2 (`v_n - 2.0 * v_t - 3.0 * v_b`, 77c5's line 46 with
  constants) stays refused. It now stops at FS_OPERAND ("a lane alone first
  in a sum"): the product reads the rebuilt pair, and that operand form has
  no reference yet. Mesa draws it as `FMA.v2f16 r0, 0x40000000.neg.h11,
  r3^, r0^`. That is the next family for 77c5 and 4d26.

The six `expect.json` files were drawn again on this train's own drm-shim
build (`tools/mali/drm_shim_ref.sh build`, Mesa 26.2.1) with
`tools/mali/deqp_ref.sh <copies of the pair> tools/mali/testdata/gm2_vload_N
--instrument shim`.

## On main (lower-train-2)

The prototype was measured at `3e788de9`; applied on main `9a74631a` plus
the train's steps before it, the sweep of the 1,810 `tools/mali/testdata`
pairs (`tests/mali_corpus_diff.rs`) moved four older references too:

- gm2_varylink_pack_v3v4v3 compiled with one word wrong: ATEST read the
  alpha `vertex_position.w` as `r3.h0`, the low half (z) of the `(z, w)` pair
  conversion. `select`'s alpha arm took a `Leaf::Pack`'s second lane as a
  one-lane value. It now keeps the high half of a pair conversion
  (`Leaf::Half { h: 1, H11 }`): `ATEST .. r3.h1`, byte-equal.
- sel_v_duplicate (two varyings of equal value, one slot) is byte-equal.
- gm2_cf_loop1_if_fs (two packed vec3s beside a `highp vec3` local) went on
  to a back-end defect (`IrIndex`). A shader that mixes precisions
  (`Fs::mixed`) keeps the varying-load refusal.
- gm2_varylink_pack_v2v3 and _v3v2 move from the colour-lane refusal to
  FS_OPERAND and to this family's refusal for a `vec2` beside a `vec3`.

## What it does not cover

- A product or difference of a straddling `vec3` (FS_OPERAND, above).
- A colour lane of a third varying beside the packed pair, such as
  `vec4(v_a + v_b, v_c.w)`. It is still refused in `colour_keeps_whole`
  (a colour lane beside a value of no measured kind).
- A `vec2` read beside the packed `vec3`s. Terrain fs `4d26`'s `vUv`, read
  as a texture coordinate beside the four `vec3`s, is TexCoordShared. Read
  in the colour's arithmetic it is still FsVaryingLoad.
- Scalars and arrays of floats in one load. deqp_mixprec3_vary_array_static
  (`varying mediump float var[4]`) compiled byte-equal with the guard
  removed, so it is a candidate for its own small step.
