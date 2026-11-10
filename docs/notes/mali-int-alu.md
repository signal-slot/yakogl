<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the integer ALU group (`/ % >> ^ ~ min max`, `int()` / `uint()`, `bool(float)`)

**Status: design, no implementation item yet** (plan #336, issue #33; section A of
`mali-corpus-families.md`, row "integer ALU": 7 sources, no item covered them).
Host research only: nothing under `src/` or `tools/` changed. Measured at the tip
`f5bc25f4`.

The seven sources are refused with `MaliError::Arithmetic` (`corpus-verdicts.txt`).
This note gives, for each, the first refusal on the tip (section 1), the Valhall
instruction Mesa 26.2.1 emits per op (section 2, **measured** from a recorded
drm-shim reference, or **inferred** from Mesa's source where no reference draws
it), what the tip has (bir/pack/emit) and lacks, the order of implementation items
with files and host tests (section 3) and the cases each moves (section 4).

How the "measured" words were read: the `text` of `fs.fragment` in the reference's
`.expect.json` (Mesa's own disassembly of the recorded program, made by
`drm_shim_ref.sh run` + `pan_ref_diff.py extract --draw`; nothing was redrawn).
The Mesa source citations are the 26.2.1 tarball's (`src/compiler/nir/`,
`src/compiler/glsl/glsl_to_nir.cpp`, `src/panfrost/compiler/bifrost/`);
`third_party/mesa-26.2.1/` vendors only `valhall/`, so these files are not in the
tree. No shim was built for this note (none on this host), hence the one source
without a same-source reference (`es100/int_div.frag`) is *inferred*, section 2.1.

## 1. The first refusal of each source on the tip

Method: the five `Err(MaliError::Arithmetic)` sites of `nir_fs.rs` and the two
`alu32_lanes` / mediump-arm catch-alls printed `file:line` and the front-end
`AluOp` while `the_corpus_verdicts_are_pinned` walked the corpus (a scratch
`eprintln!`, reverted; `git diff` empty). The walk stops at the first op, so each
source needs more than the op named (section 2 lists the rest, *inferred from the
source text* for ops past the first).

| Source | First refusal | Op | Path | What the source asks for there |
|---|---|---|---|---|
| `es100/int_div.frag` | `nir_fs.rs:2373` | `Idiv` | highp `alu32_lanes`, the power-of-two arm's `_ =>` | `int(v_color.r * 2000000.0) - 1000000` passes; `i / j` has a run-time divisor: the arm (:2366-2374) takes only a constant `2^k` |
| `es300/int_div.frag` | `nir_fs.rs:2142` | `Ineg` | highp `alu32_lanes`, last arm | `4294967295u - uint(..)`: the front end's `a - b` is `iadd(a, ineg(b))` |
| `es300/uops.frag` | `nir_fs.rs:2142` | `Udiv` | same | `u.x / u.y` |
| `es300/gcops.frag` | `nir_fs.rs:2142` | `Ineg` | same | `abs(i.x)`: builtin_call.rs:791-795 spells `abs(int)` as `imax(x, ineg(x))` |
| `es300/ifelse_uint.frag` | `nir_fs.rs:2142` | `Ult` | same | `u.x < u.y` (the `if` condition) |
| `es100/f2b.frag` | `nir_fs.rs:4216` | `Ffloor` | mediump `walk_nodes` catch-all `ir::Op::Alu(..)` | the scalar `floor(2.0 * v_color.r)` (the pair form `if w == 2` at :1792 does not cover a scalar) |
| `gltest/f1_fs.frag` | `nir_fs.rs:4216` | `Flog2` | same | `log2(16.0)` of constants, in the initialiser of `e`, a variable only a dead statement reads |

(`nir_fs.rs:2142` is the last arm of the single-lane `alu32` dispatch whose
neighbours are listed in section 2; `:4216` is `ir::Op::Alu(..) => return
Err(MaliError::Arithmetic)` of the mediump walk. `corpus-verdicts.txt` pins all
seven at the message "arithmetic other than +, -, * ...".)

