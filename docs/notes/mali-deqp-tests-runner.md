# Running the `tests/mali_deqp_*` tests: `tools/mali/run-deqp-tests.sh`

Every `tests/mali_deqp_*.rs` file names its feature set in its header, as
the line `//! cargo test <flags> --test <name>` (today always
`--no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table`).
The script reads that line from each file, groups files with identical
flags into one `cargo test <flags> --test a --test b ...` and runs the groups.

```text
tools/mali/run-deqp-tests.sh --list                       # print the invocations
tools/mali/run-deqp-tests.sh --only 'tests/mali_deqp_t3gap.rs'
tools/mali/run-deqp-tests.sh --include-ignored            # passes -- --include-ignored
```

- `--only <glob>` (repeatable) selects files by path glob or base name.
- A file without a header line is reported on stderr and counts as failed;
  the exit code is then nonzero.
- The last line is `N groups, M tests files, K failed`; exit is nonzero
  if K is not 0.
- It is not part of `tools/check.sh`. Use it for plan #387's acceptance
  check c3 and for a compiler train's per-step byte-compare tests (`--only`
  with the touched files). The whole set is long: run it in CI or on a
  build machine, not as a local habit.
