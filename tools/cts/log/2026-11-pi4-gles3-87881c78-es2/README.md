<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES2 regression rows (D FB1 T1 T3 TF) at 87881c78

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

The ES 2.0 regression rows of `docs/es3/board-recipe-pi4.md` section 8 on the
Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item board-es2 of plan
#399 (issue #1261). Rows 1, 4, 19, 21 and 24 (counted from 0) of
`tools/cts/pi4-full-tree.txt`, built from a detached worktree of `87881c78`
with `tools/cts/pi4-deqp-images.sh` (no `DEQP_MODULE`; the gles2 stage 1 of
`deps-env.sh --build --board rpi_4b DEQP_WORK`): five `rc=0 build=87881c78`
lines. Image sha256 (the board read each staged file back with the same
value): D `6503264e...9294`, FB1 `039e38da...ec61`, T1 `64067cdf...e519`,
T3 `171f9aff...d95b`, TF `8014b96d...8b62`.

## Result

All five images ran to the end (`complete=1`, `N cases begun, N with a
result, 0 without`), and every case they began passed: 1,245 of 1,245. The
mustpass columns repeat the `b1b5b845` record's (`../2026-11-pi4-es2-b1b5b845/`):
the filters are the same, and since every begun case passed, every mustpass
case in them passed. No Fail, no NotSupported, no GPU recovery, no MMU fault,
no hung job, no slow job, no allocation refused; the longest run is T3's
43.2 s.

| row | image | mustpass | Pass | Fail | NotSupported | other | begun | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | D | 292 | 292 | 0 | 0 | 0 | 305 | 12.1 s | 0 | none | DONE, all passed |
| 4 | FB1 | 390 | 390 | 0 | 0 | 0 | 392 | 11.7 s | 0 | none | DONE, all passed |
| 19 | T1 | 181 | 181 | 0 | 0 | 0 | 224 | 27.1 s | 0 | none | DONE, all passed |
| 21 | T3 | 304 | 304 | 0 | 0 | 0 | 304 | 43.2 s | 0 | none | DONE, all passed |
| 24 | TF | 17 | 17 | 0 | 0 | 0 | 20 | 2.0 s | 0 | none | DONE, all passed |
| | total | 1,184 | 1,184 | 0 | 0 | 0 | 1,245 | | 0 | | |

Against the last Pi 4 ES 2.0 record (`tools/cts/RESULTS.md`, 17,126 pass, 0
fail, 34 NotSupported over the whole tree; the rows at `b1b5b845`: the same
1,184 / 0 / 0): **no change, nothing moved from Pass.**

## The consoles are excerpts

The leased serial console reaches the session only as a tool whose output has
to be copied into the record, so a dEQP console of 120-420 KB per image could
not be kept whole. `<LABEL>.console.txt` holds, after two `#` header lines,
the end of each run read from the console: dEQP's `DONE!` block, the
`deqp: done` and `deqp: summary` lines, the `v3d_deqp DONE` line with its
`<n> cases in <ms> ms`, the record's job, carve-out and refusal lines, the
case-order line and the last `v3d_deqp: build 87881c78 DONE` line. The
per-case lines before them are not kept, and there are no `<LABEL>.tally.txt`
files (the tally needs the per-case lines); the per-row counts above are the
images' own `deqp: summary` and `done:` lines. The firmware and Linux lines
are not kept either.

## Method

By hand, as the `b1b5b845` sessions did (the serial console was held by the
leased console tool, so `tools/board.sh`'s own capture was not available), one
image at a time: ssh prechecks (sudo, no leftover `yakogl-*.bin` or
`tryboot.txt` beyond one stale `tryboot.txt` of an earlier session, removed;
`config.txt` was not touched), the image installed to `/boot/firmware` and a
`tryboot.txt` naming it, the image read back by sha256, `sudo reboot "0
tryboot"`, the console read after the run, Linux back (a new boot id), then
both files removed and read back (none left). Every cycle came back clean.
