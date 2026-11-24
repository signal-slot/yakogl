<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ramlog: the DK's RAM-log channel, host side

A blind run of the STM32MP257F-DK (`MP2_NO_CONSOLE=1`, ssh only) has no
console text. The image can copy its printk output into a RAM ring at a fixed
DDR address; the runner-owned U-Boot preboot saves the ring to a file on the
bootfs after the reset, and the runner pulls it over ssh. This directory is
the host side: the decoder, the U-Boot commands and a self-test. The image
side (the ring writer) and the runner's use are other items.

## Format (fixed, little endian)

| offset | field |
|---|---|
| 0 | u32 magic `0x474C4B59` (`YKLG` as bytes) |
| 4 | u32 version, 1 |
| 8 | u32 size: bytes of data |
| 12 | u32 head: total bytes written, mod 2^32 |
| 16 | u32 seq |
| 20 | u32 flags, bit0 wrapped |
| 24, 28 | u32 0, u32 0 |
| 32 | `size` data bytes; stream byte i is `data[i % size]` |

`head <= size`: the text is `data[:head]`. Otherwise it is the last `size`
bytes in stream order and `[ramlog: N bytes lost]` goes to stderr.

`python3 -I ramlog_decode.py ramlog.bin > console.txt` — exit 0 ok; 3 bad
magic or version (`ramlog: no ring`, also a file under 32 bytes); 4 the file
is shorter than 32 + size (bytes after the ring are ignored); 2 usage or
unreadable file. Undecodable bytes become U+FFFD; it never crashes on them.

## How the runner uses it

`uboot_snippet.txt` holds the one hush command line the runner puts in its
own variable and runs from its hook. It checks the magic
(`itest.l *${yako_ramlog_addr} == 0x474C4B59`), writes
`ext4write mmc ${boot_instance}:${yako_part} ${yako_ramlog_addr}
/yakogl/ramlog.bin ${yako_ramlog_size}` (the runner stages under
`/boot/yakogl`, which U-Boot sees as `/yakogl`; `boot_instance` and
`yako_part` are the runner's existing names), then `mw.l ${yako_ramlog_addr}
0 1` so a later boot does not save the same ring again. The runner sets
`yako_ramlog_addr` (the image's Kconfig address) and `yako_ramlog_size`
(32 + the ring's data size) itself; they are not in the runner yet.
After the reset the runner copies `/boot/yakogl/ramlog.bin` over ssh, deletes
it, decodes it into `console.txt` and gives that to
`../verdict/dk_verdict.py`.

## Limit: the DK's warm reset does not keep DDR (measured 2026-11-24)

On the DK's boot chain (TF-A `v2.10-stm32mp2-r2.0(debug)`, OP-TEE
`4.0.0-stm32mp-r3.1`, U-Boot `2023.10-stm32mp-r3.1`) the ring never comes
back. Item ramlog-fix measured this with U-Boot's console attached:

- Runner trial (`MP2_NO_CONSOLE=1 MP2_RAMLOG=1`, mp2_hello built with
  `../ramlog_conf/ramlog.conf`). The image printed its banner and
  `mp2: mp2_hello done: PSCI SYSTEM_RESET at 5022 ms`. TF-A printed
  `Reset reason: System reset (SYSRST) (0x2074). Warm boot.`, and on the
  next boot the save step printed neither `ramlog saved` nor
  `ramlog write failed`: its magic check did not match.
- From U-Boot's prompt: `mw.l` of the magic and a second word at 14
  addresses from 0x84000100 to 0xf0000040 (0xb0100000, 0xb0100040 and
  0xb010fff0 among them), `dcache flush`, `reset`. On the next boot not one
  value was left. The ring range read `ffffffff`, and the other addresses
  read uniform `00000000` or `ffffffff` regions, the decay of unrefreshed
  DRAM (0x84000100 holds U-Boot's own code, loaded by BL2 again).
- The same check across the watchdog reset (a branch-to-self at
  0xa0000000, `go`): OP-TEE printed `Panic 'Watchdog' at ...
  stm32_iwdg.c:218`, then TF-A `Reset reason: IWDG system reset
  (iwdgX_out_rst) (0x2134). Warm boot.`. The planted words at 0xb0100000
  and 0xd0000040 read `ffffffff`.

BL2 brings the DDR up again on every boot and the DRAM is not held in
self-refresh across the reset, so no DDR address can carry the ring. The
on-chip RAMs that do survive a system reset are all claimed in the boot
chain's reserved-memory: `bl31-lowpower@42000000` and `tfm-its@42001000`
(the 8 KiB backup SRAM), `cm33-retram@a080000`, and `ddr-param@a09f000`.
None of them is the A35 normal world's to take.

What the tools do about it:

- `uboot_snippet.txt` prints `yakogl-mp2: ramlog no ring at <addr>` when
  the magic is absent, so U-Boot's console shows which outcome it was.
- `tools/board-stm32mp2.sh` with `MP2_RAMLOG=1` prints a WARNING naming
  the limit before the arm and writes `ramlog_limit=ddr_lost_in_warm_reset`
  in `meta.txt`. A ring that is missing then ends the verdict line with
  ` ramlog=missing:ddr_lost_in_warm_reset` and prints a NOTE. A ring that
  does come back (another boot chain) is decoded as before, with a NOTE
  that the limit did not hold.
- The serial console (the farm's console resource) is the only console
  text of a DK run; a blind run gets the return verdict alone.

The format, decoder and writer stay as they are, for a boot chain that
keeps DDR (or a future one that leaves the ring alone).

## Evidence rule

A ramlog record is board evidence (it shows what the image printed, and that
the reset path brought the board back), but it is never a `tools/cts/RESULTS.md`
row without the console: a row needs the console log of the run.

## Self-test

`bash selftest.sh` builds synthetic dumps (empty, short, exact fill, wrapped,
bad magic, bad version, truncated, tiny, invalid UTF-8) and compares them
with `testdata/*.want`; it also wraps the verdict tool's
`return-trial.pass.log` into a ring behind junk, decodes it and checks that
`dk_verdict.py --expect return-trial` gives the original line, and checks the
snippet, including its "no ring" branch (the magic check's `else`). For the
limit it decodes what the reset left at the ring address (all `ffffffff`,
as U-Boot's `md` read it): `ramlog: no ring`, exit 3. It also has the
runner's `tally` judge two
records of a blind `MP2_RAMLOG=1` run whose ring is missing: one with
`ramlog_limit` in `meta.txt`, whose verdict line must end in
`ramlog=missing:ddr_lost_in_warm_reset`, and an older one without it, whose
line stays as it was. `--regen` rewrites the `.want` files.

## Host cross-check of the writer

`bash crosscheck/run.sh` compiles the board lib's `ramlog.c`
(`driver/zephyr/lib/board/common/`) unchanged with the host gcc against stub
Zephyr headers (`crosscheck/stubs/`: the printk hook, `SYS_INIT`, the barrier,
`device_map` onto a host buffer). `harness.c` writes known text through the
writer's hook and dumps the buffer; `ramlog_decode.py` decodes it and the
output is diffed against `crosscheck/expected/`. Cases: short, wrap (more than
five times round), empty, garbage, torn (data byte stored, head not yet
updated). Ends "N checks, 0 failed"; `--regen` rewrites `expected/`.
