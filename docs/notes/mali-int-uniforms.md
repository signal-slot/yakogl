<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): int, uint and bool uniforms, Mesa's layout and op selection

Issue #1384 / #1385, plan #153 item `study-vstex2`. Seventeen rows of
`tools/mali/corpus-verdicts.txt` are refused with `Valhall: a uniform other
than a float, vec2, vec3, vec4 or mat4 is not lowered yet`
(`MaliError::UniformType`). `docs/notes/mali-uniform-types.md` covers cases 100,
116, 166 and 80 and says Mesa packs an `int` as one 32-bit word. This note adds
seven more programs, recorded on Mesa 26.2.1's panfrost on the drm-shim
(`tools/mali/drm_shim_ref.sh`, ES 3.00 context for `#version 300 es`, no
`--uniform-values` because they declare integers), and says for each one how
Mesa lays the uniforms out in the FAU, which integer operations it selects, and
how much of the refusal lowering the type removes. No compiler code was
changed.

References, `tools/mali/testdata/intuni_<name>.{vert,frag,expect.json}`; the
`.frag` (and `idxtex`, `sharedint` `.vert`) are copies of the corpus sources,
the other `.vert` files are a pass-through of `a_pos` (`intuni_gcops.vert` also
outputs `v_t`). Reproduce with, from the repository root:

```
D=tools/mali/testdata
tools/mali/drm_shim_ref.sh run <work> $D/intuni_<n>.vert $D/intuni_<n>.frag $D/intuni_<n>
python3 -I tools/mali/pan_ref_diff.py extract $D/intuni_<n>.trace --draw > $D/intuni_<n>.expect.json
```

`<work>` is `$CONNECTED_DEPS_DIR/mesa-shim`; `<n>` is `uops`, `ishr`, `gcops`,
`ifelse_uint`, `intrt`, `idxtex`, `sharedint`. The `.trace` files are not kept.

**State since plan #154 item intbool (session 1).** `int`, `bool` and
`ivec2/3/4` uniforms take one word a component in `nir::uniform_of` (both
fp32 stages' layout), and a highp fragment shader reads an int uniform as
a value, not only as an array index. `ishr` is byte-equal
(`intuni_ishr_is_byte_equal_to_mesa_26_2_1`): `ARSHIFT_OR.i32` with the
amount moved to a register for its byte lane (emit.rs, `Alu::Ishr`),
`U8_TO_F32` of bytes 0 and 1 (`Alu::U8f32`, nir_opt_algebraic's extract
rules on `i2f32(iand(..., 0xff))`), and the `?:` ordered as GLSL's
`conditional_tmp` (nir_fs.rs `order_keys`). `sharedint` is not: its
fragment shader needs `sin` (Mesa's `FSIN_TABLE`, `FCOS_TABLE`,
`FMA_RSCALE` sequence; the Mali compiler has no `sin` lowering) and its
vertex shader an `if` (nir.rs refuses vertex control flow), so `mode !=
0` (`ICMP_OR.u32.ne.m1`) stays the `CompareForm` refusal until a
reference that compiles draws it. The vertex stage still refuses an int
uniform read as a value (`UniformType`).

**State since plan #154 item intbool-2.** `idxtex` (case 100) and
`uniidx_e` (case 116) did not fail on the int uniform after session 1:
the int index was taken, and the refusal was the run-time read of the
vec4 array element (`nir::uniform_idx`, `MaliError::UniformIndexed`), not
`UniformType` as section 3's table says. That read is lowered now in
every stage and both cases are byte-equal (docs/notes/mali-uniform-types.md,
"State since plan #154 item intbool-2"). The run-time signed division of
2.1's "not recorded" line is recorded and lowered: case 79,
docs/notes/mali-int-and-size.md "Since plan #154 item intbool-2".

## The answers

1. **Layout: one FAU word per *read component*, in declaration order, after
   four leading words.** Fragment FAU words 0 to 3 are zero (system values);
   the uniform block's pushed words follow from word 4 (`u2.w0`, `u2.w1`,
   `u3.w0`, ...). A component the shader never reads gets no word
   (`gcops`: `vec4 u` read as `.xyz`, `ivec2 i` as `.x`, `uvec2 w` as `.x`
   gives five words, not eight). Integers are raw bits in the same words as
   floats; only the reading instruction differs. Compiler-made constants
   (reciprocals, `1/255`, masks) follow the uniform words in the same table.
   This is the packing the float-only path already does
   (`src/shaderc/online/mali/fau.rs`).
