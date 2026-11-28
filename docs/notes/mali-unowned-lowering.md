<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali refusal texts at 100 or more first-refusal cases without a design (none at `9dd0be67`)

Plan #387, item `unowned-refs`. The question: which refusal texts of the
Mali online compiler (`MaliError`, `src/shaderc/online/mali/`) are the
first refusal of 100 or more must-pass dEQP-GLES2 cases at the integration
tip and have no design in `docs/notes/`? For each such text the item would
draw Mesa 26.2.1 references (`deqp_unowned_*`) and pin them in
`tests/mali_deqp_unowned.rs`.

**Result: no text qualifies.** All 12 texts at or above the line have an
owning design (section 2). So there are no `deqp_unowned_*` references and no
`tests/mali_deqp_unowned.rs`. This note and the ranking it rests on are the
whole delivery. No compiler change.

HOST-ONLY: these are compile and link outcomes of the host library, not
board results.

## 1. The measure and the ranking

- Tip: `9dd0be67accaa7a2200d9efacd5f57c47703fcee` (the integration tip, after
  train 3a; trains 3b, 3c and 4 and the plan-#402 lowerings had not landed).
- CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, the
  `HOST_DEQP_CTS` tree of `tools/cts/deps.toml`
  (`tools/cts/deps-env.sh --build HOST_DEQP_CTS`, i.e.
  `$CONNECTED_DEPS_DIR/VK-GL-CTS-host` with the yakogl target).
- Measure (4 cores), the case list of families section 19: the 10,597
  board-refused cases of the slices `2026-11-15-t6-gles2-S{1..4}-37305608`:

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
python3 -I tools/mali/host-rank.py build/measure     # default --mustpass, --threshold 100
```

- All 10,597 cases began. 106 chunk processes exited with 1 (dEQP's "not every
  case passed") and no case crashed. 4,693 cases still hit a host refusal
  (section 19 at `e9033663` had 4,716). All are in
  `tools/cts/gles2-mustpass.txt`: none was left out by the filter, and 5,904
  have no host refusal. `host-rank.py` exits with 1: 12 texts block 100 or
  more cases.

The ranking. Texts are `host-rank.py`'s short form, cut where the measure
script cuts them, so some are truncated. The `MaliError` column is the
variant that the opening words name, and the counts in it are section 19's.

| # | Text (short form) | Stage | Cases | `MaliError` (section 19) |
|--:|---|---|--:|---|
| 1 | arithmetic other than +, -, * and a dot or matrix product ... | both | 1406 | `Arithmetic` (1410) |
| 2 | a gl_FragColor lane other than the varying's own lane or a constant | fs | 955 | `ColourForm` (926) |
| 3 | a constant product in a shape Mesa's algebraic rules move or fold ... | vs | 249 | `ArithConstant`, vertex (246) |
| 4 | a texture coordinate other than a vec2 varying read whole or two constants | fs | 198 | `TexCoord` (200) |
| 5 | a loop in a vertex shader | vs | 194 | `LoopVertex` (194) |
| 6 | a fragment shader operand of a form or in a place no reference draws ... | fs | 189 | `FsOperandForm` (197) |
| 7 | an integer or boolean value | fs | 153 | `IntValue` (154) |
| 8 | a fragment varying load or conversion of a form no reference draws ... | fs | 127 | `FsVaryingLoad` (123) |
| 9 | a mediump fragment shader uniform other than ... | fs | 117 | `FsUniform` (154) |
| 10 | a fused multiply-add other than a matrix product's ... | vs | 116 | `FusedMultiplyAdd` (116) |
| 11 | a constant product in a shape Mesa's algebraic rules move or fold ... | fs | 105 | `ArithConstant`, fragment (104) |
| 12 | control flow (if, a loop, a return before the end | both | 103 | `ControlFlow` (102) |
| 13 | a fragment shader that mixes highp and mediump or lowp float values ... | fs | 92 | `MixedPrecision` (89) |
| 14 | a vertex shader texture look-up other than ... | vs | 61 | `TexVertex` (61) |
| 15 | a texture look-up other than texture2D(sampler2D, coordinate) ... | fs | 59 | `TexKind` (59) |
| 16-48 | 33 more texts | | 53 or fewer each | |
| | **Total** (48 texts) | | **4693** | |

Rows 3 and 11 have the same text: `host-rank.py` ranks the vertex and the
fragment stage apart because the measure cuts the text at different lengths.
Even together (354) they are the constant-product family. The text of row 6
appears again at rank 35 with 6 cases and a shorter cut, and the text of row 8
at rank 40 with 3 cases. Neither change crosses the line.

## 2. Each text at the line, and its design

The owners are the ones the item names: `mali-lowering-schedule.md` 5.4/5.5,
the texcoord, vscprod, irdef, fsintres and arithres notes, and the existing
family notes. Where a design is still in review and not in this tree, its
commit is given.

| # | Text | Design (owner) |
|--:|---|---|
| 1 | `Arithmetic` | `mali-arith-lowering.md` (steps A4b-A8, schedule 2.3 and 5.4 O1-O3); the residue after trains 2 and 3a in `mali-arithres-lowering.md` (arithres-refs, review, `f9ed0d8a`) |
| 2 | `ColourForm` | `mali-fclane-lowering.md` (F2B-F2E); the residue in `mali-fclres-lowering.md` (fclres-refs, review, `aee417ac`) |
| 3 | `ArithConstant` (vs) | `mali-vscprod-lowering.md` (vsconstprod-design, review, `ea869b33`, step K3); `mali-constprod-lowering.md` |
| 4 | `TexCoord` | `mali-fmaloc-lowering.md` T1; `mali-texcoord-lowering.md` (texcoord-design, review, `7dd99ff1`) |
| 5 | `LoopVertex` | `mali-deqp-vsloop.md`, `mali-deqp-vsloop2.md` (vsloop-refs, review); plan #402 `lower-vsloop` (`cbb6b45b`) lowers step 1 |
| 6 | `FsOperandForm` | `mali-opform-lowering.md` (O4-O8, train 3c, schedule 5.4) |
| 7 | `IntValue` | `mali-intval-rest-lowering.md`; the residue in `mali-fsintres-lowering.md` (fsintres-design, review, `e8b9caec`); plan #402 `lower-fsint` (`e8327e42`) |
| 8 | `FsVaryingLoad` | `mali-deqp-varyload.md` (`varyload-lower-1`, train 3c); the residue in `mali-varyres-lowering.md` (varyres-refs, review, `18167020`) |
| 9 | `FsUniform` | `mali-unifmp-lowering.md` (FM4-FM7, train 3c, schedule 5.4) |
| 10 | `FusedMultiplyAdd` (vs) | `mali-fmares-lowering.md` (K6, K7, K4's rest; merged `070608e3`); `mali-kfold-lowering.md` |
| 11 | `ArithConstant` (fs) | `mali-constprod-lowering.md` (K2, K4, K5), `mali-kfold-lowering.md`; K8 in `mali-constres-lowering.md` (constres-refs, running) |
| 12 | `ControlFlow` | `mali-deqp-ctrlflow.md` CT1; the residue in `mali-ctrlres-lowering.md` (merged `ee45eacd`) |

No other text reaches 100: rank 13, `MixedPrecision`, has 92 cases
(`mali-mixprec2-lowering.md`, `mali-mixres-lowering.md`).

## 3. Caveats

- The case list is the 10,597 board-refused cases of `37305608`, the same
  list sections 18 and 19 of `mali-deqp-families.md` use, not the whole
  must-pass tree. A case the board passed or did not refuse there is not
  measured. A board rerun (board-rerun-2) gives the board's own list.
- This only checks which texts at the line have a design. It does not check
  that the owning notes' steps cover every shape the tip refuses under each
  text. That coverage is each family's own sites item.
- The tip predates train 3b (`mali-train-3b`, review), 3c and the plan-#402
  lowerings. After them, a host measure (host-measure-4) should rerun this
  check. A text that drops under 100 simply leaves the table. A new text
  over 100 with no design would be the next `unowned-refs`.
