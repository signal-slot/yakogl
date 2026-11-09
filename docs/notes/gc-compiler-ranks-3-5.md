<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000 compiler: the design of ranks 3 to 5 and of tilt_v's extra instructions

Plan #328, item pe-wire (issues #1204, #1106; ROADMAP.md stage 2, "Slow
scenes"). Host only: no board, no Mesa build, no code change. This note
designs the three GC7000 compiler changes that `gc-terrain-shaders-rest.md`
ranks 3 to 5 (`mad` fusion, float constant folding, the blur's per-tap
offsets) and explains why `tilt_v` keeps 9 instructions more than `tilt_h`.
Rank 1 (unroll) and the sr patch (copy fold, scalar lane merge) are item
x2-tiled-probe-run's and are not designed again here.

Code is cited at the integration tip `ee229223`. Every count below was
reproduced there; `origin/main` `6d360a03` (the tip when this clone was made)
has the same `src/shaderc/` and `tests/gc_terrain_shader_count.rs`
(`git diff --stat 6d360a03 ee229223 -- src/shaderc tests/gc_terrain_shader_count.rs`
prints nothing), so the counts are origin/main's too.

## Answer in short

- **`select.rs` is GC-only.** `src/shaderc/online/mod.rs:129-130` compiles
  it under `gpu-vivante` alone, and its one caller is `codegen.rs:386`
  (`assemble_from_ir_blocks`, also `gpu-vivante`). A change in `select.rs`
  cannot move a PowerVR or V3D word. Earlier notes count it with the
  "shared files" because it sits outside `vivante/`. The shared files are
  `lower.rs`, `ir.rs`, `fold.rs` and `loop_count.rs`. All four designs below
  change only `select.rs`. They read `fold.rs` and the IR and change neither.
- **Rank 3 needs no hand-off from `codegen.rs`.** `select_blocks` holds the
  module (`select.rs:3999-4004`), so it can read `ir::IoDecl::invariant`
  (`ir.rs:601-610`) itself. Contraction is safe when no output of the
  shader is `invariant`. `gc-terrain-shaders.md` "What to do" assumed the
  pass would live in `vivante/` and need `codegen.rs` to pass the flag on.
- **tilt_v's 9 extra instructions come from a retarget that is refused.** In
  `vec2(0.0, k * stepY)` the `mov t.x, 0.0` of the constant lane is emitted
  between the `mul` and the store of its lane. `retarget_last_to`
  (`select.rs:781-788`) only retargets the last instruction, so each tap
  pays one `mov t.y, tmp.y`. `bloom_v`'s 5 extra over `bloom_h` have the same
  cause. This is a new item ("rank T" below). It removes 9 in tilt_v and 5
  in bloom_v (2.8 ms). The sr patch does not remove them (71 against 62).
- **Rank 5 as ranked (pack the offset `mul`s four to a vec4) removes
  nothing on our selection.** Our per-tap `mul` already writes the vec2's
  lane through the retarget, where etnaviv spends a `mov` per tap to take
  its lane out of the packed vec4 (its 17 `mov`s). The win at the same spot
  is a different rewrite: `coord + vec2(k * s, 0.0)` becomes one `mad`
  (`mad t, s.xxxx, (k, 0), coord`). That removes 2 instructions a tap in
  tilt_h, 3 in tilt_v and bloom_v (it absorbs rank T there) and 2 in bloom_h.
  This is "rank 5'" below. The micro-shader t5 is corrected from "1 `mul`"
  to 4 (8 today).
