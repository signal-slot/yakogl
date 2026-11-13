<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK: the standalone boot from the board's own storage (design, 2026-11-13)

Status: **design, written on the host from ST's source. Nothing here has run on
the board and nothing was read from it.** Every fact that only the board can
give is a `TODO(board)` marker with one line saying what to read (index in
section 7). The note decides which routes the board's constraints leave open;
it builds nothing.

Goal (ROADMAP stage 5, issue #7; roadmap item 2 after plan #357): the DK boots
our Zephyr image from its own storage with no host driving a one-shot, the way
`docs/standalone-boot.md` describes for the Verdin boards and the Pi 4. This
note follows the DK's one-shot design, `stm32mp257f-dk-uboot-boot.md` ("the
one-shot note" below: its sections 1, 5, 7, 8 and 9 are relied on, never
repeated), and keeps ST's boot chain (TF-A v2.10 BL2/BL31, OP-TEE 4.0, U-Boot
2023.10; `pre-study-stm32mp257f-dk.md`, "Boot route for sign-off" and
:645-661): the direct routes of `imx8mp-direct-boot.md`, `am62p-direct-boot.md`
and `x2-direct-boot.md`, which replace U-Boot proper, are not this board's,
because OP-TEE's SCMI server powers the GPU. The standalone boot is therefore
**ST's U-Boot choosing our image by itself**, through its distro loop or its
environment.

"ST/" is ST's U-Boot, tag `v2023.10-stm32mp-r3.1`
(github.com/STMicroelectronics/u-boot), **commit
`f8917a01b48e0f3bc4595cb39e2d5431774004d5`** (what the tag checks out to),
file:line. U-Boot is GPL: facts and citations only. "pre-study" is
`docs/notes/pre-study-stm32mp257f-dk.md`. The board's own U-Boot binary may not
be this exact tree: where that decides something it is a `TODO(board)`.

## 1. The constraints, and what each rules out

The requester's rules for this stage (they extend section 10 of the one-shot
note):

| rule | rules out | where it bites |
| --- | --- | --- |
| never write `fsbla1`, `fsbla2` | a changed TF-A BL2 (a "direct" chain, or a BL2 that loads our image) | the whole "replace ST's loader" family |
| never write `metadata1`, `metadata2` | marking a bank, a trial state or a rollback by hand; an FWU capsule | section 5: ST's own code writes them, ours never |
| never write `fip-a`, `fip-b` | a changed U-Boot, OP-TEE or BL31; a U-Boot built with a different `bootcmd`; our image in a FIP slot | a compiled-in default, a U-Boot with `bootcount` |
| never write the eMMC | anything staged on, or booted from, the eMMC (`mmcblk0`, whose contents were never examined) | the image lives on the microSD's bootfs only |
| never repartition the card | a partition of our own; a changed partition flag (`bootable`); a second bootfs | the distro loop's `part list -bootable` stays as ST set it (section 2) |
| touch only staged files under `/boot` | an edit, rename or overwrite of ST's `Image.gz`, device trees, `boot.scr.uimg`, `mmc0_extlinux/`, `mmc1_extlinux/` or any extlinux file | a label added to ST's `extlinux.conf` is an edit of ST's file: out |
| runner-owned environment variables, each with its undo | a persistent write of one of ST's variables (`bootcmd`, `bootcmd_stm32mp`, `altbootcmd`, `boot_targets`, `boot_prefixes`, `boot_scripts`, `boot_syslinux_conf`, `env_check`, `preboot`, ...) | section 4: that is route (b) |
| no board address, host name, UART path, serial number or credential in a file | | this note has none |

Two consequences carry the rest. **First**, the only things this stage may
change are new files under `/boot/yakogl/` (as the one-shot's) and **new**
variables. **Second**, no new variable can change ST's boot by itself: ST's
`bootcmd_stm32mp` and the distro loop read none that we invent (section 2), so
a variable of ours is a flag for a script of ours and is useless until
something ST's boot actually reads leads to that script.

## 2. What ST's distro loop does here

From ST's source (`ST/include/config_distro_bootcmd.h`, the stock macro the
DK's `ST/include/configs/stm32mp25_st_common.h` pulls in through
`stm32mp25_common.h:136-142` and the `CONFIG_SYS_CONFIG_NAME` of
`ST/board/st/stm32mp2/Kconfig:9-11`):

