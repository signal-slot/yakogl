<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): lowering unwritten gl_Position lanes and uniform-only varyings (design)

Plan #387, item posvary-design. A design for the item that lowers the three
vertex-shader refusals of [`mali-deqp-posvary.md`](mali-deqp-posvary.md)
(posvary-lower): no compiler change here. The shapes, the 18 Mesa 26.2.1
references (`tools/mali/testdata/deqp_posvary_*`, `tests/mali_deqp_posvary.rs`)
and the case counts (42, HOST-ONLY at `9ac31f30`) are that note's; this note
reads the references' words and the compiler and fixes the mechanisms and
their order. It also **corrects that note's reading of the link** (section
2): the references show a different rule from the one its section 3 and 6
lower to.

Sources read: the `text` (Mesa's own `va_disasm_instr`) of each
`deqp_posvary_*.expect.json` region (no rebuild: the words are the ones the
references hold, disassembled by the method of `tools/mali/README-deqp_ref.md`);
`nir_opt_varyings.c` of Mesa 26.2.1 (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/src/compiler/nir/`,
quoted as `M:nir_opt_varyings.c:<line>`); `src/shaderc/online/mali/` `nir.rs`,
`nir_fs.rs`, `select.rs`, `link.rs`, `mod.rs`, `motion.rs`; the notes
`mali-varying-constants.md`, `mali-partial-varyings.md`,
`mali-deqp-varylink.md`, `mali-attribute-records.md`, `mali-deqp-families.md`
section 18 (rows 25, 36, 37). Sections 1 to 8 were written from the words
and the code alone. Statements marked *(reading)* are from the code, not from
an instrumented run. Section 9 holds what a later instrumented run measured
(temporary `eprintln!`s at the refusal sites, reverted, nothing committed):
where it differs from a *(reading)*, section 9 is right.

## 1. The 18 references, word by word

Common to every reference with a vertex program: `vs.position` is the 24-word
program (`LD_ATTR_IMM.v4` of `gl_Position`, `FRCP`/`FMAX`/`FMIN` of 1/w, three
`FMA` by the viewport scale and offset, four `STORE.istream`), which the
compiler already emits. Only `vs.varying` and `fs.fragment` differ, and the
position program does not depend on any refusal below, except
`loop_phi_array` (it also holds the array initialisation, 104 words) and
`unwritten_negative_vs` (a constant position, 18 words, section 3.3).
Stores are `LEA_BUF_IMM` + `STORE.i32.estream` (a word per lane, at the
varying buffer's 4-byte word `i`: `MOV u4.w0`, `SHADDX.s64`).

| reference | vertex `vs.varying` | fragment `fs.fragment` |
|---|---|---|
| `flat_sep_rnd13` | stores **4 words**: the attribute `o` (`LD_ATTR_IMM.v4` index 1, four stores). Nothing for `e = float(n)` (uniform `n`) | `LD_VAR_BUF_IMM.v4.src_f32` once, then `FADD_IMM.v2f16` constants; 9 words |
| `flat_sep_kau_bs26` | 5 stores. Smooth vec4 `b = vec4(i,h,3,g)+f` = words 0..3: `IADD_IMM 0x3F800000` (lane 0 = 1.0, **a constant lane, stored**), `h+4.0`, `IADD_IMM 0x40800000` (lane 2 = 4.0, stored), `g-4.0`. Then word 4: `e = j+4.0` on the mediump uniform: `FADD.v2f16 r0, u5.w1.h00, 0x44000000.h11`, `F16_TO_F32 r0, r0.h0`, **`FMA.f32 r0, r0^, 0x0, r0^`**, `STORE`. | `LD_VAR_BUF_IMM.f32.v4.src_f32` (index 0x0) then `LD_VAR_BUF_IMM.f32.src_flat32 @r1, 0x0, index:0x10` (the flat word, address operand `0x0`, index = byte offset 16); 12 words |
| `flat_sep_vec4_bs51` | 5 stores. Word 0 is the smooth `b` (`F32_TO_S32`, `ICMP_OR`, `CSEL.u32.eq`: an attribute-driven select). Words 1, 2, 3 are `a.x, a.y, a.z` of `a = v - u` (uniform vec4): `F16_TO_F32 r4, u5.w1.h0; FADD.f32 r4, u6.w1, r4.neg; STORE`, **no `FMA x,0,x`**. Word 4 is `a.w`: `F16_TO_F32; FADD;` **`FMA.f32 r2, r2^, 0x0, r2^`**; `STORE`. | `LD_VAR_BUF_IMM.v4.src_f32` (words 0-3: `b, a.x, a.y, a.z`) and `LD_VAR_BUF_IMM.src_flat32 @r4, 0x0, index:0x10` (word 4 = `a.w`); 21 words |
| `flat_lane_vec4_v86` | 3 stores: `b` (attribute index 2) word 0, `a-5.0` word 1, **`F16_TO_F32 r2, u5.w1.h0` (the uniform `c`, no `FMA`)** word 2. `v_color = vec4(c, c, b, a-5)`: the two `c` lanes are one value, one word | `LD_VAR_BUF_IMM.v3.src_f32`; `FADD.f32 r2.h0`, `V2F32_TO_V2F16 r0 (b, a-5)`, `IADD.v2u16 r1, r2^.h00, 0x0` (the `c` word duplicated into lanes 0 and 1); 11 words |
| `flat_lane_vec4_c30` | 3 stores: `q*s` (`FMA.f32 r0, r0^, u6.w1, 0x0.neg`) word 0, then `F16_TO_F32 u6.w0.h0` word 1 (uniform `v`, `d.z`), `F16_TO_F32 u5.w1.h0` word 2 (uniform `y`, the varying `a`). **No `FMA x,0,x`** | `LD_VAR_BUF_IMM.v3.src_f32`; the two uniform lanes are read as smooth; the constant lanes `d.y = x-w`, `d.w = u`, `b`, `f`, `i` are fragment constants (FAU `0x40004000`, `0x3400`, …); 15 words |
| `tex_uniform_w`, `_array` | 4 stores: `TEX_SINGLE.rgb` into `r4,r5,r6` stored as words 0-2, then `F16_TO_F32 u5.w1.h0` (the uniform alpha `s_a`) word 3: **no `FMA`** | `LD_VAR_BUF_IMM.v4.src_f32`, no flat load; 7 words |
| `loop_phi_array` | after the array's 40 `STORE.i16` to `thread_local_pointer`, the loop (`ICMP_OR.s32.ge.m1 r2, r1, u5.w1, 0x0` bound from the int uniform, `LOAD.i16 [thread_local_pointer + i*2]`, `FADD.v2f16`, `BRANCHZ.reconverge`), `F16_TO_F32` of the sum **stored as is**, then three **constant stores** `IADD_IMM 0`, `0`, `0x3F800000`: v4, 109 words | `LD_VAR_BUF_IMM.v4.src_f32`, 7 words |
| `fract_{highp,mediump,lowp}_{vec2,vec3}` | v4 colour, four stores, **all four lanes stored including the constants**, the values computed at fp16 in pairs (below); 29-32 words | `LD_VAR_BUF_IMM.v4.src_f32`, 7 words |
| `unwritten_not_declared`, `_declared_unused`, `_declared_dead_read` | **no `vs.varying` region**; `vs.position` is the 24 words | `NOP; ATEST 0x3F800000; MOV r0, u2.w0; MOV r1, u2.w0; BLEND`: the constant colour (FAU `0x3c003c00`) |
| `unwritten_negative_vs` | no `vs.varying`; `vs.position` is **18 words, no `LD_ATTR`**: `LEA_BUF_IMM`, `FADD.f32 r2, u0.w0, u0.w1` (= 1.0 * scale + offset, the viewport transform of a constant, no `FRCP`/`FMAX`/`FMIN`), the same for y and z, `IADD_IMM 0x3F800000` for w | the same 7-word constant colour |

The `fract` words (`fract_mediump_vec2`, `in0 = a_in0.wy`, `color.yz = res`):
`LD_ATTR_IMM.v4 index:1` (the whole vec4: the highest lane read is w; the
highp vec3 `a_in0.zxy` loads `.v3`); `FADD.f32 r0.h0, r1^, 0x0.neg` and
`FADD.f32 r1.h0, r3^, 0x0.neg` (the f32 to f16 narrowing of lanes y and w);
**`FROUND.v2f16.rtn r2, r1.h00`** and `FROUND.v2f16.rtn r3, r0.h00` (a lane
duplicated into both halves: the `ffloor`s of the two lanes first, as
`lower_ffract` does); `MKVEC.v2i16 r2, r2^.h0, r3^.h0`; two `FADD.v2f16 x,
r2.neg.h00 / .h11` (the `x - floor(x)`); `MKVEC.v2i16 r1, 0x0.h0, r1^.h0` and
`MKVEC.v2i16 r0, r0^.h0, u6.w0.h0` (the colour's lanes with the constants 0 and
1 as halves; `u6.w0.h0` = 1.0); two `FMA.v2f16 .., u5.w1, 0x2E660000.h11`
(`* 0.8 + 0.1`, the pairs (x,w) and (y,z)); four `F16_TO_F32`; four stores in
lane order. The highp variants run `FROUND.f32.rtn` + `FADD.f32 x, r.neg` at
fp32 and narrow with `FADD.f32 rN.h0, rN^, 0x0.neg` before the `MKVEC`/`FMA.v2f16`
(`fract_highp_vec3`: 32 words).

## 2. What Mesa's link does: a correction

`mali-deqp-posvary.md` sections 3 and 6 say a uniform expression stays a
**flat** varying, loaded `.flat32` *after* the smooth ones, and a bare
uniform copy moves into the fragment shader; section 6 lowers the
`flat && smooth` refusal as "smooth first, flat lanes after". The words above
show the rule is different and simpler (it is `nir_opt_varyings`'s own
`fs_assign_slot_groups`, `M:nir_opt_varyings.c:5228-5290`, documented at
`:419-425`):

1. A lane whose stored value is **convergent** (the same for all vertices: a
   uniform, a constant, an expression of those; `nir_vertex_divergence_analysis`,
   `M:nir_opt_varyings.c:41-63`) is not flat by itself. `tidy_up_convergent_varyings`
   (`:1854-1935`) keeps it a *convergent* lane that can be packed either way.
2. Packing order (`:5253-5290`): first the interpolated lanes (in layout
   order, each to the next scalar slot from VAR0.x), then the flat lanes
   *(an ES 1.00 shader declares none)*, then **the convergent lanes fill the
   scalar slots left unused in the interpolated vec4s** (`unused_interp_slots`
   counts the free components of started interpolated vec4s; the first
   that many convergent lanes, in layout order, take them and are
   **smooth**), and only the convergent lanes that remain are packed as
   **flat**, in vec4s of their own after the interpolated ones.
3. The vertex store of a lane that ended **flat** is `ffma(x, 0.0, x)`
   (`:4964-5002`, `build_convert_inf_to_nan`); a convergent lane that ended in a
   smooth slot is stored as the value, with no `FMA` (`flat_sep_vec4_bs51` words
   1-3 against word 4; `v86`, `c30`, `tex_uniform_w`). The compiler's
   `converted` loop (nir.rs, "a convergent varying promoted to flat") makes it for every
   lane that reads no attribute: that is the part to change.
4. Two lanes of one value are one slot (`deduplicate_outputs`, `:3253`):
   `v86`'s `c, c`; already in the compiler (`equal`, `push_equal`).

Checks of each reference against the rule (`unused` = free components of
the interpolated vec4s):

| reference | interpolated lanes | convergent lanes kept | unused | result |
|---|---|---|---|---|
| `kau_bs26` | `b` (4 lanes: the two constants stay stored, section 4 note) | `e` | 0 | `e` flat, a vec4 of its own at byte 16 |
| `vec4_bs51` | `b` (1) | `a.x a.y a.z a.w` | 3 | three smooth in the vec4 (words 1-3), one flat (word 4, byte 16) |
| `v86` | `b`, `a-5` (2) | `c` (the two equal lanes merged) | 2 | one smooth, a `.v3` |
| `c30` | `q*s` (1) | `d.z` (uniform `v`), `a` (uniform `y`); `d.y`, `d.w`, `b`, `f`, `i` are constants of the fragment shader, `e`, `n` unread | 3 | two smooth, a `.v3` |
| `tex_uniform_w` | `rgb` (3, behind a `tex`: divergent) | `a` (uniform alpha) | 1 | smooth, a `.v4` |
| `rnd13` | `c` (4) | `e`: **not read** (section 3.1) | | no flat lane at all |

Lane order inside the convergent group is the layout's (`generic_layout`:
vec4 first, then vec2, float, vec3, stable), which is `c30`'s `d.z` before
`a` (`d` is the vec4).

Consequences for the compiler (the old reading in brackets): there is no
"flat beside smooth refusal to lift" with a layout of its own: the layout is
this one rule, and the all-flat case (`varyconst_117/181/191/199`, no
interpolated lane, `unused = 0`) is its degenerate case, unchanged. [The
note's "a bare uniform copy moves to the fragment shader" is not what
`c30`, `v86` and `tex_uniform_w` show: the bare uniform `y`, `v`, `c`, `s_a`
are stored (`F16_TO_F32` of the FAU word, a store each). `rnd13`'s `e` is
not moved: it is not read (section 3.1).]

## 3. The shapes grouped into four mechanisms

42 cases, 18 references; the case counts are `mali-deqp-posvary.md` section 2
(HOST-ONLY at `9ac31f30`). Section 18 of `mali-deqp-families.md` rows 25, 36,
37 are the same 32 + 6 + 4.

| mechanism | cases | references |
|---|---|---|
| **M1 convergent lanes beside interpolated ones** (section 4) | 31 = 16 `flat_sep` + 13 `flat_lane` + 2 `tex_uniform_w` | `flat_sep_rnd13`, `flat_sep_kau_bs26`, `flat_sep_vec4_bs51`, `flat_lane_vec4_v86`, `flat_lane_vec4_c30`, `tex_uniform_w`, `tex_uniform_w_array` |
| **M2 `fract` of a local copy into a mediump colour** (section 5) | 6 (up to 22 with the 16 `367:FMA` cases of the same vertex shader) | `fract_highp_vec2`, `fract_highp_vec3`, `fract_mediump_vec2`, `fract_mediump_vec3`, `fract_lowp_vec2`, `fract_lowp_vec3` |
| **M3 a declared varying the vertex shader never writes** (section 6) | 4 (3 `Fail`, 1 `Pass`) | `unwritten_not_declared`, `unwritten_declared_unused`, `unwritten_declared_dead_read`, `unwritten_negative_vs` |
| **M4 a loop's phi in a varying** (section 7) | 1 (`NotSupported` on the board) | `loop_phi_array` |
| total | 42 | 18 |

### 3.1 The first thing to find: why `flat_sep_rnd13` is refused

`rnd13`'s fragment shader assigns `gl_FragColor = vec4(.., float(e) + float(a), ..)`
and then **overwrites** `gl_FragColor = c + b * vec4(..)`: `e` is dead, Mesa's
link stores `c` alone and the fragment reads `.v4` only. With the fragment's
reads known (`compile_vs_for_reads`, `compact_layout(reads)` leaves `e`'s lane
out of `vs.masks`), the loop at `nir.rs:5421-5500` never sees it, so the
refusal comes from somewhere else: (a) the fragment analysis counts `e` as
read, because the overwritten first assignment is not dead to it
(`nir_fs.rs` `read`, ~`:7900-7990`), (b) `vs_constants` at `mod.rs:1839` or the
vertex shader built alone (`reads: None`) for the program's first compile,
(c) `nir_fs.rs:8078`. *(reading)* the first action of the posvary-lower
session is an `eprintln!` at each of the five refusal sites (`nir.rs:5456`,
`:5471`, `:5492`, `nir_fs.rs:8078`, `mod.rs:1839`) naming the site, run on the
18 references; the answer decides whether `rnd13` (and the random cases of
that kind, an unknown share of the 16) is the dead-store analysis of M1 step
1 or the packing of step 2. Mesa's `bs26`, `bs51` and the lane shapes read
their uniform lanes for real.

## 4. M1: the convergent packing (31 cases)

### 4.1 Where it is refused (all five sites are one rule)

| site | what it does now | what it lacks |
|---|---|---|
| `nir.rs:5492` `if flat && smooth` | the vertex shader's refusal: `flat` set by `vs.uniform_only(n)` (`:5469`), by a constant lane of a phi (`phis`) or of a vector operation of uniforms (`kept_flat`), `smooth` by every other lane | the unused-slot fill rule |
| `nir.rs:5456`, `:5471` | a constant not `kept`/`late_fold`; a phi of uniforms (`!reads_attribute && !reads_tex && !reads_indexed`) | `:5456` stays (a constant is propagated to the fragment shader, `compile_fs_for_vs`); `:5471` is M4 |
| `nir_fs.rs:8078` `if flat && smooth` | the fragment's mirror: a lane is flat when `declared` flat or `f.prec.consts.flat(loc, comp)` (`link.rs` `VaryingConsts::push_flat`, set by the vertex build); `v.flat = flat` is one flag for every slot of the program | a per-slot flat flag; the layout loop places every lane in layout order in one pass |
| `mod.rs:1839` (`vs_constants`) | the vertex shader built "for a fragment that reads nothing" fills `VaryingConsts`; `consts.full()` refused | `push_flat` must record *convergent* lanes (flat or smooth is decided later, by the fragment's reads) |
| `nir.rs` `converted` loop (`ffma(x, 0, x)`) | made for every lane that reads no attribute and is not `divergent` | only for lanes the pack leaves flat |

### 4.2 Design

One function decides the layout for both stages, so they cannot disagree
(the vertex shader today decides in `compact_layout`/`generic_layout`
(`select.rs`), the fragment shader in `nir_fs.rs` `layout`, and the link
(`link.rs`, `VaryingConsts`) hands one the other's tags):

```text
select.rs: pack_lanes(lanes: &[Lane]) -> Packed
  Lane   { location, comp, class: Interp | Convergent | DeclaredFlat }   (layout order)
  Packed { word[lane] (scalar slot from VAR0.x), flat_slot[vec4], nslots }
  1. every Interp lane, in layout order: next word
  2. every DeclaredFlat lane: next word of a flat vec4 (an ES 1.00 shader has none)
  3. unused = words of the last started Interp vec4 left free (4 - n % 4, 0 when n % 4 == 0);
     the first `unused` Convergent lanes: next words (smooth)
  4. the remaining Convergent lanes: flat vec4s after the interpolated ones
```

Per mechanism step:

* **Vertex, `nir.rs` `build`:** the loop at `:5421-5500` classifies each
  stored lane (`Interp`: an attribute behind it, a `tex`, `divergent`, a
  `kept` constant of a mediump vector; `Convergent`: `uniform_only`, a
  `kept_flat` constant, a `phis` constant) and drops the `flat && smooth`
  refusal. `compact_layout` calls `pack_lanes` (it still takes the fragment's
  `reads` mask first, so a lane no one reads takes no word, `rnd13`). The
  per-varying `VaryingAt.offset`/`masks` model cannot hold a lane of
  one varying in two slots (`vec4_bs51`: `a.xyz` at words 1-3, `a.w` at word 4):
  give `Nir` a per-lane `word: [[u8; 4]; MAX_VARYINGS]` and make the store
  loop (the last loop of `build`, `word = v.offset / 4 + j`) iterate the lanes **in ascending
  word** (the references' store order) with `first` = `word % 4 == 0` as it
  does. `converted` (`ffma(x, 0.0, x)`) is made only for a lane whose word is
  in a flat vec4.
* **Link, `link.rs` / `mod.rs`:** `VaryingConsts::flat(loc, comp)` becomes
  `convergent(loc, comp)` (the existing `push_flat` tag, renamed or an added
  tag); `vs_constants` records every convergent lane, `at_fragment` maps it to
  the fragment's location. Equal lanes (`push_equal`) are unchanged.
* **Fragment, `nir_fs.rs` `layout` (`:7990-8080`):** the one-pass loop
  becomes two passes over the same lanes (Interp, then Convergent) through
  `pack_lanes`; `FsVarying.flat` is per slot (`v.flat = slot's lanes are all
  flat`), the index of a flat load is its byte offset (`index:0x10`); the
  `flat && smooth` refusal is dropped. A smooth slot holding a convergent lane
  is read as the other lanes (`.src_f32`), `v86`'s `IADD.v2u16 r1, r2.h00`
  duplication is the fragment's existing equal-lane handling (`placed`,
  `lane_of`).
* **The driver-side varying records**: `src/mali/program.rs`
  (`VaryingSlot { varying, comps, section: Generic, offset, mask }`) and the
  fragment resource table (`Attribute` descriptors of the varying buffer,
  `Varying allocation`, the format of a flat slot: `RGBA32UI` for the all-flat
  case, `mali-varying-constants.md`). `tests/mali_ref` compares only the three
  code regions, **not the descriptors** (`mod.rs`: `differences`), so the
  mixed table (a smooth `RGBA32F` vec4 plus a flat 32-bit slot at byte 16) is
  unchecked by the references as committed. The session must check where the
  descriptor's format and the `Varying allocation` come from for a program
  with a flat slot beside smooth ones (`mod.rs:1431`, `:1617`, `:1958`
  `fragment_reads`; `program.rs:120-144`), and compare it against the
  `Fragment resources` of one trace (`pan_ref_diff.py extract --draw`
  prints them; the `.trace` of the references is not committed, redraw
  `bs51` with `tools/mali/drm_shim_ref.sh run`). Expect the byte length
  (`varying_bytes`) to follow `vs.masks`' words (`bs51`: 20 bytes).

### 4.3 Steps (one session each, in this order)

1. **M1a (first, smallest): the site and the dead store.** The `eprintln!`
   of section 3.1; if `rnd13` is the dead `gl_FragColor` store, fix it where
   the fragment's reads are computed; `rnd13` passes `assert_matches` with
   no packing change.
2. **M1b: `pack_lanes`, vertex and fragment, scalar flat after a full smooth
   vec4** (`kau_bs26`: `unused = 0`, `e` flat at byte 16, `FMA x,0,x`
   kept; the vertex side's per-lane `word`, the fragment's per-slot flat).
   Un-ignores `flat_sep_kau_bs26` and `flat_sep_rnd13`.
3. **M1c: the fill** (`bs51`, `v86`, `c30`): convergent lanes into the free
   smooth words, no `ffma`, the split of a vec4's lanes across two slots,
   the `.v3` shrink. Un-ignores `flat_sep_vec4_bs51`, `flat_lane_vec4_v86`,
   `flat_lane_vec4_c30`.
4. **M1d: `tex_uniform_w`**: the `divergent` tex lanes are the Interp
   lanes and the uniform alpha the fill (`unused = 1`, one `.v4`). Probably
   free after M1c; if the vertex-texture path (`Kind::VsTex`, `reads_tex`,
   `nir.rs:5470`) classifies a lane after the `tex` as flat first, fix the
   classification. Un-ignores `tex_uniform_w`, `tex_uniform_w_array`
   (and the glmark2 `vtex_struct_sampler`, `vtex_struct_sampler_array`).

Forecast: 31 cases at most; the 29 `flat_sep`/`flat_lane` and 2 `tex` ones can
move to another refusal after this (the cases came from other texts for 21
of the 42, `mali-deqp-posvary.md` section 1), the random ones the most
(`shaders.random.*` 23 of the 32).

### 4.4 Pinned tests most at risk

* `tests/mali_deqp_posvary.rs`: `refusals_pinned` (a lowered reference leaves
  `REFUSED`); the seven M1 references move to a plain `#[test]` in the step
  that lowers them.
* `src/shaderc/online/mali/tests.rs`: `:1253` (`v = vec4(c, u, 1.0)` is now
  `c` smooth, `u` smooth, the constant in the fragment: a `.v3`) and `:7053`
  (`v = vec2(p.x, u)`: a `.v2`) are refusals pinned as "no reference draws
  it": they flip, and need Mesa references (draw `vec2(p.x, u)` and
  `vec4(c, u, 1.0)` with `drm_shim_ref.sh`; do not turn them into passes
  without) or must be rewritten as the posvary references' lane shapes.
  `:2360` (`fsload_qual1..3`: the fragment's refusal for a *declared*
  `flat` beside smooth) stays only if those shaders declare a qualifier; if
  they rely on the uniform-only rule they flip. `a_varying_lane_of_uniforms_alone_is_loaded_flat`
  (117) is the degenerate case: it must not change.
* `tests/mali_deqp_varylink.rs` (`varymerge_equal`, `gm2_ideas_equal_uniform_lanes`:
  equal uniform lanes merged into one flat lane, no interpolated lane) and
  `tests/mali_deqp_colourpair.rs` (`rnd_vs_uniform_flat`, `rnd_vs_vecop_fs_uniform`:
  constants kept flat beside uniforms, byte-equal today): any change of
  `kept_flat`/`phis` classification must keep their words; they are the
  regression set. `rnd_vs_vecop_kept` (the partial-varying link, pinned
  refused) can move.
* `tests/mali_glmark2_corpus.rs` and `tests/fixtures/mali_glmark2_verdicts.txt`
  (the pinned first refusals; regenerated in the same commit):
  `vtex_struct_sampler`, `vtex_struct_sampler_array` and `xlink` carry
  `a varying lane computed from no attribute` (`tools/mali/corpus-verdicts.txt`
  lines 130, 131, 134) and move; `partial_varying.vert` (`:82`, `VaryingForm`)
  does not (section 6).
* `varyconst_*` and `cube_vs` (the `ffma(x, 0, x)` conversion measured for
  the all-flat case) unchanged.

### 4.5 A note on the constants stored beside the vec4

`kau_bs26` stores the two constant lanes of `b` (`1.0`, `4.0`) and `fract_*`
and `loop_phi_array` store theirs, where `c30` leaves its constants to the
fragment shader. That is the compiler's existing `late_fold`/`kept` rule (a
constant the back end's fold made late, a constant lane of a mediump
vector, `nir.rs:5441-5456`); the references decide whether the existing rule
covers each, in the byte compare, not here.

