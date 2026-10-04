# The online front end: GLSL ES 1.00 in the library

The design record for `src/shaderc/online/`, written as it is built. It is
milestone **M2** of the in-process shader compiler (task #55): a real GLSL
ES 1.00 front end -- lexer, preprocessor, parser, semantic analysis -- that
produces an IR for the GPU back end and the reflection the GL core reads.

Status, 2026-09-23: lexer, preprocessor, parser, semantic analysis,
reflection and the lowering to `ir::Module` all done, and running on silicon.
Measured against dEQP's own corpus at **0 false rejects in 1,778 must-build
sub-cases** and **0 false accepts of 1,005 must-fail cases**
(`tools/online_oracle.py`), with **0 wrong results in 7,317 value sub-cases**
run through the IR evaluator (`tools/online_eval_oracle.py`: 7,299 pass, 4
unmeasured, 14 with no outputs to check), and against the
offline tables at **139 of 142** GC7000 sources and **235 of 240** PowerVR
ones.

0 false accepts is not conformance. dEQP is not a complete oracle for that
direction: the missing default float precision (section 4.5.3) was found ON
SILICON, and this corpus has no case for it at all, because dEQP's own
templates always declare it. It means the corpus has nothing further to say,
and the next invalid shader we refuse will be found some other way.

Two of those numbers were once reported as 0 and 836. They were not measured;
see "The three zeros this document used to report were not true" below, which
is the most useful section in this file.

---

## Why a front end at all

Without it a shader reaches the GPU one way, which does not work for every
application:

| back end | how | why it is not enough |
|---|---|---|
| `src/shaderc/offline.rs` | SHA-256 of the GLSL text looked up in `blobs.rs`, a table built on a host | an application that writes a shader at run time is not in the table, and dEQP-GLES2 writes one on every case |

So the other way is the compiler itself, in the library. This document covers
its front half. The USC encoder is M1 and the code generator is M3.

## The two constraints that shaped everything

### 1. There is no allocator

The `yakogl` crate is `no_std` with no dependencies. `grep -rn "use alloc::"
src/` finds nothing, and `Cargo.toml` pulls no `std` in. The GL core's one heap use is the copy
`glShaderSource` keeps, through the OSAL.

The front end therefore **allocates nothing**. The caller lends it a
`&mut [u8]` and `arena.rs` bump-allocates every token, AST node and IR node
inside it; references between nodes are `u32` indices, not pointers. Running
out of scratch is a reportable outcome (`ArenaError::OutOfScratch`), never a
panic, and it carries its own info-log message -- "compiler scratch buffer
exhausted" reads nothing like "your shader is wrong", which is the point.

`AVec` grows by reallocating at the arena's top and copying. A bump allocator
cannot reuse the old block, so growth wastes it; that is deliberate, because
the alternative -- counting every list before building it -- doubles the
parser, and a compile is short-lived.

### 2. Two GPUs, one front end

The PowerVR USC is a scalar machine. The Vivante GC7000 is vec4 with write
masks and swizzles. The IR is therefore NIR's ALU model: a value is 1..4
components and **every operand carries a swizzle**. Scalarising that for the
USC is mechanical; the GC7000 encoder uses it as it stands.

## The IR

Proposed on 2026-09-22.

```rust
pub struct ValueId(pub u32);   // SSA: 1..4 components of one scalar type
pub struct LocalId(pub u32);   // mutable slot: a (Type, array_len) allocation

pub enum ScalarType { F32, I32, Bool }
pub struct Type { pub scalar: ScalarType, pub comps: u8 }  // 1..4

pub struct Src { pub val: ValueId, pub swizzle: [u8; 4] }  // no neg/abs: the back end folds those

pub enum Op {
    ConstF32([f32; 4]), ConstI32([i32; 4]), ConstBool([bool; 4]),
    Alu(AluOp, [Src; 3]),
    LoadUniform    { dword_off: u16, comps: u8 },
    LoadUniformIdx { base_dw: u16, stride_dw: u16, len: u16, index: Src, comps: u8 },
    LoadInput      { slot: IoSlot, comps: u8 },
    StoreOutput    { slot: IoSlot, src: Src },
    LoadLocal      { local: LocalId, index: Option<Src>, comps: u8 },
    StoreLocal     { local: LocalId, index: Option<Src>, src: Src, write_mask: u8 },
    Tex            { kind: TexKind, sampler: u8, coord: Src, extra: Option<Src> },
    Discard,
}

pub enum IoSlot {
    Attrib(u8), Varying(u8),                // by LOCATION
    Position, PointSize,                    // vertex outputs
    FragColor(u8), FragDepth,               // fragment outputs
    FragCoord, FrontFacing, PointCoord,     // fragment inputs
}

pub enum Node { Instr(InstrId), If { cond: Src, then_: Range, else_: Range },
                Loop { body: Range }, Break, Continue, Return }
```

Four decisions inside it, each of which moves work across the front end / back
end boundary:

**(a) Every call is inlined.** ES 1.00 forbids recursion (section 6.1), so
inlining always terminates. The IR has exactly one body: no functions, no
`out`/`inout` parameters, no call stack for the back end to model.

**(b) The whole built-in library is lowered to a ~45-opcode kernel.**
`mat4 * vec4` becomes an `fmul`/`ffma` chain; `dot`, `length`, `normalize`,
`distance`, `reflect`, `refract`, `faceforward`, `cross`, `pow` (exp2/log2),
`mod` (fract), `clamp` (min/max), `mix` (ffma), `step`, `smoothstep`,
`atan`/`asin`/`acos` (polynomials), `matrixCompMult` and the relational vector
built-ins all expand here. The back end implements 45 opcodes instead of 90
built-ins.

**(c) Matrices, structs and arrays-of-structs do not exist in the IR.** A
matrix value is its columns; a struct uniform is flattened into members
exactly as `tools/yakogl-tools/src/glsl.rs` flattens them today, which is what
`ShcUniform::inner` already encodes. Arrays survive only as `LoadUniformIdx`
and as `Local` slots with an index operand.

**(d) SSA for straight-line values, load/store slots across control flow.** No
phi nodes: a GLSL variable whose value escapes an `if` or a loop becomes a
`Local`. This is NIR before `lower_vars_to_ssa`, and it is what a register
machine wants anyway. Phis can be a pass on top; they are not a different IR.

## Reflection

The IR is one output; the reflection the GL core consumes is the other, and it
is **not** part of the IR. It must equal the offline table's rows field for
field, because those rows are observable through `glGetActiveUniform`,
`glGetActiveAttrib` and `glGetAttribLocation`.

| field | who owns it | note |
|---|---|---|
| `ShcUniform` (all seven) | front end | `name`, `dword_off`, `dwords`, `gl_type`, `array_len`, `binding`, `inner`: std430 offsets, `#define`-resolved array lengths, the GL name after struct flattening |
| `ShcAttrib::{location, name, gl_type, components}` | front end | |
| `ShcAttrib::{vi_reg, format}` | back end | register allocation |
| `ShcVarying::{location, name, interp, components}` | front end | |
| `ShcVarying::vtxout_off` | back end | |
| `uniform_dwords` | front end, as an upper bound | the front end gives the std430 block size; the table's value is the prefix the code generator actually loads. `resolve_shader` already asserts `uniform_dwords <= block_dw` |
| `uniform_sh_dwords`, `temps`, `meta_off`, `num_outputs`, `usc*` | back end | |

**Row order is load-bearing** and was pinned by f4b9bae: attributes sorted by
location, uniform rows in *declaration* order, because `glGetActiveUniform`
indices follow them. `tools/yakogl-tools/src/emit.rs` documents it at length.
The front end reproduces declaration order after struct flattening and never
sorts uniform rows.

### One known asymmetry, recorded rather than papered over

`resolve_shader` **drops declared-but-unused samplers** from the uniform rows,
using `pco_shc`'s descriptor list -- that is, using the code generator's
post-dead-code answer. A front end can only say "statically referenced", which
is not the same question. So the front end emits every declared sampler with a
`statically_used` flag, M3 applies the filter, and the acceptance test compares
after the filter. Any source where the two answers disagree is reported, not
absorbed.

## The corpus, measured

The milestone was written as "every source in `SHC_SOURCE_TABLE`". Two facts
about that table (origin/main 634235f) change what that can mean:

* It holds **533** sources, not 2,042. 2,042 is `SHC_BLOB_TABLE`: sources
  times blend and render-target variants.
* They are not all ES 1.00. By the `#version` in the stored text:

  | version | count | what they are |
  |---|---|---|
  | `#version 300 es` | 280 | 138 hash-named Qt Quick 3D shaders (struct uniform blocks, `sampler2DArray`) plus the `gl_test` set |
  | `#version 100` | 238 | the FemtoVG / Slint family: heavy `#define`, `uniform vec4 frag[UNIFORMARRAY_SIZE]` |
  | `#version 450` | 13 | hand-written desktop GLSL for the `pco_shc` pipeline (`uniform_color.frag`, `mvp.vert`, ...) |
  | none | 2 | ES 1.00 by section 3.3 |

So M2's acceptance is **240 of 240 ES 1.00 sources** (238 + the 2 without a
directive), with ES 3.00 a named follow-on and the 13 desktop-450 sources left
on the existing path -- they are our own test shaders, and no ES 2.0
application can present them. `tools/es100_oracle.py` stays at or above
2787/2793 throughout. On the GC7000 table the same count is **142 ES 1.00
sources**, which is the governing number for this GPU. (It read 125 while the
table was smaller; the count is derived from the table, so it moves with it and
is re-read rather than remembered.)

