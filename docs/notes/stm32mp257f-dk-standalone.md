<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK: standalone boot from its own storage (design, 2026-11-13)

Status: **design, written on the host from ST's source. Nothing here has run on
the board.** Facts only the board can give are marked `TODO(board-env-read)`
(a read-only look at the board) or `TODO(board-run)` (the first run records
them). The "Board run" section at the end is empty until then.

Goal (plan #362, ROADMAP stage 5, issues #7 and #1281): the stage-5 criterion
"a standalone boot from its own storage" (`docs/standalone-boot.md`) on the
DK. A product powers on and ST's chain boots our Zephyr image with no host
and no network: TF-A v2.10 BL2/BL31, then OP-TEE 4.0, then ST's U-Boot
2023.10. That chain stays (pre-study `:645-661`, Q8 `:823`, Q13 `:828`). The DK
is a shared board, so every test form returns to OpenSTLinux without a
person.

This note has the shape of the Verdin and Pi 4 sections of
`docs/standalone-boot.md`: where the image lives, the product form, a counted
default, `forever` with a person, the undo, the watchdog, and why no step
strands the board. The DK's one-shot (`stm32mp257f-dk-uboot-boot.md`, the
typed `mp2z_*` route) is the test runner. This note is the product's boot and
the lab's counted default.

Sources:

- "ST/" is ST's U-Boot, tag `v2023.10-stm32mp-r3.1`
  (github.com/STMicroelectronics/u-boot), file:line. "ST .config" is that
  tag's `stm32mp25_defconfig` resolved by Kconfig (`make stm32mp25_defconfig`,
  no build). It is cited by option name.
- "meta-st/" is ST's Yocto layer meta-st-stm32mp, tag
  `openstlinux-6.6-yocto-scarthgap-mpu-v26.06.10`
  (github.com/STMicroelectronics/meta-st-stm32mp). Its U-Boot recipe applies
  `0001-v2023.10-stm32mp-r3.1.patch`
  (`recipes-bsp/u-boot/u-boot-stm32mp/`), which matches the tag above. Its
  `recipes-bsp/u-boot/stm32mp-extlinux/boot.scr.cmd` is the source of the
  board's stock `boot.scr.uimg`. Tag `v25.06.11` has the same file with the
  same line numbers. Only the names of the coprocessor variables differ
  (`m4fw_*` there, `coprofw_*` here). Which release the board was flashed from
  is unknown (pre-study, section 1).
- "pre-study" is `docs/notes/pre-study-stm32mp257f-dk.md`; "one-shot note"
  is `docs/notes/stm32mp257f-dk-uboot-boot.md`.

U-Boot is GPL. Everything below is facts and citations, and nothing of it is
copied.

## Summary

| | STM32MP257F-DK |
|---|---|
| The image | a file on the microSD's bootfs partition (ext4, `/boot` in Linux): `/boot/yakogl/standalone/default.bin` |
| What loads it | our U-Boot script `/boot/yakogl/standalone/boot.scr.uimg`, reached by ST's distro loop through one extra entry in `boot_prefixes`; it does `load`, then `go 0xa0000000` |
| `bootcmd` | **unchanged**. No ST file under `/boot` is written or renamed |
| One boot, then Linux | the typed one-shot of the one-shot note (`mp2z_*`), not this note |
| Default for N boots, then Linux by itself | `tools/board-stm32mp2-standalone.sh default <image> N` (N = 1-9) |
| Default forever (the product) | `default <image> forever`, only with `BOARD_PERSON_PRESENT=1` |
| Undo without Linux | at the U-Boot prompt: `setenv mp2s_left 0; saveenv; boot`. With no console: the card in a PC, delete `yakogl/standalone/boot.scr.uimg` from bootfs |

`default` reboots nothing and prints its exact undo before it changes
anything. `nodefault` undoes any default from Linux.

## 1. Where the image lives

**The microSD's bootfs partition.** The DK boots from the microSD. Its
partitions are fsbla1, fsbla2, metadata1, metadata2, fip-a, fip-b, u-boot-env
(512 KiB), bootfs (64 MiB ext4, mounted on `/boot`), vendorfs, rootfs and
userfs (pre-study `:246-250`). U-Boot already reads bootfs: the distro loop
finds ST's extlinux file there and loads `Image.gz` and the device trees from
it. A file there is in reach of the same `load` command, with the device and
partition numbers the loop has just used (section 2). bootfs has about 34 MiB
free (`tools/board-stm32mp2.sh:54`), which is enough for megabyte images.
The runner refuses an image larger than half the free space.

**The eMMC is never used.** The DK has an eMMC: ST's DK device tree enables
`sdmmc2` as a non-removable, 8-bit, `no-sd` device with its own `vdd_emmc`
supply (`ST/arch/arm/dts/stm32mp257f-dk.dts:785-800`); the microSD is
`sdmmc1` (`:766-783`). The eMMC carries ST's partition layout with the same
partition labels as the card, and its contents were never examined
(pre-study `:250-251`, `:270-271`). It is never written and never read for
this design, and nothing finds a partition by label.

**Numbering.** Neither Linux's nor U-Boot's device numbers are written into
anything:

