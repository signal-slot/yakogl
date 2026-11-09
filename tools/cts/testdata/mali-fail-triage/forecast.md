<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Fixture: the parts of docs/notes/mali-gles2-forecast.md the triage reads

## 1. By mustpass group

Not read.

## 2. The refusal causes

| cause | message | source | cases | owner |
|---|---|---|---:|---|
| `ARITH` (`MaliError::Arithmetic`) | `Valhall: arithmetic other than +, -, * is not lowered yet` | `nir_fs.rs` | 3963 | plan #309 |
| `TEX_KIND` (`TexKind`) | `Valhall: a texture look-up other than texture2D is not lowered yet` | `nir_fs.rs` | 232 | plan #309 |
| `BLEND_SHADER` | `mali frame: a blend shader other than a constant colour's (min/max, SRC_ALPHA_SATURATE, unmatched factors, separate colour and alpha, a masked, 4x or non-RGBA8 target) is not built yet` (`NOT_BUILT_BLEND_SHADER`) | `frame.rs` | 845 | plan #309 |
| `MATRIX_ATTRIB` (a FAIL, not a refusal) | none: pixels wrong | `RESULTS.md` | 40 | none yet (issue #32 holds the symptom) |

The same shader may be refused by a sibling variant of the same family
(`Arithmetic` instead of `FsOperandForm`, `MixedPrecision` for a mediump/highp
mix). All of those messages end `is not lowered yet`.

## 3. Hang-risk: 208 cases

Not read.