The mediump path (`Fs::int_lane`, `nir_fs.rs:1439`) is separate from the highp
path (`alu32` / `alu32_lanes`). `int_lane` already holds the signed division
(`Fs::idiv`, :1575, case 79, byte-equal to `intloop_idiv`) but `int_op` (:8344)
routes only `Iadd Imul Idiv Ineg Ishl Ishr Ushr Iand Ior Ixor Ilt Ile Ult Ule Ieq
Ine B2i B2f I2b F2b Bnot Inot F2i I2f U2f` to it: `Imod Udiv Umod Imin Imax Umin
Umax F2u` are absent, and `int_lane`'s own arms cover only `F2i I2b Bnot Inot Ieq
Ine Ilt Ile Iadd I2f U2f Idiv` (everything else is `IntValue`). `F2b` therefore
meets `IntValue` there, and `Fne` / `F2b` in the highp path meet `CompareForm`
(`:2141`) / `Arithmetic` (`:2142`). The vertex walk (`nir.rs:2681-2815`) lowers
`Imod` by a power of two and returns `IntValue` for `Idiv Udiv Umod Ult Ule Imin
Imax Umin Umax Inot Iand Ixor ...`; none of the seven sources is a vertex shader
(`es300/int_div.vert` is a pass-through, verdict `ok`).

## 2. The op table

"Words" are Mesa's Valhall instructions. **M** = measured (a committed reference
draws the same source or the same op), **I** = inferred (Mesa source, no reference
draws it). "Ours" is the tip: `bir` = `bir.rs` `Op` + `pack.rs`; `emit` = the
`emit.rs` row table; `walk` = a lowering arm in `nir_fs.rs`.

| Op (GLSL) | Mesa words | M/I and where | Ours |
|---|---|---|---|
| `abs(int)` (`imax(x, ineg x)`) | `IABS.s32 r0, u3.w1` | M: `intuni_gcops` (23 words); rule nir_opt_algebraic.py:1086 `imax(a, ineg(a)) -> iabs` | bir `IabsS32`, emit `Alu::Iabs`; **walk missing** (`Ineg` refuses first) |
| `a - b`, `-a` (int) | `ISUB.s32 d, a, b`; `ineg` is `ISUB.s32 d, 0x0, a`; a constant minuend rides in the FAU/immediate (`ISUB.s32 r4, 0xFFFFFFFF, r4`) | M: `fsarith_int_div_es300` (`4294967295u - uint(..)`) | bir `IsubS32`, emit `Isub` `Ineg`; **walk missing in `alu32`** (present in `Fs::iop`) |
| `u ^ v` | `LSHIFT_XOR.i32 d, a, 0x0.b00, b` | M: `intuni_gcops`, `intuni_uops` (the 9-term xor chain) | bir `LshiftXorI32`, emit `Ixor` (intbool_199); **walk missing** |
| `x & 255u` then `float(..)` | `U8_TO_F32 d, x.b0` / `.b1` for `(x >> 8) & 255u` | M: `intuni_gcops`, `intuni_uops` | bir `U8ToF32`, emit `U8f32`; walk folds only `I2f` (:2100-2113), **`U2f` of the same pattern missing** |
| `u >> k` (`ushr`) | const `k`: `MOV.i32 r1, 0x3020100` (when the other source is a uniform) then `RSHIFT_OR.i32 d, u2.w0, r1.b33, 0x0`; k=8 is byte 0 of `0xB0A0908` | M: `intuni_uops`, `intuni_ifelse_uint` (`RSHIFT_OR.i32 r0, r0, 0x1000000.b33, 0x0`); `bifrost_compile.c:3050-3052` | bir `RshiftOrI32`, emit `Ushr`; **walk missing** (only the internal `Ushr` of the pow2 divide) |
| `i >> k` (`ishr`) | `ARSHIFT_OR.i32 d, a, k.bNN, 0x0`; a uniform count is `MOV`ed to a register first | M: `intuni_ishr`, `intbool_199_vsc`; `bifrost_compile.c:3054-3060` | present end to end (nir_fs.rs `AluOp::Ishr` :2121) |
| `u << k` (`ishl`) | `LSHIFT_OR.i32 d, a, 0xB0A0908.b00, 0x0` (k=8), `0x7060504.b11` (k=5), `0xB0A0908.b33` (k=11): the count is a byte of the table word `va_lookup_constant` finds; **k = 16: `MKVEC.v2i16 d, 0x0.h0, a.h0`** | M: `intuni_uops` (`sh << 16u` is the MKVEC; the cause of that rewrite was not found in `nir_opt_algebraic.py`, line 2215's `ior(ishl(a,16), ..)` needs an `ior`; the form is measured, the rule is not) | bir `LshiftOrI32`, emit `Ishl`, `fau.rs` `lut8` any byte value; **walk missing**; `MKVEC.v2i16` exists (`MkvecV2i16`) |
| `~u` | `LSHIFT_OR.i32 d, 0x0, 0x0.b00, u2.w0.not` | M: `intuni_uops`; `bifrost_compile.c:3387-3389` | emit `Alu::Inot` (a mask, "one left for selection has no reference"); **the `.not` source modifier is not in `bir::Src`** (fields `neg`, `abs`, `discard`) |
| `min(i, j)` / `max(i, j)` signed | `CSEL.s32.lt d, a, b, a, b` / `.gt` | M: `intuni_uops`; `bifrost_compile.c:3061-3071` | bir `CselS32{cmpf}`, emit `Imin` `Imax`; **walk missing** |
| `min(u, v)` / `max(u, v)` unsigned | `CSEL.u32.lt d, a, b, a, b` / `.gt` | M: `intuni_uops` | **bir has `CselU32` without `cmpf`** (select.rs/emit use it as `.eq`; `pack.rs:310` fills the `eq` form only); needs `CselU32{cmpf}` like `CselS32` |
| `u < v`, `u <= v` | `ICMP_OR.u32.lt.m1 d, a, b, 0x0`; `<=` is `uge_pan` with swapped sources: `ICMP_OR.u32.ge.m1 d, v, u, 0x0`; used as a select: `CSEL.u32.eq d, mask, 0x0, 0x0, u5.w1` | M: `intuni_uops` (`(u.x < u.y ? 0.25 : 0.0) + (u.x <= u.y ? 0.5 : 0.0)`), `intuni_ifelse_uint` | bir `IcmpOrU32`, emit `Ucmp(cmpf)`; **walk missing** (`AluOp::Ult` / `Ule` only in `fold_int`) |
| `float(uint)` | `U32_TO_F32 d, a` | M: `intuni_uops`, `intuni_ifelse_uint` | bir `U32ToF32`, emit `U2f`; highp walk missing (`int_lane` has it) |
| `uint(float)` | `F32_TO_U32.rtz d, a` | M: `intuni_uops`; `bifrost_compile.c:3137-3150` | bir `F32ToU32`, emit `F2u`; highp `F2u` arm present (:2095) |
| `int(float)` | `F32_TO_S32.rtz`; mediump source `F16_TO_S32.rtz` | M: `intbool_199_vsc`, `intloop_idiv` | present (`F2i`, `F2i32`) |
| `u / v` (variable) | `U32_TO_F32 r, v; FRCP.f32; FMA.f32 r, r, 0x4f7ffffe(FAU), -0; F32_TO_U32.rtz` (the estimate) `; IMUL.i32 t, v, e; ISUB.s32 t, 0x0, t; umul_high(e, t)` (4 `U16_TO_U32` halves + 4 `IMUL.i32` + 3 `IADD.s32` + 2 more `U16_TO_U32`) `; IADD.s32 e, e, hi; umul_high(a, e)` (same shape) `; IMUL.i32 qd, q, v; ISUB.s32 rem, a, qd; ICMP_OR.u32.ge.m1 c, rem, v, 0x0; CSEL.u32.eq t, c, 0x0, 0x0, u5.w0` (the FAU word 1) `; IADD.s32 q, q, t; ISUB.s32 rs, rem, v; CSEL.u32.eq rem, c, 0x0, rem, rs;` and the second round of the last five | M: `intuni_uops` (the `d` / `m` pair shares everything up to the refinement, 93 words in all); Mesa: `emit_udiv` nir_lower_idiv.c:32-66, `bifrost_nir.c:1078-1081` (`allow_fp16` only for the small path) | the same nodes in `Fs::idiv` (:1575) for the signed case, `Fs::umul_high` (:1546); **no `Udiv`/`Umod` caller; not reachable from the highp `alu32`** |
| `u % v` (variable) | no new sequence: `umod = n - (n/v)*v`, after CSE it is the quotient's `IMUL.i32 r1, r0, v; ISUB.s32 r1, u, r1` and the refinement updates both | M: `intuni_uops` (`m`); glsl_to_nir.cpp:2431-2434: **GLSL `%` is `umod` even for `int`** (`nir_umod`, both signs) | none |
| `i % j` signed variable | same `umod` sequence **with no `IABS`, no sign `LSHIFT_XOR`**: `U32_TO_F32 r5, r1` straight from `IADD_IMM.i32 r1, r1, #0x1` | M: `fsarith_int_div_es300` (180 words, `i % j`) | none (the vertex arm handles `Imod` by `2^k` only, nir.rs:2681) |
| `i / j` signed variable (`idiv`) | `IABS.s32` x2, `U32_TO_F32`, `FRCP.f32`, `FMA.f32`, `F32_TO_U32.rtz`, `IMUL`, `ISUB 0-`, 2 umul_high, `IMUL`, `ISUB`, `ICMP_OR.u32.ge.m1`+`CSEL`+`IADD`+`ISUB`+`CSEL`, `ICMP_OR.u32.ge.i1` (the second `b2i` folded into the compare), `LSHIFT_XOR.i32 sign, n, 0x0.b00, d`, `ICMP_OR.s32.lt.m1`, `ISUB 0-q`, `CSEL.u32.eq` | M: `intloop_idiv` (mediump twin of `es100/int_div`, 181 words; `n / d` of ivec2 lanes and `100 / (..)`); source nir_lower_idiv.c:69-91 | `Fs::idiv` (mediump only, case 79 byte-equal); the highp arm refuses (:2373) |
| `x / 3`, `x % 7u`, `u / 65537u`, `u / 2147483647u` (constant divisor) | `nir_opt_idiv_const` (`bifrost_nir.c:240, 1072`): `umul_high(n, magic)` with `IADD.u32.saturate` of the increment, `RSHIFT_OR` by the shifts, `IMUL.i32 q, #d; ISUB` for `%`; the magic's halves ride in the FAU (`IMUL.i32 r2, r1, u5.w0`, `u5.w1`) | M (unsigned): `intuni_ifelse_uint` (55 words; first word `IADD.u32.saturate.discard r0, u2.w0, 0x1000000.b3` is the `+1` increment of `/ 3u`, `% 7u` ends `RSHIFT_OR.i32; IMUL.i32 r0, r0, 0x7060504.b3; ISUB.s32 r0, u2.w0, r0`); `fsarith_int_div_es300` (`top / 65537u` etc.). **I** (signed, `k / 3` of `es100/int_div`): `build_idiv` nir_opt_idiv_const.c:80-113 with `imul_high` | the pow2 signed divide (:2366-2395, `fbload_isamp`); **no magic-number divide, no `IADD.u32.saturate` in bir/pack/emit** (`bir.rs` has `IaddS32` only, `pack.rs:339` hard-codes `saturate 0`) |
| `bool(float)` mediump pair / scalar | `FCMP_OR.v2f16.ne.m1 d, a, 0x0, 0x0` (bvec2 over two lanes packed by `MKVEC.v2i16`); the scalar `bool(z) ? 1.0 : 0.0` is `FCMP_OR.v2f16.ne.f1 d, z.h00, 0x0, 0x0` (the `b2f` folded into the compare); a mask used as a select: `S16_TO_S32 r, m.h0; CSEL.u32.eq r, r, 0x0, u2.w0, r3` (r3 = `MOV.i32 0x3F800000`) | M: `fsarith_f2b` (23 words); `bifrost_compile.c:3268-3275` `fneu_pan` -> `BI_CMPF_NE`; glsl_to_nir.cpp:2186 | bir `FcmpOrV2f16`, `Cmpf::Ne` exists, emit `Fcmp(cmpf)` 16-bit (`Ne` not exercised); `S16ToS32` present; **walk: `F2b` -> `IntValue` (:int_lane), `Fne` -> `CompareForm` (:2141)** |
| `floor` of one fp16 lane | `FADD.v2f16 r1, r1.h00, r1.h00` (`2.0 *`) then `FROUND.v2f16.rtn r1, r1.h00`: the scalar uses the pair op with `.h00` | M: `fsarith_f2b` | emit has `Ffloor`/16/2 only; **`Ffloor`/16/1 missing** (`Fceil`/16/1 and `Fmin`/16/1 exist with `lone`) |
| `bool(-z * 0.0)` | folded to the constant alpha (`ATEST ..., 0x3F800000`): no ALU | M: `fsarith_f2b` | walk needs the fold (`z * 0.0` is `+-0.0`; `-0.0 != 0.0` is false) |
| `bool(float)` highp (fp32) | `FCMP_OR.f32.ne.m1 d, a, 0x0, 0x0` | **I**: same `fneu_pan` path, fp32 size (no source draws it) | emit `Fcmp(cmpf)` 32-bit generic; walk refuses |
| `int()`/`bool()` of constants, `log2`/`atan`/`inversesqrt` of constants | no ALU: the whole program is `MOV r0, 0x3F800000; MOV r1, u2.w0; MOV r2, u2.w1; MOV r3, 0x0; TEX_SINGLE .. [r2:r3]` (9 words) and a dead `if (k) (vec3(..));` / `else d * .. < ..;` | M: `intloop_f1_fs` (the NIR is not in the `.expect.json`; that `(u2.w0, u2.w1)` are the folded coordinate `inversesqrt(vec2(1, 4)) = (1, 0.5)` is **I**) | `fold.rs` `fold_float` (shared, no libm) folds `Flog2` of a power of two, `Frsq` of a square (`sqrt_exact`), `F2i`, `F2b`, `I2f`; **the Mali walk does not call it** (grep: only `powervr` and `v3d`); `atan` is not exact there |

