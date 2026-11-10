<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: shadow's and refract's shaders, our instruction count against Mesa etnaviv's

Plan #328, item sr-shaders-count (issues #1204, #1106). Once the CPU depth
resolve is gone, the frames of glmark2 `shadow` and `refract` are GPU work
(`gc-slow-shadow-refract.md`). This note counts every shader the two scenes
draw with, in our compiler and in etnaviv's, finds where the differences come
from in our compiler, and ranks the changes with an estimate in ms a frame.

Host only. Nothing ran on a board, and no compiler or backend file was
changed. Measured 2026-11-07 on main `723a75c8`. Code is cited as `path:line`
at that commit.

## Answer in short

1. **The two compilers differ by 2 to 2.2x on the larger shaders**: ours is 86
   instructions against etnaviv's 48 for refract's fragment shader, and 23
   against 14 for the lit horse's vertex shader. The smaller ones are 1 to 5
   instructions apart. Texture look-ups are equal (refract 3, shadow's ground
   1). Temporaries are equal except refract's fragment shader: 12 against 6.
2. **That is worth about 1 to 2 ms a frame in shadow and 3 to 6 ms in
   refract**, at the documented 0.6 ns an instruction a pixel or vertex
   (`gc-slow-terrain.md`). Against GPU estimates of 10 to 25 ms (shadow) and
   15 to 40 ms (refract) and a board frame of 500 ms or more, the shaders are
   not what makes these scenes slow. Do them after the CPU depth resolve and
   the tiled targets; none of them is a first fix.
3. **Five causes explain all of the difference**, each shown alone by a
   one-statement shader below: no constant folding beyond a literal's
   reciprocal; no `mad` contraction and every negate a `mov`; a scalar
   transcendental emitted once per lane; a `vec4(v3, 1.0)` constructor that
   stays two copies its readers cannot see through; and `refract()` computed
   at vector width.
4. **There is no shadow-compare emulation to count.** shadow's fragment shader
   samples the depth texture with a plain `texture2D` and compares in the
   shader (`light_distance < sc_perspective.z`), so the compare is 4
   instructions of the ordinary kind (`cmp.lt`, `and.s32`, `select.nz`). Our
   compiler refuses `sampler2DShadow` by name (`gc-shadow-compare.md`) and
   neither scene uses one.

## What the two scenes draw with

glmark2 `22c527cb`, the deps cache's `glmark2-src`. The texts are the ones
glmark2 passes to `glShaderSource` after its own preprocessing, captured by
the host harness (below). Four programs in all, shared by the passes of
`gc-slow-shadow-refract.md`:

| program | scene and pass | glmark2 files | draws |
|---|---|---|---|
| depth | shadow K2 (1600x1200, the horse's depth), refract K2 (the bunny's depth, written as colour) | `depth.vert`, `depth.frag` | horse 21,516 indices; bunny 208,998 indices |
| ground | shadow K3 | `shadow.vert`, `shadow.frag` | 1 quad, 4 vertices, samples the depth texture |
| horse (lit) | shadow K3 | `light-basic.vert` (with texcoord), `light-basic.frag` | 21,516 indices |
| refract | refract K13 | `light-refract.vert`, `light-refract.frag` | 208,998 indices, 3 textures |

The back end also draws with three internal programs: the clear quad
(`clear_vs`, `clear_ps`) in every kick that clears, and `mipgen_ps` for the 10
levels of refract's mipmap chain.

The corpus file names are `shadow-37f46ae8961221f2.vert` and
`shadow-928e172a68b7f175.frag` (depth), `shadow-b9d18073e5979540.vert` and
`shadow-cf4bf899d84af04d.frag` (ground), `shadow-6964fcf6aaac9739.vert` and
`shadow-474800a67afed087.frag` (horse), `refract-7215f4f5052a383c.vert` and
`refract-3fae89c9ab969ea3.frag` (refract). (`index.txt` lists depth under
shadow and refract both.)

## The counts