- In Linux, bootfs is whatever is mounted on `/boot`
  (`findmnt -n -o SOURCE /boot`). Its disk must say it is an SD card
  (`/sys/block/<disk>/device/type` reads `SD`; an eMMC reads `MMC`). These are
  the rules of `tools/board-stm32mp2.sh`'s `dev_guard`
  (`tools/board-stm32mp2.sh:95-104`).
- In U-Boot the script uses `${devtype} ${devnum}:${distro_bootpart}`, the
  partition on which the distro loop found the script (section 2). U-Boot's
  own numbering of the card (`boot_instance`, which is `dev_seq()` of the
  booted SDMMC, `ST/arch/arm/mach-stm32mp/stm32mp2/cpu.c:208-221`) is never
  needed. ST's stock script calls the card `mmc0`
  (`meta-st/recipes-bsp/u-boot/stm32mp-extlinux/boot.scr.cmd:37-39`).
  `TODO(board-env-read)`: the live `boot_instance`.

**How Linux stages the file.** The method is the one the Verdin runner and
`board-stm32mp2.sh` use:

1. Copy the image to `/boot/yakogl/standalone/default.bin.part` and `sync`.
2. Rename it to `default.bin` and `sync` again.
3. Drop the page cache and read the file back from the card by sha256.

A mismatch removes the file, and nothing else changes. Linux reboots through
systemd, which unmounts or remounts bootfs read-only, so the ext4 journal is
on the card before U-Boot reads it.

## 2. The product form

### The route: one more prefix for ST's distro loop

