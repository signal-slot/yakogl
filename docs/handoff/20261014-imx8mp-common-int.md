<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx8mp-common-int, 2026-10-14, integration tip b568940021f0b097fb6dce01cd211ca0e305aac3

Session doc: `docs/board-sessions/imx8mp-common-int.md`. Record directory (not
committed): `records/imx8mp/common-b5689400`. The common set (GM SO SV CL GT)
ran on the Verdin i.MX8M Plus from images built at `b568940`.

## State of main

i.MX8MP common-set row, as measured at `b568940` (2026-10-14): GM glmark2 score
43 (33 scenes ran, 0 failed, 0 unsupported); SO and SV pass (crc16 c592);
CL passes (crc16 b107); GT 48 of 49 run, case 16 skipped, one failure: case 44.
Cases 2, 10 and 171 (viewwin) passed in this run.

## Open work

- **Apply the i.MX8MP cell text** below once the files are free (another plan
  holds them); each cites `b568940` and `records/imx8mp/common-b5689400`:
  - `tools/cts/RESULTS.md`, i.MX8MP common-set cells: `GM score 43, 33 ran, 0 failed, 0 unsupported; SO pass (crc16 c592); SV pass (crc16 c592); CL pass (crc16 b107); GT 48 of 49 run, case 16 skipped, failing case 44 only (cases 2, 10, 171 passed)`, on `b568940` (2026-10-14), record `records/imx8mp/common-b5689400`.
  - `tools/cts/STATUS.md` (Japanese): `i.MX8MP 共通セット (b568940, 2026-10-14, records/imx8mp/common-b5689400): GM スコア 43 (33 シーン実行、失敗 0)、SO・SV・CL は pass (crc16 c592 / c592 / b107)、GT は 49 件中 48 件 pass (case 16 は skip、失敗は case 44 のみ。case 2・10・171 は pass)。`
  - `README.md` Status table, i.MX8MP row: `GM 43 / SO pass / SV pass / CL pass / GT 48 of 49 run (case 16 skipped, case 44 fails)` (`b568940`).
- **gl_test case 44** (`es3hdr`, RGBA16F render target, issue #6) is the only
  GT failure left on this board; unchanged.
- **Case 171 (viewwin)**: passed at `b568940`; it can leave the queue (confirm
  once more on the next GT run, since 2 and 10 are random).
- **Cases 2 and 10** pass or fail at random (`docs/investigations/gc-case10-flake.md`);
  they passed in this one run, which settles nothing.
- **glmark2 terrain/refract on this board** are still not judged per scene
  (no console on the i.MX8MP): the record gives counts only.
- **V3D `mem.cpu` NULL guard** (`src/v3d/backend.rs`): the kept-image path at
  about line 3087 does `(mem.cpu as *mut u8).add(pad)` after `gpu_alloc`
  without a null check; lines 2150, 4089, 4110 and 4225 do check. Add the same
  guard, free the allocation and return `KeptMiss::NoMemory` (or `Address`).
- **osal-linux V3D row in `tools/check.sh`**: the host row list (about line
  2274) has `osal-linux,gpu-powervr,...` and `osal-linux,gpu-vivante,...` but
  no V3D entry; add one so the host-harness V3D build stays checked.
- **Re-check `docs/benchmarks.md` code references** with
  `python3 -I tools/docs/check-refs.py docs/benchmarks.md`. At this tip
  `tools/docs/check-refs.py` does not exist (no `tools/docs/` directory), so
  the check could not run; it is probably on an unmerged branch.

## Known defects

- gl_test case 44 on the GC7000UL (RGBA16F target, issue #6): still fails at
  `b568940`.
- Case 16 no longer fails in the common set: the set skips it by name
  (`docs/HANDOFF.md` #8 tracks the skip).

## Next board session

i.MX8MP: nothing is required by this result. If run again, use
`BOARD_BOOT=uboot-mmc` (the default is `uboot-tftp`) and rerun GT once if 2, 10
or 16 fail. The session doc's "To rerun it" has the commands.
