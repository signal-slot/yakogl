<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# tests/mali_locals.rs: the reported stack overflow does not reproduce

Item locals-stack of plan #387. Measured at ec7da2b5 and again at 7b2246c0
(6 of 6 pass at both). No change under `src/`,
none to `tests/mali_locals.rs`.

## Result

Command (the test's feature set), run on a farm machine, x86_64 host:

    cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_locals

- Test profile (what `cargo test` and the CI rows use, opt-level 3): 6 of 6
  pass, with `--test-threads=1` and with the default threads. No SIGSEGV, no
  "has overflowed its stack" abort, so there is no backtrace and no repeating
  frame to read. `RUST_MIN_STACK` was not needed.
- `the_stack_of_a_compile_does_not_grow_with_the_cap` prints
  `deepest stack 178 KiB (fvg_filter_image)` (limit 200 KiB, test
  header comment says 172 KiB measured).
- `--profile dev` (opt-level 1, less inlining): the same test FAILS, but by
  its own assertion, not by an overflow: `deqp_posvary_loop_phi_array takes
  231 KiB of stack` against the 200 KiB bound (tests/mali_locals.rs:175).
  The compile runs on a 16 MiB thread (tests/mali_locals.rs:152) and
  finishes; the other five tests pass.

## Verdict

Not reproduced. Most likely what mali-ref-struct saw is this dev-profile
assertion (or a build at another opt level), reported as an overflow; the
product's bound it guards is the 256 KiB compile thread of zephyr/Kconfig,
and the measure is 231 KiB there, i.e. still inside it but past the test's
200 KiB guard. Neither a harness stack artifact nor an unbounded recursion
(the stack reading is bounded: 178 to 231 KiB over every reference pair,
including the shapes that carry 256 locals). The lane_tree/vectorise cycle of
docs/notes/mali-deqp-defects.md is not involved in this test: no pair in
tools/mali/testdata reached it.

Nothing is changed, as the item's rule says for a non-reproduction.

## Which item fixes what

- The 200 KiB guard being profile-dependent (178 KiB at opt-level 3, 231 KiB
  at opt-level 1 for `deqp_posvary_loop_phi_array`) is a test-budget question
  for whoever next edits tests/mali_locals.rs (for instance a guard of
  256 KiB minus headroom under `cfg(debug_assertions)`-independent opt
  levels, or skipping the guard when not at opt-level 3). CI runs the test
  profile, so it is green today.
- If an overflow is seen again, record: profile, `rustc -V`, the test name,
  and `gdb -batch -ex run -ex bt` on the test binary. Real unbounded
  recursion belongs to texmisc-design (depth bound and a named `MaliError`).
