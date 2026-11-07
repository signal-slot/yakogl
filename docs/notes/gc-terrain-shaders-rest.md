<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: terrain's other shaders, our instruction count against Mesa etnaviv's

Plan #328, item terrain-shaders-rest (issues #1204, #1106; ROADMAP.md
stage 2, "Slow scenes"). Host only: nothing ran on a board and no code
changed. Code is cited at `723a75c8`; glmark2 at the pinned `22c527cb`;
Mesa 26.2.1. `gc-slow-terrain.md` ranks GPU shading first for terrain and
prices it at 0.6 ns an instruction a pixel or vertex; this note counts the
instructions of every shader terrain draws with **except the noise shader**
(`terrain-noise.frag`, `gc-terrain-shaders.md`), for our compiler and for
etnaviv's, says where the difference comes from in our compiler, and prices
the differences with that model.

## Answer in short

- Ours is longer in every shader with real arithmetic: **terrain.frag 148
  against 72 (2.06x)**, terrain.vert 56 against 39 (1.44x), tilt-shift 66 and
  75 against 46 (1.43x, 1.63x), the two bloom blurs 42 and 47 against 24
  (1.75x, 1.96x), the normal-map pass 25 against 21. The overlay pass
  (2 and 2) and the mipmap and clear programs (`src/vivante/internal.rs`,
  held word for word to etna's by `tests/internal_programs.rs`) are equal.
- At the model's cost, the non-noise shaders are **101.5 ms** of GPU time
  at cover x overdraw 1.0 on our code and **59.6 ms** on etnaviv's: the
  gap is **41.9 ms** (about 37 ms at the 0.8 cover of `gc-slow-terrain.md`'s
  central case). The frame's GPU work is 139 to 207 ms there, so closing the
  gap is 20 to 25 % of it, if the model's linear cost holds.
- Five causes, in the selection and the copy passes rather than in the
  instruction encoder or the register count: (1) the GC7000 selection keeps every `for` loop as a
  real loop and reads a uniform array through a clamped `load`, where
  etnaviv unrolls the loop and reads the constant file (terrain.frag
  alone, **54 instructions, 15.6 ms**); (2) copies are propagated only for
  full write masks, so lanes are moved one `mov` at a time (8 ms); (3) no
  `mul` + `add` becomes a `mad` outside the matrix product (7 ms); (4) no
  float constant folding in the GC7000 selection (6 ms); (5) the blur's
  per-tap offsets are built and multiplied one tap at a time, etnaviv's
  four at a time (4 ms). The ranked list with its issues is at the end.

## The two sides and the commands

Shader texts. glmark2 hands the driver the file plus what its `ShaderSource`
adds (`libmatrix/shader-source.cc`: the two precision macros, the default
`precision mediump float;` for fragment shaders, `const float` lines), and
for the blur shaders the `$CONVOLUTION$` text of `blur-renderer.cpp:100-165`
with the constants of `scene-terrain.cpp:84-121` (bloom: radius 2, sigma 4,
tilt-shift 0; tilt-shift: radius 4, sigma 2.7, tilt-shift 0.5; steps
1/800 for the horizontal passes, 1/256 for `bloom_v`, 1/600 for `tilt_v`).
`gen_terrain.py` (appendix) writes that text, one file a program, from
`GLMARK2_SRC`. It did not need the host harness (`tools/hostharness/glmark2`
`run.sh` takes the whole default list; its `GM2_ENTRIES` filter wants
environment variables an unattended run cannot set). The texts are
checked indirectly: our counts below are exactly the `fsi` / `vsi` the
harness trace gave `gc-slow-terrain.md` (42, 47, 66, 75, 148, 25, 56, and
840 for the noise).

```sh
python3 -I gen_terrain.py $GLMARK2_SRC build/terrain-shaders
```

Ours. `tests/zz_scratch_count.rs` (appendix; scratch, not committed: the
contract of this item allows no code) compiles each file through
`SHC_ONLINE.compile`, the path `glCompileShader` takes (the same call as
`tests/es3_gc_const_array.rs`), and prints the instruction count (`usc_dw /
4`), `temps`, the constant file's dwords and the opcode histogram
(`listing::instr_text` writes the listing):

```sh
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
    --test zz_scratch_count -- --nocapture
```

Etnaviv's. `etna_shc` built as `tools/vivante/README.md` "Set up once"
says, in a copy of the Mesa 26.2.1 tree
(`$CONNECTED_DEPS_DIR/mesa-etnaviv/mesa-26.2.1`, never `third_party/`;
binary sha256 `e7ef2974884640f0f35999ecf53f32d0a0d0d29310b6c5f8895e3921153bef02`,
glslangValidator 16.2.0), GC7000UL as the tool describes it (model 0x7000,
revision 0x6204, halti 5). This host has no `pycparser`, which Mesa's
`src/etnaviv/hwdb/meson.build` insists on and `pip` could not fetch here;
the copy's `hwdb.h` is made by a stand-in generator
(`hwdb_nopycparser.py`: it reads the same five `gc_feature_database.h`
files with a brace reader and runs the original `hwdb.h.py`'s template). The
hwdb only describes the GPU, the shader compiler is untouched. A host with
`pycparser` needs no stand-in. `etna_count.py` (appendix) calls
`glsl2vivante.compile_one`; glmark2's two precision macros are resolved
first because `glsl2vivante`'s ES 1.00 reader does not run a preprocessor.

```sh
python3 -I etna_count.py <etna_shc> tools/vivante build/terrain-shaders build/etna-hex
```

The listings were read with `yakogl-tools etna-dis` on both sides (ours from
the test's `.lst`, etnaviv's from the words `etna_shc` prints):

```sh
cargo run -q --release --manifest-path tools/yakogl-tools/Cargo.toml -- \
    etna-dis build/etna-hex/terrain.frag.bin
```

## The table

The constant columns below are constant-file dwords: ours `const_count` (literals and
uniforms; the compiler rounds each to whole registers), etnaviv's the length
of its `uniforms` list. The model column is the shading cost at 0.6 ns an
instruction on `pixels` (480 000 for the 800x600 passes, 65 536 for the 256x256
bloom and normal-map passes, 393 216 vertices for terrain at cover x overdraw
1.0) with the instruction counts of the table.

| shader (program of) | stage | ours | etnaviv | ratio | temps ours / etna | constant dwords ours / etna | model ms ours / etna |
|---|---|---:|---:|---:|---:|---:|---:|
| `terrain.vert` (terrain pass) | vertex | 56 | 39 | 1.44 | 9 / 9 | 52 / 52 | 13.2 / 9.2 |
| `terrain.frag` (terrain pass) | fragment | 148 | 72 | 2.06 | 19 / 11 | 56 / 52 | 42.6 / 20.7 |
| `terrain-blur.frag`, tilt_h (radius 4, 9 taps) | fragment | 66 | 46 | 1.43 | 12 / 5 | 24 / 12 | 19.0 / 13.2 |
| `terrain-blur.frag`, tilt_v (radius 4, 9 taps) | fragment | 75 | 46 | 1.63 | 12 / 5 | 24 / 12 | 21.6 / 13.2 |
| `terrain-blur.frag`, bloom_h (radius 2, 5 taps) | fragment | 42 | 24 | 1.75 | 8 / 5 | 16 / 8 | 1.65 / 0.94 |
| `terrain-blur.frag`, bloom_v (radius 2, 5 taps) | fragment | 47 | 24 | 1.96 | 8 / 5 | 16 / 8 | 1.85 / 0.94 |
| `terrain-normalmap.frag` (normal map) | fragment | 25 | 21 | 1.19 | 5 / 4 | 4 / 4 | 0.98 / 0.83 |
| `terrain-overlay.frag` (bloom overlay) | fragment | 2 | 2 | 1.00 | 2 / 2 | 4 / 4 | 0.58 / 0.58 |
| `terrain-texture.vert` (every quad pass) | vertex | 7 | 4 | 1.75 | 3 / 2 | 4 / 4 | 6 vertices: 0 |
| `terrain-luminance.frag` (specular map) | fragment | 7 | 3 | 2.33 | 4 / 3 | 4 / 4 | not in the 52-kick frame |
| **sum of the drawn ones** | | | | | | | **101.5 / 59.6** |

For reference the same run compiled `terrain-noise.frag` (item
noise-shader-count's): 840 against 414 instructions, 14 against 12 temps.
`gc-terrain-shaders.md` owns it.

Our temporaries are higher in the two long shaders (19 against 11 in
terrain.frag, 12 against 5 in the tilt-shift passes). `gc-slow-terrain.md`'s
model does not price registers and this note does not either; the
difference comes from the same causes (a value per tap kept alive until its
`mul` and `add`, where a `mad` consumes it at once).

## Why ours is longer, shader by shader

### terrain.frag: 148 against 72

Our listing has three parts. Ours, by instruction number: set-up and the
texture work 0 to 36 (37 instructions); the one-iteration lighting loop 37
to 134 (98); the epilogue 135 to 147 (13). Etnaviv's has no branch: 72 =
the same set-up (28), the lighting (37), the epilogue (7).

- **The loop is real (about 14 instructions).**
  `for (int i = 0; i < MAX_POINT_LIGHTS; i++)` with `MAX_POINT_LIGHTS` 1
  is a constant-count loop. `select.rs:2943` `loop_convert` keeps every
  loop as a loop and puts the hidden counter guard in front of it
  (`mov cnt, 0; add; cmp.ge; branch.not; branch`, the comment at
  `select.rs:2950-2985`; the loop's own exit test; the body's `i` init and
  step): the 6 `branch`es, the counter and `i` set-up, `cmp.ge.s32` and
  `add.s32`s, instructions 35 to 45 and 132 to 134. The GC7000 selection never unrolls (`rg unroll` finds
  nothing in `select.rs` or `vivante/`); the shared unroller
  `powervr/unroll.rs:218` `unroll` is used by the PowerVR and the V3D only
  (`MAX_ITERATIONS` 256 at `powervr/unroll.rs:82`, the exact count from
  `loop_count.rs:46` `constant_loop_entries`, which `loop_convert` already
  calls for the guard's limit). Etnaviv runs `nir_opt_loop_unroll`
  (`etnaviv_compiler_nir.c:193`, `max_unroll_iterations = 32` at
  `etnaviv_compiler.c:68`) between `nir_opt_algebraic` and
  `nir_opt_constant_folding` (`:181-182`), so the loop vanishes and `i` is
  the constant 0.
- **Five uniform-array reads cost 8 instructions each (40).**
  `pointLightPosition[i]`, `pointLightDistance[i]` (twice),
  `pointLightColor[i]` (twice) are indexed by `i`. The selection emits the
  run-time form of `select.rs:2455-2530`: two compares and two selects to
  clamp the index to `0..len-1` (`select.rs:2462`), `imullo0.s32` by the
  stride, an `add.s32` of the array's base, and a `load.u32` from the
  uniform block's address (the comment at `select.rs:2481-2492` explains why
  it is not the constant file's relative addressing: it reads element 0 on
  the i.MX8MP). Instructions 46 to 53, 62 to 69, 74 to 81, 106 to 113 and
  119 to 126 of our listing. Etnaviv has the loop unrolled, so `i` is 0, so
  each is a constant-offset read that `etna_nir_lower_ubo_to_uniform`
  (`etnaviv_compiler_nir.c:1315`) turns back into a constant-file register:
  no instruction. (A constant index written in the source already takes the
  constant path, `lower.rs:2087`; here the index is constant only after the
  unroll, so the unroll has to run before the selection reads the index, or
  the selection has to read the constant out of the IR.)
- **Copies (about 12 of the rest).** Ours has 33 `mov`s against etnaviv's
  12: the vec3 `normal`, `lVector`, `pointHalfVector`, the
  `texture2D(...).xyz * 2.0 - 1.0` chain and the epilogue each pass through
  partial-mask copies (`mov t8.xy__, t15`, `mov t15, t8`, `mov t1.xyz_, t6`)
  that `select.rs:3422` `propagate_copies` leaves, because it only
  propagates a copy whose destination mask is the whole register
  (`select.rs:3436`), and that `alloc.rs:5606` `coalesce_copies` leaves
  because the program has a branch (`alloc.rs:5595`, `:5607`: any program
  with a `target` is skipped). Item 1 removes the branch; the mask
  restriction stays.
- **The rest (about 8): constants and fusion.** `vec3 * 2.0 - 1.0` is a
  `mul` and an `add` (instructions 5, 6; etnaviv: one `mad` with both
  constants in the constant file, its instruction 3), `1.0 - x`
  is `mov -x; add` (instructions 17, 18; etnaviv: `add 1.0, -x` with the
  negation on the operand, its instruction 14: the negated source is the
  same hardware modifier we use elsewhere), `mix` is `mov -`, `add`, `mad`
  where etnaviv has `add`, `mad`. These are items 2 to 4 below.

### terrain.vert: 56 against 39

Both have the same 16 + 2 texture-and-matrix core; the 17 extra are:

- **`mat4 * vec4(p, 1.0)` is a full product (8).**
  `modelViewMatrix * vec4(position, 1.0)` and three `normalMatrix *
  vec4(n, 1.0)` are four-column products in ours: `mov xyz; mov w, 1.0;
  mul; mad; mad; mad` (6 instructions; listing 0 to 5, 7 to 12, ...). The
  front end builds a `Fmul` and three `Ffma` whatever the vector
  (`lower.rs:6122-6141`, `mat_times_column`), so the w = 1 lane is a
  product with a constant. Etnaviv folds it to `mov u(column 3); mad; mad; mad`
  (4: listing 0 to 3, 5 to 8): the `1.0` constant propagates into the
  `fmul`, `nir_opt_algebraic` turns `a * 1.0` into `a`, `nir_opt_constant_folding`
  does the rest. The GC7000 selection has no `fold_float` call (`fold.rs:24`
  is called from `powervr/select.rs:3865` and `v3d/prepare.rs:729`, not
  from `select.rs`), so a constant lane stays a lane.
- **Copies and negation (9).** ours 17 `mov`, etnaviv's 6:
  `mov t1, t7` and `mov t2, t6` (ours 25, 29, 33, 34), the three
  copies into the output registers at the end (ours 53 to 55: the
  allocator reserves the fixed output registers, so a value computed in
  another register is copied), and `mov t2, -t2` (ours 27) where etnaviv
  puts the negation on the next instruction's source (`-t5.xyzx` in its
  `mad`, listing 20). `mul 2.0; add -1.0` (ours 46, 47) is one `mad` with
  both constants in etnaviv (its 34).

### tilt-shift (`terrain-blur.frag`, radius 4): 66 and 75 against 46

Per tap ours emits 7 or 8 instructions, etnaviv 5. For a horizontal tap
(`texture2D(Texture0, TextureCoord + vec2(k.0 * stepX, 0.0)) * Kernel_k +`):

| per tap | ours | etnaviv |
|---|---|---|
| offset `k.0 * stepX` | its own `mul` (9 of them, listing 11 to 28) | four offsets from one vec4 `mul u0.xyzw, |step|` and the rest from one more (listing 1, 29) |
| the vec2 `(offset, 0.0)` | `mov x`, `mov y 0.0` | the same two `mov`s |
| coordinate + offset, `texld` | `add`, `texld` | `add`, `texld` |
| `* Kernel_k +` | `mul`, `add` (17 of them) | `mad` (8) and one `mul` |

and the passes' common head: `abs(TiltShift - TextureCoord.y) / abs(1.0 -
TiltShift)`. `TiltShift` is `const float TiltShift = 0.500000;` so `abs(1.0 -
TiltShift)` is 0.5: ours computes it (`mov 0.5; mov -; add 1.0; mov |.|;
rcp`, 5 instructions, listing 0 to 4), etnaviv has it folded into the
constants (`add 0.5, -y; mul u0, |.|`, its 0 to 1). The 66 - 46 = 20 are:
5 (the folded divisor) + 8 (`mad` fusion) + 7 (offsets: 9 scalar `mul`s
against 2 vec4 `mul`s). `tilt_v` is 9 more (75 against 66): its offsets are
`vec2(0.0, k.0 * stepY)`, so the `mov x 0.0` and the `mov y` land in two
partial-mask copies per tap (25 `mov`s in all against 17).
The bloom passes (radius 2) are the same code with 5 taps and the
same divisor: 42 against 24 and 47 against 24.

Where: the missing fusion is the lack of any `mul` + `add` -> `mad` rule:
the only `Ffma` the front end creates is the matrix product above
(`lower.rs:6140`) and `lower.rs:6587` (the `fma` builtin); nothing in
`select.rs` recognises `Fadd(Fmul(a, b), c)`. The packing of the offsets is
etnaviv's `nir_opt_vectorize`: our per-tap `Fmul` is a one-lane
value built from a constant and `stepX` (a scalar), so four of them are
four instructions. The divisor is the same missing fold as above.

### terrain-normalmap.frag (25 against 21), terrain-luminance.frag (7 against 3), terrain-texture.vert (7 against 4)

- normalmap: three `texld`s of `heightMap` at `vUv`, `vUv + (1/resolution.x, 0)`,
  `vUv + (0, 1/resolution.y)`; ours 9 `mov` against 6, one `mad` less
  (`0.5 * normalize(..) + 0.5`: `mul` + `add`). The same causes, 4
  instructions.
- luminance (`vec4(v, v, v, texel.w)`): ours is `dp3` into one lane then
  **four** `mov`s and an output copy, etnaviv's `dp3` writes three lanes at
  once (`t2.xyz_`) and one `mov` the w. A destination mask wider than one
  lane for a scalar result is the splat; ours has no rule for it.
  Not drawn every frame (not in `gc-slow-terrain.md`'s 52-kick frame).
- the quad vertex shader: `uvScale * (position.xy * 0.5 + 0.5) + uvOffset` is
  `mul; add; mul; add` in ours (listing 0 to 3) and `mad; mad` in etnaviv's
  (0, 1): fusion again. It runs on six vertices, so it only matters for the
  count.

## What the instructions cost: ranked compiler changes

The model of `gc-slow-terrain.md`: 0.6 ns an instruction a pixel or vertex
(0.8 ns as its upper case, which scales every number below by 1.33). One
instruction costs **0.288 ms** a frame in a full 800x600 pass (0.23 to 0.35
for terrain.frag at cover x overdraw 0.8 to 1.2), 0.039 ms in a 256x256
pass and 0.236 ms in the terrain vertex stage (393 216 vertices). The
ms below are the instructions removed from the listings above, by cause,
times those costs; the causes overlap by a few instructions, the total does
not (41.9 ms with the 0.6 ms that is in none).

| rank | change | removes | ms a frame (terrain) |
|---:|---|---|---:|
| 1 | **Unroll constant-count loops on the GC7000 and resolve the constant-index uniform arrays** (`select.rs:2943` `loop_convert`; the shared `powervr/unroll.rs:218`; `select.rs:2455-2530`) | terrain.frag 54 (14 loop control + 40 array reads) | **15.6** (12.4 to 18.7 over cover 0.8 to 1.2) |
| 2 | **Copy propagation for partial write masks, a coalescer that works past a branch, direct writes to the output registers** (`select.rs:3422-3436`, `alloc.rs:5595-5607`, `alloc.rs` `build_one`) | terrain.frag 12, terrain.vert 9, tilt_v 8, bloom 11 + normalmap 3 | **8.4** |
| 3 | **`mul` + `add` -> `mad` fusion** in the selection (nothing today beyond `lower.rs:6140`) | tilt 8 + 8, terrain.frag 4, terrain.vert 2, bloom 8 | **6.6** |
| 4 | **Float constant folding in the GC7000 selection** (`fold.rs:24` `fold_float`, used by `powervr/select.rs:3865` and `v3d/prepare.rs:729`, not by `select.rs`): `mat * vec4(v, 1.0)`, `abs(1.0 - TiltShift)`, `x * 2.0 - 1.0` | terrain.vert 8, tilt 5 + 5, bloom 10, terrain.frag 4 | **6.4** |
| 5 | **Vector packing of the blur's per-tap offsets** (one vec4 `mul` for four scalar taps) | tilt 7 + 7, bloom 6 | **4.2** |
| | all five (2 more instructions of terrain.frag are in none: 0.6) | | **41.2** (37 at cover 0.8) |

How sure each is:

- Rank 1 is the largest and the surest: the instructions are counted in the
  listing, and a loop with a branch also keeps `coalesce_copies` out (rank 2
  gets better after it). A GC7000 `branch` is not free, so the model may
  under-state it. Three conditions: a loop whose count is 1 to 32 (etnaviv's
  limit; the PowerVR's is 256) and a constant uniform index after the
  unroll. The array reads need rank 1 to be one change, not two: after the
  unroll their index is a constant and `lower.rs:2087`'s constant index path
  applies, so no separate work is needed for the load.
- Ranks 2 to 5 are small per shader and were attributed by reading the two
  listings side by side; the split between them overlaps by a few
  instructions (a `mov` that rank 4's fold makes dead counts for rank 2 or
  rank 4).
- The tilt-shift and bloom shaders are 9 and 5 `texld`s of 46 and 24
  instructions after the changes (5 instructions a load; `effect2d`'s 7 in
  the calibration of `gc-slow-terrain.md`), so a texture-bound pass would
  keep part of the 14 ms: the ALU count is the model's, the texture unit's
  rate is not measured separately.
- Total: **about 37 to 42 ms of the 139 to 207 ms** GPU time. If terrain
  stays GPU-bound as `gc-slow-terrain.md` finds, that moves the board's
  central case (183 ms, 5.5 FPS) to about 145 ms (6.9 FPS). It does not by
  itself reach the 60 FPS target; it is one of several terms (the noise
  shader's 840 against 414 is the next: 426 instructions x 0.039 ms =
  16.7 ms, item noise-shader-count's).

## Not done, and what would settle the open ends

- No board run: the X2 is not touched by this item. The prediction to check
  after a change is terrain's ms a frame on the X2 (`tools/cts/x2-glmark2-scenes.sh`
  once plan #323's configuration is on main), expected 40 to 45 ms lower for all
  five, and the per-pass GPU time from the PERF pages of fb-counters.
- The costs assume 0.6 ns an instruction and one instruction per clock
  from every kind (`texld`, `branch`, `rcp` as `mul`). The vertex stage's cost
  per instruction is the least known term (`gc-slow-terrain.md`, "Not
  settled").
- Etnaviv's compile is Mesa 26.2.1's, run on the host; the Mesa driver for
  the board may differ in the version it ships. Instruction counts are for
  the compile of one pass; Mesa's driver recompiles a program for state
  (for example a different `TextureStepY` constant does not change the
  count).
- The reconstructed texts (appendix) agree with the harness in 7 counts; not
  checked by byte comparison with the harness's `shaders/` corpus.

## Appendix: the scripts

`gen_terrain.py` (run as `python3 -I gen_terrain.py $GLMARK2_SRC <out>`):

```python
import math, os, sys

root, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
MACROS = ("#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)\n#define HIGHP_OR_DEFAULT highp\n#else\n"
          "#define HIGHP_OR_DEFAULT\n#endif\n#if defined(GL_ES)\n#define MEDIUMP_OR_DEFAULT mediump\n#else\n"
          "#define MEDIUMP_OR_DEFAULT\n#endif\n")
FPREC = "#ifdef GL_ES\nprecision mediump float;\n#endif\n"

def rd(n):
    return open(os.path.join(root, "data/shaders", n)).read()

def const(n, f):
    return "const float %s = %f;\n" % (n, f)

def wr(name, text):
    open(os.path.join(out, name), "w").write(text)

def blur(name, radius, sigma, d, step, tilt):
    side = 2 * radius + 1
    sigma = max(sigma, 1.0)
    vals, s = [], 0.0
    for i in range(radius + 1):
        s2 = 2.0 * sigma * sigma
        k = 1.0 / math.sqrt(math.pi * s2) * math.exp(-(i * i) / s2)
        vals.append(k)
        s += k
    s += s - vals[0]
    c = "".join(const("Kernel%d" % i, vals[i] / s) for i in range(radius + 1))
    c += const("TiltShift", tilt)
    c += const("TextureStepX" if d == "H" else "TextureStepY", step)
    ax = "X" if d == "H" else "Y"
    ss = "float step%s = TextureStep%s * abs(TiltShift - TextureCoord.y) / abs(1.0 - TiltShift);\n" % (ax, ax)
    ss += "result = \n"
    for i in range(side):
        o = i - radius
        v = "vec2(%d.0 * stepX, 0.0)" % o if d == "H" else "vec2(0.0, %d.0 * stepY)" % o
        ss += "texture2D(Texture0, TextureCoord + %s) * Kernel%d +\n" % (v, abs(o))
    ss += "0.0 ;\n"
    wr(name, MACROS + FPREC + c + rd("terrain-blur.frag").replace("$CONVOLUTION$", ss))

wr("terrain.vert", MACROS + rd("terrain.vert"))
wr("terrain.frag", MACROS + FPREC + rd("terrain.frag"))
wr("quad.vert", MACROS + rd("terrain-texture.vert"))
wr("normalmap.frag", MACROS + FPREC + rd("terrain-normalmap.frag"))
wr("luminance.frag", MACROS + FPREC + rd("terrain-luminance.frag"))
wr("overlay.frag", MACROS + FPREC + rd("terrain-overlay.frag"))
wr("noise.frag", MACROS + FPREC + rd("terrain-noise.frag"))
wr_args = [("bloom_h.frag", 2, 4.0, "H", 1 / 800, 0.0), ("bloom_v.frag", 2, 4.0, "V", 1 / 256, 0.0),
           ("tilt_h.frag", 4, 2.7, "H", 1 / 800, 0.5), ("tilt_v.frag", 4, 2.7, "V", 1 / 600, 0.5)]
for a in wr_args:
    blur(*a)
```

`tests/zz_scratch_count.rs` (the directory is read from a constant; the
listing goes to `build/terrain-lst/<file>.lst`):

```rust
#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

#[test]
fn count() {
    let dir = "build/terrain-shaders";
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    for n in names {
        let src = std::fs::read_to_string(format!("{dir}/{n}")).unwrap();
        let stage = if n.ends_with(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        forget_compiled();
        let text = std::ffi::CString::new(src).unwrap();
        let mut sh = ShcShader::ZERO;
        let mut log = [0 as core::ffi::c_char; 512];
        let compile = SHC_ONLINE.compile.unwrap();
        let rc = unsafe { compile(stage, text.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
        assert_eq!(rc, 0, "{n}");
        let words = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        let (mut lst, mut ops) = (String::new(), std::collections::BTreeMap::<String, usize>::new());
        for (i, c) in words.chunks(4).enumerate() {
            let ins = decode::decode_instr(&c.try_into().unwrap()).unwrap();
            let mut t = String::new();
            listing::instr_text(&mut t, i, &ins).unwrap();
            *ops.entry(t.split_whitespace().nth(5).unwrap_or("").split('.').next().unwrap().to_string()).or_default() += 1;
            lst += &(t + "\n");
        }
        println!("COUNT {n} instr={} temps={} const_dw={} ops={ops:?}", sh.usc_dw / 4, sh.temps, sh.viv.const_count);
        std::fs::write(format!("build/terrain-lst/{n}.lst"), lst).unwrap();
    }
}
```

`etna_count.py` (`python3 -I etna_count.py <etna_shc> tools/vivante <shader dir> [hex dir]`):

```python
import os, sys
os.environ["ETNA_SHC"] = sys.argv[1]
sys.path.insert(0, sys.argv[2])
import glsl2vivante as g

d = sys.argv[3]
for n in sorted(os.listdir(d)):
    t = open(os.path.join(d, n)).read()
    # glsl2vivante's ES 1.00 reader runs no preprocessor: resolve the macros
    # (GL_ES defined, GL_FRAGMENT_PRECISION_HIGH not)
    t = t.split("#endif\n", 2)[2] if t.startswith("#if defined") else t
    t = t.replace("#ifdef GL_ES\nprecision mediump float;\n#endif\n", "precision mediump float;\n")
    t = t.replace("HIGHP_OR_DEFAULT ", "").replace("MEDIUMP_OR_DEFAULT ", "mediump ")
    js, err = g.compile_one(os.path.join(d, n), t)
    if js is None:
        print("ETNA", n, "FAIL", err)
        continue
    print("ETNA %-16s instr=%d temps=%d uniform_dw=%d" % (n, js["code_size"] // 4, js["num_temps"], len(js["uniforms"])))
    if len(sys.argv) > 4:   # the words, for yakogl-tools etna-dis (little-endian dwords)
        open(os.path.join(sys.argv[4], n + ".hex"), "w").write(" ".join(js["code"]))
```

What it printed (2026-11-07, this host):

```text
COUNT bloom_h.frag instr=42 temps=8 const_dw=16 ops={add 12, mov 12, mul 12, rcp 1, texld 5}
COUNT bloom_v.frag instr=47 temps=8 const_dw=16 ops={add 12, mov 17, mul 12, rcp 1, texld 5}
COUNT luminance.frag instr=7 temps=4 const_dw=4
COUNT normalmap.frag instr=25 temps=5 const_dw=4
COUNT overlay.frag instr=2 temps=2 const_dw=4
COUNT quad.vert instr=7 temps=3 const_dw=4
COUNT terrain.frag instr=148 temps=19 const_dw=56 ops={add 20, branch 6, cmp 13, dp3 8, exp 1, imullo0 5, load 5, log 1, mad 6, mov 33, mul 24, rcp 1, rsq 5, select 14, sqrt 1, texld 5}
COUNT terrain.vert instr=56 temps=9 const_dw=52 ops={add 3, dp3 3, mad 16, mov 17, mul 12, rsq 3, texldl 2}
COUNT tilt_h.frag instr=66 temps=12 const_dw=24 ops={add 20, mov 16, mul 20, rcp 1, texld 9}
COUNT tilt_v.frag instr=75 temps=12 const_dw=24 ops={add 20, mov 25, mul 20, rcp 1, texld 9}
ETNA bloom_h.frag     instr=24 temps=5 uniform_dw=8     (add 4, mad 4, mov 9, mul 2, texld 5)
ETNA bloom_v.frag     instr=24 temps=5 uniform_dw=8
ETNA luminance.frag   instr=3 temps=3 uniform_dw=4      (dp3 1, mov 1, texld 1)
ETNA normalmap.frag   instr=21 temps=4 uniform_dw=4     (add 4, dp3 1, mad 1, mov 6, mul 3, rcp 2, rsq 1, texld 3)
ETNA overlay.frag     instr=2 temps=2 uniform_dw=4
ETNA quad.vert        instr=4 temps=2 uniform_dw=4      (mad 2, mov 2)
ETNA terrain.frag     instr=72 temps=11 uniform_dw=52   (add 5, cmp 1, dp3 7, exp 1, log 1, mad 10, mov 12, mul 19, rcp 1, rsq 5, select 4, sqrt 1, texld 5)
ETNA terrain.vert     instr=39 temps=9 uniform_dw=52    (add 1, dp3 3, mad 18, mov 6, mul 6, rsq 3, texldl 2)
ETNA tilt_h.frag      instr=46 temps=5 uniform_dw=12    (add 9, mad 8, mov 17, mul 3, texld 9)
ETNA tilt_v.frag      instr=46 temps=5 uniform_dw=12
```
