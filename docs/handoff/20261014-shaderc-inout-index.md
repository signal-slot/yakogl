# shaderc-inout-index, 2026-10-14, investigation (no src change)

## Open work
- The "indexed element as out/inout argument" item (RC13, 8 / 0 / 8) needs
  no work: at `cd738b1ae7cf` the 8 programs
  (`indexing.moredynamic.{builtin_fncall_out_parameter,
  user_defined_fncall_out_parameter, user_defined_fncall_inout_parameter,
  user_defined_fncall_inout_parameter_with_index_with_side_effects}`, both
  stages) build on PowerVR, GC7000UL and V3D. They were refused for the
  missing `trunc` (`modf`), fixed by `b1cf9e4`. The indexed copy-in/out
  (`LRef` in `lower.rs`) was already correct. Remove "indexed out/inout
  arguments" from the "no item yet" line of `docs/es3-cts-failures.md` and
  set the RC13 row to 0 / 0 / 0. Record:
  `tools/cts/log/investigations/shaderc-inout-indexed.md`.

## Known defects
- `docs/es3-cts-failures.md` RC13 table and ~line 435 are stale for this
  cause (see above).
