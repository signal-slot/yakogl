<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: a vertex-stage look-up at a coordinate with a constant lane

Case 149 `vsunits` (sixteen vertex samplers, #1385) refuses with
`MaliError::TexVertexConstant` ("a vertex shader look-up at a coordinate with
a constant lane (a shape no reference draws) is not lowered yet"). This note
records what Mesa 26.2.1 emits for that shape, from drm-shim references, so
that a compiler item can lower it byte-equal. No compiler or frame code was
changed to write it.

## 1. The references

`D=tools/mali/testdata`, `<work>` = the `drm_shim_ref.sh build` directory
(`$CONNECTED_DEPS_DIR/mesa-shim`), Mesa 26.2.1, GPU id a867:

```
tools/mali/drm_shim_ref.sh run <work> $D/case149_vsunits.vert     $D/case149_vsunits.frag     $D/case149_vsunits
tools/mali/drm_shim_ref.sh run <work> $D/case149_vstexconst.vert  $D/case149_vstexconst.frag  $D/case149_vstexconst
tools/mali/drm_shim_ref.sh run <work> $D/case149_vstexconst_s.vert $D/case149_vstexconst_s.frag $D/case149_vstexconst_s
python3 -I tools/mali/pan_ref_diff.py extract $D/case149_<x>.trace --draw > $D/case149_<x>.expect.json
```

The `.trace`, `.stdout` and `.stderr` are not kept (as for `unitype_*`); the
`expect.json` holds each region's words, text, FAU, `work_reg_count` and
preload.

| pair | what |
|---|---|
| `case149_vsunits.vert/.frag` | case 149's program: `vsunits_vs_src(16)` of `driver/tools/pvrlab/gl_test.c` verbatim (16 samplers, `vec2 c = vec2(0.5)`, `texture2DLod(u_sK, c, 0.0)` weighted by `u_w[K/4].xyzw`), fragment shader `tools/shaders/gltest/vsunits_fs.frag` |
| `case149_vstexconst.vert/.frag` | one sampler, `texture2DLod(u_s0, vec2(a_pos.x, 0.5), 0.0)`: the t lane constant |
| `case149_vstexconst_s.vert/.frag` | the same with `vec2(0.5, a_pos.y)`: the s lane constant |

`gl_ref.py` gives each sampler a 2x2 RGBA8 texture at unit K. Only the
`vs.varying` region differs from an ordinary vertex look-up; `vs.position` is
the usual position shader (`tex_vs_zero`'s shape) and `fs.fragment` is the
usual 7-word fragment shader.

## 2. How Mesa stages the coordinate

NIR for `vstexconst` (`txl` of a `vec2 %2.x, 0.5`): the look-up is
`(float32)txl <handle 0x20001800, 0x0> (texture_handle), <vec2> (backend1)`,
the same op as an unconstrained look-up. Nothing folds the constant lane: it is
an SSA `load_const 0.5` that `vec2` combines with the attribute lane.
In `vsunits` the whole coordinate is `%0 (0.500000, 0.500000)`, shared by all
sixteen `txl`.

In the Bifrost IR the staging is the ordinary `bi_emit_tex_valhall` one
(`COLLECT.i32` of the two lanes, the level register for `explicit`), and the
constant lane is a FAU word:

```
vstexconst, vs.varying (19 words; fau: 11 = 3f000000, 12 = 20001800, 13 = 0)
 0866800832017d7c  LD_ATTR_IMM.v4.f32.slot0.wait0 @r0:r1:r2:r3, r60^, r61^, index:0x0, table:0x1
 0091c1000000008b  MOV.i32 r1, u5.w1               ; the constant lane 0.5 = FAU word 11, moved into the register
 0091c2000000008c  MOV.i32 r2, u6.w0               ; the texture handle 0x20001800 = FAU word 12
 0091c300000000c0  MOV.i32 r3, 0x0                 ; the sampler word, immediate 0
 0128403453c00042  TEX_SINGLE.slot1.reserved.32.2d.zero @r0:r1:r2:r3, @r0:r1, [r2^:r3^]
 305e840480000d7b  LEA_BUF_IMM.slot2.wait12 @r4:r5, r59^, table:0xD, index:0x0
 ...                                                ; the same epilogue as tex_vs_zero
```

(the LD_ATTR_IMM here loads the whole `a_pos`; the s lane `r0` is the
attribute's x in place, so the collect is `@r0:r1` with no move for it).
`case149_vstexconst_s` is the same with the lanes exchanged:

```
 0091c0000000008b  MOV.i32 r0, u5.w1               ; s = 0.5 into r0 (the attribute's x, loaded by LD_ATTR_IMM, is dead and overwritten; y stays in r1)
 0091c2000000008c  MOV.i32 r2, u6.w0
 0091c300000000c0  MOV.i32 r3, 0x0
 0128403453c00042  TEX_SINGLE.slot1.reserved.32.2d.zero @r0:r1:r2:r3, @r0:r1, [r2^:r3^]
```

The TEX_SINGLE word is the same, `0128403453c00042`, for both (`slot1`, one look-up
in flight) and equal to `tex_vs_zero`'s. Decoded: `2d`, `zero` lod, `32` bit
result, staging `@r0:r1` (the `sr_count` 2 pair) and `[r2:r3]` (handle,
sampler word), written to `r0:r1:r2:r3`.

Rules the references show:

* A constant coordinate lane is **not an immediate in the instruction word**:
  the `TEX_SINGLE` takes registers only. The compiler's `va_lower_constants`
  makes the constant a `MOV.i32 rN, <FAU word or immediate>` first. 0.5
  (`0x3f000000`) is a FAU word (`u5.w1`); a constant that is an inline
  immediate in the ISA (0.0 → `0x0`, as the sampler word's `MOV.i32 r3, 0x0`)
  would be `MOV.i32 rN, 0x0`. A constant 0.0 or 1.0 lane was not drawn here.
* The order is: the constant lane move(s), then the handle move, then the
  sampler word move, then `TEX_SINGLE`. A lane that already lives in a register
  (the attribute's lane, `r0`/`r1` after the `LD_ATTR_IMM`) is used in place.
* Two constant lanes are two moves of the same FAU word, not one
  (`vsunits`: `MOV.i32 r0, u13.w1` then `MOV.i32 r1, u13.w1`).
* The FAU constant sits after the sysval/store-offset words (8..10) and the
  uniforms the position shader needs; in `vstexconst` it is word 11, the
  handle is word 12, the zero word 13. The position shader's FAU is the same
  table (`fau` of `vs.position`), so the pool's order is the one shared by the
  two IDVS shaders: sysvals, then constants/uniforms in first-use order, then
  the texture handle.

## 3. `case149_vsunits`: sixteen look-ups

`vs.varying` has 162 words, `work_reg_count` 32, preload `0x0800000000000000`
(the attribute is not read in this shader). Each look-up is the same
four-instruction staging:

```
 0091c4000000009b  MOV.i32 r4, u13.w1      ; s = 0.5
 0091c5000000009b  MOV.i32 r5, u13.w1      ; t = 0.5
 0091c6000000009d  MOV.i32 r6, u14.w1      ; handle k (u14.w0, u14.w1, u15.w0, ...)
 0091c700000000c0  MOV.i32 r7, 0x0
 1128443453c40046  TEX_SINGLE.slot1.reserved.32.2d.zero.wait1 @r4:r5:r6:r7, @r4:r5, [r6^:r7^]
```

* FAU: words 0..10 as above, **11..26 the sixteen weights of `u_w[4]`**
  (`u5.w1` .. `u13.w0`), word 27 the 0.5 constant (`u13.w1`), words 28..43
  the handles `20001800, 20011801, 20021802, ..., 200f180f`: the handle of
  sampler K is `0x20001800 + K * 0x10001` (texture index K in bits 16.., sampler
  K in the low bits; NIR shows `67108864 + K (texture)`, `50331648 + K
  (sampler)`).
* `TEX_SINGLE` rotates its slot (`slot0`, `slot1`, `slot2`) and waits on the
  slot it reuses (`.wait1`, `.wait2`, `.wait0`), the scoreboard `bi_` pass's
  rotation, not a lowering choice. The first look-up writes `r0:r1:r2:r3`, the
  second `r4:r5:r6:r7`, then they alternate.
* The weighted sum is Mesa's `ffma` chain: `r4 = w1 * t1` (`FMA.f32 r4, u6.w0,
  r4^, 0x0.neg`), `r0 = w0 * t0 + r4` (`FMA r0, u5.w1, r0^, r4^`), then each
  further term `FMA.f32 rK, wK, tK, acc` with the accumulator in `r0:r1:r2:r3`.
  The last instruction group is the exact `ffma x, 0.0, x` (NIR `ffma %101,
  %5 (0.0), %101 // exact, preserve:inf,nan`): `FMA.f32 r0, r0^, 0x0, r0^`
  per lane, from the `v_c` store being mediump.
* `u_w[4]` is a uniform array read at constant indices: one `load_ubo` of
  64 bytes whose lanes become FAU words, already covered by `uniarray`.

## 4. What the compiler needs

The refusal site is `src/shaderc/online/mali/nir.rs:1726` (a cube direction of
three constants) and `:1734-1736` (a 2D coordinate where `s` or `t` is
`Val::Const`), both returning `MaliError::TexVertexConstant`
(`mod.rs:172`, text at `:379`). `select.rs:531` (`Kind::VsTex`) already
implements the staging: it moves "a staging word not in a register" with
`MovI32` before the `Collect`, takes the handle as `Idx::Const(TEX_HANDLE)`
and the sampler word as `Idx::Const(0)`, and `src()` turns a `Val::Const` into
a constant source. So, for the one-sampler references
(`case149_vstexconst`, `case149_vstexconst_s`) the lowering is probably only
the removal of the two refusals, plus making sure that

1. the constant lane's `MovI32` is emitted before the handle's and the sampler
   word's moves, in lane order (`s` then `t`), and that the order matches the
   reference (`u5.w1` first, then `u6.w0`, then `0x0`);
2. the FAU assigns the constant its word before the handle (11 then 12, 13 for
   the zero), i.e. in first-use order of the staging;
3. two equal constant lanes are two `MovI32`, not one (`vsunits`).

For `case149_vsunits` the refusal is only the first obstacle (section 5).
`va_lower_constants`' choice between an immediate and a FAU word is by value:
0.5 is a FAU word. A constant lane of 0.0 is expected to be `0x0`; this is not
drawn in these references (add one when the lowering item needs it).

## 5. Does case 149 hit a further refusal?

Yes, three, in the order the compiler meets them:

1. **A second look-up.** `nir.rs:1714` guards the arm with
   `vs.sampler.is_none()`: the first `txl` sets `vs.sampler`, and the second
   falls to `IrOp::Tex { .. } => Err(MaliError::TexVertex)` (`:1751`, "a second
   one"). Sixteen look-ups on sixteen samplers need `vs.sampler` to become a
   per-look-up value (the sampler binding, with the handle
   `0x20001800 + K * 0x10001` of section 3 rather than the single
   `TEX_HANDLE`), `Kind::VsTex` to carry the sampler index, and the FAU to hold
   the 16 handles; `nir.rs:1934`'s "does the program have a look-up" scan and
   the frame's sampler table (`vs.sampler` is one binding today) follow.
2. **The weights.** `u_w[4]` read at constant indices and `r +=` of products
   with a `texture` result need the `ffma` chain above (`Alu::Ffma` is in the
   vertex path; the `ffma` of `mix` was lowered by plan #153's `ffma`) and the
   `u_w` FAU words; the `uniarray` item covers the array. The final exact
   `ffma x, 0.0, x` (marked exact, from `v_c = r`) must be reproduced as
   `FMA.f32 rK, rK, 0x0, rK` and not folded away.
3. **The frame.** Sixteen vertex samplers are sixteen texture descriptors and
   sixteen sampler descriptors in the draw (`GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS`
   on this driver is a frame matter); the frame side is not examined here.

Also a fragment shader of the pair (`vsunits_fs.frag`) is the plain
varying-to-`gl_FragColor` shape that already compiles.

This list is by reading the code against the reference, not by running it:
the compiler was not changed or run on these shaders.