Mesa references used, with the counts of their fragment programs: `intuni_uops` 93
words, `intuni_ifelse_uint` 55, `intuni_gcops` 23, `fsarith_f2b` 23,
`fsarith_int_div_es300` 180, `intloop_idiv` 181, `intloop_f1_fs` 9. All have
their `.frag`/`.vert` in `tools/mali/testdata/`; the first four, the fifth and the
last are byte-equal copies of the corpus sources (`diff` empty); `intloop_idiv.frag`
is **not** `es100/int_div.frag` (it is gl_test case 79's `idiv.frag`, mediump).

### 2.1 What is not measured

- **`es100/int_div.frag`** (highp `idiv`, `i / j`, `i / (-j)`, `k / 3`, `i - q1 * j`)
  has no reference of its own. Its `idiv` by a variable follows `intloop_idiv`
  word for word *except* where the operands come from: highp `int(v_color.r *
  2000000.0)` is `F32_TO_S32.rtz` of an fp32 product, not `F16_TO_S32.rtz` of an
  fp16 pair (integer ops are 32-bit whatever the precision, `Fs::int_lane`'s
  comment, :1431-1435). `i / (-j)` is `idiv(i, ineg(j))`: whether Mesa's
  algebraic pass folds the `ineg` into the `iabs` / sign steps is not known;
  `k / 3` is `build_idiv` with a signed magic (`imul_high` lowering:
  the `umul_high` lowering is `nir_lower_alu.c:120`; the signed form is not
  measured for the Valhall). **Step 0 (section 3)
  draws it** on a Mesa 26.2.1 shim before the highp divide is written.
- **Highp `bool(float)`, highp `float`-valued `!=`** and the `.not` modifier of
  `inot` outside `intuni_uops`.
- **`gcops`, `uops`, `ifelse_uint`, `f2b`, `int_div` ES 3.00 are all measured**, but
  only as whole programs: no test compares them (`grep intuni_uops src tests`
  finds nothing; only `intloop_idiv` and case 79 are in `INTBOOL_POPULATION`,
  tests.rs:5213, :5270).
- **`gcops`' `dot(v_t.yzw, vec3(0.5))`** is measured as `FADD`, `FADD`, `FMA 0.5`
  (the equal-lane constant dot, `0x38000000.h1`); whether our dot lowering
  reproduces it is not run: the walk stops at `Ineg`, after the dots passed. No
  other op past each source's first refusal was run.

## 3. Order of implementation items

All items are `nir_fs.rs` (and `bir.rs` / `pack.rs` / `emit.rs` / `fau.rs`), the
files several plan #336 items hold at this moment (`fs-fold`, `fs-transc`,
`fs-trunc`, `fs-fragcoord16`, `fs-indexlocal`, `isa-deriv`); these items start
after those merge. Each is one session; the host test is the same shape as
`case_79_idiv_is_byte_equal_to_mesa_26_2_1` (tests.rs:5330: a row of
`INTBOOL_POPULATION` + `check_intbool`), plus `corpus-verdicts.sh --write` in the
same commit as the verdict moves.

