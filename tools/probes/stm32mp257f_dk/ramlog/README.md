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
snippet. `--regen` rewrites the `.want` files.
