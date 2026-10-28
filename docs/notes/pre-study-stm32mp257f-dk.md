<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: STM32MP257F-DK (VeriSilicon Vivante GPU) before the board arrives

Date 2026-10-28, main `965ac247`. Host research only: no file under `src/`,
`driver/` or `tools/` changed, no board run. Written for the port that would
follow the i.MX8MP's (`imx8mp-gc-bringup.md`, `imx8mp-gc-driver-notes.md`,
`driver/docs/vivante-kernel-rust.md`).

## Verdict in six lines

1. The GPU is the same family as the i.MX8MP's GC7000UL: every ST hardware
   database row Mesa carries for a model 0x7000 or 0x8000 NanoUltra core has
   HALTI5, unified samplers, the secure AHB, no BLT engine, and the same limits
   (512 instructions, 64 temporaries, 320 constants, 16 varyings, one pixel
   pipe). **`src/vivante` extends; no new back end.** (Section 3.)
2. Which of the four candidate rows is the MP25's GPU is **not known** from
   anything readable here. Day one reads it from the chip's identity
   registers. (Section 1, question Q1.)
3. The register-level and ISA differences from the i.MX8MP's row that Mesa
   reads are three: `SH_NO_ONECONST_LIMIT` (an instruction may read two
   different uniforms), `WIDELINE_TRIANGLE_EMU` (Mesa caps the line width at
   1.0), and NPU fields in the row. One row also has one shader core and eight
   streams. (Section 3.)
4. What is new is everything around the GPU: the power and clocks (SCMI to
   OP-TEE, not a clock controller we program), the display (not read yet), the
   Zephyr A35 SoC and board (Zephyr has only the M33), and the identity table
   in the kernel side, which today refuses any part but the i.MX8MP's.
   (Sections 4 and 5.)
5. Most of the GPU work can be done on the host before the board arrives, with
   the drm-shim method the i.MX8MP and Mali work already use. (Section 5.)
6. The first board day is identity, then power-on through SCMI, then the three
   Vivante samples in their old order. (Section 6.)

## What was read, and what was not

Read for this note, all on this host:

| short name | what | version |
| --- | --- | --- |
| Mesa | `src/etnaviv/` (hwdb, hw headers, isa, drm, drm-shim, ci) and `src/gallium/drivers/etnaviv/` | 26.2.1 (the tree `tools/vivante/README.md` builds `etna_shc` in) |
| Zephyr | `boards/st/stm32mp257f_dk`, `soc/st/stm32/stm32mp2x`, `dts/arm/st/mp2`, `drivers/firmware/scmi` | 4.4.99 (the lab workspace `ZEPHYR_WS`) |
| ours | `src/vivante`, `src/shaderc/online/vivante`, `driver/rust/src/gpu/vivante`, `tools/vivante`, the notes named in each section | main `965ac247` |

