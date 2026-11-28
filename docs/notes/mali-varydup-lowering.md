<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the residue of VaryingDuplicate, AttributeNumbering and LinkedVertexNotLowered

Plan #387, item varydupres-refs (ROADMAP stage 5, issue #7). Section 19.1 of
`docs/notes/mali-deqp-families.md` (tip `e9033663`) counts three small texts
with no item: `VaryingDuplicate` 50, `AttributeNumbering` 12 and
`LinkedVertexNotLowered` 8. Their shapes were studied before by misc2
(`mali-deqp-misc2.md`), board-baseline-rows (`tests/mali_deqp_misc3.rs`)
and varylink2 (`mali-deqp-varylink2.md`). This note measures them again at
the integration tip and maps every case to a shape, a refusing site and a
lowering step. It also adds Mesa 26.2.1 references for the shapes no earlier
reference covers: `tools/mali/testdata/deqp_varydup_*`, pinned by
`tests/mali_deqp_varydup.rs`. Nothing under `src/` changed.

The texts (`MaliError`, `src/shaderc/online/mali/mod.rs`; each ends in "is
not lowered yet"):

| key | `MaliError` | text |
|---|---|---|
| dup | `VaryingDuplicate` | two varying lanes of equal value (Mesa's link merges them) |
| attrnum | `AttributeNumbering` | a vertex shader whose attribute numbering waits for a fold Mesa's back end makes after it numbers the locations, of a form the walk does not model |
| bind | `LinkedVertexNotLowered` | the Valhall back end of a vertex shader at the link's attribute locations |

## 1. How it was measured (HOST-ONLY)

- **Tip: `349f989f5392bea65c20e7663289e86481e521e0`** (the integration tip
  this item started from; `src/shaderc/online/mali/` as at that commit).
- CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, the
  `HOST_DEQP_CTS` tree of `tools/cts/deps.toml`
  (`tools/cts/deps-env.sh --build HOST_DEQP_CTS`, into
  `$CONNECTED_DEPS_DIR/VK-GL-CTS-host`).
- Cases: the 10,597 cases of the slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608` (the script's
  default), as in section 19.
- Mesa: 26.2.1 from the tarball of `third_party/mesa-26.2.1/UPSTREAM`
  (sha256 checked), the shim built by `tools/mali/drm_shim_ref.sh build`
  into `$CONNECTED_DEPS_DIR/mesa-shim`. Self-check: `deqp_ref.sh
  --instrument shim` of `deqp_misc2_attrnum_dead_attrs` gives an `expect.json`
  byte-equal (`cmp`) to the committed one, and `nir_refs.sh` gives its
  `.nir.sha` unchanged (`SAME`).

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" --out "$PWD/build/measure" --stage build
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" --out "$PWD/build/measure" --jobs 3 --chunk 100 --stage run
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" --out "$PWD/build/measure" --stage extract
```

All 10,597 cases began. 106 chunk processes exited 1 (dEQP's "not every case
passed"), and none crashed. 4,709 cases meet a compile or link refusal (4,716
at `e9033663`). **70 of them carry one of the three texts first**, the same
count per text as section 19.1: dup 50, attrnum 12, bind 8. The ranking rule
is section 19's: one text per case, the first refusal, read from
`measure.tsv`'s `host_refusal`. The `next_family` column is empty for all 70,
so the host log names no later refusal. Board status (NanoPC-T6 at
`37305608`): 61 Fail, 9 NotSupported, 0 Pass.

**The tip's manifest does not load.** `capi/Cargo.toml` has the key
`vivante-st` twice (lines 41 and 85). Commits `75a7d901` and `110080eb`
(2026-11-27) each added it, and origin/main `c28470db` has the same
duplicate. Every cargo command in the workspace then stops with "duplicate
key" (the measure's library build, and `cargo test`). For the measure and the
test run alone, line 85 was deleted in the working tree and restored
afterwards. That fix is not part of this change (section 6).

**Refusing sites.** Each of the 62 non-bind cases' pairs (from the
measure's `programs/`) and each reference were compiled through
`tests/mali_ref::compile_pair` in a throwaway worktree that printed a line
at each `return` of the two texts. The worktree was outside the result and
deleted afterwards. Each case stops at exactly one site, and it is the site
of its shape's reference. The 8 bind cases have one site only:
`compile_vs_mapped_blocks` (`mod.rs:2513`) refuses every vertex shader at
link-chosen attribute locations.

| site (at 349f989f) | what it refuses |
|---|---|
| `nir_fs.rs:14879` (`layout`) | a read lane the link merged into an earlier lane of the same input, while the input also has a distinct read lane (`merged != 0 && read != merged \| first`) |
| `nir_fs.rs:14870` (`layout`) | a read lane the link laid out at the place of another input's lane |
| `nir.rs:7685` (`build`) | a varying lane whose (origin, value) key a stored lane already has (`seen`) |
| `nir.rs:5350` (the late-fold check) | `depends & !vs.output_inputs != 0`: a lane's constant, flat bit or class changes between the first and the second walk, and depends on an attribute no output lane of the first walk reads |
| `mod.rs:2513` (`compile_vs_mapped_blocks`) | every vertex shader compiled at the link's attribute map |

## 2. Every case, by shape

| shape | text, site | cases (Fail / NS) | dEQP cases (`dEQP-GLES2.functional.`) | reference |
|---|---|---:|---|---|
| `swz_mixed`: a repeated lane beside a distinct lane of one attribute vector (`grg`, `yyw`, `abba`, ...) | dup, `nir_fs.rs:14879`, at the link | 33 (33 / 0) | `shaders.swizzles.vector_swizzles.mediump_vec{2,3,4}_*_vertex`: vec2 `grg rgrr stss tst xyxx yxy`, vec3 `bbg grg ppt rbr rggr sps stts tst xyyx xzx yxy zzy`, vec4 `aaag abba bbab gga ppqp qppq qqqt rgrr stss ttq wwwy wzzw xyxx yyw zzwz` | `deqp_varylink2_swz_mixed` |
| `prec_mixed`: the same with arithmetic lanes (`vec4(in0, 2.0*in0, -in0, -in0)`) | dup, `nir_fs.rs:14879`, at the link | 1 (1 / 0) | `shaders.linkage.varying_differing_precision_3` | `deqp_varylink2_prec_mixed` |
| `mat2`: a matrix varying of one scalar (`mat2(in0)`, `mat4(in0)`) | dup, `nir_fs.rs:14870`, at the link | 6 (6 / 0) | `shaders.conversions.scalar_to_matrix.{bool,float,int}_to_mat{2,4}_vertex` | `deqp_varylink2_mat2` |
| `loop_bcast`: `vec3(res)` of a value a run-time loop carries | dup, `nir.rs:7685`, vertex shader alone | 9 (0 / 9) | `shaders.indexing.vector_subscript.vec{2,3,4}_{component,direct,static_subscript}_write_dynamic_loop_subscript_read_vertex` | `deqp_varylink2_loop_bcast` |
| `const_cf` (**new**): constant conditions leave a varying of constant lanes, two pairs equal (`j = vec4(1.0, 0.0, 1.0, 0.0)`) | dup, `nir.rs:7685`, vertex shader alone | 1 (1 / 0) | `shaders.random.conditionals.combined.85` | **`deqp_varydup_const_cf`** |
| `zero_lanes`: an attribute read only through a product with `0`, `0.0` or `false` | attrnum, `nir.rs:5350` | 5 (5 / 0) | `shaders.random.conditionals.vertex.{25,95}`, `.scalar_conversion.vertex.{69,97}`, `.swizzle.vertex.44` | `deqp_misc2_attrnum_zero_lanes` |
| `dead_attrs`: the zero is `-0.0`, and other attributes are read only by a dead local | attrnum, `nir.rs:5350` | 3 (3 / 0) | `shaders.random.basic_expression.vertex.{49,75}`, `.conditionals.vertex.75` | `deqp_misc2_attrnum_dead_attrs` |
| `dead_global` (**new**): `zero_lanes` and a `-0.0` lane, with attributes read only by dead writes of a global (`f = f * vec2(h, d); f = g;`) | attrnum, `nir.rs:5350` | 1 (1 / 0) | `shaders.random.basic_expression.vertex.19` | **`deqp_varydup_attrnum_dead_global`** |
| `dead_output` (**new**): the varying is written two or three times; attributes (and a uniform) are read only by the overwritten writes; the last write is a product with zero lanes | attrnum, `nir.rs:5350` | 2 (2 / 0) | `shaders.random.scalar_conversion.vertex.61`, `.scalar_conversion.combined.25` | **`deqp_varydup_attrnum_dead_output`** (`vertex.61`; `combined.25` is the same shape with the same pass-through fragment shader) |
| `uniform_zero` (**new**): `dead_output` whose last write also has a uniform times `false` and an attribute times `0.0` beside live attribute lanes | attrnum, `nir.rs:5350` | 1 (1 / 0) | `shaders.random.scalar_conversion.vertex.63` | **`deqp_varydup_attrnum_uniform_zero`** |
| `bind`: two attributes bound to one location, selected by a `bool` uniform | bind, `mod.rs:2513`, at the link | 8 (8 / 0) | `attribute_location.bind_aliasing.cond_{float,vec2,vec3,vec4,mat2,mat3,mat4}`, `.max_cond_mat4` | `deqp_misc3_bind_cond_*` (8, one per case) |
| | | **70 (61 / 9)** | | |

The counts by text: dup 50 = 33 + 1 + 6 + 9 + 1; attrnum 12 = 5 + 3 + 1 +
2 + 1; bind 8. Compared with the earlier notes, every case is the same as
varylink2's (cc89a1cf) and misc2's (section 18), except for five. `const_cf`
reached dup only after its earlier refusal was lowered. `dead_global`,
`dead_output` (2) and `uniform_zero` were not attrnum cases in misc2's
count of 8.

Every new pair is the case's pair verbatim from the measure's `.qpa`
(`programs/<hash>.{vert,frag}`), and the `.license` beside each file names
the case. Each was drawn with `tools/mali/deqp_ref.sh <vs> <fs>
tools/mali/testdata/deqp_varydup_<shape> --instrument shim` (G610,
`--uniform-values`). Its `.nir.sha` comes from `tools/mali/nir_refs.sh
'deqp_varydup_*'`. At the tip all four are refused with their shape's text,
at their cases' site (`refusals_pinned`).

## 3. What Mesa 26.2.1 draws for the new references

These are read from each `expect.json`'s disassembly. "Store" is the IDVS
varying program's `STORE.i32.estream`. All four `vs.position` programs are
the usual 24 words.

| reference | regions (words: position, varying, fragment) | Mesa's varying program |
|---|---|---|
| `attrnum_dead_global` | 24, 19, 7 | lane x `FMA.f32 r0, 0x34000000.neg.h1, d, -0` (`-0.25 * d`, `d` from `LD_ATTR_IMM index:0x2`); y and z `IADD_IMM #0x0` (`-0.25 * -0.0` and `0.0 * b` fold to `+0.0`); w `#0x3E800000` (0.25). One attribute load |
| `attrnum_dead_output` | 24, 18, 7 | four constants (`#0x3F600000` = 0.875, `#0x0` three times), no `LD_ATTR` |
| `attrnum_uniform_zero` | 24, 19, 7 | lane x `g` (`LD_ATTR_IMM index:0x3`, stored as it is, `* float(true)` gone); y `#0x0` (uniform `f * false`); z `FMA.f32 u5.w1, b, -0` (`0.75 * b`, `index:0x2`); w `#0x0` (`0.0 * a`). Two loads |
| `const_cf` | 24, none, 7 | **no varying program**. `j` folds to `(1, 0, 1, 0)`, and the link moves it into the fragment shader. It writes the colour from FAU (`MOV.i32 r0, u2.w0`, `MOV.i32 r1, u2.w0`, `u2.w0 = 0x00003c00`: fp16 `(1.0, 0.0)` twice) after a constant `ATEST` |

All three attrnum fragment programs are the pass-through
`LD_VAR_BUF_IMM.v4`, two `V2F32_TO_V2F16`, then `BLEND`.

**The numbering rule.** This settles the question misc2 section 6 left
open, how Mesa numbers 1, 2, 4. The attribute indices Mesa loads fit one
rule in all five references. GLSL IR's dead-code passes first remove the
attributes that only dead code reads: overwritten varying writes, dead
locals and dead global writes. The rest are numbered in declaration order,
`dEQP_Position` first. The NIR folds `x * ±0.0 → +0.0` (and `false` as
0.0) come after that numbering. They drop an attribute's load but keep its
index:

| reference | declared | dead before numbering | numbered | loaded |
|---|---|---|---|---|
| `misc2_attrnum_dead_attrs` | g b c d e f | b, c (dead local) | g 1, d 2, e 3, f 4 | g, d, f (`e` only `* -0.0`) |
| `varydup_attrnum_dead_global` | b d g h | g, h (dead global writes) | b 1, d 2 | d (`b` only `* 0.0`) |
| `varydup_attrnum_uniform_zero` | k i a b g | k, i (overwritten writes) | a 1, b 2, g 3 | b, g (`a` only `* 0.0`) |
| `varydup_attrnum_dead_output` | h g c d | h, g, d (overwritten) | c 1 (inferred: nothing is loaded, so no index shows) | none (`c` only `* 0`) |

## 4. Steps

Each step is one mechanism at one site. Every step must byte-compare with its
references. The forecast is HOST-ONLY: a case that leaves its text may meet
another text first after it (`next_family` is empty for all 70, so the log
names none), and the 9 `loop_bcast` cases are NotSupported on the board.

| step | shapes | site | mechanism (Mesa's layout) | cases | dEQP groups | references |
|---|---|---|---|---:|---|---|
| VD1 | `swz_mixed`, `prec_mixed` | `nir_fs.rs:14879`, and `nir.rs:7685` for the program's vertex shader | `nir_opt_varyings` deduplicates output lanes that resolve to one scalar, also beside a distinct lane of the same vector. The vertex program stores each distinct lane once. The fragment shader loads the compacted vector and copies the loaded lane into the merged ones (as `swz_xx` already does). Let the merged mask hold beside distinct lanes, rather than refusing | 34 | `shaders.swizzles` 33, `shaders.linkage` 1 | `deqp_varylink2_swz_mixed`, `_prec_mixed` |
| VD2 | `mat2` | `nir_fs.rs:14870` | the GLSL linker packs a matrix varying into one vec4 (`lower_packed_varyings`, the `packed:` name). None of its lanes is merged or propagated: four stores, the 0.0 included. Matrix varying lanes stay out of the equal classes and constant propagation in both stages | 6 | `shaders.conversions` 6 | `deqp_varylink2_mat2` |
| AN1 | `zero_lanes`, `dead_attrs`, `dead_global`, `dead_output`, `uniform_zero` | `nir.rs:5350` (and the numbering in `nir.rs`) | section 3's rule: drop the attributes that only dead code reads (overwritten output writes, dead locals and dead global writes) **before** numbering. Number the rest in declaration order. Then fold `x * ±0.0 → +0.0` (`false` as 0.0, a uniform factor too) without renumbering. This replaces the late-fold check | 12 | `shaders.random` 12 | `deqp_misc2_attrnum_zero_lanes`, `_dead_attrs`, `deqp_varydup_attrnum_dead_global`, `_dead_output`, `_uniform_zero` |
| VD3 | `loop_bcast` | `nir.rs:7685` | the lanes of one vector operation over a loop phi (`f2f32` of the fp16 sum broadcast to `vec3`) are distinct keys to `nir_opt_varyings`. Three stores of one value, not merged. Key such lanes by their component, not their value. Check `fsload_dup_phi` (`tests.rs`, pinned refused) against `loop_bcast` first | 9 | `shaders.indexing` 9 (NS) | `deqp_varylink2_loop_bcast` |
| VD4 | `const_cf` | `nir.rs:7685`, vertex shader alone | the lanes are constants after the conditions fold. At the link Mesa propagates them into the fragment shader: no varying program, and a constant colour from FAU. The shader compiled alone has no Mesa counterpart. It must not refuse equal constant lanes: store them, or leave them to the link's constant pass (`varyconst_*`). The pair must then give the reference's words | 1 | `shaders.random` 1 | `deqp_varydup_const_cf` |
| LV1 | `bind` | `mod.rs:2513` | walk the vertex shader at the link's attribute map (`ShcAttribMap`), not in declaration order. Aliased attributes each load from the shared location; the `bool` uniform selects between them. misc2 section 4 step 6 estimates two sessions. The 6 `Defect(Encode)` cases of the same group (`deqp_defects_encode_*`) follow it | 8 | `attribute_location.bind_aliasing` 8 | `deqp_misc3_bind_cond_*` (8) |
| | | | | **70** | | |

**Order: VD1, VD2, AN1, VD3 + VD4, LV1.**

- VD1 moves the most cases (34).
- VD2 is the same `layout` function and one small rule, so it goes next.
- AN1 (12) is independent of the varying layout. It is one session, and its
  five references give the whole rule.
- VD3 and VD4 share the `seen` key at `nir.rs:7685`. VD3's 9 cases are
  NotSupported on the board, and VD4 has 1, so they come late.
- LV1 is last. It has the highest cost (a new walk at the link's map) for 8
  cases, plus the 6 `Defect(Encode)` neighbours after it.

None of the three texts is at or above the 100 line, before or after these
steps.

The trains not yet landed (lower-train-2 `13259474`, lower-vsloop
`8180095c` and `cbb6b45b`, lower-vsconst `2c0c6596`, lower-fsint
`e8327e42`, lower-mixprec3 `81b3ce19`) change no line that names these three
texts (`git diff HEAD...<commit> -- src/shaderc/online/mali`). Their
inflow can still bring new cases to them, as it brought `const_cf`.

## 5. Test

```text
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varydup
```

`tests/mali_deqp_varydup.rs` has `REFUSED` (all four, whole text),
`DIFFERS` and `LOWERED` (both empty), and one `#[ignore]`d byte-compare per
reference. `references_parse` checks the files, each `.license` and the
fragment word count. `refusals_pinned` checks each outcome. At 349f989f
(with the manifest fix of section 1 in the working tree): 2 passed, 4
ignored. The step that lowers a shape moves its entry and drops its
`#[ignore]` in the same commit, as `tests/mali_deqp_intval5.rs` does. The
existing references a step needs keep their pins in
`tests/mali_deqp_varylink2.rs`, `tests/mali_deqp_misc2.rs` and
`tests/mali_deqp_misc3.rs`.

## 6. Found outside this item

- `capi/Cargo.toml`'s duplicate `vivante-st` key (section 1) breaks every
  cargo command at the integration tip and on origin/main `c28470db`. It
  is the "two green pull requests, red main" failure of `CLAUDE.md`, and it
  needs its own one-line fix (delete line 85 and its comment, lines 83-84).
- The G310 equality of the four new references was not checked here
  (`tools/mali/g310_check.sh`, the g310-equiv items).
