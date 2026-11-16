<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK: standalone boot from its own storage (design, 2026-11-13)

Status: **design, written on the host from ST's source. Nothing here has run on
the board.** Facts only the board can give are marked `TODO(board-env-read)`
(a read-only look at the board) or `TODO(board-run)` (the first run records
them). The "Board run" section at the end is empty until then.

Merged since the design: the product script and its selftest
(`driver/tools/mp2-standalone/`), the QEMU harness, ST's default environment
as a fixture, the runner's safety core (`tools/board-stm32mp2-standalone.sh`:
its payload is **not wired**, `PAYLOAD_READY=0`, so `install`, `default` and
`nodefault` refuse with exit 3 after their checks), the read-only helpers
(`tools/mp2s_*`, `tools/efi_varfile.py`, `tools/uboot_env.py`), the
`mp2_wdt` probe and the `mp2_product` image (their READMEs). Tools still in
review appear as `(pending)` placeholders.

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
| Undo without Linux | at the U-Boot prompt: `setenv mp2s_left 0; saveenv; boot`. With no console: the card in a PC, `tools/mp2s_card_undo.sh --yes <bootfs partition>` renames `yakogl/standalone/boot.scr.uimg` to `boot.scr.uimg.off` |

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
6. Otherwise, the count is 0 or not a number (anything but `forever` or
   1-9):
   - with `mp2s_prefixes` empty, print `refused: mp2s_prefixes is not set,
     boot_prefixes left as it is, booting Linux` and `exit`, with nothing
     changed (an empty `boot_prefixes` would leave the loop nothing to boot).
     Linux boots on this boot, and every later boot refuses the same way until
     `nodefault`;
   - print `default used up, booting Linux`;
   - `setenv boot_prefixes "${mp2s_prefixes}"`;
   - delete `mp2s_left`, `mp2s_prefixes` and `mp2s_ver`;
   - `saveenv`;
   - `exit`.
   
   The loop goes on to ST's prefix on this same boot. If that `saveenv`
   fails, Linux still boots, and the next boot takes this branch again.
7. Size the image, load it, check it, then `dcache flush; dcache off;
   icache off; go 0xa0000000`. In order: `devtype`, `devnum` and
   `distro_bootpart` set; `size` finds `/yakogl/standalone/default.bin`; the
   file is at least 0x40 bytes and at most the 0x10000000-byte window; `load`
   succeeds and `filesize` is the sized value; the ARM64 magic at byte 56; the
   header's `image_size` (64 bits at byte 16) below 4 GiB and inside the
   window. The checks are the same as in
   `driver/tools/uboot-mp2/mp2-mmc.cmd` (`tools/mp2s_image_check.py` makes them
   on the host). The commands are `size` and `load`
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

In the order the runner makes them. "Skeleton" is what
`tools/board-stm32mp2-standalone.sh` does today; the rest is the payload, which
calls the helpers named (all read-only):

- **Skeleton: the arguments.** `<image>` and `<N>` are given; N is 1-9 or
  `forever`, else exit 2. `forever` without `BOARD_PERSON_PRESENT=1` exits 2
  (section 4).
- **The image.** `tools/mp2s_image_check.py <image> --free-bytes <bootfs free>`
  (exit 2 on a refusal): at least 0x40 bytes, at most the window, at most half
  of bootfs's free bytes, the ARM64 magic at byte 56, the header's
  `image_size` inside the window. For `forever` add `--require-marker
  'mp2_wdtr: product refresh armed'` (section 6). The skeleton's own
  `check_image` is a cut-down copy of these checks (section 7, "Runner verbs").
- **`install` was done and the staged script is ours**: the staged script's
  sha256 equals `driver/tools/mp2-standalone/build.sh`'s output for this
  commit (the payload; `install` does not stage it yet).
