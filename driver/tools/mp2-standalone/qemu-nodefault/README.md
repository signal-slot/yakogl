<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2-standalone/qemu-nodefault: nodefault partway through a count, in real U-Boot

Proves, in U-Boot v2023.10 under QEMU, the claim of sections 3 and 7 of
`docs/notes/stm32mp257f-dk-standalone.md`: on the board `nodefault` is an
environment edit computed by `tools/mp2s_env_plan.py` and applied from Linux;
done in the middle of a count it gives the stock boot next, and ST's
`bootcmd`, `bootcount` and `altbootcmd` are untouched. Host only, no board.

```sh
QEMU_SYSTEM_AARCH64=... UBOOT_V2023_10_SRC=... driver/tools/mp2-standalone/qemu-nodefault/run.sh
```

Prints `qemu-nodefault: ok` (exit 0), `qemu-nodefault: SKIP <reason>` (exit 0,
when `QEMU_SYSTEM_AARCH64`, the U-Boot tree, a cross compiler or a disk tool is
missing) or `qemu-nodefault: FAIL ...` (exit 1; the run directory and the
console logs are kept). `MP2S_QEMU_KEEP=1` keeps them on success.

It reuses `../qemu` (`build-uboot.sh redund`, `mkdisk.sh DISK BOOTFS env`,
`drive.py`) and `../qemu-product/product.py`'s constants and console lines,
changing none of them: the redundant-environment U-Boot, a GPT disk with
`bootfs` and a 512 KiB `u-boot-env` partition (two 8 KiB copies at 0x7e000 and
0x7c000, as on the DK), ST's distro-loop variables typed from the
`st-env` fixture, and a stand-in stock script `/mmc0_boot.scr.uimg` that prints
`mp2q: stock path /mmc0_ reached`.

## The flow

1. **Setup.** One boot types ST's variables and `saveenv`s twice (both copies
   valid). The partition is read; `bootcmd`, `bootcount`, `altbootcmd` are kept
   as "before".
2. **Default 3.** On the host `mp2s_env_plan.py default --count 3` computes the
   edit and the result is written back to the partition. **Boot 1** runs the real
   product script (`../build.sh`'s `boot.scr.uimg`): `mp2s: default
   /yakogl/standalone/default.bin, boots left 2`, the image, then the stock path.
3. **Power off, edit.** QEMU has exited. The host reads the `u-boot-env`
   partition, runs `mp2s_env_plan.py nodefault` (record from step 2), checks
   that exactly one copy changed (the inactive one, flags + 1, which is what
   `fw_setenv` does on a redundant env), writes the partition back, and runs
   `mp2s_env_plan.py verify --expect nodefault`.
4. **Boot 2.** Untouched boot: no `mp2s:` line, no image, our script is not even
   found (`boot_prefixes` is the stock value), and `mp2q: stock path /mmc0_
   reached`.
5. **Compare.** From the partition (`tools/uboot_env.py`): `bootcmd`,
   `bootcount` and `altbootcmd` are byte-identical to their state before step 2;
   `mp2s_ver`, `mp2s_left`, `mp2s_prefixes` are gone and `boot_prefixes` equals
   its stock value.

The run prints the `mp2s:`/`mp2q:` console lines of both boots
(`     boot 1: ...`, `     boot 2: ...`) and the three variables.

## Not run here

The task machine had neither QEMU nor a U-Boot tree, so only the SKIP path, `bash
-n` and a Python compile check were run when this was written; the first real run
may need small fixes (for example the console line text, which comes from
`../qemu-product/product.py`).
