<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The online compiler's IR: the contract between the front end and the back end

One file, because three people write against it: the front end (M2,
es100-frontend) builds it, the code generator (M3, gles2-core) reads it, and
the instruction encoders (M1) are what the code generator emits
through. `src/shaderc/online/ir.rs` implements exactly this and nothing else.

Status: **agreed**. es100-frontend proposed it, gles2-core proposed a scalar
alternative, the vector form won for the reason under "Why vectors and not
scalars", and the team lead ruled. One item is marked OPEN at the end; it
blocks neither side.

## The five decisions

### 1. Vector values, 1 to 4 components, with a swizzle on every operand

A value is 1..4 components of one scalar type. Every use of a value carries a
4-entry swizzle. This is NIR's ALU model.

**Why vectors and not scalars.** The first target is the Vivante GC7000UL,
which is a vec4 machine with write masks and swizzles; the PowerVR USC is
scalar. Scalarising a vector IR for the USC is a mechanical per-component
expansion. Going the other way — vectorising a scalar IR for the GC7000 — is a
pattern-matching pass over sequences that no longer say they belong together,
and it would have to recover exactly the information the front end had thrown
away. One IR serves both only if the vector form is the one that is stored.

### 2. Structured control flow, and no phi nodes

GLSL has no `goto`, so the parser's tree is lossless and is kept: `If`, `Loop`,
`Break`, `Continue`, `Return` over ranges of instructions. Rebuilding structure
from a flattened CFG is work neither end needs to do.

Values are SSA **within straight-line code**. Anything whose value crosses
control flow — a variable assigned in one arm of an `if`, a loop accumulator —
goes through a `Local` slot with `LoadLocal` / `StoreLocal`. That is instead of
phi nodes: the front end emits the stores the source already has, and the code
generator gives each slot a register. This is NIR before `lower_vars_to_ssa`.
Both proposals arrived at it independently, which is some evidence it is right.

### 3. The front end inlines every call

ES 1.00 forbids recursion, so inlining always terminates. The IR has one body.
The back end never sees functions, `out`/`inout` parameters, or a call stack.

### 4. Matrices, structs and arrays are not IR types

A matrix value is its columns, as separate vector values. A struct uniform is
flattened into its members, exactly as the offline path already flattens them —
that is what `ShcUniform::inner` encodes. Arrays survive only as
`LoadUniformIdx`, and as `Local` slots with an index operand.

### 5. No heap

The front end runs in a caller-supplied bump arena (`&mut [u8]`,
`src/shaderc/online/arena.rs`). Every IR node is POD, every reference is a
`u32` index, every list is a flat slice in the arena. The crate has no
allocator and the product targets are Zephyr and bare metal, so this is not a
preference.

The arena the front end is lent comes from `src/shaderc/online/store.rs`:
`with_scratch()` per compile. Anything that must outlive the compile — the
instruction words, the `ShcUniform` rows, the name bytes — is copied by the
back end into `store::keep()`, which outlives the compile because the
`ShcShader` the GL core keeps points into it; each shader's share is freed when
the GL core releases its last copy (`backend.md`, "The free path").

## The types

```rust
pub struct ValueId(pub u32);   // SSA: 1..4 components of one scalar type
pub struct LocalId(pub u32);   // mutable slot: (Type, array_len)
pub struct InstrId(pub u32);

pub enum ScalarType { F32, I32, U32, Bool }                 // U32: ES 3.00
pub struct Type { pub scalar: ScalarType, pub comps: u8 }   // 1..4

pub struct Src { pub val: ValueId, pub swizzle: [u8; 4] }   // no neg/abs: backends fold those

pub enum Op {
    ConstF32([f32; 4]), ConstI32([i32; 4]), ConstBool([bool; 4]),
    Alu(AluOp, [Src; 3]),                                   // component-wise; arity from the op
    LoadUniform    { dword_off: u16, comps: u8 },
    LoadUniformIdx { base_dw: u16, stride_dw: u16, len: u16, index: Src, comps: u8 },
    LoadInput      { slot: IoSlot, comps: u8 },
    StoreOutput    { slot: IoSlot, src: Src },
    LoadLocal      { local: LocalId, index: Option<Src>, comps: u8 },
    StoreLocal     { local: LocalId, index: Option<Src>, src: Src, write_mask: u8 },
    Tex            { dim: TexDim, shadow: bool, proj: bool, ret: ScalarType, sampler: u8,
                     coord: Src, lod: TexLod, offset: Option<[i8; 3]> },   // IR v1, below
    TexSize        { sampler: u8, dim: TexDim, lod: Src },
    Discard,
}

pub enum IoSlot {
    Attrib(u8), Varying(u8),              // by LOCATION, as the table already keys them
    Position, PointSize,                  // vertex outputs
    FragColor(u8), FragDepth,             // fragment outputs
    FragCoord, FrontFacing, PointCoord,   // fragment inputs
    VertexId, InstanceId,                 // ES 3.00 vertex inputs, one int lane each
}

pub enum Node {
    Instr(InstrId),
    If { cond: Src, then_: Range, else_: Range },
    Loop { body: Range },
    Break, Continue, Return,
}
```

