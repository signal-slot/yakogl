<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2-standalone/qemu-card: the 'forever' product card, booted in QEMU

A host-only, optional run that boots the output of `tools/mp2s_product_card.py`
(the product card of `docs/notes/stm32mp257f-dk-standalone.md`, section 2, "The
product, on a card of its own") in U-Boot v2023.10 on QEMU's `virt` machine.
The card's two environment copies are written where `../qemu`'s `redund`
U-Boot reads them, so U-Boot loads the card's own environment, as the DK
does. No board, no network. It reuses `../qemu/build-uboot.sh redund`,
`../qemu/mkdisk.sh`, `../qemu/image.S`, `../qemu/drive.py` and `../build.sh`
without changing them.

| file | job |
| --- | --- |
| `run.sh` | entry: checks its inputs, builds the `redund` U-Boot, the product script (`../build.sh`) and the stand-in image, runs `card.py` |
| `card.py` | builds the card with `mp2s_product_card.py`, writes it to a disk image, boots it N times and checks every boot; imports `../qemu/drive.py` for the console driver and the environment layout |

```sh
driver/tools/mp2-standalone/qemu-card/run.sh
```

Last lines: one `case NAME: PASS|FAIL (N ok, M failed)` per case and
`card.py: N ok, M failed`. Exit 0 only when nothing failed. Each assertion
prints a `PASS|FAIL case: ...` line, and each boot prints its `mp2s:` and
`mp2q:` console lines.

- `QEMU_SYSTEM_AARCH64`: the QEMU binary. When it is unset, the Zephyr SDK
  host tools' QEMU under `$CONNECTED_DEPS_DIR/zephyr-hosttools` is used
  (`zephyr-sdk-x86_64-hosttools-standalone-0.10.sh -y -d
  $CONNECTED_DEPS_DIR/zephyr-hosttools`). If neither is there, the run prints
  `SKIP: QEMU_SYSTEM_AARCH64 ...` and exits 0.
- `UBOOT_V2023_10_SRC`: when it is unset, `$CONNECTED_DEPS_DIR/u-boot-v2023.10`
  is used if it exists. Otherwise the run prints `SKIP: UBOOT_V2023_10_SRC`
  and exits 0. Fetch it as `../qemu/README.md` says (`git clone --depth 1 -b
  v2023.10 https://source.denx.de/u-boot/u-boot.git`).
- With no cross compiler the run prints `SKIP: CROSS_COMPILE`. A missing
  `python3`, `mke2fs`, `sfdisk` or `truncate` gives `SKIP: <tool>`.
- `MP2S_CARD_BOOTS` sets the number of boots (default 3). `MP2S_QEMU_KEEP=1`
  keeps the run directory, with `logs/<case>-boot<n>.log` and the card under
  `work/card`. A failing run keeps it anyway. The cache is `../qemu`'s
  (`MP2S_QEMU_CACHE`).

## What it does

1. **The input environment**, the one the card tool edits. It holds ST's
   distro-loop variables from `../st-env/stm32mp25-dk-initial-env.txt`, byte
   for byte, as `../qemu-product` types them (`bootcmd=run bootcmd_stm32mp`,
   `bootcmd_stm32mp`, `env_check`, `distro_bootcmd`, `scan_dev_for_*`, ...).
   It also holds the board's saved `boot_prefixes=/mmc0_`, `../qemu-product`'s
   QEMU adjustments (`preboot`, `boot_device=mmc`, `boot_instance=0`,
   `boot_targets=mmc0`, `scan_dev_for_efi`, `bootcount=0`) and the addresses
   of `../qemu/redund-default.env`. The full fixture is not used: it repeats
   one name (`boot_net_usb_start`), and the card tool refuses an input with a
   repeated name.
2. **The card**: `mp2s_product_card.py --env-text IN --env-size 0x2000
   --script boot.scr.uimg --image default.bin --out card`, then the same with
   `--no-linux`. The checks:
   - the four card files are byte-identical in both runs, and the manifest
     only gains the `NOTE: --no-linux` line, so the flag changes no byte of
     the card;
   - both copies are 0x2000 bytes with good CRCs, and copy 1 is active
     (flags 1 over 0);
   - the environment equals the input except `mp2s_ver=1`,
     `mp2s_left=forever`, `mp2s_prefixes=/mmc0_` and
     `boot_prefixes=/yakogl/standalone/ /mmc0_`.
3. **The disk**: `../qemu/mkdisk.sh DISK BOOTFS env`, a GPT disk with an ext4
   bootfs and a 512 KiB `u-boot-env` partition. Bootfs holds the card's
   `yakogl/standalone/` tree. `u-boot-env-copy1.bin` goes at disk offset
   0x1e7e000 and `u-boot-env-copy2.bin` at 0x1e7c000. These are `../qemu`'s
   copy 1 (`CONFIG_ENV_OFFSET`) and copy 2 (`CONFIG_ENV_OFFSET_REDUND`), at
   0x7e000 and 0x7c000 into the partition, as on the DK. No setup boot and
   nothing typed: the card as written is what U-Boot boots.
4. **N untouched boots** (separate QEMU runs on the same disk), in two cases:

| case | bootfs | each boot must show |
| --- | --- | --- |
| `forever` | the card's tree plus `/mmc0_boot.scr.uimg`, a stand-in for ST's `mmc0_extlinux/` (Linux) that prints `mp2q: stock path /mmc0_ reached ...` | `Loading Environment from MMC... OK` (no bad CRC). Then `Found U-Boot script /yakogl/standalone/boot.scr.uimg`, the undo line, `mp2s: default /yakogl/standalone/default.bin, forever`, `loaded 1000 bytes at 0xa0000000 ...`, `go 0xa0000000`, the image's own line, `the image returned, booting Linux`. Then, because the stand-in image returns, the loop goes on to the stock stand-in |
| `forever_no_linux` | the card's tree only: the card after the person removed `mmc0_extlinux/`, which is what `--no-linux`'s note asks for | the same up to `the image returned`, and no stock path |

After every boot, both cases also check:

- no `boots left`, `Saving Environment`, `used up` or `refused` line;
- `printenv` shows `mp2s_left=forever`, our `boot_prefixes` and ST's `bootcmd`;
- both environment copies on the disk are byte-identical to the card's files;
- the sha256 of the **whole disk** is unchanged.

So the environment does not change between boots. `env_check` saves nothing,
because the environment came from the card and not from the defaults. The
script's `forever` branch saves nothing either.

## Differences from the DK

These are `../qemu-product`'s (its README, "Differences from the DK"), with
one exception: the environment is the DK's two copies on the card, not CFI
flash.

- QEMU's U-Boot (`qemu_arm64_defconfig` plus `../qemu`'s fragments) runs ST's
  loop text, not ST's stm32mp25 build. QEMU's device tree names no
  `u-boot,mmc-env-partition`, so the environment offsets are absolute. They
  are the same bytes as at 0x7e000 and 0x7c000 into `u-boot-env`.
- The stand-in `default.bin` (`../qemu/image.S`) returns from `go`. A
  'forever' product image never returns. On the DK it runs until power-off,
  and the BL2 IWDG bounds it unless it refreshes the watchdog (design note,
  section on the hang bound). QEMU has no IWDG.
- The stock path is a stand-in script, not ST's extlinux and OpenSTLinux.
- `--env-size 0x2000` is passed explicitly, though it is also the card
  tool's default: the DK's `CONFIG_ENV_SIZE` (see
  `tools/mp2s_env_capacity.py`).