As of 2026-09-22 the front end lowers **139 of those 142**, and **235 of 240**
on the PowerVR table. Every one of the 8 refusals is the same defect in the
tables themselves -- sources that declare `#version 100` and then use
`sampler2DArray`, which arrived in ES 3.00 -- so the score against the sources
that are actually ES 1.00 is 139 of 139 and 235 of 235. That is finding 3 at
the end of this document, and it is a reason to fix the tables, not the
compiler.

Until 2026-09-23 an ES 3.00-family shader was **refused at its version
line**, by name, as out of scope. The user has since chosen ES 3.0 on both
boards, so `#version 300 es` is compiled (3.10 and 3.20 are still refused at
the version line, and a `report-es2` build refuses 300 es there too); see
"GLSL ES 3.00" below for what is in and what is still a gap. The reason the
refusal was at the version line still holds for what remains refused: parsed
under ES 1.00 rules such a shader stops on its first `in`, `out` or `uvec2`
with "expected a type", which reads as a missing feature of this compiler and
is how 130 sources were once counted against the front end that had nothing
wrong with them.

## GLSL ES 3.00

Worked in steps, each measured by three columns -- the ES 3.00 compile oracle
(`tools/online_oracle.py --gles3`: false rejects of must-build, false accepts
of must-fail), the ES 3.00 value oracle (`tools/online_eval_oracle.py
--gles3`: wrong of sub-cases) -- with the two ES 1.00 oracles held at 0/0 and
0 wrong throughout, and by how many of the 148 Qt Quick 3D sources in
`tools/cts/RESULTS.md`'s ES 3.00 target list the front end lowers. All of it
is HOST-ONLY: nothing below has run on a board.

| step | what | GLES3 compile (false rej / false acc) | GLES3 values (wrong) | Qt Quick 3D |
|---|---|---|---|---|
| before | -- | 3,454 of 3,454 / -- | -- | 0 of 148 |
| 1 | `#version 300 es`, keywords, `in`/`out`/`flat`/`layout(location)` | 1,081 / 0 of 2,053 | 0 of 13,953 (9,566 pass) | 0 |
| 2a | `uint`/`uvec`, `1u`, `%`, `~`, `&`, `\|`, `^`, `<<`, `>>` | 737 / 0 of 2,053 | 0 of 13,953 (11,841 pass) | 0 |
| 2b | arrays: `T[N](..)`, `T[](..)`, `float[3] a`, `a[] = ..`, `.length()`, `=`/`==`/`?:` on arrays, array parameters and returns, arrays of matrices | 427 / 0 of 2,053 | 0 of 13,953 (12,337 pass) | 0 |
| 2c | non-square matrices: `matCxR` as C columns of R rows in every product, constructor, uniform, local, `transpose`, `outerProduct` | 127 / 0 of 2,053 | 0 of 13,953 (13,733 pass) | 114 of 114 |
| 3a | ES 3.00 samplers and every section 8.8 look-up (IR v1's `Op::Tex`/`TexSize`), `inverse`/`transpose`/`determinant`/`outerProduct`, integer `abs`/`sign`/`min`/`max`/`clamp`, `mix` by bvec; for Qt: 16 parameters, early return from a struct function, a matrix array in a uniform struct array | 427 / 0 of 2,053 | 0 of 13,953 (12,337 pass) | **114 of 114 reachable** |

**The 148 are not all reachable.** 32 are Qt's multiview variants
(`#extension GL_OVR_multiview2 : require`), which Qt only builds for XR
rendering and which this driver does not advertise; 2 declare `#version 100`
and then use `sampler2DArray` (the table defect in finding 3). The target is
therefore 114, and after step 2b every one of them stops on the ES 3.00
texture functions and sampler types (`texture`, `textureLod`,
`sampler2DArray`, `sampler3D`), which is step 3. None of the 114 uses a
non-square matrix or `switch`, so step 3 goes before those.

After step 3a all 114 reachable sources get through the FRONT END. That is
not the same as compiling: the GC7000 and PowerVR back ends still refuse
every IR v1 look-up shape other than the five ES 1.00 ones, by name, until
each selects it (3D, 2D array and cube-with-level look-ups, `TexSize`), and
nothing here has run on a board. The dEQP compile columns do not move with
step 3a because the `.test` corpus exercises the texture functions only in
`invalid_texture_functions.test`, whose 120 cases were already refused and
still are.

Step 4, `switch` (section 6.2): 123 / 0 of 2,053 compile, 2 wrong of 13,953
values (13,755 pass), ES 1.00 still 0/0 and 0 wrong. The body is lowered as a
loop that runs once. Each run of labels becomes
`if (fell || <its labels>) { fell = true; ... }`, with `default`'s condition
being "no label matched", so it is taken only then, wherever it stands. A
`break` leaves that loop. A `continue` belongs to the loop around the switch:
it sets a flag and breaks, and the flag becomes the real `continue` after the
switch. A `return` is handled as a loop's return. The checker enforces every
rule dEQP's `switch.test` probes: a scalar `int`/`uint` selector, constant
labels of its type, no duplicate label, one `default`, no statement before the
first label, no label as the last statement, and no label outside the
switch's own statement list. All 52 of those cases are refused, as before.

dEQP's `constant_expressions.other.complex` (vertex and fragment) became
measurable once `switch` lowered. Its label `clamp(min(gl_MaxDrawBuffers, 4),
1, 6)` assumes the ES 3.0 minimum of 4 draw buffers. The ES 3.00 built-in
constants (section 7.3's list: `gl_MaxVertexOutputVectors`,
`gl_MaxFragmentInputVectors`, `gl_Min/MaxProgramTexelOffset`, and no
`gl_MaxVaryingVectors`) now take the GL core's per-GPU values, so
`gl_MaxDrawBuffers` is `GL_MAX_DRAW_BUFFERS`: 3 on the PowerVR and 1 on the
GC7000. With either, that label equals an earlier one (`_3` = 3, or
`ivec3(_1).x` = 1), and the shader is refused for a duplicate case label. That
refusal is correct for the value this driver reports. The case needs a GL
core that offers 4 draw buffers, which is on the ES 3.0 gap list in
RESULTS.md. So it counts as 2 false rejects (111 -> 113) where it was 2 wrong
values.

Two defects were found on the way, both older than ES 3.00. A scalar
argument of a component-wise built-in (`max(v3, f)`) was handed over as a
one-lane value whose other lanes nothing wrote (commit 0f7bba52); dEQP's value
cases never met it because their scalars are literals, which are splat
constants. And ES 3.00 section 4.5.4 gives only `sampler2D` and
`samplerCube` a default precision, so any other sampler declared without
one is now refused.

### Uniform blocks: flattened into the default block

Section 4.3.7's `layout(...) uniform Name { members } [instance [N]];` is
parsed in ES 3.00 and desugared by the parser into what the later passes
already know: `struct Name { members };` and then `uniform Name instance[N];`,
or, with no instance name, one `uniform T member;` per member. The struct
puts the block name in the global name space, which is where section 4.3.7
and dEQP's `conflicting_*` cases put it, and it makes the block name a type,
so reading a member through it is refused ("a type name is not a value").
The block's members are therefore ordinary rows of the default uniform block,
and they are read like any uniform struct's members. `Ast::blocks` keeps
each block's name, instance, array size, packing and matrix layout (per
block and per member, with `layout(...) uniform;` defaults) for the block
table the GL core will copy bound buffers through.