| # | Item | Files | Host tests | Moves |
|---|---|---|---|---|
| 0 | **ref-intdiv** (testdata only): draw `es100/int_div.frag` highp on the Mesa 26.2.1 shim (`tools/mali/drm_shim_ref.sh run <shim> V F target/<name>`, `pan_ref_diff.py extract --draw`) as `tools/mali/testdata/fsarith_int_div_es100.*`, and record the `i / (-j)` and `k / 3` words in `mali-int-and-size.md` | `tools/mali/testdata/fsarith_int_div_es100.*` | none (no test reads a file that is not in the population) | turns the 2.1 *inferred* rows into *measured* |
| 1 | **int-ops** (the single-instruction group, no sequences): highp `alu32` / `alu32_lanes` arms for `Ineg Isub Imul Ior Ixor Ishl Ushr Inot Imin Imax Umin Umax Ult Ule U2f` and the folds `imax(x, ineg x)` -> `Iabs`, `&255u` + `U2f` -> `U8f32(k)`; route `Imin..Umax` and `F2u` through `int_op` / `int_lane` for mediump; **new bir `CselU32{cmpf}`** (+ `pack.rs:310`, `bir.rs` info tables, `emit.rs` rows `Umin`/`Umax`); the `.not` source modifier (`Src.not`) for `inot` | `nir_fs.rs` (`alu32`, `int_lane`, `int_op`, :1439, :2142, :8344), `nir.rs` (`Alu::Umin/Umax`), `bir.rs`, `pack.rs`, `emit.rs` | `gcops` byte-equal to `intuni_gcops` (23 words); the `emit.rs` row table entries for the new rows (`emit.rs:40-188`), `pack` encoding of `CSEL.u32.lt/gt` and `LSHIFT_OR.i32 .., x.not` against `va_disasm_ref.sh` text | `es300/gcops.frag` (if the dot of `vec3(0.5)` passes: 2.1); the shifts and the min/max of `uops` |
| 2 | **int-f2b**: scalar fp16 `Ffloor` as the pair op with `.h00`, `F2b`/`Fne` -> `Fcmp(Cmpf::Ne)` (16-bit M1, the `b2f` fold to `.f1`, mask to `S16_TO_S32` + `CSEL.u32.eq`), the `z * 0.0` fold; fp32 `Fcmp(Ne)` (inferred) | `nir_fs.rs` (:1792 arm, :2141, `int_lane`), `emit.rs` (`Ffloor`/16/1), `opt.rs` if the `.f1` fold needs the 16-bit compare | `es100/f2b.frag` byte-equal to `fsarith_f2b` (23 words) | `es100/f2b.frag` |
| 3 | **int-udiv**: highp `Udiv`/`Umod`/`Imod` by a variable (`Fs::idiv`'s `umul_high` / refinement shared: `emit_udiv(modulo)` merges with the quotient by CSE, so one routine builds `(q, rem)`); `Imod` is `umod` of the bits (glsl_to_nir.cpp:2431), no `IABS`/sign; the highp divide arm calls `Fs::idiv` (the mediump `int_lane` and the highp walk use the same `int32`/`op32` nodes) | `nir_fs.rs` (:1575 `idiv`, :2366 arm, `int_op`, `int_lane`) | `uops` byte-equal to `intuni_uops` (93 words, with items 1-2 done); `es300/int_div.frag` equals `fsarith_int_div_es300` once item 4 is in; `case_79` and `intloop_idiv` unchanged (no regression) | `es300/uops.frag`; `es100/int_div.frag` once step 0 has its reference |
| 4 | **int-divconst**: `nir_opt_idiv_const`'s `build_udiv` / `build_umod` / `build_idiv` / `build_irem` (nir_opt_idiv_const.c:28-130) and Mesa's `util_compute_fast_udiv_info` / `_sdiv_info` (src/util/fast_idiv_by_const.c: 64-bit integer arithmetic only, no libm: portable to the no_std crate), `umul_high` with a constant operand folded to the two products (`Fs::umul_high` already folds the halves of a constant, :1546), **new `IADD.u32.saturate`** (bir `IaddU32Saturate`, `pack.rs:339` sets the `saturate` field; emit row `Alu::IaddSat`), the pow2 unsigned rules (`iand`/`ushr`) | `nir_fs.rs`, `bir.rs`, `pack.rs`, `emit.rs` (a new module file for the constant tables is cleaner: `src/shaderc/online/mali/idiv.rs`) | `ifelse_uint` byte-equal to `intuni_ifelse_uint` (55 words, needs item 1's `Ult` and `U2f`); `es300/int_div.frag` byte-equal to `fsarith_int_div_es300` (180 words); a unit test of the magic numbers against the words (`0x7060504.b3` = 7, the FAU halves) | `es300/ifelse_uint.frag`, `es300/int_div.frag`, `es100/int_div.frag`'s `k / 3` |
| 5 | **int-f1fold**: call `fold::fold_float` in the Mali walk for constant operands (the walk's integer folding is `nir::fold_int`, nir.rs:771), skip a dead op (an IR value with no use), and decide `atan(-1.0) < 4.0` without libm (a bound: |atan| < pi/2, not an evaluation) | `nir_fs.rs` (walk entry, :4216 catch-all), `fold.rs` only if a new exact case is added (it is shared with v3d and powervr: keep it minimal) | `gltest/f1_fs.frag` byte-equal to `intloop_f1_fs` (9 words) | `gltest/f1_fs.frag` |

Dependencies: 1 before 2 and 3; 3 and 4 share `idiv`; 0 can run now (it is a
draw), alone, on a host that can build the shim (`drm_shim_ref.sh build`,
~30-60 min cold).

## 4. Corpus and dEQP cases each item moves

Corpus sources are the seven above. None of them is a case of the 256-case
`panthor-gltest --sweep` (none of their names occurs under `tools/cts`; the
integer case the sweep has is 79, `idiv.frag`, already done): **no gl_test case
is known to move**, so the plan's `c3` count
(PASS > 147) does not change by this note's items. What they move:

- `es300/gcops.frag`, `es100/f2b.frag`: items 1, 2.
- `es300/uops.frag`: items 1, 2 (none), 3 (variable divide).
- `es300/ifelse_uint.frag`: items 1 (`Ult`, `U2f`), 4.
- `es300/int_div.frag`, `es100/int_div.frag`: items 3, 4 and step 0.
- `gltest/f1_fs.frag`: item 5.

The dEQP-GLES2 effect is not measured here: the shader texts of the dEQP cases
are not in the tree (`docs/notes/mali-gles2-forecast.md` section 2 counts 3963
`Arithmetic` and 1798 `IntValue` refusals over all causes, not by op). dEQP's
integer groups (`shaders.integer_ops` is ES 3.0; the ES 2.0 `swizzles` and
`conversions` groups are the `bool(float)` / `int(float)` shape, `f2b.frag`'s
comment says so) are what items 2 and 3 touch. A scan of the dEQP shader
sources for `/ % >> ~` on ints and `bool(` of floats would turn this into a
count; that is a read-only task for #327's tally, not done here.

## 5. Risks

- `nir_fs.rs` is the contended file; items 1, 3, 4 each add arms to the same
  three functions. Land them in the order above, one at a time.
- Item 4's constants: Mesa's `util_compute_fast_udiv_info` must be reproduced
  bit for bit; the `pre_shift` / `increment` / `post_shift` choice decides
  whether `IADD.u32.saturate` and a `RSHIFT_OR` appear. The measured words of
  `% 7u` and `/ 3u` and the three constants of `es300/int_div` (65537,
  2147483647, 33554431) are the check.
- Mediump versus highp: `int_lane` writes `Fs::idiv` with `op32`/`int32` nodes
  of "a mediump shader"; before sharing it with the highp walk, check that
  `op32`'s widening / `mark_mixed` side effects (nir_fs.rs:1299-1323) do not
  fire on integer operands of a highp shader.
