<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin iMX8M Plus: boot Zephyr from U-Boot over TFTP (plan, 2026-10-03)

Status: finished record, moved here from `driver/docs/imx8mp-netboot-plan.md` on 2026-10-11. Steps A, B and C passed on the board on 2026-10-03 (step C after the LCDIF-read fix at the end). The current design is docs/standalone-boot.md; the code is tools/board.sh, tools/uboot-netboot-lib.sh and driver/zephyr/lib/board/imx8mp/gc_wdog_arm.c.

Goal (user, 2026-10-03): a result counts only when the board's own boot
loader started Zephyr, never kexec. Until now the Verdin iMX8M Plus (Mallow
carrier, GC7000UL) had no such path at all: every run was a crash-kexec from
Torizon (`imx8mp-gc-bringup.md`, "The two things that were actually
wrong"). This is the AM62P's netboot (`am62p-netboot.md`) ported: a
one-shot U-Boot hook that fetches one image from this host and starts it,
the board back in Linux by itself on the next boot and on any failure, and an
image that arms WDOG1 itself.

Status: **written and tested on the host. On the board (2026-10-03): the
probe (step A) PASS; step B (WDOG proof image) PASS, the first image this
U-Boot started, and WDOG1 returned the board to Linux; step C (the
self-arm triangle) PASS since the panel console's fix: from the cold U-Boot
entry it leaves the same PICTURE record as on the kexec route. See
"Board run 2026-10-03: steps 0-A", "Board run 2026-10-03: steps B and C"
and "Board run 2026-10-03: step C root cause and fix" at the end.**
Board facts below were read from Linux on the board (read-only, 2026-10-03,
while another agent's dEQP chain owned it) and from a read-only copy of the
U-Boot boot partition. What only a board run can settle is marked
**unmeasured**.

Tools: `tools/board.sh imx8mp` (the runner, backend uboot-tftp; it replaced the board's own netboot runner in Step 1 of `one-method.md`), `tools/uboot-netboot-lib.sh`
(the U-Boot side, the same for the AM62P), `tools/tftp_oneshot.py`
(the TFTP server, unchanged). Image side: `CONFIG_GC_BOARD_WDOG_SELF_ARM`
(`driver/zephyr/lib/gc_board/gc_wdog_arm.c` over `driver/rust` `ip::imx_wdog`).

## 1. What the board has (read 2026-10-03)

Record: `records/imx8mp/netboot/env-2026-10-03.txt` (main checkout, gitignored;
156 lines, sha256 `5f8ad42d8b5f7bd9...`, MACs, IPs and serials redacted).
Next to it: `uboot-control-dt-2026-10-03.dts`, U-Boot proper's control DT
decompiled from the boot partition (the dtb at boot0 + 0x13ce70; boot0 copy
sha256 `b95031569ea4bf43...`).

| Fact | Value | Source |
|---|---|---|
| U-Boot | `2024.07-7.7.0+git.3f772959501c` (Toradex BSP 7.7) | `ver=` in the env; banner in boot0 |
| Env storage | `/dev/emmc-boot0` (mmcblk2boot0) offset `-0x2200`, size `0x2000`, **one copy** | `/etc/fw_env.config`; mainline `verdin-imx8mp_defconfig` `CONFIG_ENV_OFFSET=0xFFFFDE00`, `ENV_SIZE=0x2000`, `SYS_MMC_ENV_PART=1` |
| boot0/boot1 | 8192 sectors each, `force_ro=1` (libubootenv lifts it for a write; Torizon's `tdxargs` with `crashkernel=` were written that way) | sysfs |
| `bootdelay` | `0` | env |
| `bootcmd` | `run cm_boot; run distro_bootcmd`. **`cm_boot` is not defined** anywhere (env or binary), so `run cm_boot` fails and the distro boot (mmc1, mmc2, dhcp) boots Torizon | env; strings of boot0 |
| Compiled-in default `bootcmd` | the same string: a bad env CRC (`*** Warning - %s, using default environment` is in the binary) still boots Linux | strings of boot0 |
| bootcount | `bootcount=0`, `bootlimit=3`, `altbootcmd=env set rollback 1; run bootcmd`, `upgrade_available=0`: Torizon's OTA rollback, inactive (`CONFIG_BOOTCOUNT_ENV` counts only while `upgrade_available` is set) | env; mainline defconfig `BOOTCOUNT_LIMIT`, `BOOTCOUNT_ENV` |
| `tdxargs` | `crashkernel=256M@0x60000000 clk-imx8mp.mcore_booted=1` (the crash route's reservation; harmless for a U-Boot entry) | env, `/proc/cmdline` |
| Network | `ethact=ethernet@30bf0000` (EQOS), `ethprime=eth0`; `ethaddr` equals Linux `ethernet0`'s MAC, so U-Boot's DHCP gets the usual lease. No `serverip`/`ipaddr` | env; sysfs |
| Addresses | `loadaddr`/`kernel_addr_r=0x48200000`, `fdt_addr_r=0x50200000`, `scriptaddr=0x50280000`, `ramdisk_addr_r=0x50300000`. Zephyr is linked at **0x60000000** (`a53_ddr_section`, 256 MiB, board DTS); GPU carve-out 0x58000000 (64 MiB, no-map) | env; `driver/boards/toradex/verdin_imx8mp/*.dts` |
| DRAM | 4 GiB at 0x40000000 | `/proc/iomem` |
| Network commands | `dhcp`, `tftpboot`, env `tftpblocksize`, `tftpwindowsize`, `tftptimeoutcountmax`, `netretry`; **no `tftpdstp`** (port 69 only); `go` ("start application at address 'addr'", `## Starting application at 0x%08lX ...`); `booti` | strings of boot0 |
| **Watchdog in U-Boot** | the driver model watchdog is built in: `imx_wdt`, `wdt-reboot`, `WDT:   Started %s with%s servicing %s (%ds timeout)`, `WDT:   Not starting %s`, `u-boot,autostart`/`u-boot,noautostart`. The control DT has `watchdog@30280000` `status = "okay"`, `fsl,ext-reset-output`, **no `u-boot,noautostart`, no `timeout-sec`** | strings of boot0; the control DT |
| WDOG1 under Linux | `imx2+ watchdog`, active, timeout 30 s (systemd `RuntimeWatchdogUSec=30s`, `RebootWatchdogUSec=1min`), bootstatus 0 | sysfs, `systemctl show` |
| M7 | `remoteproc0` offline; U-Boot starts nothing on it (`cm_boot` undefined) | sysfs |
| SNVS record | readable at `/sys/bus/nvmem/devices/30370000.snvs:snvs-lpgpr0/nvmem` (it held the dEQP chain's record) | od |

### The watchdog question, and why the image arms WDOG1 anyway

The brief said "U-Boot leaves WDOG1 stopped on a cold boot". The binary makes
the opposite likely. U-Boot v2024.07 `drivers/watchdog/wdt-uclass.c`
(`wdt_pre_probe`: `autostart = IS_ENABLED(CONFIG_WATCHDOG_AUTOSTART)`, then the
DT's `u-boot,noautostart` / `u-boot,autostart`; `init_watchdog_dev` starts the
watchdog with `timeout-sec`, default `CONFIG_WATCHDOG_TIMEOUT_MSECS`), its
Kconfig (`WATCHDOG_AUTOSTART` `default y`; `WATCHDOG_TIMEOUT_MSECS` `default
60000` for this SoC) and mainline `configs/verdin-imx8mp_defconfig`
(`IMX_WATCHDOG=y`, `SYSRESET_WATCHDOG=y`, no `# CONFIG_WATCHDOG_AUTOSTART is
not set`) together predict that **U-Boot starts WDOG1 with 60 s and services it
only while U-Boot runs**. The Toradex tree's own defconfig could not be
fetched (git.toradex.com answered with a bot check), so this is a prediction,
**unmeasured**. U-Boot's driver writes `WCR = WDZST | WDBG | WDE | SRS | WDA
| WDT | WT` (`imx_watchdog.c` `imx_watchdog_init`), and the i.MX WDOG cannot
be stopped once enabled (Linux's imx2_wdt treats it as such), so after `go`
such a watchdog resets the board 60 s after U-Boot's last service unless the
image feeds it.

Neither outcome is safe to build on, so the image arms WDOG1 itself in every
case and **reports which case it found** (section 4): WCR as found, in the
SNVS record. The first image run (step B) settles the question with a
register read. If U-Boot does autostart, the gap the AM62P has between `go`
and the arming (`am62p-netboot.md` section 4, "Residual gap") does not
exist here: U-Boot's own 60 s watchdog covers it.

## 2. The boot flow: a one-shot armed from Linux

The same hook as the AM62P, byte for byte (`tools/uboot-netboot-lib.sh`; the
reasoning is `am62p-netboot.md` section 2). Only `yako_addr` differs:

```
yako_bootcmd_prev = run cm_boot; run distro_bootcmd
yako_addr         = 0x60000000
yako_hook         = if test -n "${yako_once}"; then echo ...; setenv yako_once;
                      if saveenv; then echo ...; run yako_${yako_src};
                      else echo yakogl-netboot: saveenv failed, booting Linux; fi; fi;
                    <the default: docs/standalone-boot.md>
yako_net          = setenv autoload no; setenv netretry no; setenv tftptimeout 1000;
                    setenv tftptimeoutcountmax 5; setenv tftpblocksize 1468;
                    setenv tftpwindowsize 8;
                    if dhcp && tftpboot ${yako_addr} ${yako_server}:${yako_file}; then
                      echo yakogl-netboot: go ${yako_addr};
                      dcache off; icache off; go ${yako_addr};
                    fi; echo yakogl-netboot: fetch failed, booting Linux
bootcmd           = run yako_hook; run cm_boot; run distro_bootcmd
```

Each value is one line in the env; the library holds the exact strings and
`status`/`run` compare the board's copy with them.

Arm (each run, one `fw_setenv -s` write): `yako_once=<token>`,
`yako_file=yakogl/<token>.bin`, `yako_server=<this host>`.

One cycle: `systemctl reboot` -> SPL -> TF-A -> U-Boot -> the hook sees
`yako_once`, deletes it, `saveenv` -> only then DHCP, TFTP of
`yakogl/<token>.bin` to 0x60000000, `dcache off; icache off; go 0x60000000`
-> Zephyr -> its own reset (PSCI `SYSTEM_RESET` at the end of the sample, or
WDOG1) -> SPL -> U-Boot -> empty hook -> `run cm_boot` (fails) -> distro boot
-> Torizon.

Why not the mechanisms in the env already: `altbootcmd`/`bootcount` are
Torizon's OTA rollback (running `altbootcmd` sets `rollback 1`); `preboot`
holds Toradex's `fdtfile` default; `cm_boot` is Toradex's Cortex-M hook and
belongs to whoever boots the M7. The same reasoning as the AM62P's.

### `go`, not `booti`

`zephyr.bin` carries the AArch64 Image header (`CONFIG_AARCH64_IMAGE_HEADER=y`
in the board defconfig): `text_offset` 0, `flags` 2, `image_size` 0x2971000
for the gc_gl_triangle image (read from the built `zephyr.bin`). `booti`
places an image whose `flags` bit 3 is clear at the start of DRAM: U-Boot
v2024.07 `arch/arm/lib/image.c:70-75` (`dst = gd->bd->bi_dram[0].start`, then
`ALIGN(dst, SZ_2M) + text_offset`), i.e. 0x40000000, and an image linked at
0x60000000 does not run there. `go` jumps to the load address (`cmd/boot.c`,
`do_go_exec`) and passes no device tree, which Zephyr does not need. This is
the AM62P finding (SafeUI measured it there on 2026-09-05); for this board it
follows from the same code and is **unmeasured**. `dcache off` cleans the
D-cache and turns the MMU off before the jump, so Zephyr enters with MMU and
caches off, as it does from kexec's purgatory.

Footprint: the runner refuses an image whose `image_size` runs past
0x70000000 (the 256 MiB `a53_ddr_section`); the gc_gl_triangle image's is
43,454,464 bytes (41 MiB: the framebuffer and heaps are BSS). TFTP writes only
the file (2.5 MB); Zephyr zeroes the rest. U-Boot relocates itself near the
top of the first 4 GiB (Linux's `/proc/iomem` shows the region U-Boot and its
EFI tables leave reserved at 0xfaa8e000..0xfeaa4fff), far from 0x60000000;
U-Boot's LMB check (`trying to overwrite reserved memory...` is in the binary)
refuses the load rather than overwriting U-Boot if that is ever wrong.

## 3. What a cold U-Boot entry lacks that the crash-kexec entry had

| State | crash-kexec entry (today) | U-Boot `go` entry | Where our code handles it |
|---|---|---|---|
| Exception level | EL2 (Linux booted at EL2) | EL2 (TF-A hands U-Boot EL2) | Zephyr's arm64 reset path; the same on both entries |
| GIC | Linux's state: split EOI mode, lines configured, stray interrupts pending | TF-A's: distributor and the boot CPU's redistributor initialised, EOImode 0, nothing pending | `gc_board_gic.c` clears EOImode (a no-op now); `gc_irq_guard.c` claims unowned lines (harmless when none fire); Zephyr's GICv3 driver wakes the redistributor |
| Caches / MMU | off (purgatory) | off (`dcache off; icache off` before `go`) | none needed |
| Arch timer | running, CNTFRQ set | running, CNTFRQ set by TF-A | none needed |
| WDOG1 | running, 30 s (systemd), fed by Linux until the crash | running with U-Boot's 60 s **or** stopped (section 1, unmeasured) | **`CONFIG_GC_BOARD_WDOG_SELF_ARM`** (section 4) |
| WMCR power-down counter | cleared by Linux and U-Boot | cleared by U-Boot (`arch/arm/mach-imx/imx8m/soc.c:612`, `imx_set_wdog_powerdown(false)`) | `ip::imx_wdog::arm` clears and reads back PDE if it is ever still set |
| Display chain | the crash route leaves LCDIF, DSIM, bridge and backlight scanning Linux's picture; `gc_panel_console` adopts it at PRE_KERNEL_1 | **dark**: this U-Boot has no display driver; the media power domains, LCDIF, DSIM, SN65DSI84 and backlight are as reset left them | `GC_COMMON_PANEL_CONSOLE_COLD_BRINGUP` (default y) calls `imx8mp_display_on()` at POST_KERNEL when the sample has `CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP` (gc_gl_triangle does): MEDIAMIX on (Rust `soc::imx8mp::media`), then the chain. **From a true cold state: unmeasured** (its runs were after Linux's `device_shutdown`, not after a reset). Nothing printed before POST_KERNEL reaches the panel |
| GPU power and clocks | Linux's galcore had the GPU domains on | GPU_SHARE_LOGIC / GPU_3D / GPUMIX off from reset | Rust `gpu::vivante` power (`gpc_power_up`: PGC 7 and 9, the GPUMIX handshake; `driver/docs/vivante-kernel-rust.md`). **From reset: unmeasured** |
| Ethernet DMA | the crash route shuts nothing down (the image lives in the crashkernel reservation Linux never handed to the EQOS) | U-Boot halts the EQOS after the transfer | none needed |
| M7 | offline (remoteproc0) | not started (`cm_boot` undefined) | none needed; nothing in our images uses it |
| SNVS LPGPR | kept from before the jump (the runner cleared it) | expected kept across the reboot (SNVS low-power domain; U-Boot 2024.07 and TF-A do not touch it, `gc_result.h`) | the runner clears it before the reboot; **survival of a U-Boot reboot: unmeasured** (step A checks it) |

## 4. Watchdog: WDOG1 armed by the image

`CONFIG_GC_BOARD_WDOG_SELF_ARM` (default n; `driver/zephyr/lib/gc_board/Kconfig`):

1. **PRE_KERNEL_1 priority 1** (after the WDOG and SNVS mappings at 0):
   `device_map` WDOG1 (0x30280000) and the CCM (0x30380000), then
   `yd_imx_wdog_arm()` (Rust, `driver/rust/src/ffi/imx_wdog.rs`):
   - the CCM WDOG clock root (`ROOT[114]`) enabled and the WDOG1 gate
     (`CCGR83`) opened, each only if it is not already (hal_nxp `fsl_clock.h`
     `kCLOCK_RootWdog`, `kCLOCK_Wdog1 = CCM_TUPLE(83U, 114U)`);
   - `ip::imx_wdog::arm(60 s, WDOG_B)`: read `WCR` and `WMCR`; clear
     `WMCR.PDE` if set and read it back; write `WCR` (a running WDOG keeps
     its bits and gets `WT` = 0x77; a stopped one gets U-Boot's own value
     0x773f, WDZST|WDBG|WDE|WDT|SRS|WDA|WT); service (0x5555, 0xAAAA to
     `WSR`); read `WCR` back and refuse unless WDE is set, `WT` is 0x77 and
     `WDT` is set;
   - SRS and WDA are 1 in every write (0 there asserts a reset now); the
     host test checks this for all 65,536 values `WCR` could be found with.
2. The timeout: `(WT + 1) x 0.5 s` (hal_nxp `WDOG_WCR_WT` value list:
   0 = 0.5 s ... 0xFF = 128 s), so 60 s is `WT` = 119 = 0x77, exactly. The
   0.5 s step does not come from a CCM root this code sets, so unlike the
   AM62P's RTI there is no rate to read back; reading `WT` back is the check.
   The IMX8MPRM Rev. 3 WDOG chapter was not on this host: its section numbers
   are **a citation pending** (the module header says so); nothing is taken
   from it. There is no window to respect: the hal_nxp field list has no
   closed window like the RTI's 50 %, and a service at any time reloads the
   counter (this is how gc_sample.c has fed it since 2026-09-20).
3. Success: `printk("gc_wdog_arm: armed WCR 0x.... -> 0x.... (was running|was
   stopped), WMCR ..., flags ...: expiry 60000 ms, fed for N s")` (at
   PRE_KERNEL_1 the panel is not up yet and the carrier has no UART: the line
   is the runner's marker in the binary; the record is what a reader sees)
   and a kind WDOG tag 1 breadcrumb in SNVS (`gc_result.h`: WCR as found and
   now, the flags, WMCR, the uptime), replaced by the sample's own records.
4. Refusal: `gc_wdog_arm: FAILED (rc, refusal ...)`, kind WDOG tag 2 (a
   verdict) and a PSCI `SYSTEM_RESET` through SMC at once. The record says
   which read-back failed.
5. Bounded feeding: a 1 s `k_timer` from POST_KERNEL 0, and **every** other
   feeder in gc_board (the hold loop, the fatal handler,
   `GC_SAMPLE_WDOG_TIMER_FEED`) goes through `gc_sample_service_wdog()`,
   which now asks `gc_wdog_arm_may_feed()`: no service after
   `CONFIG_GC_BOARD_WDOG_SECONDS` (default 300) of uptime. A hung image
   returns at most 300 + 60 s after it started, whatever hangs. The default
   hold (60 s PASS) fits; a hold code's up to 448 s would be cut by the
   watchdog (the Kconfig help says so).
6. What resets the board: `WCR.WDT` makes the timeout assert `WDOG_B`
   (`fsl,ext-reset-output` in both U-Boot's and Linux's DT). Every kexec run
   that ended in a watchdog reset went that way (`imx8mp-gc-bringup.md`:
   "WDOG1 then resets the SoC (128 s on the normal route, sooner on the crash
   route)"). After the reset: SPL -> U-Boot -> the hook finds nothing ->
   Linux.
7. The same image on a kexec entry: WDOG1 is found running at 30 s, re-timed
   to 60 s and fed under the same bound.

The proof image (`CONFIG_GC_BOARD_WDOG_PROOF`): armed, **never** fed (the gate
says no from the arming on), kind WDOG tag 3 as a verdict, rewritten when
`gc_sample_finish()` is reached, which then sleeps instead of resetting; a
one-shot timer `CONFIG_GC_BOARD_WDOG_PROOF_BACKSTOP_SECONDS` (90) after the
arming writes tag 4 and resets through PSCI. So: tag 3 left behind = WDOG1
reset the board; tag 4 = it did not, and software did.

Host tests (`cargo test -p yakogl-drivers`, 224 tests, 14 of them new):
`ip::imx_wdog` 10, `ffi::imx_wdog` 4. They cover:
- the arithmetic: 60 s -> 0x77, 30 s -> 0x3b, the 0.5 s and 128 s edges,
  every `WT` round-trips, and 0, 1, 499, 750, 59,999, 60,001 and 128,500 ms
  and `u32::MAX` refused;
- the `WCR` values: U-Boot's 0x773f for a stopped WDOG, Linux's 0x3b3d ->
  0x773d;
- the exact access sequence of a cold arm;
- refusals: a stuck PDE refused with no `WCR` write, each read-back refusal,
  and an unrepresentable timeout writing nothing;
- the CCM writes (0xb904 ENABLE, 0x4534 = 3), and none when the root and gate
  are already on;
- the C header's numbers.

A deliberately wrong expectation (0x773e) fails the run.

## 5. The result channel (no UART)

- **SNVS LPGPR** (`gc_result.h`), as on the kexec route: the runner zeroes the
  four words (and writes a selector when `SEL` is set) before the reboot and
  reads them after Linux is back; the image writes, magic last. A zero word 0
  after the cycle means nothing was written this run.
- **The TFTP server's log** (`tftp_oneshot.py --log`): the only witness of the
  U-Boot leg. `refused ... file=yakogl/<token>.bin cause=not_found` (probe),
  `done ... bytes=... seconds=...` (an image was served whole), nothing (the
  hook never asked: hook, `saveenv`, DHCP or the PHY). The runner first proves
  the log is that server's by finding the canary's `done` line in it.
- The time from the reboot to the new boot's ssh (`back_after`), and that
  boot's uptime (a smaller uptime than the time since the reboot is what
  proves a new boot).

## 6. Failure analysis: why none of these bricks the board

| Failure | What happens | Why it is safe |
|---|---|---|
| `fw_setenv` interrupted (install or arm) | one env copy: a bad CRC makes U-Boot use its default env, whose `bootcmd` is the same `run cm_boot; run distro_bootcmd` | Linux boots; `install` saved the whole env first, and Linux-side `fw_setenv` restores it (Torizon's `tdxargs`, with the crash route's `crashkernel=`, are in it) |
| `saveenv` fails in U-Boot | `if saveenv` false: no fetch, old `bootcmd`, Linux | the image loads only after the clear is on eMMC |
| `saveenv` interrupted by power loss | bad CRC, default env, Linux | as the first row |
| DHCP fails | `netretry no`: one failure, Linux | bounded; if U-Boot autostarted WDOG1 it keeps servicing it meanwhile |
| TFTP server down | 5 x 1 s timeouts, Linux | bounded by `tftptimeout`, `tftptimeoutcountmax`, `netretry` |
| File not staged / already served / older than 300 s | `File not found` at once, Linux | the server is one-shot too |
| Transfer cut half way | `tftpboot` fails, no `go`, Linux | `&&` |
| Image hangs after PRE_KERNEL_1 priority 1 | WDOG1 (armed by the image, 60 s) resets after the bound; the next boot's hook is empty; Linux | needs the self-arm; the runner refuses images without its marker |
| Image hangs before the arming | **if U-Boot autostarted WDOG1**: its 60 s resets the board. **If not**: nothing does | the residual gap: the arming runs right after the two PRE_KERNEL_1 mappings (a few ms of boot). Step B tells which case this board is in |
| Arming refused | record tag 2, PSCI reset at once, Linux | the refusal is a verdict in SNVS |
| Image ends normally | PSCI reset, empty hook, Linux | cleared before the load |
| Runner dies after arming, before the reboot | its trap disarms and unstages; if killed hard, the next reboot by anyone fires the hook, and the server refuses a file older than 300 s, so that boot gets `File not found` and the one-shot is gone | two limits: the env one-shot, and the server's one-shot plus age |
| Host goes away mid-cycle | TFTP timeouts, Linux | the way back needs no host |
| bootcount / rollback | untouched: `upgrade_available=0`, `altbootcmd` never runs | as today |

What could still need a person: a hang before the arming **if** U-Boot did
not autostart WDOG1, and a broken arming; both are answered by step B, which
has a person at the board.

## 7. On "go": the board steps, in order

Preconditions (host side, no board):
- The TFTP server on port 69. One already runs on this host
  (`tftp_oneshot.py`, root `<main checkout>/build/tftp`, bound to the host's
  LAN address, log `build/tftp.log`, started 2026-10-03 10:50 for the AM62P
  netboot). One server serves both boards, each run with its own token, so no
  new privilege is needed if its owner agrees to share it.
- The board is free: ask the owner of the dEQP chain. The runner takes the
  board lock and refuses while a kexec image is loaded.

0. `tools/board.sh imx8mp status` (read-only).
1. `tools/board.sh imx8mp install`: saves the whole env to
   `records/imx8mp/netboot/env-before-install-<time>.txt`, writes the five
   variables in one `fw_setenv -s`, reads them back. No reboot. Proves that
   libubootenv writes boot0 here. Unattended.
2. **A. `probe`**: no image, so no watchdog is involved. It arms a token that
   is never staged and reboots. Pass:
   - the server logs `refused ... cause=not_found` from the board;
   - `yako_once` is gone;
   - a new boot answers;
   - the SNVS record still holds the zeros written before, so it survived a
     U-Boot reboot untouched.

   This proves the hook, `saveenv`, the EQOS in U-Boot, DHCP, TFTP and the way
   back. Unattended.
3. **B. the WDOG1 proof** (the proof image below): `run`. **A person at the
   power switch** for about 5 minutes. The image is the first one ever
   started by this U-Boot, so a hang before the arming, or inside it, is not
   covered by anything we wrote. Expected results:
   - the record is kind 13 tag 3 (`yakogl-tools record` decodes it), with
     `WCR now` 0x773f or 0x77xx with WDE set. `WCR found` answers section 1's
     question: WDE set with WT 0x77 means U-Boot autostarted WDOG1 for 60 s;
     WDE clear means it did not.
   - `back_after` is about the image's arming uptime + 60 s + the Linux
     boot.

   Tag 4 means WDOG1 did not reset the board (software did, at 90 s). Netboot
   stays off until that is understood.
4. **C. gc_gl_triangle, self-arm image**: `run`. Unattended once step B has
   passed. If B showed U-Boot does not autostart WDOG1, the first run keeps
   the person. Its PICTURE record is compared with the same image's under the
   crash route (the i.MX8MP kexec runner of the time; today `BOARD_BOOT=kexec-dev tools/board.sh imx8mp run`): the CRCs must match. The cold
   display path and the GPU power-up from reset (section 3) are first
   exercised here.
5. If anything fails: `disarm` (a pending one-shot) or `uninstall`, both
   from Linux.

Images (2026-10-03, this branch, `-DEXTRA_CONF_FILE=prj_parity.conf`, a
1024x600 window). The build logs have no gcc warning, only one rustc
future-incompatibility note about the toolchain's own `core`, which the
control has too. Each image carries its build time (the panel console's
first line, `GC_BUILD_STAMP`), so a rebuild of the same tree has another
sha256; the row is the image, not the tree.

| gc_gl_triangle image | options | `zephyr.bin` bytes | sha256 | marker |
|---|---|---|---|---|
| control | none | 2,529,692 | `fd19e0a089f1fbd2409e00a1c99c42b01a77440ff5241fa6337d4c5f94ec1925` | absent (the runner refuses it) |
| self-arm (step C) | `GC_BOARD_WDOG_SELF_ARM=y` | 2,533,880 | `d0c415c603b85bb7278902927c7d77acf9403c34f1cfc5bce0b376f35bc46f00` | present |
| WDOG1 proof (step B) | as self-arm, plus `GC_BOARD_WDOG_PROOF=y` | 2,533,880 | `19f0b2abba877c06f8a758814dc21a838b4d1470b3f51d6e1c47a594e43eee6a` | present |

`nm` of the self-arm ELF lists `wdog_arm_init`, `gc_wdog_arm_may_feed`,
`feed_tick`, `gc_result_breadcrumb` and `yd_imx_wdog_arm`. Its disassembly
reaches WDOG1 only with 16-bit `ldrh`/`strh` (0x5555 and 0xAAAA to `WSR`) and
the CCM with 32-bit accesses. The proof ELF has `proof_backstop` in place of
`feed_tick`.

## 8. Cycle time

Measured on the crash route (`records/imx8mp/`, every kexec jump log of the time,
211 rows, counted 2026-10-03): "linux back after" (jump to ssh, the image's
run included, so it depends on the image) is 92-116 s in 168 rows (mostly
105-110 s), 224-341 s in 42 and 75 s in one. Before each jump the
chain also waits for a Linux uptime of 3 minutes
(a former hand-over note).

Predicted for the netboot (**unmeasured**): the same image run and the same
Linux boot afterwards, plus these legs:
- Torizon's shutdown, which the crash route skips: est. 10-20 s with its
  containers;
- SPL, TF-A and U-Boot: est. 3 s;
- DHCP: 1-3 s;
- TFTP of 2.5 MB: about 1 s at the host-to-host rates `tftp_oneshot.py`
  measured (U-Boot's own rate is unmeasured).

So a one-shot cycle is about 15-25 s **longer** than a crash-route jump. If
the 3-minute uptime wait is needed by the crash route only (the handoff does
not say why it is there), the netboot does not need it and wins back far more
than that. The AM62P plan's chain mode (`am62p-netboot.md` section 6)
would remove the Linux leg between images on this board too; it is designed
there, not written.

## Board run 2026-10-03: steps 0-A

Steps 0, 1, A and an uninstall, on the lead's "go" with the user's approval
of one install write for the probe and an uninstall right after. Build
`de1ab5cb` (the runner's first line); no image ran. Files (main checkout,
`records/imx8mp/netboot/`, gitignored): `env-saved-20261003-145413.raw` (the
whole env before install, 73 lines, unredacted because it is the restore and
diff reference, mode 600), `env-before-install-20261003-145425.txt` (the
runner's own copy), `env-after-uninstall-*.raw`, and `probe-20261003-145441/`:
the runner's output, `dmesg -w` until the reboot (657 lines), `journalctl -k`
of the boot before (644) and after (549); the journal is volatile, so the old
boot exists only there. Addresses are redacted in the probe directory.

Before each step the board was idle: uptime about 2 h, `remoteproc0`
offline, `kexec_loaded` 0, `kexec_crash_loaded` 0, `/dev/galcore` unheld. The
TFTP server was the one already on this host's port 69.

0. `status`: `bootcmd=run cm_boot; run distro_bootcmd`, every `yako_*` empty,
   `install: absent`.
1. `install` printed `imx8mp-netboot: installed; bootcmd = 'run yako_hook; ' +
   the saved bootcmd` (rc 0). Read back: `bootcmd=run yako_hook; run cm_boot;
   run distro_bootcmd`, `yako_bootcmd_prev=run cm_boot; run distro_bootcmd`,
   `yako_addr=0x60000000`, `yako_hook` and `yako_net` equal to the library's
   strings, `yako_once` empty. `fw_printenv` diffed with the saved copy: those
   five lines and nothing else.
2. **A. `probe`, PASS.** The runner printed:

   ```
   netboot probe rc=0 verdict=probe_ok tftp=not_found back_after=24s uptime=15s record=00000000,00000000,00000000,00000000
   ```

   The TFTP server's log (`<HOST>`/`<BOARD>` redacted):

   ```
   1791006884.174 tftp: done peer=<HOST>:59662 file=yakogl/canary-probe-none-1791006884 bytes=7 ... renamed_to=canary-probe-none-1791006884.served
   1791006895.298 tftp: refused peer=<BOARD>:2440 file=yakogl/probe-none-1791006884.bin cause=not_found
   ```

   The first line is the runner's canary (the server answers, and this log is
   its log); the second is U-Boot on the board asking for the never-staged
   file, about 9 s after the reboot command (armed at 1791006884-886). The
   boot ID changed from `f6831c87...` to `b3645549...` (first kernel line
   05:55:01 UTC); `yako_once` read back empty; the SNVS record kept the zeros
   written before the reboot. So the hook fired, `saveenv` cleared the
   one-shot before anything was fetched, the EQOS and DHCP work in U-Boot,
   TFTP reached this host, the distro boot took the board back to Torizon,
   and U-Boot and TF-A leave the SNVS LPGPR alone across a reboot.
3. `uninstall` printed `imx8mp-netboot: uninstalled; bootcmd restored`
   (rc 0): `bootcmd=run cm_boot; run distro_bootcmd`, no `yako_*` variable.

`fw_printenv` after the uninstall, diffed with the copy saved before the
install, differs in one line: `carrier_serial#` (the stored value changed
from an older serial to the current carrier's; the numbers are in the
gitignored copies). Cause: the hook's `saveenv` stores U-Boot's whole RAM
environment, and U-Boot sets `carrier_serial#` at every boot from the carrier
board's config block: U-Boot v2024.07 `board/toradex/common/tdx-common.c`,
`settings_r()`, lines 145-160 (`read_tdx_cfg_block_carrier()`, then
`env_set("carrier_serial#", tdx_car_serial_str)`, under
`CONFIG_TDX_CFG_BLOCK_EXTRA`; this board's binary has that branch's
`MISSING TORADEX CARRIER CONFIG BLOCKS` string). `serial#` is set the same
way from the module's block (line 142) and did not change. The stored copy
was stale from an earlier carrier; the RAM value every boot used was already
the new one. **It was not written back**: U-Boot would override it on the
next boot anyway. Every netboot run's `saveenv` persists U-Boot's runtime
variables like this; a diff of the env around a run shows it.

Board after: Linux, `systemctl is-system-running` = `running`,
`crashkernel=256M@0x60000000` on the command line, `remoteproc0` offline, no
kexec image loaded, `watchdog0` active with 30 s; the env as before but for
`carrier_serial#`.

Not run: steps B and C, the first images this U-Boot starts, with the WDOG1
self-arm. They wait for a person at the board's power switch (section 7).

## Board run 2026-10-03: steps B and C

Steps B and C ran at 16:39-16:48 on the board, one image each, build
`950c1ea4` (the runner's first line), after one install write and an
inert hook left in place (see the end of this section). Records: main
checkout, `records/imx8mp/netboot/stepsBC-20261003-1639/` (gitignored; 11
files and an `INDEX.txt` with sha256 and line counts; addresses, MACs and
serials redacted in the copies, 0 hits by grep).
`runB.out`, `runC.out`, `runB/`, `runC/`, `ctl.out`, `ctl/selfarm-kexec.log`,
`dmesg-oldboot.log` (618 lines), `newboot.txt`, `bootid-before`,
`install.out`, `tftp-lines.txt` (12 server lines: for each of the two tokens the
image's three lines and the runner's canary probe's three).

### Step B: the WDOG proof image: PASS

The runner's line:

```text
netboot proofB: build 950c1ea4 image sha256 19f0b2abba877c06 footprint 43454464 token proofB-19f0b2abba877c06-1791013154
netboot proofB rc=0 verdict=done tftp=done:2533880B/0.040s back_after=86s uptime=16s record=594b0d03,773f773f,00000001,00001248
magic ok, kind 13 seq 3
WDOG proof: armed and not fed at uptime 4680 ms
  WCR found 0x773f (WDE, WT 0x77 = 60.0 s)
  WCR now   0x773f (WDE, WT 0x77 = 60.0 s)
  WMCR found 0x0000, flags was-running, refusal: none
```

The TFTP server's lines for the image: `bytes=2533880 ... seconds=0.040
mib_per_s=59.9 windows_resent=0`. The record words
`594b0d03 773f773f 00000001 00001248` decode as kind 13 (WDOG), seq 3,
armed at 4680 ms (0x1248). Reading:

- U-Boot **did start WDOG1 itself**: the image found WCR 0x773f
  (enabled, 60 s) before it wrote anything. The "stopped" branch of
  section 1 did not occur on this board (this resolves that open question).
- The image did not feed it, and WDOG1 reset the board: the board came back
  to Linux 86 s after the hook ran (`back_after=86s`; the new boot's uptime
  was 16 s at the first read). Boot ID before `b3645549-...`, after
  `c29e5808-...` (`bootid-before`, `newboot.txt` line 2).
- Not measured here: the time from arming to the reset to the second. The
  record gives the arming time; `back_after` is the runner's wall clock for
  the whole cycle, which includes the firmware boot.

### Step C: the self-arm triangle: FAILS, safely

The runner's line:

```text
netboot selfarmC: build 950c1ea4 image sha256 d0c415c603b85bb7 footprint 43454464 token selfarmC-d0c415c603b85bb7-1791013320
netboot selfarmC rc=0 verdict=done tftp=done:2533880B/0.040s back_after=97s uptime=21s record=594b0d01,773f773f,00000001,00002e30
magic ok, kind 13 seq 1
WDOG armed (breadcrumb) at uptime 11824 ms
```

TFTP: `bytes=2533880 ... seconds=0.040 mib_per_s=60.4 windows_resent=0`.
The record `594b0d01 773f773f 00000001 00002e30` is kind 13, **seq 1**: the
"armed (breadcrumb)" record the image writes when it arms WDOG1, at uptime
11824 ms (0x2e30). There is **no PICTURE record** (kind 4) behind it. The
board came back to Linux 97 s after the hook, which is WDOG1's 60 s after
the arming plus the firmware boot; nothing in the record says what the
image was doing in those 60 s.

**The runner's `verdict=done` is wrong for this record.** `done` means "a
record exists and the board is back"; a breadcrumb-only record is not a
result. A step C that has not produced its picture must not be `done`. This
is an open runner defect, owned by branch `imx8mp/cold-entry` (another agent;
not touched here).

### The control arm: the same image on the crash/kexec route

The i.MX8MP wrapper-jump runner of the time (build `950c1ea4`; deleted in Step 1) ran the same image
(`d0c415c603b85bb7`, 2533880 B) with route `--crash`:

```text
2026-10-03 16:48:06 timeline: jump 0s, ping lost 10s, ping back 80s, ssh back 80s, route --crash
2026-10-03 16:48:06 record:  594b040e ff403f80 35706c4e 04000258 18.41
```

`594b040e ff403f80 35706c4e 04000258` is kind 4 PICTURE, equal to
`RESULTS.md`'s `gc_gl_triangle` baseline rows (`grep 594b040e ff403f80`:
2026-09-23 parity record, centre r=128 g=63 b=64, column CRC 0x35706c4e).
The record is the same words, so the image draws the right picture on the
kexec route (seq differs by design, not compared here).

### Finding

One image, two entries: kexec draws the picture, the cold U-Boot entry stops
between the WDOG arming (11824 ms) and the picture. The image is fine. The
suspects are the rows of section 3 (the display chain is dark and the GPU
domains are off at a cold entry; the first of them that the image does not
bring up itself). **The cause is not found.** Which row it is needs the
image to leave a record after each bring-up stage; that work is on
`imx8mp/cold-entry`. Step C is therefore not passed and the bootloader-boot
bar is not met for this board yet. It failed safely: WDOG1 returned the
board to Linux in both B and C.

Not run: any further image. The hook installed for this iteration is still
in the board's U-Boot env (inert, per the lead; not re-read from the board in
this task; `install.out` says the env was saved before installing, as
`env-before-install-20261003-163911.txt`). It stays for the next iteration;
it needs an `uninstall` when that is over.

## Board run 2026-10-03: step C root cause and fix

Step C above failed with only the WDOG "armed" breadcrumb in the record.
Two images, each run twice (netboot, then the same image on the
crash/kexec route as the control), 17:08-17:29. Records: main checkout,
`records/imx8mp/netboot/coldentry-20261003-1708/` (gitignored; 8 files and an
`INDEX.txt` with sha256 and line counts; addresses redacted, 0 hits by grep).

### The breadcrumb image: where it dies

Build `213942ef` adds kind 14 STAGE breadcrumbs (`gc_result.h` lists the
stages; `CONFIG_GC_COMMON_RESULT_STAGES`) and makes the runner say
`died_after=<stage>` (rc 4) for a breadcrumb-only record instead of `done`.
Image `1150047094b7d1d2`:

```text
netboot crumbs rc=4 verdict=died_after=console_lcdif_read tftp=done:2533880B/0.038s back_after=88s uptime=18s record=594b0e13,00000000,000012e8,00000000
STAGE 0x13 console_lcdif_read value 0 (0x00000000) at uptime 4840 ms, WDOG1 fed 0 times by the 1 s timer
```

Stage 0x13 is written by the panel console's stage 1 (PRE_KERNEL_1) just
before its first LCDIF register read; stage 0x14 (returned) never came, and
the self-arm's feeder had not started (fed 0 times). Its control:

```text
2026-10-03 17:14:07 timeline: jump 0s, ping lost 9s, ping back 79s, ssh back 79s, route --crash
2026-10-03 17:14:08 record:  594b040e ff403f80 35706c4e 04000258 17.49
```

The baseline PICTURE: the breadcrumbs change nothing on the kexec route.

### Cause

`gc_panel_console.c` stage 1 read `LCDC_V8_CTRLDESCL0_5` unconditionally.
The LCDIF is inside MEDIAMIX; this U-Boot has no display driver and leaves
MEDIAMIX powered down (section 3), and a read of a powered-down or unclocked
domain is never answered: the core stalled until WDOG1 reset the SoC 60 s
after arming. On the kexec route Linux leaves the display running, so the
read was always answered there. The display driver already guarded its own
LCDIF reads with `media_powered()` (`src/display/imx8mp_display.c`); the
console, which runs earlier, did not.

### Fix

Build `2b122e76`: stage 1 first calls the Rust
`soc::imx8mp::media::lcdif_unreachable` (C entry
`yd_imx8mp_lcdif_unreachable`) on its own mappings. It returns 0 or why the
LCDIF would not answer: MEDIAMIX not on (GPC: A53 mapping, no pending or
failed trigger, ADB400 request and acknowledge), MEDIA_APB_CLK_ROOT disabled,
CCGR93 (MEDIA) closed, or the LCDIF APB clock off or in reset in
MEDIA_BLK_CTRL. It reads the GPC and the CCM, which always answer, then
MEDIA_BLK_CTRL only when those say it answers; it writes nothing. When the
LCDIF is unreachable there is no running display to adopt: stage 1 returns
and stage 2 (`panel_console_cold_init` -> `imx8mp_display_on`) brings the
chain up. A host test on the register model (which panics on an access the
hardware would not answer) pins the order. Image `7aed8bfa43d2f393`:

```text
netboot fixed rc=0 verdict=done tftp=done:2533880B/0.039s back_after=90s uptime=19s record=594b040e,ff403f80,35706c4e,04000258
```

```text
2026-10-03 17:29:17 timeline: jump 0s, ping lost 9s, ping back 80s, ssh back 80s, route --crash
2026-10-03 17:29:18 record:  594b040e ff403f80 35706c4e 04000258 18.90
```

From the cold U-Boot entry and from kexec, the same PICTURE record, equal to
the 2026-09-23 `gc_gl_triangle` parity baseline (`RESULTS.md`). **Step C
PASS.** One run per arm; the GPU power-up from reset (section 3) is now
exercised too, since the picture needs it. Not covered: the other samples
from the cold entry, and the 1280x800 window's `imx8mp_display_on(window)`
path (this image is the 1024x600 parity build).