Deliberate: the block table is Rust-only. `reflect::block_table` gives
each block its std140 size and each member its buffer offset, array and
matrix strides, and row-major flag. The online back end copies that table
into the shader's own keep record, behind `priv_`, next to the shader, and
the GL core reads it through `shaderc::uniform_blocks`. It is not a field
of `struct shc_shader`, for two reasons. Only the online compiler produces
blocks, because the offline tables never do. And a new
field in the C record would force every generated table row to be
regenerated. The accessor answers empty for a table shader
(team-lead, 2026-09-24).

Deliberate: `shared` and `packed` are laid out as `std140`. Section 4.3.8.3
leaves both layouts to the implementation, and `std140` is one of the
layouts it could choose, so one layout serves all three.

Step 6 (the front end half), HOST-ONLY: GLES3 compile 113 -> 47 false
rejects, 0 of 2,053 false accepts. All 72 of `uniform_block.test`'s must-fail
cases are refused for the rule they probe, not by accident. GLES3 values 0
wrong; GLES2 0/0 and 0 wrong.

The GL core half, HOST-ONLY:

- The indexed GL_UNIFORM_BUFFER binding points are kept per context slot,
  outside the C-shared `GlContext`. They cover glBindBufferBase/Range and
  glGetInteger(64)i_v, and deleting a buffer unbinds it.
- At link, a program matches its stages' blocks by name. A block both
  stages declare must have the same layout in each, and a program has at
  most 24 blocks.
- The block members leave the location table (glGetUniformLocation answers
  -1 for them). They are listed as active uniforms after the default-block
  ones, under their GL names.
- glGetUniformBlockIndex, glGetActiveUniformBlockiv and
  glGetActiveUniformBlockName answer from the program's blocks, as do
  glGetUniformIndices and glGetActiveUniformsiv with the block fields, and
  glGetProgramiv's ACTIVE_UNIFORM_BLOCKS and
  ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH. glUniformBlockBinding sets a
  block's binding point.
- Before each draw, every active block is copied from the range bound to
  its binding point into its rows in each stage's default block. A
  row-major matrix is transposed, and a bool is normalised to 0 / 1.

The reported limits (MAX_*_UNIFORM_BLOCKS, MAX_UNIFORM_BUFFER_BINDINGS,
MAX_UNIFORM_BLOCK_SIZE) are gc-backend's to set per GPU.

### Arrays: always a run, never a value

An array has no single value in this IR (decision 4), so every ES 3.00 array
expression -- a constructor, a variable, a call that returns one, `?:` over
two -- is MOVED into a run of slots by one function, `array_fill`, the way
`struct_fill` moves a struct. A declaration, an assignment, an argument, a
`return` and an `==` all go through it, and an array value that is indexed
(`float[3](a, b, c)[i]`, `f()[i]`) is moved into a temporary run first. An
array of matrices is a run of runs (`Run::MatrixArray`, element k at
`k * cols`), and ES 1.00 local arrays of matrices use the same layout.
`float a[] = ...` takes its size from the checker's type of the initialiser,
and `.length()` is the checker's array length, never an evaluation of its
operand. dEQP-GLES3's `arrays.test` went from 294 false rejects of 294 to 4
(a struct declared inside a function body).

### `uint`: a kind of its own, and the bits stay put

