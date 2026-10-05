<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The USC seam: what the second back end reuses, and what it cannot

The online compiler's GC7000 back end is wired end to end (a GLSL `texture2D`
shader compiles to a `texld`, and the program draws on silicon). This document
says what the PowerVR USC back end takes from that work unchanged, what it has
to build for itself, and why the line falls where it does. Ruled 2026-09-22.

It is written before the code, on purpose: the expensive mistake available here
is to widen a type that fits one machine until it fits two, and that decision is
easier to argue about in prose than in a diff.

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

## 8. State, 2026-09-23 (usc-backend)

Measured by `tests/usc_limits.rs` over the 533-source PowerVR table; every
number below is printed by that test on each run.

- **The whole path is one function**, `powervr::chain::build_vertex`:
  selection, `prepare`, allocation, `vertex_output_writes`, `build_shader`.
  The survey, the USC-2 diff and the back end go through it.
- **63 vertex shaders go through** (the old "17" counted fragment shaders
  that were built with a vertex epilogue). Fragment shaders are refused by
  name (`FragmentNotBuilt`): the varying iterators and the pixel output write
  do not exist yet.
- **Three gates, each over every group the chain builds** (4,199 groups):
  1. read back: each group decoded and named through the ISA table's own
     enums, compared with its `SelGroup` (`verify::check_group`);
  2. corpus: each group's skeleton -- everything but registers and literals
     -- must be one pco emitted somewhere in the 576,296 reference groups;
  3. execution: a host interpreter (`tests/usc_exec`) runs our bytes against
     selection's program, and against **pco's bytes for the same source**
     with inputs placed by name through each program's own interface
     (a matrix attribute column by column, each column's values its own:
     ours at every location it occupies, pco's from its record's row per
     column).
     61 of 63 agree with pco on every output dword, one within 1 ulp
     (operation order), one reference is not runnable (it moves an
     immediate to `idx0`), none differs.
- Fixed on the way, each found by one of the gates above and not by the
  previous "29 decode back": `Bank as u32` encoded the wrong register files
  (Temp/Shared/Special); `uvsw.write` never set `imm_addr` and left `is4` at
  `ft0`; a modifier on a constant was dropped in selection (`x - 1.0`
  compiled as `x + 1.0`); multi-operation shapes (`tst`, `movc`, min/max)
  were emitted with their internal wiring left zero and are now refused.
- Uniforms are never read by arithmetic directly: pco never does it in the
  corpus, so `prepare` moves each through an `mbyp` first. Whether the
  hardware accepts `shared` in an `fmad` slot is a board question, not asked
  yet.

### Later the same day: wired, and a fragment path

- `codegen::compile` on a PowerVR build goes through
  `chain::compile_vertex` / `chain::compile_fragment`; a refusal is
  `CompileError::BackEnd(Usc(..))`, logged as -ENOTSUP naming the stage.
- The record carries the attributes the program reads (`vi_reg` is
  `abi::vtxin_reg`, `location * 4`, shared with selection), the uniform rows
  and the block size, `vs_psize_off = SHC_NO_PSIZE` (the ZERO default claimed
  a point size at dword 0), and for a fragment shader `num_outputs 1`,
  `meta_off` and `blend_consts_off` at their sentinels. Every scalar field
  and every varying is compared with pco's record for the same source: 63 of
  63 vertex records identical but for `temps`.
- The shader closes by pco's rule (`calc_align_padding`, `ROGUE_ICACHE_ALIGN`
  8): the last group starts AND ends on 8 bytes, the start padded in the
  group before it. Ours used to pad only the end.
- Fragment shaders that read no varying and write one `gl_FragColor`: four
  moves into consecutive temporaries, `pck` u8888 rpt 4 (pco's
  `group_map(O_PCK)`), the pixout0 write with `olchk` and `end`. Five
  corpus shaders go through and match pco in skeleton, alignment, record
  and computed pixel.
- es100_drawstep's pair (`gl_Position = a_pos`, constant red) is pco's
  program to the byte, both stages, and pco's record.

### Varyings (fragment)

A read varying is evaluated by pco's own sequence, transcribed from
`pco_legalize.c` and `pco_map.py` (`powervr/fragment.rs`): a fence
(`flush_p0`, `br.next` if p0, `br.next` if !p0), `ditrp` (W at coefficient 0,
pixel centre), a fence, `wdf`. Coefficients: W at 0..3, then each READ
varying in declaration order, four per component (measured with pco_shc on
vec2/vec4/float: 4, 12, 28). The components land in consecutive temporaries
from 0, pinned for the allocator; the record lists the same varyings in the
same order, with `uses_w` and the vertex-output offsets. `gl_FragColor =
v_color` is pco's program to the byte. 10 fragment shaders of the table
go through, 5 of them reading varyings, each identical to pco in skeleton,
alignment, record and interpreted pixel.

### Later still: the shapes, point size, and texture2D

