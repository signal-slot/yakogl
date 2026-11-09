<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall fragment stage: dFdx / dFdy (gl_test case 138 `deriv`)

**Status: design, implementation in isa-deriv (the ISA ops) and fs-deriv (the
lowering)** (plan #336, issue #33). Host research only: no file under `src/`
changed, no board run. First written at `17e91019`; every reference below is
re-checked at `61b77ca6` and the reference is rerun (section 8). **The
fragment-side plan changed on that check**: case 138 is a mediump shader and
takes `select`'s pair-graph route, not `walk_nodes` plus the generic row loop
(sections 4 and 8).

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
| `fsarith_138_deriv.*` | `tools/shaders/es100/deriv.frag` (byte-equal to the testdata `.frag`) | mediump, ES 1.00 |
| `alu_deriv.*` | the same `.frag` (the `fs.fragment` words and FAU are byte-equal to `fsarith_138_deriv`'s; checked) | mediump |
| `fsarith_derivs_es300.*` | `tools/shaders/es300/derivs.frag` (byte-equal; `dFdx(t.x), dFdy(t.y), dFdx(t.z) + dFdy(t.w)`) | highp, ES 3.00 |

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
   `opt.rs::lower_mkvec_swz`, :347); a lone fp16 in a fresh register is read as it is.
2. `CLPER.i32.subgroup4.xor n, a, axis`: the value of lane `L ^ axis`, `axis`
   = 1 for X, 2 for Y (the 8-bit lane source is the constant in the immediate
   table: `0x1000000.b3` is 1, `0x3020100.b2` is 2, `lut8`, fau.rs:369). The
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
| 1 | `00a5c00000044140` | `V2F32_TO_V2F16 r0, r0^, r1^` | lower_precision narrows both lanes read (`v.x`, `v.y`) into one pair: r0 = (vx, vy). Row `(F2f16,16,2)`; on case 138's route it is `convert` (nir_fs.rs:8580, the `Some(true)` pair at :8608, `Leaf::Half`), not `vectorise_int_sources`, which is the highp/`guarded16` route's (:7378, :7913). |
| 2 | `0090c1001015003b` | `U16_TO_F32 r1, r59.h1` | `gl_FragCoord.y` source: pixel y from the preload r59 (`Kind::PixelCoord`). `frag_coord`, nir_fs.rs:1237; the pair-graph route's own rows are `frag16` (:8722), which builds x and y together and `MKVEC`s them: a derivative needs each alone (section 4). |
| 3 | `0114c13f00000041` | `FADD_IMM.f32 r1, r1^, #0x3F000000` | `+ 0.5` (integer pixel centres). |
| 4 | `00b2c10000858441` | `FMA.f32 r1, r1^, u2.w0, u2.w1` | `y * T.x + T.y`, the y transform. |
| 5 | `00a441100000c041` | `FADD.f32 r1.h0, r1^, 0x0.neg` | `f2f16(y)`: the mediump built-in narrowed, low half only. Row `(F2f16,16,1)`; `frag_coord16`, nir_fs.rs:1270 (the nodes); `frag16` :8722 (the rows). |
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
| 19 | `68a0c4111c0fc400` | `CLPER.i32.subgroup4.xor.discard r4, r0, 0x1000000.b33` | X neighbour of `v.x`. `.discard` is the **flow** field (13): the *last* instruction that uses helper lanes ends them (`insert_nops`, flow.rs:317, its "terminate helpers after the last use" `NOP.discard`, which `merge_discard`, :476, joins to the instruction before it; `uses_helpers`). A `CLPER` is a helper user: **flow.rs:35 must learn it** (section 4). |
| 20 | `00a5c01028004044` | `FADD.v2f16 r0, r4^, r0^.neg` | raw `dFdx(v.x)`. |
| 21 | `0091c40000000086` | `MOV.i32 r4, u3.w0` | the constant 2 for the mask. `LSHIFT_AND` reads `lane_id` (a special FAU, page 3) and the constant (a pushed FAU word): the two cannot share an instruction, `va_repair_fau` moves the constant (`fau.rs::repair_fau` :847, `FauState::valid` :814, Special/Uniform). |
| 22 | `06b4c4011044c0e2` | `LSHIFT_AND.i32 r4, lane_id.w0, 0x0.b00, r4^` | `lane_id & 2`. Operands `(value, mask, shift)` as the existing `LshiftAndI32`; `lane_id` = word `0xe2` = page 3, entry 1, half 0 (`0xc0 | 32 + 2*1`). One instruction for both Y results (CSE'd by `opt::cse`; instructions 23 and 24 read it). |
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

## 3. The ISA table has every opcode (re-checked at `61b77ca6`)

`src/shaderc/online/mali/isa/v10.rs` is generated from Mesa's `isa-v9-v14.xml`
(unchanged since this note's first version; `isa-transc`, in review, touches
bir.rs, pack.rs and opt.rs and will move their line numbers):

| Opcode | ISA table | BIR (`bir.rs`) | Packer (`pack.rs`) |
|---|---|---|---|
| `CLPER.i32` | `IsaOp::Clper = 38` (:267), `OpInfo` :11724, text "CLPER.i32" with `subgroup` (:13748); `SYMS_38` :2337, `OPERANDS_38` :2352 | **missing** | **missing** |
| `LSHIFT_XOR.v2i16` | `IsaOp::LshiftXorV2i16 = 256` (:703), `OpInfo` :11942, `SYMS_256` :8527, `OPERANDS_256` :8544 (the fields of `SYMS_248`, `LSHIFT_AND.v2i16`, :8281, with `bitwise_operation` = 2) | **missing** (`LshiftXorI32` :437, `LshiftAndV2i16` :566) | **missing** (the shared 32-bit arm :638-659, the `v2i16` arm :660) |
| `LSHIFT_XOR.i32`, `LSHIFT_AND.i32` | present | `Op::LshiftXorI32` (:437), `Op::LshiftAndI32` (:557) | the shared arm, pack.rs:638 |
| `U16_TO_U32`, `U16_TO_F32`, `MKVEC.v2i16`, `FADD.v2f16`, `FMA.v2f16`, `FADD.f32`, `FADD_IMM.f32`, `V2F32_TO_V2F16`, `FMA.f32`, `MOV.i32`, `ATEST`, `BLEND`, `BRANCHZI`, `IADD_IMM.i32` | present | present | present |
| `lane_id` (FAU special page 3, entry 1: `M_FAU_SPECIAL_PAGE_3`, v10.rs:12063) | table has `.lane_id` | `Special` (:60) has `AtestDatum`, `BlendDescriptor`, `ProgramCounter` only: **add `LaneId`** | `src_byte` (pack.rs:24-34, `ProgramCounter` at :32) and `tail` (pack.rs:175-179, page 3 only for `ProgramCounter`): **add both** |

Mesa's own assembler test confirms the special's byte and page:
`third_party/mesa-26.2.1/src/panfrost/compiler/bifrost/valhall/test/assembler-cases.txt:5`
is `e2 00 00 00 00 c1 91 06  MOV.i32 r1, lane_id.w0` (`src0` 0xe2 =
`0xc0 | (32 + 2*1 + 0)`, `fau_page_index` 3 in the last byte).

## 4. isa-deriv, step by step

The ISA side only: no row, no node, no lowering. The item is finished when the
words below pack to the bytes and `uses_helpers` knows the op. **Encoding
words**, all from `fsarith_138_deriv.expect.json` `fs.fragment`, fields read
off `SYMS_38` / `SYMS_256`:

| Word | Instruction | Fields |
|---|---|---|
| `00a0c311180fc801` | `CLPER.i32.subgroup4.xor r3, r1, 0x3020100.b22` | `src0` 0x01, `src1` 0xc8 (`IMMEDIATES[8]`), `lanes1` 6 (`b2`), `lane_op` 1 (xor), `subgroup` 1 (subgroup4), `inactive_result` 0, `dest_register_index` 3, `dest_width` full, `fau_page_index` 0, `flow` 0 |
| `00a0c311180fc802` | the same, `r3, r2, ..b22` | `src0` 0x02 |
| `00a0c4111c0fc403` | `CLPER.i32.subgroup4.xor r4, r3, 0x1000000.b33` | `src0` 0x03, `src1` 0xc4 (`IMMEDIATES[4]`), `lanes1` 7 (`b3`), dest 4 |
| `68a0c4111c0fc400` | `CLPER.i32.subgroup4.xor.discard r4, r0, 0x1000000.b33` | `src0` 0x00, `flow` 13 (`flow::DISCARD`, bits 59-62) |
| `00b5c1011a41cb04` | `LSHIFT_XOR.v2i16 r1, r4.h00, 0xF0E0D0C.b22, r1^` | BIR `(value, other, shift)` = `(r4.h00, r1 last use, Lut(11).b2)`: `src0` 0x04, `src2` 0x41 (bit 6 = last use), `src1` 0xcb (`IMMEDIATES[11]`), `lanes1` 6, `bitwise_operation` 2, `shift_operation` 1, `swz0` 0 (h00), `not2` 0, `not_result` 0 |
| `00b5c2011a42cb44` | `LSHIFT_XOR.v2i16 r2, r4^.h00, 0xF0E0D0C.b22, r2^` | `src0` 0x44 (r4 last use), `src2` 0x42 |
| `06b5c3011e43cbe2` | `LSHIFT_XOR.v2i16 r3, lane_id.w0.h00, 0xF0E0D0C.b33, r3^` | `src0` 0xe2 (`LaneId`, `hi` false), `lanes1` 7, `swz0` 0, `fau_page_index` 3 (byte 7 = 0x06) |
| `06b5c0011e40cbe2` | `LSHIFT_XOR.v2i16 r0, lane_id.w0.h00, 0xF0E0D0C.b33, r0^` | dest 0, `src2` 0x40 |
| `06b4c4011044c0e2` | `LSHIFT_AND.i32 r4, lane_id.w0, 0x0.b00, r4^` | the existing arm: `src0` 0xe2 (value), `src2` 0x44 (mask r4, last use), `src1` 0xc0 (`IMMEDIATES[0]`), `lanes1` 4, `fau_page_index` 3 |

The shift constants are `lut8`'s output (fau.rs:369): the first byte of
`IMMEDIATES` equal to the value, 14 -> `0x0F0E0D0C` byte 2 (`.b22`), 15 -> byte
3 (`.b33`), 1 -> `0x01000000` byte 3, 2 -> `0x03020100` byte 2. `byte_lane`
(pack.rs:93) already maps `B0..B3` to 4..7, which is also the `i16` shift's
`b00 b11 b22 b33`: no new swizzle.

Steps:

1. **`bir.rs`**: `Special::LaneId` after `ProgramCounter` (:60-70; doc: `lane_id`,
   page 3 entry 1, the lane's index in its quad). `Op::Clper` (no payload:
   the lane op is always xor and the subgroup 4; the one user fixes both)
   beside `LshiftXorI32` (:437), and `Op::LshiftXorV2i16` beside
   `LshiftAndV2i16` (:566). `src_info` (:776, the arms at :836-891):
   `(Op::Clper, 0) => info(false, false, false, false, 32)`, `(Op::Clper, 1) =>
   SrcInfo { lanes: true, ..info(false, false, false, false, 8) }`;
   `LshiftXorV2i16` joins the three `LshiftAndV2i16 | LshiftOrV2i16` arms
   (:889-891). `nr_srcs` (:914): `Clper` in the 2-source list (~:950),
   `LshiftXorV2i16` in the 3-source list (~:964). Both ops are ALU ops
   (`props().msg == None`).
2. **`pack.rs`**: `src_byte` (:24): `Idx::Special(Special::LaneId, hi) => 0xc0
   | (32 + 2 + hi as u32)`; `tail` (:175): page 3 when any source is
   `ProgramCounter` **or** `LaneId` (today only `ProgramCounter` with `hi`
   false is tested). New arm `Op::Clper`: `Fill::new(IsaOp::Clper)`, `src0`
   and `src1` from `src_byte`, `lanes1` = `byte_lane(i.src[1].swz)`, `lane_op`
   1, `subgroup` 1, `inactive_result` 0, `alu_dest(&mut f, i, "dest_width")`,
   then the common `tail`. `Op::LshiftXorV2i16` goes in the `LshiftAndV2i16 |
   LshiftOrV2i16` arm (:660) with `IsaOp::LshiftXorV2i16`.
3. **`opt.rs`**: `size()` (:37): `LshiftXorV2i16` in the 16-bit list (:48).
   `mods()` (:62) has no entry for either op. `can_cse` (:574) lets `Clper`
   through (`msg == None`).
4. **`flow.rs`**: `uses_helpers` (:35): `Op::TexSingle { lod: Computed |
   ComputedBias, .. } | Op::Clper`. `analyze_helper_requirements` (:47) needs
   nothing else (its `skip` rewrite names `TexSingle` only). `insert_nops`
   (:317, the "terminate helpers after the last use" `find`) and
   `merge_discard` (:476) then give `.discard` to the last `CLPER`; with no
   `CLPER` the program keeps its leading `NOP.discard`.
5. **`fau.rs`**: no code. `FauState::valid` (:814) takes one kind of `Special`
   per instruction and `word_equiv` (:782) ignores the half. `select.rs` and
   `emit.rs` do not change unless the compiler lists an exhaustive match (at
   `61b77ca6` `LshiftXorI32` is in bir.rs :437, :862, :871, :880, :966,
   pack.rs :640, :647 and emit.rs :336 only).
6. No row, no `ROWS` change and no `corpus-verdicts.txt` change in this item:
   nothing lowers a derivative yet.

Host tests, in `src/shaderc/online/mali/tests.rs` (it has `use super::isa;`),
through `pack::pack_one(&Ins, 0)` (pack.rs:199):

* `the_clper_words_are_mesa_s`: the four `CLPER` words built from
  `Ins::new(Op::Clper, &[Dest..r3], &[reg, Src { idx: Idx::Lut(8), swz:
  Swz::B2, .. }])` (`Lut(4)` / `Swz::B3` for the X ones, `ins.flow =
  flow::DISCARD` for the last) pack to the word; each word is in
  `region(json, "fs.fragment").words` of `fsarith_138_deriv.expect.json`;
  `isa::decode(word)` is `IsaOp::Clper` and `isa::encode` returns the word.
* `the_lane_id_words_are_mesa_s`: the four `LSHIFT_XOR.v2i16` words and the
  `LSHIFT_AND.i32` word, with `Src::of(Idx::Special(Special::LaneId, false))`
  (`.swz(Swz::H00)` on the `v2i16` ones); page 3 exactly when a source is
  `LaneId` (`06..` in byte 7), and a `CLPER` or an ordinary `LSHIFT_XOR`
  stays at page 0.
* `a_lane_id_and_a_pushed_constant_do_not_share_an_instruction`: a `Prog` of
  `LSHIFT_AND.i32 lane_id, Const(2), Const(0)` through `fau::repair_fau`
  (:847) gets exactly one `MOV.i32` for the constant and keeps the
  `Special`.
* `a_clper_is_a_helper_user`: `flow::uses_helpers(&clper)`; a `Prog` of a load,
  `CLPER`, `CLPER`, `FADD` through `flow::insert_flow_control_nops(p, true)`
  and `merge_flow` has `.discard` on the second `CLPER` only and no leading
  `NOP.discard`; the same program without the `CLPER`s has the leading one.

No sweep and no whole-program byte test here: those are fs-deriv's.

## 5. fs-deriv, step by step (the route changed at the re-check)

**Which route case 138 takes.** `select` (nir_fs.rs:6150) builds a mediump
shader's colour as a *pair graph* (`Pairs`, `Leaf`; `vectorise` :5638,
`lane_word` :5910, `lane_tree` :5962; events sorted by IR key, the event
loop at :6330-6510; rows through `row`). The generic loop over `f.nir.order`
that the first version of this note cites (":7960") is `select_highp`'s,
which a mediump shader does not use. Measured with a throwaway (section 8):
once the walk accepts `fddx`/`fddy`, case 138 stops in `vectorise`'s last
arm, not at an unknown row. So the mediump half is a new leaf of the pair
graph, not an arm and a row. In the order a build meets it:

1. **`nir.rs`**: `Alu::Fddx`, `Alu::Fddy` in `enum Alu` (:96, after `Fceil`),
   arity 1 (the list at :252-277); not in `commutes` (:297) or `is_int`
   (:317). The exhaustive `match op` of the constant fold (`let r = match op`
   at :1874, its refusal arm :1930) must name them: a derivative of a
   constant has no reference (`Err(MaliError::ConstantFold)`).
2. **`walk_nodes` (nir_fs.rs:3705)**, the mediump arms, before the `Fceil`
   arm (:4120) and the default `ir::Op::Alu(..) => return Err(Arithmetic)`
   (:4216). The front end gives every call a 4-wide node whose scalar operand
   is swizzled `[0, 0, 0, 0]` (`dFdx(v.x)`: `comps_of` 4, measured), so the arm
   loops over `w = comps_of` lanes like the `Fceil` arm and `Fs::node`'s CSE
   (:971) makes them one node. This arm was built as a throwaway and the walk
   accepted all of case 138:
   ```rust
   ir::Op::Alu(a @ (AluOp::Fddx | AluOp::Fddy), s) => {
       let w = m.comps_of(ir::ValueId(id.0)) as usize;
       for (i, o) in out.iter_mut().enumerate().take(w) {
           let mut x = f.fp16_operand(lane(vals, s[0], i));
           if a == AluOp::Fddy {
               let u = Val::Undef;
               let t = f.nir.load(Kind::Uniform { dword: FB_WPOS_Y_TRANSFORM, comps: 2 })?;
               let t16 = f.node(Alu::F2f16, [Val::Lane(t, 0), u, u], at)?;
               x = f.op(AluOp::Fmul, x, t16, at, Rule::None)?;
           }
           *o = f.node(if a == AluOp::Fddx { Alu::Fddx } else { Alu::Fddy }, [x, Val::Undef, Val::Undef], at)?;
       }
   }
   ```
   A `Val::Const` operand, a vector derivative whose lanes differ and a
   mixed-precision operand stay refused (risk 9).
3. **A leaf for the derivative.** `Leaf::Deriv(n)` (enum at :5487), `n` the node
   of step 2:
   * `lane_tree` (:5962): an arm for `Fddx | Fddy` that pushes
     `P::Leaf(Leaf::Deriv(n))`; its `Fadd`-of-`Fmul` fusion already makes
     instructions 28, 29 and 31 (`ffma(8.0, d, 0.5)`, `ffma(0.125, sum,
     0.5)`).
   * `lone_tree` (:5927): `walk` returns a new class for a derivative so that
     `(blue, 1.0)` is `lane_word`'s (:5910) `Leaf::Lane` (instruction 32,
     `MKVEC.v2i16 r1, r1^.h0, u4.w0.h0`).
   * `vectorise` (:5638): before the `(Fadd, Fadd)` arm, an arm for a pair
     one of whose trees holds a derivative: `x = lane_tree(a)`, `y =
     lane_tree(b)`, `P::Leaf(Leaf::Pack { x, y })` (what `select_lane` does
     for the selects of constants); `Pack` is emitted at :6559 as
     `MKVEC.v2i16 x.h0, y.h0` (instruction 30). Mesa does not pair `dFdx` with
     `dFdy`: `nir_opt_vectorize` needs the same definition in both lanes
     (`alu_srcs_equal`) and the references show two scalar `FMA`s.
   * `Form` (:598) and `prod` / `ADMITTED` (:639, :646): the leaf is a new
     `Form`, or `Form::Op` if its places are the `Fma` / `Add` ones the
     references draw. A production outside `ADMITTED` is `FsOperandForm`
     (:6188; predicted, not run).
