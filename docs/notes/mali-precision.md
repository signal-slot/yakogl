<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall precision: Mesa's width rule, a precision per IR value, and the case notes (19, 82, 84, 119)

Parts A to E are the design: how Mesa 26.2.1 decides the width (16 or 32
bits) of every value of a panfrost (Valhall) shader, what our front end and
Mali back end know of it today, the change that makes the precision of every
IR value known once, the order of work, and the open questions. Parts 1 and
2 below them are the case notes of cases 19, 82, 84 and 119.

Citations: `G/` is Mesa's `src/compiler/glsl/`, `N/` is
`src/compiler/nir/`, `B/` is `src/panfrost/compiler/bifrost/`, `P/` is
`src/gallium/drivers/panfrost/`, all Mesa 26.2.1. Our files are under
`src/shaderc/online/` (`mali/` for the back end), at origin/main
`234b9d4a7` unless a branch is named.

A line marked **[probe pN]** is confirmed by a draw on the drm-shim
reference arm (Mesa 26.2.1, GPU id 0xa867), the NIR read from the run's
`.stderr`; **[ref X]** by a committed reference `tools/mali/testdata/X`;
**[read]** is read from the source only. The probe shaders are in A.9.

## Part A. Mesa's rule

### A.1 Where it runs

- `lower_precision` runs right after `ast_to_hir`, before function
  inlining and before any GLSL optimisation, for an ES shader of a stage
  whose screen has the `fp16` or `int16` cap
  (`G/glsl_parser_extras.cpp:2487-2491`). [read]
- Panfrost sets, for every stage (`P/pan_screen.c:548`, the loop over all
  stages): `fp16` and `glsl_16bit_consts` on, `fp16_derivatives` and
  `fp16_const_buffers` on from arch 6, `glsl_16bit_load_dst` on, `int16`
  off (`P/pan_screen.c:585-591`). Only the debug flag `PAN_DBG_NOFP16`
  turns fp16 off (`:546`). So **the vertex and the fragment stage follow the
  same rule**. [read; vertex fp16: ref `vsmp_*` on branch `mali-vs-int`,
  probes p2, p3b, p6b]
- `int16` off means **no int or uint value is ever 16-bit**
  (`can_lower_type`, `G/lower_precision.cpp:153-182`). Bools and samplers
  take part in the rule (`:167-170`).

### A.2 The declared precision of a variable

- A declaration's precision is its qualifier, else the default in force in
  its scope (`select_gles_precision`, `G/ast_to_hir.cpp:2764-2800`;
  variables `:4224-4225`, function return types `:6427`, struct members
  `:8096`).
- Defaults (GLSL ES 1.00 4.5.3): vertex float and int highp; fragment int
  mediump, float none (the shader must declare one); `sampler2D` and
  `samplerCube` lowp in both stages. `reflect.rs:848-863` has the same
  table.
- Built-in variables (`G/builtin_variables.cpp`): `gl_Position` highp
  (`:1576`); `gl_PointSize` mediump in ES 1.00, highp in 3.00
  (`:1587-1591`); `gl_FragCoord` mediump in 1.00, highp in 3.00
  (`:1333-1335`); `gl_PointCoord` mediump (`:1355-1358`); `gl_FragColor`
  and `gl_FragData` mediump (`:1372-1377`); `gl_DepthRange.near/far/diff`
  highp (`G/builtin_types.cpp:37-41`); `gl_FrontFacing` a bool with no
  precision. Built-in int constants (`gl_MaxDrawBuffers`) are ints, so
  never 16-bit.

### A.3 The class of each expression (`find_lowerable_rvalues`)

Each expression node gets one of three states (`G/lower_precision.cpp:
295-312`): **High** (CANT_LOWER), **Medium** (SHOULD_LOWER), **None**
(UNKNOWN). A node's state is joined from its children's unless the edge is
independent (`pop_stack_entry`, `:224-283`): a High child makes it High; else
a Medium child makes it Medium; else it stays None.

