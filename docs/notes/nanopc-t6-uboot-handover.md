<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6: the GPU hand-over from U-Boot (decisions, 2026-11-11)

Status: design note, decisions part. The clock and power decisions below are
merged code with host tests. The measured U-Boot and eMMC facts and the load
route are pending (sections 4 and 5). The U-Boot prompt reads (section 3) are
drafted and not run.

Why: every Z1 to Z4 image so far runs after Armbian by kexec, on the GPU clock
and power state Linux left. Stage 5's product form is U-Boot booting Zephyr
from the eMMC (docs/standalone-boot.md, "NanoPC-T6"; driver/docs/nanopc-t6-mali.md,
"Open"). That needs the image to bring up the GPU from what U-Boot leaves, or to
refuse with a cause. Rockchip's clock, power-domain and RK806 drivers are GPL:
this note and the code cite facts with file:line and transcribe nothing.

## 1. The GPU clock without Linux

Code: `gpu_clock` (read only) and `gpu_clock_set` in
driver/rust/src/soc/rk3588.rs, called first by `gpu_up` in
driver/rust/src/board/nanopc_t6_gpu.rs, before the supply check.

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

Not decided here: the QoS words (a separate item), and whether SCMI clock 5
(U-Boot's view) agrees with the CRU. Section 3 reads both.

## 2. The GPU power-on from U-Boot's state

Code: `gpu_up` and `supply_on` in driver/rust/src/board/nanopc_t6_gpu.rs;
`pd_gpu_on` in soc/rk3588.rs; `dcdc1_enable_frame` in
driver/rust/src/ip/rk806.rs. The PMU is at 0xfd8d8000 (`PWR_GATE` 0x14c,
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

The starting states:

| PD_GPU | vdd_gpu_s0 | What `gpu_up` does | Test |
|---|---|---|---|
| off | on (Linux's kexec state, measured 2026-10-19) | no RK806 write; clock set, PLL power-on, `pd_gpu_on` | `from_the_handed_over_state_it_reads_the_supply_powers_and_finds_the_g610` |
| off | off, `BUCK1_ON_VSEL` covers the rate | one enable frame, ramp wait, re-check, then `pd_gpu_on` | `b_supply_off_covering_300_mhz_is_enabled_by_one_frame_then_the_domain_comes_on` |
| on, idle request released | on | left as it is, no PMU write | `c_a_domain_left_on_is_not_written_in_the_pmu` |
| on | off | refused, no write | `a_domain_that_reads_powered_with_the_supply_off_is_refused_without_a_write` |

The stop causes (`Stop` in nanopc_t6_gpu.rs; the value crosses to C as
`YD_T6_GPU_STOP_*`, the variants from 9 on not in the C header yet). `refusal`
and `detail` are the `GpuUp` fields of those names.

| Stop | Value | Refusal | Detail | Test |
|---|---|---|---|---|
| `SupplyOff` | 3 | 1 (`SUPPLY_OFF_DOMAIN_POWERED`): PD_GPU reads powered while DCDC1 reads off | `POWER_EN0` | `a_domain_that_reads_powered_with_the_supply_off_is_refused_without_a_write` |
| `ClockNotCovered` | 4 | 0: supply on and below the OPP minimum of the clock; 1 (`CLOCK_PLL_NO_LOCK`): the PLL did not lock | the rate in Hz; `PLL_CON6` | `a_clock_the_supply_does_not_cover_stops_before_the_pmu_is_written`, `a_gpu_pll_that_does_not_lock_stops_before_the_pmu_is_written` |
| `ClockSetRefused` | 9 | 0 | the CPLL's rate, 0 for none | `a_cpll_at_another_rate_refuses_the_clock_set_without_a_cru_write` |
| `ClockReadBack` | 10 | the CLKSEL_CON index (158 or 159); 0 for a wrong rate | the word read; the rate | `a_clock_write_that_does_not_take_stops_before_the_pmu_is_written` |
| `SupplyBelowOpp` | 11 | 0: found before the enable, nothing written; 1 (`SUPPLY_BELOW_AFTER_ENABLE`): read again after it | `BUCK1_ON_VSEL` << 32 \| rate in Hz | `b_prime_supply_off_below_the_opp_stops_without_a_write`, `a_voltage_that_reads_lower_after_the_enable_stops_before_the_pmu` |
| `SupplyRampTimeout` | 12 | 0 | the last `POWER_EN0` | `b_double_prime_supply_that_never_ramps_stops_without_a_pmu_write` |
| `Rk806Write` | 13 | the `SpiRefusal` | its value | `an_enable_frame_the_bus_does_not_take_stops_before_the_pmu` |

Stops 11, 12 and 13 never write the PMU. After 12 and 13 whether the enable
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
   the image read 0x0f), `POWER_SLP_EN0` (0xe0), `BUCK1_CONFIG` (0xa4),
   `BUCK1_ON_VSEL` (0x1c, 675 mV). Read-only: never `regulator value` or
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
   Read-only. Linux reads `PRIORITY` 0x80000000 and 0 in the other four words.
6. **The eMMC.** `mmc list`, `mmc dev 0` (selects, writes nothing), `mmc part`,
   then `ext4ls mmc 0:<n> /` (or `ls`) for each partition section 4 names.
   Read-only. Never `mmc write`, `ext4write` or `saveenv`.
7. **Leave.** Have the person at the power switch return the board to Linux. Do
   not `bootm`, `booti` or `go`.

Issue text for the session: "NanoPC-T6: U-Boot hand-over reads (needs the
owner)": run this procedure at the U-Boot prompt with someone at the power
switch, reads only, record the answers here and in #26.

## 4. Measured U-Boot and eMMC facts

Filled from the t6-reads measurement (pending).

## 5. The proposed ext4load route

Filled from the t6-reads measurement (pending).