- **No other route is armed.** `tools/mp2s_guard.py --for standalone` exits 3
  while any `yako_*` or `mp2z_*` is in the environment or `boot_prefixes`
  already contains `/yakogl/standalone/`, and 2 when the environment cannot be
  read. A second `default` is refused by the runner's own `fw_printenv
  mp2s_left` check, because the guard allows `mp2s_*`.
- **Both environment copies' CRCs check** (`tools/uboot_env.py check`) and
  `boot_prefixes` is present in the stored environment. A stored environment
  holds all variables, defaults included (`env_export`), so it is always
  present unless the environment is the default one.
- **The edit fits.** `tools/mp2s_env_capacity.py --image <env image>
  mp2s_ver=1 mp2s_left=<N> 'mp2s_prefixes=<live>'
  'boot_prefixes=/yakogl/standalone/ <live>'` exits 0: both copies hold the
  edit with the default margin of 256 bytes (exit 1: does not fit; 2:
  unreadable).
- **ST's firmware update is not in its trial state.** With
  `CONFIG_FWU_MULTI_BANK_UPDATE=y`, U-Boot counts every boot while the booted
  bank is "valid but not accepted" (`bank_state` 0xFE,
  `ST/include/fwu.h:84-86`; `ST/lib/fwu_updates/fwu_v2.c:19-21`), and after
  `CONFIG_FWU_TRIAL_STATE_CNT` boots (3, `ST/lib/fwu_updates/Kconfig:26-29`)
  it reverts ST's active bank (`ST/lib/fwu_updates/fwu.c:74-95`, `:740-742`).
  N image boots that never reach Linux would spend ST's trial. The runner
  reads the metadata partition read-only and refuses unless the booted bank
  is accepted (0xFC): `tools/fwu_metadata.py` (pending, item `fwu-meta`).
  `TODO(board-env-read)`: the metadata's `bank_state` on the board. The trial
  counter is the u16 EFI variable `TrialStateCtr`
  (GUID 8be4df61-93ca-11d2-aa0d-00e098032b8c, the EFI global variable GUID;
  `ST/lib/fwu_updates/fwu.c:42-45`), present only while a trial runs. It lives in
  `ubootefi.var` on the EFI system partition (`ST/lib/efi_loader/efi_var_file.c:140-144`,
  `ST/lib/efi_loader/efi_disk.c:531-535`). `tools/efi_varfile.py --fwu FILE`
  reads a copy of that file read-only and prints `fwu-trial: counter <n>` or
  `fwu-trial: absent` (exit 0; 2 a bad file; 64 usage; selftest:
  `tools/testdata/efi_varfile_selftest.sh`). A counter that is present
  refuses `default`. Whether the file is on bootfs is
  `TODO(board-env-read)`, and the `/boot` sha256 comparison would show it.
- **Skeleton: the undo lines are printed**, then the staged `default.bin` is
  written (`.part`, then rename) and the single environment write follows.
  With `PAYLOAD_READY=0` the runner stops before the first write with exit 3
  and changes nothing.

## 4. `forever`: only with a person

`default <image> forever` writes `mp2s_left=forever`: the image boots at
every boot and nothing returns the board to Linux by itself. The runner
refuses unless `BOARD_PERSON_PRESENT=1`, with the message `board.sh` gives the
Verdins: give a count (1-9) and the board comes back by itself, or set
`BOARD_PERSON_PRESENT=1` when someone is at the board. It also refuses an image
without the product watchdog's marker, the literal `mp2_wdtr: product refresh
armed` (section 6), unless `MP2_NO_WATCHDOG=1` says so on purpose. Without
that marker, the image resets within about 32 s and `forever` becomes a boot
loop. (The skeleton's `check_image` looks for `gc_wdog_arm: armed`, the board
layer's marker, instead, and for every count: section 7 lists that as a
disagreement.) Only a person at the console or with the
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
| `default <image> forever` | `nodefault` once Linux is back | **a person at the U-Boot prompt** (a key at "Hit any key to stop autoboot", then): `setenv mp2s_left 0; saveenv; boot`, and the script then restores everything itself. Or, not relying on our script: `setenv boot_prefixes '<the value default printed>'; setenv mp2s_left; setenv mp2s_prefixes; setenv mp2s_ver; saveenv; boot`. **With no console: the card in a PC**: `tools/mp2s_card_undo.sh --yes <bootfs partition>` renames `yakogl/standalone/boot.scr.uimg` to `boot.scr.uimg.off`. The prefix then finds nothing, and ST's extlinux boots Linux. Then `nodefault` from Linux |
| a default whose count ran out | `nodefault` removes `default.bin` (the environment is already restored) | nothing |

**The undo without Linux: `tools/mp2s_card_undo.sh`.** The card is in a PC and
the bootfs partition is not mounted. Its default is `--dry-run`, which only
reads; `--yes` writes. It needs `blkid`, `debugfs`, `dumpe2fs` and
`sha256sum`. It refuses (exit 1) a whole disk (give the bootfs partition, or
an image file of it), a partition table, a mounted target, anything but a
clean ext4, a bootfs without ST's root `boot.scr.uimg` and `mmc0_extlinux/` or
`mmc1_extlinux/`, a missing `yakogl/standalone/` or `boot.scr.uimg`, and an
existing `boot.scr.uimg.off`. A missing tool exits 2. It writes exactly one
thing, in one `debugfs -w` session (`ln` then `unlink`, which is a rename), and
prints the command that renames it back. It hashes ST's stock files and the
rest of `yakogl/standalone/` before and after and fails (exit 1) if one
differs. Run again after the rename, it says "Already undone" and exits 0.
After it, the saved environment still holds the default (`mp2s_left`,
`boot_prefixes` with our prefix): the loop finds no script under our prefix,
prints `SCRIPT FAILED: continuing...` and ST's extlinux boots Linux. That
state is harmless but not clean: `tools/mp2s_guard.py --for oneshot` refuses
while `mp2s_*` is set, so the next step from Linux is `nodefault`, then
`uninstall` (which removes the `.off` file with the directory).

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
  itself, through OP-TEE's SMC watchdog service. The client is
  `driver/rust/src/soc/stm32mp2/smc_wdt_refresh.rs`, opt-in: nothing calls it
  unless an image does, and the board layer's `stm32mp2_wdog.c` is unchanged.
  Its C ABI is `tools/probes/stm32mp257f_dk/mp2_wdt/src/mp2_wdtr.h`:
  `yd_mp2_wdtr_init(0xbc000000, 32, &out)` sends INIT, SET_TIMEOUT, ENABLE 1
  and PET; `yd_mp2_wdtr_pet(&out)` and `yd_mp2_wdtr_time_left(&out)` follow.
  The rules the client keeps by construction, from the pre-study's reading of
  OP-TEE (`:258-259`):
  - INIT first (PET and GET_TIMELEFT are refused before it, with no call);
  - a timeout of 6 s or more;
  - ENABLE with 1, never ENABLE with 0;
  - PET **more often than every 27 s** (OP-TEE panics about 27 s after the last
    PET, and the IWDG resets about 5 s later), from a context that runs only
    while the application is healthy. `mp2_product` pets every 5 s
    (`CONFIG_MP2_PRODUCT_PET_S`), only if its heartbeat moved, and a build
    assert keeps `PET_S + 5 < timeout`;
  - no lifetime reset for a product: `CONFIG_MP2_PRODUCT_LIFETIME_S=0`. The
    board layer's `CONFIG_YAKOGL_STM32MP2_LIFETIME_S` has no off switch (5-3600
    s), so `mp2_product` sets it to 3600 and a product image still resets after
    an hour until the layer gets one (`mp2_product/README.md`).
  
  `mp2_wdt` proves the refresh and then stops petting on purpose
  (`mp2_wdt/README.md`: board item `board-wdt`). `mp2_product` is the product's
  shape. It prints the marker `mp2_wdtr: product refresh armed: ...` once INIT
  has succeeded, which is what `default <image> forever` looks for. Its default
  build has a 120 s lifetime, so a counted default of it ends every boot in a
  reset. **The `LIFETIME_S=0` build may only go with `forever`**: under a
  counted default it never resets, the count never runs out and the board is
  stranded. Neither the runner nor `tools/mp2s_image_check.py` refuses that
  case yet (new work, reported).

## 7. The interface contract

For the `scripts` item (`driver/tools/mp2-standalone/`) and the `runner` item
(`tools/board-stm32mp2-standalone.sh`).

### Files on bootfs

| path in Linux | path in U-Boot | written by | what |
|---|---|---|---|
| `/boot/yakogl/standalone/boot.scr.uimg` | `/yakogl/standalone/boot.scr.uimg` | `install` | our script, `mkimage -A arm64 -O linux -T script -C none -d mp2-standalone.cmd` |
| `/boot/yakogl/standalone/boot.scr.uimg.sha256` | | `install` | its sha256 |
| `/boot/yakogl/standalone/default.bin` | `/yakogl/standalone/default.bin` | `default` | the image (`default.bin.part`, then rename) |
| `/boot/yakogl/standalone/u-boot-env.bin` | | `install` | the pristine environment image |
| `/boot/yakogl/standalone/boot-sha256.txt` | | `install` | the sha256 of every file under `/boot` |

The file names are fixed in the script and in the runner (`STAGED=default.bin`);
there is no file-name variable. The runner's write guard (`guard_path`) allows
the stage directory and plain file names directly in it
(`[A-Za-z0-9][A-Za-z0-9._-]*`), so there is no subdirectory: the pristine
environment image is `u-boot-env.bin`, not `backup/u-boot-env.bin`.
The card-undo tool (section 5) adds one name, `boot.scr.uimg.off`.
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

`refused: <why>` covers the script's refusals: devtype, devnum or
distro_bootpart unset; `default.bin` not found, below 0x40 bytes or above the
window; load failed or `filesize` changed; no ARM64 magic; `image_size` of 4
GiB or more or above the window; and `mp2s_prefixes` not set in the used-up
branch (that one reads `refused: mp2s_prefixes is not set, boot_prefixes left
as it is, booting Linux`).

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

Exit codes: 0 done; 2 usage (also `forever` without a person, a bad count, a
missing image, no `BOARD_HOST`); 3 refused before anything changed (a guard,
a failed image check, `PAYLOAD_READY=0`). `--dry-run` runs every check and
prints `DRY board$ ...` for each command it would send, writing nothing and
needing no `BOARD_HOST`. The environment: `BOARD_HOST`, `BOARD_PERSON_PRESENT`,
`MP2_NO_WATCHDOG`, `MP2_SUDO`, `MP2_KNOWN_HOSTS`. Other verbs: `guard path
<path> | name <name>` (the write guard, `ok` or a refusal), `selftest`
(`tools/board_stm32mp2_standalone_selftest.sh`). `status` lists the stage
directory and runs `fw_printenv mp2s_ver mp2s_left mp2s_prefixes
boot_prefixes`. `uninstall` reads `mp2s_left` first.

The undo lines `default` prints, as the skeleton words them:

```
board-stm32mp2-standalone.sh stm32mp257f-dk: UNDO, from Linux (before the reboot, or once Linux is back): tools/board-stm32mp2-standalone.sh nodefault
board-stm32mp2-standalone.sh stm32mp257f-dk: UNDO, without Linux: none needed: after <N> boots the script restores boot_prefixes and boots Linux
```

and for `forever` instead of the second line a person's two: `UNDO, without
Linux (a person at the U-Boot prompt): setenv mp2s_left 0; saveenv; boot` and
`UNDO, with no console (a person): the card in a PC, delete
yakogl/standalone/boot.scr.uimg from bootfs`. The end of a `default` is
`default set for N boots; nothing was rebooted.`, and of a `nodefault` `no
default: ST's extlinux boots Linux again`.

