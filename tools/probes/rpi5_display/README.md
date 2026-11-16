<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_display

U3 on the Raspberry Pi 5 (`driver/docs/rpi5-display.md`): does a scanout run
when a `kernel=` image starts, and does the property mailbox hand out a
framebuffer. Not run on a board yet. It lives here, not under `rpi_5/`, until
#321 releases that tree.

The image is the design note's section 6, step by step, and it reads: it writes
no scanout, pixel valve or HDMI register. Each step's decision is one line, and
`verdicts.sh` reads those lines.

| Step | What the image does | Line |
|---|---|---|
| Z0 | `get_throttled`; the five firmware clocks (core 4, HVS disp 16, HDMI HSM 13, HDMI bvb 14, V3D 5); `iommu4`'s five words; then the mappings the blocks need (the HVS only when the disp clock is not 0, the `hd` region only when an answer says the HSM clock runs) | — |
| Z0 | the whole snapshot table, one `u3 reg` line per register | `u3 reg …` |
| Z1 | channels 0..2 sampled twice, 100 ms apart, from the same captured words, each channel judged once with each mapped pixel valve's words | `u3 scanout: …` |
| Z1 | each channel that names a list read once, its first plane decoded | `u3 plane: addr=… fmt=… pitch=… w=… h=…` |
| Z1 | the plane's buffer adopted, or the reason it cannot be, then a pattern written into it and read back twice | `u3 adopt-readback: PASS\|FAIL (…)` |
| Z2 | the six framebuffer GET tags, then `rpi_fb_alloc_virtual` 1280x720 physical, 1440 virtual rows, depth 32 | `u3 mailbox-fb: PASS\|FAIL (…)` |
| Z3 | the granted buffer filled and read back twice | `u3 mailbox-readback: PASS\|FAIL (…)` |
| Z3 | every channel's list read again and compared with the buffer, with the `iommu4` pass-through state in front of it | `u3 scanout-follows-mailbox: yes\|no (…)` |
| Z4 | the Pi 4's split screen: fbcon text on top, a pattern band below; the text band counted before the text and after it | `u3 split-screen: …` |

Every step prints its line on every path, including the paths that could not
measure it: `verdicts.sh` reads a log missing one of its lines as no verdict at
all, which says less than a `no` with its reason named.

Two things are read twice because one reading does not name them. A channel is
judged once with each mapped pixel valve's words: BCM2712 names two pixel valves
(`bcm2712.dtsi:292-302`) and the HVS three channels (its `ch0/ch1/ch2-eof`
interrupt names at `:612-623`), and no register read here says which valve a
channel feeds — that link (a DISP1 target field) is what a new issue names. The
text band is counted before the console text is printed and after it:
`fbcon_attach()` clears the rows it was given, and that blank clear alone changes
every pixel of a band that held the pattern, so "no longer the pattern" is not yet
a character drawn. A band flat after the attach and not flat after the text is.

```sh
west build -p always -b rpi_5 -d build/rpi5_display/west yakogl/tools/probes/rpi5_display \
  -- -DYAKOGL_WERROR=ON
```

| File | What |
|---|---|
| `src/main.c` | the image: the Z0..Z4 steps above, over the header-only `driver/zephyr/lib/board/rpi5_scanout` decoders and this probe's own `src/plane_match.h` |
| `src/plane_match.h` | the Z3 decision (does the display list name the image's buffer, address and pitch), the `iommu4` pass-through question in front of it, and which comparison of several lists the one decision line names (`pm_choose`); header-only, so the host test runs the same functions the image runs |
| `src/test/plane_match_test.c`, `src/plane_match_selftest.sh` | its host test (ASan+UBSan): 68 checks, last line `plane_match_test: 68 of 68 checks, 0 failed`, exit 77 with `gap: CC` when there is no C compiler |
| `verdicts.sh`, `verdicts_selftest.sh` | the log reader: the nine lines above, the route it names, and the comparison with `linux_ref.md`'s plane facts |
| `report_texts.sh`, `report_texts_selftest.sh` | the U3 report texts, from the verdict lines |
| `route_choice.sh`, `route_choice_selftest.sh` | the host rule that picks the route the board takes, from the verdict lines (`driver/docs/rpi5-display.md` section 8) |
| `throttled.sh`, `throttled_selftest.sh` | `get_throttled` decoded, from a record |
| `dump2png.py`, `dump2png_selftest.sh` | scanout readback bytes to a PNG |
| `linux_ref.sh`, `linux_ref.md`, `linux_ref_selftest.sh` | the read-only Linux dump to run first, over ssh (C0), its facts file, and the self-test of that script against a fake root |
| `mboxdump/mboxdump.py`, `mboxdump/selftest.sh` | host decoder of recorded property-mailbox buffers (does the BCM2712 answer FRAMEBUFFER_ALLOCATE 0x40001?); the self-test checks it on the fixtures under `mboxdump/fixtures/` |
| `screen_mock.sh` | host test of `driver/zephyr/lib/board/rpi5/rpi5_screen.c` (`test/rpi5_screen_mock.c`) |
| `fwfb/` | the simple-framebuffer DTB node a `kernel=` image receives, with its self-test |
| `boards/rpi_5.overlay` | the sram0 every Pi 5 image uses (32 MiB from 0x200000, below 4 GiB, so the mailbox's message buffer address fits its 32-bit word); Zephyr's own mailbox driver stays off, the board layer's `rpi_mbox.c` owns the mailbox |

The image ends with `rpi5_display: PASS (...)` or `rpi5_display: FAIL (reason)`,
then the `measured:` summary and `rpi5_display: feeding stopped, reset expected`
(its own last claim: the watchdog resets the SoC and `tryboot` returns the board
to Linux). Every register read group is announced first, so a hang names its
block. The PM watchdog (`CONFIG_YAKOGL_RPI5_WDOG`, armed before `main`) is what
brings the board back; the image never asks for power. The Pi 5 rules of
`tools/probes/rpi_5/README.md` apply: run through `tools/board.sh pi5`, record
`get_throttled` with every record (weak supply), and if the board does not come
back stop, release it and report that it needs a person. Records go to
`records/rpi5/<YYYYMMDD-HHMM>-<name>-<sha8>/` (gitignored, redacted).

The gate does not build or test this probe yet: the Zephyr image row globs
`tools/probes/rpi_5/*/`, and no row runs these `*_selftest.sh` files. The host
tests above are the local check until a row exists (see the issue this change's
report opens).