ST's `bootcmd` runs `bootcmd_stm32mp`. For an mmc boot that does `run
env_check`, narrows `boot_targets` to the booted card and runs
`distro_bootcmd` (`ST/configs/stm32mp25_defconfig:28`;
`ST/include/configs/stm32mp25_st_common.h:26-40`). On a firmware bank other
than 0, U-Boot runs `altbootcmd` instead (`ST/common/autoboot.c:478-487`). For
the DK that is `run bootcmd_stm32mp` (`stm32mp25_st_common.h:43`), the same
path. For each bootable partition, the loop runs `scan_dev_for_boot`
(`ST/include/config_distro_bootcmd.h:540-553`). That does, **for each prefix
in `boot_prefixes`**, first `scan_dev_for_extlinux` (the file
`${prefix}${boot_syslinux_conf}`), then `scan_dev_for_scripts` (the files
`${prefix}boot.scr.uimg`, then `${prefix}boot.scr`)
(`config_distro_bootcmd.h:530-538`, `:504-511`, `:518-528`). A script is run
with `load ... ${scriptaddr} ${prefix}${script}; source ${scriptaddr}`
(`:513-516`). A script or an extlinux boot that returns prints `SCRIPT
FAILED: continuing...` or `EXTLINUX FAILED: continuing...` and the loop goes on
(`:510`, `:526`).

**What the board's saved environment holds.** ST's stock `boot.scr.uimg`
runs once, on a boot where extlinux was not found under the compiled-in
prefixes `/ /boot/` (`config_distro_bootcmd.h:494`, `:499`); the extlinux
files on the card are in `mmc0_extlinux/` and `mmc1_extlinux/`, not
`extlinux/`. The stock script then:

- sets `boot_prefixes` to `/mmc0_` on the card
  (`meta-st/.../boot.scr.cmd:31`, `:41-44`);
- sets `boot_syslinux_conf` to `extlinux/<board_name>_extlinux.conf`, or to
  `extlinux/extlinux.conf` when the first is not on the card (`:55-61`);
- prefixes `boot_extlinux` with the coprocessor and overlay steps (`:32`);
- sets `boot_targets` (`:35`) and deletes `bootfile` (`:64`);
- does **`env save`** (`:67`);
- then runs the loop again with the new values (`:77`).

From then on every boot finds `/mmc0_extlinux/<board>_extlinux.conf` first.
The root `/boot.scr.uimg` is not on the path again unless the environment is
reset to the defaults. `TODO(board-env-read)`: the live `boot_prefixes`,
`boot_syslinux_conf`, `boot_scripts`, `boot_extlinux`, `boot_targets` and the
listing of `/boot`.

**Our route.** `default` puts one entry **in front of** the saved
`boot_prefixes`:

```
boot_prefixes = /yakogl/standalone/ <the value it had, e.g. /mmc0_>
```

and stages our script as `/boot/yakogl/standalone/boot.scr.uimg`. The loop
then tries, in order:

1. `/yakogl/standalone/<boot_syslinux_conf>`: absent (the runner refuses if
   `yakogl/standalone/extlinux/` exists), so nothing happens.
2. `/yakogl/standalone/boot.scr.uimg`: **our script**. It counts, saves and
   loads the image (section 3), or `exit`s.
3. `/yakogl/standalone/boot.scr`: absent.
4. `/mmc0_extlinux/<board>_extlinux.conf`: ST's untouched extlinux, which
   boots **Linux**.

`exit` in a sourced script ends only that script: `do_exit` returns
`-r - 2` (`ST/cmd/exit.c:10-20`), the parser of the script's text returns on
it (`ST/common/cli_hush.c:3211-3228`, `:3279`), and the loop goes on. Hush
builds the `for` list once, at the loop's first pass
(`cli_hush.c:1824-1835`). So when our script changes `boot_prefixes` and
exits, the loop still goes on to `/mmc0_`, and ST's extlinux boots Linux on
that same boot. `TODO(board-run)`: the console shows our `exit`, `SCRIPT
FAILED: continuing...` and then ST's extlinux on the same boot (or in the
QEMU run of the `qemu-run` item first).

What makes this the right route:

- **`bootcmd` is unchanged**, and so are `bootcmd_stm32mp`, `altbootcmd`,
  `distro_bootcmd` and every ST file under `/boot`. The rule of
  `tools/board-stm32mp2.sh:33-40` holds without an owner decision.
- **Everything ST's boot needs stays in place.** If our script is missing,
  corrupt (`source` checks the legacy image's magic and header CRC, and its
  data CRC unless `verify` is off: `ST/boot/image-board.c:1014`,
  `:1023-1037`), or exits, the next prefix
  is ST's, unchanged, and boots Linux on the same boot.
- **One ST variable is written: `boot_prefixes`.** It is written only as our
  entry in front of the exact value the runner read, and it is put back to
  that value byte for byte, by our script when the count runs out and by
  `nodefault`. ST's own script writes the same variable and saves it
  (`boot.scr.cmd:31`, `:67`). This is not `bootcmd`. The runner records the
  value at `default` and refuses when the live value does not match the
  record at `nodefault` time (section 5).
- **No file of ST's is replaced or renamed.** The undo of "the card in a PC"
  is to delete one file of ours (section 5).
- **A person can always stop it.** The loop runs inside `bootcmd`, after the
  autoboot window (`ST/common/autoboot.c:500-510`, `CONFIG_BOOTDELAY=1`,
  `ST/configs/stm32mp25_defconfig:27`).

The script is a legacy `mkimage -A arm64 -O linux -T script -C none` image,
like ST's (`boot.scr.cmd:1-2`). `source` takes the legacy format because
`CONFIG_LEGACY_IMAGE_FORMAT=y` (`stm32mp25_defconfig:25`;
`ST/boot/image-board.c:1017-1018`).

### The product, on a card of its own

A product card is the same: ST's partitions and bootfs, ST's U-Boot
environment saved with `boot_prefixes = /yakogl/standalone/ /mmc0_` and
`mp2s_left=forever`, and the two files under `yakogl/standalone/`. Linux's
extlinux may stay as the fallback the loop reaches when the image is missing.
A product that must never boot Linux removes ST's `mmc0_extlinux/` from its
own card. That is a product-image decision, never a step on the shared
board.

### Rejected alternatives

| alternative | why not |
|---|---|
| **Replace the stock `/boot.scr.uimg` with ours** (the stock file kept beside it) **and rename ST's extlinux.conf** so that the loop falls through to the script. When the count reaches 0, the script does `sysboot ... any ${scriptaddr} <the renamed conf>` | (1) The root `/boot.scr.uimg` is not on the board's path: once the stock script has saved `boot_prefixes=/mmc0_` (`boot.scr.cmd:31`, `:41-44`, `:67`), the loop looks only for `/mmc0_extlinux/...`, `/mmc0_boot.scr.uimg` and `/mmc0_boot.scr`. (2) It writes two of ST's files. (3) It strands the board: with ST's extlinux renamed, a missing or corrupt script leaves the loop with nothing to boot. `distro_bootcmd` ends, and U-Boot sits at the prompt, where its own cyclic watchdog servicing may keep it alive (one-shot note, section 9). (4) The fallback would have to repeat the coprocessor and overlay steps ST's script put into `boot_extlinux` (`boot.scr.cmd:32`). In our route ST's own extlinux does all of that |
| The same with **`/mmc0_boot.scr.uimg`** (the prefix the board uses) | It still needs ST's extlinux renamed, because extlinux is tried before scripts for each prefix (`config_distro_bootcmd.h:533-535`). Points (2)-(4) above still apply |
| **An extlinux label** in ST's extlinux file | A label boots through `bootm`, `booti` or `bootz` only (`ST/boot/pxe_utils.c:749-757`). `booti` moves the image off its link address (one-shot note, section 3), there is no `go`, and there is no counter. It also edits ST's file |
| **`preboot`** (`CONFIG_USE_PREBOOT=y`, ST .config) | `preboot` runs before the autoboot window (`ST/common/main.c:22-27`, `:53`, then `:64-68`). A `forever` image would then leave a person no key press to stop it. ST's `cpu.c` also writes `preboot` at run time for fastboot, UMS and recovery (`ST/arch/arm/mach-stm32mp/stm32mp2/cpu.c:250-269`) |
| **A hook in `bootcmd`** (`board-stm32mp2.sh`'s `yako_hook`) | It writes `bootcmd`, which the runner never does (`tools/board-stm32mp2.sh:33-40`), and needs a person to put it in |
| **The typed one-shot** (one-shot note) | It needs a host on the console at every boot, so it is not standalone |

## 3. The counted default

### The variables

All of our variables have the prefix `mp2s_`. They are disjoint from the
one-shot's `mp2z_*`, from the old runner's `yako_*`, and from U-Boot's and
ST's names.

| variable | written by | meaning |
|---|---|---|
| `mp2s_ver` | the runner (`default`) | the contract version, `1`. The script refuses any other value and exits, and then Linux boots |
| `mp2s_left` | the runner (`default`), the script (decrement, delete) | `1`..`9`: the number of boots left; `forever`: every boot; absent: no default |
| `mp2s_prefixes` | the runner (`default`) | the exact `boot_prefixes` value before `default`, restored when the default ends |
| `boot_prefixes` | the runner (`default`, `nodefault`), the script (when used up) | ST's variable. It is set to `/yakogl/standalone/ ${mp2s_prefixes}` only while a default is set, and otherwise to `mp2s_prefixes` |
| `mp2s_load`, `mp2s_window`, `mp2s_min`, `mp2s_ok`, `mp2s_size`, `mp2s_chk`, `mp2s_imgsz` | the script, in RAM | the load and its checks (the one-shot's `mp2z_*` checks under our prefix). They are set **after** the save, so they are never saved |

The runner writes `mp2s_ver`, `mp2s_left`, `mp2s_prefixes` and `boot_prefixes`
in **one** environment write. There is never a moment with the prefix but no
count.

**Never touched:** `bootcount`, `bootlimit`, `upgrade_available`,
`altbootcmd`, `bootcmd`, `bootcmd_stm32mp`, `boot_targets`,
`boot_syslinux_conf`, `boot_extlinux`, `boot_scripts`, `preboot`, `mp2z_*`
and `yako_*`. In this U-Boot, `bootcount`, `bootlimit` and `upgrade_available`
do nothing anyway: `CONFIG_BOOTCOUNT_LIMIT` is not set (ST .config;
`ST/drivers/bootcount/Kconfig:5` has no default), so `bootcount_inc()` and
`bootcount_error()` are empty stubs (`ST/include/bootcount.h:126-136`) and
`upgrade_available` is read only by the bootcount drivers
(`ST/drivers/bootcount/bootcount_env.c`, `bootcount_ext.c`,
`bootcount_nvmem.c`), which are not built. `altbootcmd` exists
(`stm32mp25_st_common.h:43`). It is chosen by the firmware-update boot index,
not by a count (`autoboot.c:478-487`, `CONFIG_FWU_MULTI_BANK_UPDATE=y`,
`stm32mp25_defconfig:194`). Our count leaves all of these alone.

### The script, in order

1. Print the undo line (section 7).
2. With no `mp2s_left`, print `no default set`, then `exit`. The loop goes on
   to ST's extlinux.
3. With `mp2s_ver` other than `1`, refuse and `exit`.
4. With `mp2s_left = forever`, print `default <file>, forever` and go to
   step 7. No count and no save.
5. With `mp2s_left -gt 0`:
   - `setexpr mp2s_left ${mp2s_left} - 1`;
   - **`saveenv`**;
   - only if the save succeeded, print `default <file>, boots left
     ${mp2s_left}` and go to step 7;
   - if the save failed, print `saveenv failed, booting Linux` and `exit`.
   
   **The count is decremented and saved before anything is loaded.**
6. Otherwise, the count is 0 or not a number:
   - print `default used up, booting Linux`;
   - `setenv boot_prefixes "${mp2s_prefixes}"`;
   - delete `mp2s_left`, `mp2s_prefixes` and `mp2s_ver`;
   - `saveenv`;
   - `exit`.
   
   The loop goes on to ST's prefix on this same boot. If that `saveenv`
   fails, Linux still boots, and the next boot takes this branch again.
7. Size the image, load it, check it (size, the ARM64 magic at byte 56, the
   header's `image_size` inside the window), then `dcache flush; dcache off;
   icache off; go 0xa0000000`. The checks are the same as in
   `driver/tools/uboot-mp2/mp2-mmc.cmd`. The commands are `size` and `load`
   on `${devtype} ${devnum}:${distro_bootpart}` (`ST/cmd/fs.c:18-19`). A
   refusal prints `refused: <why>, booting Linux` and `exit`s. A count was
   then still used up, which is the safe direction.

**Why 1-9.** `setexpr` reads its operands as hexadecimal
(`ST/cmd/setexpr.c:97`) and writes its result with `%lx`
(`ST/cmd/setexpr.c:509`, `ST/env/common.c:68-73`). `test -gt` reads base 0,
that is decimal for these values (`ST/cmd/test.c:133-135`). For one digit
they agree. A value that is not a number reads as 0 in `test` and so takes the
"used up" branch, which restores ST's prefix.

**`saveenv` and the redundant copy.** The environment is on the card that
booted (`ST/board/st/stm32mp2/stm32mp2.c:540-553`), in two copies
(`CONFIG_SYS_REDUNDAND_ENVIRONMENT=y`, `stm32mp25_defconfig:72`). Each save
writes the copy that is not in use, with the flag byte one higher
(`ST/env/mmc.c:264-309`, `ST/env/common.c:424`). At load U-Boot takes the
copy with the higher flag (`ST/env/mmc.c:365-371`). If the power fails during
our save, the older copy, the one with the count one higher, stays valid. That
costs at most one extra boot of the image, and the count is still bounded.
`tools/uboot_env.py` writes the runner's edit the same way, to the inactive
copy with the flag plus one (`tools/uboot_env.py:33-38`).

**What `saveenv` stores besides ours.** It writes the whole RAM environment,
including the loop's working variables at the moment of the save
(`devtype`, `devnum`, `distro_bootpart`, `distro_bootpart_uuid`, `bootfstype`,
`devplist`, `prefix`, `script`, `target`, `filesize`, `fileaddr`). ST's own
script saves at the same point of the loop (`boot.scr.cmd:67`), so the stored
environment already holds these names since the board's first boot. The loop
sets each of them before it uses it (`config_distro_bootcmd.h:28-43`,
`:533`, `:541-549`, `:519`). `TODO(board-run)`: the runner's before/after
`fw_printenv` diff. The expected result is that `boot_prefixes` and every
`bootcmd*` are back as recorded, there are no `mp2s_*`, and the only other
differences are among the names in this paragraph.

**`env_check`.** `bootcmd_stm32mp` runs `env_check` before the loop
(`stm32mp25_st_common.h:31`). It is `if env info -p -d -q; then env save; fi`
(`ST/include/configs/stm32mp25_common.h:94`), and it succeeds only when the
environment in use is the compiled-in default **and** can be persisted
(`ST/cmd/nvedit.c:1116-1146`). With a valid stored environment it does
nothing. With both copies bad, U-Boot runs on the defaults, and `env_check`
saves them:

- our prefix and `mp2s_*` are gone, so the default is cancelled;
- `boot_prefixes` is `/ /boot/` again, so ST's root `boot.scr.uimg` runs,
  saves ST's values again and boots Linux (`boot.scr.cmd:31-77`).

That fails toward Linux. The runner refuses to write when either copy's CRC
fails (the rule of `tools/board-stm32mp2.sh:65-66`).

### What the runner checks before `default`

- `install` was done and the staged script's sha256 matches this runner's
  build.
- No default is set (`mp2s_left` absent), and the one-shot runners' `yako_*`
  and `mp2z_*` are absent.
- Both environment copies' CRCs check, and `boot_prefixes` is present in the
  stored environment. A stored environment holds all variables, defaults
  included (`env_export`), so it is always present unless the environment is
  the default one.
- **ST's firmware update is not in its trial state.** With
  `CONFIG_FWU_MULTI_BANK_UPDATE=y`, U-Boot counts every boot while the booted
  bank is "valid but not accepted" (`bank_state` 0xFE,
  `ST/include/fwu.h:84-86`; `ST/lib/fwu_updates/fwu_v2.c:19-21`), and after
  `CONFIG_FWU_TRIAL_STATE_CNT` boots (3, `ST/lib/fwu_updates/Kconfig:26-29`)
  it reverts ST's active bank (`ST/lib/fwu_updates/fwu.c:74-95`, `:740-742`).
  N image boots that never reach Linux would spend ST's trial. The runner
  reads the metadata partition read-only and refuses unless the booted bank
  is accepted (0xFC). `TODO(board-env-read)`: the metadata's `bank_state` on
  the board. The trial counter is an EFI variable in `ubootefi.var` on the
  EFI system partition (`ST/lib/efi_loader/efi_var_file.c:140-144`,
  `ST/lib/efi_loader/efi_disk.c:531-535`); whether that is bootfs is
  `TODO(board-env-read)`, and the `/boot` sha256 comparison would show it.

## 4. `forever`: only with a person

`default <image> forever` writes `mp2s_left=forever`: the image boots at
every boot and nothing returns the board to Linux by itself. The runner
refuses unless `BOARD_PERSON_PRESENT=1`, with the message `board.sh` gives the
Verdins: give a count (1-9) and the board comes back by itself, or set
`BOARD_PERSON_PRESENT=1` when someone is at the board. It also refuses an image
without the product watchdog's marker (section 6) unless `MP2_NO_WATCHDOG=1`
says so on purpose. Without that marker, the image resets within about 32 s
and `forever` becomes a boot loop. Only a person at the console or with the
card ends `forever` (section 5).

## 5. The undo

`default` prints the undo before it changes anything. `install` records the
sha256 of every file under `/boot`, keeps the pristine environment image on
the host and on the card (as `board-stm32mp2.sh:68-75` does). `default`
records the live `boot_prefixes` on the host (the "default record") before
it writes the value into `mp2s_prefixes`.

| state | from Linux (the runner) | without Linux |
|---|---|---|
| installed, no default | `uninstall`: remove `/boot/yakogl/standalone/` and compare `/boot` with the install record | nothing to undo: without our prefix in `boot_prefixes`, the files are never read |
| `default <image> N` | `nodefault` (before the reboot, or once Linux is back) | **none needed**: after N boots the script restores `boot_prefixes`, deletes `mp2s_*`, saves and boots Linux |
| `default <image> forever` | `nodefault` once Linux is back | **a person at the U-Boot prompt** (a key at "Hit any key to stop autoboot", then): `setenv mp2s_left 0; saveenv; boot`, and the script then restores everything itself. Or, not relying on our script: `setenv boot_prefixes '<the value default printed>'; setenv mp2s_left; setenv mp2s_prefixes; setenv mp2s_ver; saveenv; boot`. **With no console: the card in a PC** (ext4): delete or rename `yakogl/standalone/boot.scr.uimg` on bootfs. The prefix then finds nothing, and ST's extlinux boots Linux. Then `nodefault` from Linux |
| a default whose count ran out | `nodefault` removes `default.bin` (the environment is already restored) | nothing |

What `nodefault` does: one environment write. If `boot_prefixes` still
begins with `/yakogl/standalone/ ` and `mp2s_prefixes` is set, it sets
`boot_prefixes` back to `mp2s_prefixes` and checks that the value equals the
default record byte for byte. Otherwise it leaves `boot_prefixes` alone,
because ST's script or ours has already set it. It then deletes `mp2s_*` and
reads everything back with `fw_printenv`. If the read-back differs, it writes
the pre-edit environment image back at once. Finally it removes
`default.bin`. It is safe to repeat.

**The stock files, byte for byte.** No step writes, renames or replaces any
of ST's files under `/boot`: `Image.gz`, the device trees, `boot.scr.uimg`,
`mmc0_extlinux/` and `mmc1_extlinux/`. The only files written are under
`/boot/yakogl/standalone/`. `status`, `nodefault` and `uninstall` compare the
sha256 of every file under `/boot` with the install record. The only
difference may be `yakogl/standalone/`, which is gone after `uninstall`.
`TODO(board-env-read)`: whether `ubootefi.var` is on bootfs (section 3). If a
stock file differs, the runner stops with NEEDS A PERSON and prints the list.
It never "repairs" ST's files. The pristine environment image is a person's
last resort (written back with `dd` from Linux, or with the card in a PC). The
runner never writes it back once U-Boot has saved after it, because the
copies' flags would then go backwards.

## 6. How a hang is bounded

- **BL2 starts the IWDG**, so it runs when U-Boot starts. OP-TEE serves it to
  the non-secure side as `arm,smc-wdt` with `arm,smc-id = <0xbc000000>`
  (`ST/arch/arm/dts/stm32mp251.dtsi:78-80`), enabled with `timeout-sec = <32>`
  on the DK (`stm32mp257f-dk.dts:216-219`). A hang resets the board in about
  32 s, or about 27 s after an OP-TEE panic (pre-study `:252-259`). The IWDG
  nodes themselves are disabled in the non-secure tree
  (`stm32mp251.dtsi:1963-1981`).
- **U-Boot's part.** The board-read says U-Boot has no `wdt` support
  (pre-study `:260-261`). The source has `CONFIG_WDT`, `CONFIG_WDT_STM32MP` and
  `CONFIG_WDT_ARM_SMC` (`stm32mp25_defconfig:183-185`), and they resolve to
  `CONFIG_WATCHDOG=y` and `CONFIG_CYCLIC=y` without `CONFIG_CMD_WDT` (ST
  .config). The SMC driver takes over a counting watchdog
  (`ST/drivers/watchdog/arm_smc_wdt.c:104-109`) and pets it from U-Boot's
  cyclic loop (`ST/drivers/watchdog/wdt-uclass.c:138`, `:151`). The one-shot
  note's section 9 has the two cases. For this design:
  - the distro loop, our script and the `load` run inside U-Boot, either
    petted or bounded by the 32 s;
  - **after `go` nothing in U-Boot runs**, so the image is bounded by the
    watchdog unless it pets it.
- **With `mp2_hello`** (the board-run image), each counted boot ends in a
  reset:
  - the image's PSCI `SYSTEM_RESET` at its 20 s lifetime at the latest
    (`tools/probes/stm32mp257f_dk/mp2_hello/README.md:27`, `:70-71`);
  - or, if it hangs, the IWDG about 32 s after the last pet.
  
  That reset is what starts the next boot. The count does not depend on how
  the image ends, because it was saved before the load. `mp2_hello` arms no
  watchdog of its own (its README `:27`), so its runs need `MP2_NO_WATCHDOG=1`
  as in `stm32mp257f-dk-bringup.md`'s step 1-2 rows. Images with the board
  layer's watchdog (`gc_wdog_arm: armed`) pet it every second until their
  lifetime ends (`driver/zephyr/lib/board/stm32mp2/stm32mp2_wdog.c:12-28`).
- **A product image that runs indefinitely** must refresh the watchdog
  itself, through OP-TEE's SMC watchdog service. The pre-study's reading of
  OP-TEE (`:258-259`) gives the rules:
  - INIT first;
  - timeouts of 6 s or more;
  - ENABLE with 1, never ENABLE with 0;
  - then PET from a context that runs only while the application is healthy;
  - no lifetime reset (`CONFIG_YAKOGL_STM32MP2_LIFETIME_S` off for a product).
  
  Today's `stm32mp2_wdog.c` sends SET_TIMEOUT, ENABLE 1 and PET but no INIT
  (`:17`, `:27-28`), and it stops petting at the lifetime. The indefinite
  refresh, with a marker the runner can check for `forever`, belongs to
  `driver/rust/src/soc/stm32mp2/**` (plan #362's `iwdg-refresh` item, ordered
  after #349). Until it exists, `forever` with any of our images is a boot
  loop of 20-32 s that only a person ends.

## 7. The interface contract

For the `scripts` item (`driver/tools/mp2-standalone/`) and the `runner` item
(`tools/board-stm32mp2-standalone.sh`).

### Files on bootfs

| path in Linux | path in U-Boot | written by | what |
|---|---|---|---|
| `/boot/yakogl/standalone/boot.scr.uimg` | `/yakogl/standalone/boot.scr.uimg` | `install` | our script, `mkimage -A arm64 -O linux -T script -C none -d mp2-standalone.cmd` |
| `/boot/yakogl/standalone/boot.scr.uimg.sha256` | | `install` | its sha256 |
| `/boot/yakogl/standalone/default.bin` | `/yakogl/standalone/default.bin` | `default` | the image (`.part` then rename) |
| `/boot/yakogl/standalone/backup/u-boot-env.bin` | | `install` | the pristine environment image |
| `/boot/yakogl/standalone/boot-sha256.txt` | | `install` | the sha256 of every file under `/boot` |

The file names are fixed in the script; there is no file-name variable.
Nothing is ever named under `mmc0_*`, `mmc1_*` or `extlinux/`, and nothing is
written at bootfs's root. `yakogl/standalone/` is apart from the one-shot's
`yakogl/<token>.scr` and `board-stm32mp2.sh`'s `yakogl/backup/` and
`once-*.bin`.

### Variables

Section 3's table. The script's lint (the `scripts` item's selftest)
allows:

- the commands `echo`, `setenv`, `test`, `itest`, `setexpr`, `size`, `load`,
  `saveenv`, `dcache`, `icache`, `go` and `exit`;
- `setenv` of `mp2s_*` and `boot_prefixes` only;
- `boot_prefixes` set only from `${mp2s_prefixes}`;
- `saveenv` exactly twice: once after the decrement, once in the used-up
  branch.

It fails on `bootcmd`, `altbootcmd`, `bootcount`, `bootlimit`,
`upgrade_available`, `boot_targets`, `preboot`, `mp2z_`, `yako_`, `env
default`, `eraseenv`, `mmc write`, `ext4write`, `fatwrite`, `gpt` and `run`.

### Console lines

Each line starts with `mp2s: ` and is one line. The runner's console scan and
the board record read them:

```
mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: delete yakogl/standalone/boot.scr.uimg)
mp2s: no default set, back to the distro loop
mp2s: refused: contract version <v>, expected 1, booting Linux
mp2s: default /yakogl/standalone/default.bin, boots left <n>      <- after the save, before the load
mp2s: default /yakogl/standalone/default.bin, forever
mp2s: saveenv failed, booting Linux
mp2s: default used up, booting Linux
mp2s: refused: <why>, booting Linux
mp2s: loaded <size> bytes at 0xa0000000, RAM footprint 0x<n> of the 0x10000000-byte window
mp2s: go 0xa0000000
mp2s: the image returned, booting Linux
```

`boots left <n>` always comes before the load. A record of `default <image> 2`
reads:

1. `boots left 1`, the image, its reset;
2. `boots left 0`, the image, its reset;
3. `default used up, booting Linux`, ST's extlinux, OpenSTLinux.

### Runner verbs

These are in the shape of `tools/board.sh`'s Verdin verbs
(`uboot_default`, `uboot_nodefault`, `default_undo`, `cmd_default`).

| verb | refuses when | writes | prints first (the undo) |
|---|---|---|---|
| `install` | a CRC fails, `/boot` is not the SD's bootfs, `yakogl/standalone/` holds anything but our files | the files of the table above except `default.bin`; **no environment write** | `UNDO: tools/board-stm32mp2-standalone.sh uninstall` |
| `default <image> <N\|forever>` | not installed or script sha256 differs; a default set; `yako_once`/`mp2z_*` present; image check fails (header, window, size, marker); FWU trial state; `forever` without `BOARD_PERSON_PRESENT=1`; N not 1-9 | `default.bin`, then one environment write: `mp2s_ver=1`, `mp2s_left=N\|forever`, `mp2s_prefixes=<live>`, `boot_prefixes=/yakogl/standalone/ <live>`; read back with `fw_printenv`, the pre-edit image back on a mismatch. Reboots nothing | `UNDO, from Linux: ... nodefault`; for N: `UNDO, without Linux: none needed: after N boots the script restores boot_prefixes and boots Linux`; for `forever`: the two person lines of section 5 |
| `nodefault` | a CRC fails; the live `mp2s_prefixes` differs from the default record | section 5 | `nothing to undo: nodefault only removes` |
| `uninstall` | a default set (it says: `nodefault` first) | removes `/boot/yakogl/standalone/`, then the `/boot` comparison | `UNDO: tools/board-stm32mp2-standalone.sh install` |
| `status` | | nothing | |

Messages end the way the Verdins' do: `default set for N boots: U-Boot loads
/yakogl/standalone/default.bin at each of the next N boots, then restores
boot_prefixes and boots Linux. Nothing was rebooted.`

Every verb takes the board lock and runs `dev_guard` before each write, and
it records what it is about to leave behind in a state file beside the lock
(`board-stm32mp2.sh:82-93`). The one-shot runners (`board-stm32mp2.sh run`,
`board-stm32mp2-uboot.sh`) must refuse while `mp2s_left` is set: the boot
after their image would not be Linux. Those files belong to plan #357 and
#349, so that is new work, reported, not done here.

## 8. Why no step strands the board

| what fails | what happens |
|---|---|
| staging or its read-back | the file is removed; the environment is not touched; no reboot |
| the runner's environment write or its `fw_printenv` read-back | the pre-edit image goes back at once; no reboot |
| both environment copies bad at boot | U-Boot uses the defaults and `env_check` saves them; our prefix is gone; ST's root `boot.scr.uimg` restores ST's values and boots Linux |
| our script missing, corrupt, or the wrong version | `SCRIPT FAILED: continuing...`; the next prefix is ST's; Linux |
| `saveenv` in the script | nothing is loaded; `exit`; Linux |
| a power cut during that `saveenv` | the older copy stays valid: at most one extra boot, then the count goes on |
| `size`/`load`/a check | `refused: ..., booting Linux`; one boot used |
| the image hangs | the watchdog resets the SoC in about 32 s; the count is already lower; Linux at the latest after N boots |
| the image returns | `the image returned, booting Linux`; the loop goes on |
| the count reaches 0 | the script restores `boot_prefixes` and boots Linux on the same boot; if that save fails, Linux boots anyway and the next boot repeats it |
| a firmware update in trial | the runner refuses `default` (section 3) |
| the host dies after `default` | nothing on the boot path needs the host |
| `forever` | only with a person, who has two undos without Linux (section 5) |

**Board constraints**, from `board-stm32mp2.sh:95-109` and the one-shot
note's section 10:

- Never write `fsbla1`, `fsbla2`, `metadata1`, `metadata2`, `fip-a` or `fip-b`.
  TF-A, OP-TEE and U-Boot live there, and the FWU metadata is only read.
- Never write `vendorfs`, `rootfs` or `userfs`: **the rootfs is left alone**.
- **Never write the eMMC.**
- Never repartition (`gpt`, `part`, `sgdisk`) or create a file system.
- Write only two things: the two copies of `u-boot-env` (our variables and
  `boot_prefixes` as above), and files under `/boot/yakogl/standalone/`.
- No module load or unload, and never touch Tailscale or sshd.
- Board addresses, host names, UART paths, serials and credentials come only
  from the environment (`BOARD_HOST`, `BOARD_UART`). This note has none.
- **A result that looks like a hardware fault is checked against the USB-C
  supply first.** BL2's `Reset reason:` line is the discriminator
  (`stm32mp257f-dk-bringup.md`, "When it looks like a hardware fault: supply
  first"):
  - `Power-on reset` or `Brownout reset` means the supply;
  - `IWDG system reset` means the watchdog, which is expected after a hang.

## What the board has to answer (index of the placeholders)

| marker | what to read or record |
|---|---|
| `TODO(board-env-read)` | `fw_printenv boot_prefixes boot_syslinux_conf boot_scripts boot_extlinux boot_targets boot_instance bootcmd`; the listing and sha256 of `/boot` (section 2) |
| `TODO(board-env-read)` | the FWU metadata's `bank_state` and the booted bank, read-only; whether `ubootefi.var` is on bootfs (section 3) |
| `TODO(board-run)` | one boot showing `mp2s: ...`, `SCRIPT FAILED: continuing...`, then ST's extlinux on the same boot (section 2) |
| `TODO(board-run)` | the `fw_printenv` diff before `default` and after the count ran out (section 3) |

## Board run

*(Empty: filled in by plan #362's `board-run` item on the three DK resources.
No board address, host name, UART path, serial or credential.)*

What goes here:

1. **The image and the build:** `mp2_hello` (built as
   `tools/probes/stm32mp257f_dk/mp2_hello/README.md` says, run with
   `MP2_NO_WATCHDOG=1`), its sha256 and size; the commit of
   `driver/tools/mp2-standalone/` and its script's sha256.
2. **Before:** `fw_printenv` (the `TODO(board-env-read)` names above), the
   `/boot` listing and sha256, and the FWU bank state.
3. **`default <image> 2`:** what the runner printed (the undo lines first),
   the read-back, and that nothing was rebooted.
4. **The console from the reboot on:**
   - `mp2s: default ..., boots left 1`, the image's lines and its reset (BL2
     `Reset reason:`);
   - `boots left 0`, the image and its reset;
   - `default used up, booting Linux`, `SCRIPT FAILED: continuing...`, ST's
     extlinux, OpenSTLinux.
5. **After:** `fw_printenv` and its diff against the before (`boot_prefixes`
   back, no `mp2s_*`), the `/boot` sha256 comparison, then `nodefault` and
   `uninstall`.
6. **Failures:** each boot's `Reset reason:`, the supply check, and the undo
   taken. A new defect gets a new issue text, not a line here.
