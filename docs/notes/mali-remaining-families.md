<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the refusal families no other step-4 item covers

Plan #156 item `triage-other`, issue #33. State: tip `7ad18d27`, newest merged
T6 record `tools/cts/log/2026-10-28-final`, `tools/mali/corpus-verdicts.txt` as
pinned. The families here are the corpus refusals that are none of control
flow, discard, arithmetic, int/bool, mixed precision, texture look-ups,
fragment outputs, fp16 vertex uniforms or VS texture lanes.

## Sources, families and the sweep cases that use them

Mapped the way `tools/mali/sweep_predict.py` does (the `run_case` switch of
`driver/tools/pvrlab/gl_test.c`, the `gl_test_*_src` symbols of the case
function, `ENTRIES` of `gen_gl_test_shaders.py`; `ES100_TWINS` applied).
`tools/mali/fvg_refusals.py` reports 36 ok, 0 refusals for femtovg: **no
family here is femtovg's** (no #31 text).

| family (refusal text) | sources | sweep cases (2026-10-28-final) |
|---|---|---|
| F1 chain (`three or more additions or multiplications in a chain`) | `es100/matdyn.frag`, `es300/texoffset.frag` | **80** (matdyn: REFUSED); texoffset: none |
| F2 uniform at a run-time index (`a uniform read at a run-time index other than ...`) | `es100/dynvec4.frag`, `es100/uniform_idx.frag`, `es300/uniidx.frag` | none |
| F3 uniform type (`a uniform other than a float, vec2, vec3, vec4 or mat4`) | `es100/light_amount_0_16.vert`, `light_amount_2_2.vert`, `sharedint.vert` | none |
| F4 local count (`more than 32 local variables`) | `es100/tmparr_const.{vert,frag}` | none |
| F5 varying lanes (`two varying lanes of equal value`, `a varying lane computed from no attribute`) | `es100/discard_dynloop.vert`, `va_vec2.vert`; `vtex_struct_sampler.vert`, `vtex_struct_sampler_array.vert` | none |
| F6 vertex loop (`a loop in a vertex shader`) | `es100/loop_nested.vert`, `loop_uniform.dowhile.vert` | none |
| F7 mediump fragment loop (`a loop Mesa keeps ... in a mediump fragment shader`) | `es100/loop_nested{,.while,.dowhile}.frag`, `loop_uniform.frag`, `dynidx_loop.frag`, `ret_dynloop_never.frag`, `discard_dynloop.frag` (`contloops.frag` is the guard form, `GuardMediump`) | none |
| F8 gl_FragColor lane (`a gl_FragColor lane other than ...`) | `es100/ifloop.frag`, `gltest/texcubenorm_fs.frag` | none (`texcubenorm_fs` is case 134's symbol, 134 is NO-CASE) |
| F9 texture coordinate (`a texture coordinate other than ...`) | `es100/xlink.frag` | none |
| F10 constant product (`a constant product in a shape Mesa's algebraic rules move or fold`) | `es100/mvstruct.vert`, `xlink.vert` | none |
| F11 comparison (`a comparison other than ...`) | `es100/sharedint.frag` | none |

Ranked: **F1 first** (the only one that blocks a sweep case: 80, and it
blocks it first, before the mixed-precision and indexed-local refusals of
`mali-uniform-types.md` section 7), then by corpus sources: F7 (7 sources),
F3 (3), F2 (3), F5 (4, two reasons), F6 (2), F10 (2), F8 (2), F4 (2), F9 (1),
F11 (1). Nothing else in this note moves a sweep verdict; every other family
is corpus-only and none is on femtovg's path.

## F1: chains of three or more adds or multiplications (case 80)

Reference: `tools/mali/testdata/other_80_matdyn.{vert,frag,expect.json}`
(Mesa 26.2.1 on drm-shim, `drm_shim_ref.sh run`, `pan_ref_diff.py extract
--draw`, no `--uniform-values`: `u_i` is an int). The bytes equal
`unitype_80.expect.json` (the earlier reference of the same program, only the
trace name differs); this item re-made it with a fresh shim.

**Mesa's lowering: `do_rebalance_tree` does not rebalance it.** The chain is

    float r = (s[1][1] + d[1][0] + q[0][1] + n[1][0] + 4.0 * p[1][0]) / 16.0;

five adds whose every leaf is `ir_dereference_array` of a matrix local, and
`is_reduction` rejects a tree with an array dereference leaf
(`opt_rebalance_tree.cpp`; `mali-rebalance.md` step 2). Final NIR keeps the
source's left-to-right order, one `ffma` where a leaf is `x * const`:

    %2  = fneg %1.z                      ; n[1][0]  (u_k.z)
    %4  = fadd 4.0, %2                   ; d[1][0] = 4 - z
    %5  = frcp %1.y                      ; q[0][1] = 2 / y
    %14 = ffma %1.w, 2.0, %4             ; + s[1][1] = 2w
    %15 = ffma 2.0, %5, %14              ; + q[0][1]
    %16 = fadd %15, %2                   ; + n[1][0]
    %17 = ffma 4.0, %1.z, %16            ; + 4 * p[1][0]
    %19 = fmul %17, 0.0625               ; / 16

and the fragment shader (61 instructions, 12 registers) is

    FADD.f32 r3, 0x44000000.h1, u2.w1.neg
    FMA.f32  r3, u3.w1, 0x40000000, r3^
    FRCP.f32 r4, u3.w0
    FMA.f32  r3, 0x40000000, r4^, r3^
    FADD.f32 r2, r3^, u2.w1.neg
    FMA.f32  r2, 0x44000000.h1, u2.w1, r2^
    FMA.f32  r2, r2^, u5.w0, 0x0.neg

(`u2.w1` = `u_k.z`, `u3.w0` = `u_k.y`, `u3.w1` = `u_k.w`, `u5.w0` = 0.0625).
The run-time indexed reads (`arr[i]`, `p[i]`, `col[i - 1]`, `v[j]`) are
`ICMP_OR.s32.lt.m1 rN, u4.w1, 0x3020100.b2/b3` and `CSEL.u32.eq` ladders
(`mali-uniform-types.md` section 7). No branch, no scratch.

**Why we refuse.** `glsl.rs` `rebalance`/`visit` meets a value whose leaf
`leaf()` cannot classify (`Reduction::Unknown`: a read of a matrix local's
element is an IR `LoadLocal` whose GLSL form, `m[1][1]`, the IR does not
say) and `chain(v) >= 3`, so it returns `Rebalance`. The refusal is
over-eager for this shape exactly as it was for matattr3 (all leaves
dereferences, no rebalance).

**Proposed item `chain-matlocal`** (one session, shares `glsl.rs`,
`tests.rs`, `corpus-verdicts.txt`): in `glsl.rs` `leaf()` a read of a local
that is a matrix, or of a local array element, is the dereference that ends
the reduction (`Err(Reduction::Not)`), as a matrix attribute column already
is. Then `rebalance` leaves the chain in order and the next refusal of case
80 appears (its mixed-precision locals, then `IndexedLocal`, per the section
7 of `mali-uniform-types.md`): the item ends with `matdyn` refused at the
next named gap, not byte-equal, unless the items for those are done; its
test is the IR shape (`glsl_rebalances_the_trees_it_takes` style) plus the
chain's NIR order above. Case 80 itself needs, in order: this, the
mixed-precision item (triage-mixed), dynamic indexing of locals (`Ilt` +
`CSEL` ladder, from `unitype_80`), then the pin of `other_80_matdyn`. Do not
write `other_80_matdyn` into `MALI_TESTS` before it is byte-equal.

`es300/texoffset.frag` is the other kind: `0.25 * (A + B + C + D)` where
A..D are texture results. A texture is an `ir_texture` rvalue, not a
dereference, so the four-leaf add tree (three adds, no constant) **is**
rebalanced to `(A + B) + (C + D)` by Mesa; `glsl.rs` refuses a texture result
as a leaf (step 5 of `mali-rebalance.md`). Proposed item `chain-texleaf`:
accept a texture result (single use) as a leaf, then a reference of
`texoffset.frag` (needs an ES 3.00 draw with a sampler: `gl_ref.py`
options) for the order. Corpus-only: after the sweep items.

## F2 to F11: corpus-only, what exists and what each needs

These block no sweep case, so no new reference was made (the contract of
this item). Mesa's lowering is recorded in the notes cited; the proposals are
sized for one session each and ordered by corpus sources.

- **F7 mediump kept loops (7 sources, `LoopMediump`)**: references exist
  (`intloop_loop_nested`, `intloop_contloops`; `mali-loop-break.md` Part 2,
  `mali-int-loops.md`): Mesa keeps loops with a uniform bound as real
  blocks with 16-bit header phis (`MOV.i32` of pairs, `ICMP_OR.s32.ge.m1`,
  `BRANCHZ`). Item `loop-mediump`: Blocks in the mediump fragment path
  (`nir_fs.rs` `walk_kept_loop`, `!prec.highp`); the table of `mali-loop-break.md`
  "Status of Part 2" gives what the words need. Then `loop-guard-mediump`
  (`contloops`, `GuardMediump`) on top. Unblocks no sweep case but the
  largest corpus group.
- **F6 vertex loops (2 sources, `LoopVertex`)**: reference
  `intloop_loop_nested_vs` (53 words). Item `loop-vertex`: blocks in the
  vertex selection, fp16 temporaries, int uniforms by value. After
  `loop-mediump`.
- **F3 uniform types (3 sources)**: `light_amount_*.vert`, `sharedint.vert`
  read struct members, `int` and `bool` uniforms in a vertex stage;
  `mali-uniform-types.md` section 1-2 and `intuni_sharedint` (a reference
  exists). Item `uniform-struct-vs`: accept the types in `uniform_of` /
  `uniform_extent`; the `int` word is read raw. Check first which of the
  three still stops here (the verdict text may have moved to the next gap).
- **F2 run-time uniform index (3 sources)**: `mali-uniform-index.md` (the
  `LD_PKA` from UBO 0 beside the FAU push), `mali-uniform-arrays.md`
  (`case108_uniidx`, `uniidx_d`/`uniidx_i` references); dynvec4 and
  uniform_idx index a vector lane / uniform array by a loop counter. Item
  `uniform-index-more`, after `loop-mediump` for the loop-counter ones
  (`dynvec4`, `uniform_idx`); `es300/uniidx.frag` alone may go first.
- **F5 varying lanes (4 sources)**: `mali-varying-constants.md` (Mesa's link
  merges equal lanes and moves constants; the seven cases of that note are
  lowered). The two new shapes: two lanes of equal value (`va_vec2`,
  `discard_dynloop.vert`) and a lane from no attribute through a sampler
  struct (`vtex_struct_sampler*`). Item `varying-merge`: needs one reference
  per shape first (`drm_shim_ref.sh`), which this item did not make because
  no sweep case uses them.
- **F4 local count (2 sources)**: `tmparr_const` declares `vec4 arr[40]`
  (`Pressure::Remat` is cited in its comment); Mesa lowers it to
  compare/select ladders over constants. The limit is our 32-local cap
  (`MAX_LOCALS`). Item `locals-cap`: needs a reference (not made), then a
  larger cap or per-element constants.
- **F10 constant product (2 sources)** and **F8, F9, F11**: each is one
  shape of a rule `mali-algebraic-folds.md` / `mali-varying-constants.md` /
  `mali-texture-lookups.md` documents; they overlap `triage-fsarith`
  and `triage-vsarith` (check their notes before opening items). `xlink`
  (F9, F10) shares one program pair; `sharedint.frag` (F11) is an
  `ICMP`/`CSEL` on the `int` uniform (`mali-int-uniforms.md` 2.7).

## Roadmap step 4 or not

F1 (chain-matlocal) belongs in step 4: it is case 80. Everything else here
has no sweep case: leave F2 to F11 out of step 4 and carry them as #33
items (texts in the report). None needs board time.

## How the reference was made

    # $CONNECTED_DEPS_DIR/mesa-pan/shim: mesa-26.2.1 panfrost + drm-shim,
    # built with the meson line of drm_shim_ref.sh build, libdir=lib64
    tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-pan/shim \
        tools/mali/testdata/other_80_matdyn.vert tools/mali/testdata/other_80_matdyn.frag \
        target/other/other_80_matdyn
    python3 -I tools/mali/pan_ref_diff.py extract target/other/other_80_matdyn.trace --draw \
        > tools/mali/testdata/other_80_matdyn.expect.json

`other_80_matdyn.vert` is `gl_test_es100/passthrough.vert` and `.frag` is
`es100/matdyn.frag`, verbatim. The `.trace` is not kept.
