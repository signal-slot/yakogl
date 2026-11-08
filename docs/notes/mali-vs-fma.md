<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex fused multiply-add (`ffma`) the vertex walk refuses

The vertex half of `notes/mali-alu-ops.md`'s `ffma` row, for the glmark2
vertex shaders that `src/shaderc/online/mali/nir.rs` refuses with
`MaliError::FusedMultiplyAdd` (#33, plan #336 `ana-vsfma`). Written from the
code on `e9e72f05`, the glmark2 sources (22c527cb) and Mesa 26.2.1 run by
`pan_shc` (`tools/mali/README-pan_shc.md`). Where a point is not measured it is
marked **inferred**. No code is changed by this note.

## What is refused, and the words

The refusal is one arm of the vertex `IrOp::Alu` match (`nir.rs`, the arm
`AluOp::Ffma => return Err(MaliError::FusedMultiplyAdd)`), reached for every
`ffma` of the front end's IR that is not a matrix-product term
(`is_mat_term`, `nir.rs`: source 0 is a `mat4` uniform column, source 1 a splat,
source 2 an `Fmul`/`Ffma`). The text (`mod.rs`, `MaliError::FusedMultiplyAdd`):

    online shaderc: the code generator refused this vertex shader: Mali(Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet)

The front end makes an `ffma` itself in two places: `builtin_call.rs` for
`cross`, `mix`, `reflect`, `smoothstep`, `mod` and the transcendental
polynomials (lines ~270-1420), and its matrix product `mat * vec` (a `Fmul`
then `Ffma` per column, `dot_of` for `dot`). The vertex walk takes the second
only when the matrix is a uniform column (`is_mat_term`).

## Which glmark2 vertex shaders refuse

The corpus is `tests/glmark2_corpus.rs`'s: `tools/hostharness/glmark2/run.sh
"$GLMARK2_SRC"` writes `build/glmark2-host/shaders/` (GPL text, never committed).
That script could not be run in this session (the command was not permitted),
and the corpus test has no `gpu-mali` build. So the list below is **an
approximation**: a throwaway test (not committed) compiled each raw
`data/shaders/*.vert` of glmark2 22c527cb in the Mali vertex stage, with a
header of `const` stand-ins for the lighting values glmark2 injects, and an
`eprintln!` at the arm printing what the three sources are (reverted). The plan
(`corpus-tally`) counts **7**; the raw files give these, all with the text above
as their first refusal:

| glmark2 file | Distinct source | What the refused `ffma` is |
|---|---|---|
| `ideas-lamp-lit.vert` = `ideas-logo.vert` (same md5) | 1 | `projection * modelview * curVertex`, `curVertex` a local `vec4(x, y, z, 1.0)` |
| `ideas-lamp-unlit.vert` | 2 | the same product, nothing else |
| `ideas-logo-flat.vert` = `ideas-logo-shadow.vert` | 3 | the same |
| `ideas-paper.vert` | 4 | the same (then `normalize`, an `if`) |
| `ideas-table.vert` | 5 | the same |
| `ideas-text.vert` | 6 | the same (then an `if`) |
| `ideas-under-table.vert` | 7 | the same |
| `bump-normals-tangent.vert` | 8 | `cross(normal, tangent)`, two attributes |

So seven distinct `ideas` sources (nine files) match the plan's 7; the eighth,
`bump-normals-tangent.vert`, refuses on `cross` in the raw file but the corpus's
text (with glmark2's injected header) may refuse earlier on another family
(**inferred**: the constant-product family, `mali-algebraic-folds.md`). The
order of the refusals matters for the lowering: the `ideas` product is
`mat4 * mat4 * vec4`, and the printed operands are, at the first refused node:
`s[0]` an `Ffma` (the inner product's column), `s[1]` a `LoadLocal` (the local
`curVertex`, `.www`), `s[2]` an `Fmul`. `is_mat_term` fails on `s[0]` (not a
uniform column) and on `s[1]` (a local, not a splat of a uniform), so the whole
class is "a matrix product whose operands are not uniform columns and a
vector".

## The fragment side already emits FMA

Yes; the vertex side already has the same machinery:

- The vertex builder's own `Alu::Ffma` node is emitted as `FMA.f32`
  (`emit.rs:209`, `(Alu::Ffma, 32, 1) => ins(p, Op::FmaF32, &s[..3])`; an
  `Alu::Fmul` is `FMA.f32 a, b, 0x0.neg`, `emit.rs:208`; the registry rows
  `emit.rs:42`, `:52`, `:127`). Packing is `pack.rs:217` (`Op::FmaF32`:
  three `fsrc` sources with their neg/abs/swizzle, `round` 0, `clamp`, a
  narrow destination). The vertex already uses it for a dot product
  (`nir.rs`, `AluOp::Fdot*`: `Fmul` of the last lane, then `Ffma` down), for a
  uniform-column matrix product and for the late fusion `fadd(fmul)` ->
  `ffma` (`nir.rs` "late fusion", `only_used_by_fadd`).
- Fragment: the `ffma` of NIR is translated in `nir_fs.rs` (`op32(Alu::Ffma,
  ..)` at ~2135-2137; the dot chain ~2293; the matrix chain ~2315-2326) and the
  fusion `fadd(fmul)`/`fadd(fneg(fmul))` at ~5686-5690. In 16 bits it is
  `Op::FmaV2f16` (`pack.rs:446`, three sources with `swz`/`neg`/`abs`;
  `emit.rs:225`, `:277`). The fragment stage is where fp16 arises
  (`mediump`), see below.

So the encoding is not new in the vertex stage: what is missing is the **choice
to lower** a general `ffma`, and getting the operand order and the fusion
points to be Mesa's.

## What Mesa 26.2.1 emits

Mesa's vertex stage is highp, **fp32 throughout** (`bit_sizes_float: 0x20` in
both runs; the committed shim references say the same,
`mali-vs-alu-ops.md`, "All f32"). A `mediump` vertex *uniform* is held as fp16
and widened per use (`mali-precision.md`, part 1); that changes where the
operand comes from (`.h0`/`.h1` of a FAU word, `f2f32`), not the FMA, which stays
`.f32`. There is no `FMA.v2f16` in a vertex program in any reference
(**inferred** for a `mediump` vertex *varying* or attribute: `pan_shc`'s
`decl_var ... highp vec4` for every attribute; no reference draws one).
`FMA.f32` takes the modifiers on the sources: `.neg`, `.abs` and the half
select; a product is `FMA.f32 d, a, b, 0x0.neg` (a `-0.0` addend), an
`fneg` of a fused addend is `.neg` on source 2 (`r3^.neg` below), never a
separate `FNEG`.

Two references were made with `pan_shc` (its SPIR-V front end; its NIR is the
shim's modulo the gaps of `README-pan_shc.md`, so a **byte** reference for the
tests still has to be a `drm_shim_ref.sh` run, see the plan), and one
committed shim reference is quoted for the mixed-operand case.

### 1. `ideas-logo.vert` + a fragment shader that reads its varyings

(`pan_shc`, `PAN_SHC_PRINT_NIR=1`, the NIR handed to the back end, before
`bi_opt`'s late fusion.) The `ideas` product is a 4x4 matrix times 4x4 matrix
times vector, **not** folded (the matrices are uniforms; `curVertex.w` is the
constant 1.0, so the last term has no multiply). The NIR for one column of
`mat * v` is the chain in reverse lane order, `w` first:

    32x4  %18 = fmul %9, %11.wwww
    32x4  %19 = fmul %7, %11.zzzz
    32x4  %20 = fadd %19, %18
    32x4  %21 = fmul %5, %11.yyyy
    32x4  %22 = fadd %21, %20
    32x4  %23 = fmul %3, %11.xxxx
    32x4  %24 = fadd %23, %22

and the position column of `projection * modelview * curVertex` ends in

    32x4  %46 = fmul %38, %2.zzzz
    32x4  %47 = fadd %46, %45
    ...
    32x4  %50 = fmul %24, %2.xxxx
    32x4  %51 = fadd %50, %49

(`%2` is the `vertex` attribute; `%45` is the `w` column of `P*M`, the
`* 1.0` being gone). The disassembly, one product (`vs position`, `FMA.f32`
only, no `FADD`):

    FMA.f32 r1, u15.w0, u15.w1, r0.neg      ; w term: a * b + -0.0  (the fmul)
    FMA.f32 r3, u16.w0, r2, 0x0.neg
    FMA.f32 r1, u11.w0, u11.w1, r1^         ; z term: ffma(c_z, v.z, w term)
    FMA.f32 r3, u12.w0, r5, r3^
    FMA.f32 r1, u7.w0, u7.w1, r1^           ; y term
    FMA.f32 r1, u3.w0, u3.w1, r1^           ; x term
    FMA.f32 r0, r2^, r48^, r0^              ; ... the final col * vertex terms

That is: the chain is `fmul` (w) then `ffma` for z, y, x, each accumulating
(`^` is the discard flag of a register's last use), because Mesa's late
fusion (`nir_opt_algebraic.py:3793-3816`) turns each `fadd(fmul(a, b), c)`
into `ffma(a, b, c)` taking the first source of the commutative `fadd` first
(`nir.rs` documents the same rule at its top, "the late fusion"). The words
are the 8 bytes of `pan_shc`'s dump, e.g. `FMA.f32 r1, u15.w0, u15.w1, r0.neg`
is `9e 9f 00 00 04 c1 b2 00`, `FMA.f32 r1, u11.w0, u11.w1, r1^` is
`96 97 41 00 00 c1 b2 00` (little-endian bytes; the `b2` byte is the
`FMA.f32` opcode with `round`/`clamp` 0). A source `uN.wK` is a FAU
half-pair: the two uniform halves read in one instruction come from the same
FAU word pair; when the operands sit in different pairs one is moved to a
register first (the committed shim reference `sel_v_attr_fma`'s varying
program: `MOV.i32.wait0 r4, u6.w0` then `FMA.f32 r0, r0^, u5.w1, r4^` for
`a_c * u_k + u_b`).

Registers: this shader needs the high register file (`r48`-`r50` appear as
operands of the last steps); the register allocator, not the
FMA, is the risk (**inferred**: `ra.rs` has to colour 51+ values; compare
`mali-int-and-size.md` for the limits).

### 2. `cross(normal, tangent)` (the `bump-normals-tangent` shape)

Source: `attribute vec4 a_pos; attribute vec3 normal, tangent; varying vec3 v_b;
void main() { gl_Position = a_pos; v_b = cross(normal, tangent); }`. Mesa's NIR
(GLSL `cross` is `a.yzx*b.zxy - a.zxy*b.yzx`):

    32x3   %6 = fmul %3.zxy, %5.yzx
    32x3   %7 = fneg %6
    32x3   %8 = fmul %3.yzx, %5.zxy
    32x3   %9 = fadd %8, %7

and the varying program, per lane (x shown):

    FMA.f32 r3, r6, r1, 0x0.neg            ; n.z * t.y      (the fneg'd product)
    FMA.f32.wait2 r3, r5, r2, r3^.neg      ; n.y * t.z - that (ffma(n.y,t.z, -p))

The late fusion fuses the **first** `fmul` of the `fadd` (%8), and the `fneg`
of the second product (%6) is the `.neg` of source 2, with no `FNEG` and no
`FADD`. This is exactly the shape of our `cross` in `builtin_call.rs`:
`m1 = fmul(a.zxy, b.yzx)`, `nm = fneg(m1)`, `ffma(a.yzx, b.zxy, nm)` -- the
front end has already done what Mesa's late fusion does. The same order for
the other two lanes: `FMA.f32 r2, r4, r2^, 0x0.neg` /
`FMA.f32 r2, r6^, r0, r2^.neg`, and `FMA.f32 r0, r5^, r0^, 0x0.neg` /
`FMA.f32.wait1 r0, r4^, r1^, r0^.neg`.

## Implementation plan (not done here)

The lowering is small because the encoding, the BIR op, the selection
(`emit.rs`) and the late fusion all exist; what changes is which shapes the
vertex walk accepts, and each is accepted only against a Mesa reference:

1. **Accept a front-end `ffma` whose Mesa form is the fused chain** instead of
   `return Err(FusedMultiplyAdd)`. Two classes, each its own commit:
   - `cross` (and the matrix chain of non-uniform operands): lower to
     `Alu::Ffma(x0, x1, x2)` unchanged, as the `is_mat_term` arm and the dot
     arm already do. Mesa's NIR is `fmul`/`fneg`/`fmul`/`fadd` and it fuses
     the first product, which is what `cross` builds, so no re-fusion is
     needed (reference 2).
   - the mat4*mat4*vec chain of reference 1: widen `is_mat_term` (source 0 an
     `Fmul`/`Ffma` column or a uniform column; source 1 a splat of an
     attribute, a local or a constant; source 2 an `Fmul`/`Ffma`), keeping the
     `x * 1.0` fold of `curVertex.w`: ours would reach `ArithConstant`
     ("a matrix product's term by a constant 1.0") where Mesa's
     `nir_opt_algebraic` folds the multiply away and fuses the next `fadd`.
     That fold is the constant-product family (`mali-algebraic-folds.md`,
     `ana-vsfold`'s note); do it with it.
2. **Leave `mix`, `reflect`, `smoothstep`, `mod` and the polynomials refused**
   until each has a Mesa reference: `mix` is `flrp` in Mesa, lowered by
   `lower_flrp32` (`flrp_lowered: true` in the dumps), and the fused or
   unfused form depends on the operand constants (**inferred**: not
   measured here); the other four have different NIR from our hand-written
   forms. Keep the refusal text for them or split the error.
3. **Order and the shared-product rule**: the late fusion only fuses a product
   `only_used_by_fadd` (`nir.rs`, see `mali-vs-alu-ops.md`: a product with
   four uses is a single `FMA.f32 .., 0x0.neg` and its users are separate).
   A front-end `ffma(a, b, c)` whose product Mesa would also use elsewhere
   cannot be lowered as one `FMA`. The `cross` and the mat chain have single
   uses by construction; assert it (`only_used_by_fadd`-style) and refuse
   otherwise.

### The host test that compares bytes

References, made with the shim (`tools/mali/drm_shim_ref.sh run <shim> V F
target/vsfma/<name>` and `python3 -I tools/mali/pan_ref_diff.py extract ...
--draw > tools/mali/testdata/vsfma_<name>.expect.json`, as
`mali-vs-alu-ops.md` records), new files, nothing existing touched:

| Reference | Vertex shader | Fragment partner | Pins |
|---|---|---|---|
| `vsfma_cross` | `attribute vec4 a_pos; attribute vec3 normal, tangent; varying vec3 v_b; ... cross(normal, tangent)` | `precision mediump float; varying vec3 v_b; ... vec4(v_b, 1.0)` | the `.neg` on source 2, the three lanes' order |
| `vsfma_ideas_unlit` | `ideas-lamp-unlit.vert` (11 lines, glmark2's text) | any constant-colour fragment | the mat4*mat4*vec4 chain with the `* 1.0` fold, the `FMA.f32` count and register count |
| `vsfma_ideas_logo` | `ideas-logo.vert` | one reading all three varyings | only after `Feq`/`if`/`Frsq` are lowered (below) |

The test is a row in `src/shaderc/online/mali/tests.rs`'s reference table
(`("sel_u_vecmat", td!(...vert), TRI_FS, td!(...expect.json))`, line ~1937)
comparing the vertex program's words, `work_reg_count`, FAU and varying layout
to the JSON, as `vsarith_*` do; `tests.rs` is being changed by item
`vs-cmp-clamp` at the time of this note, so the rows go in after that lands (or
in a new `*_tests.rs` file with the same `td!` helper). Two unit tests beside
it, on the IR (not bytes): `cross` and the mat chain lower to `Ffma` nodes in
Mesa's order; a shared product is refused. The corpus row
`the_corpus_verdicts_are_pinned` (`tools/mali/corpus-verdicts.txt`) moves with
`tools/mali/corpus-verdicts.sh --write`, in the same commit; the pinned
verdicts of the `ideas` sources will not become `ok` from this change alone.

## What the FMA lowering does not fix (forecast, **inferred** from the sources)

After the `ffma` the next refusals of the same shaders are other families:

- `ideas-logo`/`lamp-lit`/`logo-flat`/`logo-shadow`: `unitvec` has
  `if (v1.w == 0.0 && v2.w == 0.0) ...`, i.e. `Feq`, `Bcsel` or an `if` and a
  `normalize` (`Fdot3`, `Frsq`). The NIR above already shows it: `feq`,
  `frcp`, `bcsel`, `fdot3`, `frsq` on `vertex_position`. `Frsq` and `Feq` are
  refused in the vertex walk today (`mali-vs-alu-ops.md`, `es100/vrcp.vert`,
  `anyall.vert`): items `vs-cmp-clamp`, `vs-abs-if`.
- `ideas-paper`, `ideas-table`, `ideas-text`: an `if` on a uniform
  (`currentTime`) and `max`, `normalize`: `Fmax` and the vertex `if`.
- `ideas-lamp-unlit`, `ideas-under-table`: nothing but the product and a
  constant colour, so these two are the shaders that turn `ok` with the FMA
  work alone (**inferred**; check with `corpus-verdicts.sh`).
- `bump-normals-tangent`: after `cross`, `normalize` (`Frsq`), a `mat4(...)`
  constructor with constant `0.0`/`1.0` lanes and `NormalMatrix *
  TangentToObject` (a uniform times a computed matrix): the constant-product
  family.

## Handoff

- **Order of work**: `Frsq`/`Feq`/vertex `if` (`vs-cmp-clamp`, `vs-abs-if`)
  and the constant-product fold (`ana-vsfold`) gate every `ideas` shader but
  two. FMA first buys `ideas-lamp-unlit` and `ideas-under-table` (**inferred**).
- Not done here: the `drm_shim_ref.sh` byte references (the shim was not built
  in this session; `pan_shc` was), the exact glmark2 corpus run (`run.sh` was
  not permitted), `docs/README.md`'s line for this note (outside the item).
- `nir.rs` and `tests.rs` are owned by `vs-cmp-clamp` while it runs: start the
  lowering after it merges.