**Not read, because this worker could not reach the network** (every fetch
beyond the git remote was refused): ST's reference manual RM0457, the user
manual UM3385, the board schematic MB1605, the ST wiki, the ST community
thread about the OP-TEE panic, the Linux sources (`stm32mp25*.dtsi`,
`drivers/gpu/drm/etnaviv`, ST's U-Boot), and the OP-TEE and TF-A sources of
ST's SCMI server. The lab's Linux tree (`linux-v6.18-src`) holds only
`drivers/gpu/drm/panthor` and `drivers/iommu`. Every statement below that
would need one of them is written as a question in section 7 with the source
that answers it. Where this note repeats something the task brief said (the
OP-TEE panic in the PLL clock init when etnaviv loaded, ST's `scarthgap`
release, 2025), it says so; that thread was not re-read.

The Mesa hwdb was read with a throw-away script (not committed) that splits
`gChipInfo[]` into entries and compares fields; the entries and the lines
below are what to `grep` to repeat it. The repository has no tool called
`ETNA_HWDB`; `tools/vivante/mesa-cs-golden.patch` and `etna_shc.c` read the
same hwdb through Mesa itself.

## 1. The hardware

### SoC and board (from Zephyr's board document)

`boards/st/stm32mp257f_dk/doc/index.rst` (Zephyr 4.4.99) lists for the
STM32MP257F-DK: STM32MP257FAI3 with dual-core Arm Cortex-A35 (1.5 GHz maximum)
and one Cortex-M33 (400 MHz, FPU, MPU, TrustZone), the STPMIC25 power
management IC, 32 Gbit LPDDR4, 64 Gbit eMMC, 808 KiB internal SRAM, Gigabit
Ethernet, USB 2.0 and 3.0, a 4-lane LVDS connector, an HDMI connector, a
2-lane CSI-2 camera connector, an on-board STLINK-V3EC with a virtual COM
port, and (lines 84 to 88) "TrustZone peripherals, display secure layers" and a
"complete resource isolation framework". Its link list names the public
documents: the reference manual RM0457, the user manual UM3385, the
schematic MB1605, and the datasheet. These are public; none was read here.

There is no public programming manual for the GPU itself, as for the i.MX8MP
(`imx8mp-gc-driver-notes.md`, "Sources"). The reference manual is expected to
give only the integration (base address, clock, reset, interrupt, security
assignment). That is a question (Q4, Q5, Q6), not a fact.

### The GPU core: four candidate rows, none confirmed

Mesa 26.2.1 carries ST's own database, `src/etnaviv/hwdb/st/gc_feature_database.h`
("Auto created on 2024-12-23", line 56), beside NXP's. Neither the file nor
anything else under `src/etnaviv`, `src/gallium/drivers/etnaviv` or `docs`
says which ST part a row belongs to (a search for `stm32`, `mp25` and
`STMicro` found nothing). The file has six rows. Two are not candidates:
`gc7000nano_0x4652` (line 996, model 0x400) and `gc600L_0x465x` (line 1927,
model 0x600). Four are model 0x7000 or 0x8000 and are the candidates:

| row (comment line) | model | revision | product | eco | customer | shader cores | streams |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `GC8000NANOULTRA31_vip1` (:2858) | 0x8000 | 0x6207 | 0x0a080007 | 0 | 0x19 | 1 | 8 |
| `GCNANOULTRA31_VIP2` (:3789) | 0x7000 | 0x6204 | 0x00070007 | 0 | 0x13 | 2 | 16 |
| `GCNANOULTRA31_VIP2` (:4720) | 0x8000 | 0x6205 | 0x00080003 | 0 | 0x15 | 2 | 16 |
| `GCNANOULTRA31_VIP2` (:5651) | 0x8000 | 0x6205 | 0x00080003 | 1 | 0x15 | 2 | 16 |

For comparison the i.MX8MP's part is, in NXP's database
(`src/etnaviv/hwdb/nxp/gc_feature_database.h`, the two `gc7000ULN_v123`
entries, comment lines 18839 and 19733, the rows `src/vivante/backend.rs:766`
cites): model 0x7000, revision 0x6204, product 0x70003, eco 0 and 1, customer
0x11, 2 shader cores, 16 streams. (`driver/docs/vivante-kernel-rust.md` names
the `gc7000UL_6200` row, which is the informal-match fallback galcore uses;
the exact-match rows are these.)

How a row is found: `gcQueryFeatureDB` matches chip id, revision, product,
eco and customer exactly among the formal rows, then a revision masked with
0xfff0 among the informal ones (`st/gc_feature_database.h:6584-6625`), and
Mesa fills the ids from the kernel's parameters only when the DRM interface is
1.4 or newer (`src/etnaviv/drm/etnaviv_gpu.c:254-261`). So the five ids that
the GPU reports decide which row the compiler sees.

What is common to all four rows (read from the entries, repeatable by
`grep` for the `gcFEATURE_BIT_` comment names): `REG_Halti5` 1,
`REG_UnifiedSamplers` 1, `SECURITY_AHB` 1, `REG_BltEngine` 0, `DEC400` 0,
`REG_InstructionCache` 1, and the limits instruction count 512, temporaries
64, constants 320, varyings 16, pixel pipes 1, vertex output buffer 1024,
vertex cache 16. Mesa derives HALTI 5 from `REG_Halti5`
(`src/etnaviv/drm/etnaviv_gpu.c:210-230`).

The four rows also carry NPU fields: `NNCoreCount` is 2 in each (the
i.MX8MP's GPU row has 0), so Mesa marks them `CORE_NPU` as well as `CORE_GPU`
(`etna_hwdb.c:39`). Whether that is the MP25's separate NPU sharing a naming
scheme or part of the same core is Q2.

### Display path (not established)

What is read: the board has an LVDS connector (4 lanes) and an HDMI
connector (Zephyr's board document, "Features" and "Display & Camera"). The
brief names "the DSI or HDMI path". Zephyr's MP2 device trees describe only
the M33's peripherals; there is no display controller, LVDS or DSI node in
`dts/arm/st/mp2/*.dtsi`, and Zephyr has `drivers/display/display_stm32_ltdc.c`
for other STM32 families, not wired for the MP2. How a frame reaches the panel
or HDMI (controller, layers, LVDS or an external bridge for HDMI, the bridge's
control bus, the backlight) is Q7, answered by UM3385, the MB1605 schematic
and ST's Linux device tree for the DK.

### Boot chain (partly read, partly the brief's)

Read: the DK ships ST's OpenSTLinux; the A35 runs Linux and starts the M33
firmware through remoteproc (`index.rst:139-167`), and Zephyr says of the A35
"Not yet supported in Zephyr" (`index.rst:124-127`). From the brief: OP-TEE
runs in the chain and serves the GPU's clocks through SCMI (an OP-TEE panic in
its PLL init when etnaviv loaded). The expected chain is therefore Boot ROM,
then ST's first-stage loader, then OP-TEE (resident, secure), then U-Boot,
then the OS, with the OS reaching OP-TEE's SCMI server by an SMC call. That
order is the ST wiki's to confirm (Q8); it was not read here.

The Mali port met the same shape on the RK3588: the GPU clock is a firmware
service reached by SCMI over SMC (`driver/docs/nanopc-t6-mali.md:74`, and
the section "The GPU clock: the CRU's, never SCMI" near line 1233, where the
board turned out to use the clock unit directly). That is the precedent for
reading the firmware's side before assuming how the GPU is turned on.

## 2. The open-source stack today

**Mesa.** `src/gallium/drivers/etnaviv` is the GL driver; HALTI5 cores use
descriptor-based textures (`src/etnaviv/hw/texdesc_3d.xml.h`) and fetch
shaders from memory through the instruction cache
(`etnaviv_screen.c:901-906`, "this core must load shaders from memory"). Mesa's
own CI runs dEQP-GLES2 on a GC7000 revision 0x6204 (`src/etnaviv/ci/deqp-etnaviv-gc7000-r6204.toml`,
with `etnaviv-gc7000-r6204-fails.txt`, 22 lines, and `flakes.txt`, 3 lines):
the i.MX8MP's part. No CI file names an ST part. For ES 2.0 work the driver is
complete enough that the i.MX8MP port already passes dEQP-GLES2 with zero
failures (`tools/cts/STATUS.md`); the open question is only whether the ST
row differs in a way that breaks that (section 3).

**Kernel.** Linux `drivers/gpu/drm/etnaviv` is the kernel driver, GPL, and
the brief says it is what loads on the DK. Not readable here (Q9: does it
carry an entry for the MP25's ids, and what does its device tree give). Per
`driver/docs/vivante-kernel-rust.md` our kernel side is original Rust built
from permissive sources only; Linux etnaviv is read for facts and never copied,
and this note keeps that rule.

**Firmware.** The Vivante front end has no firmware: the host writes command
streams into memory and the front end fetches them (`imx8mp-gc-driver-notes.md`,
"The target"). Nothing read suggests the ST row differs. Whether ST's
secure side must program anything before a non-secure client may use the GPU
is Q4.

**ISA and command-stream knowledge we would rely on** (Mesa 26.2.1, MIT):

- Register and state descriptions: `src/etnaviv/hw/state.xml.h`,
  `state_3d.xml.h`, `state_blt.xml.h`, `common.xml.h`, `common_3d.xml.h`,
  `cmdstream.xml.h`, `texdesc_3d.xml.h`. The i.MX8MP backend already uses them
  (`driver/src/vivante/hw`, `driver/docs/vivante-kernel-rust.md` "Sources").
- Instruction set: one description for all models, `src/etnaviv/isa/etnaviv.xml`
  with its assembler and disassembler. The only chip-dependent input to the
  assembler is the flag `has_no_oneconst_limit`
  (`src/gallium/drivers/etnaviv/etnaviv_asm.c:64-72`).
- Compiler back end: `src/gallium/drivers/etnaviv/etnaviv_compiler_nir*.c`,
  the reference our online compiler is checked against (`tools/vivante`).
- Feature extraction: `src/etnaviv/hwdb/etna_hwdb.c` (the map from database
  bits to Mesa's features and limits).

## 3. What yakogl already has that applies

**Decision: the existing Vivante back end extends to this GPU; a new back end
is not needed.** `src/vivante` (the draw translation), `src/shaderc/online/vivante`
(our compiler for the GC ISA, held to Mesa's output), `driver/rust/src/gpu/vivante`
(front end ring, MMU v2, events, reset) and `tools/vivante` (the reference
tools) all target model 0x7000 revision 0x6204 of one family, and the ST rows
are the same family. `src/v3d`, `src/pvr` and `src/mali` are different
hardware and do not apply.

### Differences read from Mesa's hwdb, against the i.MX8MP's `gc7000ULN_v123`

Compared field by field (ST candidates against NXP `gc7000ULN_v123`, eco 0).
All four candidate rows share the results unless a column says otherwise.

| what | i.MX8MP | ST rows | effect on us |
| --- | --- | --- | --- |
| `HALTI5`, `UNIFIED_SAMPLERS`, `INSTRUCTION_CACHE`, `SECURITY_AHB`, no `BLT_ENGINE` | same | same | none: the clear is still the resolve engine, shaders still come from memory, the MMU enable is still the secure-AHB sequence |
| limits: instructions 512, temps 64, constants 320, varyings 16, pixel pipes 1 | same | same | `src/vivante/state.rs:12-24` constants hold |
| shader cores | 2 | 2; **1** in the 0x8000/0x6207 row | the thread count does not change in the row, nothing in our code reads the core count (to confirm by grep in the item list) |
| streams (vertex) | 16 | 16; **8** in the 0x6207 row | `GC_MAX_VERTEX_ELEMENTS` is 16 elements, not streams; the backend's stream use must be checked against 8 (`src/vivante/state.rs` `GcStream`) |
| `SH_NO_ONECONST_LIMIT` | 0 | **1** | Mesa then lets one instruction read two different uniforms (`etnaviv_asm.c:70`, `etnaviv_compiler_nir.c:851`); the i.MX8MP does not. Our compiler's output obeys the stricter rule (it runs on the i.MX8MP), so it stays valid. Relaxing it for this core is an optimisation, not a port item |
| `WIDELINE_TRIANGLE_EMU` | 0 | **1** | Mesa reports a maximum line width of 1.0 instead of 8192 (`etnaviv_screen.c:366-368`). We write `PA_WIDE_LINE_WIDTH0/1` (`src/vivante/backend.rs:1376,1425`) and `GL_ALIASED_LINE_WIDTH_RANGE` for this build: wide lines need a different path or a smaller range (Q11) |
| `NNCoreCount` and the NN/TP/VIP fields | 0 | 2 | Mesa marks the core `CORE_NPU` too and `VIP_V7` (and `NN_XYDP0` in the 0x6207 row). No effect on the GPU path; check `etna_shc` and the shim with such a row (item 5.2) |
| `SH_NO_INDEX_CONST_ON_A0` | 0 | **1** | Mesa does not read this bit (no `ETNA_FEATURE` for it in `etna_hwdb.c`). By its name, constants cannot be read through the address register. We already read a run-time-indexed uniform from memory (`tools/vivante/README.md:67-71`, the i.MX8MP reads element 0 with `movar`), so this agrees; an inference, Q12 |
| `SH_MOVAI_MOVAR_UNUSED_COMPONENTS_WRITE_DIRTY_DATA_FIX` | 1 | **0** | By its name, the fix for partial-component address-register writes is absent. Whether our compiler ever emits one is unchecked (Q12) |
| `REG_Evis`, `SH_TEXLD_U_FIX`, `TX_BORDER_CLAMP_FIX`, `FE_DRAW_DIRECT`, `MULTIVIEW_RENDER`, `PE_ADVANCE_BLEND_PART0`, `TX_UNNORMALIZED_COORD` and other fix bits | 0 | 1 | more bug fixes or extensions in hardware; Mesa maps none of them. No port work, but a candidate for changed behaviour on a conformance case, so the first CTS run is diffed against the i.MX8MP's list |
| `ChipID`, `ChipRevision`, `ProductID`, `CustomerID`, `EcoID` | 0x7000, 0x6204, 0x70003, 0x11, 0 | see the table in section 1 | our kernel side refuses it, below |

Everything in the middle three rows is from the entries in
`st/gc_feature_database.h` and `nxp/gc_feature_database.h`; the two Mesa
users (`etnaviv_screen.c:893-896`, `etna_hwdb.c:104-116`) are why those bits
are the ones that matter to a Mesa-derived design. The bits Mesa does not map
are listed because they are in the row, not because anything is known about
them.

### Where our code is pinned to the i.MX8MP

- **Identity.** `gpu::vivante` matches only model 0x7000, revision & 0xfff0 ==
  0x6200, product 0x70003, and also requires revision 0x6204
  (`driver/docs/vivante-kernel-rust.md`, "Identity: what changed"). Any ST row
  is -ENODEV or "identified from its registers alone, with a warning". The
  table becomes data keyed on all five ids, with the limits of the row.
- **Constants** in `src/vivante/state.rs`, `regs.rs` and `backend.rs`
  (`GC_NUM_CONSTANTS` 320, `NON_POWER_OF_TWO` with a comment citing the NXP
  rows, `GLES_VS_TEX_UNITS` in `src/backend.rs:664`, the renderer string). The
  limits match; the comments cite the NXP rows and need the ST row beside them.
- **Power.** `driver/rust/src/gpu/vivante/power.rs` is built over
  `soc::imx8mp::{ccm,gpc}`. The MP25 side is a different module (SCMI),
  section 4.
- **Reference tables.** `tests/reference/vivante_blobs.rs` is a Mesa reference
  for the i.MX8MP's ids. A different ISA flag (`has_no_oneconst_limit`) can
  change instruction words, so an ST table is its own artefact (item 5.2); the
  product compiles with our own compiler either way.
- **A runtime or a build choice.** The GL core selects the back end by Cargo
  feature (`gpu-vivante`). A chip profile (limits, line width, quirk flags)
  chosen at run time from the identity is the smaller change than a second
  feature, because the limits agree; this is a design decision for the first
  item, not made here.

## 4. The board on Zephyr

**Does Zephyr have this SoC and board?** Partly. Zephyr 4.4.99 has SoC series
`stm32mp2x` (`soc/st/stm32/stm32mp2x/Kconfig.soc`: `SOC_STM32MP215FXX_M33`,
`SOC_STM32MP257FXX_M33`) and the boards `stm32mp257f_dk` and `stm32mp257f_ev1`,
built as `stm32mp257f_dk/stm32mp257fxx/m33`. **Only the Cortex-M33 is a
target.** The board document says of the A35: "Not yet supported in Zephyr".
`dts/arm64` has no `st` directory. The M33's device tree
(`dts/arm/st/mp2/stm32mp2_m33.dtsi` and the per-SoC files) has no GPU,
display, LVDS, GIC or SCMI node; its DDR windows are two regions of 8 MiB, and
the M33 is started by Linux through remoteproc. The M33 is not our target (the
three finished ports run on application cores, and a GPU clock, a framebuffer
and a carveout need more than 8 MiB windows). So:

- **A35 SoC and board definition** are new and ours, as the i.MX8MP's A53 was
  (`imx8mp-gc-bringup.md`, step 3: Zephyr shipped the M7 board only, so the A53
  became a board extension in the platform repository) and the AM62P's
  (`driver/boards/toradex/verdin_am62p`, `driver/soc/ti`). Needed: an arm64 SoC
  (Cortex-A35, which is Armv8.0-A; Zephyr's `armv8-a.dtsi` exists), the
  interrupt controller, the arch timer, the console UART, and the PSCI or
  secure-monitor interface the firmware provides. All unread: Q13.
- **Clocks, resets, power domains.** Zephyr has an SCMI subsystem
  (`drivers/firmware/scmi`: `clk.c`, `reset.c`, `power.c`, `smc.c`, `shmem.c`)
  with bindings `arm,scmi`, `arm,scmi-smc`, `arm,scmi-shmem`, `arm,scmi-clock`,
  `arm,scmi-reset`, `arm,scmi-power`. If OP-TEE's server is the only way to
  enable the GPU clock, our image is an SCMI agent over SMC: it needs the SMC
  function id, the shared-memory channel for a non-secure agent, and the clock
  and reset ids of the GPU. All Q3. This replaces `power.rs`'s CCM and GPC
  programming; the brief's OP-TEE panic is the first thing to reproduce or rule
  out with our own request (Q3).
- **Memory map, interrupt, security.** The GPU's base address and size, its
  interrupt line, the free RAM for the carveout and the display buffers, and
  whether the GPU and the display belong to a resource-isolation compartment
  the non-secure A35 may use: Q4, Q5, Q6, Q14. The i.MX8MP port needed the
  same facts and found several by measurement (`imx8mp-gc-driver-notes.md`,
  numbered list, items 5 and 7).
- **Display.** A Zephyr driver for the MP2's display controller and its LVDS or
  HDMI bridge does not exist; the i.MX8MP's display was a driver of ours
  (`driver/docs/imx8mp-display-cold.md`). Q7.

**Boot route for sign-off.** The finished ports are signed off from the board's
own bootloader, never kexec (`imx8mp-netboot.md`, goal; `docs/standalone-boot.md`).
For the DK that is U-Boot (ST's, from the OpenSTLinux boot media) loading our
image over the network or from the boot media, with OP-TEE staying resident so
the SCMI server answers. A jump from Linux (kexec) would be development
only. A direct route that replaces U-Boot, as `imx8mp-direct-boot.md` did, is
a later item and needs the ST chain read first (Q8). Whether the image may
start at the exception level and with the state U-Boot leaves, and whether
U-Boot leaves the display on, are Q8 and Q13.

## 5. What can be done before the board arrives

Each item has a check. All are host-side except where Zephyr is built.

1. **Pin the hwdb comparison.** A small script under `tools/vivante` that
   reads the two Mesa headers and prints the comparison of section 3 (it
   would replace the throw-away script). Check: its output equals the table in
   this note; a gate row like the existing `tools` rows
   (`tools/check.sh` "tools   shader-table generator guards"). Needs a Mesa tree,
   so it is a gap, not a pass, where the tree is missing.
2. **A shim entry per candidate row, and the reference tables.** Add the four
   ST rows' ids to the etnaviv drm-shim next to the `GC7000UL` entry that
   `tools/vivante/mesa-cs-golden.patch` adds (the entry carries the five ids and
   no feature words, so the driver looks the row up in the hwdb:
   `hwdb_ids`, DRM 1.4). Then, for each: (a) `cs_golden` produces the golden
   command stream for the gl_test shapes; diff it against the i.MX8MP's
   golden (`tools/vivante/README.md`, "The golden command stream"). Check:
   the diff is exactly the expected one (line-width state, no
   `SH_NO_ONECONST_LIMIT` effect on the stream, nothing else); anything else is
   a finding. (b) `etna_shc` with the ST ids compiles the 40 `tools/shaders`
   programs and the table generator can write an ST table. Check: how many
   instruction words differ from the i.MX8MP's table; that count is the size of
   the `ONECONST` optimisation and proves our i.MX8MP-valid output is also
   valid on ST. (c) That `etna_shc` and the shim run at all with a row that is
   also `CORE_NPU` (`etnaviv_screen.c:859-870`). The method is the Mali port's:
   Mesa's own driver on the host drm-shim as the reference (`docs/notes/mali-*.md`,
   `tools/mali/drm_shim_ref.sh`).
3. **Table-driven identity in the kernel side.** Replace the single i.MX8MP
   row in `hw.rs` by rows keyed on the five ids, with the four ST rows (and
   a fallback that still identifies from registers and warns). Check: the
   existing model-bus tests (`driver/rust/src/gpu/vivante/tests.rs`) cover
   each row; the i.MX8MP's replay (`the_c_scenario_comes_out_identical`) stays
   byte-identical. This is `cargo test` in the gate, no board.
4. **The chip profile in `src/vivante`.** Limits and quirk flags from the row,
   line-width range from `WIDELINE_TRIANGLE_EMU`. Check: the host-test
   configurations `host-test,gpu-vivante[,shaderc-online]` pass unchanged for
   the i.MX8MP profile; a new test pins the ST profile; `tests/es3_caps.rs`
   gets the profile (grep `YakoglBackend {` in `tests/` and `src/` first, as
   CLAUDE.md says, for any new table field).
5. **The Zephyr A35 SoC and board, built not run.** An arm64 SoC and a
   `stm32mp257f_dk` A35 board with a `gc_identify` sample, built by the gate's
   Zephyr rows. Check: the build exits 0, image size and the linked entry
   points are logged. Needs `ZEPHYR_WS`; without it the row is a gap.
6. **An SCMI client with a model.** Use Zephyr's SCMI subsystem or a small
   Rust client over the model bus (`driver/rust/src/model.rs`) and test the
   message bytes of "clock enable" and "reset deassert" against the SCMI
   specification (Arm DEN0056). Check: byte-exact host tests. The agent id,
   channel and ids are filled in when Q3 is answered.
7. **Read the unread sources** and update this note: RM0457 (GPU, RCC,
   RIFSC, LTDC/LVDS chapters), UM3385, the MB1605 schematic, ST's Linux
   device tree and `etnaviv_hwdb.c` at v6.18, ST's U-Boot and OP-TEE sources
   for the SCMI server and the PLL init. Check: each question in section 7 gets
   a citation or stays an unknown.
8. **A bring-up record template.** The identify, fence, clear and triangle
   samples exist (`imx8mp-gc-bringup.md`, step 4); only the power and the
   addresses change. Check: the samples build for the new board in item 5.

## 6. What needs the board

First three things, in order:

1. **Under ST's Linux, read, do not drive.** The GPU's identity as etnaviv
   prints it (model, revision, product, eco, customer: the row, Q1), the device
   tree node of the GPU (base, interrupt, clocks, resets, power domains,
   IOMMU), `/proc/iomem` and `/proc/interrupts`, and the free RAM for a
   carveout. This settles Q1, Q3 (ids), Q5, Q6, Q9 in one sitting.
2. **From U-Boot, power the GPU through SCMI and read the identity** with the
   Zephyr image (`gc_identify`'s analogue). Alive if `CHIP_MODEL` and
   `CHIP_REV` match one of the four rows, `HI_INTR_ENBL` reads back what was
   written (the i.MX8MP's liveness test, `imx8mp-gc-driver-notes.md`), and the
   reset reaches idle. Do not read the 0x800+ state space from the CPU: two
   builds that did rebooted the i.MX8MP board (`driver/docs/vivante-kernel-rust.md`,
   "Hardware findings that still hold"); assume the same here until shown
   otherwise.
3. **The old order, `gc_fence`, `gc_clear`, `gc_triangle`**, then
   `gc_gl_triangle`.

What says the port is alive: the identity matches a row; a fence retires and
its interrupt arrives; the first clear reads back through the resolve engine;
the first triangle reads back (three apex colours, the i.MX8MP's `gl_triangle`
check); `gl_test` case 1 (the first triangle through GL) passes; then the
regression set against the i.MX8MP's list.

## 7. Risks and unknowns

An unknown stays an unknown. Each is a question, with the source that would
answer it.

| id | question | answered by |
| --- | --- | --- |
| Q1 | Which row is the MP25's GPU: model 0x7000 or 0x8000, revision 0x6204, 0x6205 or 0x6207, which eco and customer id? | the chip's own identity registers (board day one); Linux `etnaviv_hwdb.c` v6.18; RM0457 GPU chapter |
| Q2 | Why do all candidate rows carry NPU fields? Is the NPU a separate block and the row naming shared? | RM0457 (NPU and GPU chapters); the ST wiki |
| Q3 | How is the GPU turned on: which SCMI clock and reset ids, which rate, which SMC function id and shared-memory channel a non-secure agent may use, and does OP-TEE's PLL init panic on our request as it did when etnaviv loaded (the brief)? | OP-TEE and TF-A sources for the stm32mp2 SCMI server; ST's device tree `scmi` nodes; the ST community thread of 2025; board day one |
| Q4 | Is the GPU (and its DMA master) assigned to a resource-isolation compartment that a non-secure A35 image may use, or does the secure side have to set it? Does anything else have to be programmed by the secure side first? | RM0457 (RIFSC, security chapters); OP-TEE `stm32mp2` platform code; board day one |
| Q5 | The GPU's base address and size, interrupt number, and power domain, if any. | RM0457 memory map; ST device tree |
| Q6 | Interrupt controller version, how the GPU's interrupt is routed to the A35, and the arch timer rate. | RM0457; Linux `stm32mp251.dtsi`; Zephyr (no A35 node exists) |
| Q7 | The display path: controller, number of layers, formats and stride alignment it scans out, LVDS and the HDMI chain on the DK (a bridge and its control bus), the backlight. | UM3385, MB1605, ST device tree for the DK, RM0457 LTDC and LVDS chapters |
| Q8 | The exact boot chain and what state U-Boot hands over (exception level, MMU, caches, display on or off), and whether the SMC interface to OP-TEE stays open after U-Boot. | ST wiki boot chapter; ST's U-Boot and TF-A; board day one |
| Q9 | Does Linux etnaviv carry the MP25 ids, and does anything in ST's Linux depend on a vendor blob? | Linux v6.18 `etnaviv_hwdb.c`; the Starter Package's package list |
| Q10 | Is the GPU behind an IOMMU, and what physical address width reaches the GPU? The i.MX8MP port identity-maps a carveout in the GPU's own MMU v2. | ST device tree (iommu property of the GPU); RM0457 |
| Q11 | What do wide lines do on a `WIDELINE_TRIANGLE_EMU` core when we program the wide-line registers the way the i.MX8MP backend does? What should the build report as the line width range? | the golden stream (item 5.2); board test; Mesa `etnaviv_screen.c:366-368` |
| Q12 | Does our compiler ever use the address-register paths or partial-component address writes that the ST rows' `SH_NO_INDEX_CONST_ON_A0` and missing `SH_MOVAI_MOVAR...FIX` suggest are unavailable or unsafe? | a scan of the 40 compiled shaders and the online compiler's output for `movar` and `movai` (item 5.2); the board's gl_test and dEQP runs |
| Q13 | The A35 SoC definition: cache and MMU state at entry, PSCI or the monitor interface, console UART, and whether Zephyr's arm64 core runs on this part without changes. | RM0457; TF-A's stm32mp2 platform; Zephyr `arch/arm64` and `dts/arm64/armv8-a.dtsi` |
| Q14 | The GPU's coherence with the A35 cluster, and the right memory attributes for the carveout. The i.MX8MP uses a non-cacheable mapping and no flushes (`imx8mp-gc-driver-notes.md`, "M3"). | RM0457 interconnect chapter; board test |
| Q15 | Does the 0x6207 row (one shader core, eight streams) apply, and if so does the backend's stream use stay within eight? | Q1; a read of `GcStream` use in `src/vivante` |

Risks that are not questions: the finished ports took the board work (power,
display, boot) as the long part, and the GPU work as the short one; this port
adds SCMI as a new dependency on a secure-side component we do not own. The
OP-TEE panic the brief names is a reason to put the SCMI request first on the
day-one list and to log every message.

## 8. Proposed item list

Sized against the i.MX8MP port, the closest of the three: first power-up to
the first triangle on the board took from 2026-09-16 (start) to 2026-09-20
(`imx8mp-gc-bringup.md`, steps 3 and 4), a dEQP-GLES2 run with zero failures
followed by 2026-10-16 (`tools/cts/STATUS.md`, "ひとことで"), and the common
set by 2026-10-20. "S" is a part of a milestone, "M" about one i.MX8MP
milestone, "L" several. This port starts with the GPU code already written, so
the GPU-side items are smaller than they were then.

| # | item | check | size |
| --- | --- | --- | --- |
| P1 | Read the unread documents (section 5, item 7) and update this note; record the answers to Q1 to Q14 that sources give. Open GitHub issues for what stays open | each question cited or left | S |
| P2 | Pin the hwdb comparison (5.1) and the shim rows with goldens and an ST table (5.2) | gate rows; goldens diffed | M |
| P3 | Table-driven identity (5.3) and the chip profile (5.4) | host tests, both profiles | M |
| P4 | A35 SoC and board in Zephyr, `gc_identify` built (5.5) | Zephyr build row | M |
| P5 | The SCMI client and the ST power module with model tests (5.6) | byte-exact host tests | M |
| P6 | Board day one (section 6, items 1 and 2): identity under Linux, power and identity from U-Boot | the five ids read; `HI_INTR_ENBL` read-back | S, board |
| P7 | `gc_fence`, `gc_clear`, `gc_triangle`, `gc_gl_triangle` | read-backs | M, board |
| P8 | The display: controller, LVDS or HDMI bridge, backlight; scan-out of the GPU's buffer | picture on the panel, a record like `imx8mp-display-cold.md` | L |
| P9 | `gl_test` regression set, then the dEQP-GLES2 sweep, diffed against the i.MX8MP's failures; the line-width decision (Q11) | RESULTS rows | L, board |
| P10 | Netboot or direct boot from the board's bootloader (`imx8mp-netboot.md` shape), sign-off from the bootloader | a run from U-Boot; `tools/board.sh` | M |

P2 to P5 can run before the board arrives; P1 only needs documents.