Instructions are 128-bit program words (`code_size` in dwords, divided by 4);
temps are the program's temporary registers. "ours" is the online compiler of the
`gpu-vivante` build, "etna" is `etna_shc` (Mesa 26.2.1's own compiler and
hardware database for the GC7000UL). Both are compiled without the link, so
neither has the draw-time fix-ups.

| shader | stage | ours | etna | ours temps | etna temps | patch (temps) | per vertex or pixel |
|---|---|---:|---:|---:|---:|---:|---|
| depth | vertex | 6 | 4 | 3 | 3 | 5 (3) | 21,516 / 208,998 indices |
| depth | fragment | 3 | 2 | 3 | 2 | 3 (3) | 0.3 to 0.8 Mpx at 1600x1200 |
| ground | vertex | 12 | 7 | 4 | 4 | 11 (5) | 4 vertices |
| ground | fragment | 16 | 11 | 5 | 4 | 13 (5) | at most 0.48 Mpx; 1 `texld` in both |
| horse | vertex | 23 | 14 | 5 | 4 | 21 (5) | 21,516 indices |
| horse | fragment | 1 | 1 | 2 | 2 | 1 (2) | a `nop`: the colour is the varying's register |
| refract | vertex | 23 | 19 | 5 | 5 | 21 (5) | 208,998 indices |
| refract | fragment | 86 | 48 | 12 | 6 | 71 (12) | 0.1 to 0.2 Mpx; 3 `texld` in both |
| clear (internal) | vertex | 1 | 1 | 1 | 1 | 1 | `nop`, equal (`tests/internal_programs.rs` holds this word for word) |
| clear (internal) | fragment | 1 | 1 | 2 | 2 | 1 | one `mov` from the constant |
| mipgen (internal) | fragment | 4 | 2 | 2 | 2 | 4 | 10 tiny kicks; see below |

Totals of the eight application shaders: ours 170, etna 105 (1.6x). The
"patch" column is the code of "What to do: sr-compiler-fix" below, measured
on the host: 146, landed by item x2-tiled-probe-run ("Landed" at the end).
`tests/gc_sr_shader_count.rs` pins the "patch" column now (it reads the
corpus of step 1 below and prints a SKIP line without it) and the
micro-shaders, with etna's and the "ours" counts (before the patch) in its
comments. Recounted at main `51fc47e6`: unchanged from `723a75c8`, because
noise-compiler-fix committed no code. Shadow's
frame draws 6+3 (depth), 12+16 (ground) and 23+1 (horse); refract's draws 6+3
(depth) and 23+86 (refract).

`mipgen_ps` is not a like-for-like pair. Ours includes the two instructions
that undo `gl_FragCoord.y`'s flip (`mov` and `mad` with the per-draw
`u_flip`, `tools/shaders/internal/es/mipgen.frag`); `etna_shc` compiles
without the state-dependent fix-up. Without the flip, ours is 2 as well.

**Shadow-compare emulation: 0 instructions, in both compilers.** See answer 4.
The compare in the ground shader is the shader's own, and it costs the same
in both: `cmp.lt`, `cmp.lt`, `and.s32`, `select.nz.u32` (ours has one more
`mov` that materialises the 0.5, below).

## The commands

Everything below runs from the repository root.

