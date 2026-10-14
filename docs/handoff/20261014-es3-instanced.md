# es3-instanced, 2026-10-14, branch ccfarm/01a136ad-45c2-74dd-b9c8-dc442dd44eae

The 6 AM62P `dEQP-GLES3.functional.instanced` failures are
`{draw_arrays_instanced,draw_elements_instanced}.mixed.{2,4,20}_instances`
(docs/es3-cts-failures.md Appendix A; `docs/board-sessions/am62p-gles3-percase-fails.tsv`
is not on the tip or in any ref, so the names come from the appendix). The
other 39 instanced cases pass.

## Cause: not in `src/gl/vertex.rs`

All 6 have one cause, and it is in the back ends, not in the vertex state:

- The mixed cases use arrays with divisors 0, 1 and 2, and the shader reads
  `gl_InstanceID`.
- The PowerVR and the GC7000UL take divisor 1 only (`max_instance_divisor`).
  So `draw_instanced` (`src/gl/draw.rs`, `draw_instanced`) draws instance by
  instance and feeds each instance's divisor-array value as a constant
  (`instance_attrib_value`, `attrib_element`). That part is correct.
- It also sets `GlesDraw::instance_id` to the instance number. The back end
  then refuses the draw, because the PDS / FE starts `gl_InstanceID` at 0 in
  every draw: `src/pvr/backend.rs` (draw entry, "an instance drawn on its own
  (a divisor past 1) whose program reads gl_InstanceID", about line 4590) and
  `src/vivante/backend.rs` (about line 2353, returns `-EINVAL`). The draw is
  not recorded, so the image stays unwritten and the case fails.
- The V3D takes any divisor (`0xffff`), so it draws once and is predicted to
  pass.

`glVertexAttribDivisor`, the divisor queries and the integer current values in
`src/gl/vertex.rs` are not involved: divisor 0/1/2 are stored and returned
correctly (`tests/es3_vertex_api.rs`). So nothing in `vertex.rs` was changed.

## What was done

`tests/es3_instanced.rs` (new, host-only, in-process shader compiler) pins the
part of the path that the GL core owns, for both `glDrawArraysInstanced` and
`glDrawElementsInstanced` with 2, 4 and 20 instances and a divisor 0/1/2 program
that reads `gl_InstanceID`:

- a divisor-1 back end gets one draw per instance with `instance_id` = i, the
  divisor-1 value i and the divisor-2 value i/2 as constants;
- an any-divisor back end gets one draw with `instance_count` = n and the
  divisors 1 and 2 on the arrays, no constants;
- the arrays and the divisors are intact after the draw, and the next plain
  draw has `instance_id` 0.

The refusal itself could not be shown with the real PowerVR / Vivante back ends
on the host: they need a GPU. The refusal was read in the source only. dEQP was
not run (no `DEQP_WORK` here).

Runs: `es3_instanced` (3 tests) and `es3_vertex_api` (17) pass on gpu-powervr,
gpu-vivante and gpu-v3d (`host-test,gpu-v3d,shaderc-online,no-shader-table`);
`es2_report_guard` with `report-es2` passes on all three.

## Open work

Remaining causes, with owners (do not edit here; held by other plans):

| Cause | File, function | Owner |
|---|---|---|
| PowerVR refuses an emulated instance whose program reads `gl_InstanceID` | `src/pvr/backend.rs` draw entry (~4590); chunking exception ~4826 | pvr back end item (task T6 of the Appendix) |
| GC7000UL refuses the same | `src/vivante/backend.rs` (~2353) | vivante back end item (T6) |
| Option A: program the PDS instance step by the divisor (Mesa pvr does) so `max_instance_divisor` > 1 | `src/pvr/backend.rs` `max_instance_divisor`, PDS vertex fetch | pvr item |
| Option B: feed `gl_InstanceID` as a per-draw constant in the emulated path (add `instance_id` to the VS constants; the shader then reads it from a constant register instead of the PDS instance number) | back ends + `src/gl/draw.rs` `draw_instanced` | back ends and the draw.rs holder |

Option B keeps the core as is: `draw_instanced` already sets `instance_id` per
draw, so only the back ends need to use it.

## Next board session

Re-check on the AM62P (and the i.MX8MP, same refusal predicted) after a back
end fix: `dEQP-GLES3.functional.instanced.draw_arrays_instanced.mixed.{2,4,20}_instances`
and `...draw_elements_instanced.mixed.{2,4,20}_instances`. The 39 other
instanced cases must stay passing. The Pi 4 should pass already.
