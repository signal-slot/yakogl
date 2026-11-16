<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2-standalone/qemu-product: the DK product script in U-Boot v2023.10, under QEMU

A host-only, optional run of the merged product script `../mp2-standalone.cmd`
(built by `../build.sh` into `boot.scr.uimg`, the bytes the runner stages) in
real U-Boot v2023.10 on QEMU's `virt` machine, through **ST's distro loop**
(the st-env fixture's `bootcmd=run bootcmd_stm32mp` and the loop variables, byte for
byte). No board, no network. It reuses `../qemu/build-uboot.sh`,
`../qemu/image.S`, `../qemu/mkdisk.sh` and `../qemu/drive.py` without
changing them.

| file | job |
| --- | --- |
| `run.sh` | entry: checks its inputs, builds U-Boot (`../qemu/build-uboot.sh`), the product script (`../build.sh`) and the stand-in image (`../qemu/image.S`), runs `product.py` |
| `product.py` | the cases and their checks; imports `../qemu/drive.py` for the console driver and has its own case loop (drive.py's asserts that `boot_prefixes` never changes, and the product restores it) |

```sh
QEMU_SYSTEM_AARCH64=/path/to/qemu-system-aarch64 \
UBOOT_V2023_10_SRC=/path/to/u-boot-v2023.10 \
    driver/tools/mp2-standalone/qemu-product/run.sh
```

Last lines: one `case NAME: PASS|FAIL (N ok, M failed)` per case and
`product.py: N ok, M failed`; exit 0 only when every case passed. Each
assertion prints a `PASS|FAIL case: ...` line, and each boot prints its `mp2s:`/`mp2q:`
console lines and the variables the boot's saves persisted beyond `mp2s_*` and
`boot_prefixes` (`info:`).

- `QEMU_SYSTEM_AARCH64`: required, no PATH search. Unset: `SKIP: QEMU_SYSTEM_AARCH64`,
  exit 0. The Zephyr SDK host tools carry one
  (`zephyr-sdk-x86_64-hosttools-standalone-0.10.sh -y -d DIR`).
- `UBOOT_V2023_10_SRC`: unset, `$CONNECTED_DEPS_DIR/u-boot-v2023.10` when it
  exists; else `SKIP: UBOOT_V2023_10_SRC`, exit 0. Fetch it as
  `../qemu/README.md` says (`git clone --depth 1 -b v2023.10
  https://source.denx.de/u-boot/u-boot.git`).
- No cross compiler: `SKIP: CROSS_COMPILE`. Missing `python3`, `mke2fs`, `sfdisk`
  or `truncate`: `SKIP: <tool>`.
- `MP2S_PRODUCT_ONLY=NAME` runs one case; `MP2S_QEMU_KEEP=1` keeps the run
  directory (a failing run keeps it anyway) with the console log of every boot,
  `logs/<case>-setup.log` and `<case>-boot<n>.log`. The cache is `../qemu`'s
  (`MP2S_QEMU_CACHE`).

## What a case is

bootfs (partition 2 of `../qemu/mkdisk.sh`'s GPT disk) holds
`/yakogl/standalone/boot.scr.uimg` (the product script),
`/yakogl/standalone/default.bin` (the stand-in image: an arm64 Image header,
4 KiB file, `image_size` 0x2000; it prints `mp2q: image ran at its link
address 0xa0000000` and returns), and `/mmc0_boot.scr.uimg`, a stand-in for
ST's `/mmc0_extlinux/` that prints `mp2q: stock path /mmc0_ reached ...`
(the "stock marker": on the DK that is ST's extlinux and OpenSTLinux).

The **setup** boot types ST's variables from
`../st-env/stm32mp25-dk-initial-env.txt` (`bootcmd`, `altbootcmd`,
`bootcmd_stm32mp`, `env_check`, `distro_bootcmd`, `bootcmd_mmc0`, `mmc_boot`,
`scan_dev_for_boot_part`, `scan_dev_for_boot`, `scan_dev_for_extlinux`,
`scan_dev_for_scripts`, `boot_a_script`, `boot_extlinux`, `boot_scripts`,
`boot_syslinux_conf`, `scriptaddr`), the QEMU adjustments below, then the
case's presets, and `saveenv`. A `default 2` preset is what the runner's
`default` writes (section 7 of `docs/notes/stm32mp257f-dk-standalone.md`):
`boot_prefixes=/yakogl/standalone/ /mmc0_`, `mp2s_prefixes=/mmc0_`,
`mp2s_ver=1`, `mp2s_left=2`. Then **boot 1..n** are separate QEMU runs on the
same flash and disk, untouched; a `cut` boot is SIGKILLed on the console line
`mp2s: default /yakogl/standalone/default.bin, boots left <n>`, which the script prints
right after its counting `saveenv` returns.

After every QEMU run the **saved environment is read from the flash image**
(CRC-checked; a redundant pair is read too), so "unchanged" and "cleared" are
about the bytes U-Boot stored, not `printenv`.

## The cases and what each proves

| case | boots | proves |
| --- | --- | --- |
| `a_default_2` | 4 | (a) the record of `default <image> 2`: boot 1 `boots left 1`, load, `go`, the image; boot 2 `boots left 0`, the image; boot 3 `default used up, booting Linux`, then the stock marker on the same boot, with `boot_prefixes` saved back as `/mmc0_` and `mp2s_left`, `mp2s_ver`, `mp2s_prefixes` gone from flash; boot 4 never finds our script (stock only, no `mp2s:` line). The count is in flash after each boot (1, then 0) |
| `b_nodefault_absent` | 1 | (b) no `mp2s_left` while our prefix is still in `boot_prefixes`: `no default set, back to the distro loop`, stock on boot 1, and the script saved nothing (flash byte-equal to setup) |
| `b_nodefault_zero` | 1 | (b) `mp2s_left=0`: the used-up branch on boot 1, stock on the same boot, prefix restored, `mp2s_*` cleared |
| `b_nodefault_removed` | 1 | (b) what `nodefault` leaves (`boot_prefixes=/mmc0_`, no `mp2s_*`): our script is not on the path, stock on boot 1, flash unchanged |
| `d_cut_every_count` | 3 | (d) power cut after the counting `saveenv` on boots 1 and 2: the count still went 1, 0 in flash, nothing was loaded, and boot 3 is the stock path (N+1 = 3) |
| `d_cut_first_count` | 3 | (d) a cut on boot 1 only: boot 2 runs the image with `boots left 0`, boot 3 is the stock path |

(c) is checked in every case after every boot: `bootcmd`, `bootcount`,
`altbootcmd` (and `bootcmd_stm32mp`, `distro_bootcmd`, `bootlimit`,
`upgrade_available`) are byte-identical in the saved environment to what the
setup saved. ST's `bootcount` lives outside the env on the DK; the setup saves
`bootcount=0` so that the comparison is between two values, not two absences.

The console lines checked are section 7's contract and the script's own
`echo`s: the undo line, `boots left <n>`, `loaded 1000 bytes at 0xa0000000, RAM
footprint 0x2000 of the 0x10000000-byte window`, `go 0xa0000000`, `the image
returned, booting Linux`, `no default set, back to the distro loop`, `default
used up, booting Linux`, and the loop's `Found U-Boot script ...` and `SCRIPT
FAILED: continuing...`.

## Differences from the DK

- **QEMU's U-Boot, ST's loop text.** `qemu_arm64_defconfig` plus
  `../qemu/mp2-standalone.config`, not ST's stm32mp25 build. The loop is ST's,
  typed in from the fixture; the commands under it are mainline v2023.10's.
- `boot_device=mmc`, `boot_instance=0` are typed in (ST's U-Boot sets them
  from the boot ROM's boot context), so `bootcmd_stm32mp` takes the mmc path
  and sets `boot_targets=mmc0`.
- `env_check` (`env info -p -d -q`) prints `env`'s usage: QEMU's build has no
  `CMD_NVEDIT_INFO`, so the `if` fails and nothing is saved, as on the DK with a
  saved, non-default environment.
- `preboot='pci enum; mmc list'`: QEMU's SD host is on PCI and only `mmc list`
  probes it. ST's `mmc dev` alone does not. The DK's SDMMC needs no preboot.
- `scan_dev_for_efi` is replaced by `echo mp2q: efi step skipped` (QEMU's EFI
  loader would try `bootefi bootmgr`). It runs after the prefix loop has
  ended, so it changes nothing the cases check.
- The stock path is a script that prints a marker, not ST's extlinux (`sysboot`
  would need a kernel). Extlinux is still looked up first under
  `/yakogl/standalone/` (`boot_syslinux_conf`) and is absent, as on the DK.
- The stand-in image returns from `go`, so on boots 1 and 2 the loop goes on to the
  stock marker after `the image returned`. A Zephyr image never returns.
- The env is CFI flash (one copy), not ST's two copies on the microSD's
  `u-boot-env` partition. A cut lands after `saveenv` has returned, not in
  the middle of it (that is the `qemu-env-redundancy` item's case).
- No BL2 IWDG: a hang is bounded only by the harness timeouts.
