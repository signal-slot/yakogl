<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the six `deqp_vsuniarr3_*` references (partly read plain matrix arrays)

**Status: study** (plan #402, item `ref-vsuniarr3`; issue #20). HOST-ONLY: no
compiler change, nothing ran on the NanoPC-T6. Test: `tests/mali_deqp_vsuniarr3.rs`.

## 1. What was measured, and what was not

The full `deqp-host-measure.sh` re-measure (a deqp-gles2 build and a VK-GL-CTS
fetch) was **not run** for this item: neither a CTS checkout with external
sources nor a built deqp-gles2 was present, and the build is hours on this
machine. The shapes were instead taken from the existing notes
(`mali-deqp-vsuniarr.md`, `mali-deqp-vsuniarr2.md`) and the existing
references' shaders, and each candidate was compiled on the host at the tip
(`mali_ref::compile_pair`). That is a HOST-ONLY compile of hand-made
shaders in dEQP's `uniform_api` style, **not** a measure of dEQP's own
programs; the `.license` files say so. A later `remeasure-402` should check
that dEQP cases of these shapes (random.71, 75, 76, 77, 83, 94, 99, which the
vsuniarr2 note lists as undrawn) exist.

## 2. Coverage check

Existing references read a mat4 array whole (`vsuniarr_mat4`,
`vsuniarr2_mat4_mixed`, `unif2_arr_mat_array`), a single element of a mat3
array (`vsuniarr2_mat3_part`), mat2/mat4 arrays as struct members
(`vsuniarr_unused_mat2`, `vsuniarr2_struct_*`, `mat2_struct`). None reads a
plain mat4 array partly (some elements or columns), a highp matrix array, a
partly read mat2 array, a matrix array beside a scalar, or uses an element in
a matrix product.

## 3. Shape table (all refused at the tip: "a uniform array is not lowered yet")

| Reference | Declaration | Read |
|---|---|---|
| `mat4_part` | `mediump mat4 u_var[4]` | `[1]`, `[3]` |
| `mat4_highp` | `highp mat4 u_var[3]` | `[1]` |
| `mat4_col` | `mediump mat4 u_var[3]` | column `u_var[2][1]` only |
| `mat2_part` | `mediump mat2 u_var[4]` | `[3]` |
| `mat4_first` | `mediump mat4 u_var[3]`, `mediump float` | `[0]` and the float |
| `mat4_mul` | `mediump mat4 u_var[2]` | `gl_Position = u_var[1] * a_position` |

Matched: none; differs: none. Mesa 26.2.1 (G610 drm-shim,
`deqp_ref.sh --instrument shim`) made all six. No `.nir.sha` was made.

## 4. Analysis for the lowering

The refusal is `MaliError::UniformArray` from `uniform_of`; `array_element`
(nir.rs ~1185) takes float..vec4 and (per the task's triage) fp16 matrix
elements for plain arrays against fp32 for struct members, and the `shrink`
caller near nir.rs:8432 picks `Prod::UniformVec4` for any `array_len > 1`
before it checks for a matrix. The new shapes exercise exactly that: unread
elements and columns take no FAU word in Mesa, `mat4_highp` is the fp32
(non-fp16) plain case, and `mat4_mul` is the matrix-product consumer. These
claims about nir.rs come from the item's triage text; this item changed no
source and did not re-verify the line numbers.
