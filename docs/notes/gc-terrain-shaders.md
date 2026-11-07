<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: glmark2 terrain's shaders, our instruction counts against Mesa etnaviv's

Plan #328, item noise-shader-count (issues #1204, #1106; ROADMAP.md stage 2,
"Slow scenes"). It answers `gc-slow-terrain.md` rank 1 (b): "compare the noise
shader's 840 instructions with Mesa etnaviv's count for the same text". Host
only. No compiler code changed, nothing ran on a board. Code is cited at
`98251549`; glmark2 at the pinned `22c527cb` (`$GLMARK2_SRC`); Mesa is
26.2.1 (`VERSION`), the same version `tools/vivante/README.md` names.

## Answer in short

- **Mesa etnaviv compiles the noise shader to 414 instructions; ours emits
  840** (2.03 times). Every other heavy terrain shader is 1.5 to 2 times too:
  `terrain.frag` 148 against 72, `terrain.vert` 56 against 39, the two
  tilt-shift blurs 75 and 66 against 46 and 46.
- Instruction mix is not the cause: the `floor`, `dp3`, `dp4` and `select`
  counts are equal to the instruction. The excess is **five things our
  compiler does not do**: it divides by a constant with a run-time `rcp`
  (60 `rcp`, 76 `mov` of the same constant), it spends one `mov` on every
  negate (66), it never turns `a*b + c` into `mad` (16 `mad` against 67), it
  copies vectors through `mov`s that Mesa folds into the next operand (111),
  and it keeps terrain's one-pass light loop and its uniform-array reads as a
  real loop with run-time indexing (about 45 instructions).
- At the note's GPU cost (0.6 ns an instruction a pixel or vertex,
  `gc-slow-terrain.md`, "The GPU calibration"; 0.8 ns as the upper case) the
  five shaders' excess is **about 57 ms a frame at 0.6 ns and 76 ms at
  0.8 ns** of terrain's 140 to 205 ms of GPU time. The noise pass alone is 17
  to 22 ms of it. Terrain's 250 ms frame would go to about 175 to 195 ms with
  Mesa-sized shaders (a model, not a measurement).
- The gap is large, so it is worth its own issue: "the online GC compiler
  emits about twice Mesa's instructions for the glmark2 terrain shaders"
  (text in the report that comes with this note).

## How it was measured

Nothing here is committed except this note. The inputs and the two small
drivers are scratch files in `$CONNECTED_DEPS_DIR` and are reproduced below.

### The shader texts

The host harness writes the exact text glmark2 passes to `glShaderSource`
(`tools/shaders/glmark2/README.md`):

```sh
GM2_WRITE=0 GM2_ENTRIES='^terrain$' GM2_FRAMES=1 \
    tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" <out> vivante
```

`<out>/shaders/` then holds the terrain scene's eleven texts. The five that
matter (the others are the small normal-map, luminance and overlay passes and
the two bloom blurs):

| role | file | what it is |
|---|---|---|
| noise height map | `terrain-1637be2eef2e8a5e.frag.glsl` | `terrain-noise.frag`, 4 calls of `snoise` (3D simplex noise) |
| terrain, fragment | `terrain-4d266cda76461dc0.frag.glsl` | `terrain.frag`, 1 point light, fog off |
| terrain, vertex | `terrain-18bca803a331552c.vert.glsl` | `terrain.vert`, the height and normal maps in the vertex stage |
| tilt_v blur | `terrain-8c8662245addb598.frag.glsl` | `terrain-blur.frag`, radius 4, 9 texture loads, tilt-shift |
| tilt_h blur | `terrain-bbba1ac2cdc22b1a.frag.glsl` | the same with the horizontal step |

(`terrain-bdfd8dd6...` and `terrain-e043e1e8...`, 5 texture loads, are the
bloom blurs; the default terrain entry uses tilt-shift, not bloom.) The
fragment texts begin with glmark2's precision macros
(`libmatrix/shader-source.cc:493-520`).

### Ours