## 5. M2: `fract` of a local copy (6 cases, up to 22)

The refusal text is `PositionLane` but the site is `nir.rs:2120`
(`alu_sized`: an operand is `Val::Undef`), which is not about
`gl_Position`; `mali-deqp-posvary.md` section 4 established the probes
(`f01`..`f12`: `fract` of a *local* vec2 copy of an attribute swizzle, written
by lanes into a mediump colour; `fract(a_in0.wy)` directly compiles).
*(reading)* The probable cause is `AluOp::Ffract` (`nir.rs:4869`): on lane
0 it makes `ffloor(lane(w, s[0], i))` for `i < width`, **with no guard for an
`Undef` lane**, where `Fsqrt` (`:4883`) and the compares (`:4890`) have one
(`if x(0) == Val::Undef => Val::Undef`). A local copy (`in0`, declared vec2)
reaches the op as the front end's four-lane value with lanes 2 and 3
`Undef` (the comment at `:4888` names that case for compares): `width` is then
4, `ffloor(Undef)` is `alu_sized`'s `Undef` operand. `f03`/`f11`, with no local, give a
two-lane operand. The fix: skip the `Undef` lanes in the `held` loop and
return `Val::Undef` for an `Undef` `x(0)` as `Fsqrt` does; **confirm first
by the `eprintln!`** naming the operation at `nir.rs:2120` (and rename the
message: it is wrong for every other `Undef` operand; use `Defect(IrIndex)` or
a new `MaliError`, with `tests.rs:1202` (`gl_Position.xy = p`, which really
is `PositionLane`, `nir.rs:5368`) kept).

