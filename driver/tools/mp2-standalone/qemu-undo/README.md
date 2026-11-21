<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2-standalone/qemu-undo: the card-in-a-PC undo, booted

`tools/mp2s_card_undo.sh` is the undo for when Linux is not reachable (design
note `docs/notes/stm32mp257f-dk-standalone.md`, section 5). This run boots
U-Boot v2023.10 from a disk image after that undo.

```sh
driver/tools/mp2-standalone/qemu-undo/run.sh   # last line: "qemu-undo: ok" | "qemu-undo: FAIL ..." | "qemu-undo: SKIP <why>"
```

Inputs as `../qemu-product/run.sh`: `QEMU_SYSTEM_AARCH64`, `UBOOT_V2023_10_SRC`
(or `$CONNECTED_DEPS_DIR/u-boot-v2023.10`), `CROSS_COMPILE`. A missing input
prints `qemu-undo: SKIP <reason>` and exits 0. `MP2S_QEMU_KEEP=1` keeps the run
directory and console logs.

Steps (`undo.py` is the console side; `../qemu` and `../qemu-product` are
imported, never changed):

1. A disk from `../qemu/mkdisk.sh ... env` (GPT, `bootfs` ext4, 512 KiB
   `u-boot-env`) with the real product script (`../build.sh`), a stand-in
   image, the stock `/mmc0_boot.scr.uimg` marker, and the root
   `boot.scr.uimg` and `mmc0_extlinux/` the undo insists on. The redundant-env
   U-Boot (`build-uboot.sh redund`). ST's loop variables and `default 2`'s
   variables are typed and saved. Boot 1: `boots left 1`, the image, stock.
2. QEMU is off. The disk's `bootfs` partition is cut out with `dd` (the tool
   refuses a whole disk), `mp2s_card_undo.sh --yes` runs on it, it is put
   back. Its output is checked against section 5: the one path written, the
   directory read back (`boot.scr.uimg.off`, no `boot.scr.uimg`), `stock files:
   sha256 unchanged`, `filesystem state: clean`, the `nodefault` next step.
3. Boot 2 on the same disk and environment (still holding the default): no
   `mp2s:` line, our script not found, the image does not run, the stock marker
   `mp2q: stock path /mmc0_ reached` appears, no `bad CRC`.

The stand-in stock `mmc0_extlinux/` holds no `.conf`, so the loop's stock step
is the `/mmc0_boot.scr.uimg` marker. Section 5 says the loop prints `SCRIPT
FAILED: continuing...` after the undo; with no script found it may not. The run
prints a `note:` line if it is absent and does not fail on it.
