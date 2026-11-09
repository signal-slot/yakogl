<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): fragment pow, normalize, reflect, length and fract

**Status: design** (plan #336, issue #33). This note changes no code. It says,
for each of the five GLSL built-ins in a fragment shader, which NIR Mesa
26.2.1 lowers it to, which Valhall instructions Mesa emits for a small
representative shader (quoted), which of them `src/shaderc/online/mali/`
already makes and which are missing, which glmark2 shaders each unblocks, and
in what order to implement them.

## How the words were made

**Reference.** Mesa 26.2.1's own GL driver on the drm-shim (the reference arm
of `tools/mali/drm_shim_ref.sh`), compiling each program the way an
application's draw does, through Mesa's GLSL front end. The first version of
this note (7b2d529e) used `pan_shc` instead. `pan_shc` reads SPIR-V and stayed
at fp32, so that version had no fp16 words. Its highp listings are compared
with the shim's below.

**Build** (2026-11-09, on the host this task ran on). The tarball could not
be downloaded on this host (the fetch was not permitted). So the shim was
built from the Mesa 26.2.1 tree already in `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`,
the tree `tools/mali/build_pan_shc.sh` extracts. That script checks the
tarball's sha256 before it extracts, but the tarball is not on this host,
so **the tarball's sha256 was not re-checked here**. What was checked in the
tree:

- `VERSION` reads `26.2.1`;
- the 166 files that `third_party/mesa-26.2.1/UPSTREAM` lists all match
  their sha256 (`sha256sum -c`, 0 mismatches);
- only pan_shc's own files are newer than the tarball's files: the
  `subdir('pan_shc')` line that `tools/mali/mesa-pan_shc.patch` adds to
  `src/panfrost/meson.build`, and `src/panfrost/pan_shc/`. The pan_shc
  executable is `build_by_default: false`, so the shim build does not build
  it.

The build uses the meson options of `drm_shim_ref.sh build`, word for word,
with LLVM 21. It is out of tree, with the build directory and the install
prefix under `$CONNECTED_DEPS_DIR/mesa-shim`
(`$CONNECTED_DEPS_DIR/mesa-shim/build-shim.sh` holds the commands):

    meson setup $CONNECTED_DEPS_DIR/mesa-shim/build $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1 \
      -Dbuildtype=debug -Dgallium-drivers=panfrost -Dvulkan-drivers= -Dtools=drm-shim \
      -Dplatforms= -Dglx=disabled -Degl=enabled -Dgbm=disabled -Dgles1=disabled \
      -Dgles2=enabled -Dopengl=false -Dglvnd=disabled -Dprefix=$CONNECTED_DEPS_DIR/mesa-shim/inst
    ninja -C $CONNECTED_DEPS_DIR/mesa-shim/build install

The result is `$CONNECTED_DEPS_DIR/mesa-shim/inst/lib/x86_64-linux-gnu/libpanfrost_noop_drm_shim.so`,
which `drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim ...` finds. The
draws report `Mali-G610 MC4 (Panfrost)`.

**Shaders.** One vertex shader, `fsv.vert`:

    attribute vec4 a_pos;
    attribute vec4 a_v;
    varying vec4 v;
    void main() { gl_Position = a_pos; v = a_v; }

Seven fragment shaders, each written twice: `fsv_<name>_mediump.frag` and
`fsv_<name>_highp.frag`, 14 in all. Each is
`precision <p> float; varying vec4 v; uniform vec4 u; void main() { <body> }`:

| name | body |
|---|---|
| pow | `gl_FragColor = vec4(pow(v.x, u.y), 0.0, 0.0, 1.0);` |
| normalize | `gl_FragColor = vec4(normalize(v.xyz), 1.0);` |
| reflect | `gl_FragColor = vec4(reflect(v.xyz, u.xyz), 1.0);` |
| length | `gl_FragColor = vec4(length(v.xyz), 0.0, 0.0, 1.0);` |
| fract | `gl_FragColor = vec4(fract(v.x), 0.0, 0.0, 1.0);` |
| nconst | `gl_FragColor = vec4(normalize(vec3(1.0, 2.0, 2.0)) * v.x, 1.0);` |
| lconst | `gl_FragColor = vec4(length(vec3(1.0, 2.0, 2.0)) * v.x, 0.0, 0.0, 1.0);` |

