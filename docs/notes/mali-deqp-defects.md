<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): `Defect(Encode)` and the library SIGSEGV of the dEQP-GLES2 host measure, root causes and Mesa 26.2.1 references

Plan #387, item `defects-ref` (ROADMAP stage 5, issue #7). Section 18.4 of
`docs/notes/mali-deqp-families.md` names two defects that no item owns. Item
`irindex-ref` owns `Defect(IrIndex)`, and this note leaves it alone:

- **`encode`**: `MaliError::Defect(Encode)`, "Valhall back end defect: an
  invariant of the pipeline did not hold (Encode)", in 6 vertex cases;
- **`lanecycle`**: the process dies of SIGSEGV (`deqp-gles2` exit 139) in 2
  cases.

The item changes no compiler code. It commits four references,
`tools/mali/testdata/deqp_defects_<name>.{vert,frag,expect.json}` (the two
`encode` ones also have a `.bind` file), and `tests/mali_deqp_defects.rs`.
All four fail at the tip: each byte-compare test is ignored, and
`defects_pinned` pins what happens today (section 5).

**HOST-ONLY.** The outcomes at the tip come from `deqp-gles2` run on the host
against the Mali back end's C pair (`tools/mali/deqp-host-measure.sh`). Only
the compile and link outcomes and the crash are real. A board status (Fail)
is the NanoPC-T6's own, from the slices `2026-11-15-t6-gles2-S{1..4}-37305608`.

- **Tip:** `f9b376511cd1fc4b9127c821f72b76f7246c8d70`, plan #387's
  integration tip. Every `file:line` below is at this commit.
- **CTS:** VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` with
  `tools/cts/cts-yakogl-target.patch`, in `$CONNECTED_DEPS_DIR/VK-GL-CTS`.
  The clone was missing and was made again: the commit, its
  `external/fetch_sources.py`, and the patch's three new files.
- **Commands:** one build with `--stage build`, then two filtered runs:

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure-ba" --filter attribute_location.bind_aliasing --jobs 1
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure-sc" --filter shaders.random.scalar_conversion --jobs 3
```

  Results: 24 bind_aliasing cases, 6 of them `Defect(Encode)`. 271
  scalar_conversion cases, with 2 processes that exited 139.
- **References:** Mesa 26.2.1's gallium panfrost on the drm-shim
  (`tools/mali/drm_shim_ref.sh build` from the pinned tarball, into
  `$CONNECTED_DEPS_DIR/mesa-shim`). The README's self-check
  (`gm2_cf_ifelse_vs`) was byte-equal. The `lanecycle` references were made
  with `tools/mali/deqp_ref.sh <vert> <frag> <out> --instrument shim`, the way
  the negzero and fmaloc references were. The `encode` references were made
  the same way except for the bindings, see section 2.3.

## 1. Cases

| Case (`dEQP-GLES2.functional.`) | Board (`37305608`) | Tip (host) |
|---|---|---|
| `attribute_location.bind_aliasing.max_cond_float` | Fail (vertex compile failed) | vs compile: `Defect(Encode)` |
| `attribute_location.bind_aliasing.max_cond_vec2` | Fail | vs compile: `Defect(Encode)` |
| `attribute_location.bind_aliasing.max_cond_vec3` | Fail | vs compile: `Defect(Encode)` |
| `attribute_location.bind_aliasing.max_cond_vec4` | Fail | vs compile: `Defect(Encode)` |
| `attribute_location.bind_aliasing.max_cond_mat2` | Fail | vs compile: `Defect(Encode)` |
| `attribute_location.bind_aliasing.max_cond_mat3` | Fail | vs compile: `Defect(Encode)` |
| `shaders.random.scalar_conversion.fragment.96` | Fail (intval) | fs compile: SIGSEGV, stack overflow (`NoResult`) |
| `shaders.random.scalar_conversion.combined.96` | Fail (intval) | fs compile: SIGSEGV, stack overflow (`NoResult`) |

The other bind_aliasing cases at the tip:

- `max_cond_mat4` and `cond_{float,vec2,vec3,vec4,mat2,mat3,mat4}` compile and
  then meet the link's `LinkedVertexNotLowered`. These are the 8 cases of row
  32 of section 18.2.
- `cond_mat{2,3,4}_offset_1` pass on the host.
- `max_inactive_*` link with no refusal. The host's Fail for them is the
  black read-back.

