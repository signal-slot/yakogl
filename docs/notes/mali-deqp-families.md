<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 compile refusals by family and shape, measured on the host

**HOST-ONLY.** Plan #377 item `measure` (issue #20, ROADMAP stage 5, issue
#7). Every count in this note comes from `deqp-gles2` run on a host against
the Mali back end's C pair on the Linux OSAL, which keeps the command stream
instead of executing it: nothing is drawn, every `glReadPixels` reads black
and every dEQP render verdict of the run is meaningless. What is real, and
what this note counts, is each shader's compile and each program's link
outcome and our info log, the text of a `MaliError`
(`src/shaderc/online/mali/mod.rs`). A board status quoted here (Fail,
NotSupported) is the NanoPC-T6's own, from the slices, never the host's.

- Section 0 below is the refresh at the tip `0f0b1ec9`; the rest of the note
  keeps the earlier measure's counts.
- Tip of the earlier measure: `abcc9013` (plan #377's integration tip at the time of the run, a
  descendant of origin/main `1f051a7b` with the merged items of plans #373
  and #377; the compiler under `src/shaderc/online/mali/` as at that
  commit).
- CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (the commit
  `DEQP_WORK` pins, `tools/cts/deps.toml`) with `cts-yakogl-target.patch`
  applied uncommitted, its external sources fetched.
- Cases: the 10,597 cases of the four slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` whose `refusals.tsv`
  text holds `online shaderc: the code generator refused` (the board tree at
  `37305608`, docs/es2-cts-failures.md section M; the 14 Pass cases that log
  a refusal are among them, see section 2).
- Command (run on 2026-11-16, 4 cores, about 4 minutes for the run itself):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$TMPDIR/deqp-host-measure-abcc9013" --jobs 3 --chunk 100
```

The script (`tools/mali/deqp-host-measure.sh`, its header says the rest)
builds `deqp-gles2` for the host as step 1 of `tools/cts/host-deqp-run.sh`
does and the library from a generated CMake project, runs the case list in
100-case chunks with `--deqp-log-shader-sources=enable` on 256x256 pbuffers
(the board's `DEQP_SURFACE`; at 64x64 the 53 `shaders.invariance` cases end
NotSupported before compiling anything), and writes per case the first
refusal, its family, its shape and both shader sources (`measure.tsv`,
`src/<case>.vert|.frag`), per shape one representative pair (`shapes/`), and
`summary.md` with the tables below. The per-case list (2.6 MB) is not
committed: rerun the command to get it.

## 0. Refresh at the tip `0f0b1ec9` (HOST-ONLY), against `37305608`

Item `families-refresh-2`. **HOST-ONLY**: the whole of sections 1 to 18
below keeps the counts of the earlier measures (`abcc9013` and older, each
section names its own commit); this section is the one measure at the
tip `0f0b1ec9` (`git describe`: `0f0b1ec9`, clean). Same script, same 10,597
cases of the four 37305608 slices, CTS `93bca018` with
`cts-yakogl-target.patch`, 3 jobs, 100-case chunks:

```sh
tools/mali/deqp-host-measure.sh --cts <VK-GL-CTS at 93bca018 with the target patch> \
    --out <dir> --jobs 3 --chunk 100
```

Result: 10,597 cases began; 5,967 have a first compile refusal on the host
(any text, taken or not), 4,630 compile and link on the host (205 end Pass,
the rest fail or are NotSupported for a later reason: a link or a render
check, meaningless here); 106 process exits with code 1, none a crash row.
Section 14's index `tools/mali/testdata/deqp_shapes/index.tsv` now holds
these counts (`cases`, `fail`, `notsupported` by the BOARD's status, per
shape of the host's first refusal; a shape that no case reaches stays with
0; no row added or removed). Four shapes the tip produces are not rows of
the index and were left out as the item asks: loops `vs_for_constant` (31),
loops `vs_for_uniform` (37), loops `fs_kept_for` (16) and mixprec
`fs_highp_declared` (59) (they are the `#367` or newer names; see section
11).

### 0.1 Each taken refusal: the board at `37305608` against the tip

