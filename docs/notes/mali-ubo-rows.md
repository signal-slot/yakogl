<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Case 166: refused by the 32-row UBO layout, not by the FAU (Mali)

Issue #33 (case 166, "uniforms past the FAU"). Study at commit `0749ee2d`.
Nothing here changes `src/`; this records what the three references show, where
our refusal comes from, and the fix to make. It checks and sharpens
`docs/notes/mali-uniform-types.md` (section 3 and the finding 3 at its top),
which already says arms A and B need no UBO fallback.

## Finding

Case 166 (`fshprobe`) has three fragment arms in `tools/mali/testdata`
(`unitype_166a/b/c.frag`, vertex `unitype_166.vert`). **None of the three
overflows the 128-word FAU, and none of Mesa's programs has an `LD_PKA`.** The
refusal text "a uniform past the FAU's 128 words" (`MaliError::UboFallback`,
`src/shaderc/online/mali/mod.rs:368-370`) is wrong for arms A and B. They are
refused by a table of ours, `MAX_UBO_ROWS = 32`
(`src/shaderc/online/mali/select.rs:257`; `Table::UboRows` capacity,
`mod.rs:712`), which `UboLayout::fs16` fills with one row per array element.

## The references, decoded

The words are the pinned `fs.fragment` region of each `*.expect.json` (made by
`pan_ref_diff.py extract --draw`, from the drm-shim trace of Mesa 26.2.1;
`va_disasm_ref.sh` is the same disassembler that fills the `text` field). Counted
from the files:

| arm | uniform | instructions | FAU words | FAU slots read | loads (`LD_*`, `LOAD`) |
|---|---|---|---|---|---|
| A | mediump `vec4 u_p[64]` | 22 | 16 | `u2`-`u7` | none |
| B | mediump `vec4 u_p[70]` | 40 | 28 | `u2`-`u13` | none |
| C | mediump `vec4 u_w[4]`, 16 `sampler2D` | 116 | 30 | `u2`-`u14` | none (16 `TEX_SINGLE`) |

The opcodes in A and B are only `MOV`, `FMA`, `IADD_IMM`, `ATEST`, `BLEND`,
`BRANCHZI`: every uniform is a `MOV.i32 rN, uK.wH` or an `FMA.v2f16` source
straight from a FAU slot (`FMA.v2f16 r0, u2.w1.h00, u2.w0, r0^.neg`,
`MOV.i32 r1, u13.w1`). Arm C adds `TEX_SINGLE` and has the 16 texture handles
(`0x2000180N`) as FAU constants.

Where the FAU words come from. The array is mediump, so Mesa's block holds each
`vec4` as two words of fp16 (`LowerPrecisionFloat16Uniforms`), element `e` at
words `2e`, `2e+1`. `bi_pick_ubo` pushes a range per load, in ascending word
(`bi_opt_push_ubo.c:120-147`, `mali-uniform-types.md` section 3), after the four
blend constants:

- A reads `u_p[58].y` (1 word), and whole `u_p[56]`, `u_p[0]`, `u_p[32]`,
  `u_p[60]`, `u_p[63]` (2 words each): 11 words, plus 4 blend constants, plus
  the promoted `0x3c00` (fp16 1.0, the last of the 16 `fau` words) = 16.
- B reads, in addition, `u_p[57]` whole, `u_p[58].x`, and `u_p[64..67]`,
  `u_p[69]` whole; `u_p[58]` is then one word (both halves read): 11 two-word
  ranges and 1 one-word range = 23, plus 4, plus `0x3c00` = 28.
- C: 4 blend constants, the eight selector words of `u_w`, `0.5`, 16 handles
  and a pad = 30.

The largest FAU use is 30 of 128 words, and the last uniform word in the block
is word 139 (`u_p[69]`, arm B), far under the 512-word table of `fau.rs:158`.
So `bi_pick_ubo` never reaches its "does not fit" return, `ubo_mask` bit 0 stays
clear and Mesa uploads no block. The real FAU overflow is a different program,
`sel_u_fau130.vert` (`LD_PKA.i128 ... u63.w0`), already in `tests.rs:1120` and
`:2555` as `UboFallback`; it is not case 166.

## The rows each reference needs against `MAX_UBO_ROWS`

`UboLayout::fs16` (`select.rs:356-385`) loops `for e in 0..array_len` and calls
`push` once per element, for **every element of every used uniform**, not only
the ones the program reads (the packed block holds the whole used uniform, so
the words of `u_p[63]` depend on the 63 rows before it). `push` returns
`MaliError::UboFallback` when `n >= rows.len()` (`select.rs:277-279`).

| arm | rows `fs16` makes | block words (bytes) | against `MAX_UBO_ROWS` = 32 | rows read by the program |
|---|---|---|---|---|
| A | 64 (`u_p[0..64]`) | 128 (512) | refused at row 33 | 6 |
| B | 70 (`u_p[0..70]`) | 140 (560) | refused at row 33 | 12 |
| C | 4 (`u_w[0..4]`; samplers have no block word) | 8 (32) | fits | 4 |