- **The defaults.** `boot_prefixes=/ /boot/` (`config_distro_bootcmd.h:494`),
  `boot_scripts=boot.scr.uimg boot.scr` (`:495`), `boot_syslinux_conf=
  extlinux/extlinux.conf` (`:499`). The board's own values are what the stored
  environment says: `TODO(board-env-read)` `printenv boot_prefixes boot_scripts
  boot_syslinux_conf boot_targets preboot`. The stored environment replaces the
  compiled-in one wholesale on load (`env_import` clears the table before it
  imports, `ST/env/common.c:301-324`), so a variable the stored copy lacks is
  **absent**, not defaulted; an undo of a changed ST variable must write the old
  value back, never delete the variable.
- **Which device and partition.** `bootcmd_stm32mp` narrows `boot_targets` to
  `mmc${boot_instance}` on an mmc boot (`ST/include/configs/stm32mp25_st_common.h:26-40`;
  the DK's `ST_STM32MP25_BOOTCMD` is the common one plus nor/nand cases). For
  that device `scan_dev_for_boot_part` runs `part list <dev> -bootable devplist`
  and takes **each bootable partition that has a filesystem**, in the order
  listed (`config_distro_bootcmd.h:540-552`, the `for distro_bootpart` loop at :543;
  `env exists devplist || setenv devplist 1` when none is flagged).
  `TODO(board-env-read)`: `part list mmc <n> -bootable` at the prompt: whether
  bootfs is the only bootable partition and its number.
- **The order inside one partition** (`scan_dev_for_boot`, `:530-539`): for each
  prefix of `boot_prefixes`, in order, **first** `scan_dev_for_extlinux`
  (`test -e ... ${prefix}${boot_syslinux_conf}`, then `run boot_extlinux`, i.e.
  `sysboot ... any ${scriptaddr} ${prefix}${boot_syslinux_conf}`, `:500-511`),
  **then** `scan_dev_for_scripts` (each name of `boot_scripts`: `test -e`, then
  `run boot_a_script` = `load` and `source`, `:513-528`). So the four probe
  positions are, in this order:

  | position | path tried (defaults) |
  | --- | --- |
  | p1 | `/extlinux/extlinux.conf` |
  | p2 | `/boot.scr.uimg`, then `/boot.scr` |
  | p3 | `/boot/extlinux/extlinux.conf` |
  | p4 | `/boot/boot.scr.uimg`, then `/boot/boot.scr` |

  A successful extlinux boot does not return (the loop's "EXTLINUX FAILED:
  continuing" line is reached only on failure, `:504-511`), and a script that
  ends normally returns to the loop ("SCRIPT FAILED: continuing", `:518-528`),
  which goes on to the next position and finally to the next boot target.
- **What the board's files say against this.** The pre-study lists the bootfs
  root as `Image.gz`, the device trees, `boot.scr.uimg`, `mmc0_extlinux/` and
  `mmc1_extlinux/` (:241-247), and found the loop reaching extlinux and leaving
  `boot.scr.uimg` alone (section 1, "U-Boot's flow"; Q8, :823), with a note that
  `boot.scr`, if reached, ends in `env save`. The source's default path
  `extlinux/extlinux.conf` does not match a directory named `mmc0_extlinux/`.
  Either the stored environment sets `boot_syslinux_conf` (for example from
  `${boot_device}${boot_instance}`), or ST's U-Boot differs from this tree, or
  the listing is incomplete. This is the decisive unknown of the stage:
  `TODO(board-env-read)`: the live `boot_syslinux_conf`, `boot_prefixes`,
  `boot_scripts`, the complete tree of bootfs (names, sizes, sha256, free
  space) and the console lines `Found ...` of one stock boot (they name the file
  the loop took and the position it was found at).
- **A label is chosen by the file, not the loop.** Inside the extlinux file the
  menu takes `default`, `timeout`, `prompt` and the environment variable
  `pxe_label_override`, which selects the label of that name if the file has it
  (`ST/boot/pxe_utils.c:1466-1490`; a missing name only prints "Missing override
  pxe label"). `include` is a keyword of the same file format
  (`pxe_utils.c:786`, `:816`, `:1110`), read from inside an extlinux file, so it
  too needs the file edited. A kernel label runs `do_booti` for an arm64 Image
  (`pxe_utils.c:754`), with the device tree from the label's `fdt`/`fdtdir`, else
  `fdt_addr`, else U-Boot's own control DTB (`:734-740`).

**What follows for a staged file.** A file staged by us is found *instead of*
ST's only if its position comes earlier in the order above than the one ST's is
found at, and the two cannot share a path. With ST's extlinux at p1 (the likely
case, `TODO(board-env-read)`), p1 is taken, so extlinux files staged at p3 are
never reached while ST's works; and a staged `boot.scr.uimg` at p2 or p4 is
never reached either, because extlinux at p1 does not return. The names of
ST's own files cannot be reused (rule: ST's files are untouched), and the loop
reads no other name. **No file staged under the rules can pre-empt ST's boot
through the distro loop unless the board's stored environment or bootfs layout
leaves an earlier position free** (section 4, route A).

## 3. What is also in ST's boot that a standalone boot meets

Facts a design must live with, from ST's source (none depends on our choice):

- **Autoboot** counts `CONFIG_BOOTDELAY=1` seconds (`ST/configs/stm32mp25_defconfig:27`);
  a person's key stops it. No `CONFIG_AUTOBOOT_*` (one-shot note, section 1).
- **`bootcmd` is `run bootcmd_stm32mp`** (`defconfig:28`); `bootcmd_stm32mp` runs
  `env_check` (`stm32mp25_common.h:94`, saves only when the stored environment
  is the default one, one-shot note section 1), narrows `boot_targets` and runs
  `distro_bootcmd`.
- **`bootcount` is compiled out.** The defconfig has no `CONFIG_BOOTCOUNT_LIMIT`
  and no other `BOOTCOUNT` option (`grep` of `ST/configs/stm32mp25_defconfig`
  finds none; the only STM32 `BOOTCOUNT` options are the `mach-stm32mp`
  Kconfigs for the 13x/15x parts, `Kconfig.13x:32-37`, `Kconfig.15x:102-107`).
  Without it `bootcount_inc()` and `bootcount_error()` are empty and return 0
  (`ST/include/bootcount.h:134-135`), `bootdelay_process` therefore never takes
  the `bootlimit` branch (`ST/common/autoboot.c:453`, `:476-486`), and
  `bootlimit` is an inert word. **ST gives the DK no boot counter we could
  borrow**, and it gives none that could bring Linux back after N boots of
  ours. `TODO(board-env-read)`: `printenv bootcount bootlimit altbootcmd` and
  `bootcount print` (if the command exists: `help bootcount`) on the board's
  binary, which may differ from the defconfig.
- **`altbootcmd` exists but is not a counter.** ST's header defines
  `altbootcmd=run bootcmd_stm32mp` (`ST/include/configs/stm32mp25_st_common.h:43`,
  in the environment at `:46-51`). `bootdelay_process` chooses it **only** when
  `fwu_plat_get_bootidx` is nonzero (`ST/common/autoboot.c:478-486`, with
  `CONFIG_FWU_MULTI_BANK_UPDATE=y`, `defconfig:194`): the index is the low four
  bits of TAMP backup register 48 (`ST/arch/arm/mach-stm32mp/include/mach/stm32.h:188`,
  `:194-195`; the MP25 branch of that file) that TF-A set, i.e. "TF-A booted
  bank B". Both commands run the same `bootcmd_stm32mp`, so `altbootcmd`
  changes nothing for us and cannot be used as a hook.
- **U-Boot itself may write `metadata1`/`metadata2` and an EFI variable on
  every boot, before autoboot.** `fwu_boottime_checks` is an `EVT_MAIN_LOOP`
  handler (`ST/lib/fwu_updates/fwu.c:685-749`, notified at
  `ST/common/board_r.c:577`). If TF-A's boot index differs from the metadata's
  active index it **sets the active index** (`fwu.c:735-739`, a metadata write);
  in a trial state (after an update, not yet accepted by Linux) it counts boots
  in the non-volatile EFI variable `TrialStateCtr` (`fwu.c:74-102`, `:740-741`)
  and, past `CONFIG_FWU_TRIAL_STATE_CNT` (default 3, `ST/lib/fwu_updates/Kconfig:26-29`),
  **reverts the active bank** (`fwu_revert_boot_index`, `:85-93`). That is ST's
  code writing ST's partition, not our write, and it runs the same on a stock
  boot. It matters here for one reason: **a standalone boot that never lets
  Linux run cannot accept a trial update, so on a board in a trial state three
  boots of ours flip the bank.** The audit of what ST's chain writes on its own
  is plan #357's `fwu-write-audit` item; this note only depends on its answer.
  `TODO(board-env-read)`: the metadata's state (a read-only `dd` of `metadata1`
  from Linux; in U-Boot `fwu_mdata` if the command is built, `defconfig:32`),
  and whether the DK is in a trial state at all.
