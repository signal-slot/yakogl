<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): push words past FAU page 0 (FAU_PAGE)

**Status: sel_u_fau65 byte-equal; refract vs 7215 and terrain vs 18bc
compile** (plan #367 lower-fau-page, issues #32/#33). The FAU_PAGE family of
docs/notes/mali-glmark2-forecast.md: the Mali online compiler
(`src/shaderc/online/mali/`) refused a program that reads a push word past
word 63 ("a push word past FAU page 0 is not lowered yet", the removed
`MaliError::FauPage`). The references are Mesa 26.2.1 on drm-shim:
`tools/mali/testdata/sel_u_fau65` and `gm2_faupage_1..4` (ref-fau-page,
d8ae5178). The host test is `tests/mali_fau_page.rs`; the pan_shc verdicts
are in `tools/mali/testdata/cases.txt`. Background: mali-uniform-types.md
(the push layout, `sel_u_fau130`'s `LD_PKA` past 128 words).

## 1. The page rule

The FAU holds up to 128 32-bit push words (`pan_fau_layout`'s `max`), 64
a page: 32 64-bit slots. A source byte names a word of one page only
(`0x80 | word`, `va_pack_fau_64`), and the instruction word names the page
in `fau_page_index` (bits 57-58, va_pack.c:855). So:

- an instruction reads one page. Its page is the page of its **first FAU
  source** (`va_select_fau_page`, va_compiler.h:59-68). A uniform word `w`
  is on page `w / 64`; `lane_id` and `program_counter` are on special page
  3, `tls_ptr` and `wls_ptr` on page 1 (not used here), the other specials
  on page 0. A table immediate (`0xC0 | i`) has no page: as a first FAU
  source it selects page 0, and it is never checked against the page
  (`valid_src`, va_validate.c:122-129);
- `va_repair_fau` (va_validate.c:156-175) reads every uniform or special
  source on another page through a `MOV.i32` before the instruction, as it
  does for a second uniform slot or a third FAU word. The `MOV.i32` reads
  that one word, so its own page is that word's.

Two uniform words on two pages are always two 64-bit slots, so the slot rule
alone moves one of them, and an execution-engine instruction never takes a
uniform and a special word together. The page check itself decides only for
an instruction that takes two FAU indices (the message unit): a uniform and
a special word of different pages.

## 2. What the compiler does

- `fau.rs` `fau_page` is `va_fau_page`, `select_fau_page` is
  `va_select_fau_page`, and `FauState::valid` checks the page as
  `valid_src` does. `repair_fau` no longer refuses a word past 63.
- `pack.rs` `src_byte` writes `0x80 | (w % 64)` for any of the 128 words,
  and `tail` writes `select_fau_page` as `fau_page_index`. A source of
  another page reaching the packer is a defect (`Defect::Operand`): the
  constants pass leaves none. The instructions the packer makes itself
  (the blend return's `program_counter`, the derivative's `lane_id`) read
  their special first, so their page is 3 as before.
- The layout is unchanged: `push_ubo` and `lower_constants` place words up
  to 128 as before (`FAU_MAX_WORDS`), and a promoted constant past word 63
  is read on page 1 like a uniform. Mesa does the same: it has no rule that
  keeps constants on page 0. sel_u_fau65's 65th word is a promoted
  constant, read on page 1 by one instruction. The driver uploads the FAU
  as before: src/mali keeps up to `FAU_MAX_WORDS` push words
  (`src/mali/program.rs`) and writes all of them in order, the slot count
  in the FAU register's bits 56-63 (`frame.rs` `fau`).

## 3. The measured word maps (study-fau-page, 92567773)

| Shader | FAU words | Past page 0 | Read by |
|---|---|---|---|
| refract vs 7215 | 78 | words 64-77: LightMatrix dwords 57-63, and the varying-store offsets 16..40 | FMA and FADD (the uniform words), MOV.i32 (the offsets) |
| terrain vs 18bc | 93 | words 82-92: one texture-descriptor word (0x20001000) and the offsets 16..52 | MOV.i32 only; no uniform word of 18bc is past page 0 |

Neither shader has one instruction that reads a page-0 and a page-1 word.
gm2_faupage_4 does: `(u_m1 * a_pos + u_m2 * a_pos) * u_near + u_v` reads
`u_near` (word 38, page 0) and `u_v.w` (page 1) in one FMA. Mesa keeps the
FMA on page 0 (its first FAU source, `u19.w0`) and reads `u_v.w` through
`MOV.i32 r1, u41.w1` on page 1. Ours makes the same two instructions.

## 4. The references

| Case | What it shows | Ours |
|---|---|---|
| sel_u_fau65 | one constant at word 64, one page-1 instruction | byte-equal (words, facts, FAU words) |
| gm2_faupage_1 | `FADD.f32 ... u32.w0` (word 64, u_w.y) on page 1 | the same page-1 instruction; the varying program's order differs (Mesa adds the `vec4(u_w, 1.0, 1.0)` 1.0 into the first column's FMA and sums columns in pairs) |
| gm2_faupage_2 | refract's four mat4: 7 uniform words and 7 offsets on page 1 | refused by its fragment shader (FRAGCOLOR_LANE) |
| gm2_faupage_3 | terrain's layout: only offsets on page 1 | refused by its mediump fragment shader (a constant operation); with that shader highp, the vertex stage differs only by IDVS_PUSH_RANGE |
| gm2_faupage_4 | the mixed-page FMA | the same MOV/FMA pair; the words after word 38 sit three words earlier (IDVS_PUSH_RANGE) |

`tests/mali_fau_page.rs` checks the page rule on Mesa's words and ours for
every case (each instruction reads only its own page's uniform words, one
slot), byte-compares sel_u_fau65, and pins each gm2_faupage cause
(`CAUSES`); the four byte-compares stay `#[ignore]`d until those causes are
lowered.

## 5. IDVS_PUSH_RANGE (a new cause, not lowered here)

Mesa compiles both IDVS variants from one NIR: bifrost_compile.c:4374-4398
specializes each variant and then runs only constant folding, DCE, dead CF
and CSE, with no `nir_opt_shrink_vectors`. A uniform load keeps the width
that both variants' uses gave it, and `bi_opt_push_ubo` (run before the BIR
DCE, :4447-4449) pushes it whole into each variant that reads any of its
lanes. Measured:

- gm2_faupage_4: the position program reads only `u_near` (dword 64), but
  pushes four words, `u_near` and `u_v.xyz`: NIR's load vectorizer merged
  the two uniforms' loads into one vec4 load the varying program needs.
- gm2_faupage_3: the varying program pushes all 16 words of `u_eye`, where
  it reads only `e.xyz`; the position program needs `e.w`.

`fau.rs` `push_ubo` pushes, per load of `select.rs` `UboLayout::load_of`,
from the load's first word to the last word the **variant** reads, and
`load_of` does not merge two uniforms' loads. Both make our layout shorter
than Mesa's, which moves every later word (and so which words land on page
1). The program is still correct (the driver uploads our layout), but it is
not byte-equal. Lowering it needs the union of both variants' reads per load
and the vectorizer's merge of adjacent uniforms' loads, in `push_ubo` and
`load_of`.
