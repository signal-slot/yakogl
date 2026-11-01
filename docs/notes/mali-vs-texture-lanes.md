<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: a vertex-stage look-up at a coordinate with a constant lane

Case 149 `vsunits` (sixteen vertex samplers, #1385; issue #33) refuses with
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

## 4. The plan, in two steps

Status of case 149 at the tip (0749ee2d): `tools/mali/corpus-verdicts.txt`
has no line for it (it is a case of the board sweep, not of the corpus). Its
status is the 2026-10-27 sweep record,
`tools/cts/log/2026-10-27-frameboard/sweep-{1,2}.verdicts.txt` line 153:
`verdict case 149 vsunits: REFUSED compile: ... Mali(Valhall: a vertex shader
look-up at a coordinate with a constant lane (a shape no reference draws) is
not lowered yet)`. Step A alone does not make it pass (section 5). At
7ad18d27 the compiler already refuses it with `TexVertex` instead (the second
look-up), the coordinate `vec2(0.5)` being two constant lanes, which were
lowered before step A.

### Step A (landed): the 2D constant-lane refusal

What landed (`mali compiler: a vertex look-up at a coordinate with one
constant lane, case 149 step A (#33)`):

* `src/shaderc/online/mali/nir.rs`, the vertex `IrOp::Tex` arm: the `if` that
  refused a 2D coordinate with exactly one `Val::Const` lane
  (`MaliError::TexVertexConstant`) is gone. A coordinate of two constants was
  already lowered (the `unitype_100` reference), so the 2D look-up now takes
  any mix of constant and computed lanes. The cube refusal (a direction of
  three constants, `es100/cubelod.vert`) stays: no reference draws it.
* Nothing else changed: `select.rs` (`Kind::VsTex`) already moved each
  constant staging word into its register with `MOV.i32` in the order s, t,
  handle, `0x0`, and the FAU order (0.5, handle) followed from it.
* `tests.rs`: `a_vertex_look_up_at_a_constant_lane_is_byte_equal_to_mesa_26_2_1`
  checks both IDVS programs and the fragment program of `case149_vstexconst`
  and `case149_vstexconst_s` against their `expect.json` byte for byte, and
  the FAU tail `Constant(0x3f00_0000)`, `Constant(TEX_HANDLE)` (words 11 and
  12; the reference's word 13, 0, is the table's padding to an even count,
  the sampler word is the immediate `0x0`, not a FAU word). The two `.vert`
  names are in `vs_reference_sources`.
* `tools/mali/corpus-verdicts.txt`: unchanged (no corpus shader has a 2D
  vertex look-up with one constant lane; `cubelod.vert` is the cube shape).
  The populations are unchanged.

Case 149 itself does not move with step A: at the tip it already refused with
`TexVertex` ("a vertex shader texture look-up other than one 2D look-up at a
level (a second one, a size query)"), because its coordinate is
`vec2(0.5)`, two constant lanes, lowered before step A; the refusal is the
second look-up (step B item 1). The `TexVertexConstant` text ("a shape no
reference draws") is now right for the cube direction only.

### Step B (larger, an issue of its own): sixteen vertex samplers

`case149_vsunits` (`.expect.json`: 162-word `vs.varying`, 32 work registers)
needs three things beyond step A.

1. **Per-look-up sampler indices in the compiler.**
   * `nir.rs:1819` guards the look-up arm with `vs.sampler.is_none()` and
     `:1820` sets the one `vs.sampler`; the second look-up falls to `:1856`
     (`TexVertex`). `vs.sampler` is an `Option<u8>` and `:2042` clears it when
     DCE removed every `VsTex`. It must become a set of bindings (each
     look-up's own `sampler` from `IrOp::Tex`), and `Kind::VsTex` must carry
     its sampler index.
   * `select.rs:570` uses the single `TEX_HANDLE` (`nir_fs.rs:134`:
     `(4<<27) | (TEX_INDEX<<16) | (3<<11) | TEX_INDEX`, `TEX_INDEX` is 0 at
     `:129`). The handle of sampler K is that formula with K in both index
     fields: `(4<<27) | (K<<16) | (3<<11) | K = 0x20001800 + K * 0x10001`.
     The reference confirms it: `vs.varying` FAU words 28..43 are `20001800,
     20011801, 20021802, ... 200f180f`. It is **not** `0x20001800 + K * 0x1000`
     (that gives `0x20002800` for K=1; the reference has `0x20011801`). It is
     one shared function of K for texture index and sampler index alike (the
     driver binds texture K and sampler K together). The FAU then holds
     sixteen `FauWord::Constant` handles in first-use order after the 0.5
     constant (word 27).
   * `MaliShader::sampler` (`mod.rs:589-593`, set at `:1140`) is one binding;
     it must report all bindings used (a mask or a list), and its readers
     follow.
   * The weights `u_w[4]` and the final exact `ffma x, 0.0, x` are separate
     checks once the look-ups lower (section 5, item 2).
2. **N texture and sampler descriptors in the vertex tables of `src/mali`.**
   * Today `frame.rs:2807-2812` builds one `vs_texture` descriptor
     (`DrawCall::vs_texture: Option<SampledTexture>`, `frame.rs:1449`, filled
     by `backend.rs:1560` through `sampled_bind(d, true)`, `:1577`, which finds
     one binding through `MaliShader::sampler`). `emit_stage_uniforms`
     (`frame.rs:2463-2469`) allocates one `Sampler`; `:2836-2838` points
     `TABLE_SAMPLER` and `TABLE_TEXTURE` at them with count 1;
     `Layout::vs_sampler` / `vs_texture` (`:1487`, `:1493`) are single blocks;
     `frame.rs:2044` tests "samples a texture" as a boolean.
   * Needed: one texture descriptor and one sampler descriptor per used
     binding, at index K, in two arrays (at most `GLES_MAX_VS_TEX_BINDS` = 16,
     `src/backend.rs:619`; the GPU's limit is `GLES_VS_TEX_UNITS`), both
     counts in the vertex `Resource` table entries, the layout fields as
     arrays or spans, and `d.vs_textures` / `num_vs_textures` read per binding.
   * This note does not choose the layout: the descriptor order and stride and
     the table counts are known only from the board reference (item 3).
3. **A Mesa 26.2.1 board reference of the descriptor layout first**, with
   `tools/mali/mesa-board-ref.sh` (on the NanoPC-T6, only through
   `tools/mali/panthor/run-on-board.sh`), run x2, on case 149's program
   (`case149_vsunits.vert/.frag`). Pin in a README under
   `tools/cts/log/<date>-vsunits-ref/`: the vertex resource-table entries
   (`TABLE_TEXTURE` and `TABLE_SAMPLER`: count, address), the 16 texture and
   16 sampler descriptor words and their stride, and whether the two IDVS
   shaders share one handle set (the drm-shim says yes). The `src/mali` host
   tests are then pinned to those words with every differing byte named; the
   captures stay out of the tree. The drm-shim reference holds the program
   words only, not the descriptors.

   What step B must measure on the board, because the drm-shim cannot show
   it (the shim runs the compiler only and records no command stream):
   * the vertex shader's resource table: the `TABLE_TEXTURE` and
     `TABLE_SAMPLER` entries (their index in the table, entry count, address)
     for 16 bindings, and whether the varying (secondary) shader reads the
     same entries as the position shader;
   * the 16 texture descriptors and the 16 sampler descriptors: their words,
     their stride and order (is descriptor K at index K, as the handles
     `0x20001800 + K * 0x10001` imply), and the alignment of each array;
   * whether Mesa emits descriptors for all 16 units or only the used ones,
     and how a unit the shader does not read (none in case 149) is filled;
   * the FAU (push-constant) block as uploaded: the sixteen `u_w` weights,
     the 0.5 constant and the sixteen handles in the order the expect.json
     gives (words 11..43), i.e. that the driver uploads what the compiler's
     FAU table names;
   * the draw's result: the framebuffer pixels of case 149 under Mesa, to
     pin the expected colour of the sixteen weighted look-ups.

Suggested title for the step B issue: *Mali: a vertex shader with sixteen
texture look-ups (case 149 vsunits): per-look-up sampler indices and N vertex
texture/sampler descriptors*. It depends on step A (constant lanes), on the
board reference (descriptors) and on the running lowering chain (`nir.rs`,
`select.rs`).

## 5. Does case 149 hit a further refusal?

Yes, in the order the compiler meets them (re-checked at the tip):

1. **A second look-up** (step B item 1): the guard at `nir.rs:1819`, then
   `TexVertex` at `:1856`. At 7ad18d27 this is already the first refusal
   (the guard is `vs.sampler.is_none()` in the vertex `IrOp::Tex` arm of
   `nir.rs`); the next sweep's text of case 149 is the "second one" text.
2. **The weights.** `u_w[4]` read at constant indices (a 64-byte `load_ubo`
   whose lanes become FAU words, covered by `uniarray`) and the `ffma` chain of
   section 3, with the final exact `ffma x, 0.0, x` kept as
   `FMA.f32 rK, rK, 0x0, rK` and not folded away. The references decide.
3. **The frame** (step B items 2 and 3).

The fragment shader of the pair (`vsunits_fs.frag`) is the plain
varying-to-`gl_FragColor` shape that already compiles.

This list is by reading the code against the references at 0749ee2d, not by
running it: the compiler was not changed or run on these shaders.