| Kind of expression | Class | Source |
|---|---|---|
| float or bool constant | None | `:336-346` [probe p3b, p6b] |
| int or uint constant | High | `:339-340` |
| read of a variable, a struct member, an array element | the declared precision (lowp and mediump: Medium; highp: High; none: None) | `:349-382` |
| read of an int, uint or struct-typed value | High | `can_lower_type` |
| read of a `const` variable | the declared precision, as any variable | [probe p1, p1b, p1c, p1d] |
| array index, texture coordinate, lod, bias | a separate expression (independent edge), not joined with its parent | `get_parent_relation`, `:315-333` |
| texture look-up | the sampler's precision: lowp default, so Medium | `:384-394`, `handle_call` `:477-487` [probe p4] |
| arithmetic, compare, swizzle, `b2f`, `not`, `any`, `all` | the join of the operands; an int or uint result type is High (`int(f)`) | `:397-405` |
| `float(i)` of an int | High (its operand is an int) | `:397-405` |
| call of a user function | the function's declared return precision | `:461-463` [probe p2] |
| call of a built-in with no fixed return precision | Medium when every non-constant argument is a Medium root, else High | `:489-541` |
| built-ins `floatBitsTo*`, `*BitsToFloat` | High | `:495-500` |
| compiler temporary (a constructor's `vec_ctor`, a `?:`'s result, a call's return value) | None until assigned; the first Medium assignment makes it Medium; any non-constant assignment that is not Medium makes it High | `visit_leave(ir_assignment)`, `:585-608` [probe p3, p3b, p6a, p6b] |
| the target of an assignment | High (never part of a lowered tree) | `stack_enter`, `:208` |
| a derivative (`dFdx`) | joined as any operation (`fp16_derivatives` is on) | `:404-415` |

The body of a user function is classed with its own declarations: an `in`
parameter is read at the parameter's precision, a local at its own. The
call result's class does not depend on the body (probe p2: a highp body
`x * x` returning `mediump float` is `fmul` at 32 bits, `f2f16`, then the
caller's `* 1.5` at 16 bits).

### A.4 Which nodes are computed at 16 bits

- A **root** is the topmost Medium node of a chain of joined nodes (a
  Medium node whose parent is High, or whose edge to the parent is
  independent, or which has no parent) (`:252-282`).
- The root and every node below it on joined edges, None nodes included
  (its constants, a `b2f`), are retyped to 16 bits
  (`lower_precision_visitor`, `:724-756`). A float constant becomes its
  fp16 value (`_mesa_float_to_half`, `:744`); `b2f` becomes `b2f16`
  and `f2b` becomes `f162b` (`:801-806`).
- Each read below the root gets `f2fmp` (`convert_precision`, `:683-720`,
  called `:729-731`); the root gets `f2f32` unless its type is bool
  (`:853-856`). A root that is a bare read gets nothing (`:842-843`).
- A lowered built-in call is inlined with a lowered copy of its body whose
  unqualified parameters are mediump (`map_builtin`, `:896-930`).

### A.5 Which variables are stored at 16 bits (`lower_variables`)

- Stored at 16 bits (`:1023-1060`): a mediump or lowp float local or global
  (`ir_var_auto`, a `const` one too), a compiler temporary that A.3 made
  Medium, and a mediump or lowp float uniform outside a block when
  `fp16_const_buffers` is on (arrays of them too; not a struct, whose base
  type is not float).
- Stored at 32 bits: attributes, varyings in and out, function parameters
  (`in`, `out`, `inout`), every int, uint and bool.
- A store into a 16-bit variable converts the value (`f2fmp`); a 16-bit
  variable read by a 32-bit use goes through a 32-bit temporary (`f162f`)
  (`:1115-1194`, `:1233-1281`); passing one to a 32-bit parameter converts
  both ways (`:1283-1352`).
- Storage alone rounds nothing: NIR removes `f2f32(f2fmp(a@32))`
  (`N/nir_opt_algebraic.py:1991`). A mediump local that only carries an
  fp32 value to a highp use keeps it exact [probe p3, p1c, p1d]. It rounds
  where a lowered operation reads it, and where the value crosses control
  flow (a 16-bit phi, which no fold removes) [read].

### A.6 NIR rules that move widths afterwards

- `f2fmp(f2f32(a@16))` is `a` (`N/nir_opt_algebraic.py:1971`);
  `f2f32(f2fmp(a@32))` is `a` (`:1991`); `f2fmp(i2f32(a))` is `i2fmp(a)`
  (`:1322`); `f2fmp(b2f32(a))` is `b2f16(a)` (`:1980`); `f2i32(f2fmp(a))`
  is `f2i32(a)` (`:2002`).
- A contracted `fadd` or `fmul` used once, of `f2f32(a@16)` and an fp16
  representable constant (or of two such conversions), under `f2fmp`, is
  the 16-bit operation (`:2054-2060`). [read]
- The late rules (`nir_opt_algebraic_late`, run by `B/bifrost_nir.c:
  336-345` and `:363-370`): a unary, binary or ternary float operation whose
  every operand is an `f2fmp` and whose result goes to `f2f32` is done at 32
  bits, and so is a compare of two `f2fmp` (`:4164-4186`). Then every
  remaining `f2fmp` is `f2f16` (`:4191-4193`). [ref `vsstruct_pair`,
  `vsmp_missing_returns` on branches `mali-uniforms`, `mali-vs-int`]
- `bi_lower_bit_size` (`B/bifrost_nir.c:35-80`): `fexp2`, `flog2`, `fpow`,
  `fsin`, `fcos` run at 32 bits with conversions around them; the rounding
  ops (`ffloor`, `fceil`, `fround_even`, `ffract`, `ftrunc`) stay 16-bit
  below arch 11 (the G610 is arch 10).
- Bools take the width of the compare that made them
  (`pan_nir_lower_bool_to_bitsize`, `B/bifrost_nir.c:350`); a 32-bit
  compare's bool used by a 16-bit select is `i2i16` of it [probe p6b].

### A.7 Inputs, outputs, uniforms

- Vertex outputs (`gl_Position`, varyings, `gl_PointSize`) are stored at 32
  bits: the vertex varying layout is collected with `lower_mediump` false
  (`P/pan_shader.c:583-588`), and `nir_lower_mediump_io` runs for the
  fragment stage only (`B/bifrost_nir.c:912-916`). A mediump result is
  `f2f32` before the store [probe p2, p4].
- Fragment inputs and outputs of mediump or lowp precision are 16-bit
  (`nir_lower_mediump_io`, `B/bifrost_nir.c:914-916`), except a flat input
  and a varying whose x and y feed a texture coordinate as one load
  (`bi_fp32_varying_mask`, `bi_gather_texcoords`, `:450-500`). A highp
  input is 32-bit; a highp ES 3.00 output is stored at 32 bits.
- Uniforms: A.5. The FAU layout of fp16 uniforms is Part 1 below.

### A.8 The rule as a compiler writer applies it

1. Resolve each declaration's precision (A.2).
2. Class every expression node, bottom up, before inlining (A.3).
3. Mark the roots and their joined subtrees as 16-bit, with `f2fmp` on each
   read and `f2f32` on each non-bool root (A.4).
4. Give each variable its storage width (A.5); convert at stores and reads.
5. Fold the conversions (A.6, the order of `nir_opt_algebraic`, then the
   late rules), then apply `bi_lower_bit_size` and the bool widths.
6. Apply the stage's IO widths (A.7).

What a value's width is therefore depends on two things a per-shader flag
cannot hold: the class of the operation that makes it (step 2, which needs
the source's scopes and the call boundaries) and the conversions around it
after the folds (step 5).

### A.9 The probes

Each vertex shader below has `attribute highp vec4 p;` and writes
`gl_Position = p;`; `a` and `b` are `attribute highp float`. The fragment
shader draws the varying (`precision mediump float; varying highp float v;
gl_FragColor = vec4(v, 0.0, 0.0, 1.0);`, a `vec2` for p3 and p3b). Run as
`tools/mali/drm_shim_ref.sh run $W pN.vert fs.frag pN` (`$W` a shim
install). They are not committed (step D.3 commits them as references).

| Probe | Vertex shader body | Mesa's NIR (vertex) |
|---|---|---|
| p1 | `const mediump float c = 1.3;` global, `v = c * c;` | the varying folds to a constant the link moves to the FS: `0x3ec2`, the fp16 product of fp16 1.3 (the fp32 product would be `0x3ec3`) |
| p1b | `mediump float c = 1.3;` local, `v = c * c;` | the same `0x3ec2` |
| p1c | `const mediump float c = 1.3;`, `v = c * a;` | `fmul 0x3fa66666, a` at 32 bits: the constant not rounded |
| p1d | `mediump float c = 1.3;` local, `v = c * a;` | the same as p1c |
| p2 | `mediump float g(highp float x) { return x * x; }`, `v = g(a) * 1.5;` | `fmul` 32, `f2f16`, `fmul` 16 by `0x3e00`, `f2f32` (`FMA.f32`, `FADD.f32 .h00`, `FMA.v2f16`, `F16_TO_F32`) |
| p3 | `mediump float m = a; v = vec2(m, b) * 1.5;` | two `fmul` 32: the constructor is High (b is highp), m's storage folds away |
| p3b | `mediump float m = a; v = vec2(m, 2.0) * 1.5;` | `f2f16 a`, `vec2 16`, `fmul 16x2`, two `f2f32` |
| p4 | `uniform sampler2D s;` (lowp by default), `v = texture2D(s, p.xy).x * 1.5;` | `(float16)txl`, `fmul` 16, `f2f32` |
| p6a | `mediump float m = a; v = ((b > 0.5) ? m : b) * 1.5;` | `flt` 32, `bcsel` 32, `fmul` 32 |
| p6b | `mediump float m = a; v = ((b > 0.5) ? m : 2.0) * 1.5;` | `flt` 32, `i2i16`, `bcsel` 16, `fmul` 16, `f2f32` |

## Part B. What we have

### B.1 Where the front end drops the precision

- The parser keeps it: `ast::Precision` (`ast.rs:46-60`), `TypeSpec::
  precision` (`ast.rs:175-179`), `Decl::Precision` (`ast.rs:537-538`;
  `parser.rs:362-373`, `:865-881`).
- The type checker only checks it: `Sema::default_float_precision` is one
  flag (`sema.rs:256-268`), and it keeps no default-precision stack per
  type (`sema.rs:2316-2324`).
- `reflect.rs` resolves defaults for top-level declarations only
  (`uniform_decls`, `reflect.rs:938-995`; `resolved`, `:848-863`), and
  gives each uniform its precision (`reflect::UniformDecl::precision`,
  `:834-843`).
- The lowering (`lower.rs`) reads no precision: its `Local` (`lower.rs:
  1100-1133`) has none, an `in` parameter its body never assigns is an
  alias of the argument's value (`Local::alias`, `:1208-1210`, used
  `:4370-4378`), so the parameter's precision boundary has no IR node, and
  every instruction is pushed by `push_sized` (`:8529-8537`), which records
  the result width (`Module::comps`) and nothing else.
- The IR has no precision anywhere: `ir::Type` is a scalar kind and a lane
  count (`ir.rs:55-75`); `VarDecl` (`:656-662`), `IoDecl` (`:590-611`) and
  `ir::UniformDecl` (`:630-652`) have none. `Module::comps` (`:674-685`)
  is the one per-value side table, and it is the model for Part C.

### B.2 What the Mali back end receives and guesses

The back end gets the `Module`, the source text and `reflect`'s uniform
list. Every width decision is derived again, from the source or per shader:

| Where | What it decides | How |
|---|---|---|
| `mali/mod.rs:943-1052` `FsPrecision`, `of` | the fragment shader's width | re-parses the source; `highp` = the default float precision at `main`, one width for the whole shader |
| `FsPrecision::mixed` (`:948`) | a declaration of the other width anywhere | refusal `MixedPrecision` (`mod.rs:1303-1305`, `:1395-1397`; `nir_fs.rs:2786`, `:4356`, `:4444`) |
| `FsPrecision::other` (`:951`) | the varyings of the other width | `nir_fs.rs:2743`, `:4179` |
| `FsPrecision::highp_samplers` (`:955`) | a highp sampler's look-up is 32-bit | set from `reflect` (`mod.rs:1296-1299`), read `nir_fs.rs:4826` |
| `FsPrecision::colour_f32`, `colour_u32` (`:960`, `:965`) | the colour's store width | source declarations |
| `FsPrecision::frag_coord_highp` (`:970`) | `gl_FragCoord`'s width | the version (`:1003`) |
| `nir_fs.rs` `walk_nodes` (`:4238-4330`, `:4321`) | each value's width | `prec.highp` per shader, then islands patched: `Fs::widen` at stores to highp locals, `F2f16` of `gl_FragCoord` values, texel widths |
| `nir_fs.rs:3851-3852` | a loop kept in a mediump shader | refusal `LoopMediump` |
| `mod.rs:1207-1211` closure `fp16`, `nir.rs:1020-1021` `fp16_uniform` | a vertex uniform's storage width | `reflect`'s precision |
| `mod.rs:1219-1226` | a vertex uniform not highp and not lowered | refusal `UniformPrecision` |
| `nir.rs` `build` | every vertex value | fp32, except the fp16 uniform reads widened at once |

On the unlanded branches (read with `git show <branch>:<path>`):

| Branch | Addition | What it guesses |
|---|---|---|
| `mali-vs-int` | `mali/vs_precision.rs` (new), `MaliError::VertexMediump`, called at `mod.rs:1582`, `:1659` | re-implements A.3 on the source and refuses every vertex shader with a Medium operation; reads a `const` variable as having no precision (`vs_precision.rs:87-96`), which probes p1 and p1c show is not Mesa's rule |
| `mali-uniforms` | `nir::mediump_operation` (`nir.rs:1096-`), `MaliError::VsFp16Operation` (`mod.rs:1287`), `FsPrecision::vertex` (`mod.rs:1005-`) | refuses a Medium operation on fp16 uniforms or struct members, except the scalar `fadd`/`fmul` of two members that the late rule puts back at 32 bits; refuses (`UniformPrecision`, `mod.rs:1284`, `:1313`) a mediump member in a shader that declares any mediump value |
| `mali-fs-int` | `FsPrecision::float_returns` (`mod.rs:979`, read `nir_fs.rs:4533-4537`) | a function returning a float hides where Mesa narrows an int's fp32 conversion |
| `mali-vs-varyings` | `FsPrecision::colour_int` (`mod.rs:969`) | an integer colour output (a type, not a precision) |
| `mali-fs-colour` | `906f36822`: the fragment walk passes on a lane past a value's width | the front end records no width for a built-in's result or a `++`/`--` sum (`Sink::emit` uses `push_instr`, `lower.rs:8551-8555`; `step_by_one`), so `comps_of` answers 4 |
| `origin/mali-fs-alu` | no new precision code | |

## Part C. The design

### C.1 Where and what

**The front end computes Mesa's class once, while it lowers, and records it
in side tables of the `Module`.** Only `lower.rs` has what A.3 needs at the
same time: the declarations of each scope, each user function's return
precision and parameters before inlining, and the constructors and `?:`
before they become lanes. A pass after it cannot see those boundaries
(B.1: aliases, inlined calls, folded constants).

New in `ir.rs` (`Module`, next to `comps`):

```rust
/// GLSL ES precision lowering's verdict for one instruction (Mesa's
/// lower_precision.cpp), recorded by the front end; no back end must
/// read it but the one that lowers precision.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Prec {
    /// Not recorded, or a value with no float or bool width (an int).
    #[default]
    None,
    /// Computed at full width.
    High,
    /// Inside a lowered tree (A.4): computed at 16 bits before NIR's folds.
    Medium,
}

pub struct Module<'a> {
    // ... as now ...
    /// [`Prec`] of each instruction, by `ValueId` as `comps`. For a
    /// `StoreOutput`, the output's declared precision.
    pub prec: &'a [Prec],
    /// Whether each local is stored at 16 bits (A.5), by `LocalId`.
    pub local_prec: &'a [Prec],
}
```

with `Module::prec_of(id)` and `Module::local_prec_of(l)` answering `None`
for an empty table, as `comps_of` answers for `comps`.

In `lower.rs`:

1. A default-precision stack per scope for float and int, pushed and popped
   with the scopes `lower.rs` already keeps (`reflect.rs:848-863`'s
   `resolved` for each declaration).
2. `Local` gets the declared precision. An alias (`Local::alias`) keeps the
   parameter's precision, so a read of it is classed at the parameter's.
3. For each expression `value(e)` lowers, the A.3 class is computed from
   its AST children (the same walk `vs_precision.rs` does on the branch,
   moved here and given the scopes), and A.4's "lowered" bit is passed down
   to the instructions it emits: `push_sized` takes it and pushes `prec`
   beside `comps`. A built-in expansion (`builtin_call`, `Sink::emit`)
   pushes the call's bit for every instruction of the expansion.
4. A constructor and a `?:` are classed by A.3's temporary rule; a user
   call's result by the return precision; a look-up by its sampler.
5. `local_prec` is A.5 for each slot: Medium for a mediump or lowp float
   local or global, and for a temporary A.3 made Medium; High for a slot of
   an inlined parameter (`inline_with_slot`).
6. `relocate` (`lower.rs:631-`) moves the two new slices like the others.

**The Mali back end turns classes into widths** in one new pass,
`mali/width.rs`, run once before either walk. It is A.6 and A.7 on the IR:
for each value a width (16 or 32), and for each use a conversion
(`F2f16`, `F2f32` or none). It models `f2fmp`/`f2f32` at the roots, the
storage conversions of `local_prec`, the folds of A.6 in NIR's order, the
late rules, `bi_lower_bit_size`, the bool widths and the fragment IO widths
(`bi_fp32_varying_mask`). The walks then read `width.bits(id)` and
`width.conv(use)` instead of `FsPrecision::highp` and the vertex fp32
default. These rules are Mesa NIR's and panfrost's, so they live in the
Mali back end, not in the shared front end.

### C.2 The other back ends stay byte-identical

- The instruction stream, `comps`, `nodes` and the declarations do not
  change. The new tables are read by `mali/` only; `gpu-v3d`,
  `gpu-vivante`, `gpu-powervr` and the offline table never read them.
- The cost they see: two more arena slices per compile (one byte per
  instruction, one per local) in the shared scratch arena (`store.rs`
  `with_scratch`). A shader near the arena's limit could meet
  arena exhaustion sooner (open question E.6).
- `Module` gains two fields: every `Module { .. }` literal (`Module::empty`,
  `ir.rs:706-721`, and any in `tests/` or a back end's tests) gains them;
  grep `'Module {'` in `src` and `tests` before the change lands.
- `lower.rs` and `ir.rs` reach every row of `tools/check.sh`, so step D.2
  runs the whole gate; no generated reference may move.

### C.3 What each guess becomes

| Today | Becomes |
|---|---|
| `FsPrecision::highp`, `mixed` | `width.rs`: each value its own width; the per-shader paths of `nir_fs.rs` (`select` vs `select_highp`) are chosen per node by `bits` |
| `FsPrecision::other` | the class of the `LoadInput` plus A.7's flat/texcoord rule in `width.rs` |
| `FsPrecision::highp_samplers` | the class of the `Tex` value |
| `FsPrecision::colour_f32` | the `prec` of the `StoreOutput` |
| `FsPrecision::colour_u32`, `colour_int` | stay: they are the output's type (`IoDecl::gl_type`), not a precision |
| `FsPrecision::frag_coord_highp` | the class of `LoadInput { slot: FragCoord }` (the front end knows the version) |
| `FsPrecision::float_returns` (`mali-fs-int`) | gone: the call result's class is recorded |
| `FsPrecision::vertex` (`mali-uniforms`) | gone |
| `FsPrecision::consts` | stays (the link's constant lanes, not a precision); `FsPrecision` keeps only it and the type flags |
| `nir::fp16_uniform`, closure `fp16` | stay as the uniform's storage width (A.5); read from `reflect`'s precision as now |
| `nir::mediump_operation` (`mali-uniforms`) | gone: `width.rs` gives each operation's width, the late rule included |
| `vs_precision.rs`, `VertexMediump` (`mali-vs-int`) | gone; its A.3 walk moves into `lower.rs` (with the `const` fix of B.2) |
| `MixedPrecision`, `VsFp16Operation`, `UniformPrecision` (the precision part) | gone once the walks build the 16-bit forms; a form with no reference stays a refusal under a narrower name |
| `LoopMediump` | `local_prec` gives the phi width; the refusal stays until the walk carries fp16 values across a back edge |
| the result-width defect (`906f36822`'s pass-through) | the front end records the width (`push_sized` in `builtin_call` and `step_by_one`), step D.1 |

### C.4 Rejected

- **One precision per shader** (today's `FsPrecision::highp`, the
  `VertexMediump` refusal): cannot express a highp operation beside a
  mediump one, which A.3 makes common; it can only refuse them.
- **The Mali back end re-parses the source** (`FsPrecision::of`,
  `vs_precision.rs`): an AST node cannot be matched to an IR value after
  inlining, aliasing, folding and built-in expansion, so it can refuse but
  not lower.
- **A 16-bit scalar kind in `ir::Type`** (`ScalarType::F16`): every back end
  matches on `ScalarType`; the three that do not lower precision would all
  change, and the NIR folds would sit in the shared IR.
- **Explicit conversion instructions in the IR** (`f2fmp`, `f2f32` ops):
  the other back ends walk the instruction stream, so their code or bytes
  would change.
- **The front end computes final widths** (A.6 included): the folds and
  `bi_lower_bit_size` are NIR's and panfrost's; another GPU would need other
  rules, so they belong to `mali/`.

## Part D. The order of work

Counts are dEQP-GLES2 cases whose **first** refusal is the family, from the
host forecast (`tools/mali/gles2-forecast.sh` on the `mali-forecast`
branch, run at `ada67285d`, file `cases.tsv`). A lowered family moves its
cases to their next refusal, so a count is the most a step can gain. None
is a board result.

| Step | Lands alone with | Opens | Cases |
|---|---|---|---|
| D.1 result width | `lower.rs`: `push_sized` with the result type's width in `builtin_call`'s `Sink::emit` and in `step_by_one`; a front-end test that `comps_of` is 2 for `abs(vec2)` and for `--` on a `vec2` | the root of `906f36822`'s pass-through | 271 cases left `ColourForm` with `906f36822` (measured on `mali-fs-colour`); no new case by itself |
| D.2 the tables | `ir.rs` `Prec`, `prec`, `local_prec`; `lower.rs` C.1 items 1-6; front-end tests that assert the class of each form of A.3 (the A.9 shaders and the `vsmp_*` forms) | nothing (no reader); the whole gate, every back end byte-identical | 0 |
| D.3 `mali/width.rs` | the pass and its tests: for each probe of A.9 (committed as `tools/mali/testdata/precrule_*` with their `.expect.json`) the width of each value and each conversion, read from the probe's NIR | nothing (no walk reads it yet) | 0 |
| D.4 vertex walk on widths | `nir.rs` `build` builds 16-bit nodes and `F2f16`/`F2f32` from `width.rs`; `select.rs` `vs_variant` gains the `v2f16` rows the fragment side has; `vs_precision.rs` and `mediump_operation` go | `VertexMediump`, `VsFp16Operation`, `UniformPrecision` | 3,613 `VertexMediump` (measured on `mali-vs-int`, of which 691 came from `IntValue`); 79 `UniformPrecision`; `VsFp16Operation` not measured as a family (101 stages moved on `mali-uniforms`: 80 `UniformPrecision`, 21 `VsFp16Operation`) |
| D.5 fragment walk on widths | `nir_fs.rs` chooses per node from `width.rs`; `FsPrecision` keeps `consts` and the type flags | `MixedPrecision`; then `LoopMediump` with 16-bit phis | 304 `MixedPrecision` (all fragment: 112 `texture.mipmap.*`, 186 `uniform_api.*`, 6 `shaders.*`); 96 `LoopMediump` |

References each step needs (shader text, Mesa 26.2.1 drm-shim):

- D.2: none (front-end tests on source text).
- D.3: the ten probes of A.9; plus a `?:` whose first branch is highp
  (`? b : m`), a mediump `out` parameter, a lowered built-in with a
  sequence (`normalize(m)`, `length(m)`), and the A.6 contraction form
  (`mediump float m = x + y;` read by `m * 2.0`), for E.1-E.4.
- D.4: the `vsmp_*` set of `mali-vs-int` (if_else, return_in_else, vec3,
  in_float, missing_returns, vec3_highpu, if_else_highp); `vsstruct_*` and
  `vsneg_*` of `mali-uniforms`; dEQP
  `operator.binary_operator.add.mediump_vec4_vertex` (the late rule);
  `es100/attrib_hole_float.vert` (`V2F32_TO_V2F16` + `FADD.v2f16`); p2,
  p3b, p4, p6b.
- D.5: the `mixprec_*` references (`docs/notes/mali-mixed-precision.md`);
  `prec_82`, `prec_84`; the fragment shaders of gl_test cases 226 and 227
  (issue #4496); the forecast's `MixedPrecision` example
  (`int g = (ivec4(e, true, h, true)).x; gl_FragColor = vec4(-5.1, f, e,
  int(d)) - c * b;`); one `texture.mipmap.2d.generate` and one
  `uniform_api.value.initial.render.basic` fragment shader; a kept loop of
  a mediump `vec4` for `LoopMediump`.

What can run at the same time:

- D.1 and D.2 both edit `push_sized`'s callers in `lower.rs`
  (`Sink::emit`, `:8551-8555`): one after the other, or one agent.
- D.3 can start beside D.2: it is a new file, tested on hand-built
  `Module`s with the tables filled by the test, and switched to the real
  tables when D.2 lands.
- D.4 and D.5 can run in parallel after D.3: D.4 owns `nir.rs`,
  `select.rs` and `mod.rs`'s vertex branch (`:1198-1240`); D.5 owns
  `nir_fs.rs`, `vec16.rs` and `mod.rs`'s fragment branch (`:1290-1400`).
  Both remove variants from `MaliError` (`mod.rs:115-294`): a one-line
  conflict at most.
- The unlanded branches touch the same lines: `mali-uniforms` (`nir.rs`,
  `select.rs`, `mod.rs`) and `mali-vs-int` before D.4; `mali-fs-int`,
  `mali-fs-colour` and `origin/mali-fs-alu` (`nir_fs.rs`) before D.5. Land
  or rebase them first; their references stay and become D.4's and D.5's
  tests.

## Part E. Open questions

Each with the probe that settles it (the A.9 command).

1. **NIR's order of the folds.** `nir_opt_algebraic` runs to a fixed point
   before the late rules; whether a lowered tree of two operations
   (`m * 2.0 + 1.0`, m a mediump local from an attribute) keeps its inner
   operation at 16 bits after the outer one's late rule. Probe: that body,
   and the same with a third operation.
2. **The contraction rule** (`N/nir_opt_algebraic.py:2054-2060`, `contract`
   and `is_used_once`): when GLSL marks an `fadd`/`fmul` exact. Probe:
   `mediump float m = a + b; v = m * 2.0;` and the same under `invariant`.
3. **Built-in expansions.** `lower.rs` and `builtin_call.rs` expand
   built-ins in their own shapes; whether Mesa's lowered body
   (`map_builtin`) gives each step the same width (`normalize`, `length`,
   `distance`, `reflect`, `pow` of a mediump argument; `pow` is 32-bit by
   `bi_lower_bit_size`). Probe: each built-in on a mediump local, alone.
4. **`?:` and constructor order.** A.3's temporary rule depends on which
   assignment GLSL emits first for a constructor or a `?:`
   (`G/ast_function.cpp`, `G/ast_to_hir.cpp`; not read). Probe:
   `(b > 0.5) ? b : m` (highp first) and `vec2(2.0, m)`.
5. **Front-end constant folding of a lowered tree.** p1 shows Mesa folds
   `c * c` of a mediump `const` at fp16 (`0x3ec2`). If `lower.rs` (or
   `mali/glsl.rs`) folds such a tree at fp32 before the back end sees it,
   the folded constant has the wrong value and no operation is left to
   class. Check: our front end's IR for p1 (whether `v` is a `ConstF32`
   and which bits); if so, D.2 must record that the constant came from a
   Medium tree and the back end must refuse it until it is folded at fp16.
6. **Arena headroom.** Whether the two new tables push any shader of the
   corpus or of dEQP-GLES2 over the scratch arena on another back end.
   Check: the whole gate on D.2, and the largest shader's arena use before
   and after.
7. **`out` and `inout` parameters of mediump locals**
   (`G/lower_precision.cpp:1283-1352`): the conversions around a call that
   `lower.rs` inlines into stores. Probe: `void f(out mediump float r)
   { r = a * 2.0; }` called with a mediump local, then `v = r * 1.5;`.
8. **Fragment texcoord varyings.** `bi_fp32_varying_mask` decides on the
   optimised NIR (x and y from one load); whether a swizzled or partly
   constant coordinate keeps the varying at 32 bits. Probe: a mediump
   `vec4` varying read as `.zw` for `texture2D`, and `vec2(v.x, 0.5)`.

## Case notes

Date 2026-10-24, tip `916266f9`. Plan #153 item `study-precision`, for issue
#31. It builds on `docs/notes/mali-highp-fragcoord.md` (read it first; it is
not edited here): its 1.1 explains how Mesa decides fp16 or fp32, its 2.1 the
fragment-position lowering, and its open point 6 says "mediump arithmetic on
`gl_FragCoord` is not in any reference". This note closes that point with
references. Citations are Mesa 26.2.1 (`mesa-pan/mesa-26.2.1` in the deps
cache); `B/` is `src/panfrost/compiler/bifrost/`, `G/` is `src/compiler/glsl/`.
The words quoted are **read from the references** `tools/mali/testdata/prec_*.expect.json`
(Mesa 26.2.1's GL driver on drm-shim, GPU id 0xa867), not derived.

## The programs and the references

| Fixture | Case | Program |
|---|---|---|
| `prec_19.{vert,frag}` | 19 `es100` | PowerVR SDK vertex shader: `uniform mediump mat4 transformationMatrix; gl_Position = transformationMatrix * myVertex;` (`myVertex` highp), constant-colour fragment shader (`gl_test_shaders_es100.h:284-296`) |
| `prec_19b.{vert,frag}` | probe | `gl_Position = myVertex * u1 + u4 + uh` with `mediump vec4 u4`, `mediump float u1`, `highp vec4 uh`: the packing of mixed fp16 and fp32 uniforms |
| `prec_82.{vert,frag}` | 82 `fragcentre` | passthrough VS; `precision highp float`; `vec2 p = floor(gl_FragCoord.xy); vec2 f = fract(gl_FragCoord.xy);` then highp arithmetic (`:202-217`) |
| `prec_84.{vert,frag}` | 84 `fragcoord` | passthrough VS; `precision mediump float`; `vec4(gl_FragCoord.x / 64.0, gl_FragCoord.y / 64.0, 0.0, 1.0)` (`:503-511`) |
| `prec_119.{vert,frag}` | 119 `pbufup` | case 82's shader into an EGL pbuffer (`gl_test.c:20213-20224`) |

Recorded with (`$W` a shim install, `MESA_PAN_TREE` not needed):

```
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_19.vert  tools/mali/testdata/prec_19.frag  tools/mali/testdata/prec_19  --uniform-values
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_19b.vert tools/mali/testdata/prec_19b.frag tools/mali/testdata/prec_19b --uniform-values
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_82.vert  tools/mali/testdata/prec_82.frag  tools/mali/testdata/prec_82
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_84.vert  tools/mali/testdata/prec_84.frag  tools/mali/testdata/prec_84
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_119.vert tools/mali/testdata/prec_119.frag tools/mali/testdata/prec_119
python3 -I tools/mali/pan_ref_diff.py extract tools/mali/testdata/prec_<n>.trace --draw > tools/mali/testdata/prec_<n>.expect.json
```

Only the `.expect.json` files are committed (the convention of `s4_ffma` and
the others; the `.trace`/`.stdout`/`.stderr` are what the `run` command writes). Two host faults had to be worked around to run the shim here, and
are not in the repository: the install lacked `lib64/panthor_dri.so` (a symlink
to `libgallium-26.2.1.so`; `dri` is a symlink to `.`), and this host's
`stdbuf -o0`, which `drm_shim_ref.sh run` wraps the draw in, replaces
`LD_PRELOAD`, so the shim was unloaded and `eglInitialize` failed with "failed
to create dri2 screen"; a no-op `stdbuf` earlier in `PATH` fixes it.

## Part 1. Case 19: a mediump uniform in the vertex shader

### Refusal

`src/shaderc/online/mali/mod.rs:1063`, in `compile_stage`'s vertex branch:
after `nir::build` and `UboLayout::vs` (`select.rs:268`) every uniform the
program reads whose declared precision is not `Precision::High` returns
`MaliError::UniformPrecision` (text `mod.rs:344-346`). Pinned by
`tests.rs:948` (`uniform mediump vec4 u; ... gl_Position = u;`).

### What Mesa does (`prec_19`, `prec_19b`)

**Why it is fp16.** `lower_variables_visitor::visit(ir_variable)`
(`G/lower_precision.cpp:1023-1060`) retypes a non-UBO float uniform of medium
or low precision to fp16 when the stage has `fp16_const_buffers`
(`:1025-1031`): the trace's NIR says `decl_var uniform ... mediump f16mat4
transformationMatrix`. The multiply has a highp operand (`myVertex`), so it is
not lowered; each fp16 lane is widened where used (`nir_op_f2f32`,
`B/bifrost_compile.c:3072-3075`, `bi_f16_to_f32_to`).

**The FAU layout** (words of the `vs.position` table; `uN.wK` is word `2N+K`):

- `prec_19`, the mat4 is **8 words, two fp16 components a word, column-major,
  low half first**: words 6-13 are `4c404c00 4cc04c80 4d404d00 4dc04d80
  4e404e00 4ec04e80 4f404f00 4fc04f80`, i.e. (16,17) (18,19) (20,21) ... (30,31)
  with `--uniform-values`' 16+c. Column `c` is words `6+2c`, `7+2c`. No padding
  between columns. The words before are the shared constants (viewport clamp
  etc.), after them `c7000000 47000000` and the three streamout offsets
  `4 8 12`, as for any vertex program.
- `prec_19b`, declaration order `u4`, `u1`, `uh`: `u4` words 6-7 (`4c404c00
  4cc04c80`: 16..19), `u1` **one word with the high half zero** (word 8,
  `00005000`: 32.0 in the low half), then the highp `uh` as **four fp32 words**
  (9-12: `42400000 ... 424c0000`, 48..51) starting on a fresh word. So: fp16
  uniforms pack halves within a uniform (a vec4 two words, a float one word,
  its high half zero, a vec3 would be two words with the high half of the
  second zero), each uniform starts a new word, and fp32 uniforms are not
  mixed into those words. This is the form `UboLayout::fs16` / `FauWord::UniformHalves`
  (`fau.rs:36-38`) already emits for a mediump fragment uniform.

**The words** (`prec_19.expect.json`, `vs.position`; the
shape of every lane of the product):

```
F16_TO_F32.wait0 r4, u4.w1.h1        ; one conversion per use: the high half of word 9
FMA.f32 r4, r4^, r1, 0x0.neg         ; first term: x * 0 - (-0) form, i.e. a multiply
F16_TO_F32 r5, u3.w1.h1
FMA.f32 r4, r5^, r0, r4^             ; accumulate
...
```

and for `prec_19b`: `F16_TO_F32 r4, u3.w1.h1; F16_TO_F32.wait0 r5, u4.w0.h0;
FMA.f32 r3, r3^, r5, r4^` (`myVertex.w * u1 + u4.w`). Findings:

1. The conversion is `F16_TO_F32 dst, uN.wK.hH`: a **separate instruction
   per fp16 lane per use** with the half selected on the FAU source (`.h0`
   low, `.h1` high). It is **not fused into the `FMA.f32` source**, and a lane
   used twice is converted once only if its two uses are close (the same
   `F16_TO_F32 r7, u5.w0.h1` register is reused by the next column), otherwise
   again.
2. The arithmetic is the highp vertex stage's `FMA.f32` chain unchanged
   (`FMA ... 0x0.neg` for the first term, then accumulating `FMA`), so the
   rest of the lowering is shared.
3. The order of instructions is Mesa's pressure schedule (conversions
   interleaved with the `FMA`s, `.waitN` flags), which our scheduler
   (`sched.rs::pressure_schedule`) will decide; the byte-equality test will
   show whether it does.
4. The fragment shader of the pair has the `fs.fragment` constant colour
   (`MOV.i32 r0, u2.w0`) with FAU `3c003c00 3c003948`, nothing new.

### Next refusals host compiling shows

Derived from the code path (`cargo` was not runnable on this worker, so
nothing was printed; the first step of the implementation is to print them):

- Lifting only `mod.rs:1063` is **wrong, not refused**: `UboLayout::vs` pushes
  `halves: false` (`select.rs:276-277`), so the mediump matrix would be laid out as
  sixteen fp32 words (the layout of the highp program) while Mesa's is
  eight fp16 words (above), and the draw would write fp32 data for words the
  program reads as halves. The layout and the refusal must change together.
- `nir::build` has only a 32-bit `Kind::Uniform` for the vertex stage; there
  is no half read and no `F16_TO_F32` row: `grep -n F16 bir.rs` is empty, the
  generated ISA has the instruction. That is the first missing piece.
- After that, the vertex shapes in the references pass the existing rules
  (`prec_19b`'s `fma(myVertex, u1, u4) + uh`, two operations).

### Implementation order for case 19

1. `UboLayout::vs`: take `halves` for a mediump uniform (reuse the `fs16`
   push; one word per uniform start, vec2 one word, vec3/vec4 two, a float one
   with a zero high half); `FauWord::UniformHalves` in the vertex FAU
   (`src/mali/program.rs`) and the fp16 data of the vertex push block
   (`frame.rs` `emit_stage_uniforms` already writes the fragment's).
2. A half-read node (`Kind::UniformHalf { dword, hi }`) and the
   `Op::F16ToF32` row (`bir.rs`, `pack.rs`, `emit.rs` `ROWS`); lift `mod.rs:1063`
   for float, vec2-vec4 and mat4 only (keep `UniformType` for the rest).
3. Byte-equal test of `prec_19` and `prec_19b` (`vs.position` words, `fau`,
   `preload`, `work_reg_count`) in `tests.rs`; `tests.rs:948` moves to a type
   still refused (a mediump array). `corpus-verdicts.txt` forward only.
4. Board: case 19 x2, then the sweep x2.

### Lowered (plan #185 item `vsfp16-19`, 2026-11-02)

Steps 1-3 are done; the board (step 4) is not. Byte-equal, both stages and
every fact, to `tools/mali/testdata/vsfp16_19_{hello,mixed,tex,vec4}.expect.json`
(drm-shim, `--uniform-values`): `tests.rs`
`the_fp16_vertex_uniforms_are_byte_equal_to_mesa_26_2_1`, `VSFP16_POPULATION`.
`hello` is the corpus's `es100/hello.vert` (prec_19), `tex` its
`textured.vert`, `mixed` a mediump vec4 before a highp vec4, and `vec4`
a mediump vec4 alone.

- **Which uniforms** (`nir.rs` `fp16_uniform`): a float, vec2, vec3, vec4
  or mat4, not an array, declared mediump or lowp. That is
  `lower_variables_visitor::visit` (`G/lower_precision.cpp:1023-1060`): a
  uniform outside a block whose base type is a float, when the stage has
  `fp16_const_buffers` (panfrost from v6, `pan_screen.c:588`). A struct
  member stays fp32 (its base type is a struct), so `mod.rs` gives the
  walk the plain uniforms only. A mediump array (which Mesa does lower)
  and a mediump struct member have no reference and stay
  `MaliError::UniformPrecision`, and so does any other type.
- **The node.** The walk's `LoadUniform` makes the same `Kind::Uniform`
  load (`Nir::load16`, `bits: 16`) and widens each lane it reads with its
  own `Alu::F2f32` (CSE'd like any ALU node). Everything after is the fp32
  vertex walk, unchanged.
- **The layout** (`select.rs` `UboLayout::vs`): a uniform read through a
  16-bit node is a `halves` row (`FauWord::UniformHalves`, the fragment
  stage's form). The frame already uploads such words for either stage
  (`src/mali/frame.rs` `PushWord::UniformHalves`, `f32_to_f16`), so
  `src/mali` did not change.
- **The selection** (`select.rs` `vs_variant`): lane `c` of a 16-bit
  uniform node is `Idx::Ubo { base, comp }` of its word, swizzled `H00` or
  `H11` for its half. `base` is the load's first word, so a mat4 column is
  its own base (the push keeps Mesa's column order). The existing
  `F16_TO_F32` row (`emit.rs`, case 121's) reads it as `uK.wJ.hH`.
- The scheduler and register allocator put the conversions where Mesa
  does (each before its `FMA`, `r7` reused), with no change to either.

The `tex` pair declares `TexCoord` and `Tint` in the opposite order in each
stage. Our varying locations follow each stage's declaration order, so
`compile_vs_for_reads` is given the fragment shader's reads at the vertex
shader's locations (Mesa's link matches them by name); the link does this
itself since case 19's link was lowered (`mod.rs`
`reads_at_vertex_locations`, `mali-partial-varyings.md`).

`es100/texfill.vert` (`uniform Fill u_fill`, a struct with default highp
members) was refused with the same text only because the precision lookup
found no row for `u_fill.matrix`. `mod.rs` `uniform_precision` now reads a
member's precision from the struct's spelling (`reflect.rs`
`UniformDecl::ty`), and the shader is `ok` on the existing fp32 path. No
reference draws a struct uniform's block layout yet.

## Part 2. Cases 82, 84, 119: mediump arithmetic on gl_FragCoord

### Refusal sites

`MaliError::FragCoordMediump` (`mod.rs:353-355`) is raised in `nir_fs.rs`:

| Site | Condition | Cases |
|---|---|---|
| `nir_fs.rs:1484` | any `LoadInput { slot: FragCoord }` in a **mediump** shader (`prec.highp` false) | 84 |
| `nir_fs.rs:1536` | an `Alu` in a **highp** shader with a source that is directly the `FragCoord` load (`fc`, `:1531`) | 82, 119 |

A highp shader that first copies `gl_FragCoord` to a highp variable is not
refused: `f.frag_coord(at)` (`:677-700`) builds the fp32 graph (`PixelCoord`,
`U2f32`, `Fadd 0.5`, `Ffma` by `FB_WPOS_Y_TRANSFORM`). Pinned by
`tests.rs:1014-1015`.

### What Mesa does

`gl_FragCoord` is mediump in ES 1.00 whatever `precision` says
(`G/builtin_variables.cpp:1333-1335`) and an input is not retyped, so an
operation applied to the builtin itself is lowered (`handle_precision`,
`G/lower_precision.cpp:295-312`): the position is computed in **fp32** exactly
as `mali-highp-fragcoord.md` 2.1, **narrowed once to an fp16 pair**, and the
operation runs on the halves; a highp consumer widens a lane back
(`F16_TO_F32`).

**The fp32 block is the same as the highp case, in a different order** (the
scheduler's), `prec_84`: `U16_TO_F32.discard r0, r59.h0` (x), `U16_TO_F32 r1,
r59^.h1` (y), `FADD_IMM.f32` 0.5 each, `FMA.f32 r1, r1^, u2.w0, u2.w1` (y
transform; FAU words 4-5 `3f800000 00000000` = (1, 0), the unflipped
framebuffer), preload `0x3800000000000000`, `work_reg_count` 32.

**The narrowing is the two-different-defs form** of `mali-highp-fragcoord.md`
open point 4 (x and y are an `FADD` and an `FMA` result, never one def):

```
FADD.f32 r0.h0, r0^, 0x0.neg       ; f2f16 of x into the low half
FADD.f32 r1.h0, r1^, 0x0.neg       ; f2f16 of y
MKVEC.v2i16 r0, r0^.h0, r1^.h0     ; the pair (x, y)
```

### Case 84 (`prec_84.expect.json`, mediump shader)

After the narrowing, the whole division is **one instruction**:

```
FMA.v2f16.wait0126 r0, r0^, u3.w0, 0x0.neg      ; (x, y) * (1/64, 1/64)
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
MOV.i32.wait r1, 0x3C000000                      ; (0.0, 1.0) as halves
BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
```

`u3.w0` is FAU word 6 `24002400`: **`x / 64.0` is `x * 0.015625`** as a
halves pair, the multiply being `FMA.v2f16 ..., -0.0` (the same form
`select.rs` uses for the mediump `Fmul`). 14 instructions in all. No widening:
the colour is fp16.

### Cases 82 and 119 (`prec_82`, `prec_119.expect.json`, highp shader)

Only the two builtin calls are fp16; everything after them is the highp walk.
After the fp32 block (y first here: `U16_TO_F32.discard r0, r59.h1` is y, `r1`
is x) and the narrowing, where the pair's **low half is x**
(`MKVEC.v2i16 r0, r1^.h0, r0^.h0`):

```
FROUND.v2f16.rtn r1, r0            ; floor(xy) on both halves: p
FADD.v2f16 r0, r0^, r1.neg         ; fract(xy) = xy - floor(xy): f
F16_TO_F32 r2, r0.h1               ; widen f.y   (r0 = f)
FMA.f32 r2, r2^, u3.w1, 0x0.neg    ; f.y * 64
F16_TO_F32 r0, r0^.h0              ; widen f.x
FMA.f32 r0, r0^, u4.w0, r2^        ; f.x * 128 + f.y*64
F16_TO_F32 r2, r1.h1               ; widen p.y
F16_TO_F32 r1, r1^.h0              ; widen p.x
FMA.f32 r2, u3.w0, r2^, 0x0.neg    ; p.y * 4/255
FMA.f32 r1, u3.w0, r1^, 0x0.neg
FADD.f32 r2.h0 ... ; MKVEC.v2i16   ; colour narrowing (the highp colour store)
FMA.f32 r0, r0^, u4.w1, 0x0.neg    ; (.. ) / 255
...
```

(The full 30-instruction program is `prec_82.expect.json`; the FAU is words
6-11 `3c808081 42800000 43000000 3b808081 3c003c00`: 4/255, 64, 128, 1/255, the
halves pair.) Findings:

1. `floor` is **`FROUND.v2f16.rtn`** and `fract` is **`FADD.v2f16 xy, floor.neg`**,
   both on the one narrowed pair: a single narrowing is shared by the two calls
   (CSE before vectorising), not one per call.
2. Each half of `p` and `f` is widened separately, `F16_TO_F32 dst, r.hH`, at
   its use (the same instruction as case 19, the source a register half
   instead of a FAU half).
3. Everything after is the existing fp32 walk (`alu32`), and the colour
   narrowing at the store is the existing `FADD.f32 .h0` + `MKVEC.v2i16`.
4. **`prec_119` is word-for-word equal to `prec_82`**, `text` and `fau`
   (checked): the shim draws a framebuffer object, stored y-up like a
   pbuffer, so `T = (1, 0)` in both. 119 differs from 82 only on the board: the
   pbuffer is unflipped, the window surface flipped (`(-1, height)`), and the
   draw must write the transform word per target (`frame.rs`, `flip`;
   `mali-highp-fragcoord.md` open point 3). Back-face culling is draw state.

### Next refusals host compiling shows

Derived from the code path (see Part 1: `cargo` was not runnable here):

- **84, lifting `nir_fs.rs:1484`.** `frag_coord(at)` returns fp32 `Val::Lane`s,
  but a mediump shader's arithmetic is `Pairs` of fp16 (`Row { bits: 16,
  width: 2 }`, `select`/`vectorise`, `nir_fs.rs:636-`), whose leaves are
  `Leaf::Half`/`Leaf::Mk`. The fp32 lanes have to become a pair first: `Leaf::Mk`
  of two `F2f16 16 1` lanes (rows `emit.rs` already has) is exactly the
  `FADD.f32 .h0` + `MKVEC.v2i16` above. Without it the next verdict is
  `MaliError::Arithmetic` or a `Defect` at the pair construction. The multiply
  by the exact constant pair `24002400` is then the existing mediump rule
  (`F16X2_*`, `nir_fs.rs:42`), `FMA.v2f16 ..., -0.0`.
- **82/119, lifting `nir_fs.rs:1536`.** `alu32` would accept `floor`/`fract` of
  the fp32 position and emit `FROUND.f32.rtn`: **no refusal, wrong words** (and
  a different value above 2048). There is no 16-bit `FROUND` or
  `FADD.v2f16 .neg` row on this path for the builtin. So the guard must be
  replaced by the half path, not removed.

### Implementation order for 82/84/119

1. The narrowing node: a pair of the two fp32 frag-coord lanes
   (`Alu::F2f16` lane by lane, `Alu::Vec2`), emitted as above, shared by every
   use of the builtin (one `MKVEC` per shader).
2. Case 84: lift `:1484` with the narrowed pair feeding the existing 16-bit
   multiply; byte-equal to `prec_84` (14 instructions, FAU
   `... 3f800000 00000000 24002400`).
3. Cases 82/119: a half `floor` (`FROUND.v2f16.rtn`) and `fract` (`FADD.v2f16`
   with the floor negated) over the shared pair, and the `F16_TO_F32` row of a
   register half (shared with case 19); lift `:1536` only for `floor`/`fract`
   applied directly to the builtin, keep it for any other operation.
4. Tests: byte-equal `prec_82` (and `prec_119` as the same words),
   `prec_84`; `tests.rs:1014-1015` move to a still-refused shape (mediump
   `sqrt(gl_FragCoord.x)`). `corpus-verdicts.txt` forward only.
5. Board: cases 82, 84, 119 x2, then the sweep x2; 119 additionally proves the
   unflipped transform word on a pbuffer.

## Order across the four cases

`F16_TO_F32` is shared (case 19 from the FAU, cases 82/119 from a register):
do case 19 first, then the narrowing with case 84, then 82/119. Nothing in
`src/` changed in this item.