`LoadUniformIdx` clamps its index to `[0, len)`. ES 2.0 leaves an out-of-range
index undefined, and reading elsewhere in the shared registers — where another
shader's uniforms or a sampler descriptor sit — is not an acceptable
"undefined".

### `StoreLocal`'s write mask: destination lane *i* takes `src.swizzle[i]`

**Decided, and measured rather than agreed.** `write_mask` says WHICH
destination lanes are written (bit 0 = x … bit 3 = w). What it does not say on
its own is which source lane each of them takes, and there are two readings:

1. destination lane *i* takes `src.swizzle[i]` — the swizzle is indexed by the
   DESTINATION lane; or
2. the source's lanes are consumed in order across the set bits.

**The first is what the hardware does.** The evidence is `cube.vert`'s shipped
GC7000 blob, whose second instruction is

```
mov  t1.__zw, void, void, u0.xxxy
```

Lane z takes `swizzle[2]` = x and lane w takes `swizzle[3]` = y, so `z = u0.x`
and `w = u0.y` — for a position, `z = 0.0` and `w = 1.0` out of a constant
register holding `{0.0, 1.0, …}`. Under the second reading `w` would take
`u0.x` = 0.0, and the vertex would have `w = 0`.

gles2-core measured it and implemented it in **b78d369**, where a slot is a
virtual register and a store is a write to it under this mask. The front end
emits it in six places (`src/shaderc/online/lower.rs`): the constructor
assembled from several values, the lvalue swizzle, the matrix local, the
early-return result slot, the `out` parameter copy-back, and the run-time
vector lane select. Each has a test that spells out the masks and the swizzles
rather than counting instructions, because the two readings produce the same
number of instructions and a different picture, and no later pass can tell
them apart.

An argument that fills destination lanes `base..base+k` therefore places its
own lane *j* at `swizzle[base + j]`, and the lanes outside the mask are never
read. `vec4(v2, v2)` is two stores, the second with `swizzle[2] = 0` and
`swizzle[3] = 1`.

## The opcode kernel

The front end lowers the whole ES 1.00 builtin library to these. The back end
implements these and only these.

| Class | Ops |
|---|---|
| float arithmetic | `Fadd Fmul Ffma Fmin Fmax Fneg Fabs Fsign` |
| float transcendental | `Frcp Frsq Fsqrt Fexp2 Flog2 Fsin Fcos` |
| float rounding | `Ffloor Fceil Ftrunc Ffract` |
| derivatives (fragment only) | `Fddx Fddy` |
| **dot products** | **`Fdot2 Fdot3 Fdot4`** — see below |
| float compare | `Flt Fle Feq Fne` |
| integer | `Iadd Imul Idiv Imod Ineg Ishl Ishr Iand Ior Ixor` |
| integer compare | `Ilt Ile Ieq Ine` |
| conversions | `B2f B2i F2i I2f F2b I2b` |
| boolean | `Bnot Bany Ball` |
| select | `Bcsel` |
| **ES 3.00 integer** | `Udiv Umod Ushr Ult Ule Imin Imax Umin Umax Inot U2f F2u` |

`Iadd Imul Ineg Ishl Iand Ior Ixor Ieq Ine` serve `uint` as well as `int`: the
bits are the same, and the ES 3.00 row is exactly where signed and unsigned
differ (`Ishr` is arithmetic, `Ushr` logical) plus the two operations the
language added. `int`<->`uint` conversions and `floatBitsToInt`/`intBitsToFloat`
and their unsigned forms are **not** operations: the front end reuses the same
`ValueId` (a copy would cost a register on the GC7000).

Expanded by the front end, and therefore **not** opcodes: `mat*vec` (to an
`Fmul`/`Ffma` chain), `pow` (to `Fexp2`/`Flog2`), `mod` (to `Ffract`), `clamp`
(to `Fmin`/`Fmax`), `mix` (to `Ffma`), `step`, `smoothstep`, `atan`, `asin`,
`acos` (to polynomials), `matrixCompMult`, and the relational vector builtins.

### Why the dot products stayed

The original proposal expanded `dot` — and with it `length`, `normalize`,
`distance`, `reflect`, `refract`, `faceforward` — into `Fmul`/`Ffma` chains.
That is right for a scalar machine and wrong for the GPU that is now first.

