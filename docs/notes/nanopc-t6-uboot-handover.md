<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6: the GPU hand-over from U-Boot (decisions, 2026-11-11)

Status: design note. The clock, power and QoS decisions (sections 1, 2 and 7)
are merged code with host tests. The U-Boot and eMMC facts (section 4) were
measured read-only under Armbian; the load route (section 5) is a proposal
whose load addresses are from source (board confirmation pending), with a script draft in
driver/tools/uboot-t6/t6-zephyr.cmd (never run). The entry audit (section 6)
follows. The U-Boot prompt reads (section 3) are drafted and not run.

Why: every Z1 to Z4 image so far runs after Armbian by kexec, on the GPU clock
and power state Linux left. Stage 5's product form is U-Boot booting Zephyr
from the eMMC (docs/standalone-boot.md, "NanoPC-T6"; driver/docs/nanopc-t6-mali.md,
"Open"). That needs the image to bring up the GPU from what U-Boot leaves, or to
refuse with a cause. Rockchip's clock, power-domain and RK806 drivers are GPL:
this note and the code cite facts with file:line and transcribe nothing.

## 1. The GPU clock without Linux

Code (paths under driver/rust/src): `gpu_clock` (read only) and
`gpu_clock_set` (soc/rk3588.rs:404), called by `gpu_up`
(board/nanopc_t6_gpu.rs:240) after the RK806 reads and before the supply check.

The CRU is at 0xfd7c0000. `CLKSEL_CON(158)` (0xfd7c0578) holds `clk_gpu_src`:
divider in bits 4:0 (field + 1), mux in bits 7:5, the PVTPLL select in bit 14.
`CLKSEL_CON(159)` (0xfd7c057c) bits 4:0 are the stacks divider. `gpu_src`
decodes the mux (GPLL 0, CPLL 1, AUPLL 2, NPLL 3, SPLL 4, bit 14 the PVTPLL,
anything else `GpuSrc::Unknown`).

The OPP table is `GPU_OPP`: 300 to 1000 MHz in eight steps, each with the least
vdd_gpu_s0 (675 mV up to 600 MHz, then 700, 750, 800, 850 mV). Its source is
rk3588-opp.dtsi:118-153, named in the doc comment. `gpu_opp_rate` and
`gpu_opp_min_uv` read it.

The rule of `gpu_clock_set`:

- **Keep** (no write): `CLK_GPU_SRC` is a PLL the module can read, in normal
  mode, whose rate over the divider is exactly a `GPU_OPP` rate, and the
  stacks divider is 1 (field 0). Armbian's state is this one:
  `CLKSEL_CON(158)` = 0x1f24 is CPLL / 5 = 300 MHz, PVTPLL bit clear. Test:
  `the_clock_armbian_leaves_is_kept_without_a_write`.
- **Move** for the PVTPLL, an unknown mux, a divider that gives no table rate
  (also SPLL, or a PLL out of normal mode): to the one setting `SET_SRC` /
  `SET_DIV` = CPLL / 5 = `SET_HZ` 300 MHz, the lowest OPP. This is not a
  search: there is exactly one target, used only while the CPLL's `PLL_CON`
  words read `SET_CPLL_HZ` (1500 MHz) in normal mode. GPLL's 1188 MHz divides
  to no table rate; AUPLL and NPLL are not used (NPLL is the GPU's own PLL and
  Linux powers it down with the GPU, measured 2026-11-11). The lowest rate
  costs the least supply (675 mV), so any supply that covers the table at all
  covers it.
- **Refuse** with nothing written when the CPLL reads another rate
  (`ClockRefusal::NoTableRate`).

How it is written: `SET_SEL158` = 0x40ff0024 (hiword mask `SET_MASK`: the mux,
divider and PVTPLL bit in one write, so the GPU never sees an intermediate
setting), then, only when the stacks divider is not 1, `SET_SEL159` =
0x001f0000 (after 158, so the stacks clock never runs faster than the final
300 MHz). Bits outside the mask keep their value. Both words are read back and
the CRU is rated again; a field that did not take is `ClockRefusal::ReadBack`,
a rate other than 300 MHz is `ClockRefusal::Rate`.