`max_cond_mat4` is not an `encode` case. `vec4(a_mat4)` reads column 0 only,
so its 8 mat4 attributes take 8 records.

The two random cases have the same fragment shader. Only their vertex shaders
differ.

## 2. Defect `encode`: an attribute record past `LD_ATTR_IMM`'s 16

Fixed by #11388 (options A and B of the table in this note):
`docs/notes/mali-attribute-records.md`, "Two attributes bound to one
location". The text below is the state before it.

### 2.1 Shape

The shaders come from `generateVertexShaderTemplate` in
`glsAttributeLocationTests.cpp`, through `BindMaxAliasingAttributeTest::iterate`
(`:895-921`). With `GL_MAX_VERTEX_ATTRIBS` 16:

- the shader declares 32 `attribute mediump <type> a_N`;
- `a_i` and `a_{i+16}` are both bound to location `15 - i`, one guarded by
  `if (u_A != 0.0)` and the other by `if (u_A == 0.0)`;
- every guarded body is `color += vec4(a_N...)`.

GLSL ES 1.00 allows this aliasing. The vertex shader is compiled
(glCompileShader) before the program is bound and linked, and that compile
already fails.

### 2.2 Root cause (raising site)

1. The compile (`mali::compile_linked_blocks`, `src/shaderc/online/mali/mod.rs:1755`)
   compiles the vertex shader with no bindings. Each statically read
   location gets an attribute record. `nir::Nir::attribute_record`
   (`mali/nir.rs:1961-1964`) numbers the record by the count of read
   locations below it, so 32 read float attributes give records 0..31.
2. `select.rs:955` emits `ld_attr(p, vs.attribute_record(loc)?, comps)`,
   which is `Op::LdAttrImm { index: record, .. }` (`mali/select.rs:234-241`).
3. `pack.rs:662-668` puts `record` into the `LD_ATTR_IMM` operand
   `attribute_index`. In the ISA that field is 4 bits wide
   (`mali/isa/v10.rs:6360`: `Sym::field("attribute_index", 20, 4, Kind::Uint
   { bits: 4 }, ..)`).
4. **Raising site: `src/shaderc/online/mali/pack.rs:180`**, `Fill::done`:
   `isa::encode(&self.i).map_err(|_| MaliError::Defect(Defect::Encode))`.
   The error it maps (seen with a temporary print, not committed) is
   `Field { op: LdAttrImm, field: "attribute_index", why: TooWide { value:
   16, bits: 4 } }`. The invariant that fails is that every encoded operand
   fits its field: record 16 does not fit 4 bits.

The shader never reaches the record table either. Had the packer passed,
`StageCode::attributes` (`MAX_ATTRIBUTES` = 16, `mod.rs:962`) would have
failed at `mod.rs:1418` with `Defect(IrIndex)`.

Reduced in-process (`max_cond_*` cut to 2n float attributes on n
locations): 16 attributes (n = 8) compile, 18 (n = 9) give `Defect(Encode)`.
So any vertex shader that reads more than 16 attribute locations, counting a
matrix as one location per column read, hits it when compiled unbound.

The other back ends do not hit it because they defer. On those builds,
`codegen.rs` counts the declared attribute locations: PowerVR/Vivante at
`codegen.rs:867`, V3D at `codegen.rs:927`. When there are more than 16, it
returns `deferred_vertex` (`codegen.rs:1126-1172`): a record with no code,
`layout_at_link = 1` and every declared attribute. glLinkProgram then
compiles that shader again with the program's map (`compile_vs_linked`,
`src/gl/program.rs:1734`). `deferred_vertex` is `#[cfg(not(feature =
"gpu-mali"))]` (`codegen.rs:1132`), and its doc comment says "Not on the
Mali, whose back end stops before any layout". The Mali entry has no such
count. Even if it deferred, the link's relink
`mali::compile_vs_mapped_blocks` (`mod.rs:2014-2022`) always returns
`LinkedVertexNotLowered`.

This is a Rust error path, not a crash. The compile log carries the text,
and dEQP reports "Vertex shader compilation failed".

### 2.3 Mesa's words

