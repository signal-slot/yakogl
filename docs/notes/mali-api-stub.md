<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): dEQP's stub vertex shader, a constant `gl_Position` with w = 0

**Status: landed on the host** (plan #373, item `apierr-fix`; issues #20,
#11328). The online compiler change is `159f1d14` ("mali: lower a constant
gl_Position whose w is zero"), checked again at the integration tip
`4455e85c`. The test is **`tests/mali_api_stub.rs`** (feature set
`host-test,gpu-mali,shaderc-online,no-shader-table`). Board numbers wait for
the next Mali dEQP-GLES2 rerun. Mesa paths are relative to `src/` of the
26.2.1 tree.

## 1. The shape

Many dEQP-GLES2 API cases first build a program from one stub pair:

```glsl
void main (void) { gl_Position = vec4(0.0); }      // vertex
void main (void) { gl_FragColor = vec4(0.0); }     // fragment
```

(VK-GL-CTS `es2fNegativeShaderApiTests.cpp:43-44`,
`es2fNegativeVertexArrayApiTests.cpp`, `es2fShaderApiTests.cpp`
`getSimpleShaderSource`. `es2fShaderStateQueryTests.cpp` and
`es2fIntegerStateQueryTests.cpp` use the same body spread over four lines.)
Then they check a GL error code or a query value. If the vertex shader does
not compile, the program does not link, and each later call answers with
something other than what the case expects. On the R373 tree this is
section M.1 of `docs/es2-cts-failures.md`: row 7 (8 cases), row 10 (7) and
the two `shader_api.compile_link` cases of row 11.

## 2. Cause

The vertex shader's w is 0.0. The viewport transform takes `frcp(w)`, which
gives +inf, and the vertex walk's constant fold refused every operation that
reads or makes a value that is not a finite normal float
(`MaliError::ConstantFold`: "a constant operation on or to a value that is
not a finite normal float"). So the program never linked. No info log was
printed, because the refusal is the compiler's own.

## 3. What Mesa 26.2.1 does

- `compiler/nir/nir_lower_viewport_transform.c:82-88`: `w_recip =
  frcp(w)`, then `fclamp(w_recip, -MAX_W_RECIP_ABS, MAX_W_RECIP_ABS)`, with
  `MAX_W_RECIP_ABS = 2^15` (line 47). Panfrost runs this pass on every
  Bifrost/Valhall vertex shader (`panfrost/compiler/bifrost/bifrost_nir.c:942`).
- `panfrost/compiler/bifrost/bifrost_compile.c:4394`: the
  `nir_opt_constant_folding` loop after IDVS specialisation folds the whole
  chain when the stored position is one constant vector. `frcp(0.0)` = +inf,
  then the clamp gives 32768.0. Stored: x/y/z = the viewport offset,
  w = 32768.0.
- If one lane is not constant (for example `vec4(p, 0.0, 0.0)`), w becomes a
  constant only after copy propagation. `nir_opt_algebraic`
  (`compiler/nir/nir_opt_algebraic.py`) then turns the clamp of `frcp` into
  `fcanonicalize` first, and Mesa stores w = +inf (pan_shc:
  `IADD_IMM.i32 r2, 0x0, #0x7F800000`).

## 4. The change (`src/shaderc/online/mali/nir.rs`)

- The fold (`alu_sized`, `clamp_inf` at about line 2267) accepts an
  infinity as an operand of `fmax`/`fmin`, but only in the viewport
  transform (`!self.user`) and only at 32 bits. `frcp` of a zero already
  gave the IEEE infinity of its sign. An infinity in the shader's own code
  is still refused (`ConstantFold`).
- `walk` (`const_position` at about line 6897) allows a zero w only when the
  IR stores exactly one constant vector to `gl_Position`. Mesa folds that
  case, as section 3 describes. Every other shape with a zero w still gives
  `MaliError::ConstantFold`: Mesa's +inf store is not lowered. This is pinned
  in the gap list of `src/shaderc/online/mali/tests.rs` (`vec4(p, 0.0,
  0.0)`).

No section M.2 refusal family changed. The change only touches the
viewport transform of this shape.

## 5. Evidence

- **Reference:** `tools/mali/testdata/deqp_cfold2_zero_operand_rcp_inf`
  (the dEQP stub vertex shader, drm-shim reference) is byte-equal and is
  in `MATCHED` of `tests/mali_deqp_cfold2.rs`
  (`cfold2_zero_operand_rcp_inf ... ok`). pan_shc gives our words for
  `vec4(0.5, -0.25, 0.75, 0.0)`, `vec4(0.0, 0.0, 0.0, -0.0)` and
  `vec4(1.0, 2.0, 0.0, -0.0)`. No shim reference was drawn for those three.
- **`tests/mali_api_stub.rs`:** 12 tests run the stub pair through the GL
  core on the Mali mock (T6 device facts). Each test pins one dEQP
  expectation: both stubs compile (`vertex_stub_compiles`,
  `fragment_stub_compiles`, `shader_compile_status`), the pair links
  (`stub_pair_links`), `GL_CURRENT_PROGRAM` (`current_program_binding`),
  `GL_DELETE_STATUS` (`delete_status`), `GL_VALIDATE_STATUS`
  (`program_validate_status`), the active-uniform query
  (`active_uniform_query`), and the error sequences of the negative_api
  cases with and without the program
  (`negative_api_errors_without_a_program`,
  `negative_api_attach_detach_errors`,
  `negative_api_with_a_linked_program`,
  `negative_api_errors_with_the_stub_program_in_use`).
- At `4455e85c`: `cargo test --no-default-features --features
  host-test,gpu-mali,shaderc-online,no-shader-table --test mali_api_stub`
  passes 12 of 12. `cargo test --lib` with the same features and the filter
  `shaderc::online::mali` passes 330.

## 6. What stays refused

- A zero w beside a lane that is not constant (section 3, last item). No
  dEQP-GLES2 API case uses that shape.
- `deqp_cfold2_zero_operand_{icmp,inot,rcp16}` and the other ignored cases
  of `tests/mali_deqp_cfold2.rs` (`docs/notes/mali-deqp-cfold2.md`). They are
  not part of this shape.
