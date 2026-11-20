<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX 95 GPU power: what the A55 asks the System Manager for, and whether it may

On the i.MX 95 the Mali-G310's power domain, clock root and clock gate
belong to the System Manager (SM), NXP's firmware on the Cortex-M33. The
Cortex-A55 asks for them over SCMI. This note holds the facts the sequence
in `driver/rust/src/soc/imx95/gpu_power.rs` relies on, read on the host (no
board yet). The board is the Toradex Verdin iMX95.

Sources, each read at the named commit and cited by file and line, never
copied:

- Linux: torvalds/linux master `65e4fd942f72` (2026-11-19).
- NXP's SM: github.com/nxp-imx/imx-sm `eec232878ab7` (2026-09-03, the head
  of its default branch), configuration `configs/mx95evk.cfg`.
- Toradex's SM: git.toradex.com/imx-sm-toradex.git, branch `main`,
  `3aa8a77a15d6` (2026-11-16); `configs/verdin-imx95.cfg` and its generated
  `configs/verdin-imx95/` last changed in `857b380b0e29` (2026-11-13). Code
  lines below are at `3aa8a77a`.
- Zephyr at the workspace pin, `af17c0c8aa2a` (`west.yml`).
- Arm DEN0056, the SCMI specification, version E (SCMI v3.2): power domain
  management (section 4.3), performance (4.5), clock (4.6), status codes
  (table 6). The SM reports power 3.1, performance 4.0 and clock 3.0
  (`PROTOCOL_VERSION` 0x30001, 0x40000, 0x30000 in
  `sm/rpc/scmi/rpc_scmi_power.c:48`, `rpc_scmi_perf.c:48`,
  `rpc_scmi_clock.c:48`).

## 1. The numbers

| Name | Value | SM | Linux | Zephyr at the pin |
| --- | --- | --- | --- | --- |
| GPU power domain | 14 | `DEV_SM_PD_GPU`, `devices/MIMX95/sm/dev_sm_power.h:85` | `IMX95_PD_GPU`, `arch/arm64/boot/dts/freescale/imx95-power.h:23` | `IMX95_PD_GPU` 14, `include/zephyr/dt-bindings/power/imx95_power.h:24` |
| GPU performance domain | 9 | `DEV_SM_PERF_GPU`, `dev_sm_perf.h:73` | `IMX95_PERF_GPU`, `imx95-power.h:42` | none |
| GPU clock gate (panthor "core") | 174 | `DEV_SM_CLK_GPU_CGC`, `dev_sm_clock.h:255` | `IMX95_CLK_GPU_CGC` = 41 + 123 + 10, `imx95-clock.h:51`, `:186` | none (the header ends at `IMX95_CLK_XSPISLVROOT`, `imx95_clock.h:175`) |
| GPU APB clock (panthor "coregroup") | 83 | `DEV_SM_CLK_GPUAPB`, `dev_sm_clock.h:152` | `IMX95_CLK_GPUAPB` = 41 + 42, `imx95-clock.h:95` | same, `imx95_clock.h:95` |
| GPU clock root (parent of the gate) | 84 | `DEV_SM_CLK_GPU`, `dev_sm_clock.h:153` | `IMX95_CLK_GPU`, `imx95-clock.h:96` | same, `imx95_clock.h:96` |

The gate's parent is the root (`devices/MIMX95/drivers/fsl_clock.c:837`,
`.rootIdx = CLOCK_ROOT_GPU`). The root's parents come from the performance
level, not from a request: level 0 (park) the 24 MHz oscillator, level 1
SYSPLL1 PFD0 (1000 MHz) divided by 2, level 2 SYSPLL1 PFD1 (800 MHz),
level 3 PFD0 undivided (`dev_sm_perf.c:681-703`, the parents at `:58-59`);
the frequencies 24, 500, 800, 1000 MHz (`dev_sm_perf.c:706-731`,
`MIMX95_elec_spec.h:186-188`). Those are Linux's OPP rates
(`imx95.dtsi:253-270`, 500/800/1000 MHz, all at 920 mV). The SM has four
levels per SoC domain (`dev_sm_perf.c:50`) and returns at most four per
DESCRIBE_LEVELS reply (`rpc_scmi_perf.c:65`, `rpc_scmi_internal.h:83`).
Linux's dtsi references the Linux-local `imx95-clock.h` and
`imx95-power.h` (`imx95.dtsi:13`, `:15`); where Zephyr has an id it agrees,
but Zephyr has no `IMX95_CLK_GPU_CGC` and no performance-domain binding.

