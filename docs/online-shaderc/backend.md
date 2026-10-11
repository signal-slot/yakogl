# The online shader back end (M3): module, ABI assembly, oracle

The design record for the back-end half of `src/shaderc/online/`: our own
GLSL ES 1.00 compiler, compiled into the image and run on the target. Written
as the work happens, so a decision that is still a proposal says so.
`frontend.md` is the front end's record; `isa.md` and `gc7000-isa.md` are the
instruction sets' ("The codec in Rust and how it is measured").

**Target.** Task #55 was retargeted on 2026-09-22 to the **Vivante GC7000UL**
(i.MX8MP) first, and much of this file was written for it. The compiler is now
the product back end on all three GPUs: the PowerVR (`powervr/`), the GC7000
(`vivante/`) and the V3D (`v3d/`, `v3d-backend.md`). Everything in this file is
GPU-neutral unless it says otherwise, and where the GPUs differ the difference
is named -- the biggest one being where the reflection lives, below.

The milestone split (task #55): **M1** the USC decoder/encoder,
**M2** the GLSL ES 1.00 front end and reflection
(es100-frontend), **M3** this — the back-end module, the `ShcShader` assembly
and the oracle harness (gles2-core), **M4** `gl_test` 24/24 through the online
path, **M5** a Zephyr image with no table.

## Why a back end on the target

| | compiles where | needs | can ship |
|---|---|---|---|
| `offline.rs` | on the build host | every shader known at build time | yes, for a fixed shader set |
| `online/` | on the target | nothing | yes |

`offline` cannot serve an application that writes its shaders as it runs —
dEQP-GLES2 does, Qt's RHI does, any content pipeline does. This one closes
that gap on the target.

## Status

| Piece | State |
|---|---|
| `arena.rs` / `store.rs` — the memory model | **done** |
| `ir.rs` — the front end / code generator contract | **agreed** and implemented; `ir.md` is the contract |
| `backend.rs` — `ShcBackend`, cache, selection, blend variants | **done** |
| the front end (M2, es100-frontend) | **done** for GLSL ES 1.00: preprocessor, parser, sema, lowering to IR, reflection; scored by `tools/es100_oracle.py`. GLSL ES 3.00: `docs/es3-shaderc-coverage.md` |
| the encoder and allocator (M1) | **done**: round-trip over the table's blobs, `allocate_and_build` |
| `select.rs` — instruction selection | **done**: io, literals, uniforms, the `native` arithmetic and the `lower` library (the block below is resolved) |
| `abi.rs` — the `ShcShader` assembly | **done**, eight named refusals |
| `codegen.rs` — the chain | **closed end to end**: GLSL text in, `ShcShader` out |
| the oracle harness (`tests/online_oracle.rs`) | **running**, on both GPUs; see below |

The chain is whole: a `shaderc-online` build compiles GLSL ES 1.00 text to a
`ShcShader` in process, with no table and no network. What limits it now is
coverage, which the oracle counts, and not a missing stage.

**Resolved since (2026-10-11 reading of the code): `vivante/builtins.rs` now
uses `alloc.rs`'s own `SelInstr`, `SelDst`, `SelSrc` and `VReg`
(`builtins.rs:85`), so there is one definition. The account of the block, as
written then, follows.**

**One thing is blocked, and it is worth naming rather than routing around.**
`builtins.rs` (the lowering library) declares its own `SelInstr`, `SelDst`,
`SelSrc` and `VReg`, a second copy of the interface `alloc.rs` owns, and the
two have diverged: `op` is `&'static str` against `OpId`, `saturate` exists on
one side only, and `SelSrc`'s third variant is `Uniform { reg, .. }` against
`Physical(Src)`. That last one cannot be adapted in the direction the code
generator needs -- a `Physical` operand that is not a uniform register has no
representation on the other side -- and a conversion that drops what it is
given emits a plausible instruction reading the wrong register, which draws a
picture rather than failing. So `select.rs` refuses the non-native path with
that reason in the error string. Reported to pvrgpu-platform; the fix is one
definition. Nothing needed by the first programs is affected, because all of
`mul`, `mad`, `add` and the dot products are `native`, and `native` returns a
name, which both sides agree on.

`tools/check.sh` runs `shaderc-online` with each GPU, alone and with
`report-es2`: with `gpu-vivante` (the GPU it is being built for first, where the oracle harness
compares a different set of fields). By the gate's own rule, a feature it never
combines is a feature it does not cover.

## Memory: two arenas, no allocator

The crate has no `alloc` dependency and no global allocator. The online back
end must run on Zephyr and bare metal, so it takes two bump arenas in `.bss`:

- **`with_scratch`** — reset at the top of every compile. Tokens, AST, IR, SSA,
  register allocation. Nothing in it outlives the call, and `reset` takes
  `&mut`, which is what proves no allocation is still borrowed.
- **`keep`** — outlives the compile, freed per shader. The USC dwords, the
  `ShcUniform` rows, the name pool and the source of every compiled shader,
  because the `ShcShader` the GL core copied points into it and the C ABI has
  nowhere to put a lifetime. Each shader's extent goes back when the GL core
  releases its last copy (see "The free path" below).

`SCRATCH_BYTES` and `KEEP_BYTES` are **measured**, by the oracle harness over
all 307 sources in the table: peak scratch 931,496 bytes, keep 5,288. The
report prints both every run against the constants, so the headroom is a number
a reader sees rather than one someone remembers.

**The keep arena ran out, and what it cost.** `KEEP_BYTES` was 128 KiB, set
from a run in which **two** sources compiled and used 5,288 bytes. When
coverage reached 82 the arena filled and 43 sources were refused with
`Abi(OutOfMemory)` -- appearing in the work list as compiler gaps. The oracle
reported **39 of 142 when the truth was 82**. The line that caught it was the
harness's own `keep used 131060 bytes of 131072`.

The rule: *a limit measured when the thing was small, and described in its own
comment as having wide margin, is a limit that will be reached without anyone
deciding to reach it.*

**The number to size from is one shader, not a corpus.** The oracle now resets
the keep arena between sources (`backend::forget_compiled`, `host-test` only,
which clears the compile cache in the same call because every `ShcShader`
becomes dangling), and reports the largest single shader:

| measurement | bytes |
|---|---|
| largest single compiled shader | 6,408 |
| whole 324-source corpus, accumulated | 300,768 |

512 KiB therefore holds about 80 live shaders at once, which is far more than
any of our applications builds.

**The free path (2026-09-23).** Until then `glDeleteProgram` could not give
this memory back, and after about 200 compiles in one process every compile
was refused with `Abi(OutOfMemory)` -- dEQP-GLES2 on the host ran 199 honest
cases and then 7,299 identical refusals. Now:

- `store.rs` keeps a map of 64-byte granules over the region. Each compile
  opens a bump arena over the largest free run (`begin_keep`), the code
  generators allocate in it through `store::keep()` exactly as before, and
  what the compile used becomes one `KeepExtent` the shader owns. A failed
  compile drops its transaction and keeps nothing.
- `backend.rs` counts the holders of each compiled record. The GL core takes
  a reference for every copy of a `ShcShader` it keeps -- the shader object's
  (released on recompile and delete), a program's snapshot at link (released
  on relink and delete), a blend variant's (released when the slot is
  recycled or the program's GPU state is invalidated) -- through
  `shaderc::retain_shader` / `release_shader`. The last release frees the
  extent and takes the record out of the cache; the cache itself holds no
  reference.