Tests (soc/rk3588.rs): `the_pvtpll_is_left_for_cpll_over_5_in_one_write`,
`a_divider_off_the_table_is_rewritten_to_5`,
`a_mux_value_past_the_parents_is_rewritten`,
`the_stacks_divider_is_set_to_1_after_the_source`,
`a_cpll_at_another_rate_refuses_without_a_write`,
`a_read_back_that_does_not_match_refuses`. At the board level
(nanopc_t6_gpu.rs):
`a_clock_off_the_table_is_moved_to_cpll_over_5_before_the_supply_check`,
`the_850_mhz_handover_is_moved_to_cpll_over_5_and_npll_stays_down`,
`a_cpll_at_another_rate_refuses_the_clock_set_without_a_cru_write`,
`a_clock_write_that_does_not_take_stops_before_the_pmu_is_written`.

Never written, by `gpu_clock_set` or on its behalf:

- the PVTPLL (its GPU GRF words): it is only deselected, through bit 14 of
  `CLKSEL_CON(158)`. The CRU does not say its rate, which is why it is never
  trusted.
- any CPLL or GPLL `PLL_CON` word, and `MODE_CON0`. The CPLL is used at the
  rate it has.

Test: `the_set_path_never_writes_a_pll_con_mode_con_or_grf_word`. The one PLL
write in the run is `pll_on` (the power-down bit of the clock's own PLL through
its write-enable twin, then the wait for lock), which `gpu_up` calls after the
supply check; tests `a_powered_down_pll_is_powered_through_its_write_mask_and_waited_for`
and `a_gpu_clock_on_a_powered_down_pll_is_powered_before_pd_gpu`.

