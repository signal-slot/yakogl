<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall fragment stage: dFdx / dFdy (gl_test case 138 `deriv`)

**Status: design, implementation in fs-deriv** (plan #336, issue #33). Host
research only: no file under `src/` changed, no board run. Tip `17e91019`.

Everything below is read from the recorded Mesa 26.2.1 references in
`tools/mali/testdata/` (drawn by `tools/mali/drm_shim_ref.sh run`) and from
the code at the tip. `third_party/mesa-26.2.1/` vendors `panfrost/compiler/
bifrost/valhall/` and `kraid/` only: **`bifrost_compile.c` (`bi_emit_derivative`,
:1507 as `mali-alu-ops.md` cites it), `bi_helper_invocations.c` and
`nir_lower_wpos_ytransform.c` are not in the tree.** The "why" columns are
therefore the recipe the reference bytes imply, plus the line citations the
earlier notes recorded; a statement that no reference proves is marked
*(inferred)*.

References (all three drawn on the same 64x64 FBO state, `T = (1, 0)`):

| File | Source | Precision |
|---|---|---|
| `fsarith_138_deriv.*` | `tools/shaders/gltest/es100/deriv.frag` | mediump, ES 1.00 |
| `alu_deriv.*` | the same `.frag` (the `fs.fragment` words and FAU are byte-equal to `fsarith_138_deriv`'s; checked) | mediump |
| `fsarith_derivs_es300.*` | `es300/derivs.frag` (`dFdx(t.x), dFdy(t.y), dFdx(t.z) + dFdy(t.w)`) | highp, ES 3.00 |

Case 138's fragment program (`fsarith_138_deriv.frag`):

```glsl
precision mediump float;
varying vec2 v;
gl_FragColor = vec4(0.5 + 8.0 * dFdx(v.x), 0.5 + 8.0 * dFdy(v.y),
                    0.5 + 0.125 * (dFdx(gl_FragCoord.x) + dFdy(gl_FragCoord.y)), 1.0);
```

## 1. What Mesa emits for one derivative

NIR's `fddx_fine(a)` / `fddy_fine(a)` (the references use the *fine* form:
the lane's neighbour, not a fixed lane of the quad) become, per scalar
operand, in this order (`sz` = 16 or 32):

1. **the operand**, in one 32-bit register word. A 16-bit operand that is a
   half of a pair is first widened (`MKVEC.v2i16 x.h0, #0` is `U16_TO_U32`,
   `opt.rs::lower_mkvec_swz`); a lone fp16 in a fresh register is read as it is.
2. `CLPER.i32.subgroup4.xor n, a, axis`: the value of lane `L ^ axis`, `axis`
   = 1 for X, 2 for Y (the 8-bit lane source is the constant in the immediate
   table: `0x1000000.b3` is 1, `0x3020100.b2` is 2, `lut8`, fau.rs:371). The
   disassembly prints `.b33` / `.b22` for CLPER and `.b3` / `.b2` for a
   shift: the same lanes1 code, 7 / 6.
3. `FADD d, n, a.neg`: `neighbour - self` (`FADD.f32` at 32 bits,
   `FADD.v2f16` at 16, both halves the same word).
4. **the sign fix.** Lane `L` computed `value(L^axis) - value(L)`: the lanes
   with the axis bit set got the negated derivative. Unless every use of the
   result ignores the sign bit (`nir_def_all_uses_ignore_sign_bit`, as
   `mali-alu-ops.md` records, *(inferred: not vendored)*) the result is
   XORed with the sign bit of those lanes:
   * a mask `m`: X: `lane_id` itself (shifting left by `sz-1` pushes every
     bit but bit 0 out of the lane); Y: `LSHIFT_AND.i32 lane_id, 0, 2`
     (`lane_id & 2`; without the mask bit 0 would land on the exponent);
   * `LSHIFT_XOR.{i32|v2i16} d, m, #(sz - ffs(axis)), d`: shift 15 (X) or
     14 (Y) at fp16, 31 / 30 at fp32. The shift amounts are table bytes:
     `0xF0E0D0C.b33` / `.b22` (fp16), `0x1F1E1D1C.b33` / `.b22` (fp32).

`gl_FragCoord.y` and `v.y` are scaled by the y-flip `T.x` **before** the
`CLPER` (`fddy(a * T.x)`; the product of derivatives and `T.x` is the same
value, the reference puts the multiplication on the operand; *(inferred)* it
is `st_nir_lower_wpos_ytransform`'s ddy case, `nir_lower_wpos_ytransform.c`,
that nir_fs.rs's `frag_coord` doc already cites). X has no scale.

## 2. Case 138, every instruction

`fsarith_138_deriv.expect.json`, `fs.fragment`, 37 words, in order. `rN` are
the registers of the reference; `.h0`/`.h1` halves; `u2.w0 = T.x`, `u2.w1 =
T.y`, `u3.w0 = 2`, `u3.w1 = 0x30003000` (0.125 pair), `u4.w0 = 0x3c003c00`
(1.0 pair), the FAU words in the json.

| # | Word | Instruction | What it computes, and why Mesa emits it |
|---|---|---|---|
| 0 | `085c80041200007d` | `LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0` | `v` (vec2, two fp32 words). `.store`: the load keeps its value for a helper lane. Existing path (`LdVarBufImmF32`, the link's varying). |
| 1 | `00a5c00000044140` | `V2F32_TO_V2F16 r0, r0^, r1^` | lower_precision narrows both lanes read (`v.x`, `v.y`) into one pair: r0 = (vx, vy). Row `(F2f16,16,2)`, nir_fs.rs `vectorise_int_sources` (the `arity = 2` case, ~7913). |
| 2 | `0090c1001015003b` | `U16_TO_F32 r1, r59.h1` | `gl_FragCoord.y` source: pixel y from the preload r59 (`Kind::PixelCoord`). `frag_coord`, nir_fs.rs:1237. |
| 3 | `0114c13f00000041` | `FADD_IMM.f32 r1, r1^, #0x3F000000` | `+ 0.5` (integer pixel centres). |
| 4 | `00b2c10000858441` | `FMA.f32 r1, r1^, u2.w0, u2.w1` | `y * T.x + T.y`, the y transform. |
| 5 | `00a441100000c041` | `FADD.f32 r1.h0, r1^, 0x0.neg` | `f2f16(y)`: the mediump built-in narrowed, low half only. Row `(F2f16,16,1)`; `frag_coord16`, nir_fs.rs:1270. |
| 6 | `00a442100000c084` | `FADD.f32 r2.h0, u2.w0, 0x0.neg` | `f2f16(T.x)`: the scale for the Y derivatives, narrowed because both its readers are fp16. A new narrow of the state-uniform lane (`F2f16` of `Kind::Uniform{FB_WPOS_Y_TRANSFORM}` lane 0), shared by 7 and 9. |
| 7 | `00b3c10402c00241` | `FMA.v2f16 r1, r1^.h00, r2.h00, 0x0.neg` | `y16 * T.x16` in both halves: the Y operand of `dFdy(gl_FragCoord.y)`. Row `(Fmul,16,1)`. |
| 8 | `00a0c311180fc801` | `CLPER.i32.subgroup4.xor r3, r1, 0x3020100.b22` | neighbour in Y (lane ^ 2). **new op**. |
| 9 | `00a5c11028004143` | `FADD.v2f16 r1, r3^, r1^.neg` | raw `dFdy(fc.y)` (sign unfixed). Row `(Fadd,16,2)`-shaped with `neg` on the second source. |
| 10 | `00b3c20432c04200` | `FMA.v2f16 r2, r0.h11, r2^.h00, 0x0.neg` | `v.y * T.x16`: the Y operand of `dFdy(v.y)`, read as the pair's high half (the swizzle `h11` rides on the FMA, so no `U16_TO_U32`). |
| 11 | `00a0c311180fc802` | `CLPER.i32.subgroup4.xor r3, r2, 0x3020100.b22` | Y neighbour of `v.y * T.x`. |
| 12 | `00a5c21028004243` | `FADD.v2f16 r2, r3^, r2^.neg` | raw `dFdy(v.y)`. |
| 13 | `0090c3000015007b` | `U16_TO_F32 r3, r59^.h0` | `gl_FragCoord.x` source: pixel x. (Last read of r59: `^`.) |
| 14 | `0114c33f00000043` | `FADD_IMM.f32 r3, r3^, #0x3F000000` | `+ 0.5`. No `FMA`: x has no transform. |
| 15 | `00a443100000c043` | `FADD.f32 r3.h0, r3^, 0x0.neg` | `f2f16(x)`. |
| 16 | `00a0c4111c0fc403` | `CLPER.i32.subgroup4.xor r4, r3, 0x1000000.b33` | neighbour in X (lane ^ 1). The operand is the lone narrow in its own register: no `U16_TO_U32`. |
| 17 | `00a5c31028004344` | `FADD.v2f16 r3, r4^, r3^.neg` | raw `dFdx(fc.x)`. |
| 18 | `0090c00000140040` | `U16_TO_U32 r0, r0^.h0` | `v.x` alone in a word for the `CLPER` (`MKVEC.v2i16 r0.h0, #0` lowered by `lower_mkvec_swz`): `CLPER` takes no half swizzle, and `v.x` is the low half of the two-lane pair r0. |
| 19 | `68a0c4111c0fc400` | `CLPER.i32.subgroup4.xor.discard r4, r0, 0x1000000.b33` | X neighbour of `v.x`. `.discard` is the **flow** field (13): the *last* instruction that uses helper lanes ends them (`va_insert_flow_control_nops`, flow.rs:372, `uses_helpers`). A `CLPER` is a helper user: **flow.rs:35 must learn it** (section 4). |
| 20 | `00a5c01028004044` | `FADD.v2f16 r0, r4^, r0^.neg` | raw `dFdx(v.x)`. |
| 21 | `0091c40000000086` | `MOV.i32 r4, u3.w0` | the constant 2 for the mask. `LSHIFT_AND` reads `lane_id` (a special FAU, page 3) and the constant (a pushed FAU word): the two cannot share an instruction, `va_repair_fau` moves the constant (`fau.rs::repair_fau`, `FauState::valid` Special/Uniform). |
| 22 | `06b4c4011044c0e2` | `LSHIFT_AND.i32 r4, lane_id.w0, 0x0.b00, r4^` | `lane_id & 2`. Operands `(value, mask, shift)` as the existing `LshiftAndI32`; `lane_id` = word `0xe2` = page 3, entry 1, half 0 (`0xc0 | 32 + 2*1`). One instruction for both Y results (CSE'd by `opt::cse`, instructions 7/10's sign fixes). |
| 23 | `00b5c1011a41cb04` | `LSHIFT_XOR.v2i16 r1, r4.h00, 0xF0E0D0C.b22, r1^` | sign fix of `dFdy(fc.y)`: `(m << 14) ^ r1` per half. **new op in BIR** (`LshiftXorV2i16`). |
| 24 | `00b5c2011a42cb44` | `LSHIFT_XOR.v2i16 r2, r4^.h00, 0xF0E0D0C.b22, r2^` | the same for `dFdy(v.y)`; last read of the mask. |
| 25 | `06b5c3011e43cbe2` | `LSHIFT_XOR.v2i16 r3, lane_id.w0.h00, 0xF0E0D0C.b33, r3^` | sign fix of `dFdx(fc.x)`: `(lane_id << 15) ^ r3`, no mask. `lane_id` read through `.h00`: a special with a half swizzle. |
| 26 | `00a5c10000004143` | `FADD.v2f16 r1, r3^.h00, r1^.h00` | `dFdx(fc.x) + dFdy(fc.y)`, one lane each in both halves. Row `(Fadd,16,1)`. |
| 27 | `06b5c0011e40cbe2` | `LSHIFT_XOR.v2i16 r0, lane_id.w0.h00, 0xF0E0D0C.b33, r0^` | sign fix of `dFdx(v.x)`. (Mesa schedules it after the FADD of 26; the order is `sched.rs`'s, section 5.) |
| 28 | `00b3c20033da42de` | `FMA.v2f16 r2, 0x48000000.h11, r2^.h00, 0x38000000.h11` | `0.5 + 8.0 * dFdy(v.y)` fused (`ffma`, late fusion): 8.0 and 0.5 are fp16 halves of table entries. Row `(Ffma,16,1)`. |
| 29 | `00b3c00033da40de` | `FMA.v2f16 r0, 0x48000000.h11, r0^.h00, 0x38000000.h11` | the same for red, `dFdx(v.x)`. |
| 30 | `00a1c00000054240` | `MKVEC.v2i16 r0, r0^.h0, r2^.h0` | (red, green) in one word: `Vec2` row. |
| 31 | `00b3c10023da4187` | `FMA.v2f16 r1, u3.w1, r1^.h00, 0x38000000.h11` | blue: `0.5 + 0.125 * sum`; 0.125 is the pushed pair `0x30003000` (not in the table). |
| 32 | `40a1c10000058841` | `MKVEC.v2i16.wait0126 r1, r1^.h0, u4.w0.h0` | (blue, alpha 1.0): the alpha is the half of the pushed pair. `wait0126`: scoreboard wait for the varying load (slot 0) before the last registers are read; `flow.rs`. |
| 33 | `487dbc0200ead07c` | `ATEST.wait @r60, r60^, 0x3F800000, atest_datum.w0` | coverage test on alpha 1.0. Existing `blend()` tail. |
| 34 | `787f4004733c00f0` | `BLEND.slot1.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0` | the fp16 `v4` colour store. Existing. |
| 35 | `0110f000000000c0` | `IADD_IMM.i32 r48, 0x0, #0x0` | the `BLEND` return stub. Existing. |
| 36 | `002fc1100000f1c0` | `BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1` | the same stub's branch. Existing. |

The highp form (`fsarith_derivs_es300`) has the same shape at 32 bits and is
fs-deriv's second half: per component `FMA.f32 .., u*.w*, 0x0.neg` (the
uniform product `t = v_t * a`), and for the Y components a further `FMA.f32
r, r, u4.w0, 0x0.neg` (`T.x`, **not narrowed**: the 32-bit `u4.w0` is read as
it is), `CLPER.i32.subgroup4.xor r4, r, 0x3020100.b22` (Y) or `0x1000000.b33`
(X), `FADD.f32 r, r4^, r^.neg`, `MOV.i32 r4, u4.w1` (the mask 2),
`LSHIFT_AND.i32 r4, lane_id.w0, 0x0.b00, r4^`, `LSHIFT_XOR.i32 r, m,
0x1F1E1D1C.b22 / .b33, r^`, `FADD.f32 r2, r2^, r3^` for `dFdx(z) + dFdy(w)`,
`MOV.i32.wait r3, 0x3F800000` for alpha, `ATEST`, `BLEND.v4.f32`. The last
`CLPER` (the first in source order, `t.x`) carries `.discard`.

## 3. The ISA table has every opcode

`src/shaderc/online/mali/isa/v10.rs` is generated from Mesa's `isa-v9-v14.xml`:

| Opcode | ISA table | BIR (`bir.rs`) | Packer (`pack.rs`) |
|---|---|---|---|
| `CLPER.i32` | `IsaOp::Clper = 38` (:267), `OpInfo` :11724, text "CLPER.i32" with `subgroup` (:13748); fields `src0`, `src1` (lane), `lane_op` (xor = 1), `subgroup` (subgroup4 = 1), `lanes1` (:2336) | **missing** | **missing** |
| `LSHIFT_XOR.v2i16` | `IsaOp::LshiftXorV2i16 = 256` (:703) | **missing** | **missing** |
| `LSHIFT_XOR.i32`, `LSHIFT_AND.i32` | present | `Op::LshiftXorI32` (bir.rs:437), `Op::LshiftAndI32` | the shared arm, pack.rs:638 |
| `U16_TO_U32`, `U16_TO_F32`, `MKVEC.v2i16`, `FADD.v2f16`, `FMA.v2f16`, `FADD.f32`, `FADD_IMM.f32`, `V2F32_TO_V2F16`, `FMA.f32`, `MOV.i32`, `ATEST`, `BLEND`, `BRANCHZI`, `IADD_IMM.i32` | present | present | present |
| `lane_id` (FAU special page 3, entry 1: `M_FAU_SPECIAL_PAGE_3`, v10.rs:12063) | table has `.lane_id` | `Special` has `AtestDatum`, `BlendDescriptor`, `ProgramCounter` only: **add `LaneId`** | `src_byte` (pack.rs:29-32) and `tail` (pack.rs:176, page 3 only for `ProgramCounter`): **add both** |

Encoding facts read off the words, to pin in a test: `CLPER` `00a0c311180fc801`:
`src0` 0x01 (r1), `src1` 0xc8 (table entry 8), `lanes1` = 6 (b2) at bits
26-29, `lane_op` 1 and `subgroup` 1 (bits 32-39 = `0x11`), `dest_width` full;
the X one `00a0c4111c0fc403`: `src1` 0xc4, `lanes1` = 7. `byte_lane()`
(pack.rs:93) already maps `B0..B3` to 4..7, which is also the `i16` shift's
`b00 b11 b22 b33`, so **`LSHIFT_XOR.v2i16` needs no new swizzle**: `lut8(14)`
finds byte 2 of `0x0f0e0d0c` (`Swz::B2`, 6) and `lut8(15)` byte 3 (7).

## 4. Where each piece goes

No file listed here changes in this item; these are fs-deriv's places.

**`nir.rs`** (shared with the vertex walk): add `Alu::Fddx` and `Alu::Fddy`
to `enum Alu` (:96) with arity 1 (`arity()` defaults to 2 at :286, list them
beside the 1-ary ones), and keep them out of `commutes()` and `is_int()`.
They are float operations: `Nir::alu` must not fold them (a derivative of a
constant is 0, Mesa folds it; an input of this kind has no reference, so a
`Val::Const` operand is refused).

**`nir_fs.rs`**:
* `Fs::y_scale(at) -> Val` next to `frag_coord` (:1237): the `Kind::Uniform {
  dword: FB_WPOS_Y_TRANSFORM, comps: 2 }` load `frag_coord` already makes
  (CSE'd by `Nir::load`), lane 0; and `y_scale16` the `f2f16` of it (a
  `NirNode` like `frag_coord16`'s, :1270), built once.
* `walk_nodes` (:3705), the **mediump** arms (case 138's shader is
  `precision mediump`, so `prec.highp` is false; `Fs::alu16` at :1740 is the
  path of a *highp* shader reading ES 1.00's mediump `gl_FragCoord` and is not
  this one: leave its default refusal, no reference): the arm that falls to
  `ir::Op::Alu(..) => Arithmetic` (:4216) gets `ir::Op::Alu(a @ (AluOp::Fddx
  | AluOp::Fddy), s)` beside the `Fceil` arm (:4120). Per lane
  `f.fp16_operand(lane(vals, s[0], i))` (a `gl_FragCoord` operand is already
  the narrowed `x16` / `y16` of `frag_coord16`, :3884); for Y first `f.op(
  AluOp::Fmul, operand, y_scale16, ..)` (keeps the identity rules), then
  `f.node(Alu::Fddx / Alu::Fddy, ..)` (:971). Only `comps_of == 1`; a vector
  derivative has no reference and stays `Arithmetic`.
* `Fs::alu32` (:2152, the `_ => Err(MaliError::Arithmetic)` at :2142): `Fddx`
  `=> self.op32(Alu::Fddx, ..)`; `Fddy` multiplies by the *fp32* `y_scale`
  first. The highp half.
* `select` (:6150): an `Alu::Fddx/Fddy` node reaches the generic `row(p,
  &mut rows, Row { op, bits, width }, &s)` at ~:7960 unchanged; nothing is
  special except the 16-bit operand: a source that is lane 0 of a two-lane
  node (`v.x`, instruction 18) must be widened, lane 1 (`v.y`) never is alone
  because the Y scale product absorbs the `h11` (instruction 10). The 32-bit
  operand source for the row is `src32(..)` plus `Swz`; the widening (below)
  is made inside the row.
* The scheduling order of the nodes (`order_keys`, :285; `sched.rs`'s pressure
  scheduler) decides the order in the table; section 6, risk 2.

**`emit.rs`** (`ROWS`, :44, `[(Row, &str); 87]`; `emit`, :200): four rows,
`(Fddx,16,1)`, `(Fddy,16,1)`, `(Fddx,32,1)`, `(Fddy,32,1)`, each pushing
several instructions as `(Alu::F2f16, 16, 1)` does (:213):
1. widen: at 16 bits, a source `s` that is `h0` of a pair gets `MkvecV2i16
   [s.h0, Const 0]` (`opt::lower_mkvec_swz` makes it `U16_TO_U32`, opt.rs:365-370);
   a lone fp16 register is passed as is;
2. `Op::Clper { lane_op: xor, subgroup4 }` with sources `[value, Src::of(
   Idx::Const(axis))]` (the 8-bit lane source, `lanes` in `bir::props`);
3. `FaddV2f16 [n, Src { neg: true, ..a }]` (`FaddF32` at 32 bits);
4. for Y: `m = LshiftAndI32 [Special::LaneId, Const 2, Const 0 (b00)]`
   (identical in every Y row: `opt::cse` merges them, `can_cse` allows it,
   opt.rs:574); then the XOR: `LshiftXorV2i16 [m.h00 or Special::LaneId
   .h00, d, Const(sz - ffs(axis))]` (`LshiftXorI32 [m, d, Const(..)]` at
   32 bits), the BIR order `(value, other, shift)` of `LshiftAndI32`
   (pack.rs:644 `va_swap_12`). The row's value is the XOR's destination.
The sign-fix skip (`nir_def_all_uses_ignore_sign_bit`) belongs to the walk,
not the row: the row takes a flag. No reference omits the fix, so fs-deriv
always emits it (risk 1).

**`bir.rs`**: `Special::LaneId` (:60); `Op::Clper`, `Op::LshiftXorV2i16`;
`props` (:826-900): `Clper` source 0 `info(false, false, false, false, 32)`,
source 1 `SrcInfo { lanes: true, ..info(false, false, false, false, 8) }` (the
shift's third-source arm, :891), `LshiftXorV2i16` in the
`LshiftAndV2i16 | LshiftOrV2i16` arms (:889-891: value 16-bit widen, other a
plain word, shift 8-bit `lanes`); the source-count table (:955-975: `Clper`
2, `LshiftXorV2i16` 3) and the destination-count list (:925-952, one);
`Op::Clper` is an ALU op with no message (`props().msg == None`).

**`opt.rs`**: `size()` (:39) lists `LshiftXorV2i16` as 16-bit (as
`LshiftAndV2i16`, :51); `mods()` has no entry (the `neg` on `FADD`'s second
source is the existing `FaddV2f16` entry; confirm `mod_prop_*` (:849) does not
fold that `neg` into the `FADD` or the `CLPER`).

**`fau.rs`**: nothing for the op; `word_equiv` (:781) already treats
`Special` generically. `FauState::valid` (:826) puts the constant `2` in
`MOV.i32` (instruction 21) because a `Special` and a pushed word are
exclusive on one instruction (`!two && has_special()`); the inserted `MOV` is
what the table shows. The shift constants and the lane constant are table
entries (`lut8`), the 2 and the pushed pair constants are pushed FAU words, laid
out in first-use order (`2`, then the 0.125 pair, then the 1.0 pair: json FAU
words 6, 7, 8).

**`pack.rs`**: `src_byte` (:27): `Idx::Special(Special::LaneId, hi) => 0xc0 |
(32 + 2*1 + hi)`; `tail` (:176): page 3 also when a source is `LaneId`; an
`Op::Clper` arm: `Fill::new(IsaOp::Clper)`, `src0` / `src1` = `src_byte`,
`lanes1` = `byte_lane(src[1].swz)`, `lane_op` 1, `subgroup` 1, `inactive_result`
default, `alu_dest(.., "dest_width")`, `tail`; an `Op::LshiftXorV2i16` arm in
the `LshiftAndV2i16 | LshiftOrV2i16` arm (:660) with `IsaOp::LshiftXorV2i16`.
(`pack.rs` and `bir.rs` hold exhaustive matches: the compiler lists the rest.)

**`flow.rs`**: `uses_helpers` (:35): add `Op::Clper`. `analyze_helper_requirements`
then marks the `CLPER`'s source values as needed by helper lanes
(`bi_instr_uses_helpers`, bi_helper_invocations.c, *(inferred from the
`.discard` on the last `CLPER` of the references)*), and `insert_nops`
(:372) puts the `DISCARD` on the last user. Without this, instruction 19
loses `.discard` and the bytes differ.

**`select.rs`** (the vertex selection) does not change: it only provides
`alu`, `preloads` and `f16_rtne`, which the fragment side already imports.
`mod.rs`: the `Arithmetic` message (:366-370) lists "+, -, * and abs"; update
it when a derivative is taken (the text is not a test pin; grep first).

## 5. Host tests fs-deriv adds

1. `src/shaderc/online/mali/tests.rs`, beside `ALU_POPULATION` (:4452): a
   population `DERIV_POPULATION` of `(name, vs, fs, json, vs_values)` for
   `fsarith_138_deriv` and `alu_deriv` (the same words) and
   `fsarith_derivs_es300`, run through `check_program_for` (:3815), which
   compiles both stages and compares the words, FAU and work register count
   to the byte. Case 138 is the one the sweep draws.
2. `the_selection_rows_are_the_references` (:1998) needs the new fragment
   sources in `fs_reference_sources()` (:2051): each of the four rows must be
   drawn by a population, and `ROWS` grows to `[.. ; 91]`.
3. Unit tests, word by word, for the two new ops in `pack.rs`: the CLPER
   words `00a0c311180fc801`, `00a0c311180fc802`, `00a0c4111c0fc403`,
   `68a0c4111c0fc400` (flow 13), the `LSHIFT_XOR.v2i16` words
   `00b5c1011a41cb04`, `00b5c2011a42cb44`, `06b5c3011e43cbe2`, `06b5c0011e40cbe2`,
   the `LSHIFT_AND.i32` word `06b4c4011044c0e2` and the `FADD.v2f16` word
   `00a5c11028004143`, each built from a `Prog` and compared, then decoded
   with `isa::decode`; `fau::repair_fau` on `LSHIFT_AND lane_id, Const 2`
   inserts one `MOV.i32` and keeps page 3.
4. `flow::uses_helpers(Clper)` true, and the `.discard` on the last of
   several `CLPER`s only.
5. Refusals that stay (a specific `MaliError`, not a panic): a derivative of
   a constant or a uniform; `dFdx` of a vector (`w > 1`) at 16 bits; a
   mixed-precision operand.
6. The sweep delta: `tools/mali/sweep_passsets.py --delta tools/cts/log` must
   still exit 0; case 138 moves from REFUSED to a draw. The board decides the
   pixel (the host cannot): that is board-sweep's.

## 6. Risks

1. **The sign-fix omission is unproven.** Mesa skips the fix when every use
   ignores the sign bit; the references have a signed use only, and the Mesa
   function is not vendored. fs-deriv always emits the fix. A `abs(dFdx(..))`
   then differs from Mesa's bytes (same image). Either accept that, or refuse
   `Fabs` / squares of a derivative with a specific error until a reference
   exists: fs-deriv's choice, to be written in this note's follow-up.
2. **Order of instructions.** The table's order (the `dFdy(fc.y)` chain
   first, `dFdx(fc.x)` next, then `v.x` last, its sign fix after the sum) is
   `bi_pressure_schedule`'s on Mesa's NIR order; the Rust walk gets the order
   from `order_keys` and `sched::pressure_schedule` (mod.rs:862). A different
   node order gives different registers and bytes. If it drifts, fix it in
   `order_keys` for the derivative nodes, as earlier fragment items did.
3. **Late `ffma` fusion at 16-bit scalar lanes** (instructions 28, 29, 31):
   `0.5 + 8.0 * d` must fuse into `FMA.v2f16 8.0.h11, d.h00, 0.5.h11`.
   `late_rules` (:6103) and the mediump `Fadd`/`Fmul` arms have the `Ffma, 16, 1` row (texmore_2)
   but not necessarily this fusion; check with the fragment of the table
   before the derivative is added.
4. **FAU page 3 with a half swizzle on a special** (instructions 25, 27).
   `src_byte` has no `hi` for the swizzle (`.w0.h00` is `swz`, not the `hi`
   word), `FauState::valid` has no test with a half swizzle on `Special`.
5. **Helper lanes.** Without `uses_helpers(Clper)` the program is correct on
   a full quad and wrong at a triangle edge (the helper lane exits early).
   The record is the `.discard` on instruction 19; add a test that fails
   without it. A fragment `discard` in the same shader is not in a reference:
   refuse it (a derivative after a conditional discard is undefined in GLSL).
6. **A garbage high half.** Instruction 15's `r3.h0` write leaves `r3.h1`
   as the fp32 low half; `CLPER` moves it, `FADD.v2f16` computes it and the
   readers use `.h00`. A reader that took the whole word (`MKVEC`, a store)
   would see junk: the row's value is only valid as `.h00`. Keep every
   consumer lane-form (`lone()` in emit.rs).
7. **The Y scale on a window.** `T.x` is `-1` for a window (`frame.rs`
   `fb_wpos_y_transform`: `(1, 0, -1, height)`); the FBO the reference ran has
   `1`. The board's case reads `0.75` in both, so the scale must be a runtime
   uniform (it is: `Kind::Uniform`), never folded as 1.
8. **Row budget and merge conflicts.** `rows` is a `u128`; `ROWS` has 87
   entries. fs-transc, fs-trunc and fs-fold add rows too; the array length
   and `tests.rs` populations are the conflict points. Add the four rows at
   the end of `ROWS`, in one hunk.
9. **`dFdx` in the vertex shader or in a mixed-precision fragment** have no
   reference (the vertex walk has no `fddx`; a mediump derivative of a highp
   varying is `MixedPrecision`): keep both refused.

## 7. Out of this item

* Board behaviour of the lanes: `CLPER` on a partial quad is not measured;
  board-sweep reads case 138's three channels (0.75 each).
* `dFdx` of vectors, `fwidth`, `dFdx` of a loop-carried value, a derivative
  inside an `if`: no reference; the `fwidth` call is `abs(dFdx) + abs(dFdy)`
  in the front end (`builtin_call.rs:1118`) and would need risk 1 settled.
