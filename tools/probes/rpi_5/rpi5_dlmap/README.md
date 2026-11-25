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

### 2026-11-25: the first map

One tryboot session on the Pi 5 (BCM2712 D0), with the image of `22c5fc5f`
(it printed `rpi5_dlmap: build 22c5fc5f`; Zephyr `af17c0c8aa2a`; sha256
`85c94c24989a`, 53536 bytes), built with `-DYAKOGL_WERROR=ON`.
`tools/board.sh pi5 run` with `BOARD_END_RE='^rpi5_dlmap: (done|FAIL)'`
ended:

```
board.sh pi5 rpi5_dlmap rc=4 verdict=no_record boot=tryboot row boot_verdict=complete result=none resets=0 record=absent back_after=178s
```

`boot_verdict=complete` means board.sh saw the end line. `verdict=no_record`
(rc 4) is expected, because board.sh's pi5 label table does not know
`rpi5_dlmap` yet. The board came back to Linux by itself through the PM
watchdog and tryboot, 178 s after the reboot. board.sh removed the staged
image and put tryboot.txt back, and both read back as before.

What the image printed (the run's `rpi5_dlmap:` lines, with the 80
`end-word` lines cut down to their first entries and their `... more` line):

```
rpi5_dlmap: hvs version 0x00002454 stepping D0 cxm_size 0x00001000
rpi5_dlmap: ch0 ctrl0 0x877f0437 lptrs 0x000002a0 heade 672 dl 0x000002a0 status 0x00bd42a9
rpi5_dlmap: ch1 ctrl0 0x827f01df lptrs 0x00000000 heade 0 dl 0x00000000 status 0x003d61e0
rpi5_dlmap: ch2 ctrl0 0x00000000 lptrs 0x00000000 heade 0 dl 0x00000000 status 0x00400000
rpi5_dlmap: mem_words 4096 (CXM_SIZE 4096, mapped list words 22528)
rpi5_dlmap: span 0..32 n=33
rpi5_dlmap: span 34..36 n=3
rpi5_dlmap: span 38..672 n=635
rpi5_dlmap: span 674..676 n=3
rpi5_dlmap: span 678..4095 n=3418
rpi5_dlmap: end-word 0 0x80000000
rpi5_dlmap: end-word 12 0xb0b0b0b0
...
rpi5_dlmap: end-word 31 0xb0b0b0b0
rpi5_dlmap: end-word 36 0x80c00437
rpi5_dlmap: end-word 40 0xb0b0b0b0
...
rpi5_dlmap: end-word 99 0xb0b0b0b0
rpi5_dlmap: end-word ... more 3961
rpi5_dlmap: walk ch0 entry 0 word 672 ctl0 0x600cc004 valid 1 next 32
rpi5_dlmap: walk ch0 entry 1 word 704 ctl0 0x20000000 valid 0 next 32
rpi5_dlmap: walk ch0 entry 2 word 736 ctl0 0x20000000 valid 0 next 32
rpi5_dlmap: walk ch0 entry 3 word 768 ctl0 0x20000000 valid 0 next 32
rpi5_dlmap: walk ch0 entry 4 word 800 ctl0 0x20000000 valid 0 next 32
rpi5_dlmap: walk ch0 entry 5 word 832 ctl0 0x20000000 valid 0 next 32
rpi5_dlmap: walk ch0 entry 6 word 864 ctl0 0x20000000 valid 0 next 32
rpi5_dlmap: walk ch0 entry 7 word 896 ctl0 0x20000000 valid 0 next 32
rpi5_dlmap: walk ch0 entry 8 word 928 ctl0 0x80000000 valid 0 next 0
rpi5_dlmap: walk ch0 stop END at word 928
rpi5_dlmap: walk ch1 entry 0 word 0 ctl0 0x80000000 valid 0 next 0
rpi5_dlmap: walk ch1 stop END at word 0
rpi5_dlmap: walk ch2 entry 0 word 0 ctl0 0x80000000 valid 0 next 0
rpi5_dlmap: walk ch2 stop END at word 0
rpi5_dlmap: after-end none (no zero span of 20 words after word 928)
```

End line:

```
rpi5_dlmap: done (mem_words 4096, channel 0 END found)
```

`verdicts.sh` on the console, with its `HH:MM:SS ` prefix stripped, exited 0:

```
rpi5_dlmap verdict: mem_words 4096 (spans 5, zero spans 0, END-bit words 4041)
rpi5_dlmap verdict: ch0 heade 672: 9 entries, stop END at word 928
rpi5_dlmap verdict: ch1 heade 0: 1 entries, stop END at word 0
rpi5_dlmap verdict: ch2 heade 0: 1 entries, stop END at word 0
rpi5_dlmap verdict: fwend: word 928
rpi5_dlmap verdict: q4place: none (no zero span of 20 words after the END at word 928, none below HEADE 672)
rpi5_dlmap verdict: end: done (mem_words 4096, channel 0 END found)
rpi5_dlmap verdict: done
```

Reading it:

- The list memory is 4096 words (CXM_SIZE). The 256-word walk of
  `rpi5_scanout32`'s first run stopped one entry short. Channel 0's list
  starts at HEADE 672 and runs nine entries of NEXT 32 (one VALID plane at
  672, then seven entries with VALID clear, `0x20000000`). Its END is the
  word `0x80000000` at 928, so the firmware's list covers words 672..928.
- Channels 1 and 2 have HEADE 0. Word 0 is a bare END (`0x80000000`), and
  channel 1 is enabled (CTRL0 bit 31) on that empty list.
- No 20-word zero span exists anywhere: the only zero words are 33, 37, 673
  and 677. Word 678 onwards up to 4095 is non-zero, and 4041 words have
  bit 31 set. The ones printed outside the lists (12..31, 40..99) read
  `0xb0b0b0b0`. So `q4place` is `none`. The memory the firmware does not use
  seems to hold a fill pattern, not zeros. A zero-span test cannot find free
  room on this firmware. The words after 928 were not printed one by one, so
  this map does not prove that they are all `0xb0b0b0b0`.

The console was captured as in `rpi5_dispregs`. The farm's console lease
holds the serial bridge, so `BOARD_UART` was a local
`am62p_netboot_uart.py feed` relay of the lease's copy of the same console.
The relay started at the boot loader's `Starting OS` line. From
`board: watchdog armed` to the `feeding stopped` line, the image's output was
relayed whole (4949 bytes, the count the lease's console read). Every
`rpi5_dlmap:` line is as the board printed it. The run directory
`20261125-2057-rpi5_dlmap-85c94c24` stays outside the tree and is not
committed.
