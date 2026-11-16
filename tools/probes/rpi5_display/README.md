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

Each step starts with `rpi5_display: step <n> <name> at <ms> ms since boot`
(time since boot, so a record shows when each step began and a hang names its
step). The steps are numbered as the U3 item lists them; step 4 comes twice,
because `verdicts.sh`'s order puts the adopt read-back before the mailbox.

| Step | Z | What the image does | Line |
|---|---|---|---|
| — | — | the board layer arms the PM watchdog in its first `PRE_KERNEL_1` hook, before `main` (45 s hold); `main` prints the build and the watchdog line first | `rpi5_display: build <sha>`, `board: PM watchdog resets …` |
| 1 dt-fb | — | the DTB the firmware handed over in x0 (kept by `tools/probes/rpi_5/common/boot`, used as it is), its `simple-framebuffer` node through `fwfb/fw_fb_dt.c`; the node's address and its `/reserved-memory` region are printed on a record line only | `u3 dt-fb: <fmt> <w>x<h> stride <n\|none> status <okay\|disabled> <addr-set\|addr-0>`, `u3 dt-fb: none (…)`, or `u3 dt-fb: skip (<reason>)` |
| 2 scanout | Z0 | `get_throttled`; the five firmware clocks (core 4, HVS disp 16, HDMI HSM 13, HDMI bvb 14, V3D 5); `iommu4`'s five words; then the mappings the blocks need (the HVS only when the disp clock is not 0, the `hd` region only when an answer says the HSM clock runs) | `throttled=0x…`, `rpi5_display: clk …` |
| 2 scanout | Z0 | the whole snapshot table, one `u3 reg` line per register | `u3 reg …` |
| 2 scanout | Z1 | channels 0..2 sampled twice, 100 ms apart, from the same captured words, each channel judged once with each mapped pixel valve's words | `u3 scanout: running\|stopped <reason>` |
| 2 scanout | Z1 | each channel that names a list read once (`u3 dlist` lines), its first plane decoded | `u3 plane: addr=0x… fmt=… pitch=… w=… h=…` (or `u3 plane: none`) |
| 4 adopt-readback | Z1 | the plane's buffer adopted, or the reason it cannot be, then `pattern.h`'s pattern written into it, the cache cleaned, read back twice through the non-cacheable mapping, and the list read again: does it still name that buffer | `u3 adopt-readback: PASS\|FAIL (… mismatches, checksum 0x…, list still names it\|…)` |
| 3 mailbox | Z2 | the six framebuffer GET tags as `fb_query.h` builds them, classified by its parser | `u3 fb-query: present\|absent\|unknown\|malformed (phys … virt … depth … order … pitch … offset …)` or `skip (…)` |
| 3 mailbox | Z2 | `rpi_fb_alloc_virtual` 1280x720 physical, 1440 virtual rows, depth 32, in `fb_query.h`'s tokens | `u3 fb-alloc: granted (<w>x<h> pitch <n> <fmt> size <n>)`, `granted another window (…)`, `granted but not mapped (…)`, `refused or unknown (rc …)`, `skip (…)` |
| 3 mailbox | Z2 | the grant as `verdicts.sh` reads it | `u3 mailbox-fb: PASS (<w> x <h> pitch <n> <fmt>)\|FAIL (<reason>)` |
| 4 mailbox-readback | Z3 | the granted buffer filled, cleaned and read back twice | `u3 mailbox-readback: PASS\|FAIL (… mismatches, checksum 0x…)` |
| 4 mailbox-readback | Z3 | every channel's list read again and compared with the buffer, with the `iommu4` pass-through state in front of it | `u3 scanout-follows-mailbox: yes\|no (…)` |
| 5 split-screen | Z4 | the Pi 4's split screen in the granted buffer, else in the adopted 32 bpp plane when its read-back passed: fbcon text on top, a pattern band below; the text band counted before the text and after it, the pattern band checked | `u3 split-screen: drawn (… checksum 0x…)\|not drawn (…)` |
| 6 summary | — | the image's own claim, the summary and the watchdog ending; then it stops feeding and the watchdog resets the SoC | `rpi5_display: PASS\|FAIL (…)`, `rpi5_display: measured: …`, `rpi5_display: feeding stopped, reset expected (…)` |

