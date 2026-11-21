<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The Linux route on the Verdin iMX95: panthor, not the stock driver

The `gpu-mali` back end has a second way to run besides our own Zephyr
kernel side: our user-space library on the board's own Linux, through the
upstream `panthor` DRM driver (`tools/mali/panthor/`, the NanoPC-T6's first
gl_test and dEQP-GLES2 numbers, `tools/cts/RESULTS.md` "Mali: NanoPC-T6 ...
panthor"). This note answers whether that route can give ES 2 results on the
Toradex Verdin iMX95 (Mali-G310) on the Mallow on its first day, apart from
the Zephyr bring-up (`docs/notes/pre-study-imx95.md`, section 8).

Public sources only. The vendor's closed GPU stack (NXP's `mali-imx` user
space, the Arm DDK) and the `mali_kbase` kernel driver are not observed or
used as a reference, on the host or on the board: nothing below reads their
output, and route (b) unbinds `mali_kbase` before anything of ours runs.

Sources are cited as: TK `linux-toradex.git` (git.toradex.com), TC
`linux-toradex-kconfig.git`, TN `meta-toradex-nxp.git`, TT
`github.com/torizon/meta-toradex-torizon`, MF
`github.com/Freescale/meta-freescale`, NX `github.com/nxp-imx/linux-imx`,
L `torvalds/linux`, FW `linux-firmware` (read on 2026-11-21).

## 1. What the Verdin iMX95's Linux uses today

**The stock images drive the G310 with Arm's `mali_kbase`, and their kernel
has no panthor at all.**

- Toradex's released BSP 7.x and Torizon OS 7.x for the Verdin iMX95 are
  Yocto scarthgap builds of `linux-toradex` branch `toradex_6.6-2.2.x-imx`
  (6.6.161; TN `scarthgap-7.x.y` `b0b9f0fd`: `conf/machine/verdin-imx95.conf`
  selects `linux-toradex`, recipe `linux-toradex_6.6-2.2.x.bb`, SRCREV
  `8ad2b7ff`). The walnascar layer moves to `toradex_6.12-2.0.x-imx`
  (6.12.55 at the branch head; TN `walnascar` `99e13a1b`,
  `linux-toradex-imx_6.12-2.0.x.bb`, SRCREV `faa3f90b`).
- The 6.6 tree has no `drivers/gpu/drm/panthor/` (TK `toradex_6.6-2.2.x-imx`:
  the file `panthor_drv.c` does not exist). The GPU driver is the Arm DDK's
  kbase, vendored in-tree under `drivers/gpu/arm/midgard` (release
  `r56p0-18eac0`, TK `toradex_6.18-2.1.x-imx` `drivers/gpu/arm/midgard/Kbuild:62`).