After that the words of section 1 are the next differences, *(reading)* in
this order of likelihood: the fp16 `FROUND.v2f16.rtn` of a lane in both halves
(`emit.rs:126` has the row; the vertex walk's mediump path, `vec16.rs`,
must pair `(x, x)` with `.h00` and the two negated `FADD.v2f16`s with
`.h00`/`.h11`), the order `floor(w)` then `floor(y)`, the `MKVEC.v2i16` of a
constant half (`0x0.h0`) with a lane, and the `v4` attribute load for lane w
(the attribute-record note: the load width is by the highest lane read).
The highp references (`fract_highp_*`) differ only in staying at fp32 until
the narrowing. Tests: un-ignores the six `fract_*`; risk: `mali_deqp_mixprec`
and the `vsarith_fract.expect.json` unit test (the lane-by-lane `held`
schedule of the f32 `fract`, `nir.rs:4861-4867`) must stay byte-equal.
The 16 `367:FMA` cases (`angle_and_trigonometry.{acos,asin,atan,atan2}`) reach the
same text once the FMA family is lowered: this step's fix covers them, no
further reference is needed (the vertex shader is the same).
No descriptor or driver change.

## 6. M3: a varying the vertex shader never writes (4 cases)

Sites: `nir.rs:5421` (`varyings.get(v.location)... .ok_or(VaryingForm)`: the
declared output has no entry because no statement stores to it) and
`motion.rs:216`, `:225` (a moved expression reads a lane of it);
`nir.rs:5437` (`Val::Undef` in a stored lane) is the *partial* case and
stays. Mesa removes the varying whether or not the fragment shader declares it
(the three references have no `vs.varying` region, section 1), and a fragment
read of it is an undefined value that Mesa's NIR folds (`varying_4`:
`if (u_false) gl_FragColor = vec4(var); else vec4(1.0)` becomes the constant
`1.0`, the 7-word colour; the fragment words equal `_not_declared`'s).