```sh
# 1. The shader texts: the host harness writes the corpus (GPL text, in build/, not committed).
GM2_WRITE=0 GM2_FRAMES=2 GM2_ENTRIES='^(shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-host vivante
#   -> build/glmark2-host/shaders/{shadow,refract}-<hash>.{vert,frag}.glsl, 8 distinct texts
#   (GLMARK2_SRC: the deps cache's glmark2-src at 22c527cb)

# 2. etnaviv's count: the Mesa tree of tools/vivante/README.md ("Set up once"),
#    built in a private copy of the Mesa 26.2.1 tree (etna_shc sha256 6774c4e1...,
#    glslangValidator 16.2.0). glsl2vivante.py picks the stage from the extension,
#    so copy a corpus file x-<hash>.vert.glsl to x-<hash>.vert.
ETNA_SHC=<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc python3 -I - <<'EOF'
import sys; sys.path.insert(0, "tools/vivante")
import glsl2vivante as g
for p in sys.argv[1:]:   # the 8 files
    js, err = g.compile_one(p)
    print(p, err or "%d instr, %d temps" % (js["code_size"] // 4, js["num_temps"]))
EOF
#    The disassembly is the part of etna_shc's stdout after the JSON
#    (`etna_shc vert|frag <spv>`, as compile_one runs it).

# 3. Our count and listing: the online compiler of the GC build, through the library's own
#    entry point. A throw-away integration test (not committed) compiles each file and prints
#    ShcShader::usc_dw / 4, temps, and listing::body_text of each decoded instruction:
SR_DIR=build/sr-shaders SR_FILES=<comma-separated file names> \
  cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
  --test zz_sr_count -- --nocapture
```

The throw-away test (`tests/zz_sr_count.rs`, deleted after the run) is:

```rust
#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::{decode::decode_instr, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
#[test]
fn count() {
    let (dir, files) = (std::env::var("SR_DIR").unwrap(), std::env::var("SR_FILES").unwrap());
    for f in files.split(',') {
        let src = format!("{}\0", std::fs::read_to_string(format!("{dir}/{f}")).unwrap());
        let stage = if f.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let (mut sh, mut log) = (ShcShader::ZERO, [0 as c_char; 1024]);
        let c = yakogl::shaderc::online::SHC_ONLINE.compile.unwrap();
        assert_eq!(unsafe { c(stage, src.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) }, 0);
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        println!("== {f}: {} instr, {} temps", sh.usc_dw / 4, sh.temps);
        for (i, w) in dw.as_chunks::<4>().0.iter().enumerate() {
            let mut s = String::new();
            listing::body_text(&mut s, &decode_instr(w).unwrap()).unwrap();
            println!("{i:3}: {s}");
        }
    }
}
```

The internal programs were counted the same way from
`tools/shaders/internal/es/{quad.vert,clear.frag,mipgen.frag}`.

## Where the differences come from

Each cause below is shown alone by a one-statement shader (`precision
mediump float;` and the declarations, as in the corpus), compiled both ways.

| # | shader | ours | etna |
|---|---|---:|---:|
| m1 | `varying vec4 a; ... = a / a.w;` | 5 (4 `rcp`, 1 `mul`) | 2 (1 `rcp`, 1 `mul`) |
| m1b | `vec4(a.xyz / a.w, 1.0)` | 6 | 3 |
| m11 | `vec4(a.xyz * (1.0/1.2), 1.0)` | 4 (`mul 1.0, c`; the reciprocal is folded) | 2 |
| m12 | `vec4(a.xyz * (1.2*1.2), 1.0)` | 4 | 2 |
| m9 | `normalize(a.xyz/a.w - L.xyz/L.w)`, `L` a `const vec4` | 17 | 7 |
| m13 | `t = a.xyz*b.x + b; vec4(t*0.5+0.5, 1.0)` | 6 (2 `mul`, 2 `add`) | 3 (3 `mad`/`mov`) |
| m4 | `gl_Position = M * vec4(position, 1.0)` | 6 | 4 |
| m5 | `vec4(refract(a, b, 1.2), 1.0)` | 20 | 10 |
| m8 | `vec4(vec3(sqrt(1.0 - k*k)), 1.0)`, `k = dot(a,b)` | 8 | 4 |

### C1. No constant folding and no `mad`

Compare m11, m12, m13 and the refract rows below.

