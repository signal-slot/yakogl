<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 24-29 (S03 S04 S05 S06 S07 S08) at 21730230

commit: 217302306411b84b35bbee9bff1444a0b61996af

Session `2026-11-20-pi4-gles3-21730230-S03-S08`: rows 24-29 of `tools/cts/pi4-gles3-tree.txt` (6 images: S03 S04 S05 S06 S07 S08).
This directory holds only this README: the session's consoles, tallies and per-cycle records live in the runner's gitignored records directory (see *Files*). Counts are the mustpass view of the tally's TOTAL line over each image's console.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 24 | S03 | 192 | 0 | 0 | 0 | 183.4 s | 0 | none | DONE, all passed |
| 25 | S04 | 116 | 0 | 0 | 0 | 208.4 s | 0 | none | incomplete (no complete deqp summary) |
| 26 | S05 | 184 | 18 | 0 | 0 | 151.2 s | 0 | none | DONE |
| 27 | S06 | 161 | 18 | 0 | 0 | 207.9 s | 0 | none | incomplete (no complete deqp summary) |
| 28 | S07 | 35 | 0 | 0 | 0 | 193.8 s | 0 | none | incomplete (no complete deqp summary) |
| 29 | S08 | 124 | 18 | 0 | 0 | 118.6 s | 0 | none | DONE |