- The seam is not the C `struct shc_backend`: the in-process compiler
  recognises its own shaders by `priv` pointing into its region (and a live
  record's magic), so the GL core can call the pair on any back end's
  shaders and whichever back end is current.

Measured, host-only: `operator.binary_operator` (1,318 cases) in ONE
process compiles all 2,636 shaders and links all 1,318 programs with 0
refusals; with `release_shader` made a no-op the same run links 168 and
refuses the rest with "out of compiler memory". Pinned by
`backend::tests::ten_thousand_compiles_each_released_keep_the_region_flat`
and `gl::program::tests::deleting_programs_and_shaders_gives_the_compilers_region_back`.
What the region must hold is now the shaders an application keeps alive at
once, not every shader it ever compiled.

**Decided, not open:** `SCRATCH_BYTES` stays a 2 MiB static until M5. At M5 it
becomes caller-lent -- an `arena::Arena` over memory the application hands in,
sized by a Kconfig whose help text quotes the harness's high-water mark as the
floor. A product image should not carry two megabytes of `.bss` it cannot see
or size, and a Kconfig with a measured floor is how the person building the
image gets to make that trade. The high-water print stays after the move; it is
what keeps the floor honest as coverage grows.

**Exhaustion is its own error**, never a short allocation and never a silent
fall-back to the table. An arena that returned what it had left would hand the
code generator a USC buffer shorter than the program, and the shader would be
quietly cut — a compiler that ran out of room looking exactly like one that
compiled a smaller shader. The failures stay apart in the errno too:

| errno | means |
|---|---|
| `-ENOTSUP` | the compiler cannot do this yet; the log names the milestone |
| `-ENOMEM` | the keep region has no room now (it may have once something is freed); the log names the constant to raise |
| `-ENOSPC` | the program does not fit the scratch arena, which is reset for every compile, so it never will; the log names the constant to raise |
| `-EINVAL` | the shader is wrong (the front end's diagnostic is in the log) |

An application, and a conformance run, act differently on each. A build that
reported all three the same way would make "not implemented" indistinguishable
from "your source is invalid".

## The IR contract

**It lives in `ir.md`, and only there.** This section used to restate it, and
the copy went stale the moment the vector form was agreed: it still read
"Scalar, not vector -- the front end scalarises", which is the opposite of the
decision, while the header of `ir.rs` said vectors. es100-frontend found the
two instructions and stopped, correctly, rather than guessing which to build
to.

The copy is deleted rather than corrected. A contract restated in two documents
does not stay consistent by being careful; it stays consistent by existing
once. `docs/online-shaderc/ir.md` is the contract, `src/shaderc/online/ir.rs`
implements it, and those two change together.

What belongs *here* is only the part that is this file's own: the back end
fills every field that falls out of register allocation -- `temps`,
`ShcViv`'s registers, the constant-file table, `usc`/`usc_dw` -- and filters
the reflection rows the front end numbers. The split itself, and the reason for
each side of it, is in `ir.md`.

## Blend variants

Blending and colour masks are lowered into the fragment shader on this GPU, so
a blend state is a different binary from the same source. The offline table
precompiles one variant per state in the `blend` lines of
`tools/shaders/precompiled.list` — every
state named ahead of time, whether a draw ever asks for it or not.

The online back end has no list. The source is still in the `keep` arena, so
`blend_variant(fs, key)` compiles exactly the key a draw actually asked for and
caches it under that key. A shader from another back end carries a `priv` this
module cannot read, so it is refused rather than guessed at.

## The oracle harness (`tests/online_oracle.rs`)

**There are two oracles, and they ask different questions.** Do not read one
for the other.

| | corpus | question |
|---|---|---|
| `tools/online_oracle.py` (es100-frontend) | dEQP-GLES2's own `.test` cases | does the front end *accept and reject* the right shaders? |
| `tests/online_oracle.rs` (this one) | the offline table's own sources | does the compiler *produce* what the table produces -- reflection now, code later? |

A shader can pass the first and fail the second (compiled, but with the wrong
uniform offsets) or pass the second and fail the first (right output for the
shaders in the table, wrong verdict on a shader that should not compile). Both
numbers are needed, and neither substitutes for the other.

Every source in `SHC_SOURCE_TABLE` through the new compiler, reflection diffed
field by field against that source's table row, reported as **a table of
differences rather than a pass/fail**: a pass/fail is one bit over hundreds of
sources and the first run will not pass, while "which field disagrees on how
many shaders" is the next hour's work. It needs no code generator, no encoder
and no board, so it is es100-frontend's M2 scoreboard as much as M3's item.

```sh
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
    --test online_oracle -- --nocapture
```

Today, on the GC7000 build: 307 sources, 304 with a blob row, 3 variant
entries, 0 unexplained, 304 not compiled ("the GLSL ES 1.00 front end (M2) is
not implemented"). On the PowerVR build: 533 sources, 533 rows, 0 unmatched.

**It scores the oracle before it scores the compiler**, because a harness that
could not find a source's row would report "no differences" for it, and that
silent zero reads exactly like a clean result. Two things it caught on its
first GC7000 run, both defects in the harness itself:

- **The GC7000 keeps its reflection somewhere else.** Every blob of the shipped
  Vivante table has `num_varyings = 0` (307 of 307, counted); the inter-stage
  wiring is `ShcViv::in_` / `out`, as register lists. A comparison that stopped
  at `varyings` would have reported agreement on everything that GPU actually
  uses. The harness now compares the `ShcViv` fields under the GPU cfg, and a
  self-test asserts that a change to any of them shows up -- including
  `vs_load_balancing` and the constant file.
- **Three source entries hold text that is in no blob row.** The generator
  writes `<base> @<variant>` entries whose text is the variant's *rewritten*
  GLSL while the blob rows are keyed by the base source's digest. Measured:
  `facing.frag @front_cw` holds text hashing to `e7e95724...`, and both of its
  rows carry `8e9a6255...` (the base) with blend keys 0 and 1. The harness
  recognises them and scores them through their base; anything else with no row
  still fails the run.

That second one is worth someone else's attention: `SHC_SOURCE_TABLE` is
documented in `shaderc/mod.rs` as "the GLSL text a blob was built from ... what
an application must pass to glShaderSource byte for byte on an offline-only
target", and `gles_offline_shader_source("facing.frag @front_cw")` returns text
whose digest is in no row, so an offline-only target would refuse it. It is the
Vivante table generator's to decide; the harness pins the current behaviour so
that the day it changes, the run says so.

## Booleans: an all-ones integer mask, measured

A `bool` is not a machine type on the GC7000, and the code generator refused
every boolean operation until the representation was settled. It could have
been float 0.0/1.0 or an integer mask, and choosing wrongly is silent: a `not`
in the wrong type clears the wrong bits and the shader computes a plausible
wrong answer.

Settled by measurement on 2026-09-22, by compiling four sources through the
reference compiler in one process and decoding what came back:

| GLSL | what the reference emitted |
|---|---|
| `float(u > 0.5)` | `cmp.gt r1.x` then `and.u32 r1, r1, #0x3f800` |
| `!(u > 0.5)` | `cmp.gt r1.x` then `not.s32 r1.x, r1.x` |
| `any(greaterThan(v, ...))` | `cmp.gt r1.xy` then `or.s32 r1.x, r1.y, r1.x` |
| `b ? a : c` | `cmp.gt r1.x` then `select.u32.nz r1, r1, r2, r2` |

`0x3f800` is the top 20 bits of `1.0f`, and `and`-ing a mask with it yields
1.0 only if the mask is **all ones**. That fixes the representation.
`builtins.rs` had already chosen all-ones, as Mesa does, so the measurement
confirms that choice rather than making it.

The types are the reference's own, field by field: **U32** for the
mask-to-number `and` and for `select` -- including where both of the select's
arms were float `vec4`s, because selecting on a mask is a bitwise choice and
not arithmetic -- and **S32** for `not` and for a reduction's `or`/`and`.

That last point settles `Bcsel`, which the IR cannot type on its own: its arms
may be float or int and the opcode does not say which. U32 is right for both,
so the ambiguity never has to be resolved.

This took the score from 83 to 96 of 142.

### Three differences that remain, none of them wrong

Measured the same way, ours against the reference on the same sources:

1. **An extra trailing `mov`** on `float(u > 0.5)`: the reference ends at the
   `and`, we add a copy into the colour register. The `and` could have been
   retargeted to write it directly.
2. **A reduction is four lanes wide when the source is a `bvec2`.** The
   reference emits one `or`; we emit three. `or` and `and` over repeated lanes
   are idempotent, so the answer is right and two instructions are wasted.
3. **The reduction's type is U32 where the reference used S32.** `builtins.rs`
   hard-codes U32 for `Bany`/`Ball` and ignores the type the caller passes. The
   two are bit-identical in effect for a bitwise `or`/`and`; it is one field,
   the same class as PMODE, and it belongs to whoever owns `builtins.rs`.

Not in this list because it is not the back end's: `?:` and `!` are still
refused *by the front end* ("this expression is not lowered yet"), so the
`Bcsel` and `Bnot` paths above are reachable from the IR but not yet from
GLSL.

## The built-in fragment inputs, measured

`gl_FragCoord` **is temp register 0**, all four components. Measured against
the reference compiler, which reports `num_in 0` every time -- it is not an io
register and must not join the io list:

| GLSL | what the reference emitted |
|---|---|
| `gl_FragCoord.xy * k` | `mul t?, t0.xyxx, uniform` |
| `gl_FragCoord.zw` | `mov t?, t0.zwzz` |
| `gl_FragCoord` | `mov t?, t0.xyzw` |

This explains a fact that had been measured but not understood:
`FS_COLOR_MIN_REG` is 1 because 0 of the 138 real fragment blobs write their
colour to register 0. They do not, because the rasteriser has already put
`gl_FragCoord` there.

The other two are still refused, each for its own reason rather than one
shared shrug:

- **`gl_FrontFacing`** arrives in an **internal** register (`RGROUP_INTERNAL`),
  not a temp. The reference compares internal register 0 against integer 0 with
  COND 6 to build the mask. That needs an operand kind the selector does not
  have yet, and no source in the corpus asks for it.
- **`gl_PointCoord`** has no reference answer at all. Asked for one, the
  reference compiler aborts: `etnaviv_compiler_nir_liveness.c:94: set_src_live:
  Assertion 'i != ~0u' failed`. There is nothing to copy, and picking a
  register would be inventing an ABI.

### The bug this found

`gl_FragColor = gl_FragCoord` compiled to an **empty program**.

The selector has a measured rule that the fragment colour goes wherever it
already is: `gl_FragColor = v_col` gives the colour that varying's own register
and emits no instruction, which is exactly what the reference does. Once
`gl_FragCoord` became selectable it was an input too -- in register 0 -- so the
rule fired and claimed the colour was already in place there. It is not: the
colour may not live in register 0, which is the same fact `FS_COLOR_MIN_REG`
records.

The rule is now guarded by that constant, and `gl_FragColor = gl_FragCoord` is
byte-identical to the reference.

Worth noting how it was found, because the oracle could not have: **every
source in the corpus that reads `gl_FragCoord` also uses control flow**, so all
eight still refuse and the score does not move at all. The bug was only visible
by compiling one line of GLSL through both back ends in the same process and
diffing the words.

One difference that remains and is not wrong: `vec4(gl_FragCoord.z,
gl_FragCoord.w, 0.0, 1.0)` is five instructions to the reference's two, because
the constructor is built component by component instead of being folded into
one swizzled read.

## Control flow: the reference does not branch, it if-converts

Control flow is the whole of the back end's remaining refusals (41 sources), so
the question is what to build. Asked the reference compiler, and the answer is
not what a textbook orders:

| source | what the reference emitted |
|---|---|
| `if`/`else` assigning a colour | `cmp` then `select` -- **no branch** |
| arms of seven ALU operations each | no branch, temps 5 |
| nested `if` inside an `if` | no branch |
| `for (i = 0; i < 4; i++)` | no branch -- unrolled |
| one arm holds `texture2D` | **`branch_unary` COND=0xa TARGET=4**, then `branch TARGET=5` |
| loop with a `break` | **`branch_unary`** |

So it if-converts everything it safely can, and branches only when an arm holds
something it must not execute on the wrong side of the condition -- a texture
fetch -- or when a loop cannot be unrolled.

### What the corpus is actually asking for

Measured by `tests/cf_survey.rs` over the sources that lower, which agrees
exactly with the oracle's count of 41 refusals:

| | sources |
|---|---|
| any control flow | 41 |
| `if` only, no loop or `break` | 39 |
| need a loop or a `break` | 2 (`filter_image.frag`, `filter_image.nodiscard.frag`) |
| hold a texture or `discard` inside an arm | 16 |

116 `if` nodes against 2 loops, and between one and seven `if`s per source.

**So if-conversion alone unblocks 23 of the 41** -- with no CFG, no branch
instruction, no back-patching, and nothing that invalidates the straight-line
assumptions the rest of the selector rests on. Real branches are worth 16 more
and loop unrolling 2, and both are separate pieces of work that can be ordered
on their own evidence rather than bundled into "control flow".

One caution recorded because it cost a wrong headline: the question is what is
inside an **arm**, not what is in the body. A texture fetch in straight-line
code is not speculated by anything. Asking it of the whole body reported 27
sources blocked instead of 16.

### What it emits, and what it costs

Landed: **96 -> 109 of 142**. The shape, per written slot:

```text
  mov saved[l],  slot[l]        for every slot either arm writes
  <then arm>                    stores land in slot[l] as usual
  mov then[l],   slot[l]
  mov slot[l],   saved[l]       put it back for the else arm
  <else arm>
  select.nz slot[l], cond, then[l], slot[l]
```

An `if` with no `else` needs one register per slot rather than two and skips
the copy-out and the restore: the else value is the pre-`if` value, which
`saved` already holds. There is no special case for "no else" beyond that --
the select simply reads `saved`.

Checked against the reference on three sources. The operations agree, the
counts do not:

| | ours | the reference |
|---|---|---|
| `if`/`else` | `cmp` + 5 `mov` + `select` + `mov` | `cmp` + `select` |
| `if` with no `else` | `mov` + `cmp` + 2 `mov` + `select` + `mov` | `cmp` + `and` |
| nested `if` | 2 `cmp`, 2 `select`, 11 `mov` | 2 `cmp`, 2 `select` |

The `cmp`s and `select`s match one for one, including for the nested case. The
copies are the price of correctness before liveness: the reference reaches the
same answer in one `select` because it knows which registers are dead. This
compiler does not, and a reused register that was still live draws a picture
rather than failing, so it copies. It is a quality gap with a known cause, not
an unknown.

Two numbers that did not come out as predicted, recorded because the
prediction was wrong: if-conversion was supposed to unblock 23 and unblocked
13. Nine of the missing ten hit `Alloc(OutOfRegisters(257..262))` -- against
`MAX_VREGS = 256`. Those are VIRTUAL registers, counted before allocation, so
it is an array bound in `vivante/alloc.rs` and not a hardware limit. Halving
the reservation for an `if` with no `else` did not recover any of them, so
they are not marginal in the way the numbers suggest.

### Only a leaf `if`, because nesting it is quadratic

**Measured 2026-09-22, and it is the largest quality defect this file has
had.** dEQP's `hsl_to_rgb_vertex` -- the shader that hangs the GC7000 (task
#52g) -- compiled to **222 instructions in 60 temps** where the reference
compiler needs 50 in 10. Broken down by opcode through `tests/gltest_diff.rs`:

| | instructions | `mov` | `select` | branch | real work |
|---|---|---|---|---|---|
| ours | 222 | 156 | 41 | 0 | 25 |
| reference | 50 | 22 | 7 | 1 | 20 |

The real work is the same size. Everything else was if-conversion's
bookkeeping, and the reason it was so much of it is that it **compounds with
nesting**: each level saves and restores every slot its whole subtree writes,
and `hsl_to_rgb` is six `if`s deep over nineteen slots. Each save and each
then-value is also a fresh register whose live range spans the arm, so the
same nesting produced the 60 temps -- which is above the 21 its
load-balancing band was measured over, and at 67 (before the allocator's
sinking) above the six bits the driver puts the count on the wire in.

Three arms, measured over the wedge pair and the whole corpus:

| | hsl VS temps | hsl VS instrs | corpus score |
|---|---|---|---|
| if-convert everything | 60 | 222 | 130 of 142 |
| **leaf `if` only (what this does)** | **13** | **84** | **137 of 142** |
| never if-convert | 10 | 74 | 137 of 142 |

Seven sources move from refused to compiled, every one of them refused for a
temps count no load-balancing band covers.

Never if-converting scores the same and is ten instructions cheaper again, and
it is still the wrong choice: **the reference if-converts.** Its `hsl_to_rgb`
has seven `select`s and one branch. So a leaf `if` is worth converting and the
nesting is not, which is exactly the rule.

### Each arm writes its own registers, and the join picks

The second half, landed after the rule above and measured separately. An arm
used to write the slot itself, which meant undoing it afterwards:

```text
  mov saved[l], slot[l]      always
  <then arm>                 stores land in slot[l]
  mov then[l],  slot[l]      always
  mov slot[l],  saved[l]     always -- put it back for the else arm
  <else arm>
  select.nz slot[l], cond, then[l], slot[l]
```

Three `mov`s per written slot, unconditionally. Now each arm writes its own
register and nothing has to be undone:

```text
  mov then[l], slot[l]       ONLY when the arm can read it first
  <then arm>                 stores land in then[l]
  mov else[l], slot[l]       likewise
  <else arm>                 stores land in else[l]
  select.nz slot[l], cond, then[l], else[l]
```

The redirection lives in one table (`Sel::locals_in`) that `local_reg` reads,
because every read and every write of a slot goes through it and one of them
missing the redirection is a shader that computes the wrong thing without
failing.

The seeding copy is needed only when the arm can observe the slot's old value:
it reads the slot, or it writes some of the lanes and the rest must survive.
Most arms do neither. An `if` with no `else` now needs no copy at all and one
register per slot, because the false answer is the untouched slot itself.

Corpus-wide, over the 137 sources that compile (`tests/mov_survey.rs`):

| | instructions | `mov` | corpus temps | vs the reference |
|---|---|---|---|---|
| save / restore | 7,598 | 3,703 (48%) | 1,736 | 2.02x |
| both arms renamed | 6,465 | 2,570 (39%) | 1,778 | 1.72x |
| **then arm only** | **6,441** | **2,546 (39%)** | **1,657** | **1.71x** |
| the reference | 3,763 | 580 (15%) | — | 1.0x |

1,157 instructions, all of them copies, and 79 temps, over the same 137
sources (`tests/temps_survey.rs`). The wedge pair, against the reference's 10
temps / 48 instructions for the first of them:

| | save / restore | then arm only |
|---|---|---|
| `hsl_to_rgb_vertex` | 13 temps, 84 instrs | **11, 77** |
| `rgb_to_hsl_vertex` | 16, 66 | **16, 62** |

What is still owed: the reference is at 1.0x with 15% copies, so a third of
our instructions are still copies somebody does not need to emit. That is
liveness and coalescing, not a shape.

**Nothing here is verified on silicon.** The shape of every shader with an
`if` changed, so the gl_test pass list must be re-measured on the board before
any of it is believed.

### What if-conversion may not keep

The IR contract says values are SSA within straight-line code and anything
crossing control flow goes through a `Local` slot. Two of the selector's
optimisations read the flat instruction array and assume one path through it:

- `local_is_stable_after` folds a slot's store into a later load when no
  further store to that slot exists. Across an `if` that is **unsound**: a
  store in one arm with no later store would fold into a load after the `if`,
  as though the arm always ran.
- `propagable_store` propagates a slot's value the same way.

Both must be off for a module that contains an `If`. Conservative, sound, and
costs only quality on the newly-compiling sources.

## Loops: one branch backwards, named, and refused unless it can end

Two of the 142 ES 1.00 sources hold a loop (`filter_image.frag` and its
`.nodiscard` twin, FemtoVG's separable blur). Both are the same shape, because
it is the only shape the front end builds: the exit test lives **inside** the
body, as `if (c) {} else { break; }` (`lower.rs`, `break_unless`), for `while`,
`do`/`while` and `for` alike. So a loop needs exactly two things the `if` work
did not already provide.

**A branch that goes backwards.** There is no loop instruction and no trip
counter on this machine; a loop is the body followed by one `branch` to its
first instruction.

```text
L_top:
  <body>              the exit test is inside it
  branch -> L_top     the back edge
L_end:
```

**A `break` that goes forwards to a destination that does not exist yet.** It
is emitted with a placeholder target, recorded, and patched when the loop
closes -- `branch_convert`'s back-patch over a list, because a loop may hold
several. Entries below the mark taken on entry belong to an enclosing loop and
are left for it. `continue` is **refused by name**: the corpus has 4 breaks and
0 continues, and the front end copies a `for`'s step to each `continue`, so an
untested implementation of that is worth less than a refusal that says what is
missing.

### The back edge is permitted per branch, not per program

`abi::assemble` refuses any backward branch target (`every_branch_goes_forward`,
added for the GPU wedge: a program that never ends is one the driver can only
end by resetting the GPU). A loop needs to jump backwards exactly once, so
selection hands the validator the **instruction index** of each back edge in
`Selected::back_edges`, and the validator excuses those and nothing else.

Per program would have been easier and is wrong: "this shader has a loop" would
then excuse every backward jump in it, including one from a bug in this file,
which is the class of defect the validator was added for.

**Copy propagation deletes instructions, so the indices move.** The compaction
already remapped every branch *target*; the back-edge *list* is the same kind
of index and is remapped through the same map. Forgetting it leaves the
validator permitting a backward jump at some other instruction's index and
refusing the real one -- and it is not hypothetical: in the test for it,
selection emits seven instructions and propagation deletes one, so the back
edge moves from 6 to 5.

### A loop that cannot end is refused, not emitted

Selection refuses a loop whose body cannot reach a `break` of its own (a nested
loop's `break` leaves the inner loop, so it does not count). Every loop the ES
1.00 front end builds carries its exit test as a `break`, so this costs nothing
real, and what it buys is that `for(;;)` with no way out is a compile error
rather than a wedged GPU -- which on the Zephyr image means a reset, a lost
frame, and a safety event on a cluster.

It is worth being exact about what this does **not** promise: nothing here
bounds the trip count. `while (x < y)` with the wrong arithmetic still runs
forever. What is guaranteed is that the only way back is the one the source
asked for.

## The fifteen gl_test cases that drew nothing: one cause, and it is register 0

M4 had 20 gl_test cases compile and draw the wrong picture, of which cts-runner
classified **14 as drawing nothing at all**. One cause covers 13 of them.

**Our compiler allocates the fragment stage's register 0, and the rasteriser
has already put `gl_FragCoord` there.** The allocator's assignment loop starts
at register 0 and skips only what an instruction has reserved, so a fragment
shader that never *mentions* r0 is handed it for its first temporary.

`color_uv.frag` -- `gl_test_fs_src`, the default pair that eleven cases link:

```text
ours  temps 4, colour in r3        reference  temps 3, colour in r1
  mov r0.xy                          mov r2.xy
  mov r0.z                           mov r2.z
  mov r0.w                           mul r1
  mul r3
```

`vec4(v_uv, 1.0, 1.0)` is built in r0 by us and in r2 by the reference.

### The correlation, over every shader with a known board outcome

| shader | writes r0 | cases | board |
|---|---|---|---|
| `color_uv.frag` | **yes** | 3 4 5 6 8 10 11 12 13 14 22 23 | black |
| `texture2.frag` | **yes** | 2 (its two-unit checks) | black |
| `texture.frag` | no | 18 60, case 2's single-unit checks | pass |
| `compact.frag` | no | 24 | pass |
| `interp_color.frag`, `uniform_color.frag` | no (byte-identical) | 1 34 | pass |

**No passing case uses a fragment shader that writes r0, and 13 of the 14
black ones do.** That also answers the discriminator pvrgpu-platform set --
case 2 has 25 checks passing, so a cause that broke single-unit draws would be
the wrong one: `texture.frag` does not write r0, so those draws are untouched.

Case 19 is the one black case not explained; it links its own
`es100_hello`/`es100_tex` pair, which is not measured here.

### The fix is one line, and it is not in this file

`vivante/alloc.rs` needs `reserved[0] = true` for the **fragment stage only**,
which means `allocate_and_build` has to be told the stage. Verified locally
behind an environment variable: with r0 reserved, no fragment shader in the
gl_test ES 1.00 set writes it, and `color_uv.frag` moves its constructor to r4.

**Fragment only.** Measured over the shipped table: **148 of 171** real vertex
blobs write register 0 and only **3 of 138** fragment blobs do (all three ES
3.00). Reserving it in both stages would cost every vertex shader a register
for nothing.

## Constants in operands: two rules, both measured, and a defect found by asking

`src()` reads a uniform or a literal straight into the operand that uses it,
so the `mov d, <constant>` instructions that remain -- **426 of our 2,424
copies**, 156 uniform and 270 immediate -- are the ones something forced into
a register: a local slot, an if-conversion, a lowering's scratch.
`fold_constant_reads` deletes them by making every reader name the constant
directly.

Whether the machine ALLOWS that is two separate questions, and both were
measured from the reference compiler's 13,908 instructions before the pass was
written (`tests/operand_banks.rs`).

**How many operands of one group an instruction holds:**

```text
  group          0 ops     1      2      3    max
  temp            1019   6200   5537   1152    3
  uniform_0       9544   3499    823     42    3
  immediate      12100   1480    328      0    2
  internal       13904      4      0      0    1
  uniform_1      13908      0      0      0    0
  thread         13908      0      0      0    0
```

The immediate column reproduces an independent count exactly,
which is what says the reader is right; the first version of that reader was
wrong by 34x and its only guard (`instrs > 0`) passed anyway, so the test now
also carries a control blob whose two operands are read off the disassembly.

This closed a hazard rather than confirming one. `src()` already folds uniform
reads into consumers, so a limit of one constant-file operand per instruction
would have meant we were already emitting something invalid. There is no such
limit: the reference puts two in 823 instructions and three in 42.

**Which operations take an operand of which group** (`OpId::takes_group`):
`texld`'s 568 source operands are every one a temporary, and so are a branch's
208. A fold into either is refused -- the reference's silence is not proof the
machine refuses, but being wrong about a refusal costs an instruction while
being wrong about a fold costs a picture. `texldl` and `txf` DO carry
immediates, which is why the rule is per operation rather than "texture
instructions". The test derives both lists from the shipped blobs and fails if
the compiler's copy drifts.

**What the pass is worth**, measured on the same tree with the call in and out:
6,261 -> 6,185 instructions, seeds 426 -> 350, temps 1,639 -> 1,614. 84 seeds
folded, of which 8 were instructions dead-code elimination would have removed
anyway (its own count falls 129 -> 121), so the two passes' totals add up to
the 76 the program shrank by.

**And what the caps refused: nothing.** 0 declines over a cap, 0 for the
operation, 0 for a modifier, 14 because the copy was not safe to move. The
measurement that took the morning bounds nothing on this corpus -- its value
was closing the `src()` hazard, and the caps stand as a guard for shaders we
have not compiled yet.

### The defect the pass found: a swizzle over an immediate

An immediate's `swizzle`, `neg` and `abs` fields are not modifiers. They are
the top 20 bits of the value (`build::Src::imm_payload`). `compose` permuted
them like lanes, so

```glsl
vec4 c = vec4(0.5);
gl_FragColor = c.yzxw * v;
```

compiled to `mul t2, 0.00036621094:f20, t1` -- a shader that runs, draws, and
computes with a constant nobody wrote. `alloc.rs` guards the same hazard in
two places and says so in both; this path did not, and the corpus never
exercises it (0 of 137 sources), so nothing failed.

The fix is not to carry the swizzle differently but to drop it: an immediate
is one scalar broadcast to all four lanes, so every permutation of them reads
the same number. A reader's negate or absolute cannot be carried at all, and
the fold refuses such a reader rather than approximating it.

### Where we still differ from the reference, by name

Asked the other way round -- every operand WE emit against the same
per-operation rule -- our 6,192 instructions hold exactly one kind the
reference never emits: **18 `rcp`s with an immediate operand**, which is
`1.0 / <literal>` computed at run time where the reference folded it in the
front end. Reachable but not in the corpus: an `if` on a uniform whose arm
holds a texture fetch selects a real branch, and `src()` gives that branch a
constant-file condition, which the reference's 208 branch operands never are.

## What is next, in order

As written on 2026-09-23. Items 1 and 2 are done: the compiler draws on all
three boards (gl_test, dEQP-GLES2 with 0 failures on the i.MX8MP and the Pi 4,
`tools/cts/RESULTS.md`), through Zephyr images (the `/dev/gcgpu` harness item 1
names is retired), and the front end compiles dEQP-GLES2's shaders
(`tools/es100_oracle.py`). Items 3 and 4 stand as the record of how the matrix
pairing was settled.

1. **The first program end to end on silicon**: a passthrough VS and a
   solid-colour FS compiled in process, drawing on the i.MX8MP through
   `/dev/gcgpu`, verified by pixel read-back. No number from this path is
   believed until that run. The vehicle is cts-runner's minimal ES 1.00
   triangle, because `gl_test`'s own shaders are `#version 450` (13 of 46) and
   `#version 300 es` (14) and so are outside an ES 1.00 front end's scope by
   construction, not by a gap.
2. **The front end's coverage**, which is what the oracle counts. Today 2 of
   307 sources compile, and the reasons are all in front-end lowering: "this
   statement is not lowered yet" 73, "this expression is not lowered yet" 42,
   "expected a type" across ~130 (the type keywords the parser does not take
   yet), "unsupported shading language version" 13 (ES 3.00 sources, correctly
   refused). Nothing in that histogram is a code-generator gap.
3. **`mat4 * vec4`**, measured rather than chosen. `hello.vert` in the table is
   exactly `gl_Position = transformationMatrix * myVertex`, and its shipped
   GC7000 blob is `mul` + three `mad`, temps 2 -- **not** four `dp4`s, which is
   the other plausible encoding and the one a reader expects. `ir.md` already
   prescribed the `Fmul`/`Ffma` chain, so the contract was right.

   **Which column goes with which component, because the disassembly reads the
   opposite way round.** The listing is `mul t1, u0, t0.wwww` and so on, which
   looks like "column 0 against `v.w`". It is not. The constant-file map that
   ships with it, `CD_4` in `vivante_blobs.rs`, is `12,13,14,15 / 8,9,10,11 /
   4,5,6,7 / 0,1,2,3` -- constant register 0 holds block dwords 12..15 and IS
   column 3. Every term pairs **column i with `v[i]`**; what runs backwards is
   the offline compiler's constant-REGISTER numbering, not the algebra.

   This paragraph said the wrong thing until es100-frontend checked it against
   `CD_4`, and the selection test built an IR computing `M * reverse(v)`. That
   draws a correct picture for every identity or symmetric matrix, which is
   most bring-up shaders, so it would have reached silicon looking fine. The
   pairing is now stated in block dwords, where it cannot be read two ways,
   and the check that would have caught it is item 4: the whole chain against
   the shipped blob, not a hand-built IR.
4. **The generated code against the table's blob**, decoded with M1's decoder
   and diffed **structurally** -- instruction count per class, register
   footprint, emit points. Byte equality is not required and is not the goal.