- The kernel configuration comes from `linux-toradex-kconfig.git` at the
  recipes' pin `48fa4fcb`: `cfg/arm64/arm64.cfg:158-159`
  `CONFIG_MALI_MIDGARD=m`, `CONFIG_MALI_CSF_SUPPORT=y`, no `DRM_PANTHOR`
  line. Panthor was added there as a module only on the `main` branch,
  commit `8b6230f1` (2026-10-17, "cfg: arm64: Add GPU Panthor driver ... used
  by arm mali GPU on i.MX95"; `main` `fc8f0831`, line 249), beside kbase,
  which stays (`:175-176`). No released recipe pins that commit.
- The user space: meta-freescale's `imx-base.inc` gives mx95 the `imxmali`
  override and `IMXGPU_GRAPHICS_PROVIDER:imxmali ?= "mali-imx"` (MF
  `scarthgap` `44fb1b89`, `:230`, `:570`; recipe `mali-imx_r50.4.bb`,
  NXP's binary package, which also installs the firmware kbase loads);
  Torizon installs `mali-imx` on mx95 (TT `scarthgap-7.x.y` `30b82266`,
  `recipes-images/images/torizon-base.inc:126-128`). A Toradex staff answer
  on their community forum (2026-03-12) says they do not support Mesa's
  driver on this module.
- The device tree of Toradex's kernel describes the GPU for kbase:
  `mali: gpu@4d900000`, `compatible = "arm,mali-valhall", "nxp,imx95-mali"`,
  one clock `IMX95_CLK_GPU_CGC` named `"gpu"`, `power-domains =
  <&scmi_devpd IMX95_PD_GPU>, <&scmi_perf IMX95_PERF_GPU>` (`"gpumix",
  "gpuperf"`), kbase power models, an OPP table with kbase's `opp-hz-real`
  (TK `toradex_6.18-2.1.x-imx` `imx95.dtsi:3166-3192, 540-560`; the same
  node at `toradex_6.12-2.0.x-imx` `:2912`; on 6.6 `:2961` with
  `IMX95_CLK_GPU` + `IMX95_CLK_GPUAPB` and the blk-ctrl in `reg`). The
  Verdin board files do not touch it, so it is enabled.
- NXP's own BSP is the same: `lf-6.12.y` and `lf-6.18.y` build kbase in and
  panthor out (NX `arch/arm64/configs/imx.config`: `CONFIG_MALI_MIDGARD=y`,
  `CONFIG_DRM_PANTHOR=n`; TK `toradex_6.18-2.1.x-imx` `imx.config:11-12,
  19-21`). `imx_v8_defconfig` has both as modules (`:668-669, 1016-1018`).

Where panthor is in Toradex's trees:

| Branch | panthor | i.MX 95 in panthor | G310 known |
|---|---|---|---|
| `toradex_6.6-2.2.x-imx` (BSP 7.x) | absent | - | - |
| `toradex_6.12-2.0.x-imx` (walnascar) | present | no: match table `rockchip,rk3588-mali`, `arm,mali-valhall-csf` only (`panthor_drv.c:1444-1445`); Toradex's DT compatible matches neither | no (`MODULE_FIRMWARE` arch10.8 only, `panthor_fw.c:1371`) |
| `toradex_6.18-2.1.x-imx` (head `851ac0f9`, 6.18.62; in no public Yocto layer yet) | present | yes, NXP's: `nxp,imx95-mali`, `nxp,imx952-mali` (`panthor_drv.c:1686-1689`); attaches both power domains and sets `gpuperf` to 1 GHz for them (`panthor_device.c:70-81`); takes the first clock unnamed (`:46`) | yes: `GPU_PROD_ID_MAKE(10, 4)` "Mali-G310" (`panthor_hw.c:25-26`), firmware arch10.12 (`panthor_fw.c:1438`) |

In that 6.18 tree both kbase (`mali_kbase_core_linux.c:4953-4958`) and
panthor match `nxp,imx95-mali`; which binds is the module that loads first.
meta-freescale's `master` adds a switch for exactly that: `IMX_MALI_DRIVERS`
and a `mali-imx` script that blacklists `mali_kbase` and loads `panthor`
through `modules-load.d` (MF `master` `62c1c040`, `imx-base.inc:636-642`,
`recipes-graphics/mali/files/`), and installs
`linux-firmware-mali-csffw-arch1012` for the mainline BSP (`:515`).

**Mainline.** Panthor drives the G310 through the generic compatible; there
is no i.MX 95 code in it (pre-study section 1.2):

- Panthor names the G310 and declares the arch10.10/10.12 firmware from
  **v6.18** (L `2c0d0a81`, "Add support for Mali-G710, Mali-G510 and
  Mali-G310").
- The binding (`nxp,imx95-mali  # G310`, L `6918b7e8`) and the
  `gpu@4d900000` node (L `67934f24`, "arm64: dts: imx95: Describe Mali G310
  GPU") arrive in **v6.19**: `compatible = "nxp,imx95-mali",
  "arm,mali-valhall-csf"`, clocks `IMX95_CLK_GPU_CGC` ("core") and
  `IMX95_CLK_GPUAPB` ("coregroup"), one power domain `IMX95_PD_GPU`, no perf
  domain, no blk-ctrl, OPPs 500/800/1000 MHz at 920 mV (L master
  `imx95.dtsi:2298-2312, 253-270`).
- The core clock became `GPU_CGC` in **v7.0** (L `fc61fdfd`, "Use GPU_CGC as
  core clock for GPU"; its message: without it the GPU's registers cannot be
  reached on B0 silicon). v6.19 alone is not enough on a B0 part.
- The Verdin iMX95 device trees (`imx95-verdin.dtsi` and the
  `{wifi,nonwifi}-{dahlia,dev,ivy,mallow,yavia,zinnia}` boards) arrive in
  **v7.1** (L `676d7ce2`, "Add Verdin iMX95 support"). None of them touches
  `&gpu`, so the GPU is on. The arm64 `defconfig` builds
  `CONFIG_DRM_PANTHOR=m` (L `v7.1` `arch/arm64/configs/defconfig:1060`).
  The newest release is v7.3.
- Firmware: panthor asks for `arm/mali/arch10.12/mali_csffw.bin` on the
  G310 (Mesa `PAN_PROD_ID(10, 12, 4)`, pre-study section 1.2). linux-firmware
  added that file on 2025-08-08 (FW `930ef904`); its bytes are the
  arch10.8 file's, the one we pin for the T6 (pre-study section 2). Toradex's
  scarthgap layer installs linux-firmware `20250311`, older than that: the
  stock rootfs has no `arch10.12` file.

So: **(i) does not hold.** No released Verdin iMX95 image has panthor; the
nearest Toradex tree that has it for this SoC (6.18) is in no public
Yocto layer yet, and its configuration would build kbase too.

## 2. The ways to a panthor Linux on the module

| | What | Cost | What the owner does |
|---|---|---|---|
| (i) | The stock image already has panthor | - | does not hold (section 1). When Toradex releases a 6.18 image built from kconfig `main`, it becomes (i'): blacklist `mali_kbase`, load `panthor`, add the arch10.12 firmware; nothing to build. Not today. |
| (ii) | The stock kernel (6.6) plus panthor built as a module against Toradex's source, mainline's GPU node as an overlay, `mali_kbase` unbound | **high**: 6.6 has no panthor; a backport of panthor brings `drm_gpuvm`, `drm_exec` and the scheduler changes from 6.8-6.18, then the G310 and B0 clock patches. With the walnascar 6.12 kernel panthor exists but lacks the G310, the i.MX 95 match and the perf domain; an overlay (`arm,mali-valhall-csf`, clocks "core"/"coregroup", one power domain) plus a backport of `2c0d0a81` might do. Either kernel is not the stock image's (6.12 is walnascar's), so it is a kernel build anyway. | flash a walnascar image first (6.12 variant); then copy a module and an overlay, edit the overlay list, reboot. |
| (iii) | A mainline kernel (v7.1 or later; v7.3 now) with its own Verdin iMX95 Mallow DT, on a Debian trixie arm64 rootfs | **low**: arm64 `defconfig` as it is (panthor `=m`, the Mallow dtb), firmware copied in (the T6's pinned file at `arch10.12`), a debootstrap rootfs. Same rootfs as the T6's Armbian trixie: the deqp-gles2 and glmark2-es2 builds and their ABI floors (GLIBC 2.41) apply unchanged. Risk: the first boot of a mainline kernel on this module (the GPU's reset release and power, pre-study U5; the SM's SCMI version, U1). | put a microSD card in the Mallow (the card is ours, built on the host); at the U-Boot prompt, boot from it once (or confirm `boot_targets` scans the card first). The eMMC (Toradex's image) is not touched, nothing is reflashed. |
| (iv) | Toradex's own 6.18 kernel (`toradex_6.18-2.1.x-imx`, `imx_v8_defconfig`: panthor and kbase both `=m`) with its DT unchanged, `mali_kbase` blacklisted | **medium**: one cross kernel build; NXP's panthor already knows the i.MX 95 (both power domains, 1 GHz perf state) and the G310. Not a released Toradex kernel; the stock rootfs is 6.6's (its glibc is 2.39, below the deqp-gles2 and glmark2-es2 builds' floor 2.41). | replace the kernel, DT and modules in the eMMC's boot partition and `/lib/modules` (a change of the stock image), or boot them from a microSD card as in (iii). |

**Recommendation: (iii)**, mainline v7.3 with the Mallow dtb and Debian
trixie on a microSD card. It is the only option built from released public
pieces with no patch of ours, it leaves the module's eMMC as Toradex shipped
it, and it gives the board the same user space as the T6, so every script of
route (b) (section 4) runs unchanged but for `MALI_BOARD=verdin-imx95`.
(iv) is the fallback when the mainline kernel does not bring the GPU up (an
SCMI or reset difference): NXP's panthor carries the i.MX 95 power code that
mainline lacks. (ii) is not worth its cost.

What the owner must do for (iii): one microSD card in the Mallow, power
on. Toradex's U-Boot scans the card first (`boot_targets=mmc1 mmc0 dhcp`,
section 5); if an environment was changed so that it does not, one command
at its prompt boots the card once. No reflash, no change to the eMMC or to
U-Boot's environment.

The card is built on the host by `tools/mali/panthor/imx95-sdcard.sh`
(section 5). Its login, address and keys come from the lease, never a file.

## 3. Day one on the Verdin iMX95 (route (b))

Every step is a script of `tools/mali/panthor` or `tools/cts` with the board
profile set (`tools/mali/panthor/board-profile.sh`). `BOARD_HOST` comes from
the lease, as for the T6.

```sh
export MALI_BOARD=verdin-imx95 BOARD_HOST=<from the lease>
# 0. Which render node is panthor's (read-only); set MALI_RENDER_NODE if it
#    is not renderD128. The kernel, the firmware and panthor's probe lines.
ssh $BOARD_HOST 'uname -r; for n in /sys/class/drm/renderD*; do echo "$n $(basename "$(readlink -f $n/device/driver)")"; done;
  ls -l /lib/firmware/arm/mali/arch10.12/; dmesg | grep -i panthor; lsmod | grep -c "^mali_kbase " || true'
# 1. The board against the builds' floors (os, glibc, render node, panthor):
tools/cts/mali-glmark2-preflight.sh
# 2. The GPU's identity words through our own uAPI code (no GL):
tools/mali/panthor/run-on-board.sh panthor-hello
# 3. gl_test, twice, into a record (as the T6's 2026-11-t6-sweep-<sha8>):
tools/mali/panthor/run-modes.sh build/imx95-sweep panthor-gltest 2 --sweep
# 4. The dEQP-GLES2 tree as on the T6 (docs/es2/board-recipe-mali.md):
tools/cts/mali-deqp-build.sh --out build/deqp-imx95
tools/cts/mali-deqp-run.sh --build build/deqp-imx95 --rows 1-24 --session S1 --dry-run
tools/cts/mali-deqp-run.sh --build build/deqp-imx95 --out tools/cts/log/<date>-imx95-gles2-S1-<sha8> --rows 1-24 --session S1
#    then S2 rows 25-38, S3 39-68, S4 69-76, the tally and the reruns of the recipe
```

- On the card of section 5 the login is root by key (`BOARD_HOST=root@<the
  lease's address>`), the stage root stays `/root/yakogl-mali`, debugfs is
  mounted and the SCMI raw interface is in the kernel, so the day-one probe
  (`tools/probes/verdin_imx95/linux_day_one/`) and
  `tools/imx95_bbm_record.sh probe` run as they are; `uname -r` is `7.3.0`.
  Before step 0, `systemctl --failed` on the console tells whether the
  board's own drivers (ENETC, usdhc, SCMI) came up; no host test can.
- Step 0 must show `panthor` as the driver of a render node and no
  `mali_kbase` loaded. A `mali_kbase` line means the image is not (iii) or
  (iv): stop; nothing of ours runs beside kbase, and its output is not read.
- `run-on-board.sh` and `mali-deqp-run.sh` stage under `/root/yakogl-mali`
  (`MALI_STAGE_ROOT` names another directory, for a non-root login).
  `UART_DEV` capture runs at 115200 baud on this profile.
- The tree is the T6's (`tools/cts/mali-gles2-tree.txt`, labels `T6S<n>R<nn>`
  are only names); the slice is named `<date>-imx95-gles2-<session>-<sha8>`,
  its `meta.txt` says `board=verdin-imx95`, its README names the board
  (`tools/cts/mali-slice-readme.py`). The chunk limit is the T6's
  (600 s + 3 s a case); a G310 with fewer cores may need `CHUNK_LIMIT`.
- Before `src/mali/frame.rs` takes the tile-buffer budget from the GPU's
  variant (`docs/notes/mali-g310-vs-g610.md`, the fix is under way
  elsewhere), a G310 v1-v3 may fail frames of 12 bytes a pixel or more; read
  such failures against that note before filing them.

## 4. What each result tells the Zephyr side

| Result (route (b)) | What it settles for Zephyr | Pre-study item |
|---|---|---|
| `panthor-hello`'s `gpu_id` line: arch major/minor, product major, revision | the arch (10.12 expected) and the revision our `mali::ident` must accept; the firmware file `arch10.12` | B2, U6 |
| `core_features` (the variant in its low byte: v1..v5), `shader_present` and its core count, `l2_features`, `tiler_features` (max levels), `mem_features`, `as_present` | the G310 variant, so the tile-buffer budget (16 or 32 KiB) and Mesa's model line that apply; the cores and AS count the Zephyr OSAL uses | B2, U6; `mali-g310-vs-g610.md` |
| panthor's probe lines (`CSF FW using interface v<x.y.z>`, the firmware's version and features) and the firmware file's hash on the card | the GLB interface the firmware answers with (1.5.0 expected from the same bytes as the T6's) | B3 |
| csif info (`cs_reg_count`, `unpreserved`, scoreboards) | the command-stream register budget our streams assume | B3, B4 |
| gl_test sweep verdicts against the T6's (`tools/mali/sweep_passsets.py`) | which differences are the G310's and which the OS side's: a case that passes here and fails on Zephyr is ours | B4, B5 |
| dEQP-GLES2 tally | the ES 2 result the owner asked for, and the expected set for B8 | B8 |
| kernel lines between the markers (job timeouts, resets, faults) | the hang classes the Zephyr reset path must handle | B5 |

Pre-study B1 says "boot the stock Linux once and record ... Panthor's probe
lines": on the stock image there are none (section 1), and kbase's lines
are not read. Under (iii) step 0 above records panthor's instead.

What route (b) does not settle: who releases `GPURESET` and the SCMI power
sequence our own kernel side must do (U5); under Linux the SM and the kernel
do it. If mainline brings the GPU up, its sequence (one `PD_GPU`, no perf
domain, the `GPU_CGC` clock) is a working public reference for B2.

## 5. The microSD card of (iii)

`tools/mali/panthor/imx95-sdcard.sh` builds the card on the host, without
root: no sudo, no loop mount, no user namespace (Ubuntu's AppArmor refuses
unprivileged ones) and no binfmt_misc. Its inputs are pinned in
`tools/cts/deps.toml` (board `verdin_imx95`): `IMX95_LINUX_SRC` (the v7.3
tarball, the sha256 kernel.org's signed `sha256sums.asc` gives),
`IMX95_DEBIAN_TOOLS` (trixie's debootstrap, the Debian archive keyring,
mtools), `IMX95_QEMU_SYSTEM` (trixie's qemu-system-arm with its libraries)
and `MALI_CSFFW`; the rootfs's packages come from snapshot.debian.org at
`20261101T000000Z`, checked by debootstrap against the pinned keyring.

| Piece | What |
|---|---|
| kernel | v7.3, arm64 `defconfig` + `tools/mali/panthor/imx95-sdcard.config` (panthor `=m`; SCMI with the raw interface beside the drivers, `ARM_SCMI_RAW_MODE_SUPPORT_COEX` and debugfs, for `tools/imx95_bbm_record.sh`; LPUART console; usdhc; ENETC v4 + EMDIO + NETC block control + DP83867; kexec, kexec_file, crash dump; virtio for the host test). Each line of the fragment is checked in the final `.config`. Built with Debian trixie's `aarch64-linux-gnu-` gcc 14 (`MALI_DEQP_TOOLCHAIN`, or `CROSS_COMPILE`; the Zephyr SDK's gcc has no `stdint.h` for the kernel's NEON code). Release `7.3.0`. |
| dtbs | `imx95-verdin-wifi-mallow.dtb` (model "Toradex Verdin iMX95 WB on Mallow Board") and `imx95-verdin-nonwifi-mallow.dtb` ("Toradex Verdin iMX95 on Mallow Board"), both on the card |
| boot | MBR, p1 FAT32 "BOOT" 256 MiB, bootable: `Image`, `dtbs/`, `boot.scr` (and its source `boot.cmd`), the kernel's config. `boot.scr` picks `imx95-verdin-${variant}-mallow.dtb` (U-Boot sets `variant` from the module's config block), `root=PARTUUID=1a95c0de-02 rootwait rw console=ttyLP2,115200`, then `booti`. |
| rootfs | p2 ext4: Debian trixie arm64 minbase with systemd, openssh-server, sudo, psmisc (fuser), procps (pkill), util-linux (setsid, findmnt), coreutils (timeout, stdbuf), python3, busybox (devmem), i2c-tools, kexec-tools, libubootenv-tool (fw_printenv), xz/zstd, libstdc++6; the modules; `arm/mali/arch10.12/mali_csffw.bin` from `MALI_CSFFW`. systemd-networkd DHCP on every wired port, systemd-resolved, timesyncd; `serial-getty@ttyLP2` with root logged in (the way in when the network does not come up; `IMX95_SD_CONSOLE_AUTOLOGIN=0` gives a plain getty); sshd with `PasswordAuthentication no`, root by key only, the key from `BOARD_SSH_PUBKEY` at build time; root has no password; ssh host keys are made on the first boot; `/etc/machine-id` empty (made on the first boot); debugfs is mounted by systemd. |

Why U-Boot boots it: Toradex's U-Boot for the Verdin iMX95 (BSP 7.x,
`u-boot-toradex` `toradex_imx_lf_v2024.04`, the same in mainline U-Boot)
runs bootstd, `bootcmd=bootflow scan -b`, with
`boot_targets=mmc1 mmc0 dhcp` (`board/toradex/verdin-imx95/verdin-imx95.env:3`).
`mmc1` is usdhc2, the Mallow's microSD slot (`mmc0` the eMMC, DT aliases);
bootstd scans the partitions with the bootable flag, as FAT or ext4, for
`extlinux/extlinux.conf` and `boot.scr`. The card has only `boot.scr`: an
extlinux file would take the dtb from `${fdtfile}`, which Toradex derives
as `imx95-verdin-${variant}-${fdt_board}.dtb` with `fdt_board=dev`, the
wrong carrier. The load addresses are U-Boot's (`kernel_addr_r=0x90400000`,
`fdt_addr_r=0x9c400000`: 192 MiB for the `Image`).