Not decided here: whether SCMI clock 5 (U-Boot's view) agrees with the CRU.
Section 3 reads both. The QoS words are section 7.

## 2. The GPU power-on from U-Boot's state

Code (under driver/rust/src): `gpu_up` (board/nanopc_t6_gpu.rs:240) and
`supply_on` in the same file; `pd_gpu_on` (soc/rk3588.rs:662);
`dcdc1_enable_frame` (ip/rk806.rs:83). The PMU is at 0xfd8d8000 (`PWR_GATE` 0x14c,
`BUS_IDLE_REQ` 0x10c, `BUS_IDLE_ACK` 0x118, `BUS_IDLE_ST` 0x120,
`REPAIR_STATUS` 0x290; PD_GPU is bit 0 of the gate and idle words, bit 1 of
`REPAIR_STATUS` reads powered).

The decisions:

1. **Never write a voltage.** `BUCK1_ON_VSEL` is read and compared with
   `gpu_opp_min_uv` of the rate `gpu_clock_set` chose.
2. **The supply comes on before the domain**: vdd_gpu_s0 (DCDC1) is PD_GPU's
   own supply (rk3588-nanopc-t6.dtsi:628-630).
3. **One RK806 write in the whole run**: `dcdc1_enable_frame`, four bytes: the
   write command, the `POWER_EN0` (0x00) address low byte first, then
   `DCDC1_WE | DCDC1_ON` (0x11), DCDC1's bit with its write-enable twin.
   Tests `the_enable_frame_is_byte_exact` and
   `the_enable_frame_is_the_only_write_and_touches_only_dcdc1` (rk806.rs).
4. **The ramp is polled, not assumed.** After the frame `POWER_EN0` is read
   every `SUPPLY_POLL_US` (100 µs) until DCDC1 reads on, up to
   `rk806::DCDC1_RAMP_POLL_BUDGET_US` (10 ms, a conservative budget, not a
   fact: no ramp rate with a file:line is in the tree). `BUCK1_ON_VSEL` is
   read again and checked again. Only then is PD_GPU powered.
5. **The hand-over report comes first.** `on_handover` sees what was read
   before the first write to the RK806, the PMU or the GPU (`supply_enabled`
   is still false there). Test
   `the_handover_report_comes_before_any_pmu_gpu_or_rk806_write`.
6. **The domain** is powered by `pd_gpu_on` through the PMU's write masks,
   which does not depend on the other domains' bits nor on whether the GPU's
   idle request was set while the domain is off.
7. **The QoS policy is applied right after the power-on succeeds.** `gpu_up`
   calls `rk3588::qos_apply` with `GPU_QOS_POLICY` = `QosPolicy::LinuxZero`
   (nanopc_t6_gpu.rs:172 and :322) once `pd_gpu_on` has returned `Ok`, then reads
   the five words of each block into `GpuUp.qos` (:323-327). A run that stops
   at or before `PowerDomain` applies none. Section 7.

The starting states:

| PD_GPU | vdd_gpu_s0 | What `gpu_up` does | Test |
|---|---|---|---|
| off | on (Linux's kexec state, measured 2026-10-19) | no RK806 write; clock set, PLL power-on, `pd_gpu_on` | `from_the_handed_over_state_it_reads_the_supply_powers_and_finds_the_g610` |
| off | off, `BUCK1_ON_VSEL` covers the rate | one enable frame, ramp wait, re-check, then `pd_gpu_on` | `b_supply_off_covering_300_mhz_is_enabled_by_one_frame_then_the_domain_comes_on` |
| on, idle request released | on | left as it is, no PMU write | `c_a_domain_left_on_is_not_written_in_the_pmu` |
| on | off | refused, no write | `a_domain_that_reads_powered_with_the_supply_off_is_refused_without_a_write` |

The stop causes (`Stop`, nanopc_t6_gpu.rs:99-137; the value crosses to C as
`YD_T6_GPU_STOP_*`, values 0 to 13 are all in
driver/rust/include/yakogl_drivers.h:324-352). `refusal` and `detail` are the
`GpuUp` fields of those names. The table lists the causes of the clock and
supply steps. The others: 0 `None` (every step ran), 1 `SpiGate` (an SPI2 clock
gate stays closed; refusal its bit), 2 `Rk806Read` (an RK806 read failed;
refusal the `SpiRefusal`), 5 `PowerDomain` (`pd_gpu_on` refused; refusal the
`PdRefusal`, detail the PMU word; before the QoS write), 6 `NotG610` (detail
the `GPU_ID` read), 7 `Reset` (the soft reset did not complete) and 8 `L2` (the
L2 did not come up); 6 to 8 come after the QoS write.

| Stop | Value | Refusal | Detail | Test |
|---|---|---|---|---|
| `SupplyOff` | 3 | 1 (`SUPPLY_OFF_DOMAIN_POWERED`): PD_GPU reads powered while DCDC1 reads off | `POWER_EN0` | `a_domain_that_reads_powered_with_the_supply_off_is_refused_without_a_write` |
| `ClockNotCovered` | 4 | 0: supply on and below the OPP minimum of the clock; 1 (`CLOCK_PLL_NO_LOCK`): the PLL did not lock | the rate in Hz; `PLL_CON6` | `a_clock_the_supply_does_not_cover_stops_before_the_pmu_is_written`, `a_gpu_pll_that_does_not_lock_stops_before_the_pmu_is_written` |
| `ClockSetRefused` | 9 | 0 | the CPLL's rate, 0 for none | `a_cpll_at_another_rate_refuses_the_clock_set_without_a_cru_write` |
| `ClockReadBack` | 10 | the CLKSEL_CON index (158 or 159); 0 for a wrong rate | the word read; the rate | `a_clock_write_that_does_not_take_stops_before_the_pmu_is_written` |
| `SupplyBelowOpp` | 11 | 0: found before the enable, nothing written; 1 (`SUPPLY_BELOW_AFTER_ENABLE`): read again after it | `BUCK1_ON_VSEL` << 32 \| rate in Hz | `b_prime_supply_off_below_the_opp_stops_without_a_write`, `a_voltage_that_reads_lower_after_the_enable_stops_before_the_pmu` |
| `SupplyRampTimeout` | 12 | 0 | the last `POWER_EN0` | `b_double_prime_supply_that_never_ramps_stops_without_a_pmu_write` |
| `Rk806Write` | 13 | the `SpiRefusal` | its value | `an_enable_frame_the_bus_does_not_take_stops_before_the_pmu` |

Stops 9 to 13 never write the PMU. After 12 and 13 whether the enable
reached the PMIC is unknown; the supply fields of `GpuUp` (`supply_enabled`,
`supply_after`, `supply_waited_us`) show what was read.

Not measured, for the U-Boot session: how fast DCDC1 actually ramps, and
whether U-Boot leaves PD_GPU powered with its idle request still set (the
`pd_gpu_on` test `a_powered_domain_with_its_idle_request_set_is_released_from_idle`
covers the model, not the board).

## 3. The U-Boot prompt read procedure

Draft, not run. Every step is a read: no write, no `mw`, no `bootm`/`booti`,
nothing flashed. It waits for the owner's go-ahead with someone at the power
switch. The UART path and the board's address come from the environment, never
from this note. The board is shared with other work under Linux: reboot into
U-Boot only when told.

Preconditions: the board stopped at U-Boot's prompt (autoboot interrupted),
nothing run before. Log everything the console prints. Record each answer in
section 4 and in issue #26.

1. **Version and environment.** `version`, `printenv`. Read-only.
2. **The PMU, PD_GPU.** `md.l` one word each, from 0xfd8d8000: `PWR_GATE`
   (+0x14c), `BUS_IDLE_REQ` (+0x10c), `BUS_IDLE_ACK` (+0x118), `BUS_IDLE_ST`
   (+0x120), `REPAIR_STATUS` (+0x290). Read-only. Off after Linux's kexec reads
   0xffff in the gate word and 0xfad50001 in the status word. Decides: which row
   of the section 2 table U-Boot hands over.
3. **The RK806.** `pmic dump`, or `regulator dev vdd_gpu_s0` then `regulator
   status`, when U-Boot's driver has the supply; otherwise record that it does
   not (the image reads it over SPI2 itself). Wanted: `POWER_EN0` (Linux and
   the image read 0x0f), `POWER_SLP_EN0` (register 0x06, Linux reads 0xe0), `BUCK1_CONFIG` (register
   0x10, 0xa4), `BUCK1_ON_VSEL` (register 0x1a, 0x1c = 675 mV;
   ip/rk806.rs:46-48, :179). Read-only: never `regulator value` or
   `regulator enable`, which write. Decides: whether DCDC1 is on, and for an
   off supply whether `BUCK1_ON_VSEL` covers 675 mV (`SupplyBelowOpp`).
4. **The CRU.** `md.l` of 0xfd7c0578 (`CLKSEL_CON(158)`; Linux leaves 0x1f24),
   0xfd7c057c (`CLKSEL_CON(159)`), 0xfd7c0908 (`CLKGATE_CON(66)`),
   `MODE_CON0` (0xfd7c0280), the CPLL `PLL_CON` words (index 104 to 110, from
   0xfd7c01a0) and the GPLL ones (112 to 118, from 0xfd7c01c0). Then
   `clk dump` (the rates U-Boot believes, SCMI clock 5 included). Read-only.
   Decision rules, from section 1: 0x1f24 with the CPLL at 1500 MHz means no CRU
   write; bit 14 set, a mux past 4 or a divider off the table means
   `gpu_clock_set` writes CPLL / 5; a CPLL at another rate means
   `ClockSetRefused`, nothing written, and the image needs another target.
5. **The QoS words.** Only if step 2 shows PD_GPU on (reading an unpowered
   domain's block can fault the bus; this ordering is a safety assumption, not
   a measured fact): `md.l` of the five words (+0x08 to +0x18) of each of the
   four blocks at 0xfdf35000, 0xfdf35200, 0xfdf35400 and 0xfdf35600.
   Read-only. Linux reads `PRIORITY` 0x80000000 and 0 in the other four words;
   what the image applies after the power-on is section 7.
6. **The eMMC.** `mmc list`, `mmc dev 0` (selects, writes nothing), `mmc part`,
   then `ext4ls mmc 0:<n> /` (or `ls`) for each partition section 4 names.
   Read-only. Never `mmc write`, `ext4write` or `saveenv`.
7. **Leave.** Have the person at the power switch return the board to Linux. Do
   not `bootm`, `booti` or `go`.

Issue text for the session: "NanoPC-T6: U-Boot hand-over reads (needs the
owner)": run this procedure at the U-Boot prompt with someone at the power
switch, reads only, record the answers here and in #26.

## 4. Measured U-Boot and eMMC facts

Source: the read-only ssh session "t6-reads", plan #346 (no `records/` directory
is named for it). Run under Armbian with nothing written, no kexec, no reboot.
No host name, address or partition UUID is recorded here.

