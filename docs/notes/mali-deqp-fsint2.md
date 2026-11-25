<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the five `deqp_fsint2_*` references (fragment-shader integer/boolean values)

**HOST-ONLY.** Plan #402 item `note-fsint2`. A study note, no compiler change.
It names the shapes of the five references that `ref-fsint2` merged under
`tools/mali/testdata/deqp_fsint2_*`, pinned by `tests/mali_deqp_fsint2.rs`.
They are the extra fragment-shader integer/boolean references that
`mali-deqp-fsint.md` (the eight `deqp_fsint_*`) left open, in
particular the two random cases of its section 5.3. Verdicts are host
compiles at the tip `20ef73c0`, read from the test's pins; none is a render
verdict and nothing ran on the board.

## 1. Sources

Each pair is the one dEQP-GLES2 (VK-GL-CTS `93bca01`) writes, verbatim from
the `.qpa` of a HOST-ONLY `tools/mali/deqp-host-measure.sh` run at `ab919063`
(the slices `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055`). The
`.license` sibling of each `.vert` and `.frag` names the case. The references
were drawn with `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's
G610 drm-shim, `--uniform-values`).

## 2. The five references

"fs words" is the word count of Mesa's `fs.fragment` region, as in the test
header. On the board at `8ff01055` every case was refused "an integer or
boolean value".

| Ref (`deqp_fsint2_*`) | dEQP case (`dEQP-GLES2.functional.shaders.`) | Shape | fs words | Verdict at the tip | Lowering |
|---|---|---|--:|---|---|
| `rnd_cond9` | `random.conditionals.fragment.9` | dead `if`/`else` over int, ivec and bool globals and uniforms; Mesa drops every uniform, the colour is `vec4(s, 0.5, 1, r < q)` with `s` 0 and `q` -1.0 | 9 | refused: `control flow (if, a loop, a return before the end)`. The host's first refusal is not the board's (the board stopped at the integer/boolean value) | `lower-fsint` takes the value refusal behind it; the control-flow refusal is hit first and needs the fragment control-flow lowering before this reference can match |
| `rnd_trig93` | `random.trigonometric.fragment.93` | int and bool constants and globals into `gl_FragColor`, then `asin(vec4) * tan(vec4)` overwrites it | 48 | refused: `an integer or boolean value` | `lower-fsint` (the trigonometric tail may then meet its own refusal; not measured) |
| `fn_bvec2_ne` | `functions.datatypes.bool_bvec2_fragment` | `bvec2(v_in0)` into a user function `!(a.x == a.y)`, compared with `uniform bool ref_out0` (`isOk`), the bool in three colour lanes; drawn with `ref_out0` true | 14 | refused: `an integer or boolean value` | `lower-fsint` (the bool-uniform form of `deqp_fsint_conv_*`) |
| `step_lowp_float` | `operator.common_functions.step.lowp_float_fragment` | lowp `step(float, float)`, one lane, `* 0.5 + 0.25` | 12 | refused: `an integer or boolean value` | `lower-fsint` (same form as `deqp_fsint_step_float_lowp`) |
| `step_lowp_vec2` | `operator.common_functions.step.lowp_vec2_fragment` | lowp `step(vec2, vec2)`, two lanes, `* 0.5 + 0.25` | 17 | refused: `an integer or boolean value` | `lower-fsint` (same form as `deqp_fsint_step_float_lowp`, two lanes) |

Matched: none. Differs: none. All five byte-compares are `#[ignore]`d; the
`refusals_pinned` test holds the texts above and passes at the tip.

Not drawn: `operator.binary_operator.logical_or.bool_fragment`, whose pair is
byte for byte `deqp_intval4_logical_or_fs`.

## 3. Reading the verdicts

- The four integer/boolean rows are the refusal `lower-fsint` removes. Their
  lowering is the lowp-step and bool-uniform work of `mali-deqp-fsint.md`
  section 3; a lowp `step` at two lanes (`step_lowp_vec2`) is the one form
  that note does not draw.
- `rnd_cond9` has two layers. Its pin moves from the control-flow text to
  the next refusal when control flow is lowered, and from there to a match
  when `lower-fsint` takes the integer values. Moving the pin before then is
  a regression of the control-flow refusal, not of this item.

## 4. Reproducing

```sh
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsint2
```

A lowering that takes a reference moves it to `MATCHED` and drops its
`#[ignore]` in the same commit.