Every step prints its line on every path, including the paths that could not
measure it: `verdicts.sh` reads a log missing one of its lines as no verdict at
all, which says less than a `no` with its reason named. So the eight lines
`verdicts.sh` reads (`u3 scanout` to the terminal) never say `skip`: a step
that could not run prints `FAIL`, `stopped` or `no` with its reason. Only the
record lines `verdicts.sh` does not read (`u3 dt-fb`, `u3 fb-query`,
`u3 fb-alloc`) say `skip (<reason>)`. Addresses appear only in the console at
run time; no file holds one.

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

## How the board session (board-u3) runs it

Not run yet. One client at a time, with `target:rpi5` and `target:rpi5-console`
together; the session order is the design note's (the Linux reference first,
then the image):

1. Under Linux, over ssh: `linux_ref.sh` (read-only, `linux_ref.md`), and
   `vcgencmd get_throttled` into the record.
2. The image, through `tools/board.sh pi5 run <zephyr.bin> <logdir>`
   (`docs/standalone-boot.md`, "Raspberry Pi 5"): `board.sh` stages
   `yakogl-<name>.bin` and a one-shot `tryboot.txt`, reboots with
   `reboot "0 tryboot"`, reads the console, waits for Linux to come back and
   restores `tryboot.txt`; `config.txt` and `cmdline.txt` stay as found.
   `tryboot/render.sh fw|bare <name>` prints the `tryboot.txt` for the
   session's two boots (fw: routes A and B, bare: route C). `board.sh pi5`'s
   default end line does not know this probe, so the run sets
   `BOARD_END_RE='^rpi5_display: feeding stopped, reset expected'` (the
   image's last line; everything `verdicts.sh` reads comes before it). The
   image carries the `board: PM watchdog resets` marker `board.sh` checks for.
3. Right after each capture: `verdicts.sh <console.txt> [linux_ref.txt]` (exit
   0: a route is named, 1: incomplete, rerun or stop and report), then
   `route_choice.sh`, `decode/` for the `u3 reg`/`u3 dlist` lines and
   `throttled.sh` for `get_throttled`.

The image stops feeding after its last line and the PM watchdog resets the SoC
within 45 s; tryboot then boots Linux, because `tryboot.txt` is read once. If
Linux does not come back within `board.sh`'s bounds, stop, release the board
and report that it needs a person: there is no remote power. Never install an
EEPROM or persist anything on the board.

| File | What |
|---|---|
| `src/main.c` | the image: the Z0..Z4 steps above, over the header-only `driver/zephyr/lib/board/rpi5_scanout` decoders and this probe's own `src/plane_match.h` |
| `src/plane_match.h` | the Z3 decision (does the display list name the image's buffer, address and pitch), the `iommu4` pass-through question in front of it, and which comparison of several lists the one decision line names (`pm_choose`); header-only, so the host test runs the same functions the image runs |
| `src/test/plane_match_test.c`, `src/plane_match_selftest.sh` | its host test (ASan+UBSan): 68 checks, last line `plane_match_test: 68 of 68 checks, 0 failed`, exit 77 with `gap: CC` when there is no C compiler |
| `verdicts.sh`, `verdicts_selftest.sh` | the log reader: the eight lines from `u3 scanout` to the terminal line (and the optional watchdog ending), the route it names, and the comparison with `linux_ref.md`'s plane facts |
| `report_texts.sh`, `report_texts_selftest.sh` | the U3 report texts, from the verdict lines |
| `route_choice.sh`, `route_choice_selftest.sh` | the host rule that picks the route the board takes, from the verdict lines (`driver/docs/rpi5-display.md` section 8) |
| `throttled.sh`, `throttled_selftest.sh` | `get_throttled` decoded, from a record |
| `dump2png.py`, `dump2png_selftest.sh` | scanout readback bytes to a PNG |
| `linux_ref.sh`, `linux_ref.md`, `linux_ref_selftest.sh` | the read-only Linux dump to run first, over ssh (C0), its facts file, and the self-test of that script against a fake root |
| `mboxdump/mboxdump.py`, `mboxdump/selftest.sh` | host decoder of recorded property-mailbox buffers (does the BCM2712 answer FRAMEBUFFER_ALLOCATE 0x40001?); the self-test checks it on the fixtures under `mboxdump/fixtures/` |
| `screen_mock.sh` | host test of `driver/zephyr/lib/board/rpi5/rpi5_screen.c` (`test/rpi5_screen_mock.c`) |
| `fwfb/` | the simple-framebuffer DTB node a `kernel=` image receives, with its self-test; step 1 compiles `fw_fb_dt.c` with `tools/probes/rpi_5/common/fdt/fdt_walk.c` and `common/boot` (x0), both used as they are |
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