Linux's node (`imx95.dtsi:2298-2311`): `gpu@4d900000`, clocks
`<&scmi_clk IMX95_CLK_GPU_CGC>` "core" and `<&scmi_clk IMX95_CLK_GPUAPB>`
"coregroup", `power-domains = <&scmi_devpd IMX95_PD_GPU>`,
`operating-points-v2 = <&gpu_opp_table>`, no `status` (enabled). The Verdin
board files (`imx95-verdin.dtsi`, `-wifi`/`-nonwifi`, and the dahlia, dev,
ivy, mallow, yavia and zinnia carriers, plus `imx95-19x19-verdin-evk.dts`)
never mention the GPU: no override of status, supply or OPP. No GPU supply
is named anywhere: `VDD_SOC` belongs to the SM, which sets it with the
performance level (`dev_sm_perf.c`, `DEV_SM_PerfLevelSet`, the supply
update between the two frequency updates).

## 2. What Linux does, reduced to SCMI messages

Linux's order, all on the one A55 non-secure channel (`imx95.dtsi:337-340`,
MU at 0x445b0000, shared memory at 0x445b1000, `:1590-1612`):

1. At boot the SCMI power-domain driver reads every domain
   (POWER_STATE_GET) and sets again each it finds on
   (`drivers/pmdomain/arm/scmi_pm_domain.c:81-102`), so the SM counts this
   agent's vote. Before panthor's probe the platform bus attaches the
   single domain and powers it on (`drivers/base/platform.c:1502`,
   `PD_FLAG_ATTACH_POWER_ON`): `scmi_pd_power_on` (`scmi_pm_domain.c:32`)
   sends POWER_STATE_SET domain 14 ON. panthor's own
   `panthor_init_power` returns at once when a domain is already attached
   (`drivers/gpu/drm/panthor/panthor_device.c:54-62`).
2. The clock driver, at its probe, reads each clock's attributes and, as
   both the SM's protocol (3.0) and the attributes allow, its permissions
   (CLOCK_GET_PERMISSIONS, `drivers/firmware/arm_scmi/clock.c:337-350`,
   `:405-410`). A clock without state or rate permission gets no enable or
   set-rate operation at all (`drivers/clk/clk-scmi.c:230-253`,
   `:312-321`); enabling it is then a no-op that sends nothing.
3. panthor probe: clocks looked up (`panthor_device.c:30-50`), devfreq
   initialised (`panthor_devfreq.c:133`): the clock's rate read
   (CLOCK_RATE_GET 174, `:185`), the recommended OPP set
   (`dev_pm_opp_set_opp`, `:217-232`), which calls `clk_set_rate` on
   "core": CLOCK_RATE_SET 174 unless the clock framework finds the rate
   already equal.
4. Runtime resume (`panthor_device_resume`, `panthor_device.c:492`):
   `clk_prepare_enable` "core" (CLOCK_CONFIG_SET 174 enable), "stacks"
   (absent), "coregroup" (clock 83: no message, see 3. below).
5. Suspend: the three clocks disabled in reverse (`:606-610`): CLOCK_CONFIG_SET
   174 disable; the domain goes off with the last runtime user
   (`scmi_pd_power_off`, `scmi_pm_domain.c:37`: POWER_STATE_SET OFF).

