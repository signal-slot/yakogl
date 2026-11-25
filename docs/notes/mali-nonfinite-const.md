# Mali: built-ins on constants in a mediump fragment shader (plan #367 lower-nonfinite-const)

The refusal "a constant operation on or to a value that is not a finite
normal float" stopped five glmark2 fragment shaders: shading
blinn-phong-inf `994f` and cel `fea0`, bump `d442`, `487d` and `77c5`. None
of them holds a value that is not finite. Each one calls a built-in on
constants: `normalize(vec3(0.5, 0.5, 1.0))`, `length(LightSourcePosition)`
of a global `const vec4`. The front end expands that call into
`Fdot`/`Frsq`/`Fsqrt`/`Fmul` of constants. The mediump walk
(`src/shaderc/online/mali/nir_fs.rs`) had no fold for a lone product of two
constants or for `inversesqrt` of a constant, so it refused them.

## What Mesa does

Mesa's GLSL front end folds a built-in call whose arguments are constant
expressions (ast_function.cpp:657-664, `constant_expression_value`). It
works at fp32, before lower_precision:

- `dot` is `result += a[c] * b[c]` from lane 0 (ir_constant_expression.cpp
  `dot_f`);
- `inversesqrt` is `1.0F / sqrtf(x)`, and `sqrt` is `sqrtf`;
- the products, sums and negations of the expansion are folded with it.

lower_precision then rounds the folded constant once to fp16
(`_mesa_float_to_half`). That gives these references:

- gm2_nonfin_2: `normalize(vec3(0.5, 0.5, 1.0)) * 0.5` is the FAU words
  `0x32883288` and `0x36883688`;
- gm2_nonfin_4: `length(LightPos)` is `0x45d5` (sqrt(34)).

A product or sum of constants that is not a built-in's is left alone by the
GLSL front end: NIR folds it after the precision lowering, at fp16
(`fold_const16`, unchanged).

## The lowering

- `const_tree32` evaluates an IR value at fp32 as the GLSL front end does.
  Its leaves are `ConstF32` and a local stored once by such a tree (a global
  `const`). Its operations are `Fdot2/3/4`, `Frsq`, `Fsqrt`, `Fmul`, `Fadd`
  and `Fneg`.
- `builtin16`: when every lane of a mediump `Fdot`, `Frsq`, `Fsqrt`,
  `Fmul`, `Fadd` or `Fneg` folds and its tree holds a built-in, the walk
  takes the lanes rounded once (`round16`). It does this before the lane
  walk. Any other operation keeps the walk's own arms. In particular, the
  pair arm (two constants in a lane pair with a computed lane) stays an ALU
  lane.
- A lone `Fmul`/`Fadd` of two constants outside a pair (`Fs::op`) is folded
  at fp32 and rounded once (`fold_const32`). It is no longer refused.
- A mediump `Fdot` whose lanes are all constants, but which `builtin16` did
  not fold, is still refused (`ConstantFold`). For example, a lane past
  fp16's range: `length(vec3(60000.0, 60000.0, 0.0))`.
- `x / sqrt(y)` (`frcp` of `lower_fsqrt`'s `frcp(frsq(y))`) becomes
  `frsq(y)`, by nir_opt_algebraic.py's `frcp(frcp(a))`. In gm2_nonfin_4 this
  is `FRSQ.f16 r0, r0^.h00`, then `FMA.v2f16 r0, u2.w1, r0^.h00, 0x0.neg`.

## A defect kept out

`normalize(vec3(1.0, 2.0, 2.0)) * v.x` (fsvec_nconst_mediump) folds to
lanes y and z of one half (`0x3955`). The pair graph then reads z's product
as the x lane's half (`MKVEC.v2i16 r1, r0.h0, ..`), which is wrong code. The
same happens with the constant written out,
`vec3(0.33333334, 0.6666667, 0.6666667) * v.x`, at 4a858e7b, so it is a
defect of the pair graph and not of this fold. A fold whose lanes repeat
one half across two pairs is refused, as the walk refused a constant
`inversesqrt` before (`MaliError::Arithmetic`). The defect itself is a new
cause.

## Results

`tests/mali_nonfinite_const.rs`:

- gm2_nonfin_2 and gm2_nonfin_4 are byte-equal.
- gm2_nonfin_1 is one instruction longer: 20 words against 19, because
  lower-constprod-rest owns the reordering of `(v * r) * L`
  (nir_opt_algebraic.py:2394).
- gm2_nonfin_3 stops at FS_OPERAND and gm2_nonfin_5 at FRAGCOLOR_LANE.
- fsvec_lconst_mediump now matches.

The reference sweep shows no other change. The two dEQP negzero cases keep
their refusal text.

glmark2: `994f` (scene 6) and `d442` (scene 9) compile, so 16 of 33 scenes
become 18 of 33. `fea0` moves to ARITH_CONST, `487d` to TEX_SWIZZLE and
`77c5` to TEX_MULTI. `tools/mali/corpus-verdicts.txt` does not move.