(a) U-Boot, from the eMMC:

| Fact | Command that produced it |
|---|---|
| U-Boot is 2026.07_armbian, on the eMMC | `strings` over the U-Boot image read from the eMMC (a read to a pipe); `cat /proc/cmdline` for how Linux was started |
| The environment is never saved, so the compiled-in one is used | `fw_printenv` (absent, or without an environment location, so nothing to print); `strings` over the U-Boot image for the built-in default environment |
| `bootcmd` is `bootflow scan -lb` | `strings` over the U-Boot image |
| `boot_targets` starts `mmc1 mmc0 ...` | `strings` over the U-Boot image |
| No `extlinux.conf` exists, only `boot.scr` | `lsblk -f`, then listing the mounted `/boot` |

Which index is the eMMC on the real U-Boot (`mmc1 mmc0`) is not measured: it
needs `mmc list` at the prompt (section 3, step 6). Below, `<dev>` stands for it.

(b) The eMMC, from `lsblk -f`, `blkid` and `df`:

- The only ext4 partition is `mmcblk0p1`, with 10G free. `/boot` is a directory
  on it, not a partition of its own.
- The other partitions' types are in the same `lsblk -f` output of t6-reads;
  none is ext4. This note does not repeat their names, and no UUID.

Board state under Armbian, same session: the GPU sits at 300 MHz
(`CLKSEL_CON(158)` 0x1f24), vdd_gpu_s0 is enabled at 675 mV, PD_GPU is off.
This is the first row of the section 2 table.