The gap in Linux's path: the SM refuses a rate on a gate.
`DEV_SM_ClockRateSet` answers INVALID_PARAMETERS for any CGC clock
(`devices/MIMX95/sm/dev_sm_clock.c:789`, the CGC branch at `:886-889`; the
Toradex board hook passes it through, `boards/tdx-common/sm/brd_sm_clock.c`
`BRD_SM_ClockRateSet`), and the root, which would take a rate, is not
granted to the A55 (section 3). So Linux's CLOCK_RATE_SET 174 can only
succeed by not being sent (the rate already equal). The SM's own design is
the performance protocol: its configuration lists `CLK_GPU` and
`CLK_GPUAPB` under "Modified via PERF protocol" (`verdin-imx95.cfg:149`,
`:176-177`). Our sequence uses that and sends no CLOCK_RATE_SET.

The messages ours sends (`gpu_power.rs`; DEN0056E layouts, little-endian,
header = message id | protocol << 10 | token << 18, token 0; a reply is the
header, a signed status, then the values):

| # | Protocol / message | Payload | Expected reply |
| --- | --- | --- | --- |
| 1 | 0x11 / 0x0 PROTOCOL_VERSION | none | 0, version 0x0003xxxx |
| 2 | 0x13 / 0x0 PROTOCOL_VERSION | none | 0, 0x0004xxxx |
| 3 | 0x13 / 0x4 PERFORMANCE_DESCRIBE_LEVELS | domain 9, skip 0 | 0, levels word (returned bits 11:0, remaining 31:16), then per level value, power cost, attributes, indicative kHz, level index (`rpc_scmi_perf.c:810`, the index at `:870`); again with skip = levels read while some remain |
| 4 | 0x14 / 0x0 PROTOCOL_VERSION | none | 0, 0x0003xxxx |
| 5 | 0x11 / 0x4 POWER_STATE_SET | flags 0 (synchronous), domain 14, state 0 (ON) | 0 |
| 6 | 0x11 / 0x5 POWER_STATE_GET | domain 14 | 0, 0 (ON) |
| 7 | 0x13 / 0x7 PERFORMANCE_LEVEL_SET | domain 9, level index (2 for 800 MHz) | 0 |
| 8 | 0x13 / 0x8 PERFORMANCE_LEVEL_GET | domain 9 | 0, the index |
| 9 | 0x14 / 0x7 CLOCK_CONFIG_SET | clock 174, attributes 1 (enable), extended value 0 | 0 |
| 10 | 0x14 / 0x6 CLOCK_RATE_GET | clock 174 | 0, rate low, rate high (Hz) |

Power-off: CLOCK_CONFIG_SET 174 attributes 0, POWER_STATE_SET flags 0,
domain 14, state 0x40000000 (OFF), POWER_STATE_GET 14 reading 0x40000000.

Every read comes before the first write, so a wrong version or a rate with
no level (below 500 MHz; the park level is never chosen) refuses with
nothing switched. The level is chosen from DESCRIBE_LEVELS, rounded down
to the fastest at or below the rate asked for: the domain is in
level-indexing mode (`rpc_scmi_perf.c:740`) and PERFORMANCE_LEVEL_SET takes
the index (`dev_sm_perf.c`, `DEV_SM_PerfLevelSet` checks it against the
level count). The rate the SM made is read back from the gate and kept as
read (a PLL may round); one above the level's or 0 is refused. The SM
releases the GPU reset itself when the domain comes up
(`devices/MIMX95/sm/dev_sm.c:233`, `dev_sm_config.c:269-293`); nothing in
the GPU mix is ours to write before `GPU_ID`.

## 3. Permissions on the Verdin iMX95

Toradex's configuration, `configs/verdin-imx95.cfg` (`857b380b`), has
three SCMI agents: `M7` (agent 0, `:371`, MU9, the Cortex-M7 logical
machine LM1), `AP-S` (agent 1, `:496-497`, secure, MU1: the A55's secure
world) and `AP-NS` (agent 2, `:570-571`, non-secure, MU3), both of the
A55's logical machine LM2 ("AP", `:443`). The AP-NS section owns
`GPU_NPROT` and `GPU_PROT` (`:760-761`) and the GPU's register window
(`:1035`, 0x4D900000-0x4DD7FFFF, DATA). The configuration tool expands
`GPU_NPROT` to `PD_GPU, PERF_GPU, ..., CLK_GPU_CGC`
(`devices/MIMX95/configtool/gpumix.cfg:40`), and the generated agent shows
it: `SM_SCMI_AGNT2_CONFIG` "AP-NS" (`configs/verdin-imx95/config_scmi.h:230-235`)
has `clkPerms[DEV_SM_CLK_GPU_CGC]` (`:261`), `pdPerms[DEV_SM_PD_GPU]`
(`:460`) and `perfPerms[DEV_SM_PERF_GPU]` (`:469`) all `SM_SCMI_PERM_ALL`
(255). Nothing grants clock 83 or 84 to any agent: both sit in the SM's own
section (`verdin-imx95.cfg:176-177`).

