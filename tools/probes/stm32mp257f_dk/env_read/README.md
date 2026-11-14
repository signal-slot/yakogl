<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# env_read: the DK's U-Boot environment and /boot, read-only

Plan #357 boots the STM32MP257F-DK from ST's U-Boot as a one-shot that must
leave `bootcmd`, the stored environment and the stock `/boot` files byte for
byte unchanged (acceptance c5). This probe reads the Linux side of that, once
before and once after a run, and compares. It supplies the board facts
section 11 of `docs/notes/stm32mp257f-dk-uboot-boot.md` asks for
(`TODO(board-env-read)`).

```
BOARD_HOST=user@host MP2_SUDO="sudo -n " tools/probes/stm32mp257f_dk/env_read/collect.sh OUTDIR
tools/probes/stm32mp257f_dk/env_read/summarize.py OUTDIR              # section-11 rows
tools/probes/stm32mp257f_dk/env_read/summarize.py BEFORE AFTER        # c5's comparison
tools/probes/stm32mp257f_dk/env_read/selftest.sh                      # no board
```

`collect.sh` takes the environment `tools/board-stm32mp2.sh` takes
(`BOARD_HOST`, `MP2_SUDO`, `MP2_KNOWN_HOSTS`); no address, host name or UART
path is in any file. It is read only: `fw_printenv`, `findmnt`, `lsblk`,
`/proc/partitions`, a `dd if=` of the `u-boot-env` partition into a host
file, `ls -lR`, `find`, `sha256sum` and `df` on `/boot`. It never runs
`fw_setenv`, never writes with `dd`, never mounts anything and never touches
modules, tailscale or sshd; the selftest greps it for them. The
`u-boot-env` partition is the one labelled so on the card `/boot` is on.
Exit 0 all read, 2 a refusal before any read, 3 a read failed.

`summarize.py` (Python 3, standard library; run it with `python3 -I`) checks
the image with `tools/uboot_env.py` (`--redundant --size --offset --offset2`;
defaults 0x2000, 0x7e000, 0x7c000 as the board runner, overridable with
`MP2_ENV_SIZE`, `MP2_ENV_OFFSET`, `MP2_ENV_OFFSET2`). One directory: markdown
tables of the 17 variables, the partition (size, both copies' CRCs and
flags, the active copy, image sha256), `/boot` (bootfs partition number,
free space, every file's size and sha256). Two directories: the
`fw_printenv` diff, the image's sha256 and active copy, and the `/boot`
files whose sha256 changed, were added or removed, ignoring `./yakogl/`.
Exit 0 when nothing differs outside `yakogl/` (and both images check), 1
otherwise, 2 usage.

`selftest.sh` builds synthetic images with `uboot_env.py set` and runs the
identical, env-changed, bootcmd-changed, /boot-file-changed,
yakogl-only-changed and corrupt-CRC cases, and `collect.sh` against a fake
`ssh` whose log must hold no write. It ends with
`env_read selftest: N ok, M failed`.
