<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_scanout32

Stage 2 of `docs/notes/rpi5-display.md` (section 3; it answers Q4 of section 4): our own
32 bpp XRGB8888 plane placed right after the END of the firmware's display
list (wherever channel 0's `LPTRS.HEADE` names it, word 672 on the board,
walked over the whole list memory to its END by `hvs6_dlist_swap.h`; END at
word 928 in `rpi5_dlmap`'s map), at or after word 32, channel 0's list
pointer swapped at vblank between two
buffers, then the firmware's list restored byte-equal. Part of ROADMAP stage 5 (issue #7), step
P11 of `docs/notes/pre-study-rpi5.md`.

The image programs no pixel valve, HDMI or PHY register: it reads the pixel
valve's `PV_CONTROL` and `PV_V_CONTROL`, and writes only channel 0's `LPTRS` and
the 20 HVS display-list words after the END of the firmware's list, outside
every list it can see (each channel's list walked to its END). No word value
counts as free: the board's unused list memory holds a `0xb0b0b0b0` fill, not
zeros, so the probe saves those 20 words before its first write and writes
them back after the restore. When the placement refuses, it writes nothing at
all.

## What the image does

Every step prints a line, also when it fails or cannot run.

| Step | What | Console |
|------|------|---------|
| S1 | HVS `VERSION`, stepping (D0 needed: the swap's offsets are D0's), `CXM_SIZE` (the list memory in words, passed as `mem_words`) | `rpi5_scanout32: S1 ...` |
| S2 | channel 0's `CTRL0`, `LPTRS`, `STATUS` (`FRCNT`), `DL`, decoded; `STATUS` read 6 times 20 ms apart, `FRCNT` shown advancing | `rpi5_scanout32: S2 ...` |
| S3 | the firmware's list from `LPTRS.HEADE`, word by word to its END (or its first 64 words when no END is in view), its first plane decoded | `rpi5_scanout32: S3 ...` |
| S4 | pixel valve 0 (HDMI0) `PV_CONTROL`, `PV_V_CONTROL`, read only | `rpi5_scanout32: S4 ...` |
| S5 | the plan's `hvs6s_snapshot`, read only: `LPTRS` and the firmware's list from `HEADE`, walked entry by entry (`CTL0.NEXT`, entries with VALID clear included) to its END word over the whole list memory past `HEADE` (`cfg.walk_words` 0; on the board 672..928, 257 words). The snapshot is that whole walked list, END included, and every restore compares all of it. With no END in the walk the library keeps a window (`window=noend`) and the placement refuses; a walk that meets NEXT 0 or runs past the memory is refused (`FW_WALK`), and then nothing is written. FNV-1a hash of the words `[HEADE, HEADE + fw_len)`, leaving out the declared hardware-written words | `S5 dlswap snapshot ok\|FAIL ... entries .., walk stopped at word ..`, `S5 hw words '<knob>': <set>`, `u3 snapshot: list-ptr=0x.. heade=<n> words=<fw_len> hash=0x.. hwskip=<set>` (`words=0 ... FAIL` when refused) |
| S6 | the plan's placement, read only (`swap_check.h`'s `swap_place_after_end`; the library's `hvs6s_place_io` wants all-zero words and is not used): one span of 20 words, slot A (10 words) and slot B (the next 10), starting at the word after the END the walk found, never below word 32 (word 0 is a bare END that channel 1 runs), stepping past the other channels' lists (each walked to its END by `hvs6s_snapshot`; one not walked to an END refuses) and their `LPTRS.HEADE` and `DL.LACT`, within `mem_words` and the 12-bit `HEADE`, and checked once more by `hvs6s_check_span`; then refused if any channel's `LPTRS.HEADE` or `DL.LACT` is inside it. No END: refused. No word value decides it (neither zeros, below `HEADE` or anywhere, nor the fill). The 20 words are then read and saved. A refusal writes nothing | `rpi5_scanout32: S6 ch<n> LPTRS ...`, `S6 ch<n> heade .. lact .. list END-walked A..B ...` per other channel, `u3 walk: heade=<n> end=<n>\|none words=<n> entries=<n> invalid=<n> mem=<n> len=20 place=<n>\|refused (<reason>)`, `S6 placement check ok\|FAIL\|skip ...`, `S6 saved words A..B: 0x.. ...`, `u3 dlist: placed word <n> len 20 ...` or `u3 dlist: refused <reason>` |
| S7 | buffers A and B (`.bss`, 1920x1080 or the firmware plane's size if smaller, pitch `w*4`): A the `pattern.h` bars as XRGB8888, B the same bars with red and blue exchanged; cleaned with `sys_cache_data_flush_range` | `u3 plane: addr=0x.. fmt=XRGB8888 pitch=.. w=.. h=.. addr_b=0x..` |
| S8 | 8 swaps, A, B, A, B ... Swap 1 is `run_swap1` (`hvs6s_run`'s sequence with the probe's placement): its own snapshot (compared with the plan's), the placement computed again and still the plan's, the pair's words still as S6 saved them, the swap to A (`hvs6s_swap`) and its restore (`hvs6s_restore`), with its `dlswap` record. Slot B is then checked the same way (placement unchanged, slot B's words as saved), and swaps 2..8 go through `hvs6s_swap` (only when swap 1 wrote `LPTRS`, its `DL.LACT` picked our list up, its restore brought `DL.LACT` back to the firmware's head and was byte-equal; and only up to the first failed swap). `hvs6s_swap` writes the slot's list (`hvs6_dlist_write.h`, NEXT=9, inner and list END, CTL2 0xfff0, alpha-mask FIXED), reads it back, waits by reads only for an `FRCNT` step of channel 0's `STATUS` (`cfg.write_at` `HVS6S_AT_VBLANK`; no step within the read bound: `LPTRS` is not written and the swaps stop), then writes `LPTRS` once and times the `DL.LACT` pickup with a tight loop (see "Vblank landing"); waits for the read-back, an `FRCNT` advance and the `DL.LACT` pickup (within `MAX_POLLS` polls and `LACT_FRAMES` = 4 frames past the write), then two frames past the write before the other slot is touched. Each swap is judged by `swap_check.h`'s `swap_judge`, its sample filled with the firmware's span (`fw_start` = `HEADE`, `fw_words` = the walked length) and ours (`our_words` = 10); the sequence by `swap_seq_judge_lact` (every swap picked up, the plane entry read back from the slot must name the step's buffer). `u3 named: yes` needs `LPTRS` on slot A and swap 1's pickup (`swap_named`). Each buffer stays up 1 s | `dlswap at-write/after ...`, `dlswap swap<k> ...`, `dlswap vblank ...` and `dlswap lact-timing ...` (swap 1's in the record, the others after their `dlswap after` line), `dlswap lact-pickup ok\|FAIL polls=N frcnt=A..B want=W lact=L` (the k-th is swap k's: swap 1's in the record, the others after their `dlswap swap<k>` line), `u3 swap <k>: buf A\|B list-ptr=0x.. frcnt <before> -> <after> judge=... fw=<heade>+<words> ours=10`, `u3 vblank <k>: landed=yes\|no why=<reason> frcnt <w> -> <p> steps=<n> ...` after each `u3 swap <k>`, `rpi5_scanout32: S8 vblank ok\|FAIL ...` after the sequence, `u3 named: yes\|no ... DL.LACT pickup ok\|FAIL (want W lact L)` after swap 1; `dlswap readback-diff ...` for a slot word that did not read back as written |
| S9 | a 16-pixel grid of each buffer read back after a clean+invalidate (`pattern_check`) | `rpi5_scanout32: S9 readback ...` |
| S10 | `hvs6s_restore` after swaps 2..8: the firmware's `LPTRS` written back, two frames and `DL.LACT` back on the firmware's `HEADE`; then `LPTRS` and the firmware's words `[HEADE, HEADE + fw_len)` compared with the snapshot through `swap_check.h`'s `swap_restore_judge`; our 20 words written back to the words S6 saved (and read back equal) only once the restore's `DL.LACT` is back, the frames passed and channel 0's `LPTRS` and `DL.LACT` are off them (`swap_clear_ok`). When nothing more was written, only the compare runs, and the restore's `DL.LACT` is swap 1's own (`runrep.restore_lact_ok`, `swap_restore_lact`). `u3 restore: PASS` needs that `DL.LACT` back and every swap made picked up (`swap_restore_pass`); a restore after a swap the HVS never took is `FAIL DL.LACT did not reach the new head` | `dlswap restore-lact ok\|FAIL polls=N frcnt=A..B want=W lact=L` (the final restore's, after `dlswap restore-frcnt`), `dlswap restore-diff ...` per differing word and `dlswap restore-diffs total=..`, `rpi5_scanout32: S10 ...`, `u3 restore: PASS byte-equal [except hw words off <offs> (..)] list-ptr=0x.. heade=<n> words=<n> hash=0x.. hwskip=<set>` or `u3 restore: FAIL ... hwskip=<set> undeclared=<n>` |
| | the `hvs6_dlist_swap.h` record (`hvs6s_lines`): after `run_swap1` in S8, or of the plan when nothing ran | `dlswap snapshot ok\|FAIL ... entries=N end=W\|stop=W word=0x..`, `dlswap place ok ... skipped=N\|FAIL ...`, swap 1's `dlswap lact-pickup ...` and its restore's `dlswap restore-lact ...`, `dlswap restore-diffs ...`, `dlswap restore-words ok fw A..B equal` (`... byte-equal except hw words off <offs> (<n> differed) hwskip=<set>` with a set), `dlswap result ok\|FAIL ...` |
| | the end line, then the watchdog is no longer fed | `rpi5_scanout32: PASS (...)` or `rpi5_scanout32: FAIL (reason)`, then `rpi5_scanout32: feeding stopped, reset expected ...` |

The restore runs whenever anything was written, whatever failed after it.
When nothing could be placed (the walk found no END, no room after it, or
another channel's list was not walked to an END) the probe prints `u3 walk:
... place=refused (<reason>)` and `u3 dlist: refused <reason>`, and the swap lines
read `u3 swap <k>: skipped (...)`; `verdicts.sh` then gives `FAIL (placement
refused: ...)` and Q4 `unanswered`. Swaps skipped after swap 1 (slot B
refused, or swap 1's restore not byte-equal) are not counted by
`verdicts.sh`: with fewer than 4 swap lines the log is `incomplete`. A stop
at a failed `DL.LACT` pickup (the last swap line's `lact-pickup FAIL`), or
after swap 1 when its own restore's `restore-lact` is `FAIL`, is judged
instead: `FAIL`, Q4 `no`.

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

## Vblank landing

Every swap writes `LPTRS` just after a vblank found by reads, and the
pickup is timed (`hvs6_dlist_swap.h`, swap-vblank-lib): `STATUS` of channel
0 is read in a tight loop until `FRCNT` (bits 21:16) steps, the step taken
as the start of the vertical blank, and `LPTRS` is written right after it.
No pixel valve, HDMI or PHY register is read or written for this, and
`pv_irq.h`'s `pv_wait_vblank` (it writes `PV_INTSTAT`) is not used. After
the write a tight loop reads `DL` and `STATUS` until `DL.LACT` names the new
head, with the cycle counter as the clock (`io.now_ns`). Each swap prints

```text
u3 vblank <k>: landed=yes|no why=<reason> frcnt <w> -> <p> steps=<n> invblank=yes|no tight=ok|miss pickup_reads=<n> pickup_us=<us|none> read_us=<us|none> boundary=yes|no since_us=<us|none> yline <a> -> <b> (yline record only)
```

`w` and `p` are `FRCNT` at the write and at the pickup, `steps` the 6-bit
difference, `pickup_us` the pickup latency, `read_us` the measured time of
one register read in the tight loop, `since_us` the time from the step's
read to the write. `swap_check.h`'s `swap_vblank_judge` says a swap landed
when the vblank wait found its step, the write was in the vblank
(`invblank=yes`: right after the step and within the 667 us window), the
pickup came inside the tight loop (`tight=ok`), and `FRCNT` stepped at most
once from the write to the pickup and the pickup came within 40 ms of the
write (two frames at 50 Hz; this also catches a 6-bit count that aliased).
`why=` names the first condition that failed: `no-vblank`,
`write-not-in-vblank`, `pickup-timeout`, `missed-frame` (`not-tried` for a
swap made without the wait). `boundary=` (the library's frame-boundary
flag, which rests on the assumed 667 us vblank) and `yline` are printed as
a record only and decide nothing; YLINE's range and direction on the board
are unproven.

`verdicts.sh` pairs the `u3 vblank <k>:` line with `u3 swap <k>` by its
number and judges it from its fields: `invblank=yes`, `tight=ok`, at most one
step (`(p - w) & 63 <= 1`) and `landed=yes`. `q4: yes` and the PASS verdict
need every swap landed; otherwise `q4: no` names the first swap that did not:

- `swap <k>: missed a frame (frcnt <w> -> <p>, <n> steps from the write to the pickup)`,
- `swap <k>: DL.LACT pickup timeout (the tight loop ran out, tight=miss)`,
- `swap <k>: written outside the vblank (invblank=no)`,
- `swap <k>: did not land in vblank (why=<reason>)` (the probe's own `landed=no`),
- `swap <k>: no vblank found (no FRCNT step (vblank) within the read bound), LPTRS not written`
  (the `dlswap swap<k> FAIL` line of `HVS6S_ERR_VBLANK`; the probe stops
  there and the log is judged, not incomplete),
- `swap <k>: no u3 vblank line, vblank landing not shown` (an older probe).

A line claiming `landed=yes` over fields that fail is refused too. The
library's `dlswap vblank ...` and `dlswap lact-timing ...` records are shown
as `[v]` lines and not judged. The probe's own end line is `FAIL (swap <k>
did not land in vblank (<reason>))` likewise.

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
(the verdict script, 229 checks), `sh tools/probes/rpi_5/rpi5_scanout32/swap_check_selftest.sh`
(the decision header `src/swap_check.h`, the placement after the END on
`rpi5_dlmap`'s layout included, 215 checks),
`sh tools/probes/rpi_5/rpi5_scanout32/io_filter_selftest.sh` (the write
filter, 56 checks) and `sh tools/v3d/check71.d/hvs6_dlist_swap.sh`
(`hvs6_dlist_swap.h` against its channel model, 328 checks).

## What it proves

`verdicts.sh` PASS means, from the board's registers and the bytes read back
(no camera):

- the firmware's list was walked to its END (`u3 walk: end=<n>`), the
  snapshot is that whole list (`words` = END - `HEADE` + 1), and our list was
  placed after the END (never in `[0, END]`), at or after word 32, inside the
  list memory, where the walk line says, clear of every channel's list
  pointer;
- channel 0's `LPTRS` read back as our list after the first swap and its
  `DL.LACT` reached our list's head, and every swap, alternating A and B at
  least 4 times, saw `FRCNT` advance (the channel kept scanning across the
  pointer write) and its own `dlswap lact-pickup ok` (the HVS took the new
  list: Linux completes a flip only then, `rpi:vc4_crtc.c:832-855`);
- every swap was written just after an `FRCNT` step (in the vblank), and
  `DL.LACT` reached the new head inside the tight loop at most one `FRCNT`
  step after the write (`u3 vblank <k>: landed=yes`);
- the firmware's `LPTRS` and its words `[HEADE, HEADE + fw_len)` read back
  byte-equal to the snapshot (hash equal) after the restore, except the
  declared hardware-written words when a set is given (`hwskip=`, every
  differing word named), every `dlswap restore-lact` line is `ok` (`DL.LACT`
  back on the firmware's head), and our words were back to the words saved
  before the first write. Words 0..31 are never written (placement refuses
  them, and the io filter refuses any write there) and no longer compared.

`q4: yes` needs every swap's pickup; a swap without a `lact-pickup` line (an
older probe, which printed `lact-follows ... (record only)`) or with a `FAIL`
one makes it `no`, and the restore PASS is then refused too. Any
`dlswap swap<k> FAIL <error name> ...` line also makes `q4: no`, naming that
swap, whatever the FRCNT, pickup and vblank lines say (a read-back or a
restore word that failed; since 9ba3e95f). The two error
names of `hvs6_dlist_swap.h`, `DL.LACT did not reach the new head` and
`restore: DL.LACT not back on the firmware's head`, are named as the cause
of a `u3 restore: FAIL`.

It answers Q4 of `docs/notes/rpi5-display.md`: whether channel 0's list
pointer can be swapped at vblank under a running scanout. It does not prove
that the HVS fetched our buffer's pixels: there is no latch bit (linuxdesk),
`DL.LACT` shows only that the channel runs our list, "in the vblank" rests
on taking the `FRCNT` step as the start of the vertical blank (no source
states it), and the farm has no camera on the Pi 5. The plane's UPM bits that Linux sets in PTR0 are not
written (`hvs6_dlist_write.h`); whether a raw RGB plane needs them is a board
question this probe can only bound.

## On the board

### 2026-11-27: 8 vblank swaps landed, firmware list restored (PASS, q4: yes)

This is the current result. It replaces the unanswered status of the 2026-11-26 run
below.

One tryboot session on the Pi 5 (BCM2712 D0), with the image of `c0b4f33b`
(the integration tip, with s32-nullfix's 16 KiB main stack; it printed
`rpi5_scanout32: build c0b4f33b`; Zephyr `af17c0c8aa2a`; sha256
`0ff2fc09860e`, 90400 bytes), built with `-DYAKOGL_WERROR=ON` and the default
`RPI5_SCANOUT32_HW_WORDS="4:skip"`, no warnings. `tools/board.sh pi5 run`
with `BOARD_END_RE='^rpi5_scanout32: (PASS|FAIL)'` and the name argument
`rpi5_scanout32`:

```
board.sh pi5 rpi5_scanout32 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:rpi5_scanout32_pass resets=0 record=absent back_after=183s
```

The image printed its end line about 4 s after Zephyr started. The PM
watchdog then reset the SoC, and tryboot returned the board to Linux by
itself (a new `boot_id`). `meta.txt` has `end_seen=1`, `back=1`,
`throttled_before=0x0`, `throttled_after=0x0` and `cleanup=staged image
removed, tryboot.txt put back (read back)`. The card had no `tryboot.txt`
before the run and none after it. The sha256 of every file under
`/boot/firmware` (435 files) was the same before and after the run. No retry
was made.

The walk, the placement and the saved words are as `rpi5_dlmap` predicted:

```
rpi5_scanout32: S2 ch0 CTRL0 0x877f0437 LPTRS 0x000002a0 STATUS 0x00814198 DL 0x000002a0
rpi5_scanout32: S2 ch0 FRCNT series 2 4 5 6 7 8 (6 reads 20 ms apart): 6 frames, advancing
rpi5_scanout32: S3 list read from word 672: 4096 words; END at word 928 (list index 256)
rpi5_scanout32: S3 dl[928] = 0x80000000
rpi5_scanout32: S5 dlswap snapshot ok (ok) mem_words 4096 (CXM_SIZE 0x00001000) lptrs 0x000002a0 fw_head 672 fw_len 257 entries 8, walk stopped at word 928 (0x80000000), END found
u3 snapshot: list-ptr=0x000002a0 heade=672 words=257 hash=0x7fdcbefd hwskip=4:skip
u3 walk: heade=672 end=928 words=257 entries=8 invalid=7 mem=4096 len=20 place=929
rpi5_scanout32: S6 saved words 929..948: 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0
u3 dlist: placed word 929 len 20 (slot A 929, slot B 939)
u3 plane: addr=0x0000a00000 fmt=XRGB8888 pitch=7680 w=1920 h=1080 addr_b=0x0000217000 pattern_a=bars pattern_b=bars-red-blue-exchanged
```

All 8 swaps alternated A and B. Each swap was written just after an `FRCNT`
step, `DL.LACT` reached the new head within that frame, and `FRCNT` advanced
across every write:

```
dlswap lact-pickup ok polls=0 frcnt=14..15 want=929 lact=929
dlswap swap1 ok ok lptrs-readback ok frcnt-advance ok settle ok
u3 swap 1: buf A list-ptr=0x000003a1 frcnt 14 -> 15 judge=pointer-named-ours slot=929 plane=0x0000a00000 fw=672+257 ours=10
u3 vblank 1: landed=yes why=landed frcnt 14 -> 15 steps=1 invblank=yes tight=ok pickup_reads=127288 pickup_us=16666.426 read_us=0.065 boundary=yes since_us=0.037 yline 0 -> 0 (yline record only)
u3 named: yes LPTRS 0x000003a1 after swap 1, our list at word 929, first read 0x000003a1 poll 0, DL.LACT pickup ok (want 929 lact 929)
dlswap swap2 ok ok lptrs-readback ok frcnt-advance ok settle ok
dlswap lact-pickup ok polls=0 frcnt=28..29 want=939 lact=939
u3 swap 2: buf B list-ptr=0x000003ab frcnt 28 -> 29 judge=pointer-named-ours slot=939 plane=0x0000217000 fw=672+257 ours=10
u3 vblank 2: landed=yes why=landed frcnt 28 -> 29 steps=1 invblank=yes tight=ok pickup_reads=127355 pickup_us=16666.629 read_us=0.065 boundary=yes since_us=0.037 yline 0 -> 0 (yline record only)
u3 swap 3: buf A list-ptr=0x000003a1 frcnt 32 -> 33 judge=pointer-named-ours slot=929 plane=0x0000a00000 fw=672+257 ours=10
u3 vblank 3: landed=yes why=landed frcnt 32 -> 33 steps=1 invblank=yes tight=ok pickup_reads=127358 pickup_us=16666.407 read_us=0.065 boundary=yes since_us=0.037 yline 0 -> 0 (yline record only)
u3 swap 4: buf B list-ptr=0x000003ab frcnt 36 -> 37 judge=pointer-named-ours slot=939 plane=0x0000217000 fw=672+257 ours=10
u3 vblank 4: landed=yes why=landed frcnt 36 -> 37 steps=1 invblank=yes tight=ok pickup_reads=127269 pickup_us=16666.611 read_us=0.065 boundary=yes since_us=0.019 yline 0 -> 0 (yline record only)
u3 swap 5: buf A list-ptr=0x000003a1 frcnt 40 -> 41 judge=pointer-named-ours slot=929 plane=0x0000a00000 fw=672+257 ours=10
u3 vblank 5: landed=yes why=landed frcnt 40 -> 41 steps=1 invblank=yes tight=ok pickup_reads=127416 pickup_us=16666.463 read_us=0.065 boundary=yes since_us=0.037 yline 0 -> 0 (yline record only)
u3 swap 6: buf B list-ptr=0x000003ab frcnt 44 -> 45 judge=pointer-named-ours slot=939 plane=0x0000217000 fw=672+257 ours=10
u3 vblank 6: landed=yes why=landed frcnt 44 -> 45 steps=1 invblank=yes tight=ok pickup_reads=127354 pickup_us=16666.629 read_us=0.065 boundary=yes since_us=0.037 yline 0 -> 0 (yline record only)
u3 swap 7: buf A list-ptr=0x000003a1 frcnt 48 -> 49 judge=pointer-named-ours slot=929 plane=0x0000a00000 fw=672+257 ours=10
u3 vblank 7: landed=yes why=landed frcnt 48 -> 49 steps=1 invblank=yes tight=ok pickup_reads=127358 pickup_us=16666.611 read_us=0.065 boundary=yes since_us=0.037 yline 0 -> 0 (yline record only)
dlswap swap8 ok ok lptrs-readback ok frcnt-advance ok settle ok
dlswap lact-pickup ok polls=0 frcnt=52..53 want=939 lact=939
u3 swap 8: buf B list-ptr=0x000003ab frcnt 52 -> 53 judge=pointer-named-ours slot=939 plane=0x0000217000 fw=672+257 ours=10
u3 vblank 8: landed=yes why=landed frcnt 52 -> 53 steps=1 invblank=yes tight=ok pickup_reads=127359 pickup_us=16666.666 read_us=0.065 boundary=yes since_us=0.037 yline 0 -> 0 (yline record only)
rpi5_scanout32: S8 sequence ok (pass) index -1 swap pointer-named-ours, 8 of 8 swaps ok
rpi5_scanout32: S8 vblank ok: every swap written in the vblank and picked up within one frame step
rpi5_scanout32: S9 readback A ok 0 mismatches of 8160 (invd 0, sum 0xc1c58f8e), B ok 0 mismatches of 8160 (invd 0, sum 0xa9ac55b6)
dlswap restore-after frcnt=59 mode=2 lptrs=0x000002a0 lact=672 bgenb=1 status=0x00bb4008
dlswap restore-lact ok polls=0 frcnt=57..58 want=672 lact=672
dlswap restore-diff word 676 entry 0 off 4 was 0x80000188 now 0x8000037c hw=skip
dlswap restore-diffs total=1 undeclared=0 skip=1 expect=0 shown=1 hwskip=4:skip
rpi5_scanout32: S10 our words 929..948 back to the words saved at S6
rpi5_scanout32: S10 dlswap restore ok (ok); fw 672..+257 byte-equal index -1 want 0x00000000 got 0x00000000
u3 restore: PASS byte-equal except hw words off 4 (1 differed: 1 skip, 0 expect) list-ptr=0x000002a0 heade=672 words=257 hash=0x7fdcbefd hwskip=4:skip
rpi5_scanout32: S11 main stack size 16384 unused 12108 (ok, need 4096 unused)
rpi5_scanout32: PASS (8 swaps alternating A/B, list at word 929, each written in the vblank and picked up within one frame step, DL.LACT picked up each and came back, FRCNT advanced, buffers read back, firmware list restored byte-equal except hw words off 4)
```

(Swaps 3 to 7 each printed the same `dlswap swap<k> ok ok lptrs-readback ok
frcnt-advance ok settle ok` and `dlswap lact-pickup ok polls=0` lines, with
`want` alternating 929 and 939.) In our own list the hardware wrote back only
offset 4 of the plane entry: word 933 (slot A) or 943 (slot B), 0xc0c0c0c0
before and 0x800003c2 after (swap 1: 0x80000437), all `hw=skip`. In the
firmware's list it wrote back only word 676 (entry 0, offset 4). So the Q2
word set `4:skip` holds for our list as well as for the firmware's. The main
stack used 4276 of its 16384 bytes (`unused 12108`). That is more than the
earlier 4 KiB stack had, which agrees with s32-nullfix's diagnosis.

`verdicts.sh` on the console with its `HH:MM:SS ` prefix stripped exited 0
(`8 of 8 expected verdict lines ... (8 swap line(s))`, `missing: none`):

```
q4: yes (channel 0's list pointer swapped 8 times under the running scanout, named after the first swap, DL.LACT picked up every swap, FRCNT advanced through every swap, every swap written in the vblank and picked up within one frame step)
verdicts: PASS (8 swaps alternating, our list named at word 929 after the firmware list's END at word 928 (heade 672), DL.LACT picked up each swap and came back, each swap landed in vblank within one frame step, FRCNT advanced, firmware list restored byte-equal except hw words off 4 (hwskip=4:skip))
```

Reading it: Q4 is answered yes from the registers. Channel 0's list pointer
can be swapped at vblank under the running scanout. All 8 swaps landed in
the frame after the write, and the firmware's list and our 20 words came back.
What "What it proves" leaves open is still open: no camera saw the
pixels, and taking the `FRCNT` step as the start of vblank rests on no
source.

The console was captured as in the earlier sections. The farm's console lease
holds the serial bridge, so `BOARD_UART` was a local
`am62p_netboot_uart.py feed` relay of the lease's copy of the same console,
written once the image had reset. The copy left out the boot loader's lines,
the `S3 dl[...]` lines other than dl[672..680], dl[704], every 32nd word
after it, dl[927] and dl[928], and the next boot's loader lines. Every other
line from `board: watchdog armed` through `feeding stopped` is as the board
printed it. The run directory `20261127-1017-rpi5_scanout32-0ff2fc09` is
under the gitignored `records/` and is not committed.

### 2026-11-25: placement refused, nothing written (Q4 unanswered)

One tryboot session on the Pi 5 (BCM2712 D0), with the image of `458834ea`
(origin/main; it printed `rpi5_scanout32: build 458834ea`; Zephyr
`af17c0c8aa2a`; sha256 `92671f499ad3`, 86304 bytes), built with
`-DYAKOGL_WERROR=ON` and the default `RPI5_SCANOUT32_HW_WORDS="4:skip"`,
no warnings. `tools/board.sh pi5 run` with
`BOARD_END_RE='^rpi5_scanout32: (PASS|FAIL)'` and the name argument
`rpi5_scanout32`:

```
board.sh pi5 rpi5_scanout32 rc=4 verdict=failed boot=tryboot row boot_verdict=complete result=legacy:rpi5_scanout32_fail resets=0 record=absent back_after=171s
```

The board came back to Linux by itself through the PM watchdog and tryboot,
171 s after the reboot (`meta.txt`: `cleanup=staged image removed,
tryboot.txt put back (read back)`, `throttled_before=0x0`,
`throttled_after=0x0`). The card had no `tryboot.txt` before the run and
none after it. The sha256 of every file under `/boot/firmware` (435 files)
was the same before and after the run.

The probe refused to place its list, so it wrote no list word and no
`LPTRS`, and it made no swap. The firmware's list at `HEADE` 672 has no END
within the 256-word window. The walk went through 8 entries and stopped at
word 896. With no END the library places our list only wholly before
`HEADE` (`hvs6s_place_io`, `fw_noend`). Words 32..671 held 619 non-zero
words, so no 20-word zero span was found there:

```
rpi5_scanout32: S2 ch0 CTRL0 0x877f0437 LPTRS 0x000002a0 STATUS 0x00814091 DL 0x000002a0
rpi5_scanout32: S2 ch0 FRCNT series 2 3 4 6 7 8 (6 reads 20 ms apart): 6 frames, advancing
rpi5_scanout32: S3 list read from word 672: 256 words; END NOT found within them (entry walk stopped at list index 256, word 928, past the 256 words read)
rpi5_scanout32: S5 dlswap snapshot ok (ok) mem_words 4096 (CXM_SIZE 0x00001000) lptrs 0x000002a0 fw_head 672 fw_len 256 entries 8, walk stopped at word 896 (0x20000000), END NOT found: fixed window kept, our list before HEADE
u3 snapshot: list-ptr=0x000002a0 heade=672 words=256 hash=0xc3fc4aad window=noend hwskip=4:skip
rpi5_scanout32: S6 placement check FAIL: place: no END, no room wholly before HEADE (619 non-zero words stepped past); nothing is written
u3 dlist: refused place: no END, no room wholly before HEADE (619 non-zero words stepped past)
u3 swap 1: skipped (not placed)
u3 named: no (no swap was made)
rpi5_scanout32: S10 restore write skip: nothing was written; read-only compare
dlswap restore-diff word 676 entry 0 off 4 was 0x8000001e now 0x800003e3 hw=skip
dlswap restore-diffs total=1 undeclared=0 skip=1 expect=0 shown=1 hwskip=4:skip
u3 restore: PASS byte-equal except hw words off 4 (1 differed: 1 skip, 0 expect) list-ptr=0x000002a0 heade=672 words=256 hash=0xc3fc4aad hwskip=4:skip (nothing was written)
dlswap result FAIL place: no END, no room wholly before HEADE
rpi5_scanout32: FAIL (our list was refused: its words are not known free)
```

(`u3 swap 2` .. `u3 swap 8` read the same as swap 1.) This run printed no
`u3 vblank`, `dlswap lact-pickup`, `dlswap restore-lact` or
`dlswap restore-vblank` line. `verdicts.sh` on the console with its
`HH:MM:SS ` prefix stripped exited 3:

```
q4: unanswered (placement refused)
verdicts: FAIL (placement refused: place: no END, no room wholly before HEADE (619 non-zero words stepped past))
```

Reading it: this run does not answer Q4. The only write-free result is that
the read-only compare of the firmware's 256 words passed. Only word 676
(entry 0, offset 4, the declared `4:skip` word) differed between the snapshot
and the compare, so the Q2 word set holds. The probe needs room for its list
that it can prove is free on this firmware before it can make a swap. Either
the walk finds the list's END, or there is a zero span before `HEADE`. That
is a change to the probe or the library, not a board retry.

The console was captured as in `rpi5_dispregs`. The farm's console lease
holds the serial bridge, so `BOARD_UART` was a local
`am62p_netboot_uart.py feed` relay of the lease's copy of the same console.
The copy left out most of the boot loader's lines and the `S3 dl[...]` list
lines, and kept dl[672..680], 703..705 and 735. Every other `rpi5_scanout32:`,
`u3` and `dlswap` line is as the board printed it. The run directory
`20261125-1059-rpi5_scanout32-92671f49` is under the gitignored `records/`
and is not committed.

### 2026-11-26: the whole-memory walk placed the list; swap 1 landed in vblank, then the image faulted (Q4 unanswered)

One tryboot session on the Pi 5 (BCM2712 D0), with the image of `77ddb606`
(origin/main; it printed `rpi5_scanout32: build 77ddb606`; Zephyr
`af17c0c8aa2a`; sha256 `dabb0e40cb5a`, 86304 bytes), built with
`-DYAKOGL_WERROR=ON` and the default `RPI5_SCANOUT32_HW_WORDS="4:skip"`.
`tools/board.sh pi5 run` with `BOARD_END_RE='^rpi5_scanout32: (PASS|FAIL)'`
and the name argument `rpi5_scanout32`:

```
board.sh pi5 rpi5_scanout32 rc=4 verdict=no_end_line boot=tryboot row boot_verdict=no_end_line result=none resets=0 record=absent back_after=422s
```

The probe printed no `rpi5_scanout32: PASS` or `FAIL` line: the image took a
CPU exception after swap 1 and Zephyr halted (see below). `board.sh` waited its
420 s bound for an end line. The board came back to Linux by itself through
the PM watchdog and tryboot: the kernel's uptime put its boot about 70 s after
the tryboot reboot, and `meta.txt` has `back=1`, `throttled_before=0x0`,
`throttled_after=0x0`, `cleanup=staged image removed, tryboot.txt put back
(read back)`; no `yakogl*` or `tryboot*` file was left under `/boot/firmware`
afterwards. No retry was made.

The walk and the placement worked as `rpi5_dlmap` predicted. The whole-memory
walk found the END at word 928, the list went right after it, the 20 words it
covers were saved (all `0xb0b0b0b0`), and swap 1 landed in vblank:

```
rpi5_scanout32: S2 ch0 CTRL0 0x877f0437 LPTRS 0x000002a0 STATUS 0x0081418e DL 0x000002a0
rpi5_scanout32: S2 ch0 FRCNT series 2 4 5 6 7 8 (6 reads 20 ms apart): 6 frames, advancing
rpi5_scanout32: S3 list read from word 672: 4096 words; END at word 928 (list index 256)
rpi5_scanout32: S3 dl[928] = 0x80000000
rpi5_scanout32: S5 dlswap snapshot ok (ok) mem_words 4096 (CXM_SIZE 0x00001000) lptrs 0x000002a0 fw_head 672 fw_len 257 entries 8, walk stopped at word 928 (0x80000000), END found
u3 walk: heade=672 end=928 words=257 entries=8 invalid=7 mem=4096 len=20 place=929
rpi5_scanout32: S6 saved words 929..948: 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0 0xb0b0b0b0
u3 dlist: placed word 929 len 20 (slot A 929, slot B 939)
dlswap lact-pickup ok polls=0 frcnt=14..15 want=929 lact=929
dlswap restore-lact ok polls=0 frcnt=17..18 want=672 lact=672
dlswap restore-vblank in-vblank=yes found=yes reads=251577 frcnt=17..18 yline=0..0 since_us=0.019 tight=ok step=126459 dt_us=16666.666 read_us=0.065 boundary=yes
u3 swap 1: buf A list-ptr=0x000003a1 frcnt 14 -> 15 judge=pointer-named-ours slot=929 plane=0x00009ff000 fw=672+257 ours=10
u3 vblank 1: landed=yes why=landed frcnt 14 -> 15 steps=1 invblank=yes tight=ok pickup_reads=127834 pickup_us=16666.629 read_us=0.065 boundary=yes since_us=0.038 yline 0 -> 0 (yline record only)
u3 named: yes LPTRS 0x000003a1 after swap 1, our list at word 929, first read 0x000003a1 poll 0, DL.LACT pickup ok (want 929 lact 929)
[00:00:01.616,000] <err> os: ELR_ELn: 0x0000000000000020
[00:00:01.628,000] <err> os:   EC:  0x20 (Instruction Abort from a lower Exception level, that might be using AArch32 or AArch64)
[00:00:01.649,000] <err> os: FAR_ELn: 0x0000000000000020
[00:00:01.762,000] <err> os: x18: 0x00000000011f8310  lr:  0x000000000020bd18
[00:00:01.770,000] <err> os: >>> ZEPHYR FATAL ERROR 0: CPU exception on CPU 0
[00:00:01.784,000] <err> os: Halting system
```

Swap 1's own restore also read right: `dlswap restore-diff word 676 entry 0 off
4 was 0x8000017c now 0x80000007 hw=skip` is the one declared `4:skip` word,
and `dlswap restore-words ok fw 672..928 byte-equal except hw words off 4 (1
differed)`. After `u3 named: yes` the image printed no `u3 swap 2`, no
`u3 vblank 2`, no `u3 restore` and no end line. The fault is an instruction
fetch from address 0x20 (a jump through a null pointer; `lr` 0x20bd18) at
1.6 s of Zephyr time, between swap 1 and swap 2. The cause was found later
(`c0b4f33b`). The 4 KiB main stack overflowed into the idle stack. The
2026-11-27 run above used the 16 KiB stack.

`verdicts.sh` on the console with its `HH:MM:SS ` prefix stripped exited 1:

```
q4: unanswered (incomplete log)
verdicts: incomplete (the run did not print all expected verdict lines)
```

(its other lines: `5 of 8 expected verdict lines ... (1 swap line(s))`,
`missing: u3 swap (need 4, saw 1) u3 restore rpi5_scanout32 terminal`, and
`after the last seen line [6] u3 named: ...: the image stopped there`.)

Reading it: this run does not answer Q4. One swap of the vblank pointer
landed (`u3 vblank 1: landed=yes`, `lact-pickup ok`, `restore-lact ok`), but
the 8-swap loop, the alternation of the two buffers and `u3 restore: PASS`
were not reached, so `q4: yes` is not given. `u3 named: yes` shows only
swap 1; the restore compare in swap 1's own library run is not the probe's
`u3 restore`. Superseded by the 2026-11-27 run above.

The console was captured as in the 2026-11-25 section: the farm's console
lease holds the serial bridge, so `BOARD_UART` was a local
`am62p_netboot_uart.py feed` relay of the lease's copy, appended in place
after the run had started. The copy left out the boot loader's lines, the
`S3 dl[...]` lines other than dl[672..680], every 32nd list word and
dl[927..928], and the fault's register dump except the lines quoted above
(colour codes removed). Every other `rpi5_scanout32:`, `u3` and `dlswap` line
is as the board printed it. The run directory
`20261126-1518-rpi5_scanout32-dabb0e40` is under the gitignored `records/`
rule's equivalent (outside the tree) and is not committed.