Arm C therefore passes the layout and is refused later by `MaliError::TexCount`
(`nir_fs.rs`, the second look-up; `mali-texture-lookups.md`, the texture-2
item). **Case 166 stays failing until both A/B (this note) and C (texture-2)
land.** The sweep verdict of 166 does not move from this fix alone; only the
host tests for arms A and B do.

The one-row-per-element form is not an accident to remove. The selection reads
a fp16 pair as `Leaf::Uniform { base, comp: (w - base) as u8, swz }` with
`(w, half, base) = ubo.half(d)` (`nir_fs.rs:3082-3095`, `:3146`, `:3209`), where
`base` is the row's first word, and `fau.rs:163-164` keeps a lane in a `u8`
bitmask (`1u8.checked_shl(comp)`, a lane of 8 or more is `UboFallback`). With
one row per element, `base` is the element's first word and `comp` is 0 or 1. A
single row for the whole array would give `comp` up to 127 and break both.
(The highp `fs32` and the vertex `vs` layouts already make one row per uniform,
a vec4 array as one row, because their words are whole fp32; they are not the
cause.)

## Design of the fix

1. **Raise `MAX_UBO_ROWS`** from 32 to a value that holds the largest array
   the front end accepts: `array_len` is a `u8`, so one array has up to 255
   elements; use 256. The constant feeds three things, all by `MAX_UBO_ROWS`:
   `UboLayout::rows` (`select.rs:248`, 10 bytes a row, `Copy`),
   `Table::UboRows` (`mod.rs:712`) and `StageMeta::ubo`
   (`src/mali/program.rs:18,53,71,74`). The cost is about 2.5 KiB for each of
   those arrays; check that no place puts a `UboLayout` on a stack that is small
   (the compile scratch, `sc.ubo`, `finish`). If 256 is too large for the
   embedded stack, the alternative is a stride in `UboRow` (an array becomes one
   row and `half()` returns the element's own first word): it changes the
   record rows that `uniidx_i`/`uniidx_d` pin (`tests.rs:2424-2431`) and the
   frame's `check_draw` (`frame.rs:2104`), so it is a bigger change and not
   needed for 166.
2. **Do not widen anything else.** `fau.rs`'s `BASES = 512` words holds a 255
   element mediump array (510 words). `UboRow.word` and `UboLayout.words` are
   `u16`. `push_ubo` already stops at the first non-fitting range as Mesa's
   `bi_pick_ubo` does (`fau.rs:166-176`).
3. **Name the real cause in the refusals.** Today `UboLayout::push` (rows full),
   `fau.rs:163-164` (word table or lane out of range) and `fau.rs:192` (a
   range past the FAU, the true `sel_u_fau130` case) all return the same
   `UboFallback`. Give the table limits their own variant (or a `Table`-named
   error like the other tables) so the message says "uniform rows" and not "past
   the FAU's 128 words". Keep `UboFallback` for `fau.rs:192` only. Fix the
   comment of `UboLayout::rows` (`select.rs:262`, "the uniforms") to say it
   counts rows (array elements).
4. **No new instruction and no frame change.** With bit 0 clear the layout is
   not kept (`mod.rs:1240-1241`: `sc.ubo = ubo` only when `f.nubo > 0`), so the
   long layout never reaches the record or the frame. The descriptors and
   program words of arms A and B are the existing push path.

## Tests to pin

All in `src/shaderc/online/mali/tests.rs` (and its table), none new in the
frame:

- `unitype_166a` and `unitype_166b`: `check_program_for` byte-equal to
  `unitype_166a/b.expect.json` (22 and 40 words, `fau` 16 and 28 words). Assert
  `f.info.ubo` is empty and the FAU's tail equals the recorded constants, so a
  later change cannot start loading a block.
- A layout test on `fs16` for `vec4 u[64]` and `vec4 u[70]`: `rows()` is 64
  and 70, `bytes()` is 512 and 560, element 63's `word` is 126, element 69's is
  138.
- The limit: the layout of a 255 element array succeeds, and a second array that
  takes the rows past the new capacity returns the table error from step 3,
  not `UboFallback`.
- The capacity pin in the table test (`tests.rs:2524`, `("uniform rows", 14,
  32)`): the capacity becomes 256, and the high-water mark follows the new
  maximum (70 once 166 B compiles).
- `sel_u_fau130` keeps returning `UboFallback` (`tests.rs:1120`, `:2555`), so
  the real overflow is not hidden by the new row limit.
- Arm C stays `TexCount` (a refusal test) until the texture-2 item lowers the
  second look-up; then `unitype_166c` is byte-equal (116 words, 30 FAU words).
- `tools/mali/corpus-verdicts.txt` has no row for 166 (`shp_src` builds its
  shaders); nothing moves there. The `MALI_TESTS` count in `tools/mali/gate.env`
  grows by the tests added.

## What this note did not do

No board run and no new reference: the three references already exist and were
read as files. The `va_disasm_ref.sh` and `pan_ref_diff.py` outputs were taken
from the pinned `expect.json` (`words` and `text`), not re-run. The behaviour of
the frame for a row count above 32 is by reading `program.rs` and `frame.rs`,
not measured.
