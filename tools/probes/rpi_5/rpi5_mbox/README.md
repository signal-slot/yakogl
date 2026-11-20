<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_mbox

The Raspberry Pi 5's property mailbox alone, with the raw words: the first
probe of `docs/notes/rpi5-display.md` section 3. It answers section 4 Q1:
why every request of `rpi5_display`'s session of 2026-11-19 came back `-EIO`
with response code 0, `GET_THROTTLED` included
(`driver/docs/rpi5-display.md`, the Z2 paragraph of the session record), while
`rpi5_clear`'s `GET_CLOCK_RATE` of clock 5 was answered from a `kernel=` image
the same day (`rpi5_clear/README.md`: `M1 V3D clock: rate 0 (500000000 Hz)`).
Part of ROADMAP stage 5 (issue #7), step P11 of `docs/notes/pre-study-rpi5.md`.

```sh
west build -p always -b rpi_5 -d build/rpi5_mbox yakogl/tools/probes/rpi_5/rpi5_mbox -- -DYAKOGL_WERROR=ON
```

Buffer mode: both buffers stay cacheable in sram0, which Zephyr already maps.
The probe makes no `device_map()` or `k_mem_map()` of RAM: a second mapping of
that page is refused (`__arch_mem_map` -16, a kernel panic; the run at
6966a16d). The `memory` line reads the display buffer after
`sys_cache_data_invd_range()`, having copied the CPU's view out first; the
image prints `buffer mode cacheable, cache maintenance, no second mapping`.

## The two images' mailbox paths

Both images use the board layer's transport,
`driver/zephyr/lib/board/rpi4/rpi_mbox.c` built for `CONFIG_SOC_BCM2712`
(`CONFIG_YAKOGL_RPI5_MBOX`).

| | `rpi5_display` | `rpi5_clear` |
|---|---|---|
| base, channel, bus view | 0x10_7c013880, channel 8, no alias (`rpi_mbox.c:22-51`) | the same |
| call | `rpi_mbox_tag` (the untimed `transact()`, `rpi_mbox.c:62-89,143-163`) for `GET_THROTTLED` and the clocks | `rpi_mbox_tag` for `SET_CLOCK_STATE` and `GET_CLOCK_RATE` |
| buffer | `rpi_mbox.c`'s static `buf[64]`, 64-byte aligned, `.bss` in the 32 MiB `sram0` from 0x200000 (same overlay) | the same |
| init order | DTB read from x0 first (`rpi_5/common/boot`), then `rpi_mbox_init()`, first request `GET_THROTTLED` | `rpi_mbox_init()`, first request `SET_CLOCK_STATE` of clock 5 (`v3d_power_on`), then `GET_CLOCK_RATE` |
| cache maintenance | **none**: its `prj.conf` has no `CONFIG_CACHE_MANAGEMENT`, so `transact()`'s `sys_cache_data_flush_range()` and `sys_cache_data_invd_range()` return `-ENOTSUP` and do nothing (zephyr `include/zephyr/cache.h:218-227`), with the data cache on | `CONFIG_CACHE_MANAGEMENT=y` (`rpi5_clear/prj.conf`): the clean before the doorbell and the invalidate after the reply run |

Without the clean, the firmware reads a request the CPU has not written back
to memory. Without the invalidate, the CPU reads the answer from its own cached
copy, whose response code is still the request's 0. That matches `-EIO` with
code 0 on every request. The probe tests this hypothesis. The firmware does
not decide it.

## Cause candidates

For section 4 Q1, ranked by the facts above: **candidate 1** is the cache
maintenance `rpi5_display`'s image lacked. Its
[`tools/probes/rpi5_display/prj.conf`](../../rpi5_display/prj.conf) has no
`CONFIG_CACHE_MANAGEMENT` and the BCM2712 SoC does not select it, so its
`transact()` made the request with
`driver/zephyr/lib/board/rpi4/rpi_mbox.c`'s `sys_cache_data_flush_range()`
and `sys_cache_data_invd_range()` (that file's lines 68 and 87) returning
`-ENOTSUP` and doing nothing — the same statement `rpi5_clear/README.md:37-39`
makes; `rpi5_clear` sets the Kconfig (`rpi5_clear/prj.conf:12`), and its run
of 2026-11-19 got the same transport answered on the same board:
`M1 mailbox: 0` and `M1 V3D clock: rate 0 (500000000 Hz)`
(`rpi5_clear/README.md:83,86`). **Candidate 2** is the transport itself
(base, channel, or the firmware not serving this boot); only the probe's
`regs` lines can speak for it. What a run proves: with this image's
`CONFIG_CACHE_MANAGEMENT=y`, every tag answering with status `0x80000000`
(q1 `cache maintenance`: the display way not, clear and board yes) means
candidate 1 was the 2026-11-19 cause; `-EIO` again in the clear and board
ways rules candidate 1 out, and Q1 stays open with this run's raw status and
response codes as the record.

## What it does

The board layer arms the PM watchdog in its first `PRE_KERNEL_1` hook, before
`main` (`CONFIG_YAKOGL_RPI5_WDOG`, hold 60 s). Then `rpi_mbox_init()` runs,
the probe maps the mailbox itself, and seven requests go out, one at a time:
`GET_THROTTLED` (0x00030046), `GET_FIRMWARE_REVISION` (0x00000001), and
`GET_CLOCK_RATE` (0x00030002) of clocks 4 (core), 16 (HVS disp), 13 (HDMI
HSM), 14 (HDMI bvb) and 5 (V3D). Each request goes out three ways, in this
order:

- **display**: the probe's own copy of `transact()` (the same registers,
  bounds and buffer layout), without the cache calls. This is what
  `rpi5_display`'s image ran. `GET_THROTTLED` this way is the image's very
  first request, as in `rpi5_display`. After the wait, the same buffer is also
  read again after a cache invalidate (the `memory` line). That shows the words
  the firmware left in memory, which the cached read cannot show, and the words
  it found there before the doorbell.
- **clear**: the same copy with the clean before and the invalidate after.
  This is what `rpi5_clear`'s image ran.
- **board**: `rpi_mbox_request_timed()` of the board layer in this image,
  built with `CONFIG_CACHE_MANAGEMENT=y`. It keeps the response code word to
  itself, so its line says `status n/a` and its rc says whether that word was
  0x80000000.

The image only reads. `rpi5_clear`'s first request (`SET_CLOCK_STATE`) is not
sent, and `rpi_mbox.c` is not changed.

## The console

Before the requests: `rpi5_mbox: build ...`, `rpi5_mbox: cache management
on|off` (whether `CONFIG_CACHE_MANAGEMENT` is on, so a record says which case
ran: `rpi_mbox.c`'s cache calls do nothing with it off),
`cache: CONFIG_CACHE_MANAGEMENT=1 CONFIG_DCACHE=1, data line N bytes`, the
`paths:` summary, `mailbox init <rc>`,
the probe's own mapping, both buffers' physical addresses, and `regs at start:
sta0 .. cnf0 .. sta1 ..` (`MAIL0_STA`, `MAIL0_CNF`, `MAIL1_STA`). A `sta0`
without bit 30 (EMPTY) at start means a message was already waiting before the
first request.

Each request prints `request <i> of 7`, then for each way a `regs` line
(the three words before and after, the word written, the `MAIL0_RD` word that
ended the wait, any messages of another channel read on the way, and the stage
the transaction reached), then:

```
rpi5_mbox: tag <id> status <hex|n/a> answered <hex> value <hex> rc <n> way <display|clear|board> (...)
rpi5_mbox: memory tag <id> status <hex> answered <hex> value <hex> way display (...)
```

- `status`: the response code word (0x80000000 is success).
- `answered`: the tag's response word (bit 31 set means answered, the low bits
  are the value length).
- `value`: the tag's last value word (the rate in Hz for a clock).
- `rc`: 0, `-EIO` (the code is not 0x80000000), `-ETIMEDOUT`, `-ERANGE` or
  `-ENODEV`.
- `<id>`: the tag; for a clock, `0x00030002:<clock id>`.

Every line is printed even when an earlier step failed. A word that was not
read prints as 0xffffffff, with a line saying why.

At the end: `regs at end`, `answered: display a of 7, memory m of 7, clear c of
7, board b of 7`, then the end line:

- `rpi5_mbox: done (<n> of 7 answered)`: `GET_THROTTLED` was answered in some
  way (n counts the requests that the display, clear or board way answered).
- `rpi5_mbox: FAIL (<reason>)`: `GET_THROTTLED not answered in any way; n of 7
  answered`, or no mailbox mapping.

Then `rpi5_mbox: feeding stopped, reset expected (...)`. The image stops
feeding the watchdog (`rpi_wdog_alive()` returns false), the SoC resets
within 10 s, and tryboot brings Linux back. A hang anywhere ends the same way,
on the 60 s hold.

## Reading a log: verdicts.sh

```sh
tools/probes/rpi_5/rpi5_mbox/verdicts.sh <console.txt>
```

Prints, always in this order and always all of them: `cache:` (the image's
`cache management on|off` line, or `missing`), one verdict per request
(`answered`, `not answered` or `missing`, with each way's `yes`/`no`/`-` and
the value), `q1:` (`cache maintenance`, `not reproduced`, `no way answered`
or `unknown`), `end:` (the image's end line, or `missing`), and
`rpi5_mbox verdict: PASS (GET_THROTTLED answered by ...)` or `FAIL (...)`.
Exit 0 PASS, 1 FAIL, 2 usage. The rule is section 3's: PASS when
`GET_THROTTLED` is answered. A log cut short still gets all eleven lines.

`verdicts_selftest.sh` (`sh tools/probes/rpi_5/rpi5_mbox/verdicts_selftest.sh`)
feeds it canned logs and checks every line it prints: all answered; every way
`-EIO` with code 0, as on 2026-11-19; the cache case (display `-EIO`, the
answer in memory, clear and board answered); an answer in memory only; one
request missing; a log truncated inside request 3; a log cut before any
request (its `cache:` line is then `missing`); `cache management off` and the
timestamped CR form of `on`; usage. The gate gives it a row
(`tools/check_selftests.py`), and `zephyr_rpi5_probe_images` builds the image.

## On the board

Not run yet.

A session runs it through `tools/board.sh pi5 run <zephyr.bin> <logdir>` with

```sh
BOARD_END_RE='^rpi5_mbox: (done|FAIL)'
```

then `verdicts.sh` on the captured console. The rules of
`tools/probes/rpi_5/README.md` apply: `get_throttled` from Linux in
`meta.txt`, and if the board does not come back, stop and report that it needs
a person.

## Limits

- The display way runs `rpi5_display`'s path in an image built with
  `CONFIG_CACHE_MANAGEMENT=y`: the cache calls are skipped, not compiled out.
  The data cache is on in both images.
- The `memory` read goes through a second, uncached mapping of a page that
  the image also maps cacheable. A dirty line of the display buffer can be
  evicted at any time, so the memory words show what was in memory at that
  read. They are evidence, not a count: `done` and `n` never include them.
- No doorbell block beyond the mailbox's own status and config words is read.
  The tree documents no other address for it.