Design:

1. `nir.rs`: before `generic_layout` (`:5384`) filter the vertex outputs to
   those with an entry in `varyings` (some store); an output with none is
   not a varying of this program (no `VaryingAt`, no mask). `vs.nvar` is 0
   for the three references: `vs.masks[..nvar].iter().any(..)` is false, no
   `Lea { out: 1 }`, so no `vs.varying` program (the code already skips it).
2. `mod.rs` `compile_vs_for_reads`/`vs_constants` and the link's pairing
   (`link.rs` `pair`, `reads_at_vertex_locations`): a fragment read of a
   location the vertex shader does not store is today "not written by the
   vertex shader" (a link refusal, `reads_at_vertex_locations` returns
   `None`). For the dead-read shape the fragment lowers the read as
   `Val::Undef` (not a load) and `nir_fs.rs` folds `bcsel(c, Undef, x)` to `x`
   (the `if`/`else` flattened into a `CSEL`); an `Undef` that reaches the
   colour unfolded stays refused (no reference, as `p20`, `p21` of the note's
   probes: a lane left out of a vec4 the fragment reads). This is the only
   fragment-side change of M3 and it is *(reading)* the larger half: the
   first two references (not declared, declared unused) need only step 1.
3. `unwritten_negative_vs` is a constant `gl_Position = vec4(1.0)` with no
   attribute. Its `vs.position` is the 18 words of section 1. The path is
   `ViewportConstW` (`nir.rs:5388`), exercised by no committed reference of
   this kind; the first run of its byte compare says if the compiler's
   `Fmul`/`Fadd` fold of constants gives `FADD.f32 u0.w0, u0.w1` and the
   integer `1.0` store; if not, that is a fold fix in `nir.rs` (not
   an extra mechanism). Its dEQP case is a `Pass` whatever the vertex shader
   does (the dEQP fragment shader does not compile): the case count of this
   shape is 3 effective.