Build (outputs in `$IMX95_SD_OUT`, default `build/imx95-sdcard/`, which
is gitignored):

```sh
tools/cts/deps-env.sh --build --board verdin_imx95 IMX95_LINUX_SRC IMX95_DEBIAN_TOOLS IMX95_QEMU_SYSTEM MALI_CSFFW
tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN
BOARD_SSH_PUBKEY=<your public key file> tools/mali/panthor/imx95-sdcard.sh
#   steps kernel rootfs card verify, or any of them by name
```

The rootfs is made in two stages: debootstrap's first stage under fakeroot,
an ext4 image of it (`mke2fs -d`), then debootstrap's second stage and the
configuration commands run as PID 1 in qemu's `virt` machine, booting the
card's own kernel. `verify` reads the card image on the host (the partition
table, the FAT files against the build, the script image's CRCs, the kernel
config, the dtbs' compatibles and GPU node, `panthor.ko` and its
`modules.alias` line, the firmware's sha256 against the pin, root's key, no
host keys, no password, e2fsck). A boot of a copy of the finished rootfs
on `virt` (2026-11-21, by hand) reached `multi-user.target` with
`ssh.service` started (ssh's host keys made on that first boot) but showed
no serial login prompt on `ttyAMA0` within 40 minutes of TCG on a busy
host, so a host test that logs in is not in the script yet. `virt` is not
the board's DT in any case; it proves the rootfs and its init, not the
i.MX 95 drivers.

Write the card (the device is yours, never in a file; check it with
`lsblk` first, everything on it is replaced):

```sh
cd "${IMX95_SD_OUT:-build/imx95-sdcard}"
sha256sum -c card.img.sha256
sudo dd if=card.img of=/dev/<card> bs=4M conv=fsync status=progress
#   or, faster on a sparse image:  bmaptool create card.img > card.bmap
#                                  sudo bmaptool copy --bmap card.bmap card.img /dev/<card>
```

First boot: the card in the Mallow's microSD slot, power on. On the console
(ttyLP2, 115200): U-Boot's `bootflow scan` finds `mmc1`, the script prints
`imx95-sdcard: mmc 1:1 Image dtbs/imx95-verdin-<variant>-mallow.dtb`, then
the kernel, then a root shell (autologin). The ethernet port gets an address
by DHCP; ssh in as root with the key. If U-Boot boots the eMMC instead (an
environment whose `boot_targets` was changed), stop it at the prompt and boot
the card once, changing nothing that is saved:

```
bootflow scan -b mmc1
#   or, by hand:  load mmc 1:1 ${scriptaddr} boot.scr; source ${scriptaddr}
```

`setenv sd_extra_args crashkernel=256M` (or `sd_fdtfile <dtb>`) before
either adds kernel arguments (or names another dtb) for that boot only, as
long as it is not `saveenv`d.

Back to Toradex's image: power off, take the card out. Nothing on the eMMC
or in U-Boot's environment was changed.
