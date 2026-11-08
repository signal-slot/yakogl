<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: case 199 es3trunc, `trunc()` and the integer shifts of a fragment shader

**Design, implementation in fs-trunc** (plan #336, issue #33). Nothing here
changes the compiler. The Mesa 26.2.1 source is `MESA_PAN_TREE`
(`$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`); line numbers are that tree's.
What is *read from code* and what is *measured* is marked: the first refused
operation of each step was found by reading `nir_fs.rs`, not by running the
compiler (the note was written without a build).

## 1. The programs case 199 compiles

`driver/tools/pvrlab/gl_test.c` `case_es3trunc` (`:18407-18452`) links two
programs and draws each in half of the window, with `u_x = 2.75`,
`u_q = -6`, `u_a = 200` (`u_b` is unused):

| program | vertex shader | fragment shader | today (`tools/mali/corpus-verdicts.txt`) |
|---|---|---|---|
| top (`fs`) | `tools/shaders/gltest/es3trunc_vs.vert`: `v = vec4(u_x, u_q, u_a, u_b)` | `es3trunc_fs.frag`: the arithmetic below | `es3trunc_fs.frag` **refused**, `MaliError::Arithmetic` (`mod.rs:375`) |
| bottom (`vs`) | `es3trunc_vsc.vert`: the same arithmetic in the vertex shader | `es3trunc_vary_fs.frag`: `o_color = v` | all `ok` |

`es3trunc_fs.frag` (highp float and int, `in vec4 v`):

```
int i = int(v.y);   uint u = uint(v.z);
o_color = vec4((trunc(v.x) + 2.0) * 0.125, (trunc(-v.x) + 4.0) * 0.125,
               float((i >> 1) - (i << 3)) / 64.0, float(u >> 2u) / 64.0);
```

Expected pixel `fma_rgba(0.5, 0.25, 45/64, 50/64)` (gl_test.c `:18416-18421`).
Only the top program is open; the bottom one is pinned byte-equal by
`tests.rs` `case_199_vertex_integers_are_byte_equal_to_mesa_26_2_1`
(`intbool_199_vsc.*`, mali-int-and-size.md "Since plan #185 item
vs-int-convert") and `varyconst_199_es3trunc.*` (tests.rs `:4862`;
mali-varying-constants.md "Cases 117, 181, 191, 199", whose fragment is the
5-instruction flat `.v4` load).

## 2. The reference

`tools/mali/testdata/fsarith_199_es3trunc.{vert,frag,expect.json}` (new
draw of `es3trunc_vs.vert` + `es3trunc_fs.frag`, mali-alu-ops.md "The
references made here", `:271-280`; drawn **without** `--uniform-values`, so
every uniform word in the FAU is 0). `.vert` and `.frag` are byte copies of
the two gltest sources. Its three regions:

* `vs.position`: the same 24 words as every program of this vertex shader
  (equal to `varyconst_199_es3trunc.expect.json`'s).
* `vs.varying` (24 words): **Mesa moved the integer work of the fragment
  shader into the vertex stage.** The varying program computes lanes y and
  z of `v` and stores three words (x, y', z'; w is dead):
* `fs.fragment` (13 words): a flat `.v3` load, then only `trunc`.

NIR ops reached, and where each ends up in Mesa's program:

| GLSL | NIR op | stage in the reference | instruction |
|---|---|---|---|
| `trunc(v.x)`, `trunc(-v.x)` | `ftrunc`, `fneg` | fragment | `FROUND.f32.rtz`, the `fneg` its `.neg` source modifier |
| `+ 2.0`, `+ 4.0` | `fadd` of a constant | fragment | `FADD_IMM.f32 #0x40000000`, `#0x40800000` |
| `* 0.125` | `fmul` | fragment | `FMA.f32 r, r, u2.w0, 0x0.neg` (`u2.w0` = 0.125, FAU word 4 = `3e000000`) |
| `int(v.y)`, `uint(v.z)` | `f2i32`, `f2u32` | **vertex** | `F32_TO_S32.rtz`, `F32_TO_U32.rtz` |
| `i << 3` | `ishl` | vertex | `LSHIFT_OR.i32 r1, r0, 0x3020100.b33, 0x0` (count 3 is byte 3 of the byte-select word) |
| `i >> 1` | `ishr` | vertex | `ARSHIFT_OR.i32 r0, r0^, 0x1000000.b33, 0x0` |
| `(i>>1) - (i<<3)` | `iadd(a, ineg(b))` is `isub` (nir_opt_algebraic) | vertex | `ISUB.s32 r0, r0^, r1^` |
| `float(int)` | `i2f32` | vertex | `S32_TO_F32` |
| `u >> 2u` | `ushr` | vertex | `RSHIFT_OR.i32 r2, r2^, 0x3020100.b22, 0x0` |
| `float(uint)` | `u2f32` | vertex | `U32_TO_F32` |
| `/ 64.0` | `fmul` by 1/64 (the front end's `a * rcp(64.0)`, folded) | vertex | `FMA.f32 r, r, u7.w0, 0x0.neg`, `u7.w0` = `3c800000` (FAU word 14) |
| the varying store | `store_output` | vertex | the canonicalising `FMA.f32 r, r, 0x0, r` of every stored value, then `STORE.i32.estream` |

(The `FMA.f32 r, r, 0x0, r` after a stored product and the
`FMA.f32 r1, u5.w1, 0x0, u5.w1` copy of the plain x lane are the existing
vertex-output shapes of `intbool_199_vsc` and `varyconst_199_es3trunc`.)

### 2.1 Why the integer work moves and the `trunc` does not

All four lanes of `v` are computed from uniforms alone, so Mesa's link
(`gl_nir_lower_optimize_varyings`, `nir_opt_varyings`) calls them
*convergent* (`convergent32_mask`, nir_opt_varyings.c `:4620-4654`: an
input load of a convergent slot gets `FLAG_INTERP_CONVERGENT`, and the
varying is then loaded `.src_flat32`, mali-varying-constants.md "a
uniform-only lane is a flat varying"). `update_movable_flags` (`:3820-3905`)
lets an ALU instruction over convergent sources move whatever the op is:
the `can_move_alu_across_interp` test (`:3888`) is asked only when
`alu_interp > FLAG_INTERP_FLAT`. `backward_inter_shader_code_motion`
(`:4690-4770`) then takes, per input load, the farthest movable
post-dominator of its uses (`try_move_postdominator`, `:4089-4494`) and
replaces it by a new input in the load's slot:

* lane y: `f2i, ishr, ishl, isub, i2f32, fmul` is one use chain; the
  post-dominator is the final `fmul`, which moves. Lane z likewise
  (`f2u, ushr, u2f32, fmul`). Their result is `float`, so the "convergent
  results that are not interpolatable should not be moved" test
  (`:4176-4187`) passes; the intermediate integers are never post-dominators
  on their own.
* lane x: its two uses (`ftrunc(v.x)` and `fneg(v.x)`) meet only in the
  colour's `vec4`, and a vector op is `FLAG_UNMOVABLE` (`:3838-3843`), so no
  post-dominator is movable and `trunc` stays in the fragment shader.
  (This is inference from the source and the reference; it explains why
  `FROUND` is in the fragment program and not the integer work.)

The new inputs reuse slots y and z, so the fragment shader reads lanes 0, 1,
2: `LD_VAR_BUF_IMM.f32.slot0.v3.src_flat32.center.store.wait0 @r2:r3:r4`.

## 3. The fragment program and the code that makes each word

`fs.fragment` (13 words, 32 registers, preload `0x3000000000000000`, FAU
`[0, 0, 0, 0, 3e000000, 0]`):

| # | instruction | produced by |
|---|---|---|
| 0 | `LD_VAR_BUF_IMM.f32.slot0.v3.src_flat32.center.store.wait0 @r2:r3:r4, 0x0, index:0x0` | `nir_fs.rs` varying layout `:5367-5440` (`flat` from `f.prec.consts.flat`, `read` mask `0b0111` after the motion) and `:8593-8597` (`LdVarBufImmF32 { flat }`, zero barycentric); the reads reach the vertex stage through `mod.rs` `fragment_reads` (`mod.rs:1672`) |
| 1 | `FROUND.f32.rtz.discard r0, r2` | `Alu::Ftrunc` -> `Op::FroundF32 { Rtz }`, **exists** (`emit.rs:167`, `:249`); the graph node is the open part (§4.1) |
| 2 | `FADD_IMM.f32 r0, r0^, #0x40000000` | `opt.rs` `fuse_add_imm` (exists) |
| 3 | `FMA.f32 r0, r0^, u2.w0, 0x0.neg` | `Alu::Fmul` (exists), 0.125 in the FAU by `fau.rs` |
| 4 | `FROUND.f32.rtz r1, r2^.neg` | the same, `Alu::Fneg` -> source modifier (`opt.rs` `mod_prop_forward`, exists) |
| 5-6 | `FADD_IMM` 4.0, `FMA` by `u2.w0` | as 2-3 |
| 7 | `ATEST.wait0 @r60, r60^, r4, atest_datum.w0` | the existing fp32 colour path (`varyconst_199`: `ATEST r3`); here the alpha is the moved z lane `r4` |
| 8-9 | `MOV.i32 r2, r3^` / `MOV.i32.wait r3, r4^` | the BLEND staging copy of lanes already in registers, existing |
| 10-12 | `BLEND.slot1.v4.f32.end`, `IADD_IMM`, `BRANCHZI` | existing epilogue |

`vs.varying` (24 words) in order: `F32_TO_S32.rtz r0, u6.w0` (`u_q`),
`LSHIFT_OR` (`i << 3`) **before** `ARSHIFT_OR` (`i >> 1`), `ISUB.s32`,
`LEA_BUF_IMM`, the x copy `FMA.f32 r1, u5.w1, 0x0, u5.w1` and its
`STORE.i32.estream` (offset 0), `S32_TO_F32`, `IADD_IMM #0x4` + `SHADDX`
offsets, the `FMA` by `u7.w0` and its canonicalising `FMA`, the store at
offset 4, then `F32_TO_U32.rtz r2, u6.w1`, `RSHIFT_OR`, `U32_TO_F32`, `FMA`,
`FMA`, store at offset 8. All of these opcodes have rows in `emit.rs`
(`Ishl :85/:284`, `Ushr :115/:307`, `Isub :149/:338`, `U2f :151/:340`,
`Ftrunc :167`) and the vertex walk lowers every IR op involved
(`nir.rs` `:2382-2445`, "vs-int-convert"). What is missing is the *producer*
of this program: the moved expressions.

## 4. What fs-trunc has to add (the code places)

### 4.1 `nir_fs.rs`, the fragment graph (`alu32`, `:2152`; the integer arms `:1395-1505`)

Read from the arms (not run): the first operation the highp walk refuses is
`AluOp::Ftrunc`, which falls to `_ => Err(MaliError::Arithmetic)` (`:2142`)
(`int(v.y)`/`uint(v.z)` pass: `F2i`/`F2u`, `:2089-2101`; `Ishr` has an arm
at `:2121`).
In program order the next ones are `Ishl`, `Ineg`/the `isub` fusion, `Ushr`
and `U2f` (`alu32` has no arm for them; `int_lane`'s `U2f` arm `:1498` is
case 89's flat-uint one). Add, with `op32` (`:1299`) and the node CSE:

* `AluOp::Ftrunc => op32(Alu::Ftrunc, [x, u, u])` (`Alu::Ftrunc` exists,
  `nir.rs:124`). `ftrunc` of `fneg` keeps the `fneg` node (the modifier is
  `opt.rs`'s).
* `AluOp::Ishl`, `AluOp::Ushr` (with `Ishr`'s existing arm as the model;
  a constant count is the byte lane `va_lower_constants` gives, `emit.rs:306`
  and `:321`), `AluOp::Ineg`, and **`Iadd(a, Ineg(b))` -> `Alu::Isub(a, b)`**
  exactly as `nir.rs:2440` does (`nir_opt_algebraic`'s `iadd(a, ineg(b))` is
  `isub(a, b)`; the `ineg` is left to `dce`), and `AluOp::U2f`.
* `float((i>>1)-(i<<3))` must stay `I2f` of the difference (the
  `i2f32(f2i32(a)) -> ftrunc(a)` rule, nir_opt_algebraic.py `:1250`, is
  already in the `I2f` arm `:2102-2118` and does not apply: the operand is
  an `isub`).
* `/ 64.0` is the front end's `a * rcp(64.0)`; the constant `0x3c800000` is
  exact (checked: FAU word 14 of the reference).

### 4.2 `motion.rs`, the link move

1. **`flag()` (`motion.rs:350`)**: an input lane is `Flag::Flat` only when
   the varying is declared `flat`. Case 199's `v` is smooth-declared and
   Mesa made it convergent. Give a `Kind::Input` lane `Flag::Convergent`
   when `f.prec.consts.flat(loc, comp)` (the link's uniform-only set, the
   same test `nir_fs.rs:5410` uses for `.src_flat32`). A node over
   convergent sources is `Flag::Convergent`, so **every** op moves
   (`:3888`), not only the `OPS` members. Keep `OPS.contains(&op)` as the
   guard of what `encode` can number.
2. **`OPS` (`motion.rs:113`)**: append (the index is the byte of every
   record key, "an operation is only ever appended"): `F2i`, `F2u`, `I2f`,
   `Ishl`, `Ishr`, `Ushr`, `Isub`. (`U2f` and `Fmul` are there; `Ineg` only
   if the fusion above is not applied before the move.)
3. **`MAX_NODES` (`motion.rs:72`) is 8; lane y needs 10** distinct nodes:
   the lane, `F2i`, the constants 1 and 3, `Ishr`, `Ishl`, `Isub`, `I2f`,
   `1/64`, `Fmul`. Lane z needs 7. Raise it to 12. `MAX_BYTES` follows
   (`MAX_MOVED * (3 + MAX_NODES * NODE_BYTES)` = 300) and sizes
   `ProgramReads::moved` (`mod.rs:1646`) and the key of the record; the
   `motion` tests at `tests.rs:4288/4386/4427` use the constant. Two moved
   expressions (y and z) are within `MAX_MOVED = 4`.
4. **The sink guard (`motion.rs:535`, `d.up(pd) == SINK && f.prec.highp`)
   contradicts this reference.** It was pinned by `s4_fneg` (a highp
   `-v.x` stored directly stays in the fragment shader) and by reading
   Mesa's text no rule gives it. Here the moved `fmul`'s only user is the
   colour's `vec4`, in a highp shader, and Mesa moves it. The two references
   differ in the lane's class: `s4_fneg` is *interpolated* (a `fneg`), 199
   is *convergent*. Narrow the guard to lanes whose flag is
   `Flag::Interpolated` (the only family it was measured on); 89 (flat,
   mediump) is unaffected. **Open point:** the reason for the `s4_fneg`
   result is not in `nir_opt_varyings.c`'s text as read; fs-trunc should
   draw one discriminating pair (a highp `-v.x` of a uniform-only varying,
   and a highp `v.x * 2.0` of an attribute-fed one) on the shim before
   widening the guard further.
5. `apply()` (`motion.rs:185`) already runs `vs.alu(op, ..)` for each node
   in dependence order and stores the result in the lane; the vertex `Nir`
   has all of these ops (`nir.rs:1900-1917`). The **emission order**
   matters: the reference runs `ishl` before `ishr`
   (`LSHIFT_OR` then `ARSHIFT_OR`), while `expression()` visits `src[0]`
   first (`ishr`). `intbool_199_vsc` pins the same order for the in-vertex
   form, so look at how `nir.rs` orders the `isub` operands (`:2440`, the
   `ineg` operand is the second source `b`) before changing `expression()`;
   the byte-equal test decides.
6. `link.rs`: nothing is expected. The set of uniform-only lanes
   (`VaryingConsts::flat`, `link.rs:168`) is computed from the vertex
   shader before the move; lanes y and z stay uniform-only after it, so the
   fragment layout stays `flat` (no `VaryingNotFromAttribute`, `:5427`).

### 4.3 `select.rs` / `emit.rs`

No new row: every opcode is in `emit.rs`. `select.rs` is only reached for
the ordering of the `FROUND`/`FADD_IMM`/`FMA` and the `ATEST` operand
(`r4`); expect the register numbers and `.discard`/`.wait` flags
(`FROUND.f32.rtz.discard r0, r2`, `FMA.f32.wait0126 r1, ...`) to come out
of the existing allocator (`ra.rs`/`sched.rs`) as they do for
`varyconst_199`; the byte test is the check.

## 5. Tests to add (fs-trunc)

In `src/shaderc/online/mali/tests.rs` (next to
`case_89_flatlast_is_byte_equal_to_mesa_26_2_1`, `:4700`):

1. `case_199_fragment_trunc_is_byte_equal_to_mesa_26_2_1`: sources
   `td!("fsarith_199_es3trunc.vert")` / `.frag` / `.expect.json`, asserted
   equal to `tools/shaders/gltest/es3trunc_vs.vert` and `es3trunc_fs.frag`;
   `check_linked_for("199 fs", vs, fs, json)` (`:3824`, no uniform values,
   both IDVS programs and the fragment program with all facts).
2. The record: `fragment_reads(..)` of the compiled fragment shader decodes
   (`motion::decode`) to reads `(0, 0b0111)` and two moved expressions, lane
   1 = `Fmul(I2f(Isub(Ishr(F2i(Lane 0,1), 1), Ishl(F2i(Lane 0,1), 3))),
   0x3c800000)` and lane 2 = `Fmul(U2f(Ushr(F2u(Lane 0,2), 2)), 0x3c800000)`,
   as `case_89` checks its one (`:4710-4728`).
3. A key round trip with the new `OPS` bytes (`tests.rs:4288` style): the
   old numbers 0-4 are unchanged, the new ones are 5-11.
4. Sentinels: `s4_fneg` and `s4_motion_sum`/`s4_motion_dot` stay byte-equal
   (the narrowed guard must not move them) and `case_89` stays; the
   existing `case_199_vertex_integers..` and `varyconst 199` stay.
5. `tools/mali/corpus-verdicts.txt`: `gltest/es3trunc_fs.frag` becomes `ok`
   (generated file, regenerated by `tools/mali/corpus-verdicts.sh` in the
   same commit; `tools/check.sh` compares the bytes).
6. A negative control: the same fragment shader with `u_q` read as an
   *attribute-fed* varying (`in vec4 v` from `a_color`) keeps `F2i` in the
   fragment shader (an interpolated lane is not movable through `f2i`) and
   is refused as before, so the convergent test is what lets 199 through.

## 6. Risks / open

* No board is needed: the reference is the draw of the shim. The sweep case
  199 draws both programs; the top half reads `v` of the same values at every
  vertex, so the move is exact (the shifts are on exact integers).
* Pixel check (host, not measured): `trunc(2.75) = 2 -> 0.5`,
  `trunc(-2.75) = -2 -> 0.25`, `(-6 >> 1) - (-6 << 3) = -3 + 48 = 45`,
  `200 >> 2 = 50`, as `gl_test.c` says.
* If the fragment graph builds the `isub` as `Iadd`+`Ineg` and the
  `expression()` order puts `Ineg` between, the vertex words differ in the
  order of the two shifts only; that is a one-line fix in `expression()`.
* The first-refusal list of §4.1 is from reading; the first test run of
  fs-trunc will show the real order.