4. **Emission.** A new event kind `DERIV` in `select`'s events (:6330-6420;
   `FRAG` at :6353 is the template: key `f.key(..)`, emitted in the loop at
   :6510). Per derivative node, through `row`:
   * the operand word: a varying lane (`lv.conv[k][c]`, the
     `V2F32_TO_V2F16` word of `convert` :8580): lane 0 is read alone through
     a `U16_TO_U32` (a `Row { Vec2, 16, 2 }` with a zero, which
     `opt::lower_mkvec_swz` rewrites, instruction 18); lane 1 is only read
     for `dFdy` and rides the `FMA`'s `.h11` (instruction 10). A narrowed
     `gl_FragCoord` lane: x is `U16_TO_F32 r59.h0`, `+ 0.5`, `F2f16`; y is
     `U16_TO_F32 r59.h1`, `+ 0.5`, `FMA.f32 .., T.x, T.y`, `F2f16`. `frag16`
     (:8722) builds both and `MKVEC`s them, so split it into a per-lane
     builder the `Leaf::Frag` word and the derivative share (instructions
     2-5 and 13-15; the reference starts with y).
   * for Y the scale: `f2f16(T.x)` once (instruction 6, `(F2f16, 16, 1)`),
     then `(Fmul, 16, 1)` of operand and scale (7, 10).
   * the derivative rows `(Fddx, 16, 1)` and `(Fddy, 16, 1)` (`ROWS`, emit.rs:39,
     87 -> 89 entries; `rows` is a `u128`).