Board columns: cases whose `refusals.tsv` text at `37305608` contains the
text, by board status (this is the board's one refusal per case). Tip
columns: cases whose FIRST host refusal at `0f0b1ec9` contains the text, by
the board's status. A case counts once, under the refusal it meets first,
so a count can rise when a case that was refused earlier by a text now
lowered meets this one next ("moved in"). **GREW** marks a count that rose
(acceptance criterion c7); every one below is a first refusal that cases
reach now that a text before it in the shader is lowered, not a new
failure of a case that compiled before (see 0.2).

| family | refusal text | board F | board NS | tip F | tip NS | delta |
|---|---|--:|--:|--:|--:|--:|
| intval | an integer or boolean value | 3178 | 41 | 186 | 22 | -3011 |
| varylink | two varying lanes of equal value | 651 | 5 | 40 | 9 | -607 |
| unitype | a uniform other than a float, vec2, vec3, vec4 or mat4 | 362 | 94 | 314 | 148 | **+6 GREW** |
| unitype | a mediump fragment shader uniform other than | 285 | 0 | 268 | 1 | -16 |
| unitype | a vertex shader uniform that is not highp | 77 | 2 | 46 | 53 | **+20 GREW** |
| uniarray | a uniform array | 265 | 4 | 193 | 0 | -76 |
| uniarray | a uniform read at a run-time index | 2 | 1 | 0 | 8 | **+5 GREW** |
| constop | a constant operation on or to a value that is not a finite normal float | 603 | 6 | 167 | 73 | -369 |
| constop | a constant fp16 does not hold | 3 | 0 | 1 | 0 | -2 |
| colourpair | a colour pair of a constant then a varying lane | 142 | 0 | 61 | 0 | -81 |
| colourpair | a colour pair of a varying lane and 0.0 | 29 | 0 | 16 | 0 | -13 |
| vsloop | a loop in a vertex shader | 105 | 392 | 29 | 167 | -301 |
| fsloop | a loop Mesa keeps (a run-time bound) in a mediump fragment shader | 0 | 268 | 0 | 87 | -181 |
| fsloop | a break or continue behind a run-time condition | 0 | 6 | 0 | 10 | **+4 GREW** |
| fsloop | a loop past Mesa's unroll limits | 1 | 2 | 1 | 27 | **+25 GREW** |
| varyuniform | a varying lane computed from no attribute | 252 | 0 | 31 | 1 | -220 |
| varyuniform | a varying lane left unwritten | 3 | 0 | 0 | 0 | -3 |
| mixprec | a fragment shader that mixes highp and mediump or lowp | 192 | 0 | 158 | 2 | -32 |
| posunwritten | a gl_Position lane left unwritten | 155 | 0 | 6 | 0 | -149 |
| negzero | a -0.0 that a comparison reads through a variable | 117 | 0 | 137 | 0 | **+20 GREW** |
| invariant | an invariant output | 12 | 0 | 9 | 0 | -3 |
| misc | a program needing more than the split file's 32 work registers | 2 | 0 | 2 | 0 | 0 |
| misc | a shader of more than 2048 IR values | 3 | 0 | 2 | 0 | -1 |
| misc | a comparison other than | 3 | 0 | 0 | 0 | -3 |
| misc | a built-in input | 1 | 0 | 0 | 0 | -1 |
| misc | a texture look-up other than texture2D | 27 | 0 | 42 | 0 | **+15 GREW** |
| misc | a texture result's lanes | 2 | 0 | 1 | 0 | -1 |
| misc | a shape Mesa's algebraic rules rewrite without a reference | 6 | 0 | 0 | 0 | -6 |

The board columns count every line holding the text (all 10,597 cases),
so they exceed the issue's figures where one text appears in cases the
issue files under another family; the family of the first refusal is the
column to read in section 0.3.

### 0.2 What grew (acceptance criterion c7), and why

Eight texts have a higher count at the tip. All are "moved in": the
case's earlier refusal was lowered (intval, link rules, uniform arrays,
colour pairs) and the case now stops at the next one. Nothing here says a
case that compiled at `37305608` is refused now: `37305608` is a board
run whose refusal per case is the first one met, and the host measure
reports the same kind of first refusal. Not proven per case: the board
text of each moved-in case would have to be compared with its host text
(`measure.tsv` holds both, `board.tsv` the board's).

- unitype `a uniform other than a float, vec2, vec3, vec4 or mat4`: +6
  (314/148 against 362/94: the Fail count fell 48, the NS count rose 54).
- unitype `a vertex shader uniform that is not highp`: +20 (NotSupported
  2 to 53: the vertex loops that were refused first as `a loop in a vertex
  shader` hit it next).
- uniarray `a uniform read at a run-time index`: +5 (3 to 8, all NS).
- fsloop `a break or continue behind a run-time condition` +4 and
  `a loop past Mesa's unroll limits` +25 (NS 2 to 27): the mediump
  fragment loops that were refused as `a loop Mesa keeps` now stop here.
- negzero `a -0.0 that a comparison reads through a variable`: +20 (117
  to 137): random-shader cases that stopped at `an integer or boolean
  value` first.
- misc `a texture look-up other than texture2D`: +15 (27 to 42).

### 0.3 Cases per shape per family (HOST-ONLY, the shapes of `summary.md`)

Fail / NotSupported are the board's. `n` is the cases whose first host
refusal is in that family and shape.

| family | n | Fail | NS | shapes (n) |
|---|--:|--:|--:|---|
| intval | 208 | 186 | 22 | ivec_operator 72, random_mixed 41, int_uniform 37, bool_uniform 31, step_sign 21, bool_literal 2, bool_logic 2, scalar_conversion 2 |
| varylink | 87 | 77 | 10 | equal_lanes 34, lane_no_attribute 32, equal_lanes_broadcast 15, position_unwritten 6 |
| unitype | 830 | 628 | 202 | vs_int_uniform 346, vs_struct_uniform 116, fs_struct_uniform 76, fs_mat_uniform 67, vs_mediump_basic 55, fs_other 50, vs_mediump_struct 42, fs_bool_uniform 41, fs_int_uniform 35, vs_mediump_array 2 |
| uniarray | 201 | 193 | 8 | bool_array 90, int_array 47, array_in_struct 34, mat_array 21, runtime_index 8, struct_array 1 |
| constop | 387 | 305 | 73 | neg_zero_variable 137, zero_operand 126, random_mixed 60, const_other 40, const_call_transcendental 16, const_call_other 7, fp16_constant 1 |
| mixprec | 204 | 202 | 2 | fs_mixed_other 101, fs_highp_declared 59, fs_varying_other 44 |
| colourpair | 77 | 77 | 0 | constant_then_varying 61, varying_and_zero 16 |
| loops | 321 | 30 | 291 | vs_do_while 52, vs_while 52, fs_kept_for_index 45, vs_for_uniform 37, vs_for_constant 31, fs_past_limits 28, vs_for_index 18, fs_kept_for 16, fs_kept_while 16, fs_kept_do_while 10, fs_unrolled_break 10, vs_for_dynamic 6 |
| misc | 14 | 14 | 0 | invariant_output 9, work_registers 2, ir_values 2, texture_lanes 1 |

(The first refusal of the cases outside these nine families is `#367`'s and the
`new:` causes of section 18.4: ARITH 1,631, CONST_FOLD 386, FMA 139,
FRAGCOLOR_LANE 828, TEX_COORD 182, TEX_VERTEX 42, CONTROL_FLOW 93,
DEFECT_IRINDEX 187, TEX_VS_FORM 60, FRAGCOORD_FP16 42, DISCARD_FORM 20,
ATTR_LINK 8, ATTR_FOLD 8, DEFECT_ENCODE 6, COLOUR_TOO_LARGE 4, other 2.)

### 0.4 Cases that compile at the tip, by the board's first family

A case "compiles" when the host reports no refusal for it (205 of those end
Pass on the host; the rest stop later, meaninglessly). Cases of that family
at `37305608` from the board's own first-refusal family (`mali-family-tree.py`).

| board family | compile at tip | of |
|---|--:|--:|
| intval | 2,076 | 3,223 |
| varylink | 882 | 1,067 |
| unitype | 215 | 820 |
| constop | 230 | 738 |
| loops | 185 | 774 |
| mixprec | 131 | 251 |
| colourpair | 164 | 171 |
| uniarray | 33 | 272 |
| misc | 13 | 29 |
| 367:ARITH | 416 | 1,585 |
| 367:FRAGCOLOR_LANE | 213 | 793 |
| 367:FMA | 37 | 100 |
| 367:CONST_FOLD | 20 | 463 |
| 367:LOCALS | 15 | 160 |
| 367:CONTROL_FLOW, TEX_COORD, TEX_VERTEX | 0 | 15, 109, 27 |

All of these are HOST-ONLY: "compiles" is the library's compile and link,
not a render.

## 1. The library, and how it differs from the Rust feature set

The plan names the Rust feature set `host-test,gpu-mali,shaderc-online,
no-shader-table` (`MALI_CONFIG` of `tools/check.sh`). `host-test` links
`std` and starts without a GPU back end: it is a compile error in the C
archive (Cargo.toml's note on the feature), and the `yakogl-capi` crate does
not forward it. The C pair dEQP loads is therefore built with the capi
crate's `osal-linux gpu-mali shaderc-online no-shader-table report-es2`, as
`tools/hostharness/glmark2/mali/CMakeLists.txt` builds the glmark2 host
archive, plus the shared pair the top-level `CMakeLists.txt` links
(`--whole-archive`, `tools/yakogl.ver`, `-lm`). The top-level CMake takes
`powervr|vivante|v3d` only, so the script generates its own
`CMakeLists.txt` under `build/host-deqp-mali/lib-src/`. Without `host-test`
the crate has no built-in compiler regions and `src/osal/linux.c` answers
the PowerVR's OSAL contract, so a C shim in the same project (`host_osal.c`,
the `GM2_MALI` table of `tools/hostharness/glmark2/shim.c`) lends the
compiler its scratch (4 MiB) and keep (512 KiB) regions at the library's
sizes and answers `OSAL_QUERY_MALI_GPU` (the T6's counts, as
`src/mali/backend_tests.rs`'s mock) and `OSAL_QUERY_MALI_TILER_HEAP`; its
submit completes at once. The compiler is the same code either way: the
features that differ (`host-test`, `report-es2`) touch no file under
`src/shaderc/online/mali/`.

## 2. What the host reproduced

HOST-ONLY. 10,597 cases asked, 10,597 began, 10,597 ended with a compile
refusal in their log on the host. No case was lost to a crash, a hang or a
timeout (106 processes: 4 exited 0, 102 exited 1, dEQP's "not every case
passed"). Of them:

- **10,577 have the same first refusal as on the board** (same stage, same
  `MaliError` text; a board text cut at the info log's end counts as equal
  when it is a prefix).
- **20 have a different first refusal on the host** (the tip moved from
  `37305608` to `abcc9013`): all 20 are `shaders.random.*` fragment or
  vertex shaders, and all 20 moved between the constant-folding and
  integer refusals (`CONST_FOLD`, `constop`, `intval`, `ARITH`, `LOCALS`,
  `varylink`). Section 11 lists them. They are counted below under their
  HOST family, so a family's host count can differ from section M's board
  count by a few cases.
- **0 not run, 0 without a refusal.**

The 14 Pass cases of section M that log a refusal are in the 10,597 (9
`shader_api.shader_source.*`, 5 expected-failure cases): the tally below
counts them in the family of their refusal with a board status of Pass, so
a family's Fail + NotSupported can be a few short of its case count.

### Families (HOST-ONLY)

| Family | Owner | Cases | Fail (board) | NotSupported (board) |
|---|---|---:|---:|---:|
| intval | this plan | 3224 | 3179 | 41 |
| varylink | this plan | 1068 | 1062 | 5 |
| unitype | this plan | 820 | 724 | 96 |
| uniarray | this plan | 272 | 267 | 5 |
| constop | this plan | 738 | 723 | 6 |
| mixprec | this plan | 251 | 210 | 41 |
| colourpair | this plan | 170 | 170 | 0 |
| loops | this plan | 774 | 106 | 668 |
| misc | this plan | 29 | 29 | 0 |
| ARITH (`arithmetic other than ...`, `three or more additions ...`, `a fragment shader operand of a form ... no reference draws`) | #367 | 1581 | 1522 | 59 |
| CONST_FOLD (`a constant product in a shape Mesa's algebraic rules move or fold`) | #367 | 465 | 465 | 0 |
| FMA (`a fused multiply-add other than a matrix product's`) | #367 | 100 | 100 | 0 |
| LOCALS (`more than 32 local variables`) | #367 | 160 | 144 | 16 |
| FRAGCOLOR_LANE (`a gl_FragColor lane other than the varying's own lane`) | #367 | 793 | 793 | 0 |
| TEX_COORD (`a texture coordinate other than ...`) | #367 | 109 | 109 | 0 |
| TEX_VERTEX (`a texture look-up other than texture2D(sampler2D, coordinate)`) | #367 | 27 | 27 | 0 |
| CONTROL_FLOW (`control flow (if, a loop, a return before the end)`) | #367 | 15 | 13 | 2 |
| other (`a vertex shader whose attribute numbering waits for a fold ...`, a text newer than the slices) | - | 1 | 1 | 0 |
| **Total** | | **10597** | **9644** | **939** |

9,644 Fail + 939 NotSupported + 14 Pass = 10,597: the same 9,644 and 939
section M.1 counts as "have a compile or link refusal in their own log".

The families are the refusal texts the plan names; the mapping text to
family is the `FAMILIES` table at the top of the script's Python. The #367
rows are listed so that every refused case of the slices is accounted for;
their shapes are not classified here (one row `367` each in `summary.md`).

## 3. How a shape is named

A shape is the code construct of the refusing shader that a lowering has to
handle, read from the shader source by the rules of the script's `RULES`
table, tried in order per family; the first rule that holds names the
shape. The rules are regular expressions over the GLSL (declarations of
uniforms and their types, `for`/`while`/`do`, `ivec`/`bvec` constructors,
the relational built-ins, `int(`/`bool(` conversions, constant built-in
calls, literals), not the compiler's refusal site: for the integer family
the compiler names no site, so the shape is the dominant int or bool
construct in the order a lowering would meet it (a random shader first,
then an int or bool uniform, an int loop counter, int vector operators,
bool vectors, conversions, the harness, logic, float compares). Two shapes
are dEQP's own harness rather than a case's feature:

- `harness_isok` (intval): the fragment half of every shader-library
  (`.test`) case, `bool RES = isOk(out0, ref_out0, 0.05); gl_FragColor =
  vec4(RES, RES, RES, 1.0);` with nothing else int or bool in the shader.
  The whole `shaders.preprocessor`, `shaders.constants`, `shaders.functions`
  and friends sit behind this one shape when their vertex shader compiles.
- `fs_uniform_compare_harness` (mixprec): the fragment half of every
  `uniform_api.*` render case, `mediump float compare_float(...) { return
  abs(a - b) < 0.05 ? 1.0 : 0.0; }` products into `gl_FragColor`.
- `random_mixed` (intval, constop): a `shaders.random.*` or
  `fragment_ops.interaction.*` generated shader, which mixes int and bool
  globals, `ivec`/`bvec` constructors with float and bool arguments,
  conversions and constant folds in one shader; no single construct names
  it. These need the general int lowering, not a shape-sized one.

"Next refusal" is what the partner shader of the same program says: dEQP
compiles both shaders before it links, so when the vertex shader is the
refusing one, the fragment shader's own refusal is known (its family is
given); `-` is a partner that compiled. The link and the draw are not
measured: a case whose partner compiled may still be refused at the link
(`compile_vs_linked`) or at the draw. So "unblocks N" below means: N cases
have this shape as their FIRST refusal; when the partner also refuses,
those cases move to the partner's family instead of passing.

Example cases are the first case, in mustpass order, of the shape's largest
dEQP group; the shape's pair under `tools/mali/testdata/deqp_shapes/` is
that case's shader pair verbatim from the `.qpa` (section 10).

## 4. intval: `an integer or boolean value is not lowered yet` (`MaliError::IntValue`, `mod.rs:413`) (HOST-ONLY)

HOST-ONLY, 3,224 cases (board: FS 1,861 Fail + 37 NS, VS 1,317 Fail + 4 NS;
host 3,179 Fail + 41 NS + 4 Pass). Plan #367 holds glmark2's int-counter
loops (`gm2_intloop_*`) and the `&&` of two float compares in a one-armed
`if` (`gm2_cf_ifand_vs`); `docs/notes/mali-int-loops.md` lists the forms
no #367 reference touches (idiv, int arithmetic, dynamic subscripts, int
literals, short-circuit logic).

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks (first refusal) / next refusal |
|---|---:|---|---|---|---|---|
| `ivec_operator`: `int` or `ivecN` locals through `+ - * / % ++ -- += <` and the int relational built-ins, converted from a float varying and back (`shaders.operator.{binary,unary}_operator`, `int_compare`, `sequence`; 936 of them) | 981 | 981 / 0 | 444 / 537 | this plan | `shaders.operator.binary_operator.add.highp_int_fragment` | 948 to a compiling partner; 31 to another intval shader, 2 to ARITH |
| `random_mixed`: generated shaders mixing int and bool globals, `ivec`/`bvec` constructors with float and bool arguments and conversions | 771 | 771 / 0 | 589 / 182 | this plan (needs the whole int lowering, not a shape) | `shaders.random.all_features.fragment.11` | 613; 53 to intval, 50 to constop (and the CONST_FOLD and ARITH forms #367 lowers) |
| `harness_isok`: the shader-library fragment harness alone (`bool RES = isOk(...)`, a float compare to a bool to `vec4(RES, ...)`) | 448 | 444 / 0 (4 Pass) | 0 / 448 | this plan | `shaders.preprocessor.basic.empty_directive_fragment` | 448: `shaders.preprocessor` 252, `shaders.functions` 57, `shaders.constants` 49, `shaders.scoping`, `shaders.declarations`, ...; partner compiled in all 448 |
| `bvec_relational`: `bvecN` values from `lessThan`/`equal`/`notEqual`/`any`/`all`/`not` or a `bvecN(...)` constructor, written to a colour or varying as `vec4(bvec)` | 330 | 330 / 0 | 163 / 167 | this plan | `shaders.operator.binary_operator.equal.bvec2_fragment` | 245; 46 to ARITH, 21 to intval |
| `int_uniform`: a `uniform int`/`ivecN` (the swizzle cases' `ivecN ref_out0` compared with `==`, `uniform_api`'s `compare_int`, the conversions' `uniform int`) | 315 | 275 / 40 | 9 / 306 | this plan (`docs/notes/mali-int-uniforms.md`) | `shaders.swizzles.vector_swizzles.mediump_ivec2_gggg_fragment` | 315, partner compiled in all |
| `bool_uniform`: a `uniform bool`/`bvecN` (`bvecN ref_out0 == out0`, `compare_bool(u_var, true)`) | 212 | 212 / 0 | 27 / 185 | this plan | `shaders.swizzles.vector_swizzles.mediump_bvec2_gggg_fragment` | 188; 20 to ARITH, 2 to FRAGCOLOR_LANE |
| `scalar_conversion`: `int(float)`, `bool(float)`, `float(int)`, `float(bool)` on scalars (`shaders.operator.*` bool results, `shaders.conversions.scalar_to_scalar`, `builtin_variable.max_*` reads of an int constant) | 105 | 105 / 0 | 69 / 36 | this plan | `shaders.operator.binary_operator.equal.bool_fragment` | 87; 18 to intval |
| `step_sign`: `step(edge, x)` or `sign(x)` on floats (the compiler's compare-to-float path) | 21 | 21 / 0 | 0 / 21 | this plan | `shaders.operator.common_functions.step.highp_float_fragment` | 21 |
| `bool_literal`: `true`/`false` as a value (a constant `?:`, `if (false, ...)`, a bool struct member, a constant-condition return) | 15 | 15 / 0 | 13 / 2 | this plan | `shaders.constant_expressions.complex_types.nested_struct_vertex` | 6; 7 to intval, 2 to CONTROL_FLOW |
| `int_loop_counter`: `for (int i = 0; i < N; i++)` with `i == 1`/`i > 1` in the body and a `return` or `break`/`continue` (`shaders.functions.control_flow`) | 11 | 11 / 0 | 4 / 7 | #367 (`gm2_intloop_*` hold the counter; the compare and the return in the body are not in any #367 reference) | `shaders.functions.control_flow.mixed_return_break_continue_fragment` | 7; 4 to intval |
| `float_compare`: a float compare as an `if` condition or a bool local, nothing else int (`shaders.conditionals.if`, `shaders.functions`) | 8 | 7 / 1 | 0 / 8 | this plan | `shaders.conditionals.if.if_else_fragment` | 8 |
| `other`: `#define FOO(a, b) (1 a 9) b 2` int arithmetic through the preprocessor, `__LINE__`, a `const int` expression | 5 | 5 / 0 | 2 / 3 | this plan | `shaders.preprocessor.predefined_macros.line_1_fragment` | 3; 2 to intval |
| `bool_logic`: `&&`/`||` of float compares in an `if` (`compound_statement`, `sequence_statement`) | 2 | 2 / 0 | 0 / 2 | this plan (#367: only the `&&` of two uniform compares in a one-armed `if`) | `shaders.conditionals.if.compound_statement_fragment` | 2 |

What a lowering order gets, by first refusal: the general int and bool
value lowering (registers, `ICMP`/`CSEL`, conversions, the relational
built-ins: steps 1 and 2 of `mali-int-loops.md`'s order) is the only thing
that moves `ivec_operator`, `random_mixed`, `bvec_relational` and
`scalar_conversion` (2,187 cases); `harness_isok` (448) and `float_compare`
(8) need only a float compare to a bool to a float, the smallest step; the
uniform shapes (527) need the int and bool uniform words of
`mali-int-uniforms.md` on top.

## 5. varylink (`mod.rs:499-506`): 1,068 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `equal_lanes`: `two varying lanes of equal value (Mesa's link merges them)`; the vertex_arrays harness `v_color = vec4(u_colorScale * color, 1.0)` with `color = vec3(1.0)` scaled by one attribute, `draw.random`, the swizzle cases' repeated lanes | 553 | 548 / 5 | 553 / 0 | this plan | `vertex_arrays.single_attribute.first.byte_first24_offset16_stride17_quads256` | 454; 35 to FRAGCOLOR_LANE, 34 to intval |
| `equal_lanes_broadcast`: the same text where the varying is a broadcast `vecN(scalar)` (`v_color = vec3(res)`, `vec4(out0)` of a swizzle result) | 104 | 104 / 0 | 104 / 0 | this plan | `shaders.swizzles.vector_swizzles.mediump_ivec2_gggg_vertex` | 29; 69 to ARITH, 4 to FRAGCOLOR_LANE |
| `lane_no_attribute`: `a varying lane computed from no attribute (a constant or a uniform alone ...)`; `vertex_arrays.multiple_attributes` (`color.rg` from attributes, `.b` constant times a uniform), `draw.*`'s constant colour lanes | 252 | 252 / 0 | 252 / 0 | this plan (`mali-varying-constants.md` lowers the flat uniform-only lane; a flat lane beside an interpolated one is what is left) | `vertex_arrays.multiple_attributes.attribute_count.2` | 245; 3 to constop, 3 to CONST_FOLD |
| `position_unwritten`: `a gl_Position lane left unwritten`; the operator vertex harness: `gl_Position = a_position; ... color.xyz = res; v_color = color` with a `vec4(0.0, 0.0, 0.0, 1.0)` base (`shaders.operator.{unary_operator,common_functions,angle_and_trigonometry,geometric,exponential}` vertex cases) | 155 | 155 / 0 | 155 / 0 | this plan (study-varylink: the text names gl_Position but the shape is a varying built from a constant base with some lanes written) | `shaders.operator.angle_and_trigonometry.acos.highp_vec2_vertex` | 154; 1 to intval |
| `varying_unwritten`: `a varying lane left unwritten`; a declared varying never written (`shaders.linkage.varying_{2,3,4}`, `declarations.invalid_declarations.varying_in_fragment_main`) | 4 | 3 / 0 (1 Pass) | 4 / 0 | this plan | `shaders.linkage.varying_2` | 3; 1 to CONTROL_FLOW |

## 6. unitype (`mod.rs:415`, `:422`, `:425`): 820 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `vs_mat_uniform`: VS `uniform mat2`/`mat3` (`shaders.matrix.*_vertex` with a `uniform` operand, `uniform_api` mat2/mat3) | 138 | 138 / 0 | 138 / 0 | this plan | `shaders.matrix.add.dynamic_highp_mat2_mat2_vertex` | 115; 12 to ARITH, 11 to mixprec |
| `vs_struct_uniform`: VS `uniform structType` (members float, vec, int, ivec, bool, arrays, nested structs, samplers; `uniform_api.*`, `shaders.struct.{local,uniform}`, `light_amount`) | 134 | 124 / 10 | 134 / 0 | this plan | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.int_ivec4_both` | 53; 39 to ARITH, 33 to unitype (the fragment half's struct) |
| `fs_mat_uniform`: mediump FS `uniform mat2`/`mat3`/`mat4` (`shaders.matrix.*_fragment`) | 119 | 119 / 0 | 0 / 119 | this plan | `shaders.matrix.add.dynamic_highp_mat2_mat2_fragment` | 119 |
| `vs_int_uniform`: VS `uniform int`/`ivecN` (`shaders.indexing.*` `ui_zero..ui_three` subscripts, `uniform_api`, random) | 117 | 33 / 84 | 117 / 0 | this plan (`mali-int-uniforms.md`, `mali-uniform-index.md`) | `shaders.indexing.matrix_subscript.mat2_dynamic_write_dynamic_loop_read_vertex` | 80; 18 to ARITH, 10 to FRAGCOLOR_LANE |
| `vs_bool_uniform`: VS `uniform bool`/`bvecN` (`uniform_api`) | 67 | 67 / 0 | 67 / 0 | this plan | `uniform_api.value.assigned.by_pointer.get_uniform.basic.bvec2_api_float_both` | 2; 33 to ARITH, 32 to FRAGCOLOR_LANE |
| `fs_struct_uniform`: mediump FS `uniform structType` | 55 | 55 / 0 | 0 / 55 | this plan | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.bool_bvec4_fragment` | 55 |
| `vs_mediump_struct`: `a vertex shader uniform that is not highp ... struct member`: a mediump struct uniform (`uniform_api` float/vec structs, `shaders.linkage.uniform_struct*`) | 53 | 53 / 0 | 53 / 0 | this plan | `uniform_api.value.assigned.by_pointer.get_uniform.basic_struct.float_vec4_both` | 1; 20 to intval, 16 to FRAGCOLOR_LANE, 16 to ARITH |
| `fs_int_uniform`: mediump FS `uniform int`/`ivecN` outside the admitted forms (`uniform_api` `compare_int`, `negative_api.shader.uniform_matrixfv_*`) | 52 | 52 / 0 | 0 / 52 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.int_fragment` | 52 |
| `fs_bool_uniform`: mediump FS `uniform bool`/`bvecN` (and bool arrays) | 33 | 33 / 0 | 0 / 33 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.bool_fragment` | 33 |
| `vs_mediump_array`: a mediump VS uniform array (`uniform mediump vec4 u_var[3]`, `float u_arr[4]` read at a run-time index) | 25 | 23 / 2 | 25 / 0 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.vec4_both` | 3; 11 to mixprec, 11 to ARITH |
| `fs_highp_uniform`: `uniform highp vec4 u_color; gl_FragColor = u_color;` (`fragment_ops.scissor`): a highp uniform stored whole to the lowp colour | 17 | 17 / 0 | 0 / 17 | this plan | `fragment_ops.scissor.clear_color` | 17 (and the 17 scissor cases are otherwise plain draws) |
| `fs_other`: random fragment shaders with `uniform mediump float`/`vec` read in a form the text excludes, `builtin_variable.depth_range_fragment` (`gl_DepthRange`) | 7 | 7 / 0 | 0 / 7 | this plan | `shaders.random.basic_expression.combined.78` | 7 |
| `fs_vec_array`: mediump FS `uniform vec2 u_arr[4]` read at constant indices | 2 | 2 / 0 | 0 / 2 | this plan | `shaders.indexing.uniform_array.vec2_static_read_fragment` | 2 |
| `vs_mediump_basic`: `builtin_variable.depth_range_vertex` (`gl_DepthRange`, a mediump built-in uniform) | 1 | 1 / 0 | 1 / 0 | this plan | `shaders.builtin_variable.depth_range_vertex` | 1 |

## 7. uniarray (`mod.rs:417`, `:419`): 272 cases (HOST-ONLY)

Companion of `docs/notes/mali-deqp-uniarray.md` (study-uniarray), which read
the shapes from the case names; these are the measured ones.

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `bool_array`: `uniform bool u_var[3]` / `bvecN[]` read at constant indices through `compare_bool` (`uniform_api`) | 95 | 95 / 0 | 86 / 9 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.bool_both` | 0 alone: 43 to ARITH, 32 to unitype, 9 to uniarray (the fragment half) |
| `array_in_struct`: a struct uniform with array members (`m2[3]`, `m3[3]`, nested) read at constant indices | 77 | 77 / 0 | 53 / 24 | this plan | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.float_vec4_both` | 25; 26 to ARITH, 23 to uniarray |
| `int_array`: `uniform int u_var[3]` / `ivecN[]` at constant indices | 46 | 46 / 0 | 46 / 0 | this plan | `uniform_api.value.assigned.basic_array_assign_full.basic_array.int_both` | 0 alone: 23 to unitype, 23 to ARITH |
| `float_array`: `uniform float u_var[3]` at constant indices (`uniform_api`, `shaders.indexing.uniform_array.float_static_*`, `state_query`) | 36 | 36 / 0 | 24 / 12 | this plan (`mali-uniform-arrays.md` lowered the run-time index form) | `uniform_api.value.assigned.basic_array_assign_full.basic_array.float_both` | 14; 11 to uniarray, 11 to ARITH |
| `vec_array`: `uniform vec2`/`vec3 u_arr[4]` at constant or run-time indices (`shaders.indexing.uniform_array`, random) | 12 | 8 / 4 | 8 / 4 | this plan | `shaders.indexing.uniform_array.vec2_dynamic_read_fragment` | 8; 2 to ARITH, 2 to mixprec |
| `runtime_index`: `a uniform read at a run-time index other than ...`: a mediump FS `vec4 u_arr[4]` read four times at int-uniform indices, or in a loop | 3 | 2 / 1 | 0 / 3 | this plan (`mali-uniform-index.md`) | `shaders.indexing.uniform_array.float_static_loop_read_fragment` | 3 |
| `struct_array`: `uniform structType u_var[2]` (an array of structs) | 2 | 2 / 0 | 2 / 0 | this plan | `shaders.struct.uniform.nested_struct_array_vertex` | 1; 1 to ARITH |
| `mat_array`: `uniform mat3 u_var0[5]` | 1 | 1 / 0 | 1 / 0 | this plan | `uniform_api.random.10` | 0 (1 to ARITH) |

The `_both` cases of `uniform_api` refuse the vertex shader first and the
fragment shader second with the fragment family (`unitype` or `uniarray`),
so a uniarray lowering alone moves few of them: their fragment halves need
the matching unitype or uniarray fragment form too.

## 8. constop (`mod.rs:449`, `:440`, `:531`): 738 cases (HOST-ONLY)

Companion of `docs/notes/mali-deqp-constop.md` (study-constop), which
reconstructed the generators; these are the measured shapes. #367 holds
`CONST_FOLD` and `ARITH_CONST` as glmark2 meets them (that note's section
5); all constop shapes below are this plan's.

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `zero_operand`: a constant operation with a `0.0` operand: the operator harness's `res = float(0.0)` / `vecN(0.0)` initialisers and `vec4(0.0, 0.0, 0.0, 1.0)` colour base folded with `color * k + 0.5` (`shaders.operator`, `shaders.matrix` increments, `shader_api.shader_source`'s `gl_Position = vec4(0.0)`) | 225 | 216 / 0 (9 Pass) | 18 / 207 | this plan | `shaders.operator.angle_and_trigonometry.degrees.highp_float_fragment` | 223; 1 to intval |
| `random_mixed`: generated shaders whose constant globals fold through int and bool arguments and non-normal results | 202 | 202 / 0 | 20 / 182 | this plan | `shaders.random.all_features.fragment.43` | 197; 2 to intval, 2 to constop |
| `neg_zero_variable`: `a -0.0 that a comparison reads through a variable ...` (`mod.rs:440`); random shaders with `asin(0.0)`, `ivec4(true, 0.0, ...)` folds | 116 | 116 / 0 | 70 / 46 | this plan (`NegZeroShared` in `mali-deqp-constop.md`) | `shaders.random.all_features.fragment.13` | 103; 7 to intval, 3 to CONST_FOLD |
| `const_call_common`: `const float cval = <abs,sign,floor,ceil,fract,mod,min,max,clamp,mix,step,smoothstep>(constants)` (`shaders.constant_expressions.builtin_functions.common`) | 94 | 94 / 0 | 29 / 65 | this plan | `shaders.constant_expressions.builtin_functions.common.ceil_float_fragment` | 65; 29 to intval |
| `const_call_transcendental`: the same with `exp`, `log`, `pow`, `sqrt`, `inversesqrt`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `radians`, `degrees` | 32 | 32 / 0 | 12 / 20 | this plan | `shaders.constant_expressions.builtin_functions.exponential.inversesqrt_float_fragment` | 20; 12 to intval |
| `const_call_geometric`: `length`, `distance`, `dot`, `cross`, `normalize`, `faceforward`, `reflect`, `refract`, `matrixCompMult` on constants | 24 | 24 / 0 | 24 / 0 | this plan | `shaders.constant_expressions.builtin_functions.geometric.distance_vec2_vertex` | 0 (all 24 to intval: the partner is the bool harness) |
| `const_other`: a constant fold elsewhere: `x == 1. && y == 2.` on constants, `shaders.loops` constant bodies, `shaders.matrix` constant operands, `shaders.function` aliasing | 24 | 18 / 6 | 2 / 22 | this plan | `shaders.loops.do_while_constant_iterations.basic_highp_float_fragment` | 24 |
| `const_call_other`: `const bool cval = equal(ivec4(1), ivec4(2)).w` and the other relational built-ins on int constants | 18 | 18 / 0 | 18 / 0 | this plan | `shaders.constant_expressions.builtin_functions.vector_relational.equal_ivec2_vertex` | 0 (18 to intval) |
| `fp16_constant`: `a constant fp16 does not hold ...` at the relink of `shaders.matrix.sub.const_*_mat3_mat3_vertex` | 3 | 3 / 0 | 0 / 3 | this plan (`Fp16Inexact`) | `shaders.matrix.sub.const_highp_mat3_mat3_vertex` | 3 |

## 9. mixprec (`mod.rs:428`, `:525`): 251 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `fs_uniform_compare_harness`: `a fragment shader that mixes highp and mediump or lowp`: the `uniform_api` fragment harness (`compare_float`'s `abs(a - b) < 0.05 ? 1.0 : 0.0` products, `compare_bool`'s `a == b ? ...`, a mediump `v_vtxOut`), no `highp` declared: the mix is the compiler's own precision for the select or the bool | 186 | 186 / 0 | 0 / 186 | this plan (`mali-precision.md`, `mali-mixed-precision.md`) | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.sampler2D_samplerCube_both` | 186 (and these are the fragment halves behind the `_both` cases of sections 6 and 7) |
| `fs_varying_rgb_one`: `a fragment varying load or conversion of a form no reference draws`: `gl_FragColor = vec4(res.rgb, 1.0)` where `res` came from a mediump varying through a loop (`shaders.loops.*_fragment`, the 23 loop cases of section M.2 among them) | 54 | 13 / 41 | 0 / 54 | this plan | `shaders.loops.do_while_constant_iterations.basic_highp_int_fragment` | 54 |
| `fs_mixed_other`: random fragment shaders and `conversions.vector_combine.float_int_to_vec2_fragment` with a declared `highp` beside mediump values | 6 | 6 / 0 | 0 / 6 | this plan | `shaders.random.conditionals.fragment.27` | 6 |
| `fs_varying_other`: the varying-load text on a `return` form (`shaders.return.conditional_return_always_fragment`) or a random swizzle | 3 | 3 / 0 | 0 / 3 | this plan | `shaders.return.conditional_return_always_fragment` | 3 |
| `fs_varying_plain`: the varying-load text on `gl_FragColor = v_color;` alone (the vertex shader's varying precision is the shape) | 2 | 2 / 0 | 0 / 2 | this plan | `shaders.random.basic_expression.vertex.8` | 2 |

## 10. colourpair (`mod.rs:523`, `:528`): 170 cases (HOST-ONLY)

Both texts are refused at the link (`compile_vs_linked`: the board's
`refusals.tsv` says `link error: the vertex shader for the program's
fragment shader: ...`), so the shape is the pair: the operator vertex
harness writes `v_color = color` where `color` has constant lanes, and the
fragment shader is `gl_FragColor = v_color`.

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `constant_then_varying`: `a colour pair of a constant then a varying lane`: `color = vec4(0.0, 0.0, 0.0, 1.0); color.yz = res` (a constant lane before a varying lane in a pair) | 141 | 141 / 0 | 0 / 141 | this plan | `shaders.operator.binary_operator.add.highp_float_vec2_vertex` | 141 |
| `varying_and_zero`: `a colour pair of a varying lane and 0.0 (Mesa's U16_TO_U32)`: `color.x = res` then `.y` stays 0.0 (`default_vertex_attrib.float`, `binary_operator.*_float_vertex`) | 29 | 29 / 0 | 0 / 29 | this plan | `shaders.operator.binary_operator.add.highp_float_vertex` | 29 |

Board 142 + 29 = 171: `shaders.random.basic_expression.vertex.49` moved to
the newer attribute-numbering text (section 11).

## 11. loops (`mod.rs:493`, `:488`, `:491`, `:485`): 774 cases (HOST-ONLY)

Companion of the study-loops note (`docs/notes/mali-deqp-loops.md`) and of
`mali-vs-loops.md` (the design). #367 holds the plain `for` with a constant
or an int-uniform bound (`gm2_intloop_const_vs`, `gm2_cf_loop_uni_{vs,fs}`,
`gm2_intloop_uniform_{vs,fs}`); the rows marked #367 below are those forms.
A `for` whose counter indexes an array, a `while`, a `do-while` and a
varying-derived bound are this plan's. The loop texts are NotSupported on
the board for most cases (dEQP's "Loop type is not supported").

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `vs_for_index`: a VS `for` whose counter subscripts a vector, matrix or local array (`shaders.indexing.*_loop_*_vertex`) | 164 | 54 / 110 | 164 / 0 | this plan | `shaders.indexing.matrix_subscript.mat2_dynamic_loop_write_dynamic_loop_read_vertex` | 132; 16 to ARITH, 8 to loops |
| `vs_while`: a VS `while` (`shaders.loops.while_*_vertex`) | 105 | 0 / 105 | 105 / 0 | this plan | `shaders.loops.while_constant_iterations.101_iterations_vertex` | 103; 2 to intval |
| `vs_do_while`: a VS `do { } while` | 100 | 0 / 100 | 100 / 0 | this plan | `shaders.loops.do_while_constant_iterations.101_iterations_vertex` | 100 |
| `fs_kept_for_index`: `a loop Mesa keeps ... in a mediump fragment shader`: a `for` whose counter subscripts an array (`shaders.indexing.*_fragment`, `shaders.struct`) | 85 | 0 / 85 | 0 / 85 | this plan | `shaders.indexing.matrix_subscript.mat2_dynamic_loop_write_dynamic_loop_read_fragment` | 85 |
| `vs_for_uniform`: a VS `for` with an int-uniform bound (`for_uniform_iterations`, `for_dynamic_iterations`, `infinite_with_conditional_break`) | 69 | 0 / 69 | 69 / 0 | #367 | `shaders.loops.for_constant_iterations.infinite_with_conditional_break_vertex` | 69 |
| `fs_kept_for`: a kept mediump FS `for` (`for_uniform_iterations`, `for_dynamic_iterations`, `select_iteration_count`) | 67 | 0 / 67 | 0 / 67 | #367 | `shaders.loops.for_constant_iterations.select_iteration_count_fragment` | 67 |
| `fs_kept_while`: a kept mediump FS `while` | 66 | 0 / 66 | 0 / 66 | this plan | `shaders.loops.while_constant_iterations.select_iteration_count_fragment` | 66 |
| `vs_for_constant`: a VS `for` with a literal bound (`for_constant_iterations`, `shaders.invariance.*.loop_*`, `functions.control_flow`) | 50 | 48 / 2 | 50 / 0 | #367 | `shaders.loops.for_constant_iterations.101_iterations_vertex` | 29; 15 to unitype, 6 to intval |
| `fs_kept_do_while`: a kept mediump FS `do-while` | 50 | 0 / 50 | 0 / 50 | this plan | `shaders.loops.do_while_constant_iterations.select_iteration_count_fragment` | 50 |
| `vs_for_dynamic`: a VS `for` with a varying-derived bound (`basic_*_float_vertex` forms whose counter is a float) | 9 | 3 / 6 | 9 / 0 | this plan | `shaders.loops.for_constant_iterations.basic_highp_float_vertex` | 9 |
| `fs_unrolled_break`: `a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls` | 6 | 0 / 6 | 0 / 6 | this plan (`mali-loop-break.md`) | `shaders.loops.do_while_uniform_iterations.conditional_break_fragment` | 6 |
| `fs_past_limits`: `a loop past Mesa's unroll limits ...` (`101_iterations_fragment`) | 3 | 1 / 2 | 0 / 3 | this plan | `shaders.loops.do_while_constant_iterations.101_iterations_fragment` | 3 |

Board: `a loop in a vertex shader` 392 NS + 105 Fail = 497, host 497 (the
sum of the `vs_*` rows); `a loop Mesa keeps` 268, host 268; the host total
of 774 is the four texts together.

## 12. misc: 29 cases (HOST-ONLY)

| Shape | Cases | Fail / NS | VS / FS | Owner | Example | Unblocks / next |
|---|---:|---|---|---|---|---|
| `invariant_output` (`mod.rs:455`): `invariant gl_Position` / `invariant varying` (`shaders.invariance.*`, `qualification_order`, `preprocessor.pragmas`) | 12 | 12 / 0 | 12 / 0 | this plan | `shaders.invariance.highp.common_subexpression_0` | 0 alone: 9 to unitype (the invariance harness's fragment side), 2 to FRAGCOLOR_LANE, 1 to intval |
| `algebraic_rewrite` (`mod.rs:561`): `shaders.matrix.mul.const_*_mat2_float_fragment` (a constant matrix times a float) | 6 | 6 / 0 | 0 / 6 | this plan | `shaders.matrix.mul.const_highp_mat2_float_fragment` | 6 |
| `comparison_form` (`mod.rs:558`): `notEqual(vec2, vec2)` and the other float relational forms no reference draws | 3 | 3 / 0 | 0 / 3 | this plan | `shaders.constant_expressions.builtin_functions.vector_relational.notEqual_vec2_fragment` | 3 |
| `ir_values` (`mod.rs:540`): `uniform_api.random.{23,...}` programs over 2048 IR values | 3 | 3 / 0 | 2 / 1 | this plan | `uniform_api.random.23` | 1; 1 to ARITH, 1 to misc |
| `texture_lanes` (`mod.rs:476`): `shaders.struct.uniform.sampler_array_fragment` and `sampler_nested*` (a look-up result's lanes swizzled) | 2 | 2 / 0 | 0 / 2 | this plan | `shaders.struct.uniform.sampler_array_fragment` | 2 |
| `work_registers` (`mod.rs:536`): `shaders.matrix.mul.uniform_lowp_mat4_mat4_vertex` and its mediump twin | 2 | 2 / 0 | 2 / 0 | this plan | `shaders.matrix.mul.uniform_lowp_mat4_mat4_vertex` | 2 |
| `builtin_input` (`mod.rs:496`): `shaders.builtin_variable.fragcoord_xyz` | 1 | 1 / 0 | 0 / 1 | this plan (`mali-builtin-inputs.md`) | `shaders.builtin_variable.fragcoord_xyz` | 1 |

Board: 12 invariant + 2 + 3 + 3 + 2 + 1 + 6 = 29; host 29.

## 13. The 20 cases whose first refusal moved between `37305608` and `abcc9013`

All `shaders.random.*`. Board family -> host family (host text in short):

| Case | Board | Host |
|---|---|---|
| `shaders.random.all_features.fragment.37` | constop (-0.0 through a variable) | CONST_FOLD |
| `shaders.random.all_features.fragment.55` | ARITH | LOCALS |
| `shaders.random.all_features.fragment.86` | constop (-0.0 through a variable) | CONST_FOLD |
| `shaders.random.basic_expression.combined.18` | CONST_FOLD | varylink (two varying lanes of equal value) |
| `shaders.random.basic_expression.combined.85` | CONST_FOLD | intval |
| `shaders.random.basic_expression.fragment.85` | constop (non-finite fold) | CONST_FOLD |
| `shaders.random.basic_expression.vertex.49` | colourpair (constant then varying) | `a vertex shader whose attribute numbering waits for a fold Mesa's back end makes after it numbers the locations` (a text newer than the slices; "other") |
| `shaders.random.conditionals.vertex.41` | CONST_FOLD | constop (non-finite fold) |
| `shaders.random.conditionals.vertex.72` | LOCALS | CONST_FOLD |
| `shaders.random.exponential.fragment.12` | ARITH | intval |
| `shaders.random.exponential.fragment.{55,65,98}` | ARITH | constop (non-finite fold) |
| `shaders.random.exponential.vertex.55` | constop (non-finite fold) | CONST_FOLD |
| `shaders.random.exponential.vertex.98` | intval | ARITH |
| `shaders.random.scalar_conversion.combined.40` | intval | CONST_FOLD |
| `shaders.random.scalar_conversion.combined.9` | CONST_FOLD | intval |
| `shaders.random.scalar_conversion.vertex.4` | intval | CONST_FOLD |
| `shaders.random.texture.fragment.90` | constop (non-finite fold) | intval |
| `shaders.random.trigonometric.fragment.15` | CONST_FOLD | constop (-0.0 through a variable) |

They are the random shaders where #367's and #373's merged lowerings
between the two commits changed which refusal comes first. None of them
compiles on the host.

## 14. The committed pairs: `tools/mali/testdata/deqp_shapes/`

One `<family>_<shape>.vert` and `.frag` per shape this plan takes (71
pairs; the four shapes owned by #367 are not copied, their examples are in
the tables), each the example case's shader pair verbatim from the `.qpa`
(dEQP's generated source: Apache-2.0, The Khronos Group Inc.; a REUSE
`.license` sidecar per file names the case). `index.tsv` lists family,
shape, owner, cases, fail, notsupported (board statuses) and the example
case. The `ref-<family>` items make their `deqp_<family>_*` references
from these pairs (`tools/mali/deqp_ref.sh`); where a shape's example is the
harness rather than the construct (`harness_isok`, the `_both` halves), the
reference items should pick a case of the group they need from
`measure.tsv` of a fresh run.

## 15. Caveats

- The shape rules are regular expressions over the GLSL, tried in order;
  a shader with several constructs is named by the first rule that holds,
  which is the one a lowering meets first in the plan's view, not
  necessarily the one the compiler refused on. The refusal site is not
  logged; the study items have it.
- `random_mixed` is a bucket, not a shape: 973 cases (771 intval, 202
  constop) that no single lowering moves.
- Counts are first refusals only. A case passes only when every refusal of
  both shaders, the link and the draw are lowered; "next refusal" gives the
  partner shader's, the link's and the draw's are unmeasured.
- The host run reports dEQP statuses that mean nothing (Fail from black
  pixels); the script never prints them per case and this note quotes the
  board's.
- The run needs about 4 minutes; the two builds (deqp-gles2 713 steps, the
  library) about 40 minutes on 4 cores the first time and are reused from
  `build/host-deqp-mali/`.

## 16. Colourpair: random and tmp_array references

Plan #377 item `ref-colourpair-rnd`. These are the 24 cases that
`docs/notes/mali-deqp-colourpair.md` left without a reference: the 23
`shaders.random.*` cases whose board refusal (slices `37305608`) is a colour
pair (17 constant then lane, 6 lane and 0.0), plus
`shaders.indexing.tmp_array.float_static_write_static_read_vertex` (lane and
0.0). Their shader text is dEQP's generated text, taken verbatim from the
`.qpa` of a host run at `8820607a` (HOST-ONLY; VK-GL-CTS `93bca01` with
`cts-yakogl-target.patch`). `--filter` takes a single fixed string, so the
cases were measured in two runs:

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --stage run \
    --out <dir>/m-random --filter functional.shaders.random.     # 1,767 cases
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --stage run \
    --out <dir>/m-tmparray --filter indexing.tmp_array.float_static_write_static_read_vertex
# then --stage extract on each; the pairs are <out>/src/<case>.vert|.frag
```

The references are Mesa 26.2.1 drm-shim draws made by the `gm2_*` method,
the same way as the other `deqp_colourpair_*` references:

```sh
tools/mali/drm_shim_ref.sh run <work> $T/deqp_colourpair_rnd_<name>.vert \
    $T/deqp_colourpair_rnd_<name>.frag <out>/deqp_colourpair_rnd_<name> --uniform-values
tools/mali/pan_ref_diff.py extract <out>/deqp_colourpair_rnd_<name>.trace --draw --attributes \
    > $T/deqp_colourpair_rnd_<name>.expect.json      # T=tools/mali/testdata
```

`<work>` is `drm_shim_ref.sh build` of the 26.2.1 tarball, whose sha256
matches `third_party/mesa-26.2.1/UPSTREAM`. The draw is 64x64 with every float
uniform set (`--uniform-values`); none of these pairs has an int uniform.
Checks:
- The same build reproduces `deqp_colourpair_default_attrib.expect.json` byte
  for byte, except for the `trace` name.
- A second draw of each of the 14 references is identical.
- `tools/mali/deqp_ref.sh --instrument shim` draws the same `regions` for all
  24 pairs.

Each `.vert` and `.frag` has a `.license` sidecar that names its case.

### Grouping

All 24 cases were drawn. They give 13 distinct Mesa fragment programs (words
and FAU):
- **Identical pairs.** `texture.fragment.25` and `trigonometric.fragment.25`
  are the same pair, and draw the same.
- **Same fragment shader.** `basic_expression.fragment.68` and `.combined.68`
  have the same fragment shader, and its colour reads no varying. The two
  programs have the same words; only the uniform FAU words differ (`r` of the
  combined vertex shader moves the uniform layout).
- **Same bytes from both stages.** `basic_expression.vertex.60` computes
  `vec4(0.75, c, b, 0.0 * a)` in the vertex shader, and `.fragment.60`
  computes it in the fragment shader. Both produce the same bytes in every
  region. Mesa's link drops the vertex shader's constant lanes x and w and
  compacts the varying to `v2`. Each case has its own reference, because our
  compiler reaches the pair by two different paths: the linked fragment shader
  and the fragment shader's own `vectorise`.
- **Plain four-lane load.** Ten vertex cases draw the plain four-lane
  fragment program. It is byte-equal to `deqp_colourpair_ck_lane`'s
  (`LD_VAR_BUF_IMM.f32...v4`, two `V2F32_TO_V2F16`, `BLEND`), so it has **no
  colour pair**. Mesa keeps the vertex shader's constant lanes as varyings. By
  the source, these split into two shapes:
  - `vecop`: a highp **vector** ALU result whose lane becomes constant only
    through a fold inside the op, for example `0 * a.z` in
    `vec4(d, c, 0, b) * a`. The varying shader of `rnd_vs_vecop_kept` stores
    that lane as `IADD_IMM #0x0`.
  - `mediump`: the colour is a **mediump** expression. Examples:
    `vec4(res, 0.0, 0.0, 1.0)` with `res` mediump, a sum that reads the mediump
    `v_color` back, or a product with a mediump uniform. This is the operator
    harness's `mediump vec4 color` case (`ck_lane`). The varying shader stores
    the constant lanes (`IADD_IMM #0x0`, `#0x3F800000`) after an fp16
    computation.

  For both shapes, the reason is inferred from the sources and the bytes. It
  is not checked in Mesa's source.

This gives 14 references for the 24 cases:

| Reference `deqp_colourpair_rnd_<name>` | Case it was made from | Mesa's fragment program (instrs) | Pins for lower-colourpair |
|---|---|---|---|
| `vs_vecop_kept` | `random.conditionals.vertex.73` | plain v4 load (7) | **link rule**, not an MKVEC: a lane that is constant only through a fold inside a highp vector op stays a stored varying lane |
| `vs_mediump_kept` | `indexing.tmp_array.float_static_write_static_read_vertex` | plain v4 load (7) | **link rule**: the constant lanes of a mediump colour stay varyings (the `ck_lane` rule of `mali-deqp-colourpair.md`) |
| `vs_vecop_fs_uniform` | `random.basic_expression.combined.35` | v4 load, two `FMA.v2f16` by the uniform (9) | the vecop link rule under a fragment shader that is not a plain forward (`b * a`) |
| `vs_uniform_flat` | `random.basic_expression.vertex.20` | `LD_VAR_BUF_IMM.f32...v4.src_flat32` (7) | **link rule**: when every lane is a constant or a uniform (`c + vec4(1.5, b, b, a)`), Mesa keeps all four lanes and loads them **flat**; it removes none |
| `vs_ck_lz` | `random.basic_expression.vertex.60` | `v2` load, `MKVEC.v2i16 r1, u2.w0.h0, r0.h0` (FAU 0.75), `U16_TO_U32 r2, r0^.h1` (10) | constant-first **MKVEC with a FAU constant** as src0, then the lane-zero `U16_TO_U32` of the converted pair's `.h1`; the link drops x and w |
| `vs_zk_lz` | `random.conditionals.vertex.46` | `v2` load, `MKVEC.v2i16 r1, 0x0.h0, r0.h0`, `U16_TO_U32 r2, r0^.h1` (10) | constant-first MKVEC with the **inline** `0x0.h0` (as `ck_lane_highpcolour`), then lane-zero `U16_TO_U32` |
| `fs_ck_lz` | `random.basic_expression.fragment.60` | byte-equal to `vs_ck_lz` in all three regions | the same two forms, reached through the fragment shader's `vectorise` (the `nir_fs.rs` sites `mali-deqp-colourpair.md` lists) |
| `fs_ck_dup` | `random.basic_expression.fragment.94` | one-lane load, `FADD.f32 r0.h0`, `MKVEC.v2i16 r1, u2.w0.h0, r0.h0` (FAU 0.5), `IADD.v2u16 r2, r0^.h00, 0x0` (10) | constant-first MKVEC (FAU), and the (lane, lane) word as `IADD.v2u16 .h00` (`lower_mkvec_swz`) |
| `fs_ck_product` | `random.basic_expression.fragment.84` | `MKVEC.v2i16 r3, u2.w0.h0, r0.h0` and `MKVEC.v2i16 r0, u2.w1.h0, r0^.h0` as **FMA operands** (16) | constant-first MKVEC of an **operand** `vec4(0.25, a, b, a)`, not of the colour: the classifier edit must cover `vectorise` operands too |
| `fs_zk_tree` | `random.conditionals.fragment.73` | `MKVEC.v2i16 r0, u2.w0.h0, r0^.h0` (FMA operand), `FADD.v2f16 r1, r2^.neg.h00`, `MKVEC.v2i16 r1, 0x0.h0, r1^.h0` (13) | constant-first MKVEC of an operand (FAU), and a colour word (0.0, a computed tree `-a.w`) with inline `0x0.h0` |
| `fs_zword_zk` | `random.conditionals.fragment.77` | `MKVEC.v2i16 r1, 0x0.h0, r0^.h0`, `MOV.i32 r0, 0x0` (8) | an all-zero word as `MOV.i32 0x0` and constant-first MKVEC (inline zero); the swizzles `.wxyz.arbg` fold away |
| `fs_kword_ck` | `random.texture.fragment.35` | `MKVEC.v2i16 r1, u2.w0.h0, r0^.h0` (FAU 1.0), `MOV.i32 r0, u2.w1` (FAU word `0x00003c00`, (1.0, 0.0)) (8) | constant-first MKVEC (FAU) and a constant word as **FAU `MOV`** (not an immediate as in `default_attrib`) |
| `fs_ck_madd` | `random.texture.fragment.25` | `MKVEC.v2i16 r0, u3.w0.h0, r0^.h0` (FAU -3.0) as the FMA addend, `IADD_IMM` packed constants, `MKVEC.v2i16 r1, r2^.h0, r1^.h0` (13) | constant-first MKVEC of an **addend** `vec4(-3, c, 0, -1.5) + b * ...`; the colour's second word is two computed halves |
| `fs_uniform_tz_ck` | `random.basic_expression.fragment.68` | no varying: `FADD.v2f16`, `U16_TO_U32 r0, r0^.h0`, `MKVEC.v2i16 r1, u3.w1.h0, r1^.h0` (10) | lane-zero `U16_TO_U32` of a **uniform tree** (`b + 4.5`, `a * 0.0` folded to 0), and a constant-first MKVEC of (1.0, uniform `a`) |

### The 24 cases

The first refusal is the host's at `8820607a`. For every case it is also the
board's text at `37305608`, except `vertex.49`. CP stands for "a colour pair
of a constant then a varying lane", and LZ for "a colour pair of a varying
lane and 0.0 (Mesa's U16_TO_U32)"; both are fragment shader refusals, at the
link when the case is a vertex case.

| Case (`dEQP-GLES2.functional.shaders.`) | First refusal | Group | Reference | What it pins |
|---|---|---|---|---|
| `indexing.tmp_array.float_static_write_static_read_vertex` | LZ | mediump | `rnd_vs_mediump_kept` | link rule: the lanes stay varyings |
| `random.basic_expression.combined.35` | CP | vecop, fragment `b * a` | `rnd_vs_vecop_fs_uniform` | link rule |
| `random.basic_expression.combined.68` | LZ | uniform tree and 0.0 | `rnd_fs_uniform_tz_ck` (same words; the FAU uniform layout differs) | lane-zero form and constant-first MKVEC |
| `random.basic_expression.fragment.60` | CP | constant, lane, then lane, 0.0 | `rnd_fs_ck_lz` | constant-first MKVEC (FAU) and lane-zero form |
| `random.basic_expression.fragment.68` | LZ | uniform tree and 0.0 | `rnd_fs_uniform_tz_ck` | lane-zero form and constant-first MKVEC |
| `random.basic_expression.fragment.84` | CP | constant-first operand | `rnd_fs_ck_product` | constant-first MKVEC (FAU) |
| `random.basic_expression.fragment.94` | CP | constant, lane, then lane, lane | `rnd_fs_ck_dup` | constant-first MKVEC (FAU) |
| `random.basic_expression.vertex.1` | CP | mediump (reads `v_color` back) | `rnd_vs_mediump_kept` | link rule |
| `random.basic_expression.vertex.5` | CP | vecop | `rnd_vs_vecop_kept` | link rule |
| `random.basic_expression.vertex.20` | CP | uniform and constant lanes | `rnd_vs_uniform_flat` | link rule: flat, keep all lanes |
| `random.basic_expression.vertex.30` | LZ | mediump (reads `v_color` back) | `rnd_vs_mediump_kept` | link rule |
| `random.basic_expression.vertex.49` | **other**: "a vertex shader whose attribute numbering waits for a fold Mesa's back end makes ..." (the board's at `37305608` was CP; section 13) | vecop | `rnd_vs_vecop_kept` (same fragment bytes) | nothing for lower-colourpair until the attribute-numbering refusal is lowered, which is a different family |
| `random.basic_expression.vertex.60` | CP | constant, lane, then lane, 0.0, from the vertex shader | `rnd_vs_ck_lz` | link drops x and w; constant-first MKVEC (FAU) and lane-zero form |
| `random.basic_expression.vertex.63` | LZ | vecop | `rnd_vs_vecop_kept` | link rule |
| `random.basic_expression.vertex.70` | LZ | vecop | `rnd_vs_vecop_kept` | link rule |
| `random.basic_expression.vertex.73` | CP | mediump (reads `v_color` back) | `rnd_vs_mediump_kept` | link rule |
| `random.basic_expression.vertex.97` | LZ | mediump (mediump uniform `d`) | `rnd_vs_mediump_kept` | link rule |
| `random.conditionals.fragment.73` | CP | constant-first operand, 0.0 and a tree | `rnd_fs_zk_tree` | constant-first MKVEC (FAU, inline zero) |
| `random.conditionals.fragment.77` | CP | zero word, then 0.0 and a lane | `rnd_fs_zword_zk` | constant-first MKVEC (inline zero) |
| `random.conditionals.vertex.46` | CP | 0.0, lane, then lane, 0.0, from the vertex shader | `rnd_vs_zk_lz` | link drops x and w; constant-first MKVEC (inline zero) and lane-zero form |
| `random.conditionals.vertex.73` | CP | vecop | `rnd_vs_vecop_kept` | link rule |
| `random.texture.fragment.25` | CP | constant-first addend | `rnd_fs_ck_madd` | constant-first MKVEC (FAU) |
| `random.texture.fragment.35` | CP | constant word, then constant, lane | `rnd_fs_kword_ck` | constant-first MKVEC (FAU) and a FAU word |
| `random.trigonometric.fragment.25` | CP | constant-first addend | `rnd_fs_ck_madd` (identical pair) | constant-first MKVEC (FAU) |

What this means for `lower-colourpair`:
- **Link rule cases.** Ten of the 24 cases (`vecop` and `mediump`, with
  `vertex.49` counted among them) and `combined.35` and `vertex.20` need no
  MKVEC. Mesa keeps their constant lanes as varyings, so the work is in our
  link (`mali-varying-constants.md` rule 1), as for the operator cases:
  - a lane that is constant only after a fold inside a vector op stays a
    varying;
  - a lane of a mediump colour stays a varying;
  - a varying with only uniform and constant lanes stays whole and is loaded
    flat.
- **MKVEC cases.** The other 12 need the step 2 edits of
  `mali-deqp-colourpair.md`:
  - constant-first `MKVEC.v2i16`, with the constant as src0, either an inline
    `0x0.h0` or a FAU half;
  - lane-zero `U16_TO_U32` of a load half, a `.h1` half or a uniform tree.

  `fs_ck_product`, `fs_zk_tree` and `fs_ck_madd` show that the constant-first
  pair also occurs as an ALU **operand**, not only as a colour word.

## 17. The board's first refusal per family: `tools/cts/mali-family-tree.py`

Sections 4 to 12 count the host's first refusal per shader pair. The board's
own count, by the first refusal in each slice's `refusals.tsv` of
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`, is:

    python3 -I tools/cts/mali-family-tree.py tools/cts/log/2026-11-15-t6-gles2-S{1,2,3,4}-37305608

It prints a tree table (the form of `mali-gles2-tree.txt`, labels
`T6RF001..`) of rows, one family each, for reruns of one family: families in
the order of `tools/cts/mali-family-refusals.txt` (which maps refusal texts
to families and leaves out what plan #367 holds through glmark2:
`ARITH_CONST`, `ARITH`, `FRAGCOLOR_LANE`, `FS_OPERAND`, `TEX_COORD`, `FMA`,
`LOCALS`, the rebalance, control flow and varying load texts), each family's
cases in mustpass order, at most 400 per row.
`tools/cts/mali-family-tree-selftest.sh` checks the tool
(`cts-selftest.sh` check 52). `check-gles3-tree.py --mustpass
gles2-mustpass.txt --max 400` sees 27 rows, 7,313 cases matched once, 0
matched more than once and 0 empty rows; its exit is 1 because the table is a
partial rerun table, not whole-tree coverage (9,852 mustpass cases are
uncovered). Cases per family, NotSupported included:

| Family | Cases | Family | Cases |
|---|---|---|---|
| `intval` | 3,223 | `varyuniform` | 256 |
| `varylink` | 656 | `mixprec` | 192 |
| `unitype` | 820 | `posunwritten` | 155 |
| `uniarray` | 272 | `negzero` | 117 |
| `constop` | 621 | `invariant` | 12 |
| `colourpair` | 171 | `misc` | 44 |
| `vsloop` + `fsloop` | 497 + 277 | | |

## 18. Ranked refusal texts at `9ac31f30` (HOST-ONLY)

Plan #387 item `rank`. The method is this note's own (sections 1 and 2),
rerun at the integration tip `9ac31f30` (25 commits under
`src/shaderc/online/mali/` after the board run's `de2f2c10`, listed in
section 18.5). HOST-ONLY: compile and link outcomes of `deqp-gles2` against
the Mali C pair on the Linux OSAL; every board status quoted is the
NanoPC-T6's own from the slices `37305608`, never the host's.

- Cases: the same 10,597 of the slices `2026-11-15-t6-gles2-S{1..4}-37305608`
  as section 2 (9,644 Fail, 939 NotSupported, 14 Pass by the board).
- CTS: `93bca01861b0e3ef3c387027a9791e6d065f900c` with
  `cts-yakogl-target.patch` applied, in `$CONNECTED_DEPS_DIR/VK-GL-CTS`.
- Command (4 cores, `--jobs 3 --chunk 100`; most of the wall time is the
  one-off `deqp-gles2` and library builds, the run itself is minutes):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
```

- All 10,597 cases began. 106 processes exited 1 (dEQP's "not every case
  passed") and **2 exited 139** (a segfault of the library, section 18.4);
  the chunks went on in fresh processes, and 2 cases ended `NoResult`.

### 18.1 What the tip compiles

| | Cases | Fail (board) | NotSupported (board) | Pass (board) |
|---|--:|--:|--:|--:|
| asked | 10,597 | 9,644 | 939 | 14 |
| still refused by the compiler at the tip (first refusal, any text) | **6,196** | 5,340 | 846 | 10 |
| **compile and link at the tip** (no refusal text; dEQP on the host reads black, so its verdict is void) | **4,401** | 4,304 | 93 | 4 |

So 4,401 of the 10,597 cases (41.5 %) the board refused at `37305608` no
longer meet a compile refusal on the host. They are *not* passes: by the
host's dEQP verdict 4,204 are `Fail` (black read-back, as on every host
run), 195 `Pass` and 2 `NoResult` (the crashes of section 18.4). Whether they render right is the
board rerun's (item `board-rerun-1`), not this measure's. The board's own
Fail count of this population is 9,644; the first refusal still holds
5,340 of them (55.4 %) and 846 of the 939 NotSupported.

2,805 cases are refused by a **different** text than on the board (their
first refusal moved while the code landed); they are counted below under
the text they meet now. The per-case host and board texts are in
`not-reproduced.tsv` of the run directory (not committed).

### 18.2 Every refusal text still produced, ranked by must-pass cases blocked

A case is counted once, under its **first** refusal (the first one in
its log; the stage is the refusing shader's). `Fail` and `NS` are the
board's status at `37305608`; the 10 Pass cases that still refuse are in
`Cases` only. `Raised at` is the message line of the text in
`src/shaderc/online/mali/mod.rs` (the `MaliError` variant) and the sites
that return the variant (`grep MaliError::<variant>`, first non-test sites,
the count in brackets when more). Cases of the two `Defect` rows are not
refusals by name: they are the back end's own invariant failing.

The 43 rows below are every text the host measure produced; the counts sum
to **6,196** (2,454 vertex, 3,742 fragment). Eight of the texts are newly
named in the script's `FAMILIES` table (`new:*`, below; they were `other`
in section 2).

| # | Refusal text (`MaliError`, message line) | Stage vs / fs | Cases | Fail | NS | dEQP groups (top 3) | Raised at | Owner / status |
|--:|---|---|--:|--:|--:|---|---|---|
| 1 | arithmetic other than +, -, * (fs: +, -, *, abs) (`MaliError::Arithmetic`, `mod.rs:412`) | 574 / 889 | 1463 | 1400 | 63 | shaders.operator 574, shaders.random 326, uniform_api.value 136 | `nir.rs:3695; the fs walk (24 sites)` | #367 ARITH; open for the dEQP shapes (item arith-ref, arith-lower-1) |
| 2 | gl_FragColor lane other than the varying's own or a constant (`MaliError::ColourForm`, `mod.rs:522`) | 0 / 821 | 821 | 820 | 1 | shaders.operator 292, uniform_api.value 188, shaders.matrix 146 | `nir_fs.rs:1394 (18 sites)` | #367 FRAGCOLOR_LANE: glmark2 shapes landed 5244a7f5; dEQP shapes open (items fclane-ref, fclane-lower-1/2) |
| 3 | vs uniform other than float/vec2/3/4/mat4 (`MaliError::UniformType`, `mod.rs:416`) | 462 / 0 | 462 | 314 | 148 | shaders.random 160, shaders.indexing 82, uniform_api.value 78 | `nir.rs:1158, :1180, :4456` | plan #377 unitype: fp16 matrices and bvec3 all(==) landed 9a4a8d93 (11 of 20 refs); int/bool uniforms wait for intval (intval-vs16, intval-rest) |
| 4 | constant product in a shape Mesa folds (CONST_FOLD) (`MaliError::ArithConstant`, `mod.rs:444`) | 298 / 88 | 386 | 386 | 0 | shaders.random 313, shaders.constant_expressions 16, shaders.operator 16 | `glsl.rs:52, :1492, :1590 (36 sites)` | #367 CONST_FOLD; open for the dEQP random shapes (arith-ref) |
| 5 | kept (run-time bound) loop in a mediump fs (`MaliError::LoopMediump`, `mod.rs:489`) | 0 / 281 | 281 | 0 | 281 | shaders.loops 181, shaders.indexing 84, shaders.struct 6 | `nir_fs.rs:5057` | plan #377 map-loops (mali-deqp-loops.md): open, needs intval for counters and bounds; glmark2 shapes #367 |
| 6 | mediump fs uniform other than float/vec2/3/4, vec4 array, int (`MaliError::FsUniform`, `mod.rs:426`) | 0 / 280 | 280 | 280 | 0 | uniform_api.value 128, shaders.matrix 67, shaders.random 42 | `mod.rs:1459; nir_fs.rs:8372 (22 sites)` | plan #377 unitype: partly landed 9a4a8d93; matrices, structs, bool/int open |
| 7 | int or bool value (fs) (`MaliError::IntValue`, `mod.rs:415`) | 0 / 271 | 271 | 247 | 24 | shaders.operator 95, shaders.random 39, shaders.conversions 37 | `nir_fs.rs:1416` | landed in part: fs step 1 ab9cde96, vs step 2 56bed483 (plan #377); left: intval-vs16, intval-reslib, intval-rest |
| 8 | loop in a vertex shader (`MaliError::LoopVertex`, `mod.rs:495`) | 241 / 0 | 241 | 29 | 212 | shaders.loops 156, shaders.indexing 62, shaders.functions 7 | `nir.rs:4174, :4184, :4205, :4255 (11 sites)` | glmark2 vertex loops landed 011bade0 (#367); dEQP shapes open (plan #377 map-loops); int counters need intval |
| 9 | constant operation on a value that is not a finite normal float (`MaliError::ConstantFold`, `mod.rs:450`) | 177 / 27 | 204 | 167 | 28 | shaders.indexing 82, shaders.random 52, shaders.constant_expressions 26 | `nir.rs:2166, :2171, :2274, :2806 (11 sites)` | landed in part: constop 0fd576ea (plan #377); rest open |
| 10 | uniform array (`MaliError::UniformArray`, `mod.rs:419`) | 193 / 0 | 193 | 193 | 0 | uniform_api.value 154, uniform_api.info_query 20, uniform_api.random 19 | `nir.rs:1062; nir_fs.rs:7350` | open: read-only spike 74dabf88 (plan #377 uniarray), no lowering merged; int/bool arrays need intval |
| 11 | back end defect IrIndex (fs; 185 direct, 2 through a link error) (`MaliError::Defect(IrIndex)`, `mod.rs:575`) | 0 / 187 | 187 | 187 | 0 | shaders.swizzles 108, shaders.conversions 45, shaders.matrix 13 | `likeliest mod.rs:1071-1074 (FsPrecision::of re-parse); also :1788, :1913 (unverified)` | NEW at the tip, no owner: a defect, not a refusal (section 18.4) |
| 12 | texture coordinate other than a vec2 varying or two constants (`MaliError::TexCoord`, `mod.rs:468`) | 0 / 182 | 182 | 182 | 0 | texture.specification 86, uniform_api.value 52, shaders.random 25 | `nir_fs.rs (21 sites; tests.rs:1281)` | #367 TEX_COORD: glmark2 shapes landed 5244a7f5; dEQP shapes open |
| 13 | fs mixing highp and mediump/lowp floats (`MaliError::MixedPrecision`, `mod.rs:429`) | 0 / 166 | 166 | 164 | 2 | shaders.random 94, shaders.operator 45, shaders.matrix 15 | `mod.rs:1448, :1571; 17 sites` | landed in part: lower-mixprec c81b75ea, c349f227 (plan #377); rest open |
| 14 | fs operand of a form no reference draws (`MaliError::FsOperandForm`, `mod.rs:510`) | 0 / 152 | 152 | 152 | 0 | shaders.operator 69, shaders.random 35, shaders.matrix 29 | `nir_fs.rs (35 sites; tests.rs:1209)` | #367 FS_OPERAND (with ARITH); open (arith-ref) |
| 15 | fused multiply-add other than a matrix product (`MaliError::FusedMultiplyAdd`, `mod.rs:453`) | 139 / 0 | 139 | 139 | 0 | shaders.operator 42, shaders.matrix 41, shaders.constant_expressions 32 | `no literal site outside tests.rs:1198 (raised through a helper)` | #367 FMA: glmark2 vertex built-ins landed 33d4d021; dEQP forms open (arith-ref) |
| 16 | -0.0 read through a variable (`MaliError::NegZeroShared`, `mod.rs:441`) | 68 / 69 | 137 | 137 | 0 | shaders.random 135, fragment_ops.interaction 2 | `glsl.rs:1866, :2077, :2100, :2141 (15 sites)` | landed in part: 4b38072e, constop 0fd576ea; random shapes open |
| 17 | vs uniform that is not highp (`MaliError::UniformPrecision`, `mod.rs:423`) | 99 / 0 | 99 | 46 | 53 | shaders.loops 41, uniform_api.value 36, shaders.indexing 12 | `mod.rs:1364; nir.rs:1144` | plan #377 unitype, mediump vs uniforms: waits for the mediump vertex walk (intval-vs16) |
| 18 | control flow (if, loop, early return) (`MaliError::ControlFlow`, `mod.rs:411`) | 39 / 34 | 73 | 69 | 4 | shaders.random 46, shaders.conditionals 7, fragment_ops.interaction 6 | `nir.rs:3772, :3821, :3877, :3910 (14 sites)` | #367 CONTROL_FLOW; open |
| 19 | colour pair: constant then varying lane (`MaliError::ColourPair`, `mod.rs:525`) | 0 / 61 | 61 | 61 | 0 | shaders.random 49, shaders.operator 9, fragment_ops.interaction 3 | `nir_fs.rs:8706, :8771` | plan #377 colourpair; 22eb4440 landed in part; random shapes open |
| 20 | vs texture look-up other than 2D/cube at a level (`MaliError::TexVertex`, `mod.rs:459`) | 60 / 0 | 60 | 60 | 0 | uniform_api.value 44, uniform_api.random 10, uniform_api.info_query 6 | `nir.rs:4631` | glmark2 look-ups landed 80403eba; dEQP shapes open; text newly named in FAMILIES |
| 21 | two varying lanes of equal value (`MaliError::VaryingDuplicate`, `mod.rs:507`) | 9 / 40 | 49 | 40 | 9 | shaders.swizzles 33, shaders.indexing 9, shaders.conversions 6 | `nir.rs:5478; nir_fs.rs:11483` | landed in part: varylink 22eb4440, 09adb275 (plan #377); swizzle shapes open |
| 22 | fs varying load or conversion of a form no reference draws (`MaliError::FsVaryingLoad`, `mod.rs:526`) | 0 / 43 | 43 | 43 | 0 | shaders.operator 23, shaders.random 10, shaders.indexing 4 | `nir_fs.rs:5831 (11 sites)` | plan #377 mixprec; open |
| 23 | fp16 arithmetic on gl_FragCoord (`MaliError::FragCoordMediump`, `mod.rs:432`) | 0 / 42 | 42 | 42 | 0 | shaders.conversions 32, shaders.random 9, shaders.functions 1 | `nir_fs.rs:2786, :2820` | #367 FRAGCOORD_FP16: c7c377d3, 98969940 landed for glmark2; dEQP conversion shapes open; newly named in FAMILIES |
| 24 | fs texture look-up other than texture2D(sampler2D, coordinate) (`MaliError::TexKind`, `mod.rs:462`) | 0 / 42 | 42 | 42 | 0 | shaders.random 34, shaders.texture_functions 4, texture.mipmap 4 | `tests.rs:1270 (15 sites)` | #367 TEX_VERTEX (family name); open |
| 25 | varying lane computed from no attribute (`MaliError::VaryingNotFromAttribute`, `mod.rs:504`) | 32 / 0 | 32 | 31 | 1 | shaders.random 23, fragment_ops.interaction 6, shaders.struct 2 | `mod.rs:1839; nir.rs:5471, :5492` | landed in part: varylink 22eb4440 (plan #377); rest open |
| 26 | discard other than a highp fs `if (a < b) discard;` (`MaliError::DiscardForm`, `mod.rs:483`) | 0 / 16 | 16 | 16 | 0 | shaders.discard 16 | `nir_fs.rs (12 sites; tests.rs:1295)` | open, no owner (mali-discard.md); newly named in FAMILIES |
| 27 | colour pair of a varying lane and 0.0 (`MaliError::ColourZeroHalf`, `mod.rs:529`) | 0 / 16 | 16 | 16 | 0 | shaders.operator 9, shaders.random 7 | `nir_fs.rs:8671, :9065` | plan #377 colourpair; open |
| 28 | three or more additions or multiplications in a chain (`MaliError::Rebalance`, `mod.rs:447`) | 8 / 4 | 12 | 12 | 0 | shaders.random 7, shaders.invariance 3, shaders.functions 2 | `glsl.rs:1255, :1270, :1310 (15 sites)` | #367 ARITH (rebalance); open |
| 29 | break/continue behind a run-time condition in an unrolled mediump loop (`MaliError::GuardMediump`, `mod.rs:492`) | 0 / 12 | 12 | 0 | 12 | shaders.loops 12 | `nir_fs.rs:4546` | plan #377 map-loops; open |
| 30 | uniform read at a run-time index (`MaliError::UniformIndexed`, `mod.rs:420`) | 10 / 0 | 10 | 4 | 6 | shaders.indexing 10 | `nir.rs:1215, :1244` | plan #377 uniarray; open |
| 31 | invariant output (`MaliError::Invariant`, `mod.rs:456`) | 9 / 0 | 9 | 9 | 0 | shaders.invariance 9 | `nir.rs:5264` | a3651e1f landed for copies; shaders.invariance forms open |
| 32 | vertex shader at the link's attribute locations (attribute_location.bind_aliasing) (`MaliError::LinkedVertexNotLowered`, `mod.rs:407`) | 8 / 0 | 8 | 8 | 0 | attribute_location.bind_aliasing 8 | `mod.rs:2021` | open, no owner; newly named in FAMILIES |
| 33 | shape Mesa's algebraic rules rewrite without a reference (`MaliError::AlgebraicForm`, `mod.rs:561`) | 0 / 8 | 8 | 8 | 0 | shaders.matrix 6, shaders.random 2 | `nir_fs.rs:6203, :12651` | plan #377 misc; open |
| 34 | vs attribute numbering waits for a fold (`MaliError::AttributeNumbering`, `mod.rs:570`) | 8 / 0 | 8 | 8 | 0 | shaders.random 8 | `nir.rs:3991` | open (shaders.random vertex); newly named in FAMILIES |
| 35 | back end defect Encode (vs) (`MaliError::Defect(Encode)`, `mod.rs:575`) | 6 / 0 | 6 | 6 | 0 | attribute_location.bind_aliasing 6 | `not located` | NEW at the tip, no owner: a defect (section 18.4) |
| 36 | gl_Position lane left unwritten (`MaliError::PositionLane`, `mod.rs:501`) | 6 / 0 | 6 | 6 | 0 | shaders.operator 6 | `nir.rs:2120, :5368` | plan #377 varylink; open |
| 37 | varying lane left unwritten (`MaliError::VaryingForm`, `mod.rs:503`) | 4 / 0 | 4 | 3 | 0 | shaders.linkage 3, shaders.declarations 1 | `nir.rs:5421; motion.rs:216` | plan #377 varylink (shaders.linkage negative cases); open |
| 38 | fs colour of more than 255 vectorised operations (`MaliError::FragmentTooLarge`, `mod.rs:513`) | 0 / 4 | 4 | 4 | 0 | uniform_api.value 2, uniform_api.info_query 1, uniform_api.random 1 | `nir_fs.rs:8225; vec16.rs:247` | open, no owner; newly named in FAMILIES |
| 39 | loop past the unroll limits or with two run-time breaks (`MaliError::LoopForm`, `mod.rs:486`) | 0 / 3 | 3 | 1 | 2 | shaders.loops 3 | `nir.rs:1658, :1660` | plan #377 map-loops; open |
| 40 | constant fp16 does not hold (`MaliError::Fp16Inexact`, `mod.rs:532`) | 0 / 3 | 3 | 3 | 0 | shaders.matrix 3 | `nir.rs:2131, :2986` | plan #377 constop; open |
| 41 | more than 32 work registers (`MaliError::Spill`, `mod.rs:536`) | 2 / 0 | 2 | 2 | 0 | shaders.matrix 2 | `ra.rs:697` | open (shaders.matrix) |
| 42 | more than 2048 IR values (`MaliError::TooManyValues`, `mod.rs:540`) | 2 / 0 | 2 | 2 | 0 | uniform_api.random 2 | `nir.rs:2024, :4294` | open (table size) |
| 43 | texture result lanes other than its own pairs (`MaliError::TexLanes`, `mod.rs:477`) | 0 / 1 | 1 | 1 | 0 | shaders.struct 1 | `nir_fs.rs:9812, :9819` | #367 TEX_LANES; open |
| | **Total** | 2454 / 3742 | 6196 | 5340 | 846 | | | |

`Fail` + `NS` + the 10 Pass cases = 6,196. The mapping text to family is the
script's `FAMILIES` table; `tools/cts/mali-family-refusals.txt` is the
board's tree (section 17) and is unchanged: the new texts were not first
refusals of the board's tree rows (they sit in what the tree leaves out), and
changing it would move `tools/cts/mali-family-tree-selftest.sh`'s pinned
counts without a board rerun.

### 18.3 Work order and the shapes each item must cover

The order is fixed by the plan: integer/boolean first, then `gl_FragColor`
lane sources, then by count. Counts are cases whose first refusal is the
text (section 18.2); "also" names cases of the same text that other shapes
hold. The shape names are the script's (`measure.tsv`, `summary.md`); the
representative pair of each is `shapes/<family>_<shape>.vert|.frag` of a run
(not committed; section 14's `tools/mali/testdata/deqp_shapes/` holds the
committed pairs). The families of #367's glmark2 texts (`Arithmetic`,
`ColourForm`, `ArithConstant`, `TexCoord`, `FsOperandForm`,
`FusedMultiplyAdd`) have no shape classification in the script (one shape
`367`); their shapes are given by dEQP group, and item `fclane-ref` and
`arith-ref` name the real shapes.

**1. Integer and boolean values** (items `intval-vs16`, `intval-reslib`,
`intval-rest`; owner plan #377's intval, `mali-deqp-intval.md`). The direct
text is `IntValue` (271), but the family is now spread over the texts the
walk reached after it; the int/bool-bearing shapes of the table are 1,120
cases:

| Shape (script name) | Text | Cases | Groups |
|---|---|--:|---|
| `int_uniform` | IntValue (fs) | 101 | shaders.functions 30, shaders.indexing 20, shaders.scoping 16 |
| `ivec_operator` | IntValue | 72 | shaders.operator 72 (`not_equal`, `equal`, `less_than` ... `_int_fragment`, ivec) |
| `random_mixed` | IntValue | 40 | shaders.random 39, fragment_ops.interaction 1 |
| `bool_uniform` | IntValue | 31 | shaders.conversions 25, shaders.functions 6 |
| `step_sign` | IntValue | 21 | shaders.operator 21 (`step`, `sign`) |
| `bool_literal`, `bool_logic`, `scalar_conversion` | IntValue | 6 | shaders.conditionals 4, shaders.operator 2 |
| `vs_int_uniform` | UniformType (vs) | 346 | shaders.random 160, shaders.indexing 82, shaders.loops 56 |
| `vs_struct_uniform` | UniformType (vs) | 116 | uniform_api.value 66, shaders.struct 37, uniform_api.info_query 12 |
| `vs_mediump_basic` / `_struct` / `_array` | UniformPrecision (vs) | 99 | shaders.loops 41, uniform_api.value 36, shaders.indexing 12: the mediump vertex walk (`intval-vs16`) |
| `fs_bool_uniform`, `fs_int_uniform`, `fs_struct_uniform` | FsUniform (fs) | 151 | uniform_api.value, shaders.random 12 |
| `bool_array`, `int_array` | UniformArray (vs) | 137 | uniform_api.value 116 |

`intval-vs16` takes the vertex walk's mediump compare path
(`opvs_logical_and`, `opvs_not_equal_bvec3`; `_vertex` `bool_compare`,
`float_compare`, `logical_*`, bvec `equal`/`not_equal`): the
`UniformType`/`UniformPrecision` vertex rows above and the ~100 Fail
of `mali-deqp-intval.md` section 8. `intval-reslib` takes the shader-library
`_vertex` cases with an int output (the fragment compare, section 8's
`reslib_int_vs`). `intval-rest` takes the fragment `IntValue` shapes
(`int_uniform`, `ivec_operator`, `random_mixed`, `bool_uniform`,
`step_sign`) and the fs/array rows, by count. The kept loops of section 11
need the counters of this step (`LoopVertex` 241, `LoopMediump` 281, below).

**1a. `Defect(IrIndex)`, 187 cases, to triage with `intval-rest`** (section
18.4). 152 of them were board `intval` cases: fragment shaders that are mostly
`bvec`/`bool` conversions and swizzles (`shaders.swizzles.vector_swizzles`
108, `shaders.conversions.*` 45, `shaders.matrix.mul` 5,
`shaders.operator.float_compare` 5). They are a defect, not a missing
lowering: the owner decides whether it is a regression of `ab9cde96` (intval
step 1).

**2. `gl_FragColor` lane sources** (`ColourForm`, 821, all fragment; items
`fclane-ref`, `fclane-lower-1`, `fclane-lower-2`; #367 FRAGCOLOR_LANE). By
dEQP group: uniform_api.value 188, shaders.operator.binary_operator 144,
shaders.operator.unary_operator 61, shaders.matrix.mul 46, shaders.indexing
40, shaders.operator.common_functions 30, uniform_api.info_query 27,
shaders.random.basic_expression 21, texture.specification 20,
shaders.matrix.add 16. `5244a7f5` landed glmark2's shapes only. The
refusing shapes are the `gl_FragColor = vec4(v.x, v.y, ...)` colour with a
lane that is not the varying's own (a swizzle, a computed lane, a uniform
read), each with a Mesa reference to byte-compare.

**3. Then by count** (a text of 100 or more cases; Fail + NS in 18.2):

| # | Text (variant) | Cases | Shapes to cover (script shape, or dEQP group) | Item / owner |
|--:|---|--:|---|---|
| 3.1 | Arithmetic other than +, -, * | 1,463 | shaders.operator.common_functions 221, shaders.random.exponential 140, uniform_api.value 136, shaders.operator.binary_operator 126, angle_and_trigonometry 104, shaders.swizzles.vector_swizzles 93, shaders.random.trigonometric 85, shaders.constant_expressions 74, shaders.operator.exponential 64; (vs 574 / fs 889) | `arith-ref`, `arith-lower-1`; #367 ARITH |
| 3.2 | Constant product Mesa folds (`ArithConstant`) | 386 | shaders.random.basic_expression 86, `scalar_conversion` 62, `all_features` 54, `conditionals` 45, `comparison_ops` 17, shaders.constant_expressions 16, operator.angle_and_trigonometry 16; (vs 298 / fs 88) | `arith-ref`; #367 CONST_FOLD |
| 3.3 | Kept loop, mediump fs (`LoopMediump`) | 281 (all NS) | `fs_kept_for_index` 90 (shaders.indexing 84, struct 6), `fs_kept_for` 73, `fs_kept_while` 67, `fs_kept_do_while` 51 | plan #377 map-loops, after intval; open, unnumbered item |
| 3.4 | Mediump fs uniform (`FsUniform`) | 280 | `fs_mat_uniform` 79, `fs_struct_uniform` 75, `fs_other` 50, `fs_bool_uniform` 41, `fs_int_uniform` 35 (the last three in section 1) | plan #377 unitype; open |
| 3.5 | Loop in a vertex shader (`LoopVertex`) | 241 | `vs_for_index` 62, `vs_do_while` 52, `vs_while` 52, `vs_for_uniform` 38, `vs_for_constant` 31, `vs_for_dynamic` 6 (212 NS) | plan #377 map-loops, after intval; `vs_for_*` partly #367 |
| 3.6 | Constant op on a non-finite-normal value (`ConstantFold`) | 204 | `zero_operand` 98, `random_mixed` 60, `const_other` 23, `const_call_transcendental` 16, `const_call_other` 7 | plan #377 constop, rest; open |
| 3.7 | Uniform array (`UniformArray`) | 193 | `bool_array` 90, `int_array` 47, `array_in_struct` 34, `mat_array` 21, `struct_array` 1 | plan #377 uniarray (spike 74dabf88); open |
| 3.8 | Texture coordinate (`TexCoord`) | 182 | texture.specification 86, uniform_api.value 52, shaders.random.texture 25, uniform_api.info_query 11, shaders.discard 4 | #367 TEX_COORD; open for dEQP |
| 3.9 | Mixed highp/mediump (`MixedPrecision`) | 166 | `fs_mixed_other` 101 (shaders.random 94), `fs_highp_declared` 65 (shaders.operator 43, shaders.matrix 15) | plan #377 mixprec, rest; open |
| 3.10 | Fragment operand of a form no reference draws (`FsOperandForm`) | 152 | shaders.operator.binary_operator 42, unary_operator 14, shaders.random.basic_expression 14, shaders.matrix.sub 10, mul 9 | #367 FS_OPERAND; with `arith-ref` |
| 3.11 | Fused multiply-add (`FusedMultiplyAdd`) | 139 (all vs) | shaders.operator 42, shaders.matrix 41, shaders.constant_expressions 32, shaders.random 15 | #367 FMA; with `arith-ref` |
| 3.12 | -0.0 through a variable (`NegZeroShared`) | 137 | `neg_zero_variable` 137 (shaders.random 135; vs 68 / fs 69) | plan #377 constop; open |

3.1 to 3.2, 3.10 and 3.11 are the same #367 arithmetic family (1,463 + 386 +
152 + 139 = 2,140 cases, 35 % of the refusals): the `arith-*` items own them
together. Every text below 100 stays in 18.2 (`UniformPrecision` 99 is
inside section 1; `ControlFlow` 73, `ColourPair` 61 and the rest follow).

### 18.4 New at the tip: two defects and a crash (not in `de2f2c10`'s 40 texts)

- `Defect(IrIndex)` (fragment): 187 cases. The message is the generic
  "back end defect: an invariant of the pipeline did not hold". The site is
  not named by the text; `FsPrecision::of` (`mod.rs:1071-1074`) re-parses
  the shader with `map_err(|_| Defect::IrIndex)` and the cases are
  `bvec`/`bool` shaders (`bvec2 in0 = bvec2(v_in0); out0 = in0.gggg;` in
  `shaders.swizzles.vector_swizzles.mediump_bvec2_gggg_fragment`), so the
  re-parse of a bool-vector source is the first suspect. Unconfirmed: no
  `src/` change was made here. The 2 `link error:` cases are the same
  defect reached through a program link.
- `Defect(Encode)` (vertex): 6 cases, all `attribute_location.bind_aliasing`
  (`max_cond_float` and the like).
- 2 library crashes (SIGSEGV, processes exit 139): the cases
  `dEQP-GLES2.functional.shaders.random.scalar_conversion.combined.96` and
  `.fragment.96`, both board `intval` Fail. They end `NoResult`, counted in
  the 4,401, not in the 6,196.

### 18.5 Landed since `de2f2c10` (the commits behind the status column)

`git log --oneline de2f2c10..HEAD -- src/shaderc/online/mali`: intval step 1
`ab9cde96` (fragment) and step 2 `56bed483` (vertex); unitype `9a4a8d93`;
constop `0fd576ea`; mixprec `c81b75ea`, `c349f227`; varylink `22eb4440`,
`09adb275`; glmark2 colour lanes and texture coordinates `5244a7f5`;
vertex built-ins `33d4d021`; vertex look-ups `80403eba`, `ea40c2b7`;
vertex loops `011bade0`; locals `74308a71`; FAU pages `443c62f5`; fragment
operand widths `6b3eca91`, `4b38072e`, `98969940`; misc `a3651e1f`,
`c7c377d3`; attribute records `24c04082`; uniform rows `3cbecbb9`; the
read-only spikes `205547fa`, `289ac2fa`, `74dabf88` (patches, not merged).
The `LOCALS` text (`more than 32 local variables`) no longer appears
(`74308a71`: up to 256 locals compile).

The issue numbers #31, #32, #33, #4479, #4500, #4536 (precision design),
#4732, #6232 and #6433 are not cited in `docs/notes` or `tools/cts`: this
section's owner column names the plan and the note instead; `gh` was not
available to read them.

### 18.6 Caveats

- HOST-ONLY: the compile and link outcome at the tip, as section 2. A case
  that compiles here is not a pass on the board; it can still mis-render, or
  be refused at the draw.
- Only the cases refused in the slices `37305608` (10,597) are measured.
  The board run of `de2f2c10` (10,489 refused compiles of 40 texts, records
  gitignored, not available here) refused cases the slices did not: a case
  that compiled at `37305608` and was refused only at `de2f2c10` is in
  neither the table nor the 4,401. The top-level counts of
  `tools/cts/RESULTS.md` (9,545 Fail) are the `de2f2c10` run's, not these.
- The first refusal is the refusing stage's; a case whose partner shader
  also refuses moves to the partner's text, not to a pass. "Next refusal"
  per shape is in `summary.md` of a run.
- 20 of the cases of section 13 can move their first refusal between
  runs; the counts here are those of one run at `9ac31f30`.