What each request needs (`rpc_scmi_config.h:83-89`: NONE 0, GET 1,
NOTIFY 2, SET 3, PRIV 4, EXCLUSIVE 5, ALL 255): POWER_STATE_SET at least
SET (`rpc_scmi_power.c:633`), PERFORMANCE_LEVEL_SET at least PRIV
(`rpc_scmi_perf.c:1101`), CLOCK_CONFIG_SET enable at least SET
(`rpc_scmi_clock.c:1274`), CLOCK_RATE_SET at least EXCLUSIVE (`:1086`);
anything less answers DENIED. So:

- **AP-NS may power the GPU domain, set its level, and enable and disable
  its clock gate.** It may not enable the APB clock (83) or set the root
  (84): requests for them would be DENIED, and the sequence sends neither.
  The SM sets the GPU APB root itself with the bus clocks
  (`dev_sm_perf.c`, `DEV_SM_PerfBusFreqSet`, `CLOCK_ROOT_GPUAPB` at
  `:2094`), and Linux sends nothing for it either (section 2, step 2).
- The agent is the channel, not the operating system. The SM tells agents
  apart by the transport channel a message arrives on: AP-NS is SMT
  channel 5 (`config_scmi.h:627-632`), on MU instance 3 doorbell 0
  (`config_smt.h:123-129`, `config_mb_mu.h:93-95`), which is the B side of
  the always-on MU at 0x445C0000 whose A side the A55 writes at 0x445B0000
  (`devices/MIMX95/MIMX95_MU.h:817-821`, the instance table at `:953`).
  Linux uses that MU and the shared memory at 0x445b1000
  (`imx95.dtsi:339-340`, `:1590-1612`), and so does Zephyr's A55 SoC at
  the pin (`dts/arm64/nxp/nxp_mimx95_a55.dtsi:96-107`, the MU at `:486`).
  A Zephyr image on the A55 is therefore AP-NS with exactly Linux's
  permissions: it is not refused. It does not get AP-S (MU1, secure: the
  A55's secure firmware's channel) and needs nothing from it.
- What it inherits: the SM keeps each agent's votes (power state, clock
  enable) and its level per logical machine. An image started without a
  reset of the A55's logical machine (a kexec from Linux, a jump from
  U-Boot) finds the GPU domain and gate as Linux or U-Boot left them and
  votes again on top (`PowerStateSet` and `ClockConfigSet` record this
  agent's state, they do not count). A logical-machine reset drops the
  agent's votes (`rpc_scmi_clock.c`, `ClockResetAgentConfig`).
- The M7's agent has no GPU permission (`verdin-imx95.cfg:380` onwards):
  it can neither hold the domain on nor switch it off under us. A domain
  that still reads ON after our OFF means another A55-side vote, and
  `power_off` reports it as a read-back failure.

Differences from NXP's EVK (`mx95evk.cfg` at `eec23287` against
`verdin-imx95.cfg`, comments and blank lines aside): the board name; GPIO1,
CAN_FD1, LPUART2/3 pins and EDMA2 channel 0-1 moved between the M7's and the
A55's sections or turned from OWNER to ACCESS; AP-NS may start and stop the
M7 (`LMM_1 ALL`, `PD_M7 ALL`, `:408-409` region); the A55 watchdog fault
resets the system instead of the logical machine; the EVK's board controls
(PCA2131, buttons, wake lines) are gone; two DDR/M7MIX memory regions
added. **No GPU line differs**: the EVK's AP-NS has the same three
permissions (`configs/mx95evk/config_scmi.h:253`, `:451`, `:459`).