5. **`emit.rs`**: the two rows, each pushing several instructions as
   `(Alu::F2f16, 16, 1)` does (:214), the source being the operand word
   (already widened):
   1. `Op::Clper` `[value, Src::of(Idx::Const(axis))]` (axis 1 for X, 2 for
      Y; `fau.rs` turns the constant into the `Lut(..)` byte lane).
   2. `FaddV2f16 [n, Src { neg: true, ..value }]` (operands through `lone`,
      :195).
   3. Y only: `m = LshiftAndI32 [Special::LaneId, Const(2), Const(0)]`
      (`opt::cse` merges the identical masks of several Y rows).
   4. `LshiftXorV2i16 [m.h00 | LaneId.h00, d, Const(14 | 15)]` (X: `LaneId.h00`
      and 15; Y: the mask and 14).
   The sign fix is always emitted (risk 1).
6. **Order and registers.** The select-level order is the pair graph's;
   `sched::pressure_schedule` (mod.rs:862, sched.rs:341) reorders the BIR
   program afterwards (`bi_pressure_schedule`). If the bytes drift, look at
   the event keys first, then at `sched.rs`'s treatment of `Clper` (no
   message, one destination).
7. **Highp (`fsarith_derivs_es300`), optional and second.** `Fs::alu32` (:2152,
   its default `_ => Err(MaliError::Arithmetic)` at :2142): `AluOp::Fddx =>
   self.op32(Alu::Fddx, [x, u, u], at)`; `AluOp::Fddy` multiplies by the fp32
   `T.x` lane first (not narrowed: the reference reads `u4.w0`), then the
   same two rows at 32 bits (`(Fddx, 32, 1)`, `(Fddy, 32, 1)`, `FADD.f32`,
   `LSHIFT_XOR.i32` with `0x1F1E1D1C.b22 / .b33`). Measured with a throwaway:
   that arm alone ends in `Defect(IrIndex)` at a site that was not located,
   so the highp half needs its own look. It is not on case 138's path;
   `tools/mali/corpus-verdicts.txt:143` keeps its line if it is left.