Still to read from U-Boot itself (section 3): `printenv`, `mmc list`, `mmc dev`,
and the PMU, CRU and RK806 state that U-Boot hands over.

## 5. The proposed ext4load route

U-Boot's default boot is `bootflow scan -lb`, which walks `boot_targets`, finds
a bootable partition and runs its `boot.scr` (Armbian's). The image reaches the
CPU the way the Verdin route does (docs/standalone-boot.md):

1. A script (a replacement `boot.scr`, or one the existing flow runs) runs
   `ext4load mmc <dev>:1 <addr> /boot/<image>.bin`. Partition 1 is the only
   ext4 partition and `/boot` is on it, so the image file is
   `/boot/<image>.bin`.
2. Then `go <addr>` or `booti <addr> - <fdt>`.

Which applies: the Z images are flat binaries linked at one fixed physical
address, with `CONFIG_AARCH64_IMAGE_HEADER` (kexec_zephyr.c:436 refuses an image
without the arm64 Image header; nanopc_t6.dts:10: `zephyr,sram` is 256 MiB at
0x10000000; nanopc-t6.sh:104-107 the same window). So:

- `go 0x10000000` is the route: it jumps to the first byte, the header's branch,
  with no device tree and no relocation. The load address must be the link
  address, 0x10000000.
- `booti` also accepts an image with the header, but it wants a device tree and
  may relocate to the header's text offset; the image has no tree and is linked
  to a fixed address, so `booti` adds risk for no gain. Use it only if `go`
  is refused.

What the script must do besides the two commands (section 6):

- flush and disable the data cache before `go`, as the Verdin route does
  (standalone-boot.md:78);
- arm a watchdog the image can feed: U-Boot runs none and the Linux watchdog
  that covers kexec runs does not exist (standalone-boot.md:85);
- set a one-shot variable with its undo, because U-Boot's `bootcmd` reruns after
  the image's PSCI reset and would load the image again.

The script draft is driver/tools/uboot-t6/t6-zephyr.cmd (README.md beside it),
never run on a board. It sizes the file with `ext4size` (:42), refuses outside
0x40 to 0x10000000 bytes, `ext4load`s to 0x10000000, checks the size read back,
the `ARM\x64` magic at byte 56 and the header's `image_size` against the
256 MiB window (:75-89), turns the instruction and data caches off (:100) and
runs `go 0x10000000` (:104). Its mmc device `0:1` is a default, not a
measurement (:19). It writes nothing to any disk or to the saved environment;
its variables are `t6z_*`, in RAM. It differs from the list above in two
points: no one-shot variable, because nothing is saved and a reset returns to
`bootflow scan -lb` (:34-36); and no watchdog is armed: the script prints that
the watchdog marker is not checked (:37), so that step stays open.

Load addresses, from source (U-Boot v2026.07, file:line); confirmation on the
board is pending (the read-only uboot-addrs measurement). The board's defconfig
selects the config name `rk3588_common` (arch/arm/mach-rockchip/rk3588/Kconfig:443-444),
and its `ENV_MEM_LAYOUT_SETTINGS` (include/configs/rk3588_common.h:20-31) sets:

- `scriptaddr` 0x00c00000 (:22) and `pxefile_addr_r` 0x00e00000 (:25);
- `kernel_addr_r` 0x02000000 (:26) and `kernel_comp_addr_r` 0x0a000000 (:27), with
  `kernel_comp_size` 0x8000000 (:31), so the decompression area ends at 0x12000000;
- `fdt_addr_r` 0x12000000 (:28), `fdtoverlay_addr_r` 0x12100000 (:29) and
  `ramdisk_addr_r` 0x12180000 (:30);
- no `loadaddr` variable in that header; `CONFIG_SYS_LOAD_ADDR` is 0xc00800
  (configs/nanopc-t6-rk3588_defconfig:13). U-Boot's RAM starts at 0
  (rk3588_common.h:14).

The Z image's link address is 0x10000000, the 256 MiB window
0x10000000-0x1fffffff: nanopc_t6.dts:55-58 (`memory@10000000`, the node
`zephyr,sram` names at :52), the same window in nanopc-t6.sh:104-106
(`WINDOW_BASE`) and `t6z_load`, `t6z_entry` and `t6z_window` in
t6-zephyr.cmd:26-28. `CONFIG_SRAM_BASE_ADDRESS` is derived from that node, not
set in nanopc_t6_defconfig; `KERNEL_VM_BASE` is the same 0x10000000 and
`KERNEL_VM_SIZE` 0x20000000 (nanopc_t6_defconfig:16-25).

Overlap verdict, the window against those addresses:

- `scriptaddr`, `pxefile_addr_r` and `kernel_addr_r` (0x00c00000 to 0x02000000)
  are below 0x10000000: no overlap. The draft's `load mmc 0:1 ${scriptaddr}` and
  `source ${scriptaddr}` do not touch the image.
- `fdt_addr_r`, `fdtoverlay_addr_r` and `ramdisk_addr_r` (0x12000000 and up) lie
  inside the window, and `kernel_comp_addr_r` plus its size ends at 0x12000000,
  the window's inside edge. The script loads the image to 0x10000000 and uses
  none of these, so the draft is not affected; but a `booti` route with a tree
  at `fdt_addr_r` or a ramdisk would load them over the image's own pages. One
  more reason for `go`, and a rule for any later change: no tree or ramdisk at
  these defaults.
- t6-zephyr.cmd loads to 0x10000000 (:26, :63) and runs `go` at 0x10000000
  (:107): the load address equals the link address, as required.

That the window is free of U-Boot's relocated copy and the trusted firmware's own
memory stays a read for section 3 (`bdinfo` at the prompt shows U-Boot's); the
verdict above covers only the environment's default addresses.

Source citations, U-Boot v2026.07 (commit `ece349ad`) and Armbian's U-Boot
build (commit `fc874938`), confirmed by the uboot-src-lines item:
`include/config_distro_bootcmd.h` (`scan_dev_for_boot`), `cmd/boot.c`
(`do_go`), `arch/arm/lib/image.c` (the `booti` header checks) and Armbian's
`boot.cmd`. The line numbers are in that item's report; its handoff did not
carry them, so they are not written here.

This note cites U-Boot v2026.07 (`ece349ad`). The v2026.10 comparison is not
recorded in this note: check it before relying on a newer U-Boot.

## 6. What the Z images assume Linux left, against U-Boot

Source: the entry-audit item, done on mainline U-Boot source, not the board
(the board's U-Boot is mainline-like; the vendor tree was checked only where it
differs). It is source reading, not measurement. The rows this note carries
(4, 5, 9 to 13, 17 and 18 of that audit) are in its report; its table was not in
this note's handoff, so the findings that bear on the route are listed here by
what they are, not by row number:

- Cache and MMU at `go`: the image assumes MMU and caches off. Fix: the
  script's flush and `dcache off`, or `CONFIG_ARM64_BOOT_DISABLE_DCACHE=y` in the
  board defconfig (not done).
- The watchdog: arm WDT0 in the script before `go` (not done).
- The one-shot variable (section 5).
- RK806 on SPI2: probing it from U-Boot's `pmic`, `regulator` or `sf` commands
  writes `SYS_CFG3`. Run the `dm tree` and `md` reads of section 3 first.
- UART2: read from offset 4, because offset 0 pops a received byte.
- `CNTFRQ_EL0` and the `SCTLR_EL2` bits are not printed by the early banner
  (a small item after #341 that touches `ffi/rk3588.rs` and the header).

None of the changes the audit found was made by this plan.

## 7. QoS

Code (under driver/rust/src):

- soc/rk3588.rs holds the choice: `QosPolicy` with `Reset` (`MODE` 3,
  `BANDWIDTH` 0x12d, `SATURATION` 0x40, the values the image leaves) and
  `LinuxZero` (all three 0, what Linux reads with the GPU active, 2026-10-26),
  `QOS_POLICY_WORDS` (0x0c, 0x10, 0x14; :536), and `qos_apply` (:564), which
  writes those three words in each of the four blocks at `QOS_GPU` (:530):
  twelve writes, `PRIORITY` and `EXTCONTROL` never. Tests:
  `qos_reset_policy_writes_only_mode_bandwidth_saturation`,
  `qos_linux_zero_policy_writes_only_mode_bandwidth_saturation`.
- `gpu_up` applies it. Right after `pd_gpu_on` returns `Ok` and `pd_after` is
  read, it calls `rk3588::qos_apply(m, b.qos, GPU_QOS_POLICY)` with
  `GPU_QOS_POLICY` = `QosPolicy::LinuxZero` (board/nanopc_t6_gpu.rs:172, :322),
  then reads the five words of each block into `GpuUp.qos` (:323-327), so the
  record shows the words after the write. The choice is `LinuxZero` because
  every Z1 to Z4 board run so far worked under Linux's zeroed words (doc
  comment, :168-171). Test
  `from_the_handed_over_state_it_reads_the_supply_powers_and_finds_the_g610`
  (:715) checks the twelve writes of 0 and that no other QoS word is written.
- Not done: the record field naming the policy applied is the later item
  `qos-select` (build-time selectable). Until it lands the policy is the
  constant and `GpuUp` does not say which one ran.
- Which policy a real frame needs is open (nanopc-t6-mali.md "Open", "The QoS
  words' effect"): the first Z4 frame runs once with each (`Reset` by changing
  the constant).
