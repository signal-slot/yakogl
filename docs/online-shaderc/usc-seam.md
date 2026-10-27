<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The USC seam: what the second back end reuses, and what it cannot

The online compiler's GC7000 back end is wired end to end (a GLSL `texture2D`
shader compiles to a `texld`, and the program draws on silicon). This document
says what the PowerVR USC back end takes from that work unchanged, what it has
to build for itself, and why the line falls where it does. Ruled 2026-09-22.

It was written before the code, on purpose: the expensive mistake available
here is to widen a type that fits one machine until it fits two, and that
decision is easier to argue about in prose than in a diff. The PowerVR back end
has since been built on it and is the AM62P's product compiler.

## 1. What transfers unchanged

Everything above the machine layer, which is the payoff of `ir.md`'s decision 1:

- **The IR** (`ir.rs`). The `AluOp` kernel is already the intersection of two
  machines. Its one concession to a machine — `Fdot2/3/4` kept because the
  GC7000 has `DP2/DP3/DP4` — costs the USC nothing: `isa.md` records that an
  `Fdot*` scalarises into the `Fmul`/`Ffma` chain the front end would have
  written anyway. Nothing in the IR needs a USC-shaped change.
- **The whole front end**: preprocessor, parser, `sema`, the type checker,
  `lower.rs`.
- **`builtin_call.rs`**, entirely. It is the *language* half by construction and
  names no GPU; all of the ES 1.00 built-in library, the texture look-ups and
  the dEQP-only built-ins come free.
- **The `ShcShader` ABI and the oracle's shape.** `tests/online_oracle.rs`
  already selects its table by feature and prints by dialect.

So the USC needs a machine layer and nothing else.

## 2. What does not transfer, and why

`vivante/alloc.rs`, `vivante/build.rs` and `vivante/builtins.rs` are shaped
around a **fixed 128-bit instruction with three source slots**. The USC is not
that. From our own M1 decoder (`powervr/decode.rs`), a USC *group* is:

    hdr, alutype, instrs[MAX_INSTRS] each (phase, slot, Region),
    lower, upper, iss, dst, word_pad, align_pad, total

Variable length; several instructions per group in **execution phases**;
co-issue slots; a decode-assist field (`da`) computed from the phases after p1;
a byte of padding to an even length; and an i-cache alignment block closing the
shader. `SelInstr { op, ty, cond, saturate, dst, srcs: [_; 3] }` cannot express
that, and widening it to try would make the GC7000's type worse in order to
serve a machine it is not.

**`powervr::SelInstr` is therefore its own type.** The two back ends share the
*contract*, not the struct:

- `native(op, ty)` is asked first; `lower(..)` is called only when it answers
  `None`;
- the lowering emits through an `out` callback and takes scratch from a `fresh`
  counter that starts above every `ValueId`;
- an operation with no written sequence is **refused**, never approximated.

Every one of those is a decision about how a compiler should be wired rather
than about a machine, and each was paid for on the GC7000. The union type was
refused for the reason recorded in `select.rs`'s history: a converter that
cannot carry every value it is handed emits a plausible instruction computing
something else, which draws a picture instead of failing.

### A scalar machine makes dead code structural, not incidental

Selection gives every value four registers, one per component, whether the
shader uses four or one, and expands each IR operation into one group per
component. A `vec4` expression whose `.x` alone is read therefore costs four
groups and three of them compute a number nobody asks for.

Measured over the corpus: **6,037 groups before the dead-group pass, 4,853
after -- 1,184 gone, 19.6 %**. On the GC7000, a vector machine, the same pass
removed 129 instructions of 6,390 (2 %), because there one instruction covers
four lanes and a narrower write mask is the whole of the saving. Same pass,
same corpus, an order of magnitude apart; it is a property of the machine, not
of the front end.

One trap, and it is the kind that deletes a whole shader rather than spoiling
one: **an output's components are read by the BUILDER, not by a group.**
`Selected::outputs` carries a `SelSrc` per component, so a liveness that counts
only group operands finds every final value unread. Without that check all 106
sources collapse to zero groups and still assemble. The test named for it
fails first.

The two vehicles are unaffected -- `hello.vert` is 16 groups before and after,
`hello.frag` 0 -- so the hand-assembly targets do not move.

## 3. The three pieces

| piece | what exists | what it owes |
|---|---|---|
| `powervr/build.rs` | `encode_group`, byte-exact over 576,296 groups | the *construction* side: choosing phases, packing slots, computing `da`, the two kinds of padding |
| `powervr/alloc.rs` | nothing | placement across **several** register files, refusing rather than spilling |
| `powervr/builtins.rs` | nothing | `native`/`lower` over the same 52 `AluOp`s |