- Templates (`powervr/shapes.rs`, from pco's `group_map`s) for min, max,
  bcsel, the eight compares, iadd, imul and i2f; `sqrt` as `frcp(frsq)`;
  `gl_PointSize` clamped to 1..511 after the varyings; modifier sets held to
  what the corpus shows (`prepare::attested_mods`).
- `texture2D(s, uv)` in a fragment shader, as pco emits it: coordinate plus
  fourteen zeros into a 16-register data block, `smp_brief` reading the
  image at shared 20*slot and the sampler at +8 (slot = rank of the binding,
  the driver's descriptor order), `wdf`, four moves out; uniform reads move
  past the descriptors; the record lists the sampled samplers and no others.
- Selection defects the texture reach exposed, each fixed and each
  invisible to the selection-vs-bytes check (it executes selection's own
  program): store retargeting moved the first of several writers of a dot
  product's accumulator, moved a value another store still read, and moved
  a sample's lane 0 alone; the dead-group pass judged a sample by lane 0.
  Only the comparison with pco's bytes could see them.

    vertex 79, fragment 71 (61 identical to pco, 10 with a named note)

Still owed: `discard` (16 fragment shaders), built-in fragment inputs (10),
the bitwise/other shapes (28 fragment), the other texture forms (4), loops
and nested `if`. Nothing has run on silicon; the first board rung is the
drawstep pair.

### Nested `if`, and a check that does not go through selection

- `if_convert` converts a nested `if` level by level (the mapping in force
  before each `if` is saved and restored between its arms), and joins an
  output stored inside an arm with a select like a local. A loop or jump in
  an arm still refuses. A local slot the program can read before anything
  writes it gets a zero first (`zero_unwritten_locals`): the join reads
  every slot's "before" value, and on the hardware an unwritten register is
  another invocation's data.
- `tests/usc_exec/ir.rs` evaluates the front end's IR with `if` as a real
  branch, and `tests/usc_limits.rs` compares our bytes with it for every
  program through the chain: 24 inputs each, half spread, half small values
  (zero, one, two; integers for an integer uniform) that drive the
  conditions. Every `if` of every compared program must be seen going both
  ways (38 of 38), and every program must have a defined run. An input
  whose IR evaluation feeds a NaN to a comparison, min/max, sign or
  conversion is set aside as undefined and counted, not compared.
- Why it exists: pco compiles an `if` to predicated groups and branches the
  byte interpreter cannot follow, so for these programs the only execution
  check was against selection's own program, and a wrong join is in both.
  Swapping the two operands of the locals join in `if_convert` leaves the
  selection-vs-bytes check at 0 disagreements; the IR check fails 7 vertex
  and 13 fragment programs on it.

    vertex 82 (1914 IR runs agree), fragment 101 (79 identical to pco,
    22 with a named note; 2422 IR runs agree)

Refusals now: vertex 10 on tst/movc/bitwise shapes, 4 on a dynamically
indexed uniform; fragment 16 `discard`, 10 built-in inputs, 4 loops, 4 other
texture forms, 2 other outputs, 1 tst/movc shape, 1 out of temps (270
needed, a nested-`if` program). Host-verified against pco and the IR, not run
on silicon.

### Logical operations, the reductions, and what the IR check found next

- `or`/`and`/`xor` are templates now (`shapes.rs`, `group_map(O_LOGICAL)`
  as `trans_logical` fills it): a bitwise-pipeline group, `bbyp0s1` then
  `logical`, operands on `s2` and `s3`. `Bany`/`Ball` are expanded by
  selection into an `or`/`and` chain over all four lanes, the IR's
  contract. The refusal for an untranscribed shape carries the operation,
  so the survey names what each transcription would carry.
- The IR check's inputs: 48 fixed runs per program (spread values, small
  values with zeros, small positive values), then up to 400 more of mixed
  magnitude with exact zeros, only while some `if` has not been seen both
  ways. Undefined values are tracked per lane (a local nothing wrote, a NaN
  an operation had to decide about) and not compared; only a branch on an
  undefined condition sets a whole run aside.
- The sampler model maps a coordinate to a texel of a 64x64 grid before
  hashing. It hashed the coordinate's bits, so two programs one rounding
  apart in a computed coordinate read unrelated values: four "differences"
  that were the instrument's.
- Found by the search: `any()`/`all()` over a `bvec2`/`bvec3` reduced the
  lanes the vector does not have. `builtin_call.rs` now repeats the last
  real lane into them, as `compare_vector` already did for `==`/`!=`.
  `all(equal(v2, vec2(0.0)))` was false for every vertex whose spare lanes
  held the vertex fetch's (0, 1). Live on the GC7000's online path (it folds
  four lanes too); on the USC the builder refused `Bany` until this change.

    vertex 90 (was 82); 78 of 78 `if`s driven both ways, 0 disagreements

Remaining vertex refusals: 4 dynamically indexed uniforms, 2 `cos`.
Host-verified against pco and the IR, not run on silicon.

### `discard`

- A `discard` stores `true` into one more local slot after the module's
  own, so if-conversion joins it like any local: the flag at the end is
  whether the path taken discarded.
- The epilogue is pco's full-mode one (`pco_nir_pfo`,
  `pco_nir_lower_alpha_to_coverage`), reading the per-draw control word the
  driver DMAs (`SHC_FS_META_DEFAULT`) from the end of the uniform block:
  alpha to coverage (`pck.cov`, `savmsk.vm`, `1 << samp_num`), the sample
  mask, alpha to one; each field an `ubfe` with pco's zero-width select.
  Then, after the colour pack: `tst z` of the flag into p0 (no
  destination), `alphaf` under `if(p0)` with `olchk`, `wdf`, the pixel
  write. New templates: `ubfe`, `ishl`, `pck.cov`, `savmsk`, the p0 test
  and the feedback; `shapes::build` learnt headers with no destination and
  with extra fields (`cc`, `olchk`).
- The record carries `uses_discard` and `meta_off`. Where pco's table puts
  the word after the uniform prefix its program loads and reports that
  prefix as `uniform_dwords`, ours puts it after the whole block and
  reports the whole block; the survey names this as a note, and only when
  each record follows its own rule exactly.