8. **Fine and coarse.** `Fddx` and `Fddy` are the front end's only derivative
   ops (ir.rs:250-251, builtin_call.rs:181-182); GLSL ES 1.00
   (`GL_OES_standard_derivatives`) and 3.x have no coarse or fine variant, so
   only the form Mesa emits for `fddx` / `fddy` (the fine-lane `CLPER` of the
   three references) is needed. `fwidth` is `abs(dFdx) + abs(dFdy)` in the
   front end (builtin_call.rs:1118-1122) and stays out until risk 1 has a
   reference.

**Host tests (fs-deriv)**, in `src/shaderc/online/mali/tests.rs`:

1. `DERIV_POPULATION: [(&str, &str, &str, &str, bool); 2]`, beside
   `ALU_POPULATION` (:4514): `("fsarith_138_deriv", td!("fsarith_138_deriv.vert"),
   td!("fsarith_138_deriv.frag"), td!("fsarith_138_deriv.expect.json"), false)`
   and the same with `alu_deriv` (the same program, the same words); the highp
   `fsarith_derivs_es300` only with step 7. The test
   `the_derivative_population_is_byte_equal_to_mesa_26_2_1` runs
   `check_program_for` (:3878) on each (both IDVS programs and the fragment
   program: words, FAU, work registers).