(The table's "row" column counts from 1; the rows of this session are rows 23-28 counted from 0, the `TREE_ROWS` of the recipe.)

## Result

Run 2026-11-20 09:58 to 10:31 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item board-pi4 of plan #376 (issue #1261): the six `builtin_functions.precision` images that aborted on their first case at `8990a5cc` and `b1b5b845`, rebuilt from main `21730230`, which carries the V3D fix for targets up to 4096 (`6d2e7d0d`, `docs/notes/v3d-large-targets.md`). 1,147 mustpass cases in the six images, 866 with a result here: **812 pass, 54 fail, 0 NotSupported, 0 other, 281 without a result**. **No image aborted**: no `draw refused` line, no `GL_OUT_OF_MEMORY`, no ResourceError, no `Test run was ABORTED!` in any console; S03 `abs.highp_fragment.scalar`, S04 `cosh`, S05 `fract`, S06 `mix`, S07 `reflect` and S08 `sub` (the cases that stopped the images before) pass. 0 GPU recoveries, 0 MMU faults, 0 slow jobs, 0 hung jobs; `binner out of memory: 0 jobs` (2 overflow blocks of 256 KiB) and `0 overflow blocks fed by the ISR` in every image. Linux was back 297-303 s after each reboot and every cycle read back as before.

**S03, S05 and S08 ran through** (`complete=1`, every case begun has a result): S03 264 cases, 192 mustpass, all pass; S05 228 cases, 184 pass / 18 Fail; S08 150 cases, 124 pass / 18 Fail. The 54 Fails are the matrix-output precision cases `matrixcompmult` (S05), `outerproduct` (S06) and `transpose` (S08), each `{high,medium,low}p_{vertex,fragment}.{mat4,mat4x2,mat4x3}`, all `glGetError() returned GL_INVALID_ENUM at glsShaderExecUtil.cpp` at framebuffer setup: the 54 the host already shows, owned by plan #368.

**S04, S06 and S07 ran past the record deadline** (recipe section 6, *for time*; the first overrun of the tree): each ends `v3d_deqp: build 21730230 INCOMPLETE (the photo deadline came first)`, S04 at 131 cases (in `faceforward`), S06 at 179 (in `radians`), S07 at 47 (in `refract`); every case before the record passed or failed as above, and the rest have no result in this record (85, 25 and 171 mustpass cases). The precision cases are slow on the board: 0.1-4 s each, and `refract` 5-20.5 s (its vec4 cases about 20 s). The same images split for time (S04 in two, S06 in two, S07 in three with `refract` alone) ran on the same board from the same commit, each `DONE` with `complete=1`: 611 of 611 mustpass cases with a result, 593 pass, 18 Fail (`outerproduct`), so every one of the 1,147 cases has a board result at `21730230`, 1,093 pass and 54 Fail; those split runs are not in this directory (their labels are not rows of the tree until it is re-split). S03 (183 s) and S05 (151 s) are over the recipe's 150 s split threshold as well. A checker note for readers: `check-pi4-gles3-log.py` reports `aborted, no complete deqp summary (3): S04 S06 S07` for these three from their `complete=0`, the same line a dEQP abort gives; the cause is the time limit, not an abort, and the line clears when the tree is re-split.

The ES 2.0 regression rows of recipe section 8 ran in the same session from the same commit (10:32-11:00 UTC), each its own `board.sh` call:

| row | mustpass with a result | Pass | image's own count |
|---|---|---|---|
| D | 292 | 292 | 305/305 |
| FB1 | 390 | 390 | 392/392 |
| T1 | 181 | 181 | 224/224 |
| T3 | 304 | 304 | 304/304 |
| TF | 17 | 17 | 20/20 |

All five ended `DONE, all passed` with `complete=1` and 0 resets: 1,184 of 1,184 mustpass cases pass, every image `DONE, all passed`, nothing moved from Pass against the earlier section-8 result.

## Method

A clone of the repository at `21730230` (`git describe`: `21730230`). VK-GL-CTS 93bca01 with its externals, the gles2 stage 1 (`tools/cts/deps-env.sh --build --board rpi_4b DEQP_WORK`) and the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores), then the six images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` (section 3): 6 `rc=0 build=21730230` lines, every one `RAM: 166360 KB 256 MB 63.46%` (S03, S05: 166364 KB; 161,844 KB at `b1b5b845`), each `.config` passed by `check-deqp-images.py --config --gles3`.

The rows ran through `tools/board.sh pi4 tree` itself (S03 alone first, then one row per call into the same record; the five ES 2.0 rows and the seven split runs the same way). The board runner's verdict lines:

```
board.sh pi4 S03 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=298s
board.sh pi4 S04 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=302s
board.sh pi4 S05 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=303s
board.sh pi4 S06 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 S07 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 S08 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=303s
board.sh pi4 D rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=305s
board.sh pi4 FB1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=299s
board.sh pi4 T1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=305s
board.sh pi4 T3 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=300s
board.sh pi4 TF rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=300s
```

(`verdict=done` means the run completed; as the recipe says, an INCOMPLETE end line still reads `done`. The seven split runs printed the same fields with `back_after=` 295-314 s.) The leased serial console (an MCP server that connected forwards) held the UART bridge's one client slot, so `board.sh` read a local feed bridge (`tools/am62p_netboot_uart.py feed`, `tools/cts/README.md` "The console under a farm console lease"): after each image's end line the console of that cycle was read from the leased console in whole pieces and appended to the feed's file by a script that turns the console tool's escapes back into bytes and checks the byte count against the console offsets, never copied by hand. The capture's line times are therefore the relay's, not the board's; the run times above are the images' own `cases in` lines. `BOARD_HDMI_TIMINGS` was not set. `config.txt` was not touched.

## Files

This directory holds only this README. The full consoles are in the runner's gitignored records directory `records/pi4/20261120-0958-gles3-S03-S08-21730230/` (one subdirectory per cycle; each cycle's `console.txt` covers the image's console from the firmware's first line of its boot to the `v3d_deqp: build 21730230 ...` end line, CRs and NULs dropped, no time stamps), with `records/pi4/20261120-1032-es2-21730230/` for the five ES 2.0 rows and `records/pi4/20261120-1100-gles3-split-21730230/` for the seven split runs. The per-image tallies (`tools/cts/pi4-session-tally.sh`, deqp-uart-tally.py over the cut consoles), the `--table` over the six consoles and each cycle's `cycles.txt`/`meta.txt` (sha256 of `zephyr.bin`, the reboot time, the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line) are there with the consoles, not here.