Mesa compiles at link, with the bindings. The 32 names share 16 locations.
`deqp_defects_encode_max_cond_float.expect.json` has 121 `vs.position`
words, 113 `vs.varying` words and 7 `fs.fragment` words. Each position and
varying program has 16 `LD_ATTR_IMM.f32 ... attribute_index:0..15`, one per
location. The two names of a location share its load: the two arms read
the same value, and Mesa CSEs the loads. The guards are mediump compares,
`FCMP_OR.v2f16.ne.m1` / `.eq.m1` on `u3.h00` and `MUX.v2i16.int_zero`. The
reduced reference has 9 records (`attribute_index:0..8`).

`tools/mali/gl_ref.py` binds every attribute in declaration order, at
locations 0, 1, 2, ..., and has no way to alias, so Mesa would refuse the
link with 32 locations. The two `encode` references were drawn by a copy of
`gl_ref.py`, made in the build tree and not committed. It had two changes:

- the `glBindAttribLocation` loop reads the reference's `.bind` file
  (`name location`, which is dEQP's binding: `a_i` and `a_{i+n}` at
  `n - 1 - i`);
- the zero arrays are set up for locations 0..15 only. Without this,
  location 16 and up give `GL_INVALID_VALUE` and gl_ref exits non-zero.

Everything else was `drm_shim_ref.sh run` (the same environment,
`--uniform-values`) and `pan_ref_diff.py extract --draw`. The draw printed
`glGetError 0x0`. A `--bind FILE` option in `gl_ref.py` would make this
reproducible from the tree (section 7).

## 3. Defect `lanecycle`: endless mutual recursion in the fragment colour's pairing

### 3.1 Shape

The fragment shader of both cases (`deqp_defects_lanecycle_fs.frag`) is:

```glsl
precision mediump float;
...
float g = float(2.625) * -1.0 * float(float(-2.0));
void main() {
	gl_FragColor = vec4((float(-1.0)), -0.25 + h, -0.25, (b));
	gl_FragColor = vec4(-1.0, g, f, e) * gl_FragColor * (vec4(d, c, b, a));
	gl_FragColor = gl_FragColor;
}
```

Reduced in-process to `deqp_defects_lanecycle_reduced.frag`, with
`varying mediump float b`:

```glsl
gl_FragColor = vec4(2.0) * b * vec4(0.0, 0.0, b, 0.0);
```

What triggers it is a mediump vector product whose pair (z, w) is
(a computed lane, a constant):

- lane z is `(2.0 * b) * b`, an `fmul`;
- lane w is `(2.0 * b) * 0.0`. When the back end reads it, it is already the
  IR constant 0.0 (`Val::Const`). Which pass folds it was not located.

Variants (each a pair of the reduction, run in its own process):

