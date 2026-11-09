# Mali dEQP-GLES2 failure triage (draft)

Slices: `2026-11-09-t6-gles2-S1-11111111`, `2026-11-09-t6-gles2-S2-22222222`.
Forecast: `docs/notes/mali-gles2-forecast.md`.

| Cause | Cases | Examples | Slices | Forecast | Owner |
| --- | ---: | --- | --- | --- | --- |
| refused ARITH | 3 | `dEQP-GLES2.functional.shaders.operator.add.float_vertex`, `dEQP-GLES2.functional.shaders.operator.add.vec2_vertex`, `dEQP-GLES2.functional.shaders.matrix.add.a` | `2026-11-09-t6-gles2-S1-11111111`, `2026-11-09-t6-gles2-S2-22222222` | yes | plan #309 |
| fault chunk: mmu_faults | 2 | `dEQP-GLES2.functional.shaders.loops.for_constant_iterations.basic_fragment`, `dEQP-GLES2.functional.draw.second` | `2026-11-09-t6-gles2-S1-11111111` | **NEW** | - |
| hang-risk: fragment loop with a run-time bound | 2 | `dEQP-GLES2.functional.shaders.loops.for_dynamic_iterations.basic_fragment`, `dEQP-GLES2.functional.shaders.loops.while_uniform_iterations.basic_fragment` | `2026-11-09-t6-gles2-S1-11111111` | yes | forecast section 3 (loop work) |
| MATRIX_ATTRIB | 1 | `dEQP-GLES2.functional.attribute_location.bind_attrib.mat2` | `2026-11-09-t6-gles2-S1-11111111` | yes | none yet (issue #32 holds the symptom) |
| NotSupported in functional.multisample | 1 | `dEQP-GLES2.functional.multisample.a` | `2026-11-09-t6-gles2-S1-11111111` | yes | none (not a Fail) |
| not run: not-run | 1 | `dEQP-GLES2.functional.shaders.matrix.add.c` | `2026-11-09-t6-gles2-S2-22222222` | yes | none (rerun the cases) |
| refused BLEND_SHADER | 1 | `dEQP-GLES2.functional.fragment_ops.blend.equation_src_func_dst_func.x` | `2026-11-09-t6-gles2-S1-11111111` | yes | plan #309 |
| refused CompareForm | 1 | `dEQP-GLES2.functional.shaders.operator.cmp.a` | `2026-11-09-t6-gles2-S1-11111111` | yes | plan #309 |
| refused MixedPrecision | 1 | `dEQP-GLES2.functional.shaders.operator.mix.a` | `2026-11-09-t6-gles2-S1-11111111` | yes | plan #309 |
| refused TEX_KIND | 1 | `dEQP-GLES2.functional.texture.mipmap.bias` | `2026-11-09-t6-gles2-S1-11111111` | yes | plan #309 |
| refused blend_equation_separate | 1 | `dEQP-GLES2.functional.fragment_ops.blend.rgb_func_alpha_func.y` | `2026-11-09-t6-gles2-S1-11111111` | **NEW** | - |
| refused: Valhall: a made-up form is not lowered yet | 1 | `dEQP-GLES2.functional.shaders.conditionals.msg.a` | `2026-11-09-t6-gles2-S1-11111111` | yes | plan #309 |
| unexplained Fail in functional.fragment_ops.blend | 1 | `dEQP-GLES2.functional.fragment_ops.blend.rgb_func_alpha_func.z` | `2026-11-09-t6-gles2-S1-11111111` | **NEW** | - |

Non-Pass cases: 17.

## Draft issues

### Draft issue: Mali dEQP-GLES2: fault chunk: mmu_faults (2 cases)

Title: Mali dEQP-GLES2: fault chunk: mmu_faults (2 cases)

Body:

- Cause: fault chunk: mmu_faults
- Cases: 2 (Crash 1, Lost 1)
- Examples: `dEQP-GLES2.functional.shaders.loops.for_constant_iterations.basic_fragment`, `dEQP-GLES2.functional.draw.second`
- Slices: `2026-11-09-t6-gles2-S1-11111111`
- Commits: `11111111`
- Kernels: 6.1.115-vendor-rk35xx
- Detail (numbers and names stripped): `chunk-crash` x1; `segfault` x1
- How found: The chunk ended in fault and its kernel lines are of class mmu_faults.
- First kernel line: `panthor fb000000.gpu: Unhandled Page fault in AS1 at VA 0x0000000000001000 from <redacted> (<redacted>)`
- Forecast: not named in `docs/notes/mali-gles2-forecast.md`; add it to section 2 when the issue is open.

### Draft issue: Mali dEQP-GLES2: refusal blend_equation_separate is not in the forecast (1 case)

Title: Mali dEQP-GLES2: refusal blend_equation_separate is not in the forecast (1 case)

Body:

- Cause: refused blend_equation_separate
- Cases: 1 (InternalError 1)
- Examples: `dEQP-GLES2.functional.fragment_ops.blend.rgb_func_alpha_func.y`
- Slices: `2026-11-09-t6-gles2-S1-11111111`
- Commits: `11111111`
- Kernels: 6.1.115-vendor-rk35xx
- Detail (numbers and names stripped): `glDrawArrays: refused blend_equation_separate` x1
- How found: The back end refused the draw or the link with a name or message the forecast note does not list.
- Forecast: not named in `docs/notes/mali-gles2-forecast.md`; add it to section 2 when the issue is open.

### Draft issue: Mali dEQP-GLES2: unexplained Fail in functional.fragment_ops.blend (1 case)

Title: Mali dEQP-GLES2: unexplained Fail in functional.fragment_ops.blend (1 case)

Body:

- Cause: unexplained Fail in functional.fragment_ops.blend
- Cases: 1 (Fail 1)
- Examples: `dEQP-GLES2.functional.fragment_ops.blend.rgb_func_alpha_func.z`
- Slices: `2026-11-09-t6-gles2-S1-11111111`
- Commits: `11111111`
- Kernels: 6.1.115-vendor-rk35xx
- Detail (numbers and names stripped): `Image comparison failed: max difference = (#, #, #)` x1
- How found: A non-Pass with no refusal message, outside the forecast's causes.
- Forecast: not named in `docs/notes/mali-gles2-forecast.md`; add it to section 2 when the issue is open.