What is not in the skeleton yet, and is the payload's: the board lock and the
state file (`board-stm32mp2.sh:82-93`), `dev_guard` before each environment
write, the install record and the pristine environment image, the staged
script, the environment write with its read-back, and the `/boot` comparison.
The one-shot runners (`board-stm32mp2.sh run`, `driver/tools/uboot-mp2/oneshot.sh`) must
refuse while `mp2s_left` is set: the boot after their image would not be
Linux. The guard that section 3 calls with `--for standalone` has a `--for oneshot`
mode for exactly that (exit 3 on a refusal, 2 on an unreadable environment).
`oneshot.sh` (plan #357) calls it on its `fw_printenv` before it stages
anything; wiring it into `board-stm32mp2.sh run` belongs to plan #349, so that
is new work, reported, not done here.

### Where the skeleton differs from this note

The merged code wins, except where it would break a rule of the note. These
are open, and each is reported with its issue text:

- The skeleton's `check_image` wants `gc_wdog_arm: armed` (the board layer's
  marker) for every count, but `mp2_product` prints `mp2_wdtr: product refresh
  armed` and no `gc_wdog_arm`. The note keeps the product marker, and keeps it
  for `forever` (section 4).
- `default`'s undo for `forever` and the script's own undo line say "delete
  `yakogl/standalone/boot.scr.uimg`", while the card undo tool renames it to
  `boot.scr.uimg.off` (section 5). Both end the script's use; the tool is the
  one with checks.