Measured, from pvrgpu-platform's extracted instruction set
(`docs/online-shaderc/isa/gc7000-isa.json`): the GC7000 has **`DP2`, `DP3` and
`DP4`** as native opcodes, alongside `MAD`, `RCP`, `RSQ`, `SQRT`, `LOG`, `EXP`,
`SIN`, `COS`, `SELECT` and `SET`. (`gc7000-isa.md` notes that `dp4` is absent
from the sampled corpus while `dp3` is present — a fact about that corpus, not
about the hardware.)

Expanding a dot product into a chain and then recognising the chain again in
the code generator is exactly the pattern-matching pass that the vector-versus-
scalar argument rejected, one level down. So `Fdot2`/`Fdot3`/`Fdot4` stay in
the kernel. Everything built on a dot is still expanded by the front end, but
down to an `Fdot*` rather than past it.

On the USC an `Fdot*` scalarises into the same `Fmul`/`Ffma` chain the front
end would have written, so keeping it costs the PowerVR back end one expansion
it was going to do anyway, and saves the GC7000 back end a pass it cannot
afford.

## Reflection is a separate output, and it is the front end's

Reflection is not part of the IR. It is produced alongside it, and the
ownership split is what stops two people writing the same field:

| Field | Owner |
|---|---|
| `ShcUniform` — all seven fields (name, dword_off, dwords, gl_type, array_len, binding, inner) | front end |
| `ShcAttrib::location`, `name`, `gl_type`, `components` | front end |
| `ShcAttrib::vi_reg`, `format` | back end |
| `ShcVarying::location`, `name`, `interp`, `components` | front end |
| VS `ShcVarying::vtxout_off` | back end |
| FS `ShcVarying::vtxout_off` | `shaderc::link`, unchanged |
| `uniform_dwords` (the std430 block size) | front end |
| `uniform_sh_dwords`, `temps`, `meta_off`, `num_outputs`, `usc*`, `vs_output_size`, `vs_psize_off`, `blend_consts_off`, `uses_w` | back end |

Everything in the back end's column is a result of register allocation. The
front end cannot know any of it and must not guess.

**Row order is load-bearing** and is the front end's: attributes sorted by
location, uniform rows in declaration order after struct flattening, because
`glGetActiveUniform` indices follow them. The back end does not sort.

**One known asymmetry, handled rather than papered over.** The offline path
drops declared-but-unused samplers from the uniform rows, using the compiler's
post-dead-code descriptor list. The front end can only report "statically
referenced", which is not the same question. So the front end emits every
declared sampler with a `statically_used` flag, the back end applies the
filter, and the oracle harness compares with the filter applied and reports any
source where the two answers differ. Neither side pretends the difference is
not there.

## How this is measured

Two harnesses, asking different questions. Neither substitutes for the other:

| | corpus | question |
|---|---|---|
| `tools/online_oracle.py` | dEQP-GLES2's `.test` cases | does the front end **accept and reject** the right shaders? |
| `tests/online_oracle.rs` | the offline table's own sources | does the compiler **produce** what the table produces — reflection now, instructions later? |

A shader can pass one and fail the other in either direction: compiled but with
the wrong uniform offsets, or the right output for the shaders in the table and
the wrong verdict on one that should not compile at all.

## IR v1: GLSL ES 3.00 (agreed 2026-09-23)

Proposed by the front end, answered by both back ends, reviewed by the lead.
What is in `ir.rs` today is marked; the texture half lands with the ES 3.00
built-ins (step 4), in one change with both back ends, because it replaces a
type they both match on.

**In `ir.rs` now.** `ScalarType::U32`; the ES 3.00 integer row of the kernel
above; `IoSlot::VertexId` and `IoSlot::InstanceId` (one int lane each; the
GC7000 feeds both from one register, which is its business);
`INTERP_CENTROID = 3` beside `INTERP_FLAT` (a back end without the distinction
treats it as smooth and says so). Both back ends refuse the new operations by
name until they select them; the evaluator implements all of them.

**Undefined results are unmeasured.** Section 5.9 leaves a shift by a negative
amount or by 32 or more, a division by zero, and a negative float converted to
`uint` undefined. The evaluator stops the run on those instead of inventing a
value, so the oracle never holds a back end to one.

**Division stays an IR op.** `Idiv Imod Udiv Umod` are in the IR on both GPUs
though neither has an integer divide: the PowerVR back end transcribes Mesa's
`nir_lower_idiv`, the GC7000 writes its own exact 32-bit sequence. Where a
back end has no answer yet it refuses by name.

**Flat is step 1, not later.** ES 3.00 requires every integer varying to be
`flat`, and Qt Quick 3D declares 136 of them. The provoking vertex is the LAST
vertex (ES 3.0, "Flatshading"); the GC7000 sets it on every draw, the AM62P's
is in the TA/PPP state and am62p-board measures it.

