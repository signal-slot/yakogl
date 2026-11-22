<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_scanout32

Stage 2 of `docs/notes/rpi5-display.md` (section 3, question Q4): our own
32 bpp XRGB8888 plane placed in list words at or after word 32 that read
zero, clear of the firmware's display list (wherever channel 0's
`LPTRS.HEADE` names it, word 672 on the board, walked to its END by
`hvs6_dlist_swap.h`), channel 0's list pointer swapped at vblank between two
buffers, then the firmware's list restored byte-equal. Part of ROADMAP stage 5 (issue #7), step
P11 of `docs/notes/pre-study-rpi5.md`.

The image programs no pixel valve, HDMI or PHY register: it reads the pixel
valve's `PV_CONTROL` and `PV_V_CONTROL`, and writes only channel 0's `LPTRS` and
HVS display-list words it has read as zero and found outside every list it
can see; when that check refuses, it writes nothing at all.

## What the image does

Every step prints a line, also when it fails or cannot run.

| Step | What | Console |
|------|------|---------|
| S1 | HVS `VERSION`, stepping (D0 needed: the swap's offsets are D0's), `CXM_SIZE` (the list memory in words, passed as `mem_words`) | `rpi5_scanout32: S1 ...` |
| S2 | channel 0's `CTRL0`, `LPTRS`, `STATUS` (`FRCNT`), `DL`, decoded; `STATUS` read 6 times 20 ms apart, `FRCNT` shown advancing | `rpi5_scanout32: S2 ...` |
| S3 | the firmware's list from `LPTRS.HEADE`, word by word to its END (or its first 64 words when no END is in view), its first plane decoded | `rpi5_scanout32: S3 ...` |
| S4 | pixel valve 0 (HDMI0) `PV_CONTROL`, `PV_V_CONTROL`, read only | `rpi5_scanout32: S4 ...` |
| S5 | the plan's `hvs6s_snapshot`, read only: `LPTRS` and the firmware's list from `HEADE`, walked entry by entry (`CTL0.NEXT`) to its END word, at most 256 words; refused (`FW_NOEND`, `FW_WALK`) when the walk reaches no END, and then nothing is written. FNV-1a hash of the words `[HEADE, HEADE + fw_len)`, leaving out the declared hardware-written words | `S5 dlswap snapshot ok\|FAIL ... entries .., walk stopped at word ..`, `S5 hw words '<knob>': <set>`, `u3 snapshot: list-ptr=0x.. heade=<n> words=<fw_len> hash=0x.. hwskip=<set>` (`words=0 ... FAIL` when refused) |
| S6 | the plan's `hvs6s_place_io`, read only: one span of 20 words, slot A (10 words) and slot B (the next 10), the first at or after word 32 whose words all read zero, clear of words 0..31 and of the firmware's span `[HEADE, HEADE + fw_len)`, within `mem_words` and the 12-bit `HEADE`; then refused if any channel's `LPTRS.HEADE` or `DL.LACT` is inside it. A refusal writes nothing | `rpi5_scanout32: S6 ch<n> ...`, `S6 placement check ok\|FAIL\|skip ...`, `u3 dlist: placed word <n> len 20 ...` or `u3 dlist: refused <reason>` |
| S7 | buffers A and B (`.bss`, 1920x1080 or the firmware plane's size if smaller, pitch `w*4`): A the `pattern.h` bars as XRGB8888, B the same bars with red and blue exchanged; cleaned with `sys_cache_data_flush_range` | `u3 plane: addr=0x.. fmt=XRGB8888 pitch=.. w=.. h=.. addr_b=0x..` |
| S8 | 8 swaps, A, B, A, B ... Swap 1 is `hvs6s_run` with `cfg.start` = slot A: its own snapshot (compared with the plan's), slot A read and checked again, the swap to A and that run's restore, with its `dlswap` record. Slot B is then read and checked by `hvs6s_place_io`, and swaps 2..8 go through `hvs6s_swap` (only when swap 1 wrote `LPTRS` and its restore was byte-equal). `hvs6s_swap` writes the slot's list (`hvs6_dlist_write.h`, NEXT=9, inner and list END, CTL2 0xfff0, alpha-mask FIXED), reads it back, then writes `LPTRS` once; waits for the read-back and an `FRCNT` advance, then two frames past the write before the other slot is touched. Each swap is judged by `swap_check.h`'s `swap_judge`, its sample filled with the firmware's span (`fw_start` = `HEADE`, `fw_words` = the walked length) and ours (`our_words` = 10); the sequence by `swap_seq_judge` (the plane entry read back from the slot must name the step's buffer). Each buffer stays up 1 s | `dlswap at-write/after ...`, `dlswap swap<k> ...`, `u3 swap <k>: buf A\|B list-ptr=0x.. frcnt <before> -> <after> judge=... fw=<heade>+<words> ours=10`, `u3 named: yes\|no ...` after swap 1; `dlswap readback-diff ...` for a slot word that did not read back as written |
| S9 | a 16-pixel grid of each buffer read back after a clean+invalidate (`pattern_check`) | `rpi5_scanout32: S9 readback ...` |
| S10 | `hvs6s_restore` after swaps 2..8: the firmware's `LPTRS` written back, two frames; then `LPTRS` and the firmware's words `[HEADE, HEADE + fw_len)` compared with the snapshot through `swap_check.h`'s `swap_restore_judge`; our 20 words written back to zero once channel 0's `LPTRS` and `DL.LACT` are off them. When nothing more was written, only the compare runs | `dlswap restore-diff ...` per differing word and `dlswap restore-diffs total=..`, `rpi5_scanout32: S10 ...`, `u3 restore: PASS byte-equal [except hw words off <offs> (..)] list-ptr=0x.. heade=<n> words=<n> hash=0x.. hwskip=<set>` or `u3 restore: FAIL ... hwskip=<set> undeclared=<n>` |
| | the `hvs6_dlist_swap.h` record (`hvs6s_lines`): after `hvs6s_run` in S8, or of the plan when nothing ran | `dlswap snapshot ok\|FAIL ... entries=N end=W\|stop=W word=0x..`, `dlswap place ok ... skipped=N\|FAIL ...`, `dlswap restore-diffs ...`, `dlswap restore-words ok fw A..B equal` (`... byte-equal except hw words off <offs> (<n> differed) hwskip=<set>` with a set), `dlswap result ok\|FAIL ...` |
| | the end line, then the watchdog is no longer fed | `rpi5_scanout32: PASS (...)` or `rpi5_scanout32: FAIL (reason)`, then `rpi5_scanout32: feeding stopped, reset expected ...` |

The restore runs whenever anything was written, whatever failed after it.
When nothing could be placed (the snapshot found no END, or no zero span
was clear) the probe prints `u3 dlist: refused <reason>` and the swap lines
read `u3 swap <k>: skipped (...)`; `verdicts.sh` then gives `FAIL (placement
refused: ...)` and Q4 `unanswered`. Swaps skipped after swap 1 (slot B
refused, or swap 1's restore not byte-equal) are not counted by
`verdicts.sh`: with fewer than 4 swap lines the log is `incomplete`.

Buffers A and B are RAM in the image's own cacheable mapping; they are never
mapped a second time (`device_map` of a RAM page is `-EBUSY` and a panic, as
`rpi5_mbox` found before 81375c1b). The list memory is device memory in the
SoC window, so the list clean of `hvs6_dlist_swap.h` is a no-op there.

## Hardware-written words

The HVS may write a word of a plane entry back while it scans it (a context
or status word; `rpi5_scanout16` saw one word of the firmware's list change
during its hold). A byte-equal restore would then fail however correct the
swap was. The CMake knob `RPI5_SCANOUT32_HW_WORDS` declares such words by
their offset from the start of a plane entry, each one

- `skip`: left out of the byte-equal compares and of the hash, or
- `expect`: compared, a change printed and counted, never failing.

The set is `<off>:skip|expect` items joined by `,` (offsets 0..62, at most
4, each once), for example `-DRPI5_SCANOUT32_HW_WORDS="4:skip"`. The default
is `4:skip`: the plane entry's word 4 (the POS3 context word) is left out of
the byte-equal compares and of the hash. Source: `rpi5_scanout16`'s board run
of 2026-11-22 (its README, "Q2's list word"): `u3 q2: word 676 entry-off 4
was 0x8000005d now 0x800000d2 per-frame no`, q2 verdict `hvs-writeback`. Read
right after each of 32 FRCNT steps the word was 0x80008000, and at the hold
samples it was a different 0x800000xx..0x800003xx value nearly every time:
the HVS writes it while scanning, so it will not equal the snapshot after a
swap. It is `skip` rather than `expect` because it is a hardware status
word, not a rare firmware edit. Every other word stays compared;
`-DRPI5_SCANOUT32_HW_WORDS=""` compares every word again. The build refuses a
malformed set; the image prints `S5 hw words '<knob>': <set>`.

Entries are found by `CTL0.NEXT` from the start of the reference words (the
snapshot's, or the words written): the set applies to every entry of the
firmware's span (S5's hash, the plan/run snapshot compare, both restores)
and to our own slot (`hvs6s_swap`'s read-back before `LPTRS`, and the slot
read after each swap, judged by `swap_seq_judge_hw`). An END word, or a word
after a `NEXT` 0, is in no entry and never declared.

Every differing word is printed, at most 8 per compare and all counted:

```text
dlswap restore-diff word <n> entry <e> off <k> was 0x.. now 0x.. hw=<skip|expect|no>
dlswap restore-diffs total=<n> undeclared=<n> skip=<n> expect=<n> shown=<n> hwskip=<set>
dlswap readback-diff word <n> entry <e> off <k> was 0x.. now 0x.. hw=<skip|expect|no> [(swap <k>, after)]
```

(`entry none off none` for a word in no entry). `u3 snapshot:` and
`u3 restore:` carry `hwskip=<set>` (`none` when empty); a PASS with a
non-empty set reads `PASS byte-equal except hw words off <offs> (...)`.
`verdicts.sh` passes the restore only when every differing word's offset is
in the set with its mode, and FAILs naming word, entry, offset and values
otherwise, also under a probe PASS; a log without `hwskip=` (an older probe)
is judged with the empty set.

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
(the verdict script, 104 checks), `sh tools/probes/rpi_5/rpi5_scanout32/swap_check_selftest.sh`
(the decision header `src/swap_check.h`, 108 checks) and
`sh tools/v3d/check71.d/hvs6_dlist_swap.sh` (`hvs6_dlist_swap.h` against its
channel model, 185 checks).

## What it proves

`verdicts.sh` PASS means, from the board's registers and the bytes read back
(no camera):

- our list was placed at or after word 32, in words that read zero, clear of
  the firmware's list `[HEADE, HEADE + fw_len)` walked to its END, and of
  every channel's list pointer;
- channel 0's `LPTRS` read back as our list after the first swap, and every
  swap, alternating A and B at least 4 times, saw `FRCNT` advance (the
  channel kept scanning across the pointer write);
- the firmware's `LPTRS` and its words `[HEADE, HEADE + fw_len)` read back
  byte-equal to the snapshot (hash equal) after the restore, except the
  declared hardware-written words when a set is given (`hwskip=`, every
  differing word named), and our words were back to zero. Words 0..31 are never written (placement refuses them)
  and no longer compared.

It answers Q4 of `docs/notes/rpi5-display.md`: whether channel 0's list
pointer can be swapped at vblank under a running scanout. It does not prove
that the HVS fetched our buffer: there is no latch bit (linuxdesk), `DL.LACT`
following our list is recorded only (`lact-follows`), and the farm has no
camera on the Pi 5. The plane's UPM bits that Linux sets in PTR0 are not
written (`hvs6_dlist_write.h`); whether a raw RGB plane needs them is a board
question this probe can only bound.