The risk this note was asked to settle: a Zephyr image on the A55 is not
refused the GPU by the Verdin's SM as configured in Toradex's public
repository. What could still refuse it on a real module is a different SM
binary than that repository's `main` (section "Day one").

## 4. What Zephyr's SCMI drivers at the pin can send

- Power domain: `scmi_power_state_set` (`drivers/firmware/scmi/power.c:67`,
  synchronous only, `:83-85`) and `scmi_power_state_get` (`:28`). Enough
  for steps 5, 6 and the power-off.
- Clock: `scmi_clock_config_set` (`drivers/firmware/scmi/clk.c:214`),
  `scmi_clock_rate_get` (`:70`), `scmi_clock_rate_set` (`:109`),
  `scmi_clock_attributes` (`:324`), `scmi_clock_get_permissions` (`:342`);
  through the clock-control API `clock_control_on/off/get_rate/set_rate`
  (`drivers/clock_control/clock_control_arm_scmi.c:119-124`). Enough for
  steps 9 and 10, but clock 174 has no binding name in Zephyr
  (`IMX95_CLK_GPU_CGC` is missing). `scmi_clock_on_off` returns 0 without
  sending when the SM says state control is not allowed (`:50-59`), so a
  board image would not see a refusal for clock 83 through it.
- Performance: **missing.** No driver (`drivers/firmware/scmi/` has base,
  clock, power, pinctrl, reset, system and NXP's cpu, no perf), no DT node
  (the A55 dtsi has protocols 0x11, 0x14 and 0x19 only,
  `nxp_mimx95_a55.dtsi:112-127`) and no `IMX95_PERF_*` binding. Steps 2,
  3, 7 and 8 cannot be sent through a Zephyr protocol driver.
- Errors: the drivers turn the SCMI status into an errno
  (`drivers/firmware/scmi/core.c:16-45`, DENIED to `-EACCES`), losing the
  name. `scmi_send_message` (`core.c:140`) hands back the reply with the raw
  status word in its content, and the header it sends carries the protocol
  id, while the channel is shared by all protocols on this SoC.

So on the board, `gpu_power`'s `scmi::Transport` sits on
`scmi_send_message` through one protocol instance that exists (the clock
protocol's): the header names 0x11, 0x13 or 0x14, the bytes go out as
built, the status comes back untouched. That needs no Zephyr change; a
`protocol@13` node and a performance driver upstream would be the clean
form later. A shared-memory window of 0x80 bytes (`nxp_mimx95_a55.dtsi:98`)
holds the largest reply here: DESCRIBE_LEVELS with four levels, 8 + 4 + 80
bytes after the 24-byte channel header.

## Day one on the board

1. The SM build on the module: its version (BASE protocol vendor and
   implementation version, or the SM's boot banner) against Toradex's
   `main`; if it differs, read that build's AP-NS permissions for clock 174,
   domain 14 and performance domain 9 before trusting section 3.
2. From the image, the reads alone: the three protocol versions, the GPU's
   DESCRIBE_LEVELS (expect indices 0-3 at 24/500/800/1000 MHz), POWER_STATE_GET
   14, PERFORMANCE_LEVEL_GET 9 and CLOCK_RATE_GET 174 as the previous stage
   left them; then CLOCK_GET_PERMISSIONS 174 and 83 (expect state, parent
   and rate for 174, none for 83).
3. The power-on: every reply status 0, the level read back, the rate read
   (expect 800 MHz or the PLL's rounding of it), then the GPU's `GPU_ID`
   at 0x4D900000 (Mali-G310). Whether the SM's level change while the
   domain is on needs the domain on first is assumed (`DEV_SM_PerfPowerCheck`
   only asks it for the DDR and A55 domains).
4. What Linux really does with CLOCK_RATE_SET 174 (section 2): read it from
   the SM's log or a Linux boot on the module; it decides nothing in our
   sequence, which sends none.
5. The power-off: domain 14 reads OFF after our OFF (no other vote), and a
   second power-on works.
