<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx8mp-common-int, 2026-10-14, integration tip b568940021f0b097fb6dce01cd211ca0e305aac3

Session doc: `docs/board-sessions/imx8mp-common-int.md`. Result: the five
images (GM SO SV CL GT) were built from this tip; **the board run did not
happen** (the sandbox refused `BOARD_HOST=...` commands). No i.MX8MP number
changes, so no new cell text for `tools/cts/RESULTS.md`, `tools/cts/STATUS.md`
or the `README.md` Status table is proposed here. The cells stay as they are
until a session that can run the board has produced a record.

## State of main

No row changes. The i.MX8MP common-set evidence is still the `39bb67d` session
(2026-10-12): GM score 43, SO and SV pass, CL not run at that tip (pass at
`f013dbb`, crc16 b107), GT failed at case 16. The images at `b568940` build
(all five rc=0); their sha256 are in the session doc.

## Open work

- **Run the i.MX8MP common set (GM SO SV CL GT) on the board** from the
  integration tip. Recipe: the session doc's "To rerun it". It needs a session
  allowed to run `BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc tools/cts/cts-common.sh imx8mp run ...`.
  Then GT case 16, 10, 2 and 171 (viewwin) stay queued until a record names
  them; 10 and 2 pass or fail at random (`docs/investigations/gc-case10-flake.md`),
  so rerun GT once on a failure.
- **Cell text, to apply with the run's numbers** (fill `<...>` from the tally;
  cite the commit and the record directory `records/imx8mp/common-<sha8>`):
  - `tools/cts/RESULTS.md`, i.MX8MP common-set cells: `GM <score>, <n> scenes, 0 failed; SO <verdict>; SV <verdict>; CL <verdict>; GT <passed> of <run>, failing <cases>` at `<commit>`, record `records/imx8mp/common-<sha8>`.
  - `tools/cts/STATUS.md` (Japanese): `i.MX8MP 共通セット (<commit>, records/imx8mp/common-<sha8>): GM スコア <score>、SO <verdict>、SV <verdict>、CL <verdict>、GT <passed>/<run> (失敗: <cases>、viewwin 171: <pass|fail>)。`
  - `README.md` Status table, i.MX8MP row: `GM <score> / SO <verdict> / SV <verdict> / CL <verdict> / GT <passed> of <run>` (`<commit>`).
  Until then keep the current cells (the `39bb67d` numbers above).
- **V3D `mem.cpu` NULL guard** (`src/v3d/backend.rs`): the kept-image path at
  about line 3087 does `(mem.cpu as *mut u8).add(pad)` after `gpu_alloc`
  without checking `mem.cpu.is_null()`; other paths (lines 2150, 4089, 4110,
  4225) do check. Add the same guard, free the allocation and return
  `KeptMiss::NoMemory` (or `Address`).
- **osal-linux V3D row in `tools/check.sh`**: the host row list (about line
  2274) has `osal-linux,gpu-powervr,...` and `osal-linux,gpu-vivante,...` but
  no V3D entry; add one so the host-harness V3D build stays checked.
- **Re-check `docs/benchmarks.md` code references** with
  `python3 -I tools/docs/check-refs.py docs/benchmarks.md`. At this tip
  `tools/docs/check-refs.py` does not exist (no `tools/docs/` directory), so the
  check could not run here; it is probably on a branch not yet merged.

## Known defects

- gl_test case 16 on the GC7000UL (got 0x2e want 0x30) was the open GT failure
  at `39bb67d`; not re-observed at `b568940` (no run).
- Cases 10 and 2 pass or fail at random (unchanged, not re-observed).

## Next board session

i.MX8MP: run the common set in a session whose permissions allow
`BOARD_HOST=... BOARD_BOOT=uboot-mmc tools/cts/cts-common.sh imx8mp run ...`
(use `BOARD_BOOT=uboot-mmc`; the default is `uboot-tftp`). Images may be rebuilt
with `tools/cts/cts-common.sh imx8mp build <img-dir>` (needs
`AGL_SLINT_CLUSTER_DIR`; the deps cache has a clone). Record GM's score and
per-scene counts, GT's failing case and whether viewwin 171 passes.