- `vec4(2.0) * vec4(b) * vec4(1.0, 1.0, b, 0.0)` overflows too.
- With w = 1.0 in place of 0.0 the shader is refused by name ("a fragment
  shader operand of a form ... no reference draws").
- With no constant factor, `vec4(b) * vec4(0.0, 0.0, b, 0.0)`, it is refused
  by name ("a colour pair of a varying lane and 0.0").
- With the zeros in the first factor it compiles.

### 3.2 Root cause (faulting site)

This was seen with a SIGSEGV reporter preloaded into `deqp-gles2` on an
alternate signal stack (`backtrace` + `addr2line` on the release library,
which has debug info; the host has no gdb). The fault address
(`0x7ffd97b0aff0`) is 0x50 below the stack pointer, with the stack limit at
8 MiB, so it is a stack overflow. The backtrace has 24,642 frames:

- **Faulting site:** the bottom 24,620 frames alternate between
  `nir_fs::vectorise` at **`src/shaderc/online/mali/nir_fs.rs:8539`** and
  `nir_fs::lane_tree` at **`nir_fs.rs:9353`**. They are 12,310 rounds of
  each. The innermost frame, where the guard page is hit, is one of the two:
  inside `lane_tree` in one run, at `vectorise`'s entry (`nir_fs.rs:8415`)
  in another.
- **Entry:** `online_compile` (`backend.rs:1278`), then
  `compile_and_publish` (`:1050`), `compile_and_publish_linked` (`:1076`),
  `compile_and_publish_keyed` (`:1160`, `:1185`), `codegen::compile_linked_blocks`
  (`codegen.rs:643`), `mali::compile_linked_blocks` (`mod.rs:1771`),
  `compile_lowered` (`mod.rs:1794`), `compile_module_on` (`mod.rs:1226`),
  `compile_stage`, then `nir_fs::select`, which makes the first `vectorise`
  call of the colour. The call comes from glCompileShader of the fragment
  shader.

The cycle:

1. `lane_tree(v = z)` (`nir_fs.rs:9338`) takes `pr.hint`, then asks
   `Fs::pair_lane(z)` (`nir_fs.rs:1539-1563`). `z` is a 16-bit `fmul` of a
   wide IR vector op, so it is half of the pair `(lanes[2], lanes[3]) =
   (z, Const 0.0)`.
2. The pair is not in `pr.cache` yet. That cache is filled only after
   `vectorise` returns (`nir_fs.rs:9350-9358`). So `lane_tree` calls
   `vectorise(f, pr, z, Const 0.0, ..)` (`:9353`).
3. In `vectorise`, `(alu_of(z), alu_of(b)) = (Some(fmul), None)` and `b` is a
   `Val::Const`. That takes the "computed lane then a constant" arm
   (`nir_fs.rs:8517`). `z` is not an `fneg`, so its `_` branch calls
   `lane_tree(f, pr, a = z, ..)` (`:8539`) to build the lane alone.
4. That is step 1 again with the same lane: the hint is gone, `pair_lane` gives
   the same pair, and nothing is cached. Nothing in the loop bounds it.

This is **Rust, not C**, and it is **neither a panic across the FFI nor an
abort**. The C library build (`panic = "abort"`, no_std, no stack guard
handler) has no Rust runtime to turn the overflow into a message, so the
guard page fault is a plain SIGSEGV that kills the process. In
`cargo test` the same overflow hits the test thread's guard page, and std
aborts the whole process with "has overflowed its stack" (SIGABRT).
Neither can be caught in-process, so `tests/mali_deqp_defects.rs` pins the
crash by running the reduced test in a child process (section 5).

On the board, a Zephyr thread's stack is much smaller than 8 MiB. The same
loop overflows it after a few hundred rounds, and what happens then depends
on the stack guard configuration of the image (not measured, no board run).

### 3.3 Mesa's words

The reduced fragment shader is 11 words. The pair (z, w) is one half-float
FMA of (2b, 2b) by (b, 0.0):

```text
LD_VAR_BUF_IMM.f32.slot0.src_f32.center.store.wait0 @r0, r61^, index:0x0
FADD.f32.discard r0.h0, r0^, 0x0.neg          ; b to fp16
FADD.v2f16 r1, r0.h00, r0.h00                 ; (2b, 2b)
U16_TO_U32 r0, r0^.h0                         ; (b, 0.0)
FMA.v2f16.wait0126 r0, r1^, r0^, 0x0.neg      ; (2b*b, 2b*0.0)
ATEST ... ; MOV.i32 r1, r0^ ; MOV.i32 r0, 0x0 ; BLEND.slot1.v4.f16.end
```

Mesa keeps `2b * 0.0` as a product (NaN and infinity times 0.0 are not
0.0). Lanes x and y are the constant 0.0 (`MOV.i32 r0, 0x0`). The
dEQP shader's reference is 12 words.

## 4. Proposed fixes, one session each, and what each should recover

| # | Fix | Size | Cases it should recover (host compile/link) |
|---|---|---|---|
| A | **Defer the Mali vertex compile past 16 locations.** In `mali::compile_linked_blocks` (`mod.rs:1755`), for `SHC_STAGE_VERTEX`, count the declared attribute locations the way `codegen.rs:861-866` does. When there are more than 16, return a `layout_at_link` record from `deferred_vertex`. That means making `codegen.rs:1132`'s `deferred_vertex` available to the Mali build (drop the cfg or move it to a shared helper; it builds no code and needs no Mali ABI). As a backstop, make `select.rs:955` / `nir.rs:1961` refuse a record of 16 or more with a named `MaliError` instead of reaching the packer. Host tests: the `encode` pins move from `Defect(Encode)` to the link's `LinkedVertexNotLowered` text, and the deferred record has `layout_at_link == 1` and 32 declared rows. | one session | 0 passes alone. The 6 `encode` cases become named link refusals (`LinkedVertexNotLowered`, 14 cases with row 32's 8), and the defect no longer hides them. |
| B | **Lower `mali::compile_vs_mapped_blocks`** (`mod.rs:2014-2022`, today always `LinkedVertexNotLowered`). Lower the source and give each declared attribute the map's location. Names that alias read one location and so share one record and one load, as Mesa's CSE shares it. Then run the regular vertex walk. Records are numbered over the map's read locations (at most 16, `attribute_index` 0..15). Un-ignore `defects_encode_*` when byte-equal. | one session (the map and records). Byte-equality of the `max_cond` references also needs the mediump vertex compare path (`FCMP_OR.v2f16`, `MUX.v2i16`), which is item `intval-vs16`'s. | Up to 14 compile/link: the 6 `encode` cases and the 8 `LinkedVertexNotLowered` cases (`cond_*` ×7, `max_cond_mat4`). Render verdicts are the board's. |
| C | **Break the `lane_tree` / `vectorise` cycle.** In `vectorise`'s computed-lane-then-constant arm (`nir_fs.rs:8517-8545`), build the lane alone without going back through `Fs::pair_lane`. One way is a `Pairs` flag that makes `lane_tree` (`:9338`) skip the pair step for that call. Another is to enter the pair in `pr.cache` as pending before the `vectorise` call at `:9353`, and refuse by name if it is re-entered. Add a depth bound on the mutual recursion that returns a named `MaliError` (never a crash), as `reaches_frag16` bounds its own (`depth < 32`, `nir_fs.rs:8409`). The words Mesa wants for the reduced shape (section 3.3) also need lane w kept as `2b * 0.0`, not folded to 0.0 before the back end. If the fold stays, the shape should end as a named refusal (`ColourZeroHalf`/`ColourPair`-like). | one session | The 2 cases no longer kill the process (on the host, a crash also ends the rest of its 100-case chunk; on the board, the rest of the run). Whether they then compile byte-equal depends on the fold above. Otherwise they are 2 named refusals, Fail by the board. |

Order: C first. A crash costs more than its 2 cases, because every later case
of the process is lost until a fresh process starts. Then A, then B
(B after `intval-vs16` for byte-equality).

### 4.1 Fix C's guard (landed)

The cycle no longer kills the process. `nir_fs::pair_node` (called by
`lane_tree` for both of its pair steps) keeps the pairs whose `vectorise` is
under way; a pair asked for again while its own `vectorise` is open, or
pairs nested deeper than the 16 it tracks, is refused as
`MaliError::Defect(Defect::LaneCycle)`. The two `lanecycle` references are
named refusals now. Breaking the cycle itself (building the lane without
going back through `Fs::pair_lane`) and Mesa's words for the shape are
still fix C's remaining work.

The crash had also reached `tools/mali/route.sh measure` (the gate's
"mali route.sh measure on tools/mali/testdata" row): the two references'
pairs aborted the test process of their shares, so main was red there from
the merge of the product route (74ad6cc50) next to these references
(e993036bf).

## 5. The test (`tests/mali_deqp_defects.rs`)

Features: `--no-default-features --features
host-test,gpu-mali,shaderc-online,no-shader-table`.

- `defects_encode_max_cond_float` and `defects_encode_reduced`: ignored.
  They compile the pair the way the GL core does with aliasing bindings
  (`compile_bound`): both shaders compile, then `compile_vs_linked` runs
  with the map of the `.bind` file and the fragment shader, then `link`.
  The result is compared with `mali_ref::differences`. Today they fail at
  the vertex compile with `Defect(Encode)`.
- `defects_lanecycle_fs` and `defects_lanecycle_reduced`: ignored,
  `mali_ref::assert_matches`. Today they are refused with
  `Defect(LaneCycle)` (section 4.1).
- `defects_pinned` runs now. It checks that every `deqp_defects_*`
  reference on disk is named, and that each one still gives its exact
  refusal (`mali_ref::refusal_change`): `Defect(Encode)` for the `encode`
  references, `Defect(LaneCycle)` for the `lanecycle` ones. When a fix
  lands, the pin names the reference that moved.

Run at the tip: 1 passed, 4 ignored. `cargo fmt --check` and `cargo clippy`
(with the feature set above) are clean for the file.

## 6. Caveats

- HOST-ONLY. The board's statuses are from `37305608`. At that commit the two
  random cases were refused (intval) before the loop could run, so the crash
  was never seen on the board.
- The `encode` references' `.bind` files are an input the committed tools do
  not read yet (section 2.3). `deqp_ref.sh` alone cannot redraw them.
- The pass that folds `(2.0 * b) * 0.0` to 0.0 was not located.
- `max_inactive_*` (7) link at the tip with no refusal. They are outside both
  defects.

## 7. Issues to post

### Issue 1

**Title:** Mali online compiler: `Defect(Encode)` on vertex shaders that read more than 16 attribute locations (dEQP bind_aliasing.max_cond_*, 6 cases)

**Labels:** gpu-mali

> At `f9b37651`, glCompileShader of a vertex shader that reads more than 16
> attribute locations fails with "Valhall back end defect: an invariant of
> the pipeline did not hold (Encode)". The Mali compile numbers one attribute
> record per read location without the program's bindings
> (`nir.rs:1961` `attribute_record`). Record 16 goes into `LD_ATTR_IMM`'s
> 4-bit `attribute_index` (`select.rs:955`, `pack.rs:662-668`), and
> `isa::encode` fails at `pack.rs:180` (`Field { op: LdAttrImm, field:
> "attribute_index", why: TooWide { value: 16, bits: 4 } }`). The other back
> ends defer such a shader to the link (`codegen.rs:867`/`:927`,
> `deferred_vertex` at `codegen.rs:1126`, which is cfg'd out for gpu-mali),
> and the Mali's link relink `compile_vs_mapped_blocks` (`mod.rs:2014`) is
> still `LinkedVertexNotLowered`.
>
> Cases: dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_{float,vec2,vec3,vec4,mat2,mat3}
> (board Fail). With `cond_*` ×7 and `max_cond_mat4` (`LinkedVertexNotLowered`
> today), 14 cases depend on this path.
>
> References: `tools/mali/testdata/deqp_defects_encode_{max_cond_float,reduced}`
> (Mesa 26.2.1 with dEQP's aliasing bindings, the `.bind` files), and
> `tests/mali_deqp_defects.rs` (ignored byte-compares, `defects_pinned`).
> Note: `docs/notes/mali-deqp-defects.md` sections 2 and 4 (fixes A and B).
>
> Fix A, one session: defer to the link past 16 locations (`layout_at_link`
> via `deferred_vertex`), plus a named refusal for a record of 16 or more.
> Fix B, one session: lower `compile_vs_mapped_blocks` so aliased names share
> one record and load. Byte-equality of `max_cond_*` also needs the mediump
> vertex compares of intval-vs16.

### Issue 2

**Title:** Mali online compiler: stack overflow (SIGSEGV) in nir_fs::lane_tree <-> vectorise on a colour pair of a computed lane and a constant (dEQP shaders.random.scalar_conversion.*.96)

**Labels:** gpu-mali

> At `f9b37651`, glCompileShader of the fragment shader of
> dEQP-GLES2.functional.shaders.random.scalar_conversion.fragment.96 and
> .combined.96 never returns. `nir_fs::lane_tree` (`nir_fs.rs:9353`) and
> `nir_fs::vectorise` (`nir_fs.rs:8539`) call each other without end (12,310
> rounds, then the 8 MiB stack is gone: `deqp-gles2` dies with SIGSEGV, exit
> 139; in `cargo test` it is "has overflowed its stack", SIGABRT). For a lane
> that is half of a pair whose other lane is an IR constant, `lane_tree` asks
> `pair_lane`, finds the pair uncached and calls `vectorise`. `vectorise`'s
> computed-lane-then-constant arm (`:8517`) calls `lane_tree` on the same
> lane again, and the cache is filled only after `vectorise` returns.
>
> Smallest shader: `precision mediump float; varying mediump float b; void
> main() { gl_FragColor = vec4(2.0) * b * vec4(0.0, 0.0, b, 0.0); }`. Lane w
> `(2b) * 0.0` is already a constant when the back end reads it. Mesa keeps
> it as a product: `U16_TO_U32` of b, then one `FMA.v2f16` of (2b, 2b) by
> (b, 0.0); 11 words.
>
> References: `tools/mali/testdata/deqp_defects_lanecycle_{fs,reduced}`, and
> `tests/mali_deqp_defects.rs` (ignored byte-compares; `defects_pinned` checks
> in a child process that the reduced shape still overflows). Note:
> `docs/notes/mali-deqp-defects.md` sections 3 and 4 (fix C).
>
> Fix C, one session: build the lane alone in that arm without going back
> through `pair_lane` (or cache the pair as pending and refuse on re-entry),
> plus a depth bound that returns a named `MaliError`. A crash ends every
> later case of its process, so this goes first.
