<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# linux_display_dump

A read-only dump of the STM32MP257F-DK's display path (LTDC, DSI host and ST
wrapper, ADV7535, EDID) as ST's Linux drives it, to be compared with our
drivers' sequences (`driver/docs/stm32mp257f-dk-display-allow.txt` holds the
bits that may differ). Run on the DK's Linux, never on our image.

## What it does and does not do

- Registers: `/dev/mem` opened `O_RDONLY`, `mmap` `PROT_READ`, 32-bit loads
  (`--reader devmem` uses `devmem ADDR 32`, no write value). Each block is
  read in a child process, so a bus error costs one `ERR` value, not the dump.
- The list of addresses is the `#@read` / `#@name` lines in the header of
  `driver/docs/stm32mp257f-dk-display-allow.txt`. DSI registers that change
  on read (`GEN_PLD_DATA`, `INT_ST0`, `INT_ST1`) are not in it.
- ADV7535: the regmap debugfs files `/sys/kernel/debug/regmap/<bus>-00<addr>/registers`
  of the maps main 0x3d, CEC 0x3c, EDID 0x3f, packet 0x38 (the DK's
  device tree), on the bus that holds the main map. debugfs must already be
  mounted; the script does not mount it.
- EDID and modes: `/sys/class/drm/card*-*/{edid,modes,status,enabled}` and
  `modetest -c -e -p` (listing, no mode set; `--no-modetest` skips it).
- It writes no register, loads no module, mounts nothing and does not touch
  the network, sshd or Tailscale. It needs root on the board (for `/dev/mem`)
  and `python3`; with `CONFIG_STRICT_DEVMEM` the reads of device registers may
  be refused (shown as `ERR`; report it, do not change the kernel).

## Run

From the host, with the board named in the environment only:

    DK_SSH=<ssh destination> tools/probes/stm32mp257f_dk/linux_display_dump/run_dump.sh \
        > driver/docs/stm32mp257f-dk-display-dump-linux.txt

`DK_SSH_OPTS` adds ssh options. The script and read list travel over the
connection; nothing is written to the board's storage. On the board itself:
`python3 linux_display_dump.py --readlist stm32mp257f-dk-display-allow.txt`.

## Output format

Sections, in this order, each starting at its `[name]` line; `#` lines are
comments (`# kernel <release>` is the second line):

    [regs]       0xADDRESS 0xVALUE NAME     (VALUE is ERR if the read failed)
    [adv7535]    MAP 0xOFFSET 0xVALUE       (MAP: main, cec, edid, packet)
    [edid]       CONNECTOR HEX              (the whole EDID, one line)
    [modes]      CONNECTOR status|enabled|mode VALUE
    [modetest]   the raw `modetest` listing

A range word without a `#@name` is `NAME+0xOFFSET`.

## Selftest

    bash tools/probes/stm32mp257f_dk/linux_display_dump/selftest.sh

Checks that the allow file parses (every non-comment line is `hex hex text`),
that the script holds no write-capable construct, that a fake register file
and a fake `/sys` tree produce the exact expected dump (including an `ERR`
word and the bus choice for the ADV7535 maps), and that the real read list
covers the display blocks and avoids the read-sensitive DSI registers.