- The skeleton's `check_image` does not check `image_size` or half of bootfs's
  free space; the image check tool of section 3 does all of them and is what
  the payload calls.

## 8. Why no step strands the board

| what fails | what happens |
|---|---|
| staging or its read-back | the file is removed; the environment is not touched; no reboot |
| the runner's environment write or its `fw_printenv` read-back | the pre-edit image goes back at once; no reboot |
| both environment copies bad at boot | U-Boot uses the defaults and `env_check` saves them; our prefix is gone; ST's root `boot.scr.uimg` restores ST's values and boots Linux |
| our script missing, corrupt, or the wrong version | `SCRIPT FAILED: continuing...`; the next prefix is ST's; Linux |
| `saveenv` in the script | nothing is loaded; `exit`; Linux |
| a power cut during that `saveenv` | the older copy stays valid: at most one extra boot, then the count goes on |
| `mp2s_prefixes` empty in the used-up branch | `refused: mp2s_prefixes is not set, ...`; nothing is changed; `exit`; Linux on every boot until `nodefault` |
| `size`/`load`/a check | `refused: ..., booting Linux`; one boot used |
| the image hangs | the watchdog resets the SoC in about 32 s; the count is already lower; Linux at the latest after N boots |
| the image returns | `the image returned, booting Linux`; the loop goes on |
| the count reaches 0 | the script restores `boot_prefixes` and boots Linux on the same boot; if that save fails, Linux boots anyway and the next boot repeats it |
| a firmware update in trial | the runner refuses `default` (section 3) |
| the host dies after `default` | nothing on the boot path needs the host |
| `forever` | only with a person, who has the undos without Linux (section 5) |

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