2. `the_selection_rows_are_the_references` (:2003): `fs_reference_sources()`
   (:2052) gets `srcs.extend(DERIV_POPULATION.iter().map(|c| c.2))`; the test
   fails on a row that nothing draws.
3. `a_derivative_of_a_constant_is_refused`, `a_vector_derivative_is_refused`,
   `a_derivative_of_a_mixed_precision_value_is_refused`: the specific
   `MaliError`s, not a panic (sources edited from `deriv.frag`).
4. Generated: `tools/mali/corpus-verdicts.txt:30` (`tools/shaders/es100/
   deriv.frag`) turns `ok`; regenerate with `tools/mali/corpus-verdicts.sh`
   in the same commit (the gate compares the bytes). Leave `MaliError::
   Arithmetic`'s text (mod.rs:374) unchanged: every other refusal line of that
   file quotes it.
5. `tools/mali/sweep_passsets.py --delta tools/cts/log` must still exit 0; case
   138 moves from REFUSED to a draw. The board decides the pixel (0.75 in
   all three channels): board-sweep's.

## 6. Risks

1. **The sign-fix omission is unproven.** Mesa skips the fix when every use
   ignores the sign bit (`nir_def_all_uses_ignore_sign_bit`, not vendored);
   the references have signed uses only. fs-deriv always emits the fix. An
   `abs(dFdx(..))` then differs from Mesa's bytes (same image). Accept that,
   or refuse `Fabs` of a derivative with a specific error until a reference
   exists.