**Fragment outputs** are `IoSlot::FragColor(n)` for `layout(location = n) out`,
and `IoDecl::gl_type` (float, int or uint vector) is all a back end needs for
the output format; the attachment formats are the driver's.

**Textures (in `ir.rs` since 2026-09-23; agreed by both back ends).** IR v0's
`TexKind` is replaced by an orthogonal shape:

```rust
pub enum TexDim { D2, D3, Cube, D2Array }
pub enum TexLod { Implicit, Bias(Src), Lod(Src), Grad { ddx: ValueId, ddy: ValueId }, Fetch(Src) }
Op::Tex { dim: TexDim, shadow: bool, proj: bool, ret: ScalarType /* F32|I32|U32 */,
          sampler: u8, coord: Src, lod: TexLod, offset: Option<[i8; 3]> }
Op::TexSize { sampler: u8, dim: TexDim, lod: Src }   // -> ivec2 (D2, Cube) or ivec3 (D3, D2Array)
```

- `coord` lanes: D2 xy; D3 and Cube xyz; D2Array xy and the layer in z. The
  layer is a raw FLOAT: the back end rounds (to nearest even, section 8.8)
  and clamps it -- the PowerVR in the shader, as pco does, since that core
  has no hardware array indexing; the GC7000 as etnaviv does -- and the front
  end never rounds it, so there is no double rounding. A shadow reference
  follows in z for D2 and in w for Cube and D2Array.
- `proj` (D2 and D3 only) puts the divisor in lane w, ALWAYS: a vec4
  coordinate has it there, a vec3's z is repeated into w. This moved: IR v0's
  `Tex2DProj` divided by lane z, and a vec4 arrived as (x, y, w, w).
- `Fetch` coordinates are integer lanes of the base width (ivec2 for D2,
  ivec3 for D3 and D2Array); there is no cube fetch in ES 3.00.
- The value is four lanes of `ret`, or ONE F32 lane for a shadow look-up.
- `Grad` names its gradients by `ValueId`, read with the identity swizzle at
  the coordinate's base width (2 for D2 and D2Array, 3 for D3 and Cube); the
  front end materialises them at that width. Two `Src`s would make
  `size_of::<Op>()` 40 bytes instead of 32 -- measured, and on every
  instruction of every module -- where it was 28 with IR v0's `Tex`.
- The front end never hands `Implicit` or `Bias` to a vertex module: an
  implicit look-up there is `Lod(0.0)`, the base level (section 8.8).
- `offset` is a compile-time constant, as the language requires, never on a
  Cube; the GC7000 expands it, the PowerVR has it natively. On the GC7000 a
  level or bias rides in coord.w, so shadow with a level would collide with
  the reference there -- no shader this compiler serves uses that pair yet.
- `TexSize`'s `lod` is one int lane; the GC7000 shifts the base size right
  by it and clamps to at least 1.
- The ES 1.00 kinds map one to one: `Op::es100(kind, ..)` builds one from an
  `Es100Tex` and `Op::es100_tex()` reads it back (`Tex2D` = `{D2, Implicit}`,
  `Tex2DBias` = `{D2, Bias}`, `Tex2DLod` = `{D2, Lod}`, `Tex2DProj` = `{D2,
  proj, Implicit}`, `TexCube` = `{Cube, Implicit}`); the cube level and bias
  forms that were on hold are `{Cube, Lod | Bias}`. A back end matches the
  ES 1.00 shapes through `es100_tex()` and refuses every other shape by name
  until it selects it; `Op::tex_srcs()` lists what a look-up reads, for the
  passes that walk uses.
- `Op` stays `Copy` with `TexLod` inline: selection copies ops by value.

**Not in the IR.** `switch` is lowered by the front end to a `Loop` holding
an if-chain and a `Break`. The float built-ins ES 3.00 adds (`round`,
`roundEven`, the hyperbolics, `isnan`, `isinf`, `modf`, `transpose`,
`inverse`, `determinant`, `outerProduct`) and the pack/unpack family are
front-end expansions. Uniform blocks change reflection and the GL core, not
the IR; they are a separate proposal (Qt Quick 3D declares none: its UBOs
arrive flattened into struct uniforms).

## OPEN

**`clamp` may not want expanding either.** The extracted GC7000 instruction set
contains `clamp0_max`, which suggests a native clamp against zero and a
maximum, and `clamp(x, 0.0, hi)` is common in shader code. Nobody has decoded
that opcode's operands, so the front end expands `clamp` to `Fmin`/`Fmax` for
now, which is safe. If it turns out to be a real clamp it joins the kernel the
same way the dot products did — a decision for whoever decodes it, and not a
reason to hold the IR.
