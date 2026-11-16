<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): draws refused as GL_OUT_OF_MEMORY (plan #373, cause 6)

Plan #373 item `oom-fix`, from the finding of item `inv-oom`; host work only,
no board run. The board records are the dEQP-GLES2 slices
`tools/cts/log/2026-11-1{2,3}-t6-gles2-S*`.

## 1. What the error was

33 dEQP-GLES2 cases ended in ResourceError: `shaders.matrix` `*_vertex` 24,
`shaders.random` 6, `shaders.conditionals` 2, `fragment_ops.interaction` 1.
The draw returned an errno that `src/mali/backend.rs` (`frame_error`) reports
as `GL_OUT_OF_MEMORY`. It was not memory exhaustion: the frame builder's
check `(vs.varyings != 0 || fs.varyings != 0) && d.vs.varying_spd.is_none()`
(`src/mali/frame.rs`, `INVALID_VARYINGS`: "a stage reads or writes varyings
but the vertex shader carries no varying program") fired, a
`FrameError::Invalid`, which is an errno and not a named refusal.

## 2. Why

The vertex shader's layout (`VaryingSlot` rows, `src/shaderc/online/mali`)
keeps a row for every vertex output, but only the components the fragment
shader reads are stored (`compact_layout`, `nir.rs`: `mask`, `comps` =
`mask.count_ones()`). A vertex output whose every lane the fold made a constant
has `mask == 0`, `comps == 0`: a placeholder row. Mesa's link moves those
constants into the fragment shader, so the vertex shader stores no varying and
the compiler emits no varying program (`secondary: None`).
`StageMeta::of` (`src/mali/program.rs`) counted every Generic row as a varying
written, so `ShaderMeta::varyings` was 1 or more for a vertex shader that
writes none, and the frame builder found "varyings, no varying program".

## 3. The fix

`StageMeta::of` counts a vertex Generic row only when it has a component
(`comps != 0`). A fragment record's rows are the varyings it reads and are
counted as before. `varying_bytes` was already right (a placeholder adds
`offset + 0` and the offset is the running end of the stored rows). The check
in `frame.rs` and the compiler's placeholder rows are unchanged; the check
still names a record that breaks the contract.

Mesa 26.2.1 (`MESA_PAN_TREE`) counts only the slots it stores:
`pan_nir_collect_varyings.c` (the generic layout and `generic_size_B`,
lines 415-444, the value `ShaderMeta::varying_bytes` carries into the IDVS
`VARY_SIZE` register) and the link's constant propagation
(`compact_varyings`, the step our `compact_layout` and `motion.rs` follow).

## 4. After the change

Draws whose vertex varyings all fold to constants reach the draw. They draw
correctly only if the fragment shader outputs the folded constants as Mesa's
does; the pixels are checked on the board only (the host mock draws nothing).
Any of the 33 cases the compiler still refuses (M.2 refusals) or whose fragment
program does not carry the constants stays a named `NOT_BUILT`/`MaliError`
refusal from the compiler, not an unnamed errno. Which of the 33 pass is
settled by the board rerun (item `board-rerun`).

## 5. Tests

`src/mali/program_tests.rs` (wired from `program.rs`; feature set
`--no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table`,
`cargo test --lib mali::program::tests`):

- `vertex_all_placeholders_write_no_varying`: only placeholder rows, 0
  varyings, 0 bytes (the 24 `shaders.matrix` cases' shape).
- `vertex_placeholder_beside_stored_row`: placeholders not counted, bytes of
  the stored row.
- `vertex_stored_rows_counted`: stored rows counted, partial masks by
  components.
- `fragment_rows_counted_as_read`: fragment rows counted as before.