* **Folding.** `Lower::reciprocal` folds `1/x` when `x` is a direct float
  literal (`src/shaderc/online/lower.rs:5465-5476`), and nothing else folds:
  `alu_sized` pushes every operation as it comes (`lower.rs:7251-7255`) and
  the GC selection has no folder (`select.rs` never calls
  `fold::fold_float`; only `powervr/select.rs:3865` and `v3d/prepare.rs:729`
  do). So `1.0/1.2` becomes a `mul 1.0, u0.x` (m11), `1.2*1.2` a `mul u0.x,
  u0.x` (m12), and a `const vec4` light position divided by its `w` a chain
  of four `rcp` and two `mul` of constants (refract's fragment shader, 11
  instructions; the horse's vertex shader normalises the constant light
  direction at run time: `dp3`, `rsq`, `mul`, 3 instructions). `etna_shc`
  turns all of these into immediates (`-1.44`, `0.8333`, `-0.6944`, `0.832`,
  `0.5547` in the listings).
* **Negate.** `Fneg` is a source modifier on this ISA, and the lowering
  says so (`vivante/builtins.rs:29-34`: "cost no instruction of their own
  when their consumer can take the modifier"). But selection emits `mov d,
  -s` for it (`builtins.rs:228-232`), and `propagate_copies` declines any
  copy that carries a negate or absolute (`select.rs:3448`, the
  `neg: false, abs: false` pattern) or writes a partial mask (`:3436`). So
  `a - b` is `mov t, -b; add a, t` and `1.0 - x*x` is `mul; mov neg; add`.
  Refract's fragment shader has 8 such `mov`; etnaviv has none (it writes
  `mad t, -t1.w, t1.w, 1.0`).
* **No `mad` contraction.** `Ffma` is produced only for the matrix times
  vector products and `dot` expansions (`lower.rs:6140`, `:6587`), never for
  `a*b + c`. `etna_shc` contracts them (`mad`). m13: 6 against 3.

### C2. A scalar transcendental is emitted once per lane

`select.rs:1903-1915`: "A scalar operation computes ONE value ... so it is
emitted once per lane, each reading its own lane; the lanes nobody reads are
deleted afterwards". For `a / a.w` all four lanes read the same `t1.w`, so the
four `rcp t2.x/y/z/w, t1.wwww` could be one `rcp t2.xyzw, t1.wwww`: the
hardware writes the scalar to every lane of the mask, and etnaviv does exactly
that (m1: 2 against 5; `rcp t1.___w` and a `.wwww` read). Ground fragment: 3
`rcp` for 1; refract fragment: 2 for 1 and 4 for 1 (the `vec3 / float`
divisions of `MapCoord` and `vertex_position`, which also widens the divisor to
four lanes first: `lower.rs:5727-5733` `reciprocal(rv, want)` after
`operand_at(rhs, want)`).

### C3. The `vec4(v3, 1.0)` constructor stays two copies

m4 (`M * vec4(position, 1.0)`): ours `mov t1.xyz_, t0.xyzx; mov t1.___w, 1.0`
and then `mul t0, u3, t1.wwww` (a multiply by a lane that is the constant
1.0), against etnaviv's `mov t1, u0; mad ...` with no copy and no multiply by
the constant. The copies stay because the vector is two masked writes of one
register: `propagate_copies` skips partial masks (`select.rs:3436`), and
`fold_constant_reads` folds only a register written once
(`select.rs:3617-3620`, "another write of `d`"), so the `w` lane's
constant is never seen by the reader. A constructor with a constant 0 lane
(the ground's `vec4(position, 0.0, 1.0)`) also keeps its `mad` of the zero
lane: 3 `mov` and 2 extra `mad` over two matrices (ground vertex 12 against
7). The same pattern at an output (`mov t1.___w, 1.0; mov t1, t3.xyzw`) is the
extra final copy in depth, ground and horse fragments and the horse's vertex
shader; `alloc` places outputs but not through two masked writes.

### C4. `refract()` computes its scalars at vector width

`builtin_call.rs:1162-1176` splats `eta` and `dot(N, I)` to the vector width
and does the whole of `k = 1 - eta^2*(1 - d^2)` and `sqrt(k)` on that width.
Vector arithmetic costs one instruction either way, but `sqrt` is scalar, so
the splat costs one `sqrt` per lane (3 for a `vec3`: m5 has 3 `sqrt`, etnaviv
1). Refract's fragment shader has two `refract()` calls: 3 and 2 `sqrt`. The
constant `eta` and the `k < 0` test also stay (`cmp.lt` and `select`, 2
instructions per call): etnaviv's NIR proves `k >= 1 - eta^2 > 0` for the
second call (`eta = 1/1.2`) and drops the test there.

### C5. Temporaries (refract's fragment shader: 12 against 6)

The sum of the above: each unfolded constant, negated copy and per-lane result
holds a register until its reader. Fewer temporaries raise the threads in
flight on the GC7000; whether 12 limits refract's three dependent `texld` was
not measured (nothing here ran on a board).

### Per shader: which causes the difference is made of

| shader | ours - etna | made of |
|---|---:|---|
| depth vertex | 2 | C3: the two copies of `vec4(position, 1.0)` |
| depth fragment | 1 | C3: the output copy |
| ground vertex | 5 | C3: 3 copies of `vec4(position, 0.0, 1.0)` and 2 `mad` of the zero lane |
| ground fragment | 5 | C2: 2 `rcp`; C3: the lane copy `mov t3.w, t3.z` and the final copy; 1 `mov` that materialises 0.5 for the `select` (its second constant operand) |
| horse vertex | 9 | C3: 2 copies of `vec4(normal, 1.0)`, 2 of `vec4(position, 1.0)` and the output copy: 5; C1: the constant light direction 3 and a multiply by 1.0 (`diffuse * MaterialDiffuse.rgb`) 1 |
| refract vertex | 4 | C3: 2 copies each of `vec4(position, 1.0)` and `vec4(normal, 1.0)` |
| refract fragment | 38 | C1 constant folding 11 (`E*E` 1, `1/E` chain 3, constant `rcp` 4, `lightSpecular * matSpecular`, the light direction's `xyz * rcp(w)` and a negated constant 3); C2 lane merge 4; C4 `refract` 3; C1 negate and `mad` contraction about 20 (8 negate `mov`, the rest `mul`+`add` pairs) |

The refract fragment line is a reading of the two listings, checked against
m1 to m13; the first three parts count exactly, the last is the remainder.

## Ranked compiler changes and their estimate

The estimate is `saved instructions x covered pixels (or vertices) x 0.6 ns`,
with the covered counts of `gc-slow-shadow-refract.md` (horse and bunny depth
0.3 to 0.8 Mpx, refract's pass 0.1 to 0.2 Mpx, the ground at most 0.48 Mpx,
vertices at most the index count: 21,516 and 208,998). It assumes the
shaders' instruction count scales their time, which holds only above the
3.9 ns a pixel floor; treat each figure as an upper bound.

| rank | change | where | saves (shadow / refract) | ms a frame (shadow / refract) |
|---:|---|---|---|---|
| 1 | `mad` contraction of `a*b + c`, and Fneg/Fabs folded into the reader's source modifier (copy propagation takes a negated source; partial-mask copies) | `select.rs:3436-3448`, a contraction pass before `legalise_operands` (`select.rs:3749`) or in `lower.rs:7251` | 0 / ~20 | 0 / 1.2 to 2.4 |
| 2 | constant folding of `Fadd/Fmul/Ffma/Frcp/Frsq/Fsqrt/Fneg` on constants, with `fold::fold_float` (`fold.rs:24`) | `lower.rs:7251-7255` `alu_sized`, or the GC `select_blocks` (`select.rs:3999`) | 4 vertex (horse) / 11 | 0.05 / 0.7 to 1.3 |
| 3 | the constructor's lanes visible to readers (per-lane constants and sources through `vec4(v3, w)`) | `select.rs:3436`, `:3617`; `lower.rs:4023` `constructor` | vertex 7 (depth 2, horse 5), fragment 3 (depth 1, ground 2) / vertex 6 (depth 2, refract 4), fragment 1 | 0.6 to 1.2 / 0.9 to 1.2 |
| 4 | one scalar instruction with a lane mask when the lanes read one source | `select.rs:1903-1915` | 2 / 4 | 0.3 to 0.6 / 0.24 to 0.5 |
| 5 | `refract()` with scalar `k` (compute at width 1, splat at the end) | `builtin_call.rs:1162-1176` | 0 / 3 | 0 / 0.2 to 0.4 |

The ms column adds up to about 0.9 to 1.8 (shadow) and 3.2 to 5.8 (refract);
rows 1 to 3 together are most of it. The same rows
help every other shader the GC compiles. The PowerVR side already folds
(`powervr/select.rs:3865`); a change in the shared `lower.rs` would move its
and the V3D's output too, so a GC-only change belongs in `select.rs`.
`tests/internal_programs.rs` holds the internal programs word for word and
must stay green.

Nothing here replaces the chain of `gc-slow-shadow-refract.md`: the resolve,
the cache maintenance, the waits and the clears are larger by an order of
magnitude.

## What to do: sr-compiler-fix (ranks 1, 3 and 4 inside the GC code generator)

Plan #328, item sr-compiler-fix, at main `51fc47e6`. The item could edit only
`src/shaderc/online/vivante/`, not the shared compiler files (`lower.rs`,
`select.rs`, `fold.rs`, `codegen.rs`, ...) or `tests/fixtures/` and
`tests/reference/`, and had to stop if a pinned fixture moved. **No compiler
code was committed.** The part of ranks 1, 3 and 4 that can live in the code
generator was written and measured on the host. It shortens the GC
offline-table programs too (11.5% of their code bytes), so it moves the
shared pin
`tests/fixtures/precompile_counts.txt`, and the item stopped there, as
noise-compiler-fix did (`gc-terrain-shaders.md`, "What to do"). The patch
stood at the end of this note until item x2-tiled-probe-run landed it
("Landed" below).

**The two passes** (a new `vivante/peephole.rs`, called from
`alloc::eliminate_dead`):

1. **Copy fold** (rank 1's negate half and rank 3's lane half), at the start
   of `eliminate_dead`. Each `mov t.mask, s` of a temporary or an input, with
   or without a negate or an absolute, is read through by the readers after
   it: the reader's source becomes `s` with the swizzles composed and the
   modifiers combined (the magnitude before the negate, as the hardware
   applies them). Only in straight-line code (no branch target and no branch
   in between, at most 512 instructions on), only while every lane the reader
   consumes (`consumed_lanes`, over its whole write mask) was written by the
   copy and not since, and only until the source is written. A copy with a
   modifier goes only into F32 `mov`, `add`, `mul`, `mad`, `dp2/3/4`, `rcp`,
   `rsq`, `sqrt`, `floor`, `ceil`, `frc` (not `select` or `cmp`, which pass
   bits). The copy, once unread, is deleted by the liveness that follows. This
   is noise-compiler-fix's negate fold generalised to partial-mask plain
   copies: `vec4(v3, 1.0)`'s `mov t.xyz_, s.xyzx` disappears for the readers
   of x, y and z.
2. **Scalar lane merge** (rank 4), at the end of `eliminate_dead`, over the
   survivors. Adjacent copies of one `rcp`, `rsq`, `sqrt` or `exp` that write
   disjoint lanes of one temporary from the same source lane (same register,
   modifiers and first swizzle lane) become one instruction with the union of
   the masks, as etnaviv writes `rcp t1.___w`. It relies on the scalar unit
   writing its one value to every lane of the mask, which `select.rs` states
   from a board observation (`sin` of a vec4 emitted once) and the interpreter
   of `tests/common` models; `sin`, `cos` and `log` (a pair of lanes on this
   core) and the two-source `div` and `imullo0` are left alone. Not when a
   branch goes to the later copy, nor when the copy reads the register it
   writes.

| measured on the host (main `51fc47e6`) | without | fold only | merge only | both |
|---|---:|---:|---:|---:|
| the eight application shaders, instructions | 170 | 155 | 161 | **146** (etna 105) |
| refract fragment | 86 | 78 | 79 | **71** (etna 48) |
| ground fragment | 16 | 15 | 14 | 13 (etna 11) |
| depth vertex, ground vertex, horse vertex, refract vertex | 6, 12, 23, 23 | 5, 11, 21, 21 | 6, 12, 23, 23 | 5, 11, 21, 21 |
| temporaries | | ground vertex 4 to 5, the rest equal | equal | ground vertex 4 to 5 |
| micro m1, m1b, m9, m5, m8, m4, `a - b` | 5, 6, 17, 20, 8, 6, 2 | 5, 6, 16, 17, 7, 5, 1 | 2, 4, 11, 20, 8, 6, 2 | 2, 4, 10, 17, 7, 5, 1 |
| `precompile_counts.txt` `gc7000 es2=0 app=all` code_bytes | 129376 | | 129088 | 114448 (**moves**) |
| `precompile_counts.txt` `gc7000 es2=1 app=all` code_bytes | 109680 | | | 95344 (**moves**) |
| `src/vivante/internal.rs` (`--internal --verify`) | | | | unchanged |

With `PRECOMPILE_PIN_WRITE=1` (written, read, then restored), every gc7000
line of the fixture moves and the PowerVR lines do not:

| fixture line | rows | code_bytes now | with the patch |
|---|---:|---:|---:|
| `gc7000 es2=0 app=all` | 226 | 129376 | 114448 |
| `gc7000 es2=0 app=femtovg` | 36 | 48304 | 38720 |
| `gc7000 es2=0 app=gl_test_es100` | 111 | 32512 | 29248 |
| `gc7000 es2=1 app=all` | 183 | 109680 | 95344 |
| `gc7000 es2=1 app=femtovg` | 36 | 48304 | 38720 |
| `gc7000 es2=1 app=gl_test_es100` | 82 | 25792 | 22960 |

m13 stays at 6: the patch has no `mad` contraction (below).

**Correctness, with both passes:** the whole GC7000 host suite
(`cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --no-fail-fast`,
140 test binaries) passes except three pins that the shorter code moves:

- `tests/backend_differential.rs`, the value check (the IR evaluator against
  the interpreter running the encoded program, the offline sources, the dEQP
  sources and the long straight-line program): 9 of 9 pass.
- `tests/internal_programs.rs`: passes (the clear shaders are etna's word for
  word).
- the five unit tests of `peephole.rs` (a negated copy read through an add's
  modifier with the swizzles composed; none into a `select` nor past a write
  of the source; a partial copy folded only for its own lanes; four `rcp` of
  one lane become one; different lanes, a `sin`, and a self-read stay):
  pass.
- moved: `tests/fixtures/precompile_counts.txt` (above);
  `tests/lane_move_equivalence.rs`'s population pin, 103 to 102 programs
  (the `report-es2` build's 87 not measured); and the unit test
  `alloc::loop_and_io_tests::a_seed_whose_lanes_are_replaced_before_anything_needs_them_is_dead`,
  whose copy at 2 is now read through by 3 and dies with the seed (removed 1
  to 2; the patch updates it).

At 0.6 ns an instruction a pixel (above), the 15 instructions off refract's
fragment shader are 0.9 to 1.8 ms of refract's frame, and the shadow side's 7
about 0.3 to 0.6 ms: the estimate of ranks 1, 3 and 4 holds.

**What landing it needs, in the shared files, and nothing else:**

- `tests/fixtures/precompile_counts.txt`: every `gc7000` line re-pinned with
  `PRECOMPILE_PIN_WRITE=1 tools/precompile/prove.sh gpu-vivante` and
  `gpu-vivante,report-es2`, to the six values of the table above;
- `tests/lane_move_equivalence.rs`: the population pin, 103 to 102 (and the
  `report-es2` figure re-derived);
- `tests/gc_sr_shader_count.rs`: the pins moved to its "patch" comments
  (146 in all);
- nothing in `src/vivante/internal.rs` or `tests/reference/`.

**What the code generator cannot do alone:**

- **`mad` contraction** (rank 1's other half; m13, and most of refract's
  remaining `mul`+`add` pairs): needs the `invariant` outputs handed to the
  GC passes from `codegen.rs` `assemble_from_ir_blocks` (or `select.rs`), as
  `gc-terrain-shaders.md` says; the pairing itself can then live in
  `vivante/`.
- **Constant folding** (rank 2: `1.0/1.2`, `1.2*1.2`, the `const vec4`
  light's `xyz * rcp(w)`, 11 in refract's fragment shader and 3 plus 1 in the
  horse's vertex shader): the constants are constant-file reads by the time
  the code generator sees them; folding belongs in `lower.rs` `alu_sized` or
  the GC `select_blocks` with `fold::fold_float`.
- **A negated uniform** (`mov t, -uN`): the fold would have to check the
  one-constant-register-an-instruction rule, private to `select.rs`
  `within_caps`.
- **The constructor's constant lane** (rank 3's other half: `mul t0, u3,
  t1.wwww` with `t1.w = 1.0`, the zero lane's `mad`): needs the lane's
  constant visible to the reader, `select.rs` `fold_constant_reads` (only a
  register written once).
- **`refract()` at scalar width** (rank 5): `builtin_call.rs`.
## Landed: x2-tiled-probe-run (the patch, re-pinned)

Plan #328, item x2-tiled-probe-run, at main `4c19d79f`. sr-compiler-fix's
patch applied as is (`git apply`, no rebase needed: nothing of noise-compiler-fix,
sr-compiler-fix or terrain-compiler-fix had touched `vivante/`). It is now
`src/shaderc/online/vivante/peephole.rs` and the two calls in
`alloc::eliminate_dead`; the commit that lands it is the diff that stood
here, with one fix: the copy fold also ends a copy of an input at a write
of that register to a fixed destination. The fragment stage writes
`gl_FragDepth` to r0.z, where `gl_FragCoord` arrives, and the patch as
written read selection's copy of r0 through past that write
(`tests/es3_fragdepth_gc.rs`, which the patch's own suite run at `51fc47e6`
did not yet hold, failed). It moves no count below. Measured
again on the host after it, every figure the same as the table above:

| measured at `4c19d79f` | before | with the patch |
|---|---:|---:|
| the eight application shaders, instructions (`tests/gc_sr_shader_count.rs`, the host harness's corpus) | 170 | **146** (etna 105) |
| depth vertex, depth fragment, ground vertex, ground fragment | 6, 3, 12, 16 | 5, 3, 11, 13 |
| horse vertex, horse fragment, refract vertex, refract fragment | 23, 1, 23, 86 | 21, 1, 21, 71 |
| temporaries | | ground vertex 4 to 5, the rest equal |
| micro m1, m1b, m13, m9, m5, m8, m4, `a - b` | 5, 6, 6, 17, 20, 8, 6, 2 | 2, 4, 6, 10, 17, 7, 5, 1 |
| `precompile_counts.txt` gc7000 lines (all, femtovg, gl_test_es100), es2=0 | 129376, 48304, 32512 | 114448, 38720, 29248 |
| the same, es2=1 | 109680, 48304, 25792 | 95344, 38720, 22960 |
| `tests/lane_move_equivalence.rs` population (default, `report-es2`) | 103, 87 | 102, 86 |
| `src/vivante/internal.rs`, the powervr lines of the fixture, `tests/reference/` | | unchanged |

The `report-es2` population, not measured by sr-compiler-fix, is 86 (one
program fewer moves a lane, as in the default build). terrain's ten shaders
moved as `gc-terrain-shaders-rest.md` predicted (475 to 428); tilt_v's extra
nine are explained there ("What to do", item x2-tiled-probe-run).