- **The watchdog** is armed from BL2 and bounds a hang of our image (one-shot
  note, section 9); it is not an alarm clock for a booted-and-finished one. For
  a prompt nobody types at, whether U-Boot services it is `TODO(board-env-read)`
  (the same section).

## 4. The routes, judged

Standalone means: after a power-on or reset, with no console and no host, ST's
U-Boot starts our image. Two families, as the requester names them.

### Route A (a): a staged item, no persistent write of ST's variables

Allowed by section 1: files under `/boot/yakogl/` plus new variables. Two
sub-forms, and a third that is out:

- **A1, an extlinux file at an earlier position than ST's** (section 2). It
  needs the loop to read our file first. Under the defaults that means p1
  `extlinux/extlinux.conf` or nothing, and p1 is ST's own path: **unreachable
  unless ST's extlinux is not at p1.** If the board shows ST's file at p3 (a
  `boot/` directory inside bootfs) or at a path built from `boot_device` that
  p1 does not name, then a staged `/extlinux/extlinux.conf` is first and A1 is
  open. The staged file would hold one label of ours: `kernel /yakogl/<image>`,
  `fdt` omitted, a `default` of that label. The image goes through `do_booti`
  (`pxe_utils.c:754`), which **moves it** to RAM base + `text_offset` unless
  the image header's flags have bit 3 (`ST/arch/arm/lib/image.c:65-80` and
  `ST/cmd/booti.c:77-86`, one-shot note section 3): a different entry than the
  one-shot's `go` (x0 gets the DTB pointer, caches are cleaned by
  `cleanup_before_linux` instead of the script's `dcache off`, and the EL is
  switched or kept by `boot_jump_linux`, `ST/arch/arm/lib/bootm.c:291-330`).
  `TODO(host)`: the flags and `text_offset` of our image header (bytes 8-31),
  and whether the image runs after `booti`'s relocation; `TODO(board-run)`: the
  `mp2-handover:` record from that entry.