Our count is `usc_dw / 4` of the online compiler's result, the number the back
end programs (`src/vivante/shader.rs:391`). A scratch integration test
(`tests/zz_scratch_count.rs`, not committed) compiles each text through
`SHC_ONLINE.compile` and prints the count and the listing
(`vivante::listing::instr_text`):

```rust
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::decode::for_each_instr;
use yakogl::shaderc::online::vivante::listing::instr_text;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
// for each text file `p` of a directory:
let stage = if name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
let mut s = std::fs::read_to_string(&p).unwrap();
s.push('\0');
yakogl::shaderc::online::backend::forget_compiled();
let mut out = ShcShader::ZERO;
let mut log = [0 as c_char; 512];
let compile = yakogl::shaderc::online::SHC_ONLINE.compile.unwrap();
let rc = unsafe { compile(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
let code = unsafe { core::slice::from_raw_parts(out.usc, out.usc_dw as usize) };
// eprintln!("{name}: {} instr, {} temps", out.usc_dw / 4, out.temps);
// for_each_instr(code, |i, ins| { instr_text(&mut text, i, ins); text.push('\n'); Ok(()) })
```

```sh
cargo test --features host-test,gpu-vivante,shaderc-online \
    --test zz_scratch_count -- --nocapture
```

Its 840, 56, 148, 75 and 66 equal the counts `gc-slow-terrain.md` traced from
the running draws (840, 56, 148, 75 and 66), so the texts are the ones the
board draws with.

### Mesa

`$CONNECTED_DEPS_DIR/mesa-etna-2621-git` is a new writable tree
(`third_party/` untouched; the shared `mesa-pan` copy untouched):

```sh
git clone --depth 1 --branch mesa-26.2.1 https://gitlab.freedesktop.org/mesa/mesa.git \
    "$CONNECTED_DEPS_DIR/mesa-etna-2621-git"          # checked out da14d65e4499
cd "$CONNECTED_DEPS_DIR/mesa-etna-2621-git"
cp <yakogl>/tools/vivante/etna_shc.c src/gallium/drivers/etnaviv/
# tools/vivante/mesa-etna_shc.patch: its three hunks applied by hand (this session's
# sandbox refused patch and git apply): etna_get_specs made non-static in
# etnaviv_screen.c, its prototype in etnaviv_screen.h, the etna_shc target at the end
# of src/gallium/drivers/etnaviv/meson.build
meson setup build-etnaviv -Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dplatforms= \
    -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dllvm=disabled -Dtools=
meson compile -C build-etnaviv src/gallium/drivers/etnaviv/etna_shc
```

`pip` was refused, and Mesa's hwdb step needs pycparser: a `git clone` of
`eliben/pycparser` sits in `$CONNECTED_DEPS_DIR/pycparser-src`, and two lines
in this private Mesa tree (`src/etnaviv/hwdb/meson.build` and `hwdb.h.py`) put
it on `sys.path`. That changes nothing in the compiler. The binary's sha256 is
`58250454d3a849e7...` (it does not match the `c9845f2b...` of
`tools/vivante/README.md`'s table: a different Mesa checkout and host
compiler; the compiler sources are 26.2.1's in both). glslang is 16.2.0.

The count is `tools/vivante/glsl2vivante.py`'s `compile_one` (ES 1.00 to GLSL
4.60, `glslangValidator -G`, `etna_shc frag|vert`; `code_size / 4`). That
script does not run a preprocessor, so the driver cuts glmark2's macro block
and substitutes the two macros as `GL_ES` with `GL_FRAGMENT_PRECISION_HIGH`
defined would (`MEDIUMP_OR_DEFAULT` to `mediump`, `HIGHP_OR_DEFAULT` to
`highp`):

```python
import os, sys
os.environ["ETNA_SHC"] = "<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc"
sys.path.insert(0, "<yakogl>/tools/vivante")
import glsl2vivante as g
text = open(path).read()
text = text[text.index("#ifdef GL_ES"):].replace("MEDIUMP_OR_DEFAULT", "mediump") \
                                       .replace("HIGHP_OR_DEFAULT", "highp")
js, err = g.compile_one(path, text=text)
print(js["code_size"] // 4, js["num_temps"])
```

```sh
python3 -I mesa_count.py
```