- The micro-shader targets, by hand from the listings: **t3 3 -> 2** (rank
  3, confirmed); **t4 9 -> 4** with rank 4 alone, **-> 2** with rank 4 and
  the sr patch (confirmed: the note's 2 needs both); **t5 8 -> 4** with
  rank 5' (corrected); new **t5v 10 -> 8** with rank T, **-> 4** with rank 5'.
- At the model's cost (0.288 ms an instruction in an 800x600 pass, 0.039 in
  a 256x256 pass, 0.236 in terrain's vertex stage), on today's listings:
  rank 5' **14.6 ms** alone, or 11.8 ms after rank T; rank 3 **6.3 to
  6.5 ms**; rank 4 **5.6 ms**; rank T **2.8 ms**. The ranked list for the
  planner is at the end.

## The commands

The shader texts come from `gc-terrain-shaders-rest.md`'s appendix. The
generator is `gen_terrain.py` with its `noise.frag` line dropped, and the
four micro-shaders are added as files: t3, t4 and t5 are the texts of
`tests/gc_terrain_shader_count.rs:132-144`, and t5v is t5 with the two
lanes of each `vec2` swapped, which isolates tilt_v's shape. The extra
lines, appended to the generator:

```python
HEAD = "precision mediump float;\nvarying vec4 a;\nvarying vec4 b;\nuniform sampler2D T;\n"
wr("t3.frag", HEAD + "void main() { gl_FragColor = texture2D(T, a.xy) * 0.25 + b; }\n")
wr("t4.frag", HEAD + "const float S = 0.5;\nvoid main() { gl_FragColor = vec4(abs(S - a.y) / abs(1.0 - S)); }\n")
wr("t5.frag", HEAD + "void main() { gl_FragColor = vec4(a.xy + vec2(-1.0 * b.x, 0.0), a.xy + vec2(1.0 * b.x, 0.0)); }\n")
wr("t5v.frag", HEAD + "void main() { gl_FragColor = vec4(a.xy + vec2(0.0, -1.0 * b.x), a.xy + vec2(0.0, 1.0 * b.x)); }\n")
```

The glmark2 checkout was the one in this machine's dependency cache
(`$CONNECTED_DEPS_DIR/glmark2-src`). Our listings come from that note's
scratch test `tests/zz_scratch_count.rs`, unchanged. It is not committed;
it writes `build/terrain-lst/<file>.lst`.

```sh
python3 -I gen_terrain.py $GLMARK2_SRC build/terrain-shaders
mkdir -p build/terrain-lst
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
    --test zz_scratch_count -- --nocapture
```

What it printed (2026-11-09, at `ee229223`). The glmark2 lines equal the
note's of 2026-11-07, and the micro-shaders equal the pins of
`tests/gc_terrain_shader_count.rs`:

```text
COUNT bloom_h.frag instr=42 temps=8 const_dw=16 ops={"add": 12, "mov": 12, "mul": 12, "rcp": 1, "texld": 5}
COUNT bloom_v.frag instr=47 temps=8 const_dw=16 ops={"add": 12, "mov": 17, "mul": 12, "rcp": 1, "texld": 5}
COUNT luminance.frag instr=7 temps=4 const_dw=4 ops={"dp3": 1, "mov": 5, "texld": 1}
COUNT normalmap.frag instr=25 temps=5 const_dw=4 ops={"add": 5, "dp3": 1, "mov": 9, "mul": 4, "rcp": 2, "rsq": 1, "texld": 3}
COUNT overlay.frag instr=2 temps=2 const_dw=4 ops={"mul": 1, "texld": 1}
COUNT quad.vert instr=7 temps=3 const_dw=4 ops={"add": 2, "mov": 3, "mul": 2}
COUNT t3.frag instr=3 temps=3 const_dw=0 ops={"add": 1, "mul": 1, "texld": 1}
COUNT t4.frag instr=9 temps=3 const_dw=0 ops={"add": 2, "mov": 5, "mul": 1, "rcp": 1}
COUNT t5.frag instr=8 temps=5 const_dw=0 ops={"add": 2, "mov": 4, "mul": 2}
COUNT t5v.frag instr=10 temps=5 const_dw=0 ops={"add": 2, "mov": 6, "mul": 2}
COUNT terrain.frag instr=148 temps=19 const_dw=56 ops={"add": 20, "branch": 6, "cmp": 13, "dp3": 8, "exp": 1, "imullo0": 5, "load": 5, "log": 1, "mad": 6, "mov": 33, "mul": 24, "rcp": 1, "rsq": 5, "select": 14, "sqrt": 1, "texld": 5}
COUNT terrain.vert instr=56 temps=9 const_dw=52 ops={"add": 3, "dp3": 3, "mad": 16, "mov": 17, "mul": 12, "rsq": 3, "texldl": 2}
COUNT tilt_h.frag instr=66 temps=12 const_dw=24 ops={"add": 20, "mov": 16, "mul": 20, "rcp": 1, "texld": 9}
COUNT tilt_v.frag instr=75 temps=12 const_dw=24 ops={"add": 20, "mov": 25, "mul": 20, "rcp": 1, "texld": 9}
```

The listings below are cut from the `.lst` files: the instruction number,
the opcode and the operands (`cut -c1-4,42-`). In the blur shaders `u5`
(tilt) and `u3` (bloom) hold `TextureStepX/Y`, a literal that is not an f20
immediate and so goes to the constant file. `u0` to `u4` hold the kernel
weights. The "after" listings are written by hand: no code was changed to
produce them.

## What every design shares: where it goes and how it stays GC-only

The selection runs `nodes` and then the passes `propagate_copies`,
`fold_constant_reads`, `legalise_operands` and `eliminate_dead`
(`select.rs:4056-4060`). After that, `codegen.rs:420-433` runs `alloc`'s
`eliminate_dead`, `sink_definitions` and `coalesce_copies`. Every design
below puts its code in `select.rs`, either at selection time (`src`,
`alu`, `retarget_last_to`) or as one more pass in that list.

- **PowerVR and V3D.** They do not compile `select.rs`
  (`mod.rs:129-130`). `fold::fold_float` (`fold.rs:24`) is called, not
  changed. `lower.rs` (the front end: `mat_times_column` `lower.rs:6115`,
  `Lower::reciprocal`, the constructors) is not touched. So
  `src/pvr/internal.rs`, `tests/reference/*.rs` and the `pvr`/`v3d` lines
  of `tests/fixtures/precompile_counts.txt` cannot move. The check is
  `tools/check.sh --changed origin/main`: it rebuilds those generated files
  and compares the bytes.
- **What moves on the GC side.** Every design shortens GC programs, so each
  item re-pins in its own commit:
  - the six `gc7000` lines of `tests/fixtures/precompile_counts.txt`
    (`PRECOMPILE_PIN_WRITE=1 tools/precompile/prove.sh gpu-vivante`, and
    again with `gpu-vivante,report-es2`);
  - the pins in `tests/gc_terrain_shader_count.rs` and
    `tests/gc_sr_shader_count.rs`;
  - possibly the population pin of `tests/lane_move_equivalence.rs` (103,
    or 87 with `report-es2`);
  - possibly `src/vivante/internal.rs`. The mipmap program is 4 instructions
    against etnaviv's 2 (`gc-shadow-refract-shaders.md`), so ranks 3 and 4
    may shorten it. It is regenerated by `yakogl-precompile --internal`,
    and `tests/internal_programs.rs` then decides: a program held word for
    word to etnaviv's must still match.
- **Correctness check for each item.** Run the programs of the offline
  table that it shortens on the host interpreter (`tests/common`
  `run_mem`) before and after, as noise-compiler-fix did for its negate
  fold (`gc-terrain-shaders.md`, "Negate fold"), and require bit-identical
  results. The exception is rank 3, whose rounding differs by design: there
  the check is a tolerance of one ulp of the product, plus the `invariant`
  test below.

## Rank 3: `mul` + `add` -> `mad`

**(a) What it removes.** A `mul` whose result has one reader, an `add`,
becomes the `mad` that adds there. tilt_h, instructions 048 to 063 (the
same in tilt_v at 057 to 072):

```text
before                                   after
048 mul t2, t2.xyzw, u4.xxxx             mad t2, t2.xyzw, u4.xxxx, t4.xyzw
049 add t2, t2.xyzw, void, t4.xyzw
050 mul t3, t5.xyzw, u2.xxxx             mad t2, t5.xyzw, u2.xxxx, t2.xyzw
051 add t2, t2.xyzw, void, t3.xyzw
... (052/053 to 062/063, the same pair)  ... (one mad each)
```

That is 8 pairs in each tilt pass (the first tap's `mul` 045 stays: it
starts the sum), and 4 in each bloom pass (bloom_h 032/033 to 038/039).
The others:

- t3: `texld; mul t1, t1, 0.25; add t1, t1, t2` becomes
  `texld; mad t1, t1, 0.25:f20, t2`.
- terrain.vert: 036/037 (`mul t2.w, u12.x, h; add t2.w, u12.y`) and 046/047
  (`x * 2.0 - 1.0`, `mad t3.xyz, t2, 2.0:f20, -1.0:f20`). Two immediates
  in one instruction are allowed: `group_cap(RGROUP_IMMED)` is 2
  (`select.rs:4385`).
- quad.vert: 000/001 and 002/003.
- terrain.frag: 002/003 (`uv * scale + offset`), 005/006 (`* 2.0 - 1.0`),
  130/131, 143/144, and the loop body's pairs, which rank 1 straight-lines.
  The note's figure of 4 is a lower bound here.

**(b) Where.** A new pass `Sel::fuse_mul_add` in `select.rs`, run after
`propagate_copies` and before `fold_constant_reads`, so that the fold sees
the `mad`'s constant operands (`select.rs:4057-4058`). The rules, all on
`SelInstr`, mirror `powervr/prepare.rs:996` `fuse_mul_add`:

- the `mul` is F32, not saturated, with no condition, and writes a
  `SelDst::Virtual` written once;
- the only reader of that register is an F32 `add` with no `saturate`, at
  most one of whose sources reads it;
- every lane the `add` reads through its swizzle was written by the `mul`;
  the swizzle is composed into the `mul`'s sources
  (`compose`, `select.rs:4490`, only when `op.computes_lanewise()`);
- a `neg` on the `add`'s read of the product moves onto the `mul`'s first
  source (`-(a*b) = (-a)*b`, exact); an `abs` there refuses the pair;
- nothing between the two writes either `mul` source, and no branch or
  branch target lies between them (straight-line only, as the negate fold);
- `within_caps` (`select.rs:4398`) holds for the three sources (at most one
  constant-file register, two immediates), and `mad` takes every source's
  group (`build.rs:336` `takes_group`). If not, the pair is left alone.
  Folding a second uniform into a `mov` would cost what it saves.
- **the `invariant` guard:** no pair is fused in a shader one of whose
  `m.outputs` has `invariant` set. The flag is read in `select_blocks`
  (`select.rs:3999`), no `codegen.rs` change is needed, and a
  `#pragma STDGL invariant(all)` sets it on every output (`ir.rs:601-610`).
  This is the conservative form of PowerVR's per-output rule
  (`powervr/prepare.rs:1103` `fusion_pairs`). Matrix products are already
  `Ffma`s from the front end (`lower.rs:6140`) and stay as they are.

Whether the GC7000's `mad` rounds once or twice is not known
(`gc-terrain-shaders.md`). The guard makes the question matter only for
invariance, which it already keeps.

**(c) GC-only:** yes, `select.rs` only.

**(d) Targets:** **t3 3 -> 2** (confirmed). tilt_h 66 -> 58, tilt_v
75 -> 67, bloom 42 -> 38 and 47 -> 43, terrain.vert 56 -> 54, quad.vert
7 -> 5, terrain.frag 148 -> 144 or lower.

**(e) Tests.**
- Change the pin of `tests/gc_terrain_shader_count.rs` t3 from 3 to 2.
- A unit test in `select.rs` tests: `gl_FragColor = a * b + c;` selects one
  `mad`.
- A unit test: the same expression written to an `invariant varying` (and
  under `#pragma STDGL invariant(all)`) keeps `mul` + `add`.
- A unit test: a product read twice (`p = a*b; x = p + c; y = p * d`) is
  not fused.
- A unit test: `a*b + c` where `a` and `c` are two different uniforms
  (`within_caps`) is not fused.

**(f) Order and fixtures.** After the sr patch: its negate fold turns
`mul; mov -t; mad` (terrain.vert 026 to 028, the cross product) and
`mov -x; add` into forms with no `mov` between the `mul` and its reader, so
more pairs qualify. After rank 4, so that folded constants arrive as
immediates. The fixtures are those of the shared list.

**Cost:** tilt 16 x 0.288 = 4.6 ms, bloom 8 x 0.039 = 0.3 ms, terrain.vert
2 x 0.236 = 0.5 ms, terrain.frag 4 x 0.288 x 0.8 to 1.0 = 0.9 to 1.2 ms.
Total **6.3 to 6.5 ms** (the ranking's 6.6).

## Rank 4: float constant folding in the GC selection

**(a) What it removes.** Three kinds of instruction.

**4a, all operands constant.** tilt_h 000 to 004 compute `abs(1.0 - TiltShift)`
and its reciprocal from `const float TiltShift = 0.5`:

```text
before                                       after
000 mov t2.x___, void, void, 0.5:f20         (none: rcp(|1.0 - 0.5|) is 2.0)
001 mov t2._y__, void, void, -t2.xxzw
002 add t2._y__, 1.0:f20, void, t2.yyzw
003 mov t2._y__, void, void, |t2.yyyy|
004 rcp t2._y__, void, void, t2.yyyy
007 add t1.x___, t2.xyzw, void, t1.xyzw      add t1.x___, 0.5:f20, void, t1.xyzw
010 mul t1.x___, t1.xyzw, t2.yyzw, void      mul t1.x___, t1.xyzw, 2.0:f20, void
```

That removes 5 in each tilt pass: tilt_v 001 to 005, where 000 is a tap's
`mov 0.0`. The bloom passes remove 5 each (bloom_h 000 to 004, bloom_v 001
to 005): `TiltShift = 0.0`, `rcp(|1.0 - 0.0|) = 1.0`. t4 removes 5
(000 to 004, the same shape).

**4b, identities with one constant operand,** at selection time:

- `x * 1.0` becomes `x`. In bloom, 010 `mul t1.x, t1, rcp(1.0)`, after 4a
  (1 each).
- `0.0 + x` and `x + 0.0` become `x`. This is the sum's tail `add t1, t1,
  0.0` (tilt_h 064, tilt_v 073, bloom_h 040, bloom_v 045: 1 each). In
  bloom, `TiltShift - y` with `TiltShift = 0.0` (bloom_h 007, bloom_v 008:
  1 each) becomes a negated source.
- `(x * c1) * c2` with `c2` a power of two becomes `x * (c1 * c2)`. This is
  exact barring overflow or a denormal, which `fold_float` already refuses.
  In tilt, 009 and 010 (`TextureStep * |..| * 2.0`) become one `mul` with
  `u5 * 2.0` as a new constant (1 each). That is etnaviv's `mul u0, |.|`.
- `x * 0.0` becomes `0.0`, and `x * -1.0` becomes `-x`. These remove
  nothing on their own here: each tap's lane still needs its write. They
  matter for rank 5' below.

The `+ 0.0` rule changes `-0.0 + 0.0` (`+0.0`) into `-0.0`. GLSL ES does
not require signed zeros to be kept, and Mesa folds it the same way: NIR's
`fadd(a, 0.0) -> a` under no-signed-zero, which GLSL compiles with. The
item that lands this states the rule in a comment, as `fold.rs` does for
its own exceptions.

**4c, one constant lane of a constructed vector.** terrain.vert's four
`m * vec4(v, 1.0)` products:

```text
before                                       after (4c)                      after (4c + sr patch / rank 2)
000 mov t4.xyz_, void, void, t0.xyzx         000 mov t4.xyz_, ..., t0.xyzx   (none: mads read t0)
001 mov t4.___w, void, void, 1.0:f20         (none)
002 mul t0, u3.xyzw, t4.wwww, void           mov t0, void, void, u3.xyzw     mov t0, void, void, u3.xyzw
003 mad t0, u2.xyzw, t4.zzzz, t0.xyzw        mad (unchanged)                 mad t0, u2, t0'.zzzz, t0
...
```

`fold_constant_reads` (`select.rs:3548`) folds a constant seed only when
its register is written once (`select.rs:3617-3623`). Here `t4` is
written twice (`xyz` and `w`), so `t4.wwww` never reads as `1.0`. With 4c
the fold accepts a register written more than once, provided every reader
it rewrites reads only lanes inside the seed's write mask (`t4.wwww`
inside `.___w`) and no other write of those lanes lies between them.
Then `mul u3, 1.0` becomes `mov u3` by 4b. That removes 1 per product with
a `1.0` lane: terrain.vert 001, 008, 017 and 048, so 4. The `mov xyz` copy
is rank 2's (partial-mask copy propagation), and the sr patch's 56 -> 48
probably includes it; this note did not apply the patch. The same rule
reaches shadow's ground vertex shader `vec4(position, 0.0, 1.0)` and the
horse's `vec4(normal, 1.0)` (`gc-shadow-refract-shaders.md`, rank 3 of its
table: "the constructor's lanes visible to readers").

**(b) Where.**
- 4a: a table `const_of: &mut [Option<[u32; 4]>]`, one entry per IR value,
  filled in one forward walk over `m.instrs` before `s.nodes`
  (`select.rs:4056`). `ConstF32` gives its bits. `Alu(op, srcs)` whose
  sources are all in the table gives `fold::fold_float(op, x, y, z)` lane
  by lane, after each source's swizzle; one `None` lane leaves the value
  unfolded. `Sel::src` (`select.rs:1676`) then treats a folded value as it
  treats `ConstF32` (`select.rs:1699-1719`: an immediate when the lanes
  agree and `imm_f32` takes it, the constant file otherwise), and
  `Sel::alu` (`select.rs:1857`) emits nothing for a folded value. The
  comment at `select.rs:4037-4040` ("the folded definitions never get one")
  already allows the gaps in numbering this leaves. Integer and boolean
  operations are left as they are: the PowerVR rules for them
  (`powervr/select.rs:3993-4058`) are a separate item if ever needed.
- 4b: in `Sel::alu` before `builtins::native`, a match on `(op, constant
  operand)` that records the value as an alias of its other operand (with
  `neg` for `* -1.0`) instead of emitting. This uses the existing
  `value_alias` (`select.rs:4013`, read by `src` at `select.rs:1764-1769`).
  The alias must not be taken when the operand is a local slot whose
  register a later store changes: use the same check as `src`'s
  `local_is_stable_after`.
- 4c: in `fold_constant_reads`, replace the single-write test
  (`select.rs:3617-3623`) with a per-lane one, as described above.

**(c) GC-only:** yes. `fold.rs` is only called. Calling `fold_float` from
`lower.rs` instead (`lower.rs` `alu_sized`, the position
`gc-shadow-refract-shaders.md` also names) would fold for all three back
ends and move PowerVR and V3D fixtures. Do not.

**(d) Targets:**
- **t4 9 -> 4** with 4a. Left are `mov -a.y; add 0.5; mov |.|; mul 2.0`.
  **t4 -> 2** with 4a and the sr patch's negate fold (`add t1.x, 0.5,
  -t1.y; mul t1, |t1.x|, 2.0`). This confirms the note's 2, which needs
  both.
- tilt_h 66 -> 59 and tilt_v 75 -> 68 (4a 5, 4b 2 each).
- bloom_h 42 -> 34 and bloom_v 47 -> 39 (4a 5, 4b 3 each).
- terrain.vert 56 -> 52 (4c).

**(e) Tests.**
- Change the pin of `tests/gc_terrain_shader_count.rs` t4 to 4 (2 once the
  sr patch is in).
- Add a micro-shader t4b `gl_FragColor = vec4(a.xyz, 1.0) * 1.0 + 0.0;`
  (4b: the `mul` and the `add` go).
- Add a micro-shader t4c `gl_Position = m * vec4(p, 1.0);` in the vertex
  stage (4c: one `mov` of the column and three `mad`s, 4 against 6 today
  with the copy).
- Unit tests in `select.rs`: a constant whose fold `fold_float` refuses
  (`exp2(0.5)`, a denormal) is still computed; `x * -1.0` reads `-x`; a
  value aliased through a slot that is stored again later is not aliased.
- The interpreter check of the shared list (folding must be bit-exact).

**(f) Order and fixtures.** First of the three: it shrinks what ranks 3 and
5' see (rank 3's pair `x * 2.0 - 1.0` keeps both immediates either way).
It is independent of rank 1 and of the sr patch. In terrain.frag the
accumulators `vec3(0.0)` (033, 034, 135 to 140) become `0.0 + x` only once
rank 1 has straight-lined the loop; land rank 1 first if terrain.frag's
share is wanted, about 4 more (1.0 ms). The fixtures are those of the
shared list.

**Cost:** tilt 14 x 0.288 = 4.0 ms, bloom 16 x 0.039 = 0.6 ms,
terrain.vert 4 x 0.236 = 0.9 ms. **5.6 ms**, or about 6.6 with terrain.frag
after rank 1 (the ranking said 6.4).

## tilt_v's 9 more: rank T, the retarget past a constant lane

**What makes them.** tilt_h builds each tap's offset with two instructions,
tilt_v with three:

```text
tilt_h (horizontal: vec2(k * s, 0.0))         tilt_v (vertical: vec2(0.0, k * s))
011 mul t2.x___, -4.0:f20, t1.xxxx            012 mul t1._y__, -4.0:f20, t1.xxzw
012 mov t2._y__, void, void, 0.0:f20          013 mov t2._y__, void, void, t1.yyyy
                                              014 mov t3.x___, void, void, 0.0:f20
```

The IR for `vec2(0.0, k * s)` is the `Fmul`, then a `StoreLocal` of `0.0`
to the slot's `x`, then a `StoreLocal` of the product to its `y`. When the
second store is selected (`select.rs:2576-2601`), it asks
`retarget_last_to` (`select.rs:722`) whether the `mul` can write the
slot's lane `y` itself. The `mul` is no longer the last instruction: the
first store's `mov t.x, 0.0` came after it. So the check at
`select.rs:781-788` refuses (`retarget_not_last`) and a `mov` is emitted.
In tilt_h the product is stored first, while it is still the last
instruction, and is retargeted. Per tap that is one `mov`: 9 taps make the
9, and bloom_v's 5 taps make its 5 (47 against 42). The listings show it
directly: tilt_v has 25 `mov`s against 16, the same 20 `mul` and 20 `add`.
`alloc::sink_definitions` later moves the `mul` down beside its copy, which
is why the listing shows it between the two `mov`s.

The sr patch's copy fold does not take these `mov`s: each moves a lane to
a different lane (`t1.y` into `t2.y` with a different register's `x`
already written). So the 9 survive it (71 against 62).

**Where.** In `retarget_last_to`, accept a producer that is not the last
instruction when every instruction after it:
- neither reads nor writes the producer's register `v`;
- writes no register that the producer reads;
- writes none of the lanes of `dst` that the retarget will write (the
  `mov t.x, 0.0` writes lane `x`, the retarget lane `y`);
- is not a branch or a branch target;
- and the producer itself has no `tex` and no `target` (as today).

The search stops at the first instruction that breaks one of these. Only
the last two or three instructions need looking at; a bound of 4 keeps
the cost linear. The swizzle composition (`select.rs:793-813`) is
unchanged.

**GC-only:** yes, `select.rs`.

**Targets.** **t5v 10 -> 8** (`mov t3.x 0.0; mul t3.y; mov t4.x 0.0;
mul t4.y; add; add; mov; mov`). tilt_v 75 -> 66 (71 -> 62 with the sr
patch), bloom_v 47 -> 42.

**Tests.**
- Add t5v to `micro()` in `tests/gc_terrain_shader_count.rs` with pin 10
  today, 8 after.
- A unit test in `select.rs`: `vec2(0.0, a.x * b.x)` stored and read is
  `mov` + `mul`, with no lane copy.
- A unit test: when the instruction between the two writes is a write of
  a register the producer reads (`t = a * b; a = 0.0; v = vec2(0.0, t)`
  through slots), there is no retarget.

**Order and fixtures.** Independent of every other item. It is the
smallest change of this note (one function), and is worth landing first.
Rank 5' below would absorb it for the blur shaders, but it also removes
copies in any constructor whose constant lane comes first. The fixtures
are those of the shared list.

**Cost:** tilt_v 9 x 0.288 = 2.6 ms, bloom_v 5 x 0.039 = 0.2 ms.
**2.8 ms.**

## Rank 5: the per-tap offsets

**What the ranking assumed, and why it removes nothing here.** The ranking
read etnaviv's two vec4 `mul`s for the eight non-zero taps as the saving
over our nine scalar `mul`s. On our listing, each of those `mul`s is
retargeted into the tap's vec2 slot (tilt_h 011 `mul t2.x___`, the slot
read whole by 046 `add t2, t3, t2`). Etnaviv, after its packed `mul`,
spends one `mov` per tap to put that lane into the vec2: its 17 `mov`s are
9 `mov y, 0.0` and 8 lane `mov`s. Packing our `mul`s would replace each by
such a `mov`, a saving of 0. For t5 the same: packing the two `mul`s into
`mul t.xy, (-1, 1), b.xx` still needs both lanes moved into the two vec2s,
so 8 -> 8. The ranking's "t5 to 1 `mul`" is not reachable by packing.

**Rank 5': the tap coordinate as one `mad`.** `coord + vec2(k * s, 0.0)` is
`fma(vec2(k, 0.0), s, coord)`: lane `x` is `k * s + coord.x`, and lane `y`
is `0.0 * s + coord.y`. The two agree when `s` is finite, which GLSL ES
leaves to the implementation as Mesa does, and up to the rounding rank 3
already allows. tilt_h, one tap:

```text
before                                       after
011 mul t2.x___, -4.0:f20, t1.xxxx           mad t2, t1.xxxx, u6.xyyy, t3.xyzw   (u6 = (-4.0, 0.0))
012 mov t2._y__, void, void, 0.0:f20
046 add t2, t3.xyzw, void, t2.xyzw
047 texld t2, tex0.xyzw, t2.xyzw             texld t2, tex0.xyzw, t2.xyzw
```

When `k` is the same in both lanes it needs no constant register:
`(k, 0)` against `(0, k)`. When it is not, `(k, 0)` takes two dwords of
the constant file, and taps `±k` share one `vec4` (`k, 0, -k, 0`), so 4
dwords a pair. The pass reads that through the swizzle. Tap 0 becomes
`coord` itself (4b's `0 * s`), so the `texld` reads `t3` directly.

Removed per tap:
- tilt_h, 2 (the `mul` and the `mov`, 3 at tap 0): 19.
- tilt_v, 3 (`mov x 0.0`, `mul`, the lane `mov`, 4 at tap 0): 28.
- bloom_h: 11. bloom_v: 16.

tilt_h and tilt_v both end at **47**, so rank 5' also removes tilt_v's 9.

**Where.** The pattern lives in the IR, not in `SelInstr`. When `Sel::alu`
selects `Fadd(c, l)` (or `Fadd(l, c)`) and `l` is a `LoadLocal` of a slot
that is not under control flow, look at its stores
(`masked_stores_share_one_value`, `select.rs:1240`, is the existing walker
of a slot's masked stores). If every lane holds a constant or `Fmul(constant,
s)` with one `s` for all lanes (constants count as `k = 0`), emit `mad dst,
s.xxxx, const(k0..k3), c`. The slot's stores then have no reader and are
dropped by `eliminate_dead` (`select.rs:3367`). The same `invariant` guard
as rank 3. It needs `const_reg` (`select.rs:1576`) for the `k` vector.

**GC-only:** yes, `select.rs`.

**Targets.** **t5 8 -> 4**: two `mad`s, then the `vec4(vec2, vec2)`
assembly `mov t2.zw` and the output copy, which are rank 2's. That
corrects the note's "1 `mul`". t5 -> 1 (one `mad t1, b.xxxx, (-1, 0, 1, 0),
a.xyxy`) would also need the two `mad`s packed into one vec4. glmark2's blur
gains nothing from that, because every tap feeds its own `texld`, so it is
not proposed. **t5v 10 -> 4.**

**Tests.**
- Change the pins of t5 to 4 and of t5v to 4.
- A unit test: `a.xy + vec2(2.0 * b.x, 0.0)` is one `mad` reading
  `(2.0, 0.0)`.
- A unit test: with `invariant` on the output it is not.
- A unit test: `vec2(2.0 * b.x, 3.0 * c.x)` (two different `s`) keeps the
  `mul`s.
- The interpreter check, with a tolerance for the contraction.

**Order and fixtures.** After rank 3 (the same guard and the same rounding
question) and after rank 4 (the zero tap). With rank T landed first,
rank 5' removes 9 fewer in tilt_v and 5 fewer in bloom_v than counted
above. The fixtures are those of the shared list.

**Cost:** rank 5' alone is tilt (19 + 28) x 0.288 = 13.5 ms and bloom
(11 + 16) x 0.039 = 1.1 ms, so **14.6 ms**. Rank 3 does not overlap it:
rank 3 touches the sum's pairs only, and rank 5' touches the offsets. Rank
T does overlap it, because both remove tilt_v's 9 and bloom_v's 5. After
rank T, rank 5' adds (47 - 9) x 0.288 = 10.9 ms and (27 - 5) x 0.039 =
0.9 ms, so **11.8 ms**. The list below counts T apart.

The tilt passes then go to about 47 - 8 (rank 3) - 7 (rank 4) = **32
instructions**, below etnaviv's 46. Nine of them are `texld`s, so the
texture unit's rate, which the model does not price, may cap the gain
(`gc-terrain-shaders-rest.md`, "How sure each is").

## Ranked implementation items for the planner

Each is one commit with its own re-pins (the shared list above),
`tools/check.sh --changed origin/main` against the tip, and no file
outside `select.rs` and the tests (and the regenerated
`precompile_counts.txt` / `internal.rs` if they move). ms a frame at the
model's 0.6 ns (x 1.33 for its 0.8 ns case), terrain only, counted on
today's listings without the sr patch or rank 1.

| # | title | files | test | ms a frame |
|---:|---|---|---|---:|
| 1 | vivante compiler: tap offsets as one `mad` per tap (rank 5') | `select.rs` (`Sel::alu`, slot walker, `const_reg`) | t5 8 -> 4, t5v 10 -> 4; unit tests: one `mad`, invariant kept, two `s` refused | 11.8 after T (14.6 alone) |
| 2 | vivante compiler: `mul` + `add` -> `mad`, none toward an `invariant` output (rank 3) | `select.rs` (`fuse_mul_add` between `propagate_copies` and `fold_constant_reads`) | t3 3 -> 2; invariant, read-twice and two-uniform refusals | 6.3 to 6.5 |
| 3 | vivante compiler: float constant folding and one-operand identities in the selection (rank 4a, 4b) | `select.rs` (`const_of` table, `src`, `alu`) | t4 9 -> 4 (2 with the sr patch); t4b; refused folds still computed | 4.6 (5.6 after rank 1) |
| 4 | vivante compiler: retarget a product past a constant-lane store (rank T) | `select.rs` (`retarget_last_to`) | t5v 10 -> 8; unit tests both ways | 2.8 |
| 5 | vivante compiler: a constant lane of a constructed vector read as the constant (rank 4c) | `select.rs` (`fold_constant_reads`) | t4c 6 -> 4 (vertex) | 0.9 (+ shadow's ground and horse vertex) |

The order to land them, smallest risk first: **4 (T), 3 (4a/4b), 5 (4c),
2 (rank 3), 1 (rank 5')**. Item 1 has the largest win but depends on 2's
guard and 3's zero rule. T is listed fourth by size and lands first
because it is one function with no rounding question. All of them
come after or beside x2-tiled-probe-run (sr patch, rank 1). None changes
the same lines as the sr patch (`vivante/alloc.rs`). Rank 1 touches
`select.rs` `loop_convert` (`select.rs:2943`), away from all five. The
expected total, without overlap with the sr patch's 10.6 ms and rank 1's
15.6 ms: **about 26 ms a frame** (35 at 0.8 ns). The tilt-shift passes'
texture rate is the open limit.

## Not done

- No item's code was written, so the "after" listings are by hand. Each
  item re-measures with the commands above and pins its own numbers.
- Etnaviv's listings were not re-run (no Mesa build). The ETNA lines of
  `gc-terrain-shaders-rest.md` were used as they are.
- terrain.frag was read for ranks 3 and 4 only where its listing shows the
  pattern outside the loop. Its share after rank 1 needs rank 1's listing.
- shadow and refract: the same three causes are `gc-shadow-refract-shaders.md`
  C1 (folding, `mad`) and C3 (constructor lanes, item 5 above). Their
  shader counts were not re-listed here; the items' re-pins of
  `tests/gc_sr_shader_count.rs` give them.
- Whether the GC7000's `mad` rounds once is still unmeasured. A board case
  (`gl_test`, `mad` against `mul` + `add` on values whose product rounds)
  would settle whether rank 3 and 5' need the `invariant` guard at all.
