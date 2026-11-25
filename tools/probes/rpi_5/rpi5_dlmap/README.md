<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_dlmap

A read-only map of the HVS6 display-list memory on the Pi 5 (BCM2712 D0) and
of where the firmware's list really ends. Part of ROADMAP stage 5 (issue #7),
step P11 (U3) of `docs/notes/pre-study-rpi5.md`, Q4 of
`docs/notes/rpi5-display.md` section 4.

Why: `rpi5_scanout32`'s first run (2026-11-25, see its README, section
"placement refused, nothing written") walked the firmware's list from channel
0's `LPTRS.HEADE` (672) by `CTL0.NEXT` for at most `HVS6S_FW_MAX` = 256 words,
went through 8 entries of NEXT 32 (the last at word 896, `0x20000000`, VALID
clear) and found no END; words 32..671 held 619 non-zero words, so no 20-word
zero span either, and the placement was refused. This image shows what is in
the whole memory.

The image writes no HVS register and no list word (`src/main.c` has no write
call; the PM watchdog, in the board layer, is the only writer).

## What it reads

| Step | Reads | Prints |
|---|---|---|
| HVS | VERSION (stepping), CXM_SIZE (list memory size in words) | `hvs version ...`, `mem_words <n>` (capped at the mapped list words, 24576) |
| channels | per channel 0..2: CTRL0, LPTRS (HEADE = bits 11:0), DL, STATUS (`hvs6_chan.h`) | `ch<n> ctrl0 ... heade ... dl ... status ...` |
| list memory | words 0..mem_words-1 at HVS + 0x4000, copied once into RAM with 32-bit reads | `span <start>..<end> n=<count>` per maximal non-zero run (at most 150 lines), `zero <start>..<end>` per zero run of at least 20 words (at most 80), `end-word <i> 0x<val>` per word with bit 31 set (at most 80); a `... more <n>` line says what was left out |
| walk | from each channel's HEADE by CTL0.NEXT (`hvs6_dlist.h`: END bit 31, VALID bit 30, NEXT 29:24), at most 64 entries per channel | `walk ch<n> entry <k> word <i> ctl0 0x.. valid <0\|1> next <n>`, then `walk ch<n> stop END at word <W>` or `NEXT 0 at word <W>`, `past memory at word <W>`, `step bound at word <W>` |
| after the END | the first zero run of at least 20 words after channel 0's END (not below word 32) | `after-end zero <start>..<end>` or `after-end none (<reason>)` |

The end line is `rpi5_dlmap: done (...)` or `rpi5_dlmap: FAIL (<reason>)`,
then `feeding stopped, reset expected`. The output is under about 600 lines in
the worst case.

## Build

```sh
west build -p always -b rpi_5 -d build/rpi5_dlmap yakogl/tools/probes/rpi_5/rpi5_dlmap -- -DYAKOGL_WERROR=ON
```

with `ZEPHYR_WS`, `ZEPHYR_SDK_INSTALL_DIR` and `west` from `$ZEPHYR_WS/.venv/bin`
as for `rpi5_dispregs`. `tools/check.sh`'s zephyr row picks the directory up by
its glob.

## Run

```sh
BOARD_END_RE='^rpi5_dlmap: (done|FAIL)' \
  tools/board.sh pi5 run build/rpi5_dlmap/zephyr/zephyr.bin <logdir> rpi5_dlmap
sed -E 's/^[0-9]{2}:[0-9]{2}:[0-9]{2} //' <logdir>/<run>/console.txt > console.log
sh tools/probes/rpi_5/rpi5_dlmap/verdicts.sh console.log
```

The PM watchdog (armed in the board layer's first init hook, fed for 60 s at
most) resets the board within 10 s of the end line, and tryboot returns it to
Linux, as for `rpi5_dispregs`.

`verdicts.sh` prints, among a detail line per channel:

```
rpi5_dlmap verdict: fwend: word <W>|none
rpi5_dlmap verdict: q4place: after-end <start>|before-head <start>|none (<reason>)
```

`fwend` is the word of the END that stopped channel 0's walk. `q4place` is where
a second list of 20 words or more may start: after the firmware's END, else
below its HEADE (from word 32), else nowhere. Exit status: 0 done, 1 no end
line, 2 usage, 3 FAIL. `verdicts_selftest.sh` covers each case, including a log
shaped like the 2026-11-25 numbers (HEADE 672, no END).

## On the board

Not run yet.
