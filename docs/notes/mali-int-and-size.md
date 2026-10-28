<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: integer and boolean values, and shaders over the 256-value cap

Plan #153, item study-intbig. Eight cases of the sweep
`0dce654a8:tools/mali/board-ref/2026-10-23-s6-discard/ours/03-panthor-gltest--sweep-1.log`
are refused by the Valhall back end for two reasons that have nothing to do
with each other: four (42, 72, 79, 112) by an integer or boolean value, four
(184, 193, 194, 195) by the size of the shader.

**State of this note.** The references below were recorded with
`tools/mali/drm_shim_ref.sh` on Mesa 26.2.1 (`tools/mali/testdata/intbig_*`:
`.trace`, `.stdout`, `.stderr` per program). The Mesa source reading, the
refusal sites and the caps are from the code. **Since plan #154 item
sizelimit** the caps are raised and the offarm programs of 193, 194 and 195
are byte-equal (see "The caps since plan #154"); the sections between were
written before, against the old caps.

## Since plan #154 item intbool (session 1)

Cases 72 and 112 compile byte-equal in both stages
(`case_72_intlit_is_byte_equal_to_mesa_26_2_1`,
`case_112_affectand_is_byte_equal_to_mesa_26_2_1`; their
`intbig_case72/112.expect.json` are `pan_ref_diff.py extract <trace>
--draw` of the traces below, drawn with no uniform values). What the
Mali compiler (`src/shaderc/online/mali`) does for them:

- **Typed values.** An int is its 32-bit bits and a bool NIR's 0 / ~0
  (`ConstI32`, `ConstBool` in either precision). A mediump shader's integer
  and boolean operations are 32-bit nodes (`Fs::int_lane`): `F2i` of an
  fp16 value is `Alu::F2i32` (`F16_TO_S32.rtz`), `Ilt`/`Ieq` the compares,
  `!` an `Alu::Inot` the walk folds into its users (`inot(inot(a))`,
  `iand(a, a)`, `bcsel(inot(a), b, c)` as `bcsel(a, c, b)`), a bool
  uniform's `i2b` the word itself (Mesa tests it as it is,
  `intbig_case112.stderr`).
- **`nir::fold_int`**, the integer constant folding and the rules a constant
  or a repeated operand decides (`iadd(a, 0)`, `iand(a, ~0)`, `ior(a, a)`,
  ...), in both walks (the highp walk's own folding moved into it).
- **Control flow of a mediump shader.** A loop is walked (unrolled) as a
  highp shader's when its passes fold (72's counter); an `if` whose sides
  assign booleans only is flattened into folded selects (112's inlined
  calls, `walk_if16`); anything else stays `ControlFlow`.
