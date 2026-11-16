<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# DK u-boot-env fixtures for the standalone default

U-Boot environment images of the STM32MP257F-DK for the no-board selftests of
the standalone runner (`tools/board-stm32mp2-standalone.sh`) and the board
session driver: ST's defaults, and that environment at each step of
`default <image> 2` (`docs/notes/stm32mp257f-dk-standalone.md`, sections 3
and 7). #362 (plan), #1281 (DK bring-up). No byte comes from a board.

## Layout

Every image is **0x4000 bytes**: a redundant environment of two copies of
`CONFIG_ENV_SIZE` 0x2000 bytes (`tools/mp2s_env_capacity.py`'s default; ST's
`arch/arm/mach-stm32mp/Kconfig:33-34`). Each copy is a CRC32 (zlib,
little-endian) over the data area, a flags byte, and a data area of 8187
bytes: `name=value\0` ... `\0`, padded with `0x00` as U-Boot's `env_export`
pads. Copy 1 is at 0, copy 2 at 0x2000. Read them with

    python3 -I tools/uboot_env.py --redundant --size 0x2000 print|check IMAGE

Copy 1 is uboot_env.py's primary: on equal flags it is the active copy, as
U-Boot's primary (the copy at the top of `u-boot-env`, 0x7e000) is. On the
card the order is the other way round: the redundant copy is at 0x7c000,
**below** the primary. A dd of the partition's top 0x4000 bytes is therefore
`--offset 0x2000 --offset2 0` to uboot_env.py, not these files' layout. A
test that needs a whole 512 KiB partition places copy 1 at 0x7e000 and copy 2
at 0x7c000, as `tools/board_stm32mp2_selftest.sh` does.

## The images

"ST defaults" is `driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt`
(ST U-Boot `v2023.10-stm32mp-r3.1`) with one entry per name: 54 variables, in
the text's sorted order. The text has `boot_net_usb_start` twice; the image
keeps only the last value, `true`, as U-Boot's import does. ST's defaults
have `boot_prefixes=/ /boot/` and no `bootcount`, `bootlimit`,
`upgrade_available` or `mp2s_*`.

"The default write" is the four variables the runner's `default` writes in
one environment write (note, section 3, table; section 7, "Runner verbs"),
with the values of `mp2-standalone.cmd`:

| variable | value |
|---|---|
| `mp2s_ver` | `1` |
| `mp2s_prefixes` | `/ /boot/` (ST's `boot_prefixes` before `default`) |
| `boot_prefixes` | `/yakogl/standalone/ / /boot/` (changed in place, at its sorted position) |
| `mp2s_left` | `2`, `1`, `0` below |

The new variables are appended in that order (`mp2s_ver`, `mp2s_prefixes`,
then `mp2s_left`), as uboot_env.py's `set` writes them; U-Boot's own
`saveenv` would sort them.

| image | copy 1 (flags) | copy 2 (flags) | active | what it is |
|---|---|---|---|---|
| `pristine.bin` | ST defaults (0) | ST defaults (0) | copy 1 | ST's defaults only, both copies valid and equal |
| `installed.bin` | ST defaults (0) | + `mp2s_ver`, `mp2s_prefixes`, `boot_prefixes` (1) | copy 2 | the default write without `mp2s_left` |
| `counting2.bin` | + `mp2s_left=2` (2) | as `installed.bin` (1) | copy 1 | `default <image> 2` written: the script's next boot prints `boots left 1` |
| `counting1.bin` | as `counting2.bin` (2) | `mp2s_left=1` (3) | copy 2 | after the first boot's decrement and save |
| `left0.bin` | `mp2s_left=0` (4) | as `counting1.bin` (3) | copy 1 | after the second boot's save (or the console undo `setenv mp2s_left 0; saveenv`): the next boot is "default used up" |
| `one-copy-bad.bin` | `counting1.bin`'s copy 1, CRC byte 0 inverted: **bad** | `counting1.bin`'s copy 2, valid (3) | copy 2 (U-Boot's choice) | a broken inactive copy; U-Boot boots from copy 2 (`mp2s_left=1`) |
| `both-bad.bin` | as `one-copy-bad.bin`: **bad** | `counting1.bin`'s copy 2, CRC byte 0 inverted: **bad** | none | U-Boot would fall back to its compiled-in defaults and `env_check` would save them |

Each step from `installed.bin` on is one uboot_env.py `set` on the image
before it: it writes the inactive copy with flags = active + 1 and leaves the
active copy as it was, as U-Boot's `env_save` does. So the inactive copy of
each image holds the state one step earlier.

Two caveats:

- `installed.bin` is not a state the runner leaves: `install` writes no
  environment, and `default` writes all four variables at once (never the
  prefix without a count). It is the base the counting images are built
  from, and the "no default set" case of the script (`mp2s_left` absent)
  with the prefix present. The item's names `mp2s_installed` and
  `mp2s_file` do not exist in the design: section 7 fixes the file names in
  the script ("there is no file-name variable"), so they are not in any
  image.
- uboot_env.py refuses to `print` a pair with a bad copy (U-Boot would read
  the valid one). `test_mp2s_env.py` reads `one-copy-bad.bin`'s copy 2 by
  pairing it with itself; `check` names the bad copy in both bad images.

## Regenerate and check

    TMPDIR=<dir> tools/testdata/mp2s-env/regen.sh           # writes the *.bin that differ
    TMPDIR=<dir> tools/testdata/mp2s-env/regen.sh --check   # rebuilds in $TMPDIR, exit 1 on a difference
    TMPDIR=<dir> python3 -I -B tools/testdata/mp2s-env/test_mp2s_env.py

`regen.sh` builds `pristine.bin` from the text with a few lines of Python
(dedupe, pad, CRC), the next four with `tools/uboot_env.py set`, and the two
bad ones by inverting CRC byte 0 of `counting1.bin`'s copies. Nothing depends
on time or the host: two runs give the same bytes. A change to the text
fixture or to uboot_env.py's edit moves these files; regenerate them in the
same commit. `test_mp2s_env.py` checks every image's CRCs, flags, active copy
and the variables of both copies against the table above, and runs
`regen.sh --check`.
