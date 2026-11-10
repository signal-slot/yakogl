<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/check.sh` from a symlinked path: the release-key rows fail, and the `cd -P` fix (not applied)

Run from a path whose `pwd` differs from `pwd -P`, `tools/check.sh` fails the
rows that compare cargo's dep-info with the repository root. The fix is one
word. It is **not applied**: `tools/check.sh` belongs to plan #193, which
applies it. Until then, run the gate from a canonical path.

## 1. What was measured

Fresh HTTPS clone of `origin/main` at 962318605a3e (2026-11-10), at
`$CONNECTED_DEPS_DIR/symlink-record/clone`, run through a symlink
`.../symlink-record/links/yk` (`pwd` = `.../links/yk`, `pwd -P` =
`/home/orion/.local/share/ccfarm/deps/yakogl/symlink-record/clone`).
`MALI_CSFFW` set to the pinned file of `tools/cts/deps.toml` (282624 bytes).
Host: 4 cores, rustc 1.101.0-nightly (5d8fec066 2026-10-16), clippy 0.1.101,
cargo-nextest 0.9.148. Command: `tools/check.sh --quick`, from a cold tree.

| run | summary line | wall time |
| --- | --- | --- |
| unpatched | `169 ok, 6 failed, 2 gap(s)` | 3867 s |
| `cd -P` (scratch clone only), `CC_aarch64_unknown_none` unset | `175 ok, 0 failed, 2 gap(s)` | 2186 s (warm tree) |

The two gaps are the host's, in both runs: no `lz4` command (am62p second
stage) and no `hal_nxp` (`HAL_NXP_DIR`).

The six unpatched failures are two causes:

- Path (the subject of this note), three-way release-key family:
  `no table in the artefact`, `no test knob in the artefact`
  (`dep_info_within: .../target/release/libyakogl.d` and `libyakogl_capi.d`
  "reads inputs the row's key does not hold (RELEASE_KEY_PATHS)", every
  `src/**`, `build.rs`, `tests/reference/*.rs` input listed); the third row of
  that family, `no Mesa reference in the artefact`, failed in this run on
  cause 2 (its control build), so it is not evidence for the path.
- Host environment, not the path: `clippy osal-zephyr,gpu-powervr,...
  (aarch64-unknown-none)`, `clippy osal-zephyr,gpu-v3d,...` and `check
  osal-zephyr,gpu-powervr,precompiled-shaders` failed in `build.rs` (cc-rs):
  the shell had `CC_aarch64_unknown_none=/home/orion/connected-deps/bootlin-aarch64/
  aarch64--glibc--stable-2024.05-1`, a directory that does not exist
  ("failed to find tool"). The patched run unset it. The unpatched run was
  not repeated without it, so the claim is: `no table` and `no test knob`
  are the path's; the three aarch64 rows and `no Mesa reference` are the
  stale variable's.

## 2. Cause

- `tools/check.sh:107`: `cd "$(dirname "$0")/.." || exit 2`. A logical `cd`
  keeps the symlink in `$PWD`.
- `tools/check.sh:2821` `dep_info_within`: passes `"$PWD"` as `root`
  (line 2824) to the Python, which takes `os.path.realpath(d)` of each
  dep-info entry (physical) and tests `p.startswith(root + "/")`. With a
  logical root no input is under it, every `src/**` file falls to `outside`
  and the row fails. `RELEASE_KEY_PATHS` (line 2802) is relative to the same
  root, so the keyed comparison is the one that breaks.

## 3. Proposed patch (not applied; for plan #193)

```diff
--- a/tools/check.sh
+++ b/tools/check.sh
@@ -107 +107 @@
-cd "$(dirname "$0")/.." || exit 2
+cd -P "$(dirname "$0")/.." || exit 2
```

Tested only in the scratch clone (the second row of the table). A
belt-and-braces alternative inside `dep_info_within` is to pass
`$(pwd -P)` instead of `$PWD` at line 2824.

## 4. Workaround

Run the gate from a canonical path (`pwd` equal to `pwd -P`), or
`cd -P` into the clone before calling `tools/check.sh`. Also unset a stale
`CC_<target>` variable that names a missing compiler.