- The byte model evaluates the execution condition (`if(p0)`, `if(!p0)`)
  and the p0 test, records the pass feedback as an output, and knows
  `ubfe`, shifts, `pck.cov`, `savmsk` and `samp_num`. That also made 10
  more of pco's vertex programs runnable, each agreeing with ours on every
  output GLSL defines (an output a path never writes is not compared: the
  IR run says which).
- Checks: against pco with the control word cycling through every field
  (default, alpha to one, alpha to coverage, sample 0 uncovered); against
  selection's program; against the IR, which discards on 367 runs where
  ours withholds the feedback. Storing `false` for a discard fails all seven
  discarding programs there.

    vertex 90 (71 bit-identical to pco); fragment 108 through (82
    identical to pco, 26 with a named note); 97 of 97 `if`s both ways

Of the 16 discarding shaders, 7 go through; 8 also read gl_FragCoord (the
built-in inputs, parked on the y-flip question) and 1 has a loop.
Host-verified against pco and the IR, not run on silicon.

### Loops, unrolled

- `powervr/unroll.rs` expands every loop before selection, IR to IR: it
  walks in execution order evaluating constants and locals that hold them,
  so the front end's `if (cond) {} else { break; }` at an iteration's head
  resolves and the arm taken is inlined. Each iteration's instructions are
  copies with fresh value ids. A `break`/`continue`/condition that depends
  on data, a `return` in a loop, and more than 256 iterations or 16,384
  copied instructions are refused by name: those need a real branch.
- The chain gate's IR interpreter runs the ORIGINAL module with its loops,
  so it checks the unroller independently; capping every loop at two
  iterations fails temporalaa.frag there while selection and our bytes
  still agree with each other.

    fragment 109 through (temporalaa.frag); shapestroke*.frag now stop at
    `cos`; filter_image*.frag break on data and are refused by name

### `sin` and `cos`

- As pco's `trans_trig` builds them, per component: `fred` part A (into
  `w1`), `fred` part B of x and part A, `fsinc` (which sets p0), and
  `psel_trig` (the `fsinc` result, or its product with part B under p0).
  Templates learnt a destination in `w1`.
- The byte model cannot know the reduction's arithmetic; it models the
  chain so that it yields sin/cos of x only when every link reads what
  the previous one wrote (part A's output is tagged with a function that is
  not its own inverse, so a swapped part B fails by name).
- Found on the way: the model applied a phase-0 `mbyp`'s negate to pco's
  programs not at all, so pco's `step(0.0, -x)` ran backwards and
  shapestroke.frag "differed from pco". And the vertex path counted OUR
  program failing to run in the IR loop or against pco's inputs as a
  refusal (a `?`), which drops the program from the population instead of
  failing the gate; both are failures now.

    vertex 92 (71 bit-identical to pco); fragment 112 through (85 identical
    to pco); 118 of 118 `if`s both ways

### Dynamically indexed uniforms

- `block[base + clamp(i, 0, len - 1) * stride + lane]` in a vertex shader,
  as pco reads one: the dword set into index register 0 (`Bank::Idx0`,
  index 5 names the register itself), then an `mbyp` per lane from
  `shared[idx0 + lane]` (index `3 | lane << 3`: `idxbank` shared, Mesa
  `pco_map_reg_index`). The clamp is the IR's rule, as compares and
  selects; `an_indexed_uniform_reads_its_element_and_clamps_outside_the_array`
  is its exercise (the corpus reads its tables in range only), and fails
  with the clamp removed. A fragment shader's block moves past the
  descriptors after selection, so the fragment form is refused by name.
- The byte model learnt index register 0 and writes to vtxin from an
  immediate move, `imadd32` and a wired group: pco computes the address
  there, and `imadd32` used to write every destination to temp.

    vertex 96 through (72 bit-identical to pco); no vertex refusal left in
    the table's ES 1.00 sources

### `texture2DProj`, and a fix to both halves

- `texture2DProj(s, p)` as pco emits it: (x, y, q) in the sample data
  block and `smp`'s `proj` (the extended `smp_exta` form); the hardware
  divides.
