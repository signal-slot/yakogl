<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# instanceid-design, 2026-10-14, plan #144 item instanceid-design

Investigation only, at `e0eb883d`. The full note, with quotes, options and
host output, is `tools/cts/log/investigations/instanceid-per-draw.md`.

## State of main

No source change. The design for `gl_InstanceID` in instance-by-instance
draws (the six `dEQP-GLES3.functional.instanced.*.mixed.{2,4,20}_instances`
cases) is in `tools/cts/log/investigations/instanceid-per-draw.md`.

## Open work

This changes the es3-instanced rows (`docs/handoff/20261014-es3-instanced.md`,
"Open work"). The design is now chosen:

- PowerVR (plan #18): **P1**, Mesa's PDS `instance_id_modifier` set to
  `GlesDraw::instance_id`. It is a new parameter of
  `pvr_pds_render_vertex_program_ids` (`driver/src/pvr_pds_render_gen.{c,h}`,
  `src/pvr/ffi.rs`, `src/pvr/ffi_layout_check.c`), the refusal at
  `src/pvr/backend.rs:4585-4599` is removed, and `VDATA_DWORDS` grows. About
  60 lines plus tests. Then **P3**: hardware divisors through the PDS stream
  `multiplier`/`shift`, with `max_instance_divisor` 0xffff.
- GC7000UL (plan #18, compiler part plan #19): **V0**, the GC7000UL online
  compiler sets `ShcShader::vs_instance_id_reg1` when the program reads
  `gl_InstanceID` (`src/shaderc/online/select.rs:3983-3987`,
  `src/shaderc/online/abi.rs`). Plus **V1**, the stream divisor from
  `GlesAttrib::divisor` (`src/vivante/backend.rs:2261`) and
  `max_instance_divisor` 0xffff (`:2886`). The FE already gets the divisor
  register 0x14680 on every draw. This is what Mesa etnaviv does on HALTI2+.
- `src/gl/draw.rs` (plan #19) needs no change for any of these.

## Known defects

- **GC7000UL: a split instanced draw that reads `gl_InstanceID` is drawn
  wrong without any message, not refused.** The refusal at
  `src/vivante/backend.rs:2353` tests `vs_instance_id_reg1`, and only the
  PowerVR compiler sets that field (`src/shaderc/online/powervr/chain.rs:212`).
  On the host, the dEQP mixed shader compiles to `viv.vs_id_in_reg` 3 and
  `vs_instance_id_reg1` 0. `gc_draw` takes instances 1..3, and
  `gc_refused_draws` stays 0. The es3-instanced note said the GC7000UL
  refuses. That is wrong; the six cases still fail, but because the image is
  wrong. The unit test
  `an_instance_drawn_on_its_own_that_reads_gl_instance_id_is_refused`
  (`src/vivante/backend_tests.rs`) sets the field by hand, so it does not
  catch this. Fixed by V0 (or by V1 for divisors up to 0xffff).
- PowerVR: confirmed on the host. Instances 1..3 of the dEQP mixed shader
  return `-EINVAL` with
  `yakogl/pvr: draw refused: an instance drawn on its own (a divisor past 1) whose program reads gl_InstanceID`.

## Next board session

After a fix lands, run the `dEQP-GLES3.functional.instanced.*` group (45
cases) on the AM62P and the i.MX8MP. The commands and the expected verdict
table are in section 5 of the note. The six `mixed.{2,4,20}` cases must pass,
and the 39 others must stay passing. Watch the `attrib_divisor` cases after
P3 or V1, because they then use the hardware divide. If they fail on the
i.MX8MP after V1, the GC7000UL FE ignores divisors > 1: set
`max_instance_divisor` back to 1 and use V2 (a per-draw uniform).