## Board session procedure

The ordered commands of a board session for `default <mp2_hello zephyr.bin> 2`
(plan #362's `board-run` item), on `resource:stm32mp257f-dk`,
`resource:stm32mp257f-dk-console` and `resource:stm32mp257f-dk-reset`. The board's
Linux is `$BOARD_HOST` and its console is `$BOARD_UART` (a device, or
`HOST:PORT`); both come from the lease and appear nowhere else. Run from the
repository root. `BOARD_PERSON_PRESENT` stays unset: this session has no person,
so it never runs `forever`. Logs go to a directory under `$TMPDIR`, never into
the tree:

```
LOG=$(mktemp -d "$TMPDIR/mp2s-session.XXXXXX")
IMG=<build dir>/zephyr/zephyr.bin       # mp2_hello, built as its README says
ssh() { command ssh -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" "$@"; }
```

A line marked `(pending)` is a tool that is not merged yet. Its name is the
planned one, and until it merges the step is read by eye from the same files.
**Any non-zero exit before step 4 ends the session with nothing changed.**

### Step 1: Build, host only

```
driver/tools/mp2-standalone/selftest.sh                    # "N ok, 0 failed"
driver/tools/mp2-standalone/build.sh "$LOG/stage"          # prints the script's sha256
sha256sum "$IMG"; stat -c %s "$IMG"                        # the image's sha256 and size
python3 -I tools/mp2s_image_check.py "$IMG"                # exit 0; refusals exit 2
```

`mp2_hello` has no product marker, so there is no `--require-marker` here; the
runner is told so on purpose with `MP2_NO_WATCHDOG=1` in step 4.

### Step 2: Preflight, read-only on the board

```
ssh "${MP2_SUDO-sudo -n }fw_printenv" > "$LOG/env-before.txt"
python3 -I tools/mp2s_guard.py --for standalone --printenv "$LOG/env-before.txt"
```

The guard exits 0 (`allow`); 3 means a `yako_*`, an `mp2z_*` or our prefix is
set, and 2 means the environment could not be read. Then the capacity of the
two edits `default` will make, with `PFX` the live `boot_prefixes`:

```
PFX=$(sed -n 's/^boot_prefixes=//p' "$LOG/env-before.txt")
python3 -I tools/mp2s_env_capacity.py --text "$LOG/env-before.txt" \
  mp2s_ver=1 mp2s_left=2 "mp2s_prefixes=$PFX" "boot_prefixes=/yakogl/standalone/ $PFX"
```

Exit 0 only (1: does not fit, 2: unreadable). Then the image against the free
space of bootfs, and ST's firmware-update trial counter:

```
FREE=$(ssh "df -B1 --output=avail /boot | tail -n 1")
python3 -I tools/mp2s_image_check.py "$IMG" --free-bytes "$FREE"
python3 -I tools/efi_varfile.py --fwu "$LOG/ubootefi.var"    # must print: fwu-trial: absent
tools/fwu_metadata.py (pending)                              # the booted bank is accepted
tools/mp2s_bootfs_check.py (pending)                         # /boot is the microSD's bootfs
tools/mp2s_fwenv_check.py (pending)                          # fw_env.config reaches the microSD
tools/mp2s_boot_manifest.py record (pending)                 # sha256 of every file under /boot
```

`"$LOG/ubootefi.var"` is a read-only copy of the file; where it lives is
`TODO(board-env-read)`. `fwu-trial: counter <n>` ends the session, because `default` would
spend ST's trial. Until the manifest tool merges, the record is
`ssh "find /boot -type f -print0 | sort -z | xargs -0 sha256sum" > "$LOG/boot-before.txt"`.
The preflight of `default` itself (section 3) repeats the image, guard and
capacity checks; the session runs them first to see a refusal before anything is
sent.

### Step 3: Console watch, started first

```
python3 -I tools/mp2s_console_watch.py watch "$BOARD_UART" "$LOG/raw.log" "$LOG/console.log" \
  --boots 2 --timeout 900 --linger 60 &
```

It only reads: a byte sent at U-Boot's autoboot prompt would stop the default.
`--boots 2` is the number of **image** boots; the log then has 2+1 boot cycles
(three BL2 banners), and the tool ends `--linger` s after Linux starts in the
third. A record is read by BL2's `NOTICE:  BL2` line (one boot cycle each), `boots left <n>`, `*** Booting Zephyr`, `PSCI SYSTEM_RESET at <n> ms` and `Starting kernel ...` / `Linux version`; for `default <image> 2` it wants 3 cycles: the first two with the image and its end, `boots left` 1 and 0, the third with Linux and no image. Exit 0 is the verdict `ok`, 1 another verdict, 4 the console is busy or
unreachable. The watch has no login regex of its own: it knows Linux by `Starting
kernel ...` or `Linux version`, and the login prompt is read from the raw record
in step 6.

### Step 4: Install and default, the undo printed first

```
tools/board-stm32mp2-standalone.sh install                 # prints "UNDO: ... uninstall" first
MP2_NO_WATCHDOG=1 tools/board-stm32mp2-standalone.sh --dry-run default "$IMG" 2 | tee "$LOG/dry.txt"
tools/mp2s_undo_order.py "$LOG/dry.txt" (pending)          # every UNDO line comes before its change
MP2_NO_WATCHDOG=1 tools/board-stm32mp2-standalone.sh default "$IMG" 2 | tee "$LOG/default.txt"
ssh "${MP2_SUDO-sudo -n }fw_printenv mp2s_ver mp2s_left mp2s_prefixes boot_prefixes"
```

The undo lines (section 7) are read before the change. After `default`, the read-back
is `mp2s_ver=1`, `mp2s_left=2`, `mp2s_prefixes=$PFX` and `boot_prefixes=/yakogl/standalone/
$PFX`, and the runner says `default set for 2 boots; nothing was rebooted.`
Exit 3 with `PAYLOAD_READY=0` means the runner's payload is not merged: the
session stops there (no board change was made, `install` aside). Nothing has been
rebooted yet.

### Step 5: Reboot and wait

```
ssh "${MP2_SUDO-sudo -n }systemctl reboot"
wait %1                                                     # the watch's exit: 0 is "ok"
python3 -I tools/mp2s_console_watch.py scan "$LOG/console.log" --boots 2
```

Expected, by cycle: `mp2s: default /yakogl/standalone/default.bin, boots left 1`,
the image and its `PSCI SYSTEM_RESET at <n> ms`; `boots left 0`, the image and its
reset; then `mp2s: default used up, booting Linux`, `SCRIPT FAILED: continuing...`,
ST's extlinux and Linux. The reboot is the only reset of the session. The
reset resource is not used to retry (step 8).

### Step 6: The verdict

```
grep -aE '^[[:alnum:]-]+ login: ?$' "$LOG/raw.log"           # OpenSTLinux is back: a login prompt
tools/mp2s_record_check.py "$LOG/console.log" (pending)      # boots left 1, image, boots left 0, image, Linux
tools/mp2s_reset_reason.py "$LOG/raw.log" (pending)          # BL2 'Reset reason:' per boot, supply first
tools/mp2s_comeback_check.py (pending)                       # Linux is back: ssh, mounts, no mp2s_*
ssh "${MP2_SUDO-sudo -n }fw_printenv" > "$LOG/env-after.txt"
tools/mp2s_env_diff.py "$LOG/env-before.txt" "$LOG/env-after.txt" (pending)
python3 -I tools/mp2s_guard.py --for oneshot --printenv "$LOG/env-after.txt"
python3 -I tools/efi_varfile.py --fwu "$LOG/ubootefi-after.var"   # still: fwu-trial: absent
```

Pass means: the console verdict is `ok`; a login prompt follows the third BL2
banner; `boot_prefixes` is `$PFX` again; no `mp2s_*` is left (the one-shot guard
exits 0); the only other differences are among the loop's working variables
(section 3); and the trial counter is absent. A `Reset reason:` of `Power-on
reset` or `Brownout reset` is the supply; `IWDG system reset` is the watchdog.

### Step 7: Clean up and the stock files

```
tools/board-stm32mp2-standalone.sh nodefault               # removes default.bin; safe to repeat
tools/board-stm32mp2-standalone.sh uninstall               # removes /boot/yakogl/standalone/
tools/mp2s_boot_manifest.py compare (pending)              # /boot equals the record, byte for byte
ssh "find /boot -type f -print0 | sort -z | xargs -0 sha256sum" > "$LOG/boot-after.txt"
cmp "$LOG/boot-before.txt" "$LOG/boot-after.txt"           # until the manifest tool merges
python3 -I tools/mp2s_redact_check.py (pending) "$LOG"/*   # before any log is quoted
```

`nodefault` and `uninstall` print their undo first. A difference in ST's files
is never repaired: it is a stop (below).

### Step 8: Stop rules

- A refusal (exit 2 or 3), a tool's non-zero exit or a differing read-back
  ends the session where it is. Nothing is retried with other flags and no
  check is skipped. Report which step and its output.
- **A board that does not come back needs a person.** If the third cycle shows no Linux
  (the watch's `no_linux`, or no login prompt by the timeout), `ssh` does not
  answer, or the count is not 0: stop. No second reboot, no reset through the
  reset resource, no `nodefault` over a board that may be mid-boot. State the
  record, the undo the runner printed, and `tools/mp2s_card_undo.sh` as the
  last resort for a person with the card.
- **A suspected hardware fault: the USB-C supply first.** Read BL2's `Reset
  reason:` of each cycle. `Power-on reset` or `Brownout reset` means the supply
  or cable, and a person checks it before anything else. Only `IWDG system
  reset` is the watchdog.
- No `forever`, ever, without `BOARD_PERSON_PRESENT=1` and a person at the
  board; this session sets neither.
- No board address, host name, UART path, serial number or credential goes into
  the report or a commit: redact the record and say `$BOARD_HOST` or `$BOARD_UART`.

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
