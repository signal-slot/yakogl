<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_scanout32

Stage 2 of `docs/notes/rpi5-display.md` (section 3, question Q4): our own
32 bpp XRGB8888 plane placed after word 32 of the firmware's display list,
channel 0's list pointer swapped at vblank between two buffers, then the
firmware's list restored byte-equal. Part of ROADMAP stage 5 (issue #7), step
P11 of `docs/notes/pre-study-rpi5.md`.

The image programs no pixel valve, HDMI or PHY register: it reads the pixel
valve's `PV_CONTROL` and `PV_V_CONTROL`, and writes only HVS display-list
words after word 32 and channel 0's `LPTRS`.

## What the image does

Every step prints a line, also when it fails or cannot run.

| Step | What | Console |
|------|------|---------|
| S1 | HVS `VERSION`, stepping (D0 needed: the swap's offsets are D0's), `CXM_SIZE` | `rpi5_scanout32: S1 ...` |
| S2 | channel 0's `CTRL0`, `LPTRS`, `STATUS` (`FRCNT`), `DL`, decoded | `rpi5_scanout32: S2 ...` |
| S3 | the firmware's list word by word through word 32 to its END, its first plane decoded | `rpi5_scanout32: S3 ...` |
| S4 | pixel valve 0 (HDMI0) `PV_CONTROL`, `PV_V_CONTROL`, read only | `rpi5_scanout32: S4 ...` |
| S5 | `hvs6s_snapshot`: `LPTRS`, words 0..31 and the firmware's list kept for the restore; FNV-1a hash of words 0..31 | `u3 snapshot: list-ptr=0x.. words=32 hash=0x..` |
| S6 | `hvs6s_place`: one span of 20 words after word 32 and clear of the firmware's list, slot A (10 words) and slot B (the next 10); refused when it does not fit | `u3 dlist: placed word <n> len 20 ...` |
| S7 | buffers A and B (`.bss`, 1920x1080 or the firmware plane's size if smaller, pitch `w*4`): A the `pattern.h` bars as XRGB8888, B the same bars with red and blue exchanged; cleaned with `sys_cache_data_flush_range` | `u3 plane: addr=0x.. fmt=XRGB8888 pitch=.. w=.. h=.. addr_b=0x..` |
| S8 | 8 swaps, A, B, A, B ...: `hvs6s_swap` writes the slot's list (`hvs6_dlist_write.h`, NEXT=9, inner and list END, CTL2 0xfff0, alpha-mask FIXED), reads it back, then writes `LPTRS` once; waits for the read-back and an `FRCNT` advance, then two frames past the write before the other slot is touched. Each swap is judged by `swap_check.h`'s `swap_judge`, the sequence by `swap_seq_judge` (the plane entry read back from the slot must name the step's buffer). Each buffer stays up 1 s | `dlswap at-write/after ...`, `dlswap swap<k> ...`, `u3 swap <k>: buf A\|B list-ptr=0x.. frcnt <before> -> <after> judge=...`, `u3 named: yes\|no ...` after swap 1 |
| S9 | a 16-pixel grid of each buffer read back after a clean+invalidate (`pattern_check`) | `rpi5_scanout32: S9 readback ...` |
| S10 | `hvs6s_restore`: the firmware's `LPTRS` written back, two frames, then `LPTRS`, words 0..31 and the firmware's list compared with the snapshot, again through `swap_check.h`'s `swap_restore_judge` | `u3 restore: PASS byte-equal ... hash=0x..` or `u3 restore: FAIL ...` |
| | the `hvs6_dlist_swap.h` record of the placement, swap 1 and the restore | `dlswap snapshot/place/write/.../result ok\|FAIL ...` |
| | the end line, then the watchdog is no longer fed | `rpi5_scanout32: PASS (...)` or `rpi5_scanout32: FAIL (reason)`, then `rpi5_scanout32: feeding stopped, reset expected ...` |

The restore runs whenever the snapshot was taken, whatever failed after it.
When nothing could be placed the swap lines read `u3 swap <k>: skipped (...)`,
which `verdicts.sh` does not count: the log is then `incomplete` and Q4
`unanswered`, while the image's own end line is `FAIL`.

Buffers A and B are RAM in the image's own cacheable mapping; they are never
mapped a second time (`device_map` of a RAM page is `-EBUSY` and a panic, as
`rpi5_mbox` found before 81375c1b). The list memory is device memory in the
SoC window, so the list clean of `hvs6_dlist_swap.h` is a no-op there.

## How to run

Build (from the Zephyr workspace, `ZEPHYR_TOOLCHAIN_VARIANT=zephyr`):

```sh
west build -p always -b rpi_5 -d build/rpi5_scanout32 yakogl/tools/probes/rpi_5/rpi5_scanout32 -- -DYAKOGL_WERROR=ON
```

A board session runs it through `tools/board.sh pi5 run <zephyr.bin> <logdir>`
(tryboot; the PM watchdog brings the board back to Linux) with

```sh
BOARD_END_RE='^rpi5_scanout32: (PASS|FAIL)'
```

then `verdicts.sh` on the captured console:

```sh
tools/probes/rpi_5/rpi5_scanout32/verdicts.sh <logdir>/console.txt
```

The rules of `tools/probes/rpi_5/README.md` apply. A run that does not come
back is reported as needing a person.

Host checks: `sh tools/probes/rpi_5/rpi5_scanout32/verdicts_selftest.sh`
(the verdict script) and `sh tools/probes/rpi_5/rpi5_scanout32/swap_check_selftest.sh`
(the decision header `src/swap_check.h`, 52 checks).

## What it proves

`verdicts.sh` PASS means, from the board's registers and the bytes read back
(no camera):

- our list was placed at or after word 32, clear of the firmware's list;
- channel 0's `LPTRS` read back as our list after the first swap, and every
  swap, alternating A and B at least 4 times, saw `FRCNT` advance (the
  channel kept scanning across the pointer write);
- the firmware's `LPTRS`, words 0..31 and its list read back byte-equal to
  the snapshot (hash of words 0..31 equal) after the restore.

It answers Q4 of `docs/notes/rpi5-display.md`: whether channel 0's list
pointer can be swapped at vblank under a running scanout. It does not prove
that the HVS fetched our buffer: there is no latch bit (linuxdesk), `DL.LACT`
following our list is recorded only (`lact-follows`), and the farm has no
camera on the Pi 5. The plane's UPM bits that Linux sets in PTR0 are not
written (`hvs6_dlist_write.h`); whether a raw RGB plane needs them is a board
question this probe can only bound.