- **A2, a `boot.scr.uimg` or `boot.scr` at an earlier position than ST's
  extlinux:** impossible by the same order (a script at p2 is after extlinux at
  p1; at p4 after p3). A script is reached only when **every** extlinux position
  before it fails, i.e. only on a bootfs without a working ST extlinux.
- **A3, a label inside ST's `extlinux.conf`, or a changed `default`:** an edit of
  ST's file. Out by section 1, even with a backup and a sha256 undo, without a
  person's decision.

Whatever the form, **A is not self-limiting.** An item the loop finds is run on
every boot. A test image that ends in PSCI `SYSTEM_RESET` or in the watchdog
(one-shot note section 9) is booted again by the next reset, forever; ST gives
no counter (section 3). A bound must come from our side, and each choice has a
price:

| bound | how | cost / rule |
| --- | --- | --- |
| a count in the environment | the staged script decrements a runner-owned `yako_left` and `saveenv`s, as `docs/standalone-boot.md`'s hook does | `saveenv` writes the **whole** RAM environment, runtime variables included (`boot_device`, `boot_instance`: `ST/arch/arm/mach-stm32mp/stm32mp2/cpu.c:194-229`) and a script can run it only from a console-less boot: a persistent environment write by a script that has no person present. Not authorised by section 1's wording ("runner-owned variables, each with its undo") unless the undo is a restore of the `u-boot-env` dump taken before (route B write path). A decision |
| a count in a register that survives a reset | a U-Boot script `mw`s a TAMP backup register that ST's firmware does not use | `TODO(board-env-read)`: whether the non-secure world may write TAMP backup registers other than 48 on this board (RIF-protected zones, `stm32.h:190-191`), whether TF-A or OP-TEE clears them on reset, and that a power cycle clears the count; not a board read yet. A power cycle resets the count by design, which is the right undo |
| a count in a file on bootfs | U-Boot `ext4write`s the file (`CONFIG_CMD_EXT4_WRITE=y`, `defconfig:62`) | a U-Boot write to the filesystem Linux boots from. A file under `/boot/yakogl/` is "staged" by the letter of section 1, but a write by U-Boot's ext4 writer to a mounted-by-Linux filesystem is a corruption risk to ST's own files; **not proposed** |
| no count | the standalone image is the product, with no way back from the board | the product form of `docs/standalone-boot.md`; needs a person's decision and a person for the undo |