2. **Order of instructions.** The table's order is `bi_pressure_schedule`'s
   on Mesa's NIR order; the events give the BIR program and
   `sched::pressure_schedule` reorders it (step 6). A drift is fixed in the
   event keys of the derivative nodes, not in the rows.
3. ~~Late `ffma` fusion~~ **resolved by the route**: `lane_tree` (:5962) and
   `late_rules` (:6103) make the `ffma`s, and the rows `(Ffma, 16, 1)`,
   `(Fmul, 16, 1)`, `(Fadd, 16, 1)` exist (emit.rs:127, :104, :100). The 0.125
   pair is a pushed FAU word (`u3.w1`), 8.0 and 0.5 table halves
   (`0x48000000.h11`, `0x38000000.h11`): the FAU pass decides, nothing to
   code.
4. **FAU page 3 with a half swizzle on a special** (instructions 25, 27):
   `swz0` is the instruction's, the special's `hi` picks `w0` / `w1`
   (`src_byte`); `FauState::valid` has no test with a half swizzle on a
   `Special`: `the_lane_id_words_are_mesa_s` is it.
5. **Helper lanes.** Without `uses_helpers(Clper)` the program is correct on
   a full quad and wrong at a triangle edge (the helper lane exits early).
   Record: the `.discard` on instruction 19; the isa-deriv test fails
   without it. A fragment `discard` in a shader with a derivative is in no
   reference: refuse it (GLSL: a derivative after a conditional discard is
   undefined) with a new `MaliError` variant (the list at mod.rs:305, the
   messages at :374).