The M1 round trip stays the builder's gate: whatever the construction side
emits must still decode and re-encode to the same bytes.

`abi.md` already records the allocator's dangerous parts. **TEMPS is the field
most likely to be wrong and silently so** — too small and the shader reads
another thread's registers — along with the vertex-id register-plus-one rule,
the 20-dword descriptors that sit before the uniform block, and the DOUTI
coefficient layout with W at `cf0..3`.

### A worked example: `min`, and why `native()` returning a name is wrong here

The GC7000's `Fmin` is one instruction: `select` with `COND = Gt`. The USC's is
one GROUP containing four operations in three phases. From pco's own mapping
(`pco_map.py`, `group_map(O_MIN, ...)`, read and transcribed):

    header  oporg = p0_p1_p2, w0p = true
    phase 0   MBYP  ft0 <- src0
    phase 1   MBYP  ft1 <- src1
    phase 2   TST   ftt <- is1 `less` is2      (tst_op_main = less)
    phase 2   MOVC  dest <- ftt ? ft0 : is4

Four operations, two of which exist only to move the sources into the internal
registers the test and the select read. That is what `mbyp` being the most
common operation in the whole corpus means -- 218,917 of 576,296 groups -- and
it is not an inefficiency: it is how a phased machine feeds its own pipeline.

This is the concrete reason `powervr::SelInstr` cannot be the GC7000's. On that
machine `native(op, ty) -> Option<&str>` answers "which opcode", and the
question is well posed because the answer is one instruction. Here the answer
to "what is `Fmin`?" is a group shape: which operations, in which phases, with
which internal registers wired between them. A function returning an opcode
name has nowhere to put phase 2's second operation.

So the USC's `native`/`lower` split survives -- an operation either has a group
shape this file knows or it is refused -- but what `native` RETURNS is a group
template rather than a name. The contract holds; the type does not transfer.
Recorded here rather than discovered while writing the builder.

## 4. Order of work

**`powervr/builtins.rs` first**, before the builder's construction side. It is
the piece whose answers come from *measurement* rather than design: every entry
gets its count from the 2,042-blob corpus, or is named as transcribed from
Mesa's `pco` the way `Ineg` and `texldb` are on the GC7000 side. It can be
written and tested before anything else exists, and it answers early whether
the USC shares the GC7000's awkward gaps (no `min`/`max` opcode, no integer
negate, four usable compare conditions rather than six).

## 5. The part that is not measurable, and must not be guessed

**Phase assignment.** Decoding phases is solved; *choosing* them is a scheduling
decision, and the corpus shows what the offline compiler chose, not why. The
ruling is to read `pco`'s own scheduler (MIT, read and transcribed, never
vendored) before choosing anything, and to write what that says into
`usc-isa.md` marked as read rather than measured — the same separation the
GC7000 documents keep between "Mesa's table says" and "our corpus counts".

## 6. The oracle

Two levels, both with their harness already built:

- **M1, done**: 2,042 blobs, 576,296 groups, byte-exact round trip
  (`powervr/roundtrip.rs`), green in the gate.
- **The M3 equivalent**: the PowerVR table's 533 sources (240 of them ES 1.00)
  compiled through the online path and compared field by field against the
  table's rows. Two numbers from the first commit that assembles anything —
  how many lower to IR and how many compile through the whole chain — plus the
  histogram of the first construct each refusal stops on. The GC7000's
  98-to-IR / 39-through pair is the model; the gap between the two numbers is
  the back end's work list.

## 7. Division

`pvrgpu-platform` takes the builder, the allocator and the built-ins;
`gles2-core` takes the USC selection and `abi` halves. The front end and the
built-in expansions belong to neither, because they are already written.

## 8. Contracts settled while building it

The build-up of this back end (2026-09-23 onward, section 8 of this file until
2026-10-11) is the record `docs/notes/usc-backend.md`. Two of
its results are contracts code depends on, and stay here.

### Loops counted at run time: the cap

**The cap is a deliberate deviation from GLSL.** `unroll::UNIFORM_LOOP_CAP`
= 4096: a bound past it stops the loop after 4096 iterations and the result
is undefined, where GLSL would run on. A bound of +inf would otherwise never
end, and a program that does not end stops the GPU; the cap is the
structural termination guarantee for loops the compiler cannot count. The
IR evaluator the host tests compare against applies the same cap, and the
test prints it (`a_loop_bounded_by_a_uniform_runs_as_many_times_as_it_says`),
so no comparison hides it. A constant bound past the expansion limit (256)
now runs the same way (exact, leaving on its flag).