**What route A accepts.** It writes nothing of ST's and every file it leaves is
inert if the loop never reads it. A1 is **acceptable** under the rules **if and
only if** the board reads of section 7 show that p1 is free and the bound is
the register count (or a person decides otherwise); A2 and A3 are not
available. Today A is a design with an unknown that decides it, not a route.

### Route B (b): a persistent default

A persistent change to a variable ST's boot reads, so that U-Boot goes to our
files by itself. Three variants, from the least to the most dangerous:

- **B1, prepend our own prefix to `boot_prefixes`:** `boot_prefixes=/yakogl/ / /boot/`
  (the loop then probes `/yakogl/extlinux/extlinux.conf` and `/yakogl/boot.scr.uimg`
  first). The loop's own code does all the work; **if `/yakogl/` is absent or
  empty the tests fail and the loop goes on to ST's prefixes, so removing the
  staged directory is a full undo of the boot, without a console**, and a wrong
  or missing file can never stop ST's Linux booting. The persistent part is one
  ST variable changed in a way that is a no-op without the directory. It still
  is a persistent write of an ST variable, and the stored environment replaces
  the compiled-in one (section 2), so the old value must be recorded from the
  board first. This is the variant to put to a person, if one is asked.
- **B2, `boot_syslinux_conf` or `boot_scripts`:** the same idea with a worse
  failure: a wrong value hides ST's own extlinux or script (they share the
  variable), and Linux does not boot until the variable is put back.
