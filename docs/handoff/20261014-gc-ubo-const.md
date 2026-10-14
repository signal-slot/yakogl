<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc-ubo-const: GC7000 ubo cases over 320 constant registers, 2026-10-14, main cd738b1ae7cf

Details: `tools/cts/log/investigations/gc-ubo-const-limit.md`. No file under
`src/` or `tests/` changed.

## Open work

- GC7000 `functional.ubo` `single_nested_struct_array.{per_block_buffer,single_buffer}.{shared,std140}_instance_array_both`
  (the four `KNOWN_LIMITS` of `tests/es3_ubo.rs`): each stage needs 184
  constant registers (108 for the copied block, 76 for literals), 368 against
  the 320 the i.MX8MP's GC7000UL has (`NumberOfConstants` 0x140 in Mesa
  26.2.1's hwdb). The 320 is real; the cases are fixable. Mesa etnaviv reads
  every user UBO from memory with `LOAD` and puts only default-block uniforms
  in the constant file (`etnaviv_nir_lower_ubo_to_uniform.c`, `block > 0`
  stays a load). Reading block-member rows with `load` here (the path of
  `LoadUniformIdx`, `SHC_VIV_CONST_UBO_ADDR`, `ubo_copy`) would need about 77
  registers per stage. The fix design (files and sizes) is in the note:
  `src/shaderc/online/codegen.rs` (~25 lines), `select.rs` (`uniform_block`,
  `uniform_src`, ~80), `src/vivante/backend.rs` (`take_uniforms` cap of 1280
  dwords, ~30). Do it after `gc-const-array` merges (both touch the vivante
  code generator). One i.MX8MP board run is needed for a constant-offset
  `load` from a user block; it is not yet done.

## Known defects

- `take_uniforms` (`src/vivante/backend.rs:2287`) cuts a stage's uniform block
  at `GC_MAX_UNIFORM_DWORDS` (1280 dwords). Harmless while the whole block
  must fit the 320 registers; it becomes a silent truncation as soon as
  blocks are read from memory. The fix above lifts it.