Mesa's words (`js["code"]`) went through the same disassembler as ours
(`for_each_instr` and `instr_text`), so both listings read alike and the
class counts below come from one parser.

## Counts

| shader | ours | Mesa | ours / Mesa | temps ours / Mesa |
|---|---:|---:|---:|---:|
| noise height map (`terrain-noise.frag`) | **840** | **414** | 2.03 | 14 / 12 |
| terrain fragment (`terrain.frag`) | 148 | 72 | 2.06 | 19 / 11 |
| terrain vertex (`terrain.vert`) | 56 | 39 | 1.44 | 9 / 9 |
| tilt_v blur | 75 | 46 | 1.63 | 12 / 5 |
| tilt_h blur | 66 | 46 | 1.43 | 12 / 5 |
| (bloom blur, 5 loads) | 47 and 42 | 24 and 24 | 1.9 | 8 / 5 |

Texture loads (`texld`, `texldl`) are equal everywhere (5, 2, 9, 9): the
excess is all ALU work.

## Noise shader: by instruction class

Ours minus Mesa, from the two listings (the noise shader, 426 in all):

| class | ours | Mesa | difference |
|---|---:|---:|---:|
| `mov` of an immediate | 86 | 1 | +85 |
| `mul` | 151 | 67 | +84 |
| `mov` into part of a register (write mask not full) | 151 | 85 | +66 |
| `mov` that only negates (`mov t, -t`) | 66 | 1 | +65 |
| `rcp` | 60 | 0 | +60 |
| `add` | 129 | 73 | +56 |
| `mov` copying a whole register | 45 | 0 | +45 |
| `cmp.lt` + `not.s32` + `and.u32` against `set.lt` | 24 | 8 | +16 |
| `mad` | 16 | 67 | -51 |
| `floor`, `dp3`, `dp4`, `select.lt`, `select.gt` | 112 | 112 | 0 |

### By construct

**`mod(x, 289.0)` (permute, 12 vec4 and 4 vec3 per shader).** The largest
single item. Ours (listing `047`-`062` of one permute):

```
mov t1.y, 289.0 ; rcp t7.x, t1.yyyy     (four times, once a lane)
mul t7, t5, t7  ; floor t7, t7 ; mov t7, -t7 ; mad t5, 289.0, t7, t5
```

12 instructions for a vec4, 9 for a vec3. Mesa's, for the same `mod`:

```
mul t10, t9, u1.yyyy   ; floor t10, t10 ; mad t9, u1.zzzz, t10, t9
```

3 instructions: `1/289` is a constant, folded at compile time and read from
the constant file, and the negate is a source modifier (`mad t2, -t4, 49.0, t2`).
Over the shader that is 12 x 9 + 4 x 6 = **132 instructions**, and it is the
whole `rcp` count (4 x 12 + 3 x 4 = 60) and about 76 of the 86 immediate
`mov`s (the constant has to be in a register for `rcp`, and is moved again
before each one, `t1.y` being reused by every lane).