- **B3, `bootcmd` (or `bootcmd_stm32mp`):** the old hook, `board-stm32mp2.sh`'s
  design (a hook before ST's command). **Ruled out**: it is the persistent
  `bootcmd` write the DK runner's rule forbids (one-shot note section 7), a
  fault in it leaves a board without Linux until someone types at the
  console, and ST's `env_check` can overwrite or keep it depending on the
  stored environment's state (one-shot note section 1).

**Route B is not acceptable without a person's decision.** Writing the
environment also needs a way: the DK's Linux has `fw_printenv` and no
`fw_setenv` (`board-stm32mp2.sh`'s header, one-shot note section 5), so a write
is either `saveenv` typed by a person at the console, or a rewrite of the
`u-boot-env` partition from Linux with `tools/uboot_env.py` (the older runner's
way: both copies, CRC and flag byte, `tools/uboot_env.py --size 0x2000
--redundant --offset 0x7e000 --offset2 0x7c000`, one-shot note section 1). Both
are writes to `u-boot-env`, a partition outside the forbidden list in section 1
and inside the "touch only" rule: a person says whether it is allowed.

### The verdict

| route | meets section 1 | reachable | decision |
| --- | --- | --- | --- |
| A1 extlinux staged earlier than ST's | yes | only if ST's extlinux is not at p1 (`TODO(board-env-read)`) | none, once the board shows it open; the bound needs a decision if it is the environment |
| A2 `boot.scr` staged | yes | no (order of `scan_dev_for_boot`) | n/a |
| A3 a label in ST's extlinux | **no** (an edit of ST's file) | yes | a person |
| B1 `boot_prefixes` prepended | no (a persistent write of an ST variable) | yes, fail-safe | a person |
| B2 `boot_syslinux_conf`/`boot_scripts` | no | yes, not fail-safe | a person, not recommended |
| B3 `bootcmd` | **no** (the rule of the DK's runner) | yes | not asked |
| the typed one-shot of the one-shot note | yes | yes | none; **not standalone** |

**Which is acceptable:** route (a), in the form A1, once the board's layout is
read, and nothing else without a person. Today the only form that runs without
a decision is the one-shot, which needs the host. The honest summary is that the
DK's standalone boot, under the requester's rules, **is blocked on one board
read (where ST's extlinux is found, section 7) or on a person's choice of B1**.
That choice is not made here.

## 5. Coming back to ST's Linux

Bringing the board back to Linux is the property that separates a test stage
from the product (`docs/standalone-boot.md`, "Why the default counts its own
boots"). For the DK:

- **No ST mechanism returns it** (section 3): `bootcount`/`bootlimit` are
  compiled out and `altbootcmd` is the same command as `bootcmd`. The Verdins'
  count of N boots needs a variable of ours (section 4, "the bound").
- **The watchdog does not return it either.** It resets a hung image in about
  32 s (one-shot note section 9); the next boot goes to our item again if the
  loop still finds it. It bounds one boot, not the number of boots.
- **What does return it, per route.** A1: the staged `/extlinux/extlinux.conf`
  removed from Linux **or** the register count used up (a power cycle resets
  it); with the item gone, the next boot is ST's. B1: the staged `/yakogl/`
  directory removed (the variable may stay); a person restores the old
  `boot_prefixes` value for a full undo. The undo is printed before anything
  changes, as in the one-shot note section 8, **and Linux has to be reachable
  to run it**: a board that reboots into our image every time and has no ssh in
  our image is reachable only through the console (a person types `reset` at
  the autoboot, `STOP_KEY` of `tools/mp2_uboot_console.py`, then `run
  bootcmd_stm32mp`-style recovery with the staged item removed by a person
  with the card in a PC). The `<count>`-bounded form therefore matters more
  here than on the Verdins, which have a UART bridge in the runner.
- **Recovery by a person**, last resort, never a write by us: the microSD in a PC,
  `rm -r /yakogl` (and the extlinux file of A1) on bootfs, nothing else
  changes. Supply first if the board looks dead: BL2's `Reset reason:` line
  (one-shot note section 10).
- **The trial-state hazard of section 3:** until the board's metadata is read
  (`TODO(board-env-read)`), the standalone boot is not run on a board that is in
  a trial state. In a trial state ST's own code counts our boots against Linux.

## 6. What the standalone image needs from the image side

(Not decisions; they follow the Verdins' and the one-shot's.)

- The image is the same file the one-shot boots, linked at 0xa0000000, with the
  watchdog marker `gc_wdog_arm: armed`; the standalone item must name the same
  window and size rules (one-shot note section 3). `go` and `booti` give
  different registers and cleanup (section 4, A1): the image must be run once
  by each entry that the route uses (`TODO(board-run)`).
- A standalone item must end in the `mp2-handover:` record (the image prints
  it) so that the first standalone boot is read as the one-shot's was.
- A product image has no Linux to return to: the time bound of the layer
  (`CONFIG_YAKOGL_STM32MP2_LIFETIME_S`, 20 s) is a test setting, and a product
  build must not carry it. That is the image's, not the boot's, and is listed
  only so the follow-up does not forget it.

## 7. What is unknown

| marker | what to read | decides |
| --- | --- | --- |
| `TODO(board-env-read)` | `printenv boot_prefixes boot_scripts boot_syslinux_conf boot_targets boot_device boot_instance preboot` and the lines `Found ...` of a stock boot | the position of ST's extlinux (section 2), hence route A1 |
| `TODO(board-env-read)` | `printenv bootcount bootlimit altbootcmd`, `help bootcount`, `bootcount print` | whether the board's U-Boot has a counter (section 3) |
| `TODO(board-env-read)` | `part list mmc <n> -bootable` | the bootable partitions the loop scans (section 2) |
| `TODO(board-env-read)` | the full tree of bootfs with sha256 and free space, from Linux | the before record for every undo; where ST's extlinux is (section 2) |
| `TODO(board-env-read)` | `dd` of `metadata1` and `metadata2` (read-only), `fwu_mdata` if built | trial state (section 3, 5) |
| `TODO(board-env-read)` | a read of whether the non-secure world may write TAMP backup registers other than 48, and whether a reset keeps them | the register count (section 4) |
| `TODO(board-env-read)` | whether the stored environment is the full one or a sparse one | the undo value of a changed ST variable (section 2) |
| `TODO(host)` | flags and `text_offset` of the image header | whether `booti` leaves the image at 0xa0000000 (A1) |
| `TODO(board-run)` | the `mp2-handover:` record through the standalone entry | state at entry (one-shot note section 2) |
| `TODO(board-run)` | the board returning to ST's Linux with the item removed, and `fw_printenv` and `/boot` sha256 identical to the before record | the undo (one-shot note section 8) |

The citations above were read against a fresh shallow clone of the tag
(`f8917a01b48e0f3bc4595cb39e2d5431774004d5`). Anyone re-checking them later
should fetch the tag again: the earlier read-only study's cache is gone.

## 8. The follow-up list (what has to be built)

In this order; none exists, none is started by this note.

1. **Board read**, extending the one-shot note's `board-env-read` (the read is
   read-only, no write): add the rows of section 7 to what it records, in
   particular `printenv boot_prefixes boot_scripts boot_syslinux_conf preboot
   bootcount bootlimit altbootcmd`, `part list mmc <n> -bootable`, the whole
   bootfs tree with sha256 and free space, and `metadata1`/`metadata2`. The
   Linux-side half is plan #357's `env-read-tool` (`tools/probes/stm32mp257f_dk/env_read/`).
2. **A person's decision on route B1** (or none): whether a persistent
   `boot_prefixes` write is allowed, with the undo of section 5, and whether
   a write of the `u-boot-env` partition from Linux by `tools/uboot_env.py` (an
   "uboot_env" diff before and after, one-shot note section 8) is allowed for it.
3. **The staged items**, in a new directory beside `driver/tools/uboot-mp2/`
   (not in it: that directory is the typed one-shot's, and its lint forbids the
   names `extlinux` and `boot.scr` on purpose): an `extlinux.conf` with one
   label (A1), or a `boot.scr` with the checks of `mp2-mmc.cmd` (the file
   check, size, `go`), a build step (`mkimage`) and a host selftest, plus a QEMU
   test in the shape of `driver/tools/uboot-mp2/qemu/` for the loop's order
   (staged file at p1 against ST's at p1).
4. **The runner**: a `tools/board-stm32mp2-uboot.sh` verb pair `arm` and `disarm`
   (or the Verdins' `default`/`nodefault` names) that stages, records before,
   prints the undo first and, for a bound, sets the count; refuses on a trial
   state, on a missing before record and without the person flag the decision of
   item 2 needs (`BOARD_PERSON_PRESENT=1` in `docs/standalone-boot.md`).
5. **`tools/board.sh`**: the `stm32mp257f-dk` board's `default`/`nodefault`
   verbs (after plan #349's items that touch `tools/board.sh`,
   `board_selftest.sh` and `README.md`).
6. **`docs/standalone-boot.md`** gets a DK section naming this mechanism and
   its approvals, after the board reads are in. It is plan #341's file; this
   note did not edit it. `docs/README.md` gets this note's index line after
   plan #327.
7. **A board session**: one boot of `mp2_hello` through the chosen route, the
   return to Linux, the before and after records (`fw_printenv`, `/boot`
   sha256, the `u-boot-env` dump), then the product form with a person present.

### Text for the issue #7 comment

> STM32MP257F-DK standalone boot: the design is `docs/notes/stm32mp257f-dk-standalone-boot.md`
> (host-only, from ST's U-Boot `v2023.10-stm32mp-r3.1`, commit `f8917a01b48e`).
> Result: under the board rules (no write to fsbla/metadata/fip or the eMMC, no
> repartition, only staged files under /boot and runner-owned variables), the
> distro loop cannot pre-empt ST's own extlinux with a staged file: it probes
> `/extlinux/...`, `/boot.scr*`, `/boot/extlinux/...`, `/boot/boot.scr*` in that
> order and a successful extlinux boot does not return. A staged extlinux file
> works only if the board shows ST's extlinux is not at the first position; a
> staged `boot.scr` cannot. ST's U-Boot has no boot counter (`BOOTCOUNT_LIMIT`
> is off) and `altbootcmd` equals `bootcmd`, so the way back to Linux needs a
> count of ours. ST's own `fwu_boottime_checks` counts boots of a trial update
> and reverts the bank after 3: the DK must not be in a trial state. Open:
> one read-only board session (the live `boot_prefixes`, `boot_scripts`,
> `boot_syslinux_conf`, bootfs tree, `part list -bootable`, metadata) and a
> person's decision on route B1 (prepend `/yakogl/` to `boot_prefixes`), the
> fail-safe persistent variant. Follow-up list: section 8 of the note.
