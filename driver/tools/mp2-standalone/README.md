<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2-standalone: the STM32MP257F-DK's standalone boot script

The U-Boot script of the DK's standalone default: a counted default of N boots
(`mp2s_left` 1-9) or `forever`. ST's chain (TF-A, OP-TEE, ST's U-Boot
v2023.10) stays. ST's distro loop sources the script as
`/yakogl/standalone/boot.scr.uimg` from the microSD's bootfs, through the one
extra `boot_prefixes` entry the runner's `default` puts in front. `bootcmd` is
never changed. The design and the interface contract are in
[`docs/notes/stm32mp257f-dk-standalone.md`](../../../docs/notes/stm32mp257f-dk-standalone.md)
(sections 2, 3, 5, 6 and 7).

| file | job |
| --- | --- |
| `mp2-standalone.cmd` | the hush script, in section 3's order: undo line; `no default set` exit; `mp2s_ver` check; `forever`, or the count 1-9 decremented and **saved before anything is loaded**; when used up, `boot_prefixes` back from `${mp2s_prefixes}`, `mp2s_*` deleted, saved, `exit`; then `size`/`load` of `/yakogl/standalone/default.bin` to `0xa0000000` with the window checks, caches off, `go`; `the image returned` and `exit` |
| `build.sh` | `build.sh [OUTDIR]`: `mkimage -A arm64 -O linux -T script -C none -n mp2-standalone -d mp2-standalone.cmd OUTDIR/boot.scr.uimg` and prints its sha256. Without `mkimage`, `tools/uboot_script.py build` (the same bytes) |
| `selftest.sh` | the host test, no board |
| `qemu/` | the QEMU harness (its own README) |

## Build

```sh
driver/tools/mp2-standalone/build.sh out/
# build.sh: out/boot.scr.uimg sha256 <hex> (via mkimage)
```

The image is not committed. The header's time is `SOURCE_DATE_EPOCH` when set,
else the commit time of `mp2-standalone.cmd` (0 outside git), so one commit
always builds the same bytes and the runner can compare the staged script's
sha256 with its own build. `MP2_NO_MKIMAGE=1` forces `tools/uboot_script.py`.

## Selftest

```sh
driver/tools/mp2-standalone/selftest.sh
# mp2-standalone selftest: N ok, 0 failed, ...
```

It needs `python3` and `sha256sum`, nothing else. It checks:

- `build.sh` prints the image's sha256, and two builds give the same bytes;
- the uImage header, **parsed in Python by the selftest itself** (magic, both
  CRCs, linux/arm64/script/no compression, the length table, the script bytes
  equal the source). With `mkimage` present (on `PATH`, or `MKIMAGE=/path`),
  `mkimage -l` must also read it and mkimage's build must equal
  `uboot_script.py`'s; without it that is reported as skipped, and
  `MP2_REQUIRE_MKIMAGE=1` makes it a failure;
- section 7's lint: the commands `echo setenv test itest setexpr size load
  saveenv dcache icache go exit` only; `setenv` of `mp2s_*` and
  `boot_prefixes` only; `boot_prefixes` set only from `${mp2s_prefixes}`;
  `saveenv` exactly twice (after the decrement, in the used-up branch); no
  `bootcmd`, `altbootcmd`, `bootcount`, `bootlimit`, `upgrade_available`,
  `boot_targets`, `preboot`, `mp2z_`, `yako_`, `env default`, `eraseenv`,
  `mmc write`, `ext4write`, `fatwrite`, `gpt` or `run` anywhere in the file,
  comments included;
- section 3's order: the undo first, `boots left` right after the first save,
  no RAM variable, `size`, `load` or `go` before the last save, and the script
  ends with the `go` sequence, `the image returned`, `exit`;
- every `mp2s: ` line is one of section 7's console lines (read from the
  note), and each of those is printed somewhere;
- `mp2s_load`/`mp2s_window` equal `../uboot-mp2/mp2-mmc.cmd`'s
  (`0xa0000000`/`0x10000000`), which that directory's selftest ties to the
  board's DTS;
- negative cases: mutated copies of the script (each forbidden token, a third
  or a missing `saveenv`, `boot_prefixes` from a literal, a load before the
  save, an unlisted console line, ...) that must be refused, each printed
  with its first reason.

What it does not check: that U-Boot runs the script as intended. That is the
QEMU run's (`qemu/`) and the board run's (the note's "Board run" section).