6. **A garbage high half.** Instruction 15's `r3.h0` write leaves `r3.h1` as
   the fp32 low half; `CLPER` moves it, `FADD.v2f16` computes it and the
   readers use `.h00`. A reader that took the whole word would see junk: the
   derivative's value is only valid as `.h00`; the pair graph's consumers
   (`Leaf::Pack`, `Leaf::Lane`) read `.h00` / `.h0`.
7. **The Y scale on a window.** `T.x` is `-1` for a window
   (`src/mali/frame.rs:577` `fb_wpos_y_transform`: `(1, 0, -1, height)`); the
   FBO the reference ran has `1`. The board's case reads `0.75` in both, so
   the scale must be a runtime uniform (it is: `Kind::Uniform`), never folded
   as 1.
8. **Row budget and merge conflicts.** `ROWS` has 87 entries (emit.rs:39).
   fs-transc, fs-trunc and fs-fold add rows too; the array length and the
   `tests.rs` populations are the conflict points. Add the rows at the end of
   `ROWS`, in one hunk.
9. **`dFdx` in the vertex shader or in a mixed-precision fragment** have no
   reference (the vertex walk has no `fddx`; a mediump derivative of a highp
   varying is `MixedPrecision`): keep both refused.
10. **The pair graph's capacity and productions.** `MAX_PAIR_NODES` is 255
    (nir_fs.rs:50); case 138 needs about 20. `ADMITTED` (:646) lists the
    productions the references drew; a new leaf outside it is
    `FsOperandForm`. Extend it for exactly the places case 138 draws.

## 7. Out of this item

* Board behaviour of the lanes: `CLPER` on a partial quad is not measured;
  board-sweep reads case 138's three channels (0.75 each).
* `dFdx` of vectors, `fwidth`, `dFdx` of a loop-carried value, a derivative
  inside an `if`: no reference.

## 8. Checked at `61b77ca6`

Tip of origin/main at the check (`vivante: plane colour masks and plane clear
words for 128-bit targets (inert)`); `isa-transc` was in review and is not in
that tree, so the bir.rs / pack.rs / opt.rs line numbers move when it lands.

