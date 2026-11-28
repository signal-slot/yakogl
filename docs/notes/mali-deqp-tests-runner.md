# Running every `tests/mali_deqp_*` test (`tools/mali/run-deqp-tests.sh`)

Each `tests/mali_deqp_*.rs` names its feature set in a header comment line,
`cargo test <flags> --test <name>` (today always
`--no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table`).
The script reads that line, groups files with identical flags into one
`cargo test <flags> --test a --test b ...` and runs the groups.

```text
tools/mali/run-deqp-tests.sh [--list] [--include-ignored] [--only <glob>]...
```

- `--list` prints the planned invocations and runs nothing.
- `--include-ignored` appends `-- --include-ignored` to every group.
- `--only <glob>` restricts the files (repeatable; the glob is relative to the
  repository root and must match `tests/mali_deqp_*.rs`, else exit 2).
- A file without a `cargo test ... --features ...` header line is reported on
  stderr, counts as failed and makes the exit code nonzero; the other files
  still run.
- The last line is `N groups, M tests files, K failed`; the exit code is 0
  only if K is 0.

Use it for plan #387's acceptance check c3 and for a compiler train's
per-step byte-compare run (`--only 'tests/mali_deqp_<family>*.rs'`).
It is not part of `tools/check.sh`.
