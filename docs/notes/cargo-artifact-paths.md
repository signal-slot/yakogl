<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Where cargo writes release artefacts across nightlies, and why `tools/check.sh` reads cargo's artifact messages

A gate row that inspects a built artefact (the rlib, its dep-info) cannot
assume a fixed path under `target/release/`: from rustc 1.101.0-nightly
(708c94ef0 2026-11-03) the intermediate artefacts are not there. This note
says what was measured, which rows are concerned, and the rule for new code.

## 1. What was measured

Both builds: `cargo build --release -p yakogl --no-default-features
--features host-test,gpu-vivante,shaderc-online --message-format=json` at
5bb0f0ad, with a fresh `CARGO_TARGET_DIR` (x86_64-unknown-linux-gnu).

| toolchain | `compiler-artifact` `filenames` of `yakogl` |
| --- | --- |
| rustc 1.101.0-nightly (5d8fec066 2026-10-16) | `release/libyakogl.rlib`, `release/build/yakogl/<hash>/out/libyakogl-<hash>.rmeta` |
| rustc 1.101.0-nightly (708c94ef0 2026-11-03), `nightly-2026-11-04` | `release/build/yakogl/<hash>/out/libyakogl-<hash>.rlib`, `.../libyakogl-<hash>.rmeta` |

On the 2026-10-16 nightly `release/libyakogl.rlib` and `release/libyakogl.d`
exist. On `nightly-2026-11-04` the directory `release/` has no
`libyakogl.*` at all; `find` shows the rlib, the rmeta and the dep-info
(`yakogl-<hash>.d`) only under `release/build/yakogl/<hash>/out/`. `<hash>`
(16 hex digits) is cargo's metadata hash and varies with the feature set.

Not measured: the `lib*.a` of a staticlib crate (`yakogl-capi`) on
`nightly-2026-11-04`, and any toolchain other than the two above (stable
included). Do not read this note as saying where they are.

## 2. The rule

A tool or gate row that needs a built artefact takes its path from the
`filenames` of cargo's JSON `compiler-artifact` messages
(`--message-format=json`, one JSON object per stdout line, the message with
the wanted `target.name` and `crate_types`). It never spells a fixed
`target/release/...` path and never a hash. `filenames` is a list: pick the
entry by extension, do not take the first. Keep cargo's own stderr out of
the row's stdout, since the row may be capturing it.

## 3. Consumers

`table_symbols`, `mesa_reference_symbols` and `knob_symbols` in
`tools/check.sh` read `release/libyakogl.rlib` and `release/libyakogl.d`
and fail on `nightly-2026-11-04`. They are fixed by the pending
table-artifacts change, not by this note.

An earlier audit found no other fixed-path consumer that breaks:
`tools/api_coverage.py`, `tools/slint/replay/run.sh`, the examples'
`CMakeLists.txt`, `capi/assemble.cmake` and `driver/zephyr/CMakeLists.txt`.
(That audit was not repeated here.)