**References.** Every `file:line` in sections 1-7 was read against that tree
and corrected in place: `fau.rs::lut8` :369, `opt.rs::lower_mkvec_swz` :347,
`flow.rs` `insert_nops` :317 and `merge_discard` :476 (the note had `:372`),
`emit.rs` `ROWS` :39 and `emit` :205, `bir.rs` `src_info` :776 and `nr_srcs`
:914 (the note had "`props`" and a "destination-count list"),
`tools/shaders/es100/deriv.frag` and `es300/derivs.frag` (the `gltest/` path
did not exist), `frame.rs` is `src/mali/frame.rs`. Names that held:
`Alu`, `Fceil`, `frag_coord`, `frag_coord16`, `Fs::node`, `alu16`, `alu32`,
`walk_nodes`, `select`, `late_rules`, `order_keys`, `repair_fau`, `can_cse`,
the ISA table lines, `MaliError::Arithmetic`, `FB_WPOS_Y_TRANSFORM` (:267).
**Wrong in the first version**: (a) case 138 does not reach the generic row
loop but the pair graph (section 5); (b) `vectorise_int_sources` is not the
source of instruction 1 (`convert` is); (c) `frag16` makes the position as
one word, the derivatives need lanes; (d) risk 3 (the `ffma` fusion) is the
pair graph's, not open.

**Reference rerun.** Mesa 26.2.1's gallium panfrost on drm-shim, built the
documented way (the options of `tools/mali/drm_shim_ref.sh build`:
`-Dgallium-drivers=panfrost -Dtools=drm-shim -Degl=enabled -Dgles2=enabled`,
LLVM 21) into `$CONNECTED_DEPS_DIR/mesa-pan/inst` from the tree
`mesa-pan/mesa-26.2.1` (the same one `build_pan_shc.sh` made; the shim in
`build-shim`); then per case `tools/mali/drm_shim_ref.sh run <work> X.vert
X.frag <prefix>` (64x64 framebuffer, default viewport and depth range, no
option) and `tools/mali/pan_ref_diff.py diff-ref <prefix>.trace
tools/mali/testdata/X.expect.json`:

| Case | Result |
|---|---|
| `fsarith_138_deriv` | `fs.fragment` byte-equal, 37 instructions, facts equal (10 FAU words); `vs.position` 24 and `vs.varying` 7 byte-equal; `secondary_offset` 256; **0 differ or missing**; `extract --draw` equals the committed JSON in words, text and FAU |
| `alu_deriv` | 3 regions, 0 differ or missing |
| `fsarith_derivs_es300` | 3 regions, 0 differ or missing |

The reference matches: sections 1-2's words and text are the committed
JSON's, re-read. The traces are not committed.

**The next refusal of case 138.** With a throwaway (uncommitted) `Alu::Fddx`
/ `Alu::Fddy` in nir.rs and section 5's `walk_nodes` arm, compiling
`fsarith_138_deriv.frag` through `compile(SHC_STAGE_FRAGMENT, ..)`
(tests.rs:172):

* today: `MaliError::Arithmetic` ("Valhall: arithmetic other than +, -, * and a
  dot or matrix product ...") from the default arm `ir::Op::Alu(..)`,
  nir_fs.rs:4216 (the operation is `Fddx`, `comps_of` 4, `highp` false);
* with the walk arm: **`MaliError::ColourForm`** ("a gl_FragColor lane other
  than the varying's own lane or a constant is not lowered yet") from the last
  arm of `vectorise`, nir_fs.rs:5790, on the pair of `Fddx` and `Fddy` leaves;
  the walk, the Y-scale node and the constant fold all passed;
* after that, **predicted, not run**: `FsOperandForm` (`ADMITTED`, :646) for a
  leaf outside the references' productions, then `Defect(IrIndex)` in
  `operand` (:8761) for a leaf with no emitted value.

For `fsarith_derivs_es300.frag` the walk-side refusal is `Fs::alu32`'s
default (nir_fs.rs:2142); with an `Fddx` / `Fddy` arm there it ends in
`Defect(IrIndex)` (site not located), so the highp half needs its own look
(step 7). `tools/mali/corpus-verdicts.txt:30` and `:143` are today's
`Arithmetic` lines for the two.

Not done here: no implementation, no board, no commit of traces or of the
throwaway edits.