**Per draw** (`$CONNECTED_DEPS_DIR/mesa-shim/fsv/run.sh <checkout>` runs all
14 and writes the shaders and `out/`). The commands are these two, run from
this checkout at 6d360a03:

    tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim fsv.vert fsv_<name>_<p>.frag out/fsv_<name>_<p> --uniform-values
    python3 -I tools/mali/pan_ref_diff.py extract out/fsv_<name>_<p>.trace --draw > out/fsv_<name>_<p>.expect.json

The draw is the default 64x64 triangle draw. Each of the 14 draws printed
`glGetError 0x0`. `--uniform-values` sets `u` to (16, 17, 18, 19), so a pushed
uniform word can be told from a folded constant. In highp the words are
fp32 `41800000`, `41880000`, `41900000`, `41980000`; in mediump they are
fp16 halves `4c00`, `4c40`, `4c80`, `4cc0`.

**What is quoted.** Each listing below is the `fs.fragment` region of that
JSON: its `words` (the 64-bit instruction words, as hex) and its `text`
(Mesa's disassembly of them). These are the programs of the trace's first
draw, which `pan_ref_diff.py` pairs. They are not taken from the run's
stderr. Every fragment program ends with the same three words, which the
listings leave out:

    787f4004733c00f0  BLEND.slot1.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
    0110f000000000c0  IADD_IMM.i32 r48, 0x0, #0x0
    002fc1100000f1c0  BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1

For each draw, the table gives the number of words, the sha256 of all of
them (each word as 16 lower-case hex digits and a newline, concatenated, in
program order), the FAU words the trace prints, and `work_reg_count` (32 in
all 14) and the preload mask (`0x3000000000000000` in all 14). A re-run of
the two commands can be checked against these values.
`python3 -I $CONNECTED_DEPS_DIR/mesa-shim/fsv/check_note.py <this note> $CONNECTED_DEPS_DIR/mesa-shim/fsv/out`
checks two things: every listing below is a contiguous run of its draw's
words with the same text, and the table is right. It passed on this
version of the note.

| draw | words | sha256 of the words | FAU words (from u0.w0) |
|---|---|---|---|
| fract highp | 10 | `1dddb15b56bba864cab2d7abcf62b1f97d61bff72a7c4b449f177af59a1a2bba` | 0 x4 |
| fract mediump | 10 | `a0948ca9b3026527458544213d832462b28b9826b5ff1b91fded1c29c16adcb9` | 0 x4 |
| length highp | 13 | `413262dba972ff5e5c2e8ec9a48284988c76faf4fbb9995131a8f082a773662a` | 0 x4 |
| length mediump | 14 | `efc1b7db4d852caea1d8376ffe5bd8397500aa3ebeccef35328a3aa9d6635836` | 0 x4 |
| normalize highp | 17 | `f93523cbeef0ed692d36a7e8a252ec0526ca64e374214e8ac23432b4fbdc0fde` | 0 x4, `3c003c00`, 0 |
| normalize mediump | 14 | `90c3fd6a46a49afcc17e00bb64ccf81a530d7575c0d078f4eadee0e696dd09fb` | 0 x4, `3c003c00`, 0 |
| nconst highp | 13 | `d4c450cfc329eda75865c8946283695ef82678eeb4175f0684ef03fbcf745b8f` | 0 x4, `3eaaaaab`, `3f2aaaab`, `3c003c00`, 0 |
| nconst mediump | 11 | `33001ce2d02aaabb049059ee52cb32d8dcaf2e5e37c2eddcc706fc06e797c763` | 0 x4, `39553555`, `39553955`, `3c003c00`, 0 |
| lconst highp | 8 | `efa9ea61851593334e7ab2378555ffaf9dc20d27bfd4112d814ba65cf6e0094a` | 0 x4 |
| lconst mediump | 9 | `6c2ecdee7b268dc1fab5bb9200b1040e33b2e63a9b4235ebfe9093fcb65a2889` | 0 x4, `42004200`, 0 |
| reflect highp | 17 | `1d0260bcc8d7b53b2ac843c9120297f30a151578b2d2c21b79dc0890293d3c9c` | 0 x4, `41800000`, `41880000`, `41900000`, `3c003c00` |
| reflect mediump | 14 | `2fb5b716f5601cb05281e0cc57fb55d63160a85a38b08cebeba8a9593aa44586` | 0 x4, `4c404c00`, `4cc04c80`, `3c003c00`, 0 |
| pow highp | 17 | `90ad994b5dc1f5f394351807b3a69b3df920a783a74040fd6d16b3445fb48233` | 0 x4, `41800000`, `41880000`, `00000018`, 0 |
| pow mediump | 17 | `f5fbbb85dfef503558c648d9c792143842c25bae741924daadc2ce93724cb7bc` | 0 x4, `4c404c00`, `00000018` |

The first four FAU words (u0, u1) are 0 in every draw. The column starts at
u0.w0, so a listing's `u2.w0` is the fifth word.

**What the run settles.** The highp listings of `fract`, `length`,
`normalize` and `pow` equal the first version's `pan_shc` listings, in every
instruction that version quoted. It quoted text only (no load, no tail, no
words), so only the text was compared. `reflect` highp has the same instructions in the same
order, but a different operand order (see reflect). The mediump forms are
new: Mesa's GLSL front end lowers a mediump operand to fp16. This gives
`V2F32_TO_V2F16`, `.v2f16` instructions and `FRSQ.f16` / `FRCP.f16`, except
in pow, which stays fp32. A mediump program always loads all four lanes of
`v` (`LD_VAR_BUF_IMM.f32...v4`, `@r0:r1:r2:r3`). A highp program that reads
only `v.x` loads one lane (`@r0`). The two questions the first version left
open are answered: reflect emits **one** `FADD`, and a constant normalize is
folded to pushed constants (see normalize).

No `tools/mali/testdata` fixture is added by this note. The first
implementation commit adds the 14 JSON files (item 1 below).

## What the front end already gives the back end

`src/shaderc/online/builtin_call.rs` (:860-990) expands the built-ins to the IR
the Mali walk reads:

| GLSL | IR |
|---|---|
| `normalize(v)` | `Fdot(v, v)`, `Frsq`, `Fmul(v, splat)` |
| `length(v)` | `Fdot(v, v)`, `Fsqrt` |
| `reflect(I, N)` | `Fdot(N, I)`, `Fmul(d, -2.0)`, `Ffma(splat, N, I)` |
| `pow(x, y)` | `Flog2(x)`, `Fmul(y, l)`, `Fexp2(m)` |
| `fract(x)` | `AluOp::Ffract` |

so there is nothing for the front end to do; the work is in
`nir_fs.rs` / `nir.rs` / `emit.rs` / `bir.rs` / `pack.rs`.

## The walk today

The line numbers in this section are those of 7b2d529e.

* `Alu` (nir.rs:96-) has `Frcp`, `Frsq`, `Ffloor`, `Fceil`, `Ftrunc`, `Fadd`,
  `Fmul`, `Ffma`, `Fneg`, `Fabs`, `Fclamp`; **no** `Flog2`, `Fexp2`.
* The highp lane arm (`Fs::lane32`, nir_fs.rs ~2043-2056) makes `Fsqrt` as
  `Frcp(Frsq(x))`, `Frsq`, `Ffloor` and `Ffract` as
  `Fadd(x, Fneg(Ffloor(x)))`. The two-lane mediump arm (~1792) does `Ffract`
  on a pair of halves (`FROUND.v2f16`).
* The mediump walk's catch-all `ir::Op::Alu(..) => Err(Arithmetic)`
  (nir_fs.rs:4216) refuses everything else, among them `Ffract` of a single
  mediump lane or of three or four lanes, `Fsqrt`, `Frsq`, `Flog2`, `Fexp2`.
  The dot products are handled (`Fdot2/3/4`, ~:2284).
* `bir.rs` (at 6d360a03) has `FrcpF32`, `FrsqF32`, `FroundF32`, `FroundV2f16`, `FmaF32`,
  `FmaV2f16`, `FaddF32`, `FaddV2f16`, `V2f32ToV2f16`, `F16ToF32`,
  `MkvecV2i16`, and, since 3d937fe5, `FlogdF32`, `FexpF32`, `FrexpeF32`,
  `FaddLscaleF32` and `FmaRscaleF32`. It has no fp16 `FRSQ` or `FRCP`.
  `F32ToS32` always writes `.rtz` (pack.rs, `Round::Rtz`).
  mali-transcendentals.md (case 77) designs `sin`/`cos`/`log2`/`exp2` on
  these ops. **pow is that note's log2 and exp2 plus one multiply.**

## Per built-in

Each listing: the instruction word, then Mesa's disassembly of it. The
program's first word (the varying load) and its colour packing are
included. The common three-word tail is left out (see above).

### fract

NIR: `lower_ffract` (`nir_opt_algebraic.py`, for arch < 11): `ffract(a)`
becomes `fadd(a, fneg(ffloor(a)))`.

highp (`fsv_fract_highp`):

```
085c80020200007d  LD_VAR_BUF_IMM.f32.slot0.src_f32.center.store.wait0 @r0, r61^, index:0x0
6890c100800d0000  FROUND.f32.rtn.discard r1, r0
00a4c01000004140  FADD.f32 r0, r0^, r1^.neg
40a440100000c040  FADD.f32.wait0126 r0.h0, r0^, 0x0.neg                 # the colour's narrowing
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
0090c00000140040  U16_TO_U32 r0, r0^.h0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

mediump (`fsv_fract_mediump`): the lane is narrowed first, then `FROUND`
and `FADD` run as `v2f16` on its half (`.h00`):

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68a440100000c040  FADD.f32.discard r0.h0, r0^, 0x0.neg
0090c100800f0000  FROUND.v2f16.rtn r1, r0.h00
40a5c01000004140  FADD.v2f16.wait0126 r0, r0^.h00, r1^.neg.h00
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
0090c00000140040  U16_TO_U32 r0, r0^.h0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

Yakogl: the instruction pair exists (`Alu::Ffloor` -> `FROUND.rtn`, `Fadd`
with `Fneg` folded as a `.neg` source), in both widths (`FroundV2f16`,
`FaddV2f16`). The highp lane arm builds the highp form. Missing: an arm for
a single mediump lane. The shim's form is the narrowing, then the `v2f16`
pair on `.h00`. Three or four mediump lanes are also missing (a
`Word::Lanes` colour, nir_fs.rs ~6750). The existing `FROUND.v2f16` path
covers a pair of fp16 lanes, so three lanes are a pair plus a lone lane.
Reference (tools/mali/testdata): `s4_ffract.*` is the highp case.

### length

NIR: `fdot3(v, v)` -> `fsqrt` -> `lower_fsqrt` (`frcp(frsq(a))`).

highp (`fsv_length_highp`):

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68b2c20400c04242  FMA.f32.discard r2, r2^, r2^, 0x0.neg
00b2c10000424141  FMA.f32 r1, r1^, r1^, r2^
00b2c00000414040  FMA.f32 r0, r0^, r0^, r1^
009cc00000020040  FRSQ.f32 r0, r0^
009cc00000000040  FRCP.f32 r0, r0^
40a440100000c040  FADD.f32.wait0126 r0.h0, r0^, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
0090c00000140040  U16_TO_U32 r0, r0^.h0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

mediump (`fsv_length_mediump`): x and y are narrowed as one pair
(`V2F32_TO_V2F16`) and z alone. The dot is three `FMA.v2f16` on single
halves, and the root is `FRSQ.f16` then `FRCP.f16`:

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68a5c00000044140  V2F32_TO_V2F16.discard r0, r0^, r1^
00a441100000c042  FADD.f32 r1.h0, r2^, 0x0.neg
00b3c10402c04141  FMA.v2f16 r1, r1^.h00, r1^.h00, 0x0.neg
00b3c1003c410000  FMA.v2f16 r1, r0.h11, r0.h11, r1^.h00
00b3c00000414040  FMA.v2f16 r0, r0^.h00, r0^.h00, r1^.h00
009cc00000030040  FRSQ.f16 r0, r0^.h00
409cc00000010040  FRCP.f16.wait0126 r0, r0^.h00
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
0090c00000140040  U16_TO_U32 r0, r0^.h0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

The dot is a chain of `FMA` in both precisions, in Mesa's order z, y, x:
`v.z*v.z + -0.0`, then `v.y*v.y +`, then `v.x*v.x +`. In mediump the
result is already the colour's fp16 half, so no `FADD.f32 r0.h0` narrowing
follows. Yakogl: the highp form exists (the `Fdot3` expansion, `Frsq`,
`Frcp`; the highp `Fsqrt` arm is `Frcp(Frsq(x))`). Missing: the `Fsqrt` arm
in the mediump walk (a scalar from three lanes), and with it `FRSQ.f16` and
`FRCP.f16`, which bir.rs does not have (two new ops, the fp16 forms of
`FrsqF32` / `FrcpF32`: `.f16` instead of `.f32`, a `.h00` source).

`length` of a constant (`lconst`, `length(vec3(1.0, 2.0, 2.0)) * v.x`) is
folded to 3.0, so no `FRSQ`/`FRCP` is emitted. In highp the product is not
in the fragment program at all: `nir_opt_varyings` moved the multiply into
the vertex shader. That stage's varying program is
`FMA.f32 r0, u5.w1, r0^, 0x0.neg`, and its FAU word u5.w1 is `40400000` (3.0).
The fragment program only loads the lane and narrows it:

```
lconst highp:
085c80020200007d  LD_VAR_BUF_IMM.f32.slot0.src_f32.center.store.wait0 @r0, r61^, index:0x0
40a440100000c040  FADD.f32.wait0126 r0.h0, r0^, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
6890c00000140040  U16_TO_U32.discard r0, r0^.h0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

In mediump the multiply stays in the fragment program. 3.0 is a pushed pair
of fp16 halves (u2.w0 = `42004200`):

```
lconst mediump:
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68a440100000c040  FADD.f32.discard r0.h0, r0^, 0x0.neg
40b3c00422c04084  FMA.v2f16.wait0126 r0, u2.w0, r0^.h00, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
0090c00000140040  U16_TO_U32 r0, r0^.h0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

### normalize

NIR: `fdot3(v, v)`, `frsq`, three `fmul`.

highp (`fsv_normalize_highp`):

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68b2c30400c00202  FMA.f32.discard r3, r2, r2, 0x0.neg
00b2c30000430101  FMA.f32 r3, r1, r1, r3^
00b2c30000430000  FMA.f32 r3, r0, r0, r3^
009cc30000020043  FRSQ.f32 r3, r3^
00b2c00400c00340  FMA.f32 r0, r0^, r3, 0x0.neg
00b2c10400c00341  FMA.f32 r1, r1^, r3, 0x0.neg
00b2c20400c04342  FMA.f32 r2, r2^, r3^, 0x0.neg
00a440100000c040  FADD.f32 r0.h0, r0^, 0x0.neg
00a441100000c041  FADD.f32 r1.h0, r1^, 0x0.neg
40a442100000c042  FADD.f32.wait0126 r2.h0, r2^, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
00a1c00000054140  MKVEC.v2i16 r0, r0^.h0, r1^.h0
48a1c10000058442  MKVEC.v2i16.wait r1, r2^.h0, u2.w0.h0                 # u2.w0 = 3c003c00, the alpha 1.0
```

mediump (`fsv_normalize_mediump`): x and y are one fp16 pair and z has its
own register. The three products are one `v2f16` FMA on the pair and one
single-half FMA:

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68a5c00000044140  V2F32_TO_V2F16.discard r0, r0^, r1^
00a441100000c042  FADD.f32 r1.h0, r2^, 0x0.neg
00b3c20402c00101  FMA.v2f16 r2, r1.h00, r1.h00, 0x0.neg
00b3c2003c420000  FMA.v2f16 r2, r0.h11, r0.h11, r2^.h00
00b3c20000420000  FMA.v2f16 r2, r0.h00, r0.h00, r2^.h00
009cc20000030042  FRSQ.f16 r2, r2^.h00
00b3c00422c00240  FMA.v2f16 r0, r0^, r2.h00, 0x0.neg
40b3c10402c04241  FMA.v2f16.wait0126 r1, r1^.h00, r2^.h00, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
48a1c10000058441  MKVEC.v2i16.wait r1, r1^.h0, u2.w0.h0
```

The mediump colour needs no per-lane narrowing. `r0` already holds the x,y
halves, and `MKVEC.v2i16` packs z with the constant 1.0 (`u2.w0.h0`).

`normalize` of a constant (`nconst`, `normalize(vec3(1.0, 2.0, 2.0)) * v.x`)
is folded to (1/3, 2/3, 2/3): no dot, `FRSQ` or `FRCP`. The folded
components are pushed words. In highp, u2.w0 = `3eaaaaab` (1/3) and
u2.w1 = `3f2aaaab` (2/3); y and z share the 2/3 product. In mediump,
u2.w0 = `39553555` holds the halves (1/3, 2/3) and u2.w1 = `39553955`
holds (2/3, 2/3). One `v2f16` FMA makes the x,y pair and a second one
makes z:

```
nconst highp:
085c80020200007d  LD_VAR_BUF_IMM.f32.slot0.src_f32.center.store.wait0 @r0, r61^, index:0x0
68b2c10400c00084  FMA.f32.discard r1, u2.w0, r0, 0x0.neg
00b2c00400c04085  FMA.f32 r0, u2.w1, r0^, 0x0.neg
00a441100000c041  FADD.f32 r1.h0, r1^, 0x0.neg
40a440100000c040  FADD.f32.wait0126 r0.h0, r0^, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
00a1c10000050041  MKVEC.v2i16 r1, r1^.h0, r0.h0
00a1c20000058640  MKVEC.v2i16 r2, r0^.h0, u3.w0.h0
0091c00000000041  MOV.i32 r0, r1^
4891c10000000042  MOV.i32.wait r1, r2^

nconst mediump:
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68a440100000c040  FADD.f32.discard r0.h0, r0^, 0x0.neg
00b3c10422c00084  FMA.v2f16 r1, u2.w0, r0.h00, 0x0.neg
40b3c00422c04085  FMA.v2f16.wait0126 r0, u2.w1, r0^.h00, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
00a1c20000058640  MKVEC.v2i16 r2, r0^.h0, u3.w0.h0
0091c00000000041  MOV.i32 r0, r1^
4891c10000000042  MOV.i32.wait r1, r2^
```

Yakogl: every highp row exists (`Frsq`, `Fmul` as `FMA ... -0.0`, the
`Fdot3` chain). Missing: the mediump arm for `Frsq` (the catch-all) with
`FRSQ.f16` (new, as for length), and a splat of the scalar over three
lanes: one `FMA.v2f16` on the pair, one on the lone half. The byte test
has to match the dot's operand order (z, y, x) and the register reuse.

### reflect

NIR: `fdot3(N, I)`, `fmul(d, -2.0)`, `ffma(k, N, I)`. The fragment shader's
final NIR in the highp run's stderr is the dot as `fmul` and two `ffma`
(z, y, x), then `fadd %6, %6`, `fneg %7` and three `ffma %8, N.c, I.c`. The
result, in both precisions: **one** `FADD` (`d + d`), then three `FMA`s with
the negated `2d` as `src0`.

highp (`fsv_reflect_highp`; u2.w0, u2.w1, u3.w0 = `41800000`, `41880000`,
`41900000` are u.x, u.y, u.z):

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68b2c30400c00286  FMA.f32.discard r3, u3.w0, r2, 0x0.neg
00b2c30000430185  FMA.f32 r3, u2.w1, r1, r3^
00b2c30000430084  FMA.f32 r3, u2.w0, r0, r3^
00a4c30000004343  FADD.f32 r3, r3^, r3^                                 # 2*dot
00b2c04000408403  FMA.f32 r0, r3.neg, u2.w0, r0^
00b2c14000418503  FMA.f32 r1, r3.neg, u2.w1, r1^
00b2c24000428643  FMA.f32 r2, r3^.neg, u3.w0, r2^
00a440100000c040  FADD.f32 r0.h0, r0^, 0x0.neg
00a441100000c041  FADD.f32 r1.h0, r1^, 0x0.neg
40a442100000c042  FADD.f32.wait0126 r2.h0, r2^, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
00a1c00000054140  MKVEC.v2i16 r0, r0^.h0, r1^.h0
48a1c10000058742  MKVEC.v2i16.wait r1, r2^.h0, u3.w1.h0
```

mediump (`fsv_reflect_mediump`; u2.w0 = `4c404c00` holds the halves
(u.x, u.y) and u2.w1 = `4cc04c80` holds (u.z, u.w)). The dot and the 2*dot
are `v2f16`:

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
68a5c00000044140  V2F32_TO_V2F16.discard r0, r0^, r1^
00a441100000c042  FADD.f32 r1.h0, r2^, 0x0.neg
00b3c20402c00185  FMA.v2f16 r2, u2.w1.h00, r1.h00, 0x0.neg
00b3c2003c420084  FMA.v2f16 r2, u2.w0.h11, r0.h11, r2^.h00
00b3c20000420084  FMA.v2f16 r2, u2.w0.h00, r0.h00, r2^.h00
00a5c20000004242  FADD.v2f16 r2, r2^.h00, r2^.h00
00b3c0400a408402  FMA.v2f16 r0, r2.neg.h00, u2.w0, r0^
40b3c14000418542  FMA.v2f16.wait0126 r1, r2^.neg.h00, u2.w1.h00, r1^.h00
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
48a1c10000058641  MKVEC.v2i16.wait r1, r1^.h0, u3.w0.h0
```

How this differs from the highp `pan_shc` (SPIR-V, fp32) listing of the
first version:

- the dot's FMAs name the uniform first: `u3.w0, r2` where `pan_shc` had
  `r2, u3.w0`, and likewise `u2.w1, r1` and `u2.w0, r0`;
- the three final FMAs carry the negation on the register, as their first
  source: `r3.neg, u2.w0, r0^` where `pan_shc` had `u2.w0.neg, r3, r0^`, and
  likewise for r1 and r2.

The instruction count and the order of the rows are the same. The byte test
must produce the shim's operand order.

Yakogl: all highp instructions exist. Missing: the rewrite
`fmul(d, -2.0)` -> `fadd(d, d)` (the `d*2.0 = d+d` rule,
`nir_opt_algebraic.py`), and `ffma(fmul(d, -2.0), n, i)` ->
`ffma(fneg(2d), n, i)` with the negated `2d` as `src0`, in `nir.rs` (check
mali-algebraic-folds.md for what is already transcribed). Also missing: the
mediump `Ffma` arm, with the pair form for x,y (`u2.w0` as a whole pair) and
the single-half form for z. A source `neg` on `src0` exists (`Src.neg`). No
new bir op.

### pow

NIR: `lower_fpow` (panfrost sets it): `fpow(a, b)` ->
`fexp2(fmul(flog2(a), b))`, then Bifrost's own expansion of `flog2` and
`fexp2` (`bi_flog2_32`, `bi_fexp_32`).

highp (`fsv_pow_highp`; u2.w1 = `41880000` is u.y, u3.w0 = `00000018` is the
constant 24):

```
085c80020200007d  LD_VAR_BUF_IMM.f32.slot0.src_f32.center.store.wait0 @r0, r61^, index:0x0
6899c10002020000  FREXPE.f32.log.discard r1, r0
0090c10000090041  S32_TO_F32 r1, r1^
00a4c240000900d0  FADD_LSCALE.f32 r2, 0x3F800000.neg, r0
009cc00000080040  FLOGD.f32 r0, r0^
00b2c00000414240  FMA.f32 r0, r0^, r2^, r1^                             # log2(x)
0091c10000000086  MOV.i32 r1, u3.w0                                     # 24
0160c00441c04085  FMA_RSCALE.f32 r0, u2.w1, r0^, 0x0.neg, r1^           # y * log2(x) * 2^24
0090c100000c0000  F32_TO_S32 r1, r0                                     # round field 0, no .rtz
00a4c00000084041  FEXP.f32 r0, r1^, r0^
40a440100000c040  FADD.f32.wait0126 r0.h0, r0^, 0x0.neg
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
0090c00000140040  U16_TO_U32 r0, r0^.h0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

mediump (`fsv_pow_mediump`; u2.w0 = `4c404c00` holds the halves (u.x, u.y),
u2.w1 = `00000018`): the log and exp stay fp32, and the varying is read as
fp32. Only `y` is an fp16 half, converted first. The 24 is read straight
from u2.w1 as `src3`, with no `MOV`:

```
085c80083200007d  LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
6899c10002020000  FREXPE.f32.log.discard r1, r0
0090c10000090041  S32_TO_F32 r1, r1^
00a4c240000900d0  FADD_LSCALE.f32 r2, 0x3F800000.neg, r0
009cc00000080040  FLOGD.f32 r0, r0^
00b2c00000414240  FMA.f32 r0, r0^, r2^, r1^                             # log2(x)
0090c100100b0084  F16_TO_F32 r1, u2.w0.h1                               # y
0160c00485c04041  FMA_RSCALE.f32 r0, r1^, r0^, 0x0.neg, u2.w1           # y * log2(x) * 2^24
0090c100000c0000  F32_TO_S32 r1, r0
00a4c00000084041  FEXP.f32 r0, r1^, r0^
00a440100000c040  FADD.f32 r0.h0, r0^, 0x0.neg
4090c00000140040  U16_TO_U32.wait0126 r0, r0^.h0
087dbc0200ead07c  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
4891c100000000db  MOV.i32.wait r1, 0x3C000000
```

This is mali-transcendentals.md's rows 23-27 (log2) and 30-34 (exp2). The
`* y` is folded into `FMA_RSCALE` instead of a separate multiply, in both
precisions. `FMA_RSCALE` takes `y` as `src0`, `log2(x)` as `src1` and the
scale 24 as `src3`, so `fexp2(y * log2(x))` costs one instruction, not a
multiply plus an `FMA_RSCALE` by 1.0 (case 77's form). Two forms of the 24
appear here: a `MOV.i32` from a pushed word (highp), or the pushed word
itself as `src3` (mediump). Case 77 has `IADD_IMM.i32 r4, 0x0, #0x18`.
`fau.rs` chooses among them, as it already does.

Yakogl: the ops exist in bir.rs since 3d937fe5 (`FrexpeF32 { log }`,
`FaddLscaleF32`, `FlogdF32`, `FmaRscaleF32`, `FexpF32`, `S32ToF32`,
`F16ToF32`). Missing:

- `F32_TO_S32` without `.rtz`: the shim's word has round field 0, but
  pack.rs writes `Round::Rtz`;
- `Alu::Flog2` / `Fexp2` with their rows (mali-transcendentals.md);
- the fusion: `emit.rs` expands `Fexp2` of a product as one `FMA_RSCALE`,
  with the factors as `src0`/`src1`;
- in mediump, `F16_TO_F32` of the fp16 `y` before the fp32 chain, and the
  `FADD.f32 r0.h0` narrowing after it, not fp16 log/exp.

## glmark2 shaders and sweep cases

Source: `$CONNECTED_DEPS_DIR/glmark2-src/data/shaders/*.frag` and `*.all`,
searched for each call (a name match; whether the shader then passes also
depends on its other refusals, which the corpus tally of
tests/glmark2_corpus.rs decides and this note did not run):

| built-in | fragment shaders using it |
|---|---|
| pow | bump-normals, bump-height, bump-normals-tangent, bump-poly, ideas-lamp-lit, ideas-logo, jellyfish, light-advanced, light-phong, light-cel, light-refract, terrain |
| normalize | bump-height, bump-normals-tangent, bump-normals, bump-poly, ideas-lamp-lit, ideas-logo, light-refract, light-advanced, light-cel, terrain-normalmap, light-phong, terrain |
| reflect / refract | ideas-lamp-lit, ideas-logo, light-phong, light-cel, light-refract |
| length | light-cel, terrain |
| fract | conditionals, function, loop, light-basic-tex-bilinear and the `-step-*.all` variants of conditionals, function and loop |

gl_test (tools/shaders/es100): `equirect.frag` (`normalize`) and
`fragcentre.frag` (`fract`) are `ok` in `tools/mali/corpus-verdicts.txt`
already, so no sweep case is unblocked by the five alone. `pow` shares its
log2/exp2 with case 77 (`transc`), which the transcendentals note covers.

`reflect` appears in the list but `refract()` is another built-in (not
covered here): light-refract needs it after `reflect`.

## Implementation order

1. Commit the references: the 14 draws above as `tools/mali/testdata`
   fixtures (the shaders and the `extract --draw` JSON), re-made with the
   two commands of "How the words were made" and checked against the sha256
   table. The shim runs are done, and the `pow` fusion is confirmed in both
   precisions.
2. `length` and `normalize`: the mediump arms for `Fsqrt` and `Frsq`, the
   new `FRSQ.f16` / `FRCP.f16` ops and the scalar-splat multiply. Unblocks
   the `normalize`-only shaders (terrain-normalmap) once their other
   refusals are gone.
3. `fract` of a single mediump lane (the narrowing, then the `v2f16` pair)
   and of three or four lanes.
4. `reflect`: the `d+d` and negated-operand rewrites in `nir.rs` in the
   shim's operand order, then the mediump `Ffma` arm. Unblocks the light and
   ideas shaders together with 2.
5. `pow`: `Alu::Flog2` / `Fexp2` (mali-transcendentals.md), `F32_TO_S32`
   without `.rtz`, then the fused multiply. Largest item, and the one most
   glmark2 shaders wait for (12 of them).
6. Host tests per item in `src/shaderc/online/mali/tests.rs` in the style of
   `check_fs_stage`; `tools/mali/corpus-verdicts.txt` and
   `sweep_passsets.py --delta tools/cts/log` move forward only.

## Risks

* The shim was built from the tree in `$CONNECTED_DEPS_DIR/mesa-pan`, not
  from a freshly checked tarball (see "How the words were made"). The
  fixtures of item 1 should be re-made on a shim built with
  `drm_shim_ref.sh build mesa-26.2.1.tar.xz <dir>` and compared with the
  sha256 table. A difference would mean the tree is not the release.
* The register choice, the `^` flags and the operand order are those of
  these runs. They are pinned by the sha256 table, but no committed fixture
  pins them yet (item 1).
* The folded `lconst` product moves to the vertex shader in highp
  (`nir_opt_varyings`); a fragment lowering sees only the narrowing there.
* docs/README.md has no index line for this note: that file is outside this
  item's files (plan #327 is editing it); the planner adds it.
