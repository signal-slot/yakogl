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

| shader | stage | ours | etna | ours temps | etna temps | per vertex or pixel |
|---|---|---:|---:|---:|---:|---|
| depth | vertex | 6 | 4 | 3 | 3 | 21,516 / 208,998 indices |
| depth | fragment | 3 | 2 | 3 | 2 | 0.3 to 0.8 Mpx at 1600x1200 |
| ground | vertex | 12 | 7 | 4 | 4 | 4 vertices |
| ground | fragment | 16 | 11 | 5 | 4 | at most 0.48 Mpx; 1 `texld` in both |
| horse | vertex | 23 | 14 | 5 | 4 | 21,516 indices |
| horse | fragment | 1 | 1 | 2 | 2 | a `nop`: the colour is the varying's register |
| refract | vertex | 23 | 19 | 5 | 5 | 208,998 indices |
| refract | fragment | 86 | 48 | 12 | 6 | 0.1 to 0.2 Mpx; 3 `texld` in both |
| clear (internal) | vertex | 1 | 1 | 1 | 1 | `nop`, equal (`tests/internal_programs.rs` holds this word for word) |
| clear (internal) | fragment | 1 | 1 | 2 | 2 | one `mov` from the constant |
| mipgen (internal) | fragment | 4 | 2 | 2 | 2 | 10 tiny kicks; see below |

Totals of the eight application shaders: ours 170, etna 105 (1.6x). Shadow's
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