`uint` is a fourth scalar kind in the checker, never converted to or from
`int` implicitly (section 4.1.10), so `int v = 0u;` is refused as a mismatch
-- dEQP's `constants.int_from_uint_2` -- instead of by a special rule. A
`1u` is its own AST node (`Expr::UintLit`) for the same reason. In the IR a
uint lives in an integer lane as its bit pattern, and only the operations
where signed and unsigned differ say which it is (ir.md's ES 3.00 row):
`Udiv`, `Umod`, `Ushr`, `Ult`/`Ule`, `U2f`, `F2u`. `int(u)` and `uint(i)`
are no instruction at all -- section 5.4.1 keeps the bit pattern -- and the
same ValueId is reused.

The integer operators follow ES 3.00 section 5.9, whose shape rules are not
the arithmetic ones: `%` and the bit-wise three need both operands of one
signedness, while a shift may mix them, takes the LEFT operand's type (and
the left operand's signedness decides whether `>>` extends the sign), and
refuses a vector count on a scalar. ES 1.00 still reserves every one of them.

Two instruments had to learn the type before the numbers meant anything.
The value oracle's harness wrote a `uint` reference uniform as float bits,
so the first run read 943 wrong -- every one of them a harness encoding, none
a compiler defect (`onlineeval.rs` now writes a uint's own bits). And the
evaluator produced a value for a signed shift by 32 or more and for a signed
division by zero, which section 5.9 leaves undefined and ir.md already said
it did not; it now stops those unmeasured, as it did for the unsigned forms.

## What the specification says must be an error

dEQP checks that an invalid shader *fails to compile* (`shaders.negative`, the
preprocessor negative cases), so this list is as load-bearing as the positive
path. It grows as each pass lands; the ticked entries are implemented and have
a test named after the rule.

Lexical (section 3), all implemented in `lexer.rs`:

- [x] a non-ASCII character outside a comment (3.1)
- [x] a comment that reaches the end of the file
- [x] backslash line continuation -- ES 1.00 has none (3.1); ES 3.00 does
- [x] a floating-point suffix, `1.0f` -- ES 3.00 only
- [x] an unsigned suffix, `1u` -- ES 3.00 only
- [x] any other letter glued to a constant (`1east` is one bad token, never
      `1` followed by `east`)
- [x] an octal constant with a digit above 7
- [x] `0x` with no digits after it
- [x] an illegal character (`@`, `$`, `"`, `'`)
- [x] a keyword reserved for future use, used anywhere (3.6) -- classified as
      `Kw::Reserved`; the parser reports it

Preprocessor (section 3.4), all implemented in `pp.rs`:

- [x] `#version` must be the first thing in the shader but for comments and
      white space, and must name a version this implementation has
- [x] an unrecognised directive (but not inside a dead conditional group,
      where only the conditional directives are interpreted)
- [x] `#if` / `#ifdef` without `#endif`; `#endif` / `#else` / `#elif` without
      `#if`; `#else` or `#elif` after `#else`
- [x] tokens after `#endif`, `#else`, `#ifdef`, `#ifndef`
- [x] `#error` (which fails with the message that follows it)
- [x] redefining a macro to a different replacement list -- *including* a
      different arrangement of white space, which is the rule
      `invalid_object_whitespace` turns on
- [x] `#define` or `#undef` of a predefined macro, or of `defined`
- [x] a recursive macro
- [x] the wrong number of arguments to a function-like macro; a duplicate
      parameter name; a missing `)`
- [x] `##`, and any `#` that is not the first token of a line -- ES 1.00 has
      no paste operator and no stringize
- [x] `#extension` with a missing name, a missing or wrong `:`, an invalid
      behavior, `all` required or enabled, or an unsupported extension
      required
- [x] a non-integer or undefined value in a live `#if` expression, and
      division by zero in one
- [ ] **not** an error, and each one was checked against a dEQP must-pass
      case: a macro name with a double underscore; an undefined identifier
      that a `&&` or `||` short-circuits away; anything at all inside a dead
      conditional group; `#undef` of a name that was never defined; an
      unrecognised `#pragma`; a function-like macro name with no `(` after it

Syntactic (section 9), implemented in `parser.rs`:

- [x] a keyword reserved for future use, wherever it appears -- as a type, a
      name or a statement (`switch`, `half`, `sampler3D`, ...)
- [x] a struct with no members, an anonymous struct, a struct redefined
- [x] a declaration used as a loop condition with no initialiser
- [x] the ordinary shape errors, each naming what was expected rather than
      what was found
- [x] a shader nested past `MAX_DEPTH`, refused rather than overflowing the
      stack
- [ ] **not** an error here, on purpose: a reserved *operator* (`%`, `&`,
      `<<`, ...) parses into the tree so semantic analysis can refuse it by
      name, and a constructor is an ordinary call because whether a name is a
      type is not a question for the parser

Semantic (sections 4..9), implemented in `sema.rs`:

- [x] no implicit conversion in either direction, in an initialiser, an
      assignment, an operator or a comparison (4.1.10)
- [x] assignment to a `const`, a `uniform`, an `attribute`, a `varying` in
      the fragment shader, or a read-only built-in (`gl_FragCoord`,
      `gl_FrontFacing`, `gl_PointCoord`)
- [x] assignment to a swizzle that names a component twice (5.8)
- [x] a swizzle that mixes `xyzw` / `rgba` / `stpq`, names more than four
      components, or names one the type does not have (5.5)
- [x] a reserved operator: `%`, `&`, `|`, `^`, `<<`, `>>`, `~` and their
      assignment forms (5.1)
- [x] `attribute` in a fragment shader, on a non-float type, or on an array;
      `varying` on a non-float type; a `sampler` that is not a uniform; an
      initialiser on a `uniform`, `attribute` or `varying` (4.3)
- [x] a relational operator on anything but two scalars of one type (5.1)
- [x] operand types that cannot be combined, including the matrix product
      shapes
- [x] a condition that is not a `bool`; `break` or `continue` outside a loop;
      `discard` in a vertex shader; no `main`, a `main` with parameters or a
      non-void `main`
- [x] an undeclared identifier; a redeclaration in the same scope (shadowing
      an outer one is legal, 4.2.3)
- [x] an array size that is not an integer constant expression, and a
      constant expression that folds through a built-in call (5.10)
- [ ] still to come: the built-in function signatures (a call is currently
      unmodelled, so the rules above it stay silent), the precision-qualifier
      rules, and the cross-stage link rules for varyings and uniforms

## The lowering: GLSL to IR

`lower.rs` is the last pass and the largest. It walks the checked tree and
emits `ir::Module`. Six decisions shape it, and each was settled by a
measurement rather than a preference.

### 1. A shader is one body. Functions are inlined.

There is no call instruction in the IR and neither GPU wants one for shaders
this size. ES 1.00 forbids recursion (section 6.1), so inlining always
terminates; `MAX_INLINE_DEPTH` bounds it anyway and refuses rather than
overflowing the host stack.

`out` and `inout` parameters fall out of this: a parameter is a slot, an `out`
argument is a slot the caller already has, and binding the two is a copy the
back end removes when it can. A parameter the body never assigns is *aliased*
to the caller's value instead of copied -- but only after the body has been
checked, because deciding to alias after allocating a slot leaves the slot
allocated. That was a real regression: instruction count fell from 236 to 194
while slots stayed at 42, which is visible only if you kept the earlier number.

### 2. Early returns become structure, never a `Node::Return` in the middle.

Selection treats `Return` as the end of the shader, so a `return` emitted
where it stands would silently drop the code after it. An early return becomes
a result slot plus an `else`: what followed the `if` moves into the `if`'s
`else` arm. A corpus assertion pins it -- no `Node::Return` anywhere but the
last node -- and that assertion is what caught the real bug, which was not
inlining at all but `main`'s own top-level `return;`, on a path that did not
go through the rewrite.

That rewrite covers a `return` whose `if` arm returns on EVERY path. The
rest -- a `return` inside a loop, inside a bare block, or in an arm that
returns only on some paths -- goes through a one-lane bool flag: the return
stores its value, sets the flag and, in a loop, breaks; each enclosing loop
of the same function tests the flag right after itself and breaks too; and
the statements after the outermost one run under `if (!flag)`. The
some-paths arm compiled WITHOUT this before 2026-09-23 and was wrong:
`if (c) { if (d) return 1.0; } return 2.0;` let the second return overwrite
the first. `eval.rs` runs each shape (`return_rule_tests`).

### 3. Matrices are N column vectors everywhere.

Neither GPU has a matrix register, so a `matN` is N vectors wherever it lives
and a `matN` local is N consecutive slots. `ir.md` decision 4.

The column-to-component pairing is **measured, not assumed**: in `hello.vert`'s
shipped blob, column *i* multiplies `v[i]`. The blob's reversed *register*
numbering comes from its constant-file map `CD_4` (`12,13,14,15 / 8,9,10,11 /
4,5,6,7 / 0,1,2,3`), so `u0` **is** column 3. Checking `CD_4` rather than the
disassembly is what caught a wrong pairing in a back-end test before anything
was built against it -- and it would have drawn correctly for identity and
symmetric matrices, which is exactly the class of bug that survives a demo.

### 4. Booleans are all-ones masks.

Measured by `gles2-core` and `pvrgpu-platform` on both machines. `!` is
`Bnot`, `?:` is `Bcsel`. It is also why the reductions work: with all-ones
booleans `any` is `or` and `all` is `and`.

### 4a. An integer literal is a `ConstI32`.

Every integer operation (`Iadd`, `Ilt`, `I2f`, ...) reads its operands'
BITS. Until 2026-09-23 an integer literal was lowered as a float constant,
so `i < 3` compared against the bits of 3.0 (1,077,936,128): on the GC7000
`for (int i = 0; i < 3; i = i + 1)` compiled to `cmp.lt.s32 ..., 3.0:f20`
and ran twice, and FemtoVG's `int(frag[10].z) == 1` was never true, which
left the premultiply and alpha-texture branches of 19 FemtoVG/Slint sources
dead. It compiled, ran and drew. A constant constructor folds to its own
type the same way (section 5.4.1: to an int the fraction is dropped, to a
bool zero is false). Found by `eval.rs`, the test-only IR evaluator, the
first time a test RAN a loop rather than inspecting its shape; gl_test case
72 draws it.

### 5. `StoreLocal`'s write mask: destination lane *i* takes `src.swizzle[i]`.

Measured, and the two readings differ. `cube.vert`'s
`mov t1.__zw, void, void, u0.xxxy` gives lane w `swizzle[3]` = y under this
rule; the other reading makes `w = 0`. Written up in `ir.md`.

### 6. Vector equality is a compare and a reduction, and the spare lanes matter.

Section 5.9 makes `u == v` a single `bool`, not a vector of them, so it is
`Feq` then `Ball`, and `u != v` is `Fne` then `Bany`. The component-wise forms
are the built-in functions `equal` and `notEqual`, a different type; `<`, `<=`,
`>` and `>=` do not take vectors at all.

`Ball` and `Bany` fold **all four lanes** by contract, so a `vec2` compare must
repeat a real component into z and w or the reduction folds whatever the
register held -- a wrong answer with no failure, and one that a test written
only on `vec4` would never catch. Repeating costs nothing (`and` and `or` are
idempotent) and has to be the caller's job, because the reduction cannot know
the width: a repeated lane is legally a real one.

### A width is a question about a type, not about a representation

The recurring defect in this compiler, stated once so it can be recognised:
**two things share a representation and the code reads the representation
instead of the type.** It has cost us four times.

| what was read | what it also meant | the wrong picture |
|---|---|---|
| `components()` | a `vec4` **and** a `mat4` | a matrix compared or multiplied as one column |
| `components()` | a struct answers 0, `.max(1)` makes it 1 | two structs compared as scalars |
| `BaseType` | `vec4` **and** array-of-`vec4` | `frag[i]` taken for a component selection; 33 sources refused |
| a swizzle | the value's width **and** a repeat | a reduction folding lanes nothing wrote |

So: before a width, a count or a length is taken from a value, check that the
**type** admits the question. In `compare()` that is a guard refusing
matrices, structs and arrays *by name*, placed before the width is computed.
Delete it and `mat4 == mat4` compiles -- it does not fail, it draws.

### What the lowering refuses, and why that is not the same as a bug

Every refusal carries a line, a token and a reason, in the shape
`glGetShaderInfoLog` has always used, and the wording distinguishes two kinds:

* "... is not lowered yet" -- **a gap**. The construct is legal ES 1.00 and
  the compiler owes it. These are the false rejects, and the number must reach
  0.
* anything else -- **a verdict**. The shader is wrong, or the construct is not
  ES 1.00, and refusing is the correct answer.

**A verdict must name the specification section it rests on.** This is not
style. On 2026-09-22 three rows -- 20 dEQP cases, all `expect pass` -- were
verdicts with nothing behind them, and each told a user their legal shader
was broken:

| the refusal | what the specification actually says |
|---|---|
| `this function does not return on every path` | 6.1: the result is **undefined**, not illegal. dEQP's `functions.misc.missing_returns` says so in a comment of its own. |
| `this function returns no value`, on `f(x);` | 6.1: a value-returning function **may** be called for its effect and the result discarded. It returns a value and the caller ignores it -- the message was backwards. |
| `'return' with a value in 'void main()'` | It was not `main`. The function was an ordinary one and the `return` was inside a **loop**, which is a gap in our rewrite. Wrong function, wrong problem, and worded as a verdict. |

Two of the three carried a comment asserting "the checker refuses this, so a
shader cannot reach here" -- an assumption, never tested, and false in all
three cases. The first two rules were deleted; the third is now worded as the
gap it is.

A wrong verdict costs twice. It tells a user to rewrite working GLSL, and it
hides the work from us: these sat in the histogram all day looking like
correct rejections, which is exactly what a verdict is supposed to look like.

So: **before writing a refusal that is not "not lowered yet", find the
sentence in the specification that makes the shader wrong and put its section
number in the comment. If there is no such sentence, it is a gap and must say
so.** The category a refusal's wording carries is part of its identifier, in
the same way the string is -- see the histogram note under "Measuring it".

Nothing is ever approximated. A construct with no written lowering stops the
compile; it never emits a plausible sequence that draws something else.

The false rejects as of 2026-09-22, from `tools/online_oracle.py`. There are
29, and **every one of them is a gap**: the histogram below is entirely "not
lowered yet" and "is not reflected". Nothing in it tells a user their legal
shader is wrong.

      10  a `return` inside a loop           3  assignment to an indexed lvalue
       4  a function call                    2  a matrix product
       4  a struct member that is a struct   2  a nested struct is not reflected
       4  a declaration as a loop condition

That took two corrections, and both are worth remembering because neither
showed up in a total.

**The 18 matrix-constructor rows were not gaps and not verdicts -- they were
an unimplemented rule wearing a verdict's clothes.** Section 5.4.2 says the
arguments' components are consumed left to right, each argument entirely, in
column-major order, and says nothing about their widths or types matching
anything. The lowering had two special shapes and refused the rest with "this
matrix constructor takes its components in order", which reads as a rule the
shader broke. Implementing the sentence closed all 18.

**The 2 `this function is already defined` rows moved without the total
moving.** Section 6.1's overload rule "includes exact match of array size as
well", and the checker compared array-NESS, so `func(float[3])` and
`func(float[4])` looked like one signature declared twice. Fixing it turned
two wrong verdicts into two honest gaps -- the count stayed at 69 and only the
diff against the parent build showed anything had happened at all.

Then two expression forms the subset had simply never had, 51 -> 41:

* **Section 5.9's sequence operator** (8 cases). `a, b` as a statement and as
  an expression: all operands evaluated, in order, value of the rightmost.
  The care is in the left operand -- an assignment, a call and `++i` go
  through the statement path, and everything else in ES 1.00 is side-effect
  free, so evaluating it as a value and never reading the result is exactly
  what "is evaluated" means.
* **Section 5.8's assignment-as-an-expression** (2 cases closed, 4 advanced).
  `x[2] = x[3] = 0.0` is a chain. The l-value is READ BACK rather than the
  stored value reused, because the spec names "the type and precision of the
  lvalue-expression"; that is only sound when re-evaluating the l-value
  cannot repeat a side effect, so `b = (x[i = i + 1] = 1.0)` keeps its gap.

Then two more, 41 -> 29, both of them a spec sentence the subset had read too
narrowly:

* **Section 5.9's `++` and `--` as expressions** (4 cases). Same store as the
  statement form; the only difference is WHEN the l-value is read -- after it
  for the prefix, before it for the postfix. This is the one place in the
  front end where getting it backwards produces no error of any kind: both
  forms type-check, both store, and the picture is off by one. Reading before
  is sound because a `LoadLocal` produces a value and the store writes the
  SLOT, which the back end's load-copy elimination is careful to keep apart.
* **Section 5.9's scalar-and-matrix arithmetic** (8 cases). `-2.0 * a` scales
  every column and is not the matrix product sitting in the same arm; the
  OPERAND TYPES tell them apart, and the checker had already recorded them.
  `matrix / scalar` is the same expansion, so the literal-divisor fold applies
  to a matrix too.

Both tests assert what compiling cannot. Swapping prefix and postfix fails
one; scaling column 0 and copying it across the others fails the second.

With the sequence operator and assignment-as-an-expression, `value`'s fallback
arm became UNREACHABLE -- every `Expr`
form now has a lowering to reach -- and was deleted. A refusal that can no
longer happen is worse than no refusal: it reads like a live safety net. The
match being exhaustive is now what stops a new `Expr` form arriving without
one.

## The link rules that need the sources

`shaderc::link` pairs the two stages by their compiled tables, which say what
a stage writes and reads -- not what it DECLARES, and not a struct member
nobody reads. `backend::link_online` runs first, on the two records' sources
through the front half (`reflect::interface`, and the vertex IR), and adds
(2026-09-23):

| rule | what happens |
|---|---|
| 4.3.5: a varying the fragment shader uses must be declared by the vertex shader | never declared: link error, "is not declared in the vertex shader" with the section |
| 4.3.5: with the same type | declared with another type, written or not: link error |
| declared, never (statically) written | legal: the fragment shader is recompiled with that varying read as 0 in the IR (`lower::zero_varyings`) before either code generator, and the program holds that shader |
| 4.3.4 / 4.5.3: a shared uniform has one type and precision | a struct uniform's type is its whole declaration, every member's name, type and resolved precision; any difference is a link error |
| 4.5.3 for a plain uniform | precision compared when both stages use it (Mesa's ES 1.00 rule) |

"Written" is read from the vertex IR, not from the tables, because the tables
differ: the PowerVR lists a declared, unwritten varying as an output whose
register nothing writes, the GC7000 omits it. Measured, HOST-ONLY (the linux
OSAL, GC7000 library): all 58 `dEQP-GLES2.functional.shaders.linkage.*`
cases link or refuse as dEQP expects -- the 14 negatives refused, including
the five that used to link (`uniform_struct_type_conflict_1`,
`uniform_struct_precision_conflict_1..4`), and `varying_4` linked where it
was refused -- and the whole 17,165-case mustpass had no other link refusal
from these rules. The PowerVR arm is pinned by `backend.rs` unit tests only.

## Stack: what a thread that compiles must reserve

The compiler's memory is two caller-sized regions (`store.rs`), but its
STACK is the calling thread's, and nobody had measured it until am62p-board
found it overflowing a 64 KiB Zephyr main-thread stack into the idle
thread's (4 of 4 runs faulted; 0 of 1 at 256 KiB). `glCompileShader` and
`glLinkProgram` run the compiler on whatever thread calls them -- Qt's render
thread, an application's main thread -- so the requirement is the
application's to meet, and ours to publish.

**Requirement: 256 KiB for any thread that calls `glCompileShader` or
`glLinkProgram` with the in-process compiler.** That is the size
am62p-board ran clean at; the measurement below puts the worst compile at
104 KiB on the host, and the margin is for what the host cannot see (an
aarch64 frame is not an x86_64 frame, and the corpus is not every shader).

Measured 2026-09-23 by `tests/stack_survey.rs` (a painted stack: each phase
runs just below a frame filled with a pattern, and the lowest byte it
overwrote is its depth), over the table's sources, every `tools/shaders`
source (ES 1.00 and the ES 3.00 Qt Quick 3D ones) and adversarial nesting at
the parser's and the inliner's own limits. HOST-ONLY, x86_64:

| phase | GC7000 build | PowerVR build | worst source |
|---|---|---|---|
| pp | 1.9 KiB | 1.9 KiB | fill_color.frag |
| parse | 48 KiB | 48 KiB | adversarial: 60 nested parentheses (corpus: 8 KiB) |
| sema | 23 KiB | 23 KiB | adversarial: 30 nested parentheses (corpus: 8 KiB) |
| reflect | 1.2 KiB | 1.2 KiB | SaturationLightness.batchable.vert |
| lower | 85 KiB | 85 KiB | adversarial: calls inlined 15 deep (corpus: 24 KiB, simplequad_filmic.frag) |
| **whole compile, back end included** | **102 KiB** | **74 KiB** | dynidx.frag / imageparticle_colored.batchable.vert |

(`opt-level = 1`; `--release`, the images' `opt-level = 2`, reads within a few
KiB of these.)

Re-measured 2026-09-24 at 634be0b4 (the ES 3.00 built-ins and the
destinations resolved once), HOST-ONLY, x86_64: lower 86 KiB on both builds
(adversarial inlining, 15 deep; the new `LRef` paths are out of line and a
call's `Bound` keeps its `LRef` in the arena, or it was 99 KiB); **whole
compile 96 KiB on the GC7000 build and 93 KiB on the PowerVR build**, both
now at the adversarial inlining shape rather than a corpus shader. The
PowerVR figure rose from 74 KiB because the adversarial shape now reaches
its back end. Still under the 160 KiB tripwire and the 256 KiB requirement;
the board figure is to be re-checked on the next Zephyr gl_test image. So **the back ends dominate** on real shaders -- the front
end's own worst on the corpus is 24 KiB -- and the front end reaches their
size only when a shader nests at the limits the parser (`MAX_DEPTH`) and the
inliner (`MAX_INLINE_DEPTH`) already enforce, refusing by name beyond them.
The survey asserts tripwires (front end at most 96 KiB, a whole compile at
most 160 KiB), so a change that moves the requirement fails the gate rather
than a board.

The Zephyr samples that can be built with the in-process compiler set
`CONFIG_MAIN_STACK_SIZE=262144`, and `zephyr/Kconfig`'s
`YAKOGL_SHADERC_ONLINE` help says the same.

## Measuring it

Hand-picked unit tests say what their author already knew. `tools/online_oracle.py`
is the corpus: it reuses `tools/es100_oracle.py`'s dEQP `.test` parser, asks
`yakogl-tools online-check` (the same JSON protocol `shader-check` speaks, so
the two front ends are comparable), and reports three columns rather than one
score, because a single "N of M" over a corpus whose failures mostly need
passes that do not exist yet would measure nothing:

* **false rejects** -- cases dEQP says must BUILD that the front end refuses.
  Each one is a defect in code that exists. **This number must be 0**, and the
  script exits non-zero if it is not.
* **must-fail rejected** -- of the cases that must fail to compile, how many
  the front end already refuses. Coverage, not a pass mark; it rises as passes
  land.
* **excluded** -- cases needing a verdict the front end cannot give (a link
  error) or not ES 1.00. `online-check` answers a link request with a *service*
  error, never `ok`, which is what keeps this honest: answering `ok` would turn
  every `link_fail` case into a silent pass.

### The three zeros this document used to report were not true

Run at 3527bba, dEQP-GLES2 `data/gles2/shaders`, this section once recorded:

```
at 3527bba (lexer + preprocessor)
TOTAL                    must-build 1778   false rej 0   must-fail 1005   rejected 248   excl 10

with the parser
TOTAL                    must-build 1778   false rej 0   must-fail 1005   rejected 497   excl 10

with semantic analysis
TOTAL                    must-build 1778   false rej 0   must-fail 1005   rejected 836   excl 10
```

**The false-reject column was measuring half the compiler.** `online-check`
ran preprocess, parse and `sema` and stopped: it never called `lower`. So a
shader the analyser accepts and the lowering refuses -- a rejection in the
real pipeline, and a false reject by this table's own definition -- was
counted as a pass. The column that "must be 0" read 0 because it could not
see the pass that was doing the refusing.

It is the defect this compiler keeps growing, in the instrument instead of the
code: two things share a representation (here, a verdict) and the reader takes
one for the other. The tool's own doc comment said "as much of the front end
as exists", which was true when it was written -- the front end *was* the
parser -- and nobody re-read it as the front end grew. Fixed 2026-09-22; the
function now runs all five passes and names the one that refused, with a
comment saying to keep it in step with `compile_to_ir`, because a pass added
there and not here is a pass that silently stops being measured.

The honest readings, same data, same 1,778 must-build programs:

```
before file-scope variables (the state the zeros above describe)
TOTAL                    must-build 1778   false rej 1396   must-fail 1005   rejected 935   excl 10

with file-scope variables
TOTAL                    must-build 1778   false rej  744   must-fail 1005   rejected 930   excl 10

with vector comparison
TOTAL                    must-build 1778   false rej  160   must-fail 1005   rejected 930   excl 10
```

The rule this cost us, and the one worth keeping: **before believing a
number, establish that the thing being counted could have appeared in what was
measured.** The check here took one command -- feed the instrument the exact
shader the gap was supposed to refuse and see whether it says `ok`.

#### The second column, and how a rule gets landed

The false-ACCEPT column -- cases dEQP says must not compile that we compile --
was long printed as "rejected N of M (coverage, not a pass mark)", and phrasing
it that way is why nobody read it as defects. Each one is a rule the compiler
does not have, and until it does, a shader a conformant implementation refuses
compiles here, ships, and fails somewhere else.

Working it down has a method, and the method is the point rather than the
score:

1. **Name the rows first**, from the oracle, before writing any code.
2. **Find the sentence in the specification**, and quote it in the code. There
   is no copy of the spec in this repository, so it is fetched and extracted:
   `pdftotext -layout GLSL_ES_Specification_1.00.pdf build/glsl_es_100.txt`.
   Reading the grammar productions settled two rules that prose alone did not.
   Do not write a rule from memory of what GLSL does -- ES 1.00 differs from
   desktop GLSL exactly where habit is strongest.
3. **Read the false-REJECT column first** after the change. A new verdict that
   refuses a legal shader is worse than the false accept it closed.
4. **Diff the accepted SET against a build of the parent commit**, never
   reason about which cases moved from a total. Build both binaries, dump both
   sets, `diff`. This has twice found what a total hid: four cases silently
   lost, and a change that moved nothing at all.
5. **Show the test failing with the fix reverted.**

Scored that way, the column went **75 -> 0** on 2026-09-22, in thirteen
commits, with false rejects **69 throughout** and both offline tables
unchanged. The last seven:

```
scoping.test   (10 cases, 5 rules, 4 commits)
  4.2.2 + the grammar   a for/while body is statement_no_new_scope: it SHARES
                        the scope holding the loop's own declaration
  4.2.6 + 4.2.7         one name space per scope; a structure name was in no
                        table at all, so it could never collide
  4.2.2                 a global name's scope starts where it is declared, so
                        a call above the callee is out of scope
  4.2.6 + 10.10         a built-in's SIGNATURE cannot be redeclared, though
                        the built-in may be overloaded

functions.test (10 cases, 5 rules, 4 commits; 2 closed by the scoping work)
  6.1                   a return agrees with its function's return type, both
                        directions of the one sentence
  4.3                   "Local variables can only use the storage qualifier
                        const"
  6.1                   a return type may not be, or contain, an array

and the last six cases, one rule each
  8.7                   bias is fragment-only and "Lod" is vertex-only, told
                        apart by ARITY (4 cases; 2 already refused, but not
                        for this reason -- checked, not assumed)
  7.2                   gl_FragColor or gl_FragData, never both, where a
                        "static assignment" is defined as text and not as
                        flow: under `if (false)` and in an uncalled function
                        both count (3 cases, one rule)
  3.7                   no user-defined name begins with "gl_" (one line in
                        `declare`, covering locals, globals, functions,
                        structures and parameters at once)
  4.6.1                 only a varying or a built-in may be invariant
  3.4                   "GL_" is a reserved macro prefix -- HALF the sentence,
                        because dEQP's own must-PASS case defines `__VALUE__`
                        and the must-pass side wins
  3.4                   #extension comes before any non-preprocessor token
```

Three of those four scoping defects are the same shape, and it is the shape
this codebase keeps producing: **a rule written as a list of the cases someone
thought of.** `attribute` and `varying` were refused on a local and `uniform`
was not; a function and a variable shared a name space and a structure did
not. The spec states each of them once, generally. A rule written the way the
spec states it cannot be short by one.

`online_oracle.py` now also groups the false rejects by construct, each group
keeping one whole example, so the column that must be 0 arrives with the work
list that would make it 0. The reason is the message with its line and token
stripped, because

    0:15: 'in0' : reading this is not lowered yet
    0:10: 'var' : reading this is not lowered yet

are one missing pass and not two.

Semantic analysis took the must-fail count from 497 to 836 in one step,
because it is what finally rejects `invalid_implicit_conversions.test` --
**all 256** of its cases, the group that exists because GLSL ES 1.00 has no
implicit conversions in either direction and the shipping pipeline judges the
shader as GLSL 450, which has them.

Its one false reject was a constant-expression gap, not a type-system one:
section 5.10 lets a constant expression contain a built-in call whose
arguments are all constant expressions, and `constant_expressions.array`
sizes an array with `int(max(-1.0, 2.0))`. The folder stopped at literals and
operators; it now folds the scalar constructors and `min`, `max`, `abs`,
`sign`, `floor`, `ceil` and `clamp`, and still refuses a *float* constant as
an array size, which section 4.1.9 requires.

The parser's first run against the corpus found one real gap and cost eight
rejections that were wrong for a second reason. Section 9's `condition` is
`expression | fully_specified_type IDENTIFIER EQUAL initializer`, so
`while (bool done = step())` is legal and declares `done` in the loop's scope;
the parser accepted only an expression. That refused four `scoping` cases
dEQP says must build -- and it had also been "rejecting" eight `scoping`
cases that must fail, for the wrong reason. Fixing it took the must-fail
count from 505 to 497, which is the honest direction: eight rejections that
happened to be correct verdicts reached by a wrong argument stopped counting.

The first run of it, before those numbers, found **four real defects** that 55
hand-written tests had not, each now pinned by a test that was shown to fail
with its fix reverted:

1. A block comment spanning lines ended the directive it was inside. A comment
   is one space (C's translation phase 3), so `#define F(a,b)(a /* ...\n... */
   +b)` is one directive; ending it there truncated the replacement list, and
   the next identical `#define` then looked like a redefinition with different
   text. dEQP `function_redefinition_3`, `redefine_object_multiline_comment`.
2. `defined` was folded only before macro expansion, so `#define AAA
   defined(BBB)` followed by `#if !AAA` left a `defined` in the expression and
   it was read as an undefined identifier. It is now folded again afterwards.
   dEQP `conditional_inclusion.basic_2`.
3. A character outside the source character set was a lexical error, which
   made a `#pragma` of arbitrary bytes fail to compile -- and dEQP
   `pragmas.pragma_unrecognized_token` expects that shader to build. Such a
   character is now a token (`Tok::Unknown`); the preprocessor rejects one that
   reaches live code, which is where it is actually wrong.

`tools/es100_oracle.py` was re-run unchanged afterwards and still reports
**2787 of 2793**. Worth recording: all six of its remaining mismatches are
compile verdicts in the preprocessor group, and the online front end gives the
right answer for all six -- the double-underscore name, `__FILE__`, and
`defined` from a macro are exactly what glslang refuses and this front end does
not.

## The acceptance test: agreeing with the offline table

`tests/online_reflection.rs` is M2's acceptance. For every GLSL ES 1.00 source
in the table compiled into the build, it preprocesses, parses and reflects,
then compares against that source's row -- looked up by the SHA-256 of the
source text, which is the key the table itself uses.

| | GC7000 (`gpu-vivante`) | PowerVR |
|---|---|---|
| ES 1.00 sources agreeing | **121 of 121** | **221 of 221** |
| not ES 1.00 (3.00 or 4.50) | 185 | 296 |
| mismatched | 0 | 0 |

### What "equal" can mean, and what it cannot

Only the **block uniforms** can be compared as equality: `block_uniforms`
reads the declarations, so every declared one has a row. A **sampler**, an
**attribute** and a **varying** reach the table only if the code generator
kept it -- they are register wiring, and an unused one gets no register. So
the claim the test makes is the strongest a front end can make: *every row the
table has, the front end has, with identical fields and in the same order.*
Rows the front end has beyond that were eliminated later; they are counted and
printed (32 samplers, 27 attributes or varyings on the GC7000) rather than
quietly accepted. It is also the contract M3 needs: the back end filters this
list by what it kept, and a name it keeps must be one the front end numbered.

### Three findings about the tables, each detected by a rule

None of these is a front-end defect, and none is excluded by name -- each is
recognised by a rule, so a new one is reported rather than predicted and a
fixed one stops being counted on its own.

1. **A table row that disagrees with itself.** `qt3d/qtappshaders_1.vert`
   declares `attr_pos, attr_norm, attr_uv0, attr_textan, attr_binormal`; its
   third `ShcAttrib` row reads `location: 0, components: 0, name: "attr_pos"`,
   and `attr_uv0` has no row at all. `glGetActiveAttrib(program, 2, ...)`
   therefore names `attr_pos` twice and never names `attr_uv0`. The mechanism
   is `gen_shader_table.py`'s attribute loop: when the compiler's slot does
   not map to a declared location it falls back with `loc = e["slot"]` and
   carries on, so an input it cannot place becomes a row pointing at whatever
   location that number happens to be. It has no outcome for "this attribute
   was eliminated" -- no representation for absence, so a value
   indistinguishable from a real one is substituted.
2. **Shaders requiring an extension the implementation does not advertise.**
   Fourteen PowerVR-table sources `#extension GL_OES_standard_derivatives :
   require`, and `src/gl/state.rs` does not list it -- its comment says every
   name there "is something this library really does". A shader that
   `require`s an absent extension must fail to compile, so the front end
   refuses them, correctly. Either the extension should be advertised (the
   derivative built-ins exist on both GPUs) or those shaders do not belong in
   the table; that is the GL core's call, not the front end's.
3. **Shaders that declare `#version 100` and use a later version's type.**
   Five sources use `sampler2DArray`, which arrived in ES 3.00. The offline
   pipeline never noticed: it rewrites the text without checking types and
   hands it to glslang as GLSL 4.60, where the type exists.

## Re-measuring, from nothing

Every number in this document comes from one of three instruments. None needs
a board; all three run on the host.

```sh
# 1. dEQP: the false-reject column, and the work list under it.
#    Needs VK-GL-CTS checked out ($VK_GL_CTS, or --cts).
tools/online_oracle.py                  # the whole gles2 shader set
tools/online_oracle.py conditionals     # one file
tools/online_oracle.py --show 20        # the first 20 false rejects, in full

# 2. The two offline tables: 139 of 142 (GC7000) and 235 of 240 (PowerVR).
#    The feature selects which table; each prints its refusals by construct.
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
      --test online_lowering -- --nocapture
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
      --test online_lowering -- --nocapture

# 3. The shipping pipeline, which must not move: >= 2787/2793.
tools/es100_oracle.py

# 4. What the accepted shaders COMPUTE: every `expect pass` value case in the
#    same corpus, compiled online and run by the IR evaluator (eval.rs) with
#    dEQP's own inputs and references. 0 wrong of 7,317 sub-cases (7,299 pass);
#    the front end of 9c0f78a scores 7,203 wrong (a NaN colour: bools into
#    vec4(RES, RES, RES, 1.0) unconverted). Front end only, by construction.
tools/online_eval_oracle.py

# 5. dEQP-GLES2 itself on this host, against the linux-OSAL library: the
#    compile and link outcome of every C++-generated shader (shaders.*,
#    uniform_api, texture, ...) the .test corpus cannot reach. HOST-ONLY:
#    every read-back is black, so no render verdict from it is ever quoted.
#    100 cases per process by default, for crash isolation; CHUNK=100000
#    runs a group in one process (the keep region is freed on delete).
tools/cts/host-deqp-run.sh              # the whole mustpass list
tools/cts/host-deqp-run.sh shaders.struct
CHUNK=100000 tools/cts/host-deqp-run.sh operator.binary_operator

# And the gate, which runs clippy and the tests in every configuration.
tools/check.sh
```

Two habits that paid for themselves here:

* **A test that pins a fix is shown to fail with the fix reverted**, and the
  revert is described in the test's own doc comment so the next reader can
  repeat it. Twice this caught a test that passed for the wrong reason. Once
  it caught a *revert* that did not revert: setting the leading `if` of an
  `if / else if / else` chain to `false` leaves the other arms live, so the
  guard looked unnecessary when it was simply still there. A revert that does
  not revert proves nothing, and the tell is that the result disagreed with
  the prediction written down beforehand.
* **Predict the number before running.** A prediction that misses is worth
  more than one that lands: the branch-constant test expected 3 constants and
  got 4 (the comparison's own `0.5` was forgotten), and the ES 3.00 refusals
  were predicted to move into a version bucket when in fact they left the
  corpus -- they were not ES 1.00 at all.

## Files

| file | what |
|---|---|
| `arena.rs` | the bump arena and `AVec`; the whole memory model |
| `lexer.rs` | tokens, keywords, the ES 1.00 / ES 3.00 lexical differences |
| `pp.rs` | directives, conditional groups, macro expansion, the `#if` evaluator |
| `ast.rs` | the syntax tree: `Copy` nodes, `u32` ids, no resolved types |
| `parser.rs` | recursive descent, section 9's grammar and section 5.1's precedence |
| `sema.rs` | the ES 1.00 type system: scopes, types, lvalues, the stage rules |
| `reflect.rs` | the uniform, attribute and varying rows, the std430 block, and constant folding |
| `lower.rs` | the tree to `ir::Module`: inlining, slots, structured control flow, the six decisions above |
| `builtin_call.rs` | the *language* half of the built-in library; names no GPU (the machine half is `<backend>/builtins.rs`) |
| `tests/online_reflection.rs` | the acceptance test above |
| `tests/online_lowering.rs` | the corpus floors, the refusal histogram, and the corpus-wide assertions |
| `tools/yakogl-tools/src/onlinecheck.rs` | the front end as a batch service, `shader-check`'s protocol |
| `tools/online_oracle.py` | the dEQP measurement above |
| `tools/online_eval_oracle.py` | the value measurement above (the IR evaluator) |
| `tools/cts/host-deqp-run.sh` | dEQP-GLES2 on the host: compile and link outcomes only |

The gate (`tools/check.sh`) grew a sixth configuration, `host-test,shaderc-online`.
It is the only row that compiles `no_std` code with no allocator behind it, so
a stray `std` import or `alloc` dependency in the front end fails there and
nowhere else -- on Zephyr it would not surface until the image failed to link.