### Cube maps, 2D arrays and 3D textures: the stacked 2D image

The PowerVR path has no hardware cube, array or 3D texture. The GL core keeps
all three in ONE linear 2D image (`GlTexture.layers`, context.rs): a cube's six
faces and an array's layers stacked vertically (face or layer k at rows
k·h..(k+1)·h), a 3D texture's slices in a grid. Both shader paths sample that
image as a `sampler2D`:

- the table path through glsl2usc's GLSL helpers (`EMU_HELPERS`,
  tools/yakogl-tools/src/glsl.rs: `pvr_cubeUV`, `pvr_arrUV`, `pvr_volUV` and
  their Lod/Fetch/Size forms), compiled by pco;
- the online path through the same arithmetic in selection, one operation
  for one (`Sel::cube_uv`, `Sel::array_uv`), so the two sample the same texel.

(pco's own array path -- a layer offset from image meta LAYER_SIZE, the
maximum from IMAGE_WORD1.depth -- does not apply: our images are STRIDE, and
those bits of word 1 are the stride.)

**The `pvr_layers_<sampler>` contract.** A 2D array's layer count, and a 3D
texture's slice count, reach the shader as a float in the uniform block
named `pvr_layers_` followed by the sampler's name. Both compilers bind to it:

- **position**: for each `sampler2DArray` / `sampler3D` the body uses, in
  declaration order, the member is appended to the block at the point the
  sampler is declared (glsl2usc, glsl.rs around `uniforms.push(format!("\tfloat
  pvr_layers_{name};"))`); a sampler the body never reads gets none. pco's
  records place it there, so the online reflection must too, or every later
  uniform's offset differs from the table's.
- **writer**: the GL core, by name, before the block is uploaded, in BOTH
  stages (draw.rs `fill_stage_layer_uniforms`); 1 when nothing is bound.
- **reader**: the shader, by name (`Sel::layers_dword`); a cube needs no
  count (6).
- **GPU**: PowerVR only. The GC7000 samples the same stacked storage with
  its native cube/array/3D types and has no such member.

Status (usc-tex, frontend3's branch, host only): cube look-ups with the
implicit or a replacing level go through (all 18 samplerCube fragment
shaders of the table; 15 identical to pco on every run, the lightProbe three
checked against selection and the IR because pco's program uses ignorepe);
2D arrays are selected but refused until the front end declares the member;
3D, `texelFetch` on a stacked texture and `textureSize` of a cube or array
are next. `texelFetch` must read the exact texel whatever the bound
sampler's filter (the table path's helpers inherit the binding's LINEAR
sampler -- gl_test case 48 read 0.75·80 + 0.25·40; am62p-board's case 93).

Known defect, kept deliberately for parity: the emulated cube samples each
face as a separate region, so the implicit level is computed across the
seam between stacked faces (gl_test's cube residue).

The face clamp (uv kept half a texel inside the face) was taken at level 0
on every look-up. At a level where a face is a texel or two tall that
clamps nothing, and a bilinear filter blends in the rows of the face
stacked next to it: dEQP texture_functions.vertex.texturecubelod drew a
gradient across its high-LOD corner (am62p-board, 30899c1f), and Qt Quick
3D's IBL, which samples its prefiltered cube with textureLod at
roughness-driven levels, reads the same blend on the table path. Fixed for
an EXPLICIT level on both paths: glsl2usc's pvr_cubeLod (its own
pvr_cubeUVLod) and `Sel::cube_uv` clamp with the face height at the coarser
level a trilinear filter reads, `max(h * exp2(-ceil(max(lod, 0.0))), 1.0)`
(gl_test 114 "cubelod"). DELIBERATE DEVIATION that remains: an implicit
level or a bias is the hardware's to choose, so those look-ups keep the
level-0 clamp and still blend neighbouring faces at levels whose faces are
a texel or two tall; and an explicit level past a chain the application
capped (GL_TEXTURE_MAX_LEVEL below the last level) clamps with a face one
texel tall, narrower than the level the sampler really reads. The native
cube descriptor (below) removes both. The native cube
descriptor works on silicon (am62p-board's probe, 0/3072 wrong); moving to
it changes the table path and the online path together, and the online
side is the one function `Sel::cube_uv`.