The cause is in the front end, `src/shaderc/online/builtin_call.rs:1013-1023`:
`mod` is written as `x - y * floor(x / y)` with `Frcp [y]` on the divisor as
an IR operation emitted unconditionally. The `/` operator does fold a
literal divisor (`lower.rs:5440-5474` `reciprocal`: "dividing by a LITERAL needs
no instruction at all"); `mod` calls `alu(e, Frcp, ...)` directly and never
asks. So `mod(x, 289.0)` costs a run-time reciprocal and `x / 289.0` does not.

**Negate: `a - b`, `-x`, and the `mod`'s `-floor`.** 66 `mov`s that only
negate; 16 of them are in the `mod`s above, the other 50 are `a - b`
(`lower.rs:5712-5717`: `Fneg` then `Fadd`, with the comment "the back end folds
the negate into it") and `-step(...)`, `1.0 - x`. `vivante/builtins.rs:20-33,
222-231` says it plainly: a negate "costs exactly one `mov` when the IR asks for
the value itself"; the fold into the consumer's `SRC_NEG` bit is what is
missing for these. Mesa has one such `mov`.

**`a*b + c` as `mad`.** Ours: 296 `mul` + `add` + `mad`, of which 16 `mad`.
Mesa: 207, of which 67 `mad`. Every `((x * 34.0) + 1.0) * x` is two instructions
where Mesa has one (`mad t10, t9, u2.zzzz, u2.yyyy`). Our `mad` appears only
inside built-in expansions (`mod`, `mix`, `exp`). The IR says why there is no
contraction: `ir.rs:601-608` (`invariant` outputs must not be contracted),
and the PowerVR does contract where the output is not invariant
(`powervr/prepare.rs` `fusion_pairs`). The GC path has no such pass. The
noise shader's output is not `invariant`.

**vec4 packing and swizzles.** 151 partial-mask `mov`s against 85 and 45 whole
copies against none. `vec4(0.0, i1.z, i2.z, 1.0)` is four one-lane `mov`s in
ours (`054`-`057`), three in Mesa (`mov t9.x__w, u2.xxxy`: one `mov` fills two
lanes from a constant vector in the constant file). Swizzled operands
(`b0.xzyw`, `s0.xzyw * sh.xxyy`) are read through the source swizzle in Mesa and
sometimes first copied to a temporary in ours. The whole-register copies are
`mov t7, t4.xxzx`-style.

**`step(a, b)`.** Ours is `cmp.lt` + `not.s32` + `and.u32` (and a `mov`/`add`
pair for `1.0 - g`), Mesa's `set.lt` once (`069`): 8 times, +16.

**Loops.** None in the noise shader. Terrain's fragment shader has one.

### Where the 840 go

840 is 4 x about 210. Mesa's 414 is 4 x about 103. The prologue and the
`gl_FragColor` of the shader are small; the difference scales with the
octaves because `snoise` is inlined four times.

## The other shaders

**`terrain.frag` (148 against 72).** About 45 of the 76 are one cause:
`for (int i = 0; i < MAX_POINT_LIGHTS; i++)` with `MAX_POINT_LIGHTS` 1, and
`pointLightPosition[i]`, `pointLightDistance[i]`, `pointLightColor[i]` reads.
Ours keeps a real loop (4 `branch`, `branch.not.u32`, `cmp.ge.s32`) and reads
each array element through a run-time index: two `cmp.lt.s32` bounds tests, a
`select.nz.u32`, `imullo0.s32`, `load.u32` from the constant block, five times
(`cmp.lt.s32` 11, `select.nz.u32` 11, `add.s32` 7, `imullo0.s32` 5,
`load.u32` 5, branches 6, against none of them in Mesa). Mesa unrolls a
one-pass loop and the index is constant. The unroller that does this exists
(`powervr/unroll.rs`, used by the PowerVR and the V3D); the GC7000 instead
gives a recognised loop its exact count as the guard (`loop_count.rs:1-14`,
`select.rs` `loop_convert`). The rest is the same classes as above (`mul`
+5, `add` +8, `mov` +21, `mad` -4).

**`terrain.vert` (56 against 39).** `mul` +6, `mov` +11, `add` +2, `mad` -2:
contraction and copies, 393 216 vertices a frame.

**Tilt-shift blurs (75 and 66 against 46).** `mul` 20 and `add` 20 where
Mesa has 3 `mul`, 9 `add` and 8 `mad`: the `result = sample * Kernel + ...`
chain is two instructions a tap in ours, `mad`-based in Mesa. Plus one `rcp` of
the constant `abs(1.0 - TiltShift)` (the `stepX` of the tilt) and 5 copy
`mov`s.

## Ranked compiler changes, with the GPU time each is worth

GPU time is `instructions saved x pixels (or vertices) x 0.6 ns`, with 0.8 ns
as the upper case: the model of `gc-slow-terrain.md` ("The GPU calibration"),
which is `imx8mp-frame-floor.md` candidate 2's 3.9 ns a pixel for the floor plus
one instruction a clock on each of two shader cores. Pixel counts: the noise
pass 256 x 256 = 65 536; terrain's fragment pass 800 x 600 at cover 1.0
(`gc-slow-terrain.md` puts it at 0.8 to 1.2); the tilt passes 480 000 each;
the terrain mesh 393 216 vertices. These are a model: no GC time is measured
on a board, and an instruction is not always the same time (a `texld` waits
longer than a `mad`). The attributions overlap a little, so the rows add to
about 52 of the 57 ms of the total excess; the rest is classes that no one
change owns.

| rank | change | where | noise | terrain.frag | tilt (v + h) | terrain.vert | ms a frame at 0.6 / 0.8 ns |
|---:|---|---|---:|---:|---:|---:|---|
| 1 | Contract `a*b + c` into `mad` when no `invariant` output reads it, and fold every `Fneg` and `a - b` into the consumer's `SRC_NEG` (no `mov`) | a GC pass after lowering, like `powervr/prepare.rs` `fusion_pairs`; `vivante/builtins.rs:228-231`, `lower.rs:5712-5717` | about 138 | about 20 | 40 | about 8 | **about 25 / 33** |
| 2 | One-pass (small constant trip count) loops unrolled and constant-index uniform-array reads become plain constant-file reads | `select.rs` `loop_convert`, the shared `powervr/unroll.rs` | 0 | about 45 | 0 | 0 | **about 13 / 17** |
| 3 | vec4 packing and copy propagation: fill two lanes of a constant vector with one `mov`, read swizzles through the source, no whole-register copies | `vivante/alloc.rs` (coalescing of `mov`s, near `:1380`), `lower.rs` constructors | about 90 | about 10 | 9 | about 6 | **about 8 / 11** |
| 4 | Divide and `mod` by a constant: fold `1/c` at compile time, put it in the constant file, no `rcp`, no `mov` of the constant per lane | `builtin_call.rs:1013-1023` (`mod`), `lower.rs:5457-5474` | **132** | 0 | 1 | 0 | **about 5.5 / 7** |
| 5 | `step(a, b)` as `set.lt` | `builtin_call.rs` `step` | 16 | 0 | 0 | 0 | about 0.6 / 0.8 |

For the noise shader alone the order is 1 (138), 4 (132), 3 (90), 5 (16), and
the first two together take it from 840 to about 570; all four to about 420,
which is Mesa's 414. The order by frame time puts 4 (the clearest single
defect, a missed constant fold; the cheapest to write) after the contraction
and the loop, because 1 and 2 also reach terrain's 480 000-pixel passes.

Suggested order of work: 4 first (a few lines, exact, no precision question),
then 1 (the largest, with the `invariant` rule and a host test that an
`invariant` output stays unfused), then 2, then 3.

What to expect: all five together, the noise pass goes from about 33 to 16
ms, terrain's fragment pass from about 43 to 21 ms, the two tilt passes from
about 45 to 31 ms, the mesh's vertex work from about 13 to 9 ms: about 57 ms
a frame (76 at 0.8 ns), or terrain from 250 ms to about 175 to 195 ms. It does
not make terrain reach the panel's rate (`gc-slow-terrain.md`: early depth and
fast clears, the other half of rank 1, are separate).

## What this does not show

- No GPU time of either listing. Mesa's code is shorter, not proven faster:
  the board must run both, which needs a build with the Mesa words
  (`tests/reference/vivante_blobs.rs` carries Mesa's compiled shaders for the
  shaders in `tools/shaders`, not for these glmark2 texts).
- Precision qualifiers are kept in the text Mesa compiles, but the Mesa side
  goes through `glslang -G`, whose `#version 460` does not define `GL_ES`;
  whether etnaviv treats `mediump` differently was not checked. The counts
  agree with ours on the instructions that carry no precision question
  (`floor`, `dp3`, `dp4`, `select`, `texld`).
- Mesa's `etna_shc` is the offline path of `tools/vivante` (SPIR-V through
  `glslang`), not the GL state tracker; it runs the preparation passes
  `tools/vivante/README.md` lists, so the count is what etnaviv's NIR
  compiler does for this text, the compiler `tests/reference/` was made with.
- The attribution of the 426 to classes is exact for the class table and
  approximate for the per-change rows (a `mov` can belong to two causes).