2. **A `bool` is not recorded.** None of the seven declares one; a `bool`
   uniform is a 32-bit word that Mesa compares `!= 0` (`ICMP_OR.u32.ne`, as
   `sharedint`'s `mode != 0`, below). Nothing here proves the upload value
   (GL writes 0 or 1).
3. **The integer ops Mesa selects** are all single instructions on v10 except
   division and modulo (section 2). They are small: `IADD`, `ISUB`, `IMUL`,
   `IABS`, `CSEL` for min/max and for `?:`, `ICMP_OR` for compares,
   `LSHIFT_OR`/`RSHIFT_OR`/`ARSHIFT_OR` for shifts, `LSHIFT_XOR` for xor,
   `U32_TO_F32`/`F32_TO_U32.rtz`/`U8_TO_F32` for conversions.
4. **The refusal removed by the type alone is small.** Behind
   `UniformType` the next refusal for every shader below is
   `MaliError::IntValue` (`nir_fs.rs:1668`, `nir.rs:1607` for the first
   integer constant or `Arithmetic` for the first integer operator;
   `dynidx.frag` in es100 shows it: `integer or boolean value`). The
   per-shader table in section 3 lists what remains.

## 1. The seven references

Each row: FAU words (`uN.wM`; word index = 2N+M), instructions, and the
program's work registers (`work_reg_count` 32 throughout).

| ref | FAU layout (fragment unless noted) |
|---|---|
| `uops` | `uvec2 u` words 4, 5; `ivec2 i` 6, 7; `float f` 8; constants 9 `0x4f7ffffe` (2^32 - 512 as float, the udiv reciprocal scale), 10 `1`, 11 `0.25`, 12 `0.5`, 13 `1/255` |
| `ishr` | `ivec2 v` words 4, 5; 6 `1/255` |
| `gcops` | `vec4 u` read `.xyz` words 4 to 6; `ivec2 i` `.x` word 7; `uvec2 w` `.x` word 8; constants 9 to 11 (0.114, 0.587, 0.299), 12 `1/255`; `v_t` is a varying (`LD_VAR_BUF_IMM`) |
| `ifelse_uint` | `uvec2 u` words 4, 5; `vec4 a` words 6 to 9; constants 10 to 13 (`0x9249`, `0x4924`, `0xaaab`, `0xaaaa`, the 16-bit magic multipliers of `/7` and `/3`) |
| `intrt` | `uvec4 u_v` read `.xyz` words 4 to 6, no constants; alpha is the pixel column from the preloaded `r59` (`U16_TO_U32 r3, r59.h0`); blend is `BLEND.v4.u32` (the output type is `uint`, see 2.5) |
| `idxtex` | vertex (varying shader): words 0 to 10 are the stage's own (viewport scale, varying buffer offsets 4, 8, 12), `int vidx` word 11 (`u5.w1`), constant `0.25` word 12, texture descriptor handle `0x20001800` word 13; fragment: `int fidx` word 4 (`u2.w0`), constant `0.75` word 5, the handle is an immediate (`IADD_IMM #0x20001800`); `vcols`/`fcols` are *not* in the FAU, they are read by `LD_PKA` |
| `sharedint` | vertex: words 6 to 21 `mat4 matrix` (all 16), 22 to 33 `mat4 rotation` (12 words: `r.xyz` is read, the `w` row is not), 34 `float scale` (`u17.w0`), 35 `int mode` (`u17.w1`), then constants; fragment: `color` 4 to 7 (`rgb` and `a`), `int mode` 8 (`u4.w0`), constants 9 to 11 (2/pi, 786432.0, -pi/2 for the `sin`); `scale` is not read there, so it has no word; `v_xy` is a varying |

The exact word values are in each `expect.json`
(`fau`); with no `--uniform-values` the uniform words are zero, so the layout
is read from the instructions, not from values.

## 2. Selected ops

### 2.1 Division and modulo (`uops`, `ifelse_uint`)

- `uint / uint` with a **run-time divisor** (`uops`: `u.x / u.y`, `u.x % u.y`):
  Mesa's NIR lowering `nir_lower_idiv` (the "fast" path) gives about 46
  instructions per divide: `U32_TO_F32`, `FRCP.f32`, `FMA` by `0x4f7ffffe`
  (word 9), `F32_TO_U32.rtz`, a 32x32 multiply built from `U16_TO_U32` halves
  and `IMUL.i32`, `ISUB`, then **two correction steps** each
  `ICMP_OR.u32.ge.m1` + `CSEL.u32.eq` (`+1` from word 10) + `ISUB`. `uops`
  holds two of them (the divide, then the modulo's own) in 93 instructions.
- `uint / constant` and `uint % constant` (`ifelse_uint`: `% 7u`, `/ 3u`):
  no float: **multiply by a 16-bit magic number** split in two halves
  (`U16_TO_U32`, `IMUL.i32`, `IADD.s32`, constants `0x9249/0x4924` for 7,
  `0xaaab/0xaaaa` for 3), `RSHIFT_OR` by the shift, `IMUL` back and `ISUB` for
  the remainder (`% 7u`: `RSHIFT_OR ... 0x1000000.b33`, `IMUL ... 0x7060504.b3`
  = 7). 55 instructions for the whole program.
- A signed divide is not in these programs. Not recorded.

### 2.2 Shifts, bitwise, not

`>> 3u` is `RSHIFT_OR.i32 r, x, 0x3020100.b33, 0x0`: the shift amount is a
**byte select of an immediate constant word** (`.b33` selects byte 3 of
`0x3020100`, i.e. 3). A run-time amount is a register with `.b00`
(`ishr`: `ARSHIFT_OR.i32 r0, u2.w0, r0^.b00, 0x0` after `MOV r0, u2.w1`, the
amount copied to a temporary because only a register or a constant takes the
byte select). Left shifts are `LSHIFT_OR.i32 ..., 0x0` (the `OR` operand is 0);
`^` is `LSHIFT_XOR.i32 r, a, 0x0.b00, b` (shift by 0, xor b): there is no plain
`LOP`. `~x` is `LSHIFT_OR.i32 r, 0x0, 0x0.b00, x.not` (the `.not` modifier on the
third source). `&` and `|` with a constant mask are not separate instructions
when folded into the next conversion: `& 255u` is `U8_TO_F32 r, r.b0` and
`(x >> 8) & 255u` is `U8_TO_F32 r, r.b1` (byte select of the source), so the
mask and the conversion are one op. A general `&` was not recorded.

### 2.3 min, max, abs, compare, select

- `min`/`max` (signed and unsigned): `CSEL.<u32|s32>.lt` / `.gt r, a, b, a, b`
  (select a if `a < b`, else b).
- `abs(int)`: `IABS.s32` (`gcops`).
- Compare as a value: `ICMP_OR.<u32|s32>.<lt|ge|ne>.m1 r, a, b, c` (`m1`: the
  result is a mask, all ones or zero, the third source is OR-ed in) followed
  by `CSEL.u32.eq r, mask, 0x0, <false>, <true>` to turn it into a float or
  integer (`ishr`: `s < 0 ? 1.0 : 0.0`; `uops`: `(u.x < u.y ? 0.25 : 0.0) +
  (u.x <= u.y ? 0.5 : 0.0)`). `u.x <= u.y` is `ge(u.y, u.x)`, swapped operands.
  `!= 0` (a `bool`, an `int` as a condition) is `ICMP_OR.u32.ne.m1 r, x, 0x0, 0x0`
  (`sharedint`).
- `if (u.x < u.y) ... else ...` on two sides without side effects
  (`ifelse_uint`) is **both sides computed and one `CSEL.u32.eq` per
  component**; no branch (the programs have no branch).
- `a * b` is `IMUL.i32`; `+` `IADD.s32`; `-` `ISUB.s32`; `uint <-> int`
  casts are no instruction (`uops`: `uint(mn)` is the same register).

### 2.4 Conversions

`uint(f)`: `F32_TO_U32.rtz`; `float(u)`: `U32_TO_F32`; the `/ 255.0` of the
colour is a multiply by the FAU constant `3b808081` (`1/255`), as in float code
(`FMA.f32 r, r, u6.w1, 0x0.neg`).

### 2.5 `intrt`: an integer render target

The output is `uvec4`; Mesa writes the **raw words** with `MOV.i32` and ends
in `BLEND.slot0.v4.u32` (type `u32`, vs `v4.f32` for a float target). The
blend descriptor of the frame must then be an integer format (here the
64x64 `gl_ref.py` target is still RGBA8, so the shader program is
independent of the target's bytes). `gl_FragCoord.x` as an integer is
`U16_TO_U32 r3, r59.h0` from the preloaded register.

### 2.6 `idxtex`: the indexed array

A run-time-indexed `vec4 cols[4]` is not pushed: the shader keeps the
**index in the FAU** (`vidx`, `fidx`) and loads the elements with
`LD_PKA.i64.unsigned` (a pair of components, `.rg` in the vertex stage, `.ba`
in the fragment stage) at byte offset `idx << 4` (`LSHIFT_OR ... 0x7060504.b00`
= 4 for the vertex stage: shift by the `4` of byte 0). In the fragment stage
the offset starts at `idx << 4` plus `IADD_IMM #0x8` for the `.ba` pair. The
texture look-up uses the pushed descriptor handle `0x20001800`
(`TEX_SINGLE`, `2d.zero` in the vertex stage, `2d.computed` in the fragment
stage). Details are in `docs/notes/mali-uniform-index.md`; this adds only the
int index being a raw FAU word shifted into the offset.

### 2.7 `sharedint`: one struct with an int member

All members of `Params` (`mat4`, `mat4`, `vec4`, `float`, `int`) are in one
struct uniform: Mesa gives each *read* member its own words (section 1), so the `mode != 0` branch is one `ICMP_OR.u32.ne.m1` and two
`CSEL`s (vertex: the rotated `position`; fragment: the colour), no branch. The
`mat4` reads are the same `FMA.f32` chains as the float-only path. Nothing is
special about a struct holding an `int`: it is the `int` that raises the
refusal (the corpus lines `sharedint.vert`, `sharedint.frag`).

## 3. How much of the refusal each lowering removes

| shader | after `UniformType` is lowered | also needed |
|---|---|---|
| `es300/ishr.frag` | `IntValue`: the shift, compare, `&`, `/ 255.0` of ints | nothing else; ARSHIFT/ICMP/CSEL/`U8_TO_F32` (2.2, 2.3) |
| `es300/uops.frag` | `IntValue`, then `Arithmetic` for `/`, `%`, `min`, `max`, `~`, `^`, shifts | integer divide (2.1), the rest of 2.2 and 2.3 |
| `es300/gcops.frag` | `IntValue`, `Arithmetic` for `abs`, `^` | `IABS`, `LSHIFT_XOR`, `floor`, `sqrt`, `dot` already float ops |
| `es300/ifelse_uint.frag` | `IntValue`, `Arithmetic` for `%`, `/` | constant divide (2.1), compare, per-component `CSEL` |
| `gltest/intrt.frag` | `IntValue` (the `uvec4` output) | an integer colour output and `BLEND.v4.u32` (2.5), `gl_FragCoord` as int; the frame must also allow an integer target |
| `es300/idxtex.{vert,frag}` | `UniformArray` (the vertex stage), then the texture look-up refusals | a run-time index (2.6, `mali-uniform-index.md`) |
| `es100/sharedint.{vert,frag}` | `IntValue` in `mode != 0`, nothing else | `ICMP_OR.ne` + `CSEL` (2.3); the fragment's `sin` is the existing float lowering |
| `es100/uniidx_e.vert` | `UniformArray` then the index word | as `idxtex` (2.6) |
| `es100/ifelse_loop.frag`, `es100/light_amount_2_2.vert`, `es300/dynidx.frag`, `es300/tmpidx.frag`, `es300/uniidx.frag`, `es300/texfetch.frag` | not recorded here | loops with an `int` counter, arrays of structs, `mat3`, `texelFetch` are other studies' (`acceptcheck`, `study-texfetch`) |

Reading the list: the **type refusal itself is a one-line gate** in
`uniform_of` and `uniform_extent` (it hides 17 rows), but only `sharedint`
(both stages) and `uniidx_e.vert`'s index word are one op past it. The six
fragment programs (`ishr`, `uops`, `gcops`, `ifelse_uint`, `intrt`) need the
integer ALU of section 2 behind the gate, so lowering `int`/`uint`/`bool`
uniforms alone moves two corpus rows (`sharedint.vert`, `sharedint.frag`) to
`ok` and turns the rest into `IntValue` or `Arithmetic` refusals.

## 4. Suggested order

1. Let a uniform of type `int`/`uint`/`bool` (and `ivec2`/`uvec2`/`uvec4`) take
   one FAU word per read component, as a float (`UniformType` gate only).
2. `ICMP_OR.<u32|s32>.<ne|lt|ge>.m1` plus `CSEL.u32.eq` (selects, comparisons,
   `bool`): `sharedint`, the float-select part of `ifelse_uint`.
3. `IADD.s32`, `ISUB.s32`, `IMUL.i32`, `IABS.s32`, min/max by `CSEL`, shifts
   and xor/not by `LSHIFT_*`/`RSHIFT_OR`/`ARSHIFT_OR` (2.2): `ishr`, `gcops`.
4. Constant `/` and `%` by magic multiply, then the run-time divide (2.1):
   `ifelse_uint`, `uops`.
5. Integer render target (`intrt`): output type `u32` plus the frame's colour
   format.
6. The `idxtex` index is `mali-uniform-index.md`'s step.

`bool` and signed divide have no reference yet; record one before lowering
them.