Tests: un-ignores the four `unwritten_*`; `tests/mali_deqp_varylink.rs`
`unwritten` (a pinned `VARYING` refusal, `deqp_varylink_unwritten`: shape U of
`mali-deqp-varylink.md`) is the same site and **will flip**: it needs
its own look (which of the three it is) in the same commit. Stay refused,
and pinned as they are: `tests.rs:1250` (`v.xy = p.xy`, a partial vec4),
`:2938` (`vsarith_partial_varying.vert`), `corpus-verdicts.txt:82`
(`partial_varying.vert`). No descriptor change (no varying program: the
program's `varyings` is 0).

## 7. M4: a loop's phi in a varying (1 case)

`nir.rs:5470-5474`: `!reads_attribute && !reads_tex && !reads_indexed` is the
arm that refuses it (*reading*, the note confirmed it by the lane's form
only). Mesa stores the loop result as an ordinary smooth lane (plain
`F16_TO_F32` + `STORE`, **no `FMA x,0,x`**, section 1): its source is a read
of a thread-local array at a loop index, which the divergence analysis does
not take as convergent. So the lane is `Interp` (the `divergent` mask, the one
`tex` uses), and `reads_indexed` must include a function-temporary array read
at a run-time index. The three constants stored beside it follow the same
`kept` rule as in 4.5. This reference needs the loop with an `int` uniform
bound at fp16 (`ICMP_OR.s32.ge.m1`, `LOAD.i16 [thread_local_pointer + ..]`):
it belongs to the loops and integer items (`tests/mali_deqp_loops.rs`,
`mali_deqp_intval.rs`; plan items loops-ref, intval-rest), so schedule it
**last**: when the loop compiles, this refusal is the only thing left; if the
loop lowering is not there, the case moves to another text. The array
initialisation in `vs.position` (40 `STORE.i16`) is the same: both programs
carry it (the position program has no DCE of the local array in Mesa).

## 8. Order, and what posvary-lower needs to know

| step | mechanism | cases | files | un-ignores in `tests/mali_deqp_posvary.rs` |
|---|---|---|---|---|
| 1 | M3 (unwritten) | 4 | `nir.rs`, `nir_fs.rs`, `link.rs` | `unwritten_not_declared`, `unwritten_declared_unused`, `unwritten_declared_dead_read`, `unwritten_negative_vs` |
| 2 | M2 (`fract_local`) | 6 (+16) | `nir.rs`, `vec16.rs`, `mod.rs` (message) | the six `fract_*` |
| 3 | M1a, M1b | 16 (part) | `nir.rs`, `select.rs`, `nir_fs.rs` | `flat_sep_rnd13`, `flat_sep_kau_bs26` |
| 4 | M1c, M1d | 15 | same + `link.rs` | `flat_sep_vec4_bs51`, `flat_lane_vec4_v86`, `flat_lane_vec4_c30`, `tex_uniform_w`, `tex_uniform_w_array` |
| 5 | M4 | 1 | `nir.rs` | `loop_phi_array` |

M1 is the largest (31 of 42) but the riskiest; M3 and M2 are small and
independent and can go first: they unblock 10 cases without touching the
layout code. If one session must pick one thing, take M1b+M1c together: they
are one rule (section 2) and splitting the byte compares across two
sessions gains nothing.

Every step is checked the project's way: the byte compare of
`tests/mali_deqp_posvary.rs` for the references it un-ignores (feature set
of its header:
`--no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table`);
the unit tests of `src/shaderc/online/mali/tests.rs` named in 4.4 and 6; and
the host dEQP forecast (`tools/mali/deqp-host-measure.sh`, section 7 of
`mali-deqp-posvary.md`) against the 42-case list for each step. Pinned
files that move (`tests/fixtures/mali_glmark2_verdicts.txt`,
`tools/mali/corpus-verdicts.txt`, the `REFUSED` table, `mali-deqp-posvary.md`'s
counts) are regenerated in the same commit. No `tools/check.sh`.

What is not covered by a reference and must be drawn before it is lowered:
a lane left out of a vec4 the fragment reads (`p20`, `p21`); a uniform lane
beside attribute lanes in a vec2 (`tests.rs:7053`'s shape) and `vec4(c, u,
1.0)` (`:1253`), unless the lowering's own tests are the posvary references
alone; a program with **two** flat vec4s or a *declared* ES 3.00 `flat`
beside a smooth one (not reached by the 42 cases).

## 9. Measured at the integration tip `3497799a`, HOST-ONLY

Method: the 18 references through `mali_ref::compile_pair` (what
`refusals_pinned` runs), with a temporary `eprintln!` at every site that
returns `PositionLane`, `VaryingForm` or `VaryingNotFromAttribute`
(`nir.rs`, `nir_fs.rs`, `mod.rs`, `motion.rs`) and at `compile_vs_for_reads`,
printing the site, the vertex layout's masks and the reads the vertex shader
is built for. The patch and the scratch test were removed again; `git diff`
of `src/` and `tests/` is empty.

| reference | site that refuses | state there |
|---|---|---|
| `flat_sep_rnd13` | `nir.rs:5492` `flat && smooth` | layout `(0, 4 lanes, byte 0), (1, 1 lane, byte 16)`, masks `[15, 1]`; built for reads `[(0, 15), (1, 1)]` |
| `flat_sep_kau_bs26` | `nir.rs:5492` | masks `[15, 1]` (vec4 at 0, float at 16); reads `[(0, 1), (1, 15)]` (the fragment's own numbering) |
| `flat_sep_vec4_bs51` | `nir.rs:5492` | three varyings `(0, 4, byte 0), (1, 1, 16), (2, 1, 20)`, masks `[15, 1, 0]`; reads `[(0, 15), (1, 1)]` |
| `flat_lane_vec4_v86` | `nir.rs:5492` | one varying, mask `13` (x, z, w; y is `push_equal` of x); reads `[(0, 15)]` |
| `flat_lane_vec4_c30` | `nir.rs:5492` | seven varyings, masks `[5, 1, 0, 1, 0, 0, 1]`; reads `[(0, 1), (2, 5), (3, 1), (6, 1)]` |
| `tex_uniform_w`, `_array` | `nir.rs:5492` | one varying, mask `15`; reads `[(0, 15)]` |
| `loop_phi_array` | `nir.rs:5471` (the phi arm), not `:5492` | |
| `fract_*` (6) | `nir.rs:2120` only | `op = Ffloor, bits = 32, srcs = [Undef, Undef, Undef]`; `nir.rs:5368` is never reached |
| `unwritten_*` (4) | `nir.rs:5421` only | |

What this changes in sections 1 to 8:

1. **M2 is confirmed to the operation.** The `Undef` operand is the `Ffloor`
   the `Ffract` arm makes at lane 0 (`nir.rs:4869-4876`, every lane `i <
   width`), as section 5 read it; the dEQP shader has no `floor`, and the
   probe in the note `mali-deqp-posvary.md` (`f11`, no local, compiles) fits.
   The guard of section 5 (skip `Undef` lanes in the `held` loop) is the first
   edit; the `eprintln!` of section 5 is no longer needed.
2. **M3 has one site**, `nir.rs:5421`, for all four references; `motion.rs`
   and `nir.rs:5437` are not reached. The edit of section 6 step 1 (filter the
   outputs with no store before `generic_layout`) is the whole vertex side;
   whether it also needs the fragment side's `Undef` fold (section 6 step 2)
   is for `unwritten_declared_dead_read` to say: it is the one whose fragment
   reads `var`.
3. **Section 3.1 is half answered.** `rnd13` and `c30` reach `:5492` with a
   vertex shader built for reads that **include lanes Mesa's fragment shader
   does not read**: `rnd13`'s `e` (mask 1 of the second varying; its only use
   is in a `gl_FragColor` assignment the next assignment overwrites) and
   `c30`'s `e` and `n` (read in an overwritten assignment and in an expression
   statement with no effect). The packing rule alone does not give Mesa's
   words for them: the dead lanes must leave the reads first, else `e` is an
   extra `Interp` lane in `c30` (shifting the fill) and a flat lane in `rnd13`
   (the `ffma` store and a flat load Mesa does not have). `kau_bs26` and
   `bs51` are not affected (`kau`'s `e` is read for real; `bs51`'s dead `d`
   is already mask 0). Where the reads come from was **not** found: an
   `eprintln!` in `nir_fs.rs` `layout` (at its `decls` line, `:7974`) printed
   nothing for any of the 18 pairs before the vertex refusal, so the
   reads handed to `compile_vs_for_reads` do not come from a run of that
   function for these fragment shaders (`vs_for_fragment`, `backend.rs:2121`:
   `link_consts_of` then `compile_and_publish_linked`, or `partial_reads` of
   the stand-alone record). Finding which record carries `e` is step M1a of
   section 4.3, now with the symptom named: the record's mask is the
   fragment's own, not the link's.
4. **The flat load may share a vec4 with smooth ones.** The ES 3.00
   references `fsload_qual1..3` (`tools/mali/testdata/fsload_qual*.expect.json`,
   pinned at `tests.rs:2360` as a fragment refusal `VaryingNotFromAttribute`)
   have `out vec2 p; flat out vec2 q;`: the fragment loads
   `LD_VAR_BUF_IMM.f32.v2.src_f32 @r0:r1, r61, index:0x0` and
   `LD_VAR_BUF_IMM.f32.v2.src_flat32 @r2:r3, 0x0, index:0x8`, the vertex
   stores four words in order. A *declared* flat lane is therefore the next
   scalar word after the interpolated ones, **not** a new vec4 (section 4.2's
   `pack_lanes` step 2 says "next word of a flat vec4"; and its per-slot flat
   flag cannot say it): the load is **per run of equal qualifier**, `.vN` of
   the run's width at byte `4 * first word`, `.src_flat32` for the flat run. For
   the 42 cases the two readings give the same bytes (the flat run starts at a
   vec4 boundary there: `kau_bs26` 4 interpolated words, `bs51` after the fill
   of 3), so the per-run form is the safe one to build and it makes
   `fsload_qual1..3` byte-comparable too, a pin that flips at
   `tests.rs:2360` (the three references exist; no case count: they are ES 3.00
   and not in the 42). What a *convergent* lane does when it lands after a
   flat run that is not at a vec4 boundary is not measured.
5. **The record model.** `link.rs` `pair` (`:420-470`) finds one `(offset,
   mask)` per varying (`.find(|s| s.varying == Some(loc))`) and compares them
   between the stages; `compile_module` (`mod.rs:1617`) pushes one
   `VaryingSlot` per `f.reads` entry; `src/mali/program.rs:120` only takes the
   largest `offset + 4 * comps`. A varying whose lanes are in two places
   (`v86`: `z, w` at words 0, 1 and `x`/`y` at word 2; `bs51`: `a.x..a.z` at
   words 1-3 and `a.w` at 4) therefore needs **one record per run** (the same
   `varying`, disjoint `mask`s, each `offset` its first word) and `pair` must
   collect all entries of a location and compare the sets; the rank of
   `VaryingOrder` takes the lowest offset. This is compiler-side only: the
   frame's use (`program.rs`) is unchanged, and no driver table gains a field
   (the record has no interpolation field today either, and the all-flat
   `varyconst_117` shipped without one, `mali-varying-constants.md`).
6. **Attribute records follow the stored lanes.** `c30`'s varying program
   loads one attribute (`LD_ATTR_IMM.f32 ... index:0x1`; the dead `t`, `r` and
   the `q*s` partners are not loaded and take no index) and `rnd13`'s loads
   `o` at index 1 alone: the attribute numbering of
   `mali-attribute-records.md` is the bits of `inputs_read` of what survives.
   Dead lanes (item 3) must be removed *before* `vs.counted = vs.inputs_read()`
   (`nir.rs:3938`, `:3974`), or the attribute table of `c30` is longer than
   Mesa's and the byte compare differs in the load indexes.
7. **The classification for `loop_phi_array`** is as section 7 says (`Interp`,
   plain store); the constants stored after it are the `kept` rule of
   section 4.5 (the fragment reads the mediump `v_color` whole, as in
   `kau_bs26`). It also needs the thread-local array and the int-uniform
   loop, which no posvary step owns; keep it last.

Revised first step of M1 (section 4.3, M1a): find which fragment record gives
`rnd13`/`c30` their dead lanes, and make those lanes leave `reads`; check
`kau_bs26`/`bs51` stay unchanged; then `rnd13` should reach the compare with
no packing change, and `c30`'s `e`/`n` drop out of its seven-varying layout
(the fill then has two lanes to place, as section 2's table says).