- **The colour of masks.** A mediump colour of constants and selects of
  constants on 32-bit masks (`nir_fs::mask_colour`, builtin_49's path) now
  takes masks of uniforms too, with the fp16 uniform block (`fs16`, a bool
  one 32-bit word), and a constant before a select's half as
  `MKVEC.v2i16 u2.w1.h0, r0^.h0`.

- **Integers of fp16 varying lanes** (`intloop_int_compare`, byte-equal):
  `docs/notes/mali-int-loops.md`, "State since plan #154 item intbool".

Case 79 (integer division) and case 42 stay refused (intbool-2): 79 at
`idiv.frag`'s division (`IntValue`), 42 at `array.frag`'s `textureSize`
(the texture refusal). Cases 80, 100 and 116 are no longer refused by an
integer: 80's fragment shader stops at the chain refusal (`do_rebalance_tree`),
100's two stages and 116's vertex stage at the run-time uniform index
(`UniformIndexed`); 116's fragment shader compiles. Of the integer
references, `intloop_tri_int` (fp32 integer arithmetic and `sign()` in a
mediump program, mali-int-loops.md) and `intuni_sharedint` (its fragment
`sin`, which the Mali compiler does not lower, and its vertex `if`, which
nir.rs refuses as vertex control flow; mali-int-uniforms.md) are not
lowered.

## The eight cases and their shaders

| case | name | vertex shader | fragment shader | refused by |
|---|---|---|---|---|
| 42 | es3array | `tools/shaders/es300/cube.vert` | `tools/shaders/es300/array.frag` (`textureSize` -> ivec3, `int(layer)`, `texelFetch`) | integer |
| 72 | intlit | `tools/shaders/gl_test_es100/passthrough.vert` (`tools/shaders/passthrough.vert` is GLSL 4.50, which Mesa's ES driver rejects) | `tools/shaders/es100/intlit.frag` (int loop, `t == 2`, `ivec2`/`bvec2` constants) | integer |
| 79 | idiv | `tools/shaders/gl_test_es100/passthrough.vert` (`tools/shaders/passthrough.vert` is GLSL 4.50, which Mesa's ES driver rejects) | `tools/shaders/es100/idiv.frag` (`ivec2 / ivec2`, `100 / int`) | integer |
| 112 | affectand | `tools/shaders/gl_test_es100/passthrough.vert` (`tools/shaders/passthrough.vert` is GLSL 4.50, which Mesa's ES driver rejects) | `tools/shaders/es100/affectand.frag` (`bool`, `&&`, comma, `u_false == sideEffectA`) | boolean |
| 184 | depchain | `tools/shaders/gltest/position_only2.vert` | `tools/shaders/gltest/dep1_fs.frag`, `dep2_fs.frag`, `dep4_fs.frag` (48 `x*u_b+u_c` steps in 1, 2, 4 chains) | size |
| 193 | offarm | `tools/shaders/femtovg/main.vert` | `tools/shaders/offarm/fill_color.nodiscard.offarm{8,32,128}.frag` (8 compiles; 32 and 128 refuse), and `case_offarm` also draws `tools/shaders/femtovg/fill_color.nodiscard.frag` (offarm0) | size |
| 194 | offarmn | same | `offarm{0..256}`; 2, 4, 8, 12, 16 compile (`corpus-verdicts.txt`), 24 and up refuse | size |
| 195 | ifbranch | same | 193's four programs (offarm0, 8, 32, 128) | size |

The case bodies are `case_es3array` (`driver/tools/pvrlab/gl_test.c:6425`),
`case_intlit` (`:9950`), `case_idiv` (the `link_program` at about `:14675`),
`case_affectand` (`:14333`), `case_depchain` (`:16027`), `case_offarm`
(`:16806`), 194 and 195 (`:16847`); the shader names are bound in
`driver/tools/pvrlab/gen_gl_test_shaders.py` (`gl_test_dep_vs_src` is
`gltest/position_only2.vert`, line 256). The sweep's own words per case are
in the log at the `verdict case N` lines (3147, 4763, 5001, 6104, 8821, 9196,
9305, 9364).

## References (recorded)

Work dir of the existing Mesa 26.2.1 drm-shim build:
`$CONNECTED_DEPS_DIR/mesa-shim` (`inst/lib64/libpanfrost_noop_drm_shim.so`;
the item brief names `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, a tree
whose `build-pan` has `pan_shc` but no `inst/`; `drm_shim_ref.sh build`
makes an `inst/` from the tarball, `tools/mali/README-pan_shc.md`).
From the repository root, one command per program:

```
W=$CONNECTED_DEPS_DIR/mesa-shim S=tools/shaders O=tools/mali/testdata
tools/mali/drm_shim_ref.sh run $W $S/es300/cube.vert   $S/es300/array.frag  $O/intbig_case42
tools/mali/drm_shim_ref.sh run $W $S/gl_test_es100/passthrough.vert  $S/es100/intlit.frag $O/intbig_case72
tools/mali/drm_shim_ref.sh run $W $S/gl_test_es100/passthrough.vert  $S/es100/idiv.frag   $O/intbig_case79
tools/mali/drm_shim_ref.sh run $W $S/gl_test_es100/passthrough.vert  $S/es100/affectand.frag $O/intbig_case112
for n in 1 2 4; do tools/mali/drm_shim_ref.sh run $W $S/gltest/position_only2.vert $S/gltest/dep${n}_fs.frag $O/intbig_case184_dep$n; done
for n in 32 128; do tools/mali/drm_shim_ref.sh run $W $S/femtovg/main.vert $S/offarm/fill_color.nodiscard.offarm$n.frag $O/intbig_case193_offarm$n; done
for n in 24 256; do tools/mali/drm_shim_ref.sh run $W $S/femtovg/main.vert $S/offarm/fill_color.nodiscard.offarm$n.frag $O/intbig_case194_offarm$n; done
```

(195 uses 193's programs: its reference is 193's.) The commands above are
the ones that produced the files (the case 72/79/112 lines with the ES 1.00
`gl_test_es100/passthrough.vert`). Each gives `.trace` (pandecode: the
program's bytes), `.stdout` and `.stderr` (`BIFROST_MESA_DEBUG=shaders,shaderdb`:
NIR, BIR and final disassembly, and shaderdb's line). `gl_ref.py` bound case
42's `sampler2DArray` (a 2D texture; the program compiled). `u_false` of case
112 is not set (drm-shim runs nothing), which does not change the code.

### Mesa's fragment shader per case (shaderdb, `MESA_SHADER_FRAGMENT`)

| program | instrs | code size | registers | spills:fills | uniforms |
|---|---|---|---|---|---|
| case 42 es3array | 41 | 384 | 12 | 0:0 | 4 |
| case 72 intlit | 9 | 128 | 4 | 0:0 | 4 |
| case 79 idiv | 181 | 1536 | 13 | 0:0 | 6 |
| case 112 affectand | 9 | 128 | 4 | 0:0 | 3 |
| case 184 dep1 / dep2 / dep4 | 58 / 61 / 66 | 512 / 512 / 640 | 5 / 5 / 7 | 0:0 | 4 |
| case 193 offarm32 / offarm128 | 88 / 184 | 768 / 1536 | 8 / 8 | 0:0 | 11 |
| case 194 offarm24 / offarm256 | 80 / 312 | 768 / 2560 | 8 / 8 | 0:0 | 11 |

Findings from the dumps:

- **Case 72 and 112 are almost nothing in Mesa** (9 instructions each).
  Case 72's fragment shader (`intbig_case72.stderr`, final disassembly) is
  `F16_TO_S32.rtz r0, u2.w0.h0` (`int(u_in.x)`: `u_in` is `mediump`, a
  `f16vec4` uniform, so the conversion reads the half `u2.w0.h0`),
  `ICMP_OR.u32.eq.m1 r0, r0, 0x3020100.b2, 0x0` (the compare with `t == 2`:
  `0x3020100.b2` is byte 2 of the constant `0x03020100`, i.e. 2; the `.m1`
  result is a 32-bit mask word, all ones for true; the `_OR` accumulates
  with the 0 operand), `MUX.v2i16.int_zero r0, 0x0, 0x3C000000.h11, r0.h00`
  (the bool as the half pair 0.0 / 1.0, tested on the low half of `r0`),
  `ATEST`, `MKVEC.v2i16 r0, u2.w1.h0, r0.h0`, `MOV.i32 r1, u3.w0` and
  `BLEND`. The loop is gone (red is the constant 0.75 = `0x3a00` half,
  loaded as a uniform word) and the `ivec2`/`bvec2` constants are folded
  (blue is the constant 1.0, `0x3C00`). So the integer work of case 72 is
  exactly three things: `F2i` (`F16_TO_S32` for a mediump source,
  `F32_TO_S32` for highp), an `Icmp eq` against a constant, and a `Bcsel`
  between two floats (`MUX.v2i16`).
  Case 112 (`intbig_case112.stderr`) is `MOV.i32 r0, u2.w0` (the bool
  uniform, one 32-bit word), two `MUX.v2i16.int_zero` (`0.0 / 1.0` and the
  inverse, both on `r0.h00`), `MKVEC.v2i16`, `MOV.i32 r1, 0x3C000000`,
  `ATEST`, `BLEND`. `affect_and` is decided at compile time apart from
  `u_false`; the NIR is `load_ubo` 32-bit, `i2i16`, two `bcsel_pan`.
  Both need only `MUX` with a bool word.
- **Case 79 is the heavy one** (181 instructions, 13 registers, no loop,
  0 spills). Mnemonic histogram of the fragment shader's final
  disassembly (`intbig_case79.stderr`, lines 985-1164, which includes the
  `BRANCHZI` / `BLEND` / `ATEST` tail):
  `U16_TO_U32` 44, `IADD.s32` 30, `IMUL.i32` 28, `ISUB.s32` 12, `CSEL.u32.eq` 9,
  `ICMP_OR.u32.ge` 6, `FMA.f32` 6, `IADD_IMM` 5, `IABS.s32` 5,
  `F16_TO_S32.rtz` 4, `U32_TO_F32` 3, `S32_TO_F32` 3, `LSHIFT_XOR` 3,
  `ICMP_OR.s32.lt` 3, `FRCP.f32` 3, `FADD.f32` 3, `F32_TO_U32.rtz` 3.
  That is three `nir_lower_idiv` expansions of about 55 instructions each
  (see "Division" below), plus the `F16_TO_S32` of the operands and the
  `S32_TO_F32` / float epilogue.
- **Mesa has no spill for any of the eight** and uses at most 13 work
  registers. So the register file is not why we cannot lower the size cases.
- **Size cases:** 184's three programs are 58 to 66 instructions, 5-7
  registers; Mesa fuses `x*u_b+u_c` into one `FMA.f32` per step. The offarm
  programs grow by exactly one instruction per step: offarm24 80, offarm32 88,
  offarm128 184, offarm256 312 (=56 + N). Even offarm256 is 312 instructions
  against our `bir::MAX_INSTRS = 192`.

## Integers: what Mesa does (read from the 26.2.1 source)

All in `src/panfrost/compiler/bifrost/`; Valhall shares the BIR with Bifrost.

- **Booleans are 32-bit.** `pan_nir_lower_bool_to_bitsize`
  (`bifrost_nir.c:350`) turns every comparison into the `*_pan` form with a
  bool of the operand width (32 here), so a comparison result is a 32-bit
  0 / 0xffffffff word; `bcsel_pan` consumes it (`MUX.i32 .int_zero`,
  `docs/notes/mali-lowering-map.md:212`).
- **Comparison**: `ieq_pan ine_pan ilt_pan ige_pan` -> `ICMP.s32 .eq/.ne/.lt/.ge`
  with `BI_RESULT_TYPE_M1` (`bifrost_compile.c:3253-3258`); `ult_pan uge_pan`
  the `.u32` forms (`:3260-3265`); float compares are `FCMP` (the existing
  `Flt` row). The recorded case 72 shows the equality as
  `ICMP_OR.u32.eq.m1` (the `_OR` form with a zero accumulator; equality does
  not depend on signedness), case 79 the ordered ones as
  `ICMP_OR.u32.ge` and `ICMP_OR.s32.lt`.
- **Bool to number**: `b2i32` is `MUX.i32 0, 1, s0 .int_zero`
  (`:3248-3250`). `b2f32` of a mediump value is the `bcsel_pan` the dumps
  show: `MUX.v2i16.int_zero 0x0, 0x3C000000.h11, x.h00` (case 72 and 112):
  a `MUX` between the half constants 0.0 and 1.0, no conversion
  instruction. `f2i32` is `F32_TO_S32` (`:3100`), `F16_TO_S32.rtz` for a
  half source (case 72, 79); `i2f32` `S32_TO_F32` (`:3193`, `u2f32`
  `:3167`).
- **Arithmetic**: `iadd` -> `IADD.s32` (`:3306`, 64-bit `SHADDX`), `imul` ->
  `IMUL.i32` (`:3367`), `ineg`, `iabs`, `iand/ior/ixor/inot` (`LSHIFT_*`-family
  or `LOP`; `:3375-3387`), `ishl/ishr/ushr` (`:3047-3060`), `imin/imax`.
- **Division**: not a hardware op. `nir_lower_idiv` (`bifrost_nir.c:1078-1081`),
  after `nir_opt_idiv_const` has taken every constant divisor (`:1072`, and
  in the optimiser loop `:240`). Case 79's NIR (`intbig_case79.stderr`
  lines 248-300 for the first lane) shows the precise sequence: `iabs` of
  both operands, `u2f32` of the divisor, `frcp`, `fmul` by `4.294967e+09`
  (`0x4f7ffffe`), `f2u32` (the quotient estimate), `imul` + `ineg` (its
  error), then a 32x32 -> high-32 multiply built from `extract_u16` and four
  16-bit `imul` plus `iadd`s (the `U16_TO_U32` and `IMUL.i32` runs), a
  second multiply for the quotient estimate, `imul` + `isub` for the
  remainder, and two compare-and-adjust steps (`uge_pan`, `b2i32`, `iadd`,
  `isub`, `bcsel_pan` = `ICMP_OR.u32.ge` + `CSEL`), then the sign fix-up
  (`ixor`, `ineg`, `bcsel` = `LSHIFT_XOR` + `CSEL`). Case 79 has run-time
  divisors (`int(abs(u_d.z)) + 1`) so it takes the full sequence three
  times (two lanes plus `100 / (...)`): 181 instructions in all, so about
  55 per division. This is the cost an integer-division lowering has to
  match byte for byte.
- **Case 72's loop** (`for (int i = 0; i < 3; i = i + 1)`) has a constant
  trip count; Mesa unrolls it (`nir_opt_loop_unroll`) and folds it: no
  integer op is left for red. The green channel (`t == 2`) is
  `F16_TO_S32.rtz` + `ICMP_OR.u32.eq.m1` + `MUX.v2i16`, as above. The blue
  channel's `ivec2(1,2)`, `bvec2(1,0)` constant-fold to 1.0. The only
  integer values left in the whole program are the `F16_TO_S32` result and
  the compare mask.

## Where our side refuses

Two copies of the same walk, one per stage:

- `src/shaderc/online/mali/nir_fs.rs:1373`: `ir::Op::ConstI32(_) | ir::Op::ConstBool(_) => Err(MaliError::IntValue)`
  (a `ConstI32` is accepted just before it in a highp shader, `:1372`, as
  femtovg's `texType == 1`); and `:1504`, `:1514`: the `F2i` and `Iadd` arms
  accept only a constant or `x + 0`. Anything else integer falls to the
  `Alu(..) => Err(Arithmetic)` arm (`:1517`). `docs/notes/mali-lowering-map.md:213`
  already plans the `ICMP` row ("S4, `texType == 1`").
- `src/shaderc/online/mali/nir.rs:1351`: the vertex walk, the same arm.
- The message: `src/shaderc/online/mali/mod.rs:335`
  ("Valhall: an integer or boolean value is not lowered yet"); the variant
  `MaliError::IntValue` is at `mod.rs:123`.

So today an integer survives only as a constant, `f2i` of a constant, and
`+ 0`. What the four cases need, per the shaders above:

| case | needs |
|---|---|
| 72 intlit | `ConstI32`/`ConstBool` as values; `F2i`; `Icmp eq`; `Bcsel` to a float (`MUX`); `&&`/`!` on bools; loop unrolled before the walk (or a counted loop in the flow code) |
| 112 affectand | `ConstBool`, bool uniform load (a 32-bit word from the FAU; Mesa reads it as `MOV.i32 r0, u2.w0` and tests the low half with `.h00` in `MUX.v2i16.int_zero`, `intbig_case112.stderr`), `Fcmp`/`Icmp eq` on bool, `&&`, `!`, `Bcsel`, the comma operator, local bools |
| 79 idiv | `F2i`, `Iadd`, `Iabs`, `Idiv` (the `nir_lower_idiv` sequence), `I2f`, `Fmul` |
| 42 es3array | `TextureSize` and `TexelFetch` (texture work, see `docs/notes/mali-texture-refusals.md` from item study-tex) plus `F2i`, `I2f`, int vectors: the largest of the four |

## The size caps

All three are fixed arrays in the compiler's scratch arena, "the tables
bound memory" (`mod.rs:636`, "Table"):

- `nir::MAX_VALUES = 256`, "the IR values the walk keeps lanes for (the
  fragment walk's too)" (`nir.rs:56`). The fragment walk checks
  `m.instrs.len() > MAX_VALUES` -> `MaliError::TooManyValues`
  (`nir_fs.rs:1057`, the vertex walk `nir.rs:1321`). `m.instrs` is the
  front-end IR, one entry per value, *not* per component: a `vec4` is one
  value (`m.comps_of`).
- `nir::MAX_NODES = 384`, "the instructions one vertex shader may hold
  here" (`nir.rs:54`): the NIR-node arena, per node `lanes`, `order`, `live`,
  `block`, `nonuniform` (`nir.rs:571-590`), `TooManyValues` at `nir.rs:919`
  and `:1812` and `select.rs:447`.
- Both map to the one message (`mod.rs:438`, "(or 384 instructions before
  selection)").
- Behind them, the *lowered* limits: `bir::MAX_NODES = 192` SSA values
  (`bir.rs:38`, `ValueTableFull`), `bir::MAX_INSTRS = 192` (`InstrTableFull`),
  64 immediates (`TooManyConstants`), `MAX_LOCALS = 32`, `MAX_IFS = 8`
  (`nir.rs:60`), and a split register file of 32 work registers: more is
  `MaliError::Spill` ("the full file, spilling", `ra.rs:689`). The measured
  high-water marks of the corpus so far are in `tests.rs:2176`: 34 IR values,
  52 NIR instructions, **116 of 192 BIR instructions, 56 of 192 BIR values**.
  Raising `MAX_VALUES` alone moves a refusal from the front to
  `InstrTableFull` / `ValueTableFull` / `Spill`; all four tables have to be
  raised together.

### The four shaders' sizes

Mesa's final sizes are in the table above. Our own IR / NIR value counts are
not measured here. Rough counts from the sources: `dep1/2/4_fs` hold 3 uniform
loads and 48 `x*u_b+u_c` steps, each an `Fmul` and an `Fadd` (96), plus 4
starting values and the sum: about 105-110 IR values, *below* 256, yet
`corpus-verdicts.txt` lists all three as `TooManyValues`. So either the front
end makes more IR values per step than counted, or the 384 NIR-node check
(`nir.rs:919`, `select.rs:447`) fires. **Which cap fires is the first thing to
measure**: `module_facts(stage, src)` in `src/shaderc/online/mali/tests.rs`
returns `marks` per `Table` (`Table::ALL`, `mod.rs:703`); a one-off test
printing it for dep1/2/4 and offarm 0-256 gives it directly. `offarm16`
compiles and `offarm24` does not (`corpus-verdicts.txt`): Mesa's offarm24 is 80
instructions and offarm16 would be 72, so the cap is crossed between
those. Whatever the IR counts, the back end's `MAX_INSTRS = 192` and
`bir::MAX_NODES = 192` are below Mesa's own 312 instructions for offarm256.

### Mesa's register use and spill

None spills; 5 to 13 registers (table above). Our file refuses past 32
(`ra.rs:689`), so register pressure is not the limit for any of the eight.

### What raising the cap takes

1. `MAX_VALUES`/`MAX_NODES` are `const`s sizing stack-and-arena arrays
   (`[[Val; 4]; MAX_VALUES]` in `nir.rs:1232` is 256 x 4 x sizeof(Val)):
   doubling them is a one-line change, with the arena budget
   (`Arena`, `OutOfScratch`) checked for the no-std targets, since the
   walk's frame arrays live there.
2. `bir::MAX_NODES` and `MAX_INSTRS` (192, with the instruction table the
   packer and scheduler index with `u8`/`u16`s: check `sched.rs`, `ra.rs`)
   must grow with them; `Table` high-water output tells by how much.
3. The register allocator is the real limit: dep4/offarm256 are long
   chains; a straight-line program with few live values does not need the
   spill path, a wide one does (`Spill`, `ra.rs:689`).

### The caps since plan #154 (item sizelimit)

Measured with `module_facts` (`src/shaderc/online/mali/tests.rs`), each
table's mark for the fragment shader, in a scratch build with every cap
raised far past it:

| program | IR values | NIR instructions | BIR instructions | BIR values | scratch peak |
|---|---|---|---|---|---|
| offarm24 (194) | 266 | 147 | 97 | 98 | 0.80 MB |
| offarm32 (193, 195) | 314 | 163 | 105 | 106 | 0.82 MB |
| offarm128 (193) | 890 | 355 | 201 | 202 | 1.57 MB |
| offarm256 (194) | 1658 | 611 | 329 | 330 | 2.73 MB |
| femtovg main.vert | 27 | 35 | 56 | 32 | |

The IR grows by six values an arm, the NIR by two, the BIR by one (Mesa's
312 instructions for offarm256 are ours less the constant and FAU moves).
Case 184's dep1/2/4 need 336 IR values (sizelimit-measure) and are then
refused by the gl_FragColor lane and the chain (the chains item).

The caps, each the offarm256 mark with headroom:

| cap | was | now |
|---|---|---|
| `nir::MAX_VALUES` | 256 | 2048 |
| `nir::MAX_NODES` | 384 | 768 |
| `bir::MAX_INSTRS` | 192 | 384 |
| `bir::MAX_NODES` | 192 | 384 |

With them `intbig_case193_offarm32/128`, `intbig_case194_offarm24/256` and
`intbig_case195` are byte-equal in both stages
(`case_19*_is_byte_equal_to_mesa_26_2_1`; their `.expect.json` are
`pan_ref_diff.py extract <trace> --draw`, the vertex shader's uniforms unset),
and every offarm source of the corpus compiles. The scheduler's order is
untouched (sched-u16 widened its indices; its edge table is sized by the
largest block, 329² u16, about 216 KB for offarm256). The vertex walk's value
table moved from a stack temporary into the scratch arena (2048 x 4 lanes);
the other `[_; MAX_VALUES]` arrays stay on the stack, at most about 18 KB at
once (`order_keys`' 10 KB calling `grafted_locals`' 8 KB; not measured on a
target), inside the 256 KiB the compiling thread is given
(`zephyr/Kconfig`, YAKOGL_SHADERC_ONLINE). The largest scratch use,
offarm256's 2.73 MB, is inside the 4 MiB default region.

With the value table at 384 two stress sources of the gap tests moved: the
alternating chain of 24 attribute loads that filled the 192 values now fits,
and a vertex shader of more than 16 attributes is then refused by the encoder
as a defect (`Encode`), not by a gap (open, not in this item); the value table
is now reached by 27 matrix products, the instruction table by 21.

## Recommended order

1. **Measure first** (no code): the `marks` of the 14 programs and the 8 Mesa
   dumps above. It decides whether 184/193/194/195 are one cap or three.
2. **Size before integers for the four size cases**: they need no new
   lowering, only the tables and a byte-equal check against the new
   `intbig_case184_*` / `case193_*` / `case194_*` words, and they unlock
   the cost-measurement cases (184-195 are PowerVR perf probes, so
   the board result is a PASS of a draw, not a pixel check). Raise
   `MAX_VALUES` and `MAX_NODES` together with `bir::MAX_NODES/MAX_INSTRS`,
   to the largest of the measured programs plus headroom, one step per
   commit.
3. **Integers, smallest first**: 72 (constant `Icmp eq`, `F2i`, `Bcsel`
   to float: the `ICMP` row of `mali-lowering-map.md:213` is already drawn),
   then 112 (booleans as 32-bit words, `&&`, `!`, comma), then 79 (the
   `nir_lower_idiv` sequence, many instructions, so after the size work),
   and 42 last because it needs the texture-size / texel-fetch work of
   study-tex.
4. Each step: its `intbig_*` reference first, the program words byte-equal,
   `corpus-verdicts.txt` moving forward only.

## Case 195 (`ifbranch`): what it refuses and its reference

Plan #153 item `study-matdyn-ifbranch`, issue #1386. Case 195 draws 193's four
programs (offarm0, 8, 32, 128) with `femtovg/main.vert`, each once compiled for
PowerVR as predicated and once as branched (`PVR_BACKEND_OPT_IF_LOWERING`; a
PowerVR option Mesa does not have, so the Mali back end sees the same four
sources twice). In the newest sweep
(`0dce654a8:tools/mali/board-ref/2026-10-24-refusals/ours/15-panthor-gltest--sweep-1.log`,
lines 9882-9907) offarm0 and offarm8 compile and draw, and the third,
`offarm/fill_color.nodiscard.offarm32.frag`, is refused (`does not compile`,
then the `TooManyValues` message). So 195's blocker is the first program over
the cap, offarm32, exactly where `corpus-verdicts.txt` puts it (offarm16 ok,
offarm24 refused).

**Reference.** `tools/mali/testdata/intbig_case195.{vert,frag,trace,stdout,stderr}`:
`main.vert` and `offarm32.frag`, verbatim, recorded with

```
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/mali/testdata/intbig_case195.vert tools/mali/testdata/intbig_case195.frag tools/mali/testdata/intbig_case195
```

Mesa 26.2.1's counts (shaderdb) are identical to `intbig_case193_offarm32` (the
trace differs only in addresses): fragment **88 instructions**, 768 bytes of
code, **8 registers**, 0 spills, 11 uniforms; position shader 25 instructions,
9 registers; varying shader 17 instructions, 11 registers. offarm128, the
largest program 195 draws, is 184 instructions / 8 registers
(`intbig_case193_offarm128`). Nothing spills.

**Which tables must grow.** Same as 193's: the front-end IR walk
(`nir::MAX_VALUES = 256`, and `MAX_NODES = 384`) and the lowered tables
(`bir::MAX_NODES`/`MAX_INSTRS = 192`). For 195 the target is offarm128 (184
Mesa instructions, so `bir::MAX_INSTRS` of at least about 200 with headroom and
the IR/NIR tables sized from the measured `marks`); offarm32 (88) needs only
`bir` as is and the front-end tables past the point where it currently trips,
which the unmeasured `marks` of "The four shaders' sizes" decide. 195 needs no
integer lowering: the offarm arms are float steps behind a `scissorRadius`
test.

**Order.** The recommended order above already covers it: 195 is one of the
"size before integers" cases, with 193, and needs no step of its own beyond
the same table growth; it passes once offarm128 compiles (offarm32 first), so
the `intbig_case193_offarm32/128` words are also 195's check. The `.trace`
here is kept, as for the other `intbig_*` references.