- The front end handed `texture2DProj(s, vec4)` over with the vec4 as is,
  and both back ends divide by component 2: the vec4 form divided by z,
  not w (live on the GC7000's online path). `builtin_call.rs` now moves w
  to component 2, and `a_projective_look_up_divides_by_the_last_component`
  pins it.
- The byte model required the sample data block's unused words to be zero,
  from the first pco program seen; pco's other programs overlap them with
  live values, so the hardware does not read them for these forms. Relaxed,
  twelve more of pco's fragment programs run, and every one agrees with
  ours. Our builder still zeroes them, which costs moves and is safe.

    fragment 116 through (101 identical to pco, 15 noted)

### A loop that leaves on data

- A `break` or `continue` inside an `if` on data sets a flag local of its
  loop (a `continue`'s is cleared per iteration); once a flag has been set,
  the rest of the loop runs under `if (flags) {} else { ... }`, which
  selection converts. The loop still expands to its constant bound. The
  schedule follows the path that goes on: an arm that ended by setting a
  flag contributes nothing to what is known (a `continue` arm steps the
  index itself).
- `a_loop_that_leaves_on_data_computes_what_the_loop_does` runs a shader
  with both through the chain against the IR's own loop over 100 inputs
  reaching every exit point; with the guard disabled it fails at the first.
- filter_image*.frag now unroll but run the 2 MB scratch arena out
  (24 iterations, 48 samples, 3,137 groups selected): refused as
  OutOfScratch. The arena is the board's configuration, not raised here.
- The tests of usc_limits share the one scratch arena, which refuses a
  second borrower: each test holds a lock for its run (the indexed-uniform
  test raced the survey and passed by luck until then).

### `fract`, and source modifiers in the hardware's order

- `fract(x)` as pco's program: `floor(x) + 0.0` (the floor a source
  modifier), then `-f + x`; an operand carrying abs or negate is computed
  first.
- A source modifier set applies floor, then abs, then negate. Selection
  folded nested `Fneg`/`Fabs`/`Ffloor` into one set whatever their order:
  `abs(-x)` compiled as `-abs(x)`, `floor(-x)` as `-floor(x)`,
  `floor(abs(x))` as `abs(floor(x))`. `|-x|` now drops the negate; the two
  a set cannot say are refused by name. Live on origin/main (6b5e8e0,
  fetched 02:26Z) for any shader the USC path accepts; no table shader
  reaches it. And prepare's split of an unattested set moved the abs out
  alone and left the floor on the consumer, turning `|floor(x)|` into
  `floor(|x|)`; it now moves the leading floor and abs together (local
  only, never on main).
- `source_modifiers_compose_in_the_hardware_order_or_refuse` and
  `fract_is_x_minus_its_floor` run each through the chain against the IR on
  both signs; both defects were found by the first run of the second.

### Locals as SSA, and dead code done exactly

- A `LoadLocal`'s value is the slot's value AT the load. Selection resolved
  it to the slot's register at each USE, so a later store changed an earlier
  load: `x = i++` got the incremented `i`, `gl_FragColor = c; c = ...;`
  wrote the new `c`, and an `if` whose condition was a flag the else arm
  set joined on the new flag (the inlined early returns of
  SaturationLightness.frag's hueToIntensity). Live on origin/main for any
  shader the USC path accepts. Now a load snapshots the slot's registers
  (`Sel::loaded`), a store moves the slot to the stored value's register
  (or a fresh copy) instead of writing in place, and an `if` joins the two
  arms' mappings into fresh registers: nothing is written twice. The seed
  copies and store retargeting are gone.
  `a_load_reads_the_value_the_slot_held_when_it_ran` pins all four.
- Dead-code elimination asked "does any group read this register", which
  kept a join alive through a read BEFORE it; lanes 1..3 of every scalar
  local fed each other to the end. It is now one exact backward pass over
  straight-line code.
- Together: SaturationLightness.frag from 677 selected groups and a 268-wide
  live peak (270 temporaries, refused) to 130 groups and 21, identical to
  pco; the corpus's temporary peak from 238 to 53.

    vertex 96 (72 bit-identical to pco); fragment 119 through (104
    identical to pco, 15 noted); 160 of 160 `if`s both ways

### dEQP's shaders against the IR

`deqp_vertex_shaders_compute_what_the_ir_means` (tests/usc_limits.rs) runs
when `DEQP_PROGRAMS` names a JSON-lines file of `{name, vert, frag}`, built
from VK-GL-CTS `data/gles2/shaders/*.test` with tools/es100_oracle.py's own
parser and `programs_of`:

```text
python3 -c 'import sys, json; sys.path.insert(0, "tools"); import es100_oracle as o
for p in sorted((o.DEFAULT_CTS / "data/gles2/shaders").glob("*.test")):
    for c in o.Parser(o.tokenize(p.read_text()), p.name).parse():
        if c.expect == "pass" and c.version == 100:
            for n, v, f in o.programs_of(c):
                print(json.dumps({"file": p.stem, "name": n, "vert": v, "frag": f}))' > deqp_programs.jsonl
```

The `_vertex` variants write what they computed to varyings, so every
output dword is compared with the IR over the same 48 inputs as the survey.
Without the variable the test measures nothing and says so. First run
(2026-09-23, 1,774 programs, 861 vertex variants):

    861 vertex variants, 814 through, 814 agree with the IR on every output
      43 matrix varyings (not built), 2 integer divide/modulo (refused by
      the machine half), 2 front-end gaps

### Matrix varyings

- Since 3beded7 the front end declares a matrix varying once per location
  (`m`, `m[1]`), each declaration still typed as the matrix; the link pairs
  the stages by those names. The chain gives each such declaration its
  column's vector type (`abi::with_matrix_varyings_split`) and expands only
  a matrix declared once, which is select.rs `varying_columns`'s rule
  (7902405). The record's layout is pco's for the same source (dEQP's
  float_to_mat2: 2 components at location 0, dword 4; 2 at location 1,
  dword 6; 8 dwords of output); pco leaves the second column unnamed where
  ours carries `out0[1]`
  (`a_matrix_varying_is_recorded_as_its_columns_as_pco_records_it`).
- `link::by_name` skipped unnamed fragment inputs, leaving the later columns
  of a pco-recorded matrix at the fragment shader's own packing -- wrong
  whenever the vertex shader packs anything before the matrix (the
  reference compiler's pair). An unnamed input with one to four components now follows the one
  before it (`a_matrix_varyings_later_columns_follow_its_first`).

    dEQP vertex variants: 859 of 861 through, all agreeing with the IR
    (2 refused: integer divide/modulo)

### filter_image: an unrolled loop that fits

femtovg's filter_image unrolls to 24 iterations and 49 samples. It was
refused twice over: 2,699 groups after preparation needed ~19 KB of image
(the limit is 16 KB) and ~2.1 MB of scratch (the arena is 2 MB). Three
changes, none specific to the shader:

- The sample data block's fourteen spare words are zeroed where nothing has
  zeroed them yet (`expand_samples`): the block is fixed temporaries only
  samples write, and the program is straight-line, so a word stays zero
  until a projective sample puts its divisor there. pco zeroes them before
  every sample; the words the hardware reads are the same either way.
- Common subexpressions over the prepared program (`prepare::share_common`):
  every iteration moved the same uniforms and took the same `rcp(extent)`
  (102 reciprocals, now 3). Only values that cannot differ between the two
  points are shared: virtual registers written once, uniforms, specials,
  vertex inputs and immediates, never a fixed temporary or an idx0 read.
- `bcsel(c1, a, bcsel(c2, a, b))` becomes `bcsel(c1 | c2, a, b)`
  (`Sel::merge_nested_selects`): a value carried past a `break` was selected
  twice per iteration, on the loop's old flag and on this iteration's.
  `bcsel` tests against zero, so the or is exact.

Result: 1,504 groups, 4,094 dwords (16,376 of 16,384 bytes -- 8 to spare,
so a longer loop in this shape will be refused again), 57 temporaries,
~1.9 MB of scratch. Both variants go through and agree with the IR on
every run, their six `if`s driven both ways. They are noted rather than
pco-identical: pco's program uses an execution condition the byte model
does not know (`ignorepe`), so ours is checked against selection's.

The survey now lends itself a region four times the product's and holds
every compile it counts to `store::SCRATCH_BYTES` (`within_product_scratch`):
it selects each program a second time in the same arena to check it, and
that cost is the harness's, not the compiler's.

### Real loops: one body and a counted back edge

A loop that can leave on data (a second `break`, any `continue`) and sits
in straight-line code is kept as a loop instead of expanded
(`unroll::roll`). Its body appears once; every exit is a flag, as in the
expansion; its trip count is the static schedule's, counting the entry
whose head test breaks (`Trip`). Selection emits:

    h[l] = <local l>        every local lane the body stores, carried
    c = 0                   the loop's counter
    LoopBegin
    <body>                  reads h[l]; stores fresh registers
    h[l] = <local l>        what the next iteration starts from
    c = c + 1
    b = (N <= c)            Ile(Imm N, c)
    LoopTest b              tstz.p0: p0 = (b == 0), i.e. c < N
    LoopEnd                 br if(p0), backward, to LoopBegin

**The back edge is uniform by construction.** pco runs divergent control
flow on its `cnd*` execution-mask stack; this path does not use it, so the
only branch is one every instance takes or none does. p0 at the back edge
comes from the counter alone:

- `select::verify_loops` refuses any other shape: the test must read
  `Ile(Imm N, c)`, `c` must be written only by its zero before the loop and
  its `+ 1`, and the back edge must directly follow the test.
- The builder (`assemble::Arithmetic`) emits a branch for `LoopEnd` and
  nothing else, refuses one that does not directly follow a `LoopTest`, and
  `fragment::branch_back` refuses a forward or zero offset.
- `the_back_edge_is_driven_by_the_loop_counter_and_nothing_else`,
  `a_back_edge_not_after_its_test_is_refused`.

The forms are pco's: `tstz.p0` is its predicate write (`KILL_TEST`), and
its own back edge in filter_image is `br` with `cc=if(p0)`, `bpred=cc`, the
offset in bytes from the branch group's start. The skeleton check treats a
branch offset as a literal, so ours is attested by pco's.

Around it: dead-code elimination treats everything read in a loop as live
at its end; the allocator extends every range live into a loop to its back
edge (`a_value_live_into_a_loop_keeps_its_register_to_the_back_edge`,
failing without the extension); the sample zero-tracking forgets at the
loop's top what a projective sample in it overwrites; the select merge
checks that nothing writes a moved operand between the two selects (a
carried register is written twice). The byte model and the
selection-level interpreter follow the branch, with step budgets.

filter_image: 636 dwords (pco: 498; unrolled it was 4,094), 56 temps,
~0.7 MB of scratch; the .nodiscard variant 510. A loop kept inside an
`if` arm, an output stored in a loop, and a kept loop inside another are
refused by name (the arm case falls back to expansion).

Host-verified only. gl_test case 87 (a data-dependent break, pixels on
both sides of it) is the silicon check, run by am62p-board.

### Loops counted at run time (a uniform bound), and their cap

GLSL ES 3.00 lets a loop's bound be a uniform: Qt's light loops are
`for (int i = 0; float(i) < counts.y; i++)`. Such a loop cannot be expanded
(its trip count is not known when the shader is compiled), so it is kept as
ONE real loop -- the same machinery as a loop that leaves on data -- with
two additions:

- its exits are classified: the condition of every `if` a `break` sits in
  is traced (`unroll::uniform_exits`) through every operation to uniforms,
  constants and locals only such values are stored to (a greatest fixpoint,
  so a counter qualifies). Uniform exits leave every instance at the same
  iteration, so the plain back edge stays sound. Anything else -- a
  varying, an attribute, gl_VertexID, a texture look-up or size, a
  derivative -- makes the loop DIVERGENT (`unroll::Trip::divergent`): it
  runs under pco's execution mask (below) instead of being refused;
- the back edge is taken while `counter < UNIFORM_LOOP_CAP` AND the loop's
  break flag is clear (`Ior(Ile(CAP, c), flag)` before `LoopTest`;
  `verify_loops` accepts exactly that second shape by name).

**A divergent loop runs under the execution mask**, pco's own construction
(Mesa `src/imagination/pco/pco_cf.c`, MIT; `powervr::cf`, whose groups are
pco's to the byte against filter_image.frag's program). One temporary, the
execution mask counter (emc), holds each instance's nesting state; groups
run with `cc = e1_zx` only in active instances (emc 0). Selection emits, for
such a loop:

```text
  CndInit     cndst 1 (from special 0)   once per program: every instance in
  CndEnter    cndst 2                    a loop level entered
LoopBegin
  ...body...  (breaks are flags, stores guarded by them, as for any kept loop)
  c += 1; b = Ile(CAP, c); e = Ior(b, flag)
  LoopTest e  p0 = (e == 0): the instance goes on
  CndLeave    cndsm p0_false, by 2       the instances that are done leave
  CndLoopEnd  cndlt 2                    p0 while the instance is still in
LoopEnd       br if(p0)                  taken while any instance is
```

The cap applies here too. The byte model runs the mask for one instance
(semantics inferred from pco_cf.c's use of the operations, checked by
running pco's own programs that use them against the IR: they agree); what
one instance cannot show -- instances leaving at different passes -- is
gl_test 104's to show on the board.

**The cap is a deliberate deviation from GLSL.** `unroll::UNIFORM_LOOP_CAP`
= 4096: a bound past it stops the loop after 4096 iterations and the result
is undefined, where GLSL would run on. A bound of +inf would otherwise never
end, and a program that does not end stops the GPU; the cap is the
structural termination guarantee for loops the compiler cannot count. The
IR evaluator the host tests compare against applies the same cap, and the
test prints it (`a_loop_bounded_by_a_uniform_runs_as_many_times_as_it_says`),
so no comparison hides it. A constant bound past the expansion limit (256)
now runs the same way (exact, leaving on its flag).

Status (usc-tex, host only): the loop form and its tests are in; the Qt lit
materials reach allocation and need 504-537 temporaries where pco's
programs use about 53 (register pressure across a kept loop: every local
the body stores is carried), and 22 still exhaust the scratch arena.

### An `if` with arithmetic arms: predicated, or a branch past each arm

An `if` whose two arms are straight-line arithmetic (`select::predicable`:
no nested `if`, loop or jump, no look-up, varying read, derivative,
sine/cosine, discard or indexed uniform read) is lowered one of two ways,
both pco's (Mesa `src/imagination/pco/pco_cf.c`, MIT):

- **predicated** (`lower_if_pred_exec`, the default, `afa12e9b`): the
  condition's `tstz.p0`, the then arm's groups under `if(!p0)`, the else
  arm's under `if(p0)`;
- **branched** (`lower_if_cond_exec`, pco's `PCO_DEBUG=no_pred_cf`): the
  arms under the execution mask (`powervr::cf`), each skipped by a `br`
  taken when no instance is active (`bpred = allp`), the arms carrying no
  condition.

```text
  CndInit     cndst 1 always (from special 0)   once per program, before the test
  LoopTest c  tstz.p0
  CndIf       cndst 1 p0_false                  the then arm's instances stay
Pred(Then)    br allp -> the cndef              (none for an empty arm)
  ...then arm...
Pred(Else)
  CndElse     cndef 1                           (none without an else arm)
              br allp -> the cndend
  ...else arm...
Pred(End)
  CndIfEnd    cndend 1
  ...the joins, as predicated...
```

Selection emits the mask groups around the predicated form's markers and
arms; the joins (`one_register_from_both_arms`, or a select) are the
predicated form's, since an instance the mask leaves out writes nothing,
exactly as one a condition leaves out. Dead-code elimination keeps the
mask groups as effects, and `drop_idle_masks` takes out the `cndef` before
an else arm left with no groups and all of the mask of an `if` with none in
either arm. The assembler (`assemble::Arithmetic`) tells the forms apart by
the group before the then marker (the test, or the `cndst`), places each
`br` by building the arm ahead to measure it, and refuses the form out of
order. An `if` whose then arm alone is empty keeps its `cndst p0_false`
and `cndef` and loses the then arm's branch: pco would invert the condition
(`cndst p0_true`), a form no program of pco's table or of its no_pred_cf
output has (NIR inverts such an `if` itself), so it is not emitted.

**The choice** is `PVR_BACKEND_OPT_IF_LOWERING`: `PVR_IF_LOWERING_DEFAULT`
(0, the compiler's choice: today it predicates), `_PREDICATE` (1) or
`_BRANCH` (2), set through `pvr_backend_set_option`
(`shaderc::IfLowering`). It is read when a shader is compiled from its
source; the online back end keys the record by it (`backend::Facts`), and
its blend, link and sampler-state variants keep it, so one source compiles
both ways on one context. The size threshold HANDOFF item 2 asks for is not
in: the crossover waits for the branch's cost (gl_test 195).

Host evidence (`tests/usc_limits.rs`):

- `a_predicable_if_branches_as_pcos_no_pred_cf_does`: fill_color.nodiscard
  and 193's offarm twins (N = 8, 32, 128) at femtovg's blend 0x98661.
  Ours branched is ours predicated plus six control groups (104 -> 110,
  114 -> 120, 138 -> 144, 234 -> 240, the emc one more temporary), as
  pco's no_pred_cf programs (`tests/fixtures/pco_no_pred_cf_98661.txt`, the
  table's toolchain) are its predicated rows plus the same six (105 -> 111
  ... 235 -> 241). The six sit where pco's do, skeleton for skeleton, and
  each is what our builders build from its own operands (the emc and the
  offsets differ); the branches land on the `cndef` and the `cndend`; the
  byte model gives the predicated pixel at radius 0 and 4, pco's branched
  program the same, the branches taken the way the radius says; at radius
  0 the branched program issues 79 groups at every N.
- The survey (`what_the_usc_path_asks...`) compiles every source whose
  program has such an `if` again branched and requires its own verdict
  unchanged: 94 programs (42 vertex), 168 `if`s, every group attested,
  ours against selection's program, the IR and pco's.

Not run on silicon; gl_test 195 is the board's measurement.

### gl_FragCoord.xy, and the flip pair

pco reads `gl_FragCoord.x`/`.y` from the special registers `x_p`/`y_p` (97,
100), or `x_s`/`y_s` (98, 101) when the draw is multisampled, chosen at run
time by `fs_is_single_sampled`: `pck.cov(1.0)`, shifted right by one, tested
for zero (Mesa `pco_trans_nir.c` `trans_load_input_fs`, MIT, transcribed).
A special-class register is read through S1/S2/S4 only (`ref_src_map_valid`
in `pco_internal.h`), so each is moved with `movs1` first. Three templates
were added for it (`USHR`, `TSTZ`, `MOVS1`), each a group pco emits in
fill_gradient.frag (0x5e, 0x68, 0x7a); the prologue selects them once at the
program's start (`Sel::frag_coord_prologue`).

gl_test case 82 on the AM62P (the reference compiler's pair, 63b50b3): the +0.5 is already in
the registers on both targets, and only y is mirrored on the window. So y
goes through a pair the driver writes per draw, `y' = y * scale + offset`,
(-1, height) on a flipped target and (1, 0) otherwise: an `ffma` against
`shared[fragcoord_ytf_off]`, placed straight after the uniforms (the
control word moves two dwords later when a shader has both).

The record names the pair: `uint16_t fragcoord_ytf_off1` at offset 1244
of `shc_shader` (padding before; no field moved), ONE-BASED so that every
zero-initialised record -- pco's table rows, `ShcShader::ZERO` -- means "no pair" rather than "a pair at sh0".
The backend DMAs the pair per draw, keyed on the draw's `flip_y` (the GL
core's `fb_flip_y`: today the window surface, on Zephyr and Linux alike):
(-1, target height) when set, (1, 0) otherwise. When the AM62P's Zephyr
windows become y-up with the display controller mirroring them, `flip_y`
clears for them and the pair follows; the GC7000's y' = H - y keys on the
same bit (gc-backend). The driver words after the uniform block, in order:
the pair, the control word, the front-face op
(`the_driver_words_sit_after_the_uniforms_in_one_order`).
`gl_frag_coord_is_the_centre_register_with_y_through_the_drivers_pair` runs
the record's program in the byte model single- and multisampled, flipped
and not, and checks every group's skeleton against pco's corpus; the 16
femtovg gradient shaders now go through the whole chain (the survey places
the pair as (1, 0), which is pco's unflipped reading).

A draw can now need six common-store DMA kicks (descriptors, uniforms, the
control word, the front-face op, the pair, the tile-buffer addresses). The
program held four and the backend dropped the rest without a word, leaving
registers the shader reads unwritten. The bound is 8 now (Mesa's pvr: one
DOUTD per buffer, `PVR_PDS_MAX_NUM_DMA_KICKS` 32), and a kick that does not
fit fails the draw with -E2BIG and one log line instead.
`gl_FragCoord.z` and `.w` are the Z and W planes iterated with no divide
(`ditr`, count 1), as pco does (Mesa pco_trans_nir.c trans_load_input_fs,
VARYING_SLOT_POS, NOPERSPECTIVE): Z's coefficient set at cf0 when .z is
read (the record's `uses_z`, which the driver's coefficient program already
honours), W's next when .w is read or any varying is listed (`uses_w`),
then `gl_PointCoord`, then the varyings -- and a perspective iterate then
names W at cf4 (`woff`). Selection declares both planes and keeps only the
one the program reads after dead-code removal. The y pair is asked for only
when y is read. `gl_frag_coord_z_and_w_iterate_what_pcos_programs_iterate`
holds four shapes against pco_shc build-new's programs field by field
(coefficient, W offset, count, mode); no table program can, because the
table's 32 readers of .z are Qt particle shaders with two colour outputs.

### gl_PointCoord, and why case 83 read 1.0 with the reference compiler's shaders

pco iterates `gl_PointCoord` with `ditr` (no W divide), two components, in
the eight coefficients straight after W's four -- from 0 when nothing
iterates W -- and places every varying after it (pco_shc build-new: `ditr
coff=0` alone; `coff=4` with a varying then at 12). Mesa's driver feeds those
coefficients with a DOUTI of its own (`pvr_arch_pipeline.c`: GOURAUD, 2D,
`pointsprite = true`, after W/Z and before the varyings).

Ours does not, and cannot know to: `pco_shc` reports `pntc=` on its `uses:`
line, `glsl2usc` parses only `w` and `discard`, so neither the offline
table nor `ShcShader` says the shader needs it, and
the coefficient program (`backend.rs`) has no point-sprite iterator. The
shader iterates coefficients nobody wrote -- gl_test case 83's constant 1.0 --
and a varying beside it would be iterated eight coefficients early.

The record now carries it: `uint8_t uses_pntc` at offset 1242 (padding that
was there; no field moved, sizeof unchanged), parsed from pco_shc's `uses:`
line by `glsl2usc` into the offline table. The table's four gl_PointCoord rows (imageparticle_coloredpoint
and _simplepoint, blend keys 0x0 and 0x98661), regenerated with pco_shc
build-new, are their old rows to the dword but for `uses_pntc: 1`; every
other row gained `uses_pntc: 0` and nothing else. The driver's point-sprite
iterator that reads the flag is am62p-board's.

The online path now selects it the same way (`IterMode::Linear`, the chain
iterating it first) and `compile_fragment` refuses it by name until the
record has the field. `gl_point_coord_is_iterated_where_pco_puts_it` checks
the layout against pco's in both shapes, runs it in the byte model and
checks every group's skeleton. Every ES 1.00 fragment shader of the table
now goes through the whole chain and is refused only by the two record
fields (`fragcoord_ytf_off`, 16; `uses_pntc`, 2).

### Blend variants

The PowerVR blends in the fragment shader: a draw with blending runs a
variant of its fragment shader, `blend_variant(fs, key)`, which the GL core
uploads once per program and drops when blending stops. The online back end
answered every non-zero key with -ENOENT, so the GL core drew the unblended
shader: gl_test case 8 on Zephyr read the plain source in every blended check
(am62p-board, 63b50b3).

`compile_fragment_for(.., key)` now lowers the blend into the program's
tail (`blend.rs`), in pco's forms: the source clamped (`fadd.sat`), the
pixel read from the pixel output with `olchk` and unpacked (`unpck u8888`,
repeat 4), then per channel the two factors and the equation (every GL
factor and equation, colour and alpha apart), a masked channel keeping the
pixel's value, and the usual pack. The record says `uses_fbfetch` (the pass
that allows the read) and carries the key; a factor that reads the blend
colour gets four dwords appended to the uniform block (`blend_consts_off`),
refused beside the control word or gl_FragCoord's pair for now. In a shader
that discards, alpha-to-one applies to the BLENDED colour, as pco orders it
(`Selected::alpha_to_one`); applying it first differed from pco's variants.

Survey: of the table's 481 blend-variant rows for sources that go through,
400 blend exactly as pco's variant on every input (the pixel and the blend
colour varied), 0 differ; 48 are refused by name (render-target classes
RGBA16F/R16F/R32F, Quick 3D's; next), 33 are pco programs the model cannot
run (`ignorepe`). `gl_test_case_8_blends_on_the_host` gives case 8's seven
words. Case 8 on the Zephyr gl_test sample is the deciding check.

### Float render targets (RGBA16F, R16F, R32F)

Quick 3D renders a layer with post-processing into RGBA16F (`qquick3dscene
renderer.cpp`: `postProcessingNeeded = firstEffect`, and YakoGL advertises
`GL_EXT_color_buffer_half_float`), opaque passes and blended transparent ones
alike. The key's class (bits 30..31) now selects the target's format, as in
pco's float variants (pco_shc build-new, `...@f16x4`, `@f16`, `@f32`):

- RGBA16F: two `pck f16f16` (repeat 2) into two dwords, written to pixel
  outputs 0 and 1 (the record's `num_outputs` 2); R16F one `pck f16f16`
  (repeat 1); R32F the float written as it is.
- A blend reads the pixel in that format (`unpck f16f16`, one or two
  dwords; R32F as a float), the channels a format lacks as (0, 0, 0, 1),
  and does NOT clamp the source -- only an RGBA8 target is UNORM.
- A variant that does not read the pixel (a plain replace) reads nothing
  and does not set `uses_fbfetch`, as pco's replace rows do not.

The table carries float BLEND rows: `tools/glsl2usc.sh` asks for Quick 3D's
premultiplied blend into RGBA16F (`one,inv_src_alpha@f16x4`, key 0x40098661)
for every qt3d and es300 fragment shader, 192 rows compiled by pco_shc
build-new. Survey: 468 of the table's 501 variant rows blend as pco's on
every input (the 48 float replace rows and 20 float blend rows the model can
run included), 0 differ; 33 pco programs the model cannot run.

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

### gl_FrontFacing

pco computes it in the shader (Mesa `pco_nir_pvfio.c` lower_front_face, MIT,
transcribed in `Sel::front_face_prologue`): front is the ISP's `face_ccw`
(special 44, read through S1) being zero, then a front-face op the driver
writes per draw chooses, `r = front; r = op == 1 ? !front : r; r = op == 2 ?
true : r` (0 keeps the ISP's sense, 1 swaps it, 2 for points and lines;
src/pvr/backend.rs has written it for the table's programs all along).

- **where**: one dword after the uniforms, the `gl_FragCoord` pair and the
  control word, each when the shader has one (`select::ff_dword`), moved past
  the texture descriptors like the rest; pco puts it at sh0 when the shader
  has nothing else, and so do we.
- **record**: `fs_front_face_off1` = that register plus one, and only
  while the program still reads the op: a `gl_FrontFacing` whose value
  nothing uses is gone after dead-code removal, and the record then asks
  for no op, as pco's does (8ad4bf6d52933374.frag). No new field.
- **refused with it**: a constant-colour blend variant, whose four dwords
  sit at the same place (as with the control word and the flip pair).
- **checks**: `gl_front_facing_is_face_ccw_through_the_drivers_op` runs the
  built program in the byte model for all three ops on both faces, with
  noise in every other shared register, in a plain shader and one that also
  discards and reads a uniform (the op at 5, the control word at 4), and
  checks every group's skeleton against pco's corpus. gl_test case 49
  (es3facing, window and FBO, CW and CCW) covers ops 0 and 1 on silicon; no
  case draws a point or a line with it yet.
- **survey**: the face and the op cycle with each run and the IR is told
  their meaning; the comparison with pco places both at each record's own
  offset (they used to be sentinel noise, which meant op 0 on a front face
  only). facing.frag is pco-identical; 266 of 266 ifs are driven both ways.
  One program, 52660c1a3d65bec6.frag (a Qt lit material), differs from pco
  on a back face while ours and the IR agree to the pixel (88eba1f1 against
  pco's 88ebacf1): named in `PCO_ALONE_DIFFERS`, admitted only while the IR
  sides with ours exactly. A negated-normal run on a front face agrees, and
  `a_tangent_frame_flipped_by_gl_front_facing_matches_the_ir_on_both_faces`
  checks the frame's flip alone; a board capture of a back-facing lit
  material on the table path against a reference decides whether pco's
  program is wrong.
