<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: STM32MP257F-DK (VeriSilicon Vivante GPU) before the board arrives

Date 2026-10-28, main `6a9fe4c0`. Host research only: no file under `src/`,
`driver/` or `tools/` changed, no board run, no board yet. This is the
second pass of this note: the first pass (`720c8224`, 2026-10-28) closed at
review over citations of our own tree that had drifted or had been named
loosely. Every repository line cited below was re-read against main
`6a9fe4c0`, every Mesa line against the project's Mesa 26.2.1 tree
(`VERSION` 26.2.1), every Zephyr line against the Zephyr 4.4.99 workspace,
and every Linux line against the pinned clone named in the read table, in
this pass. Written for the port that would follow the
i.MX8MP's (`imx8mp-gc-bringup.md`, `imx8mp-gc-driver-notes.md`,
`driver/docs/vivante-kernel-rust.md`).

## Verdict in six lines

1. The GPU is the same family as the i.MX8MP's GC7000UL: every ST hardware
   database row Mesa carries for a model 0x7000 or 0x8000 NanoUltra core has
   HALTI5, unified samplers, the secure AHB, no BLT engine, and the same
   limits (512 instructions, 64 temporaries, 320 constants, 16 varyings, one
   pixel pipe). **`src/vivante` extends; no new back end.** (Section 3.)
2. Which of the four candidate rows is the MP25's GPU is **not known** from
   anything readable here. Day one reads it from the chip's identity
   registers. (Section 1, question Q1.)
3. The register-level and ISA differences from the i.MX8MP's row that Mesa
   reads are three: `SH_NO_ONECONST_LIMIT` (an instruction may read two
   different uniforms), `WIDELINE_TRIANGLE_EMU` (Mesa caps the line width at
   1.0), and NPU fields in the row. One row also has one shader core and
   eight vertex streams; our kernel side already models eight for the
   i.MX8MP. (Section 3.)
4. What is new is everything around the GPU: the power and clocks (SCMI
   clocks through OP-TEE per the brief; mainline's model adds plain RCC
   gates fed from SCMI-managed parents), the display (the controller
   and the DK's panel chain are known from Linux mainline now; the
   scan-out details are not),
   the Zephyr A35 SoC and board (Zephyr has only the M33), and the identity
   table in the kernel side, which today refuses any part but the i.MX8MP's.
   (Sections 4 and 5.)
5. Most of the GPU work can be done on the host before the board arrives,
   with the drm-shim method the i.MX8MP and Mali work already use.
   (Section 5.)
6. The first board day is identity, then power-on through SCMI, then the
   three Vivante samples in their old order. (Section 6.)

## What was read, and what was not

Read for this pass, all on this host:

| short name | what | version |
| --- | --- | --- |
| Mesa | `src/etnaviv/` (hwdb, hw headers, isa, drm, ci) and `src/gallium/drivers/etnaviv/` | 26.2.1 (`VERSION` 26.2.1; the tag the Vivante tools name: `tools/vivante/README.md:5`, `src/vivante/backend.rs:765-767`) |
| Linux | `drivers/gpu/drm/etnaviv/`, `arch/arm64/boot/dts/st/`, `include/dt-bindings/clock/st,stm32mp25-rcc.h`, `Documentation/devicetree/bindings/gpu/vivante,gc.yaml` | master `65ba8f34ec886ec72b2c07ddc8cf3d815c32869d` (dated 2026-10-27), a depth-1 clone fetched 2026-10-28 |
| Zephyr | `boards/st/stm32mp257f_dk`, `soc/st/stm32/stm32mp2x`, `dts/arm/st/mp2`, `dts/bindings/firmware`, `drivers/firmware/scmi`, `drivers/display` | 4.4.99 (`VERSION`, the lab workspace `ZEPHYR_WS`) |
| ours | `src/vivante`, `src/shaderc/online/vivante`, `driver/rust/src/gpu/vivante`, `driver/rust/src/ffi/vivante.rs`, `driver/rust/src/model.rs`, `tools/vivante`, `tools/shaders`, `tests/`, the notes named in each section | main `6a9fe4c0` |

The Mesa hwdb was compared field by field with `grep` over both
`gc_feature_database.h` files and the entry ranges; every value quoted in
sections 1 and 3 is a line in those files, at the line given or greppable
by the quoted name. The repository has no tool called `ETNA_HWDB`
(`git grep ETNA_HWDB` finds nothing); `tools/vivante/etna_shc.c` and
`tools/vivante/mesa-cs-golden.patch` read the same hwdb through Mesa
itself.

**Read late.** The first writing of this version could not reach the
network at all. A fetch grant arrived later in the review: the Linux
mainline was cloned on 2026-10-28 and read at the pinned commit named in
the table above, and what it answers is recorded in sections 1, 2, 5, 6
and 7.

**Not read, and why.** ST's reference manual RM0457, the user manual
UM3385, the board schematic MB1605 and the datasheet (all four with public
URLs, listed in Zephyr's board document at `index.rst:199-212`), the ST
wiki, the ST community thread about the OP-TEE panic, and ST's Starter
Package sources — its own kernel device tree, its `etnaviv` fork, its
U-Boot, and the OP-TEE and TF-A sources of ST's SCMI server — were not
read: this node reaches `git.kernel.org` and nothing else. Everything that
would need one of them is written as
a question in section 7 with the source that answers it; a question stays a
question, and no verdict above depends on one. Where this note repeats
something the task brief said (the OP-TEE panic in the PLL clock init when
etnaviv loaded, ST's `scarthgap` release, 2025), it says so; that thread
was not re-read.

## 1. The hardware

### SoC and board (from Zephyr's board document)

`boards/st/stm32mp257f_dk/doc/index.rst` (Zephyr 4.4.99) lists for the
STM32MP257F-DK: the STM32MP257FAI3 in a VFBGA424 package (`:16-17`) with a
dual-core Arm Cortex-A35 at 1.5 GHz max and one Cortex-M33 at 400 MHz max
with FPU, MPU and TrustZone (`:8`, `:60-61`), the STPMIC25 power management
IC, 32-Gbit LPDDR4 (`:19`), 64-Gbit eMMC v5.1 (`:20`), 808-Kbyte internal
SRAM (`:66`), Gigabit Ethernet (`:92`), USB 2.0 and USB 3.0 Type-C
(`:33-34`), a microSD holder, a dual-lane MIPI CSI-2 camera expansion
connector, an HDMI connector and an LVDS connector (`:35-38`), the "LVDS
interface (4-lane)" and "Camera CSI-2 interface (2-lane)" under *Display &
Camera* (`:99-100`), and an on-board STLINK-V3EC debugger with a virtual
COM port (`:42-44`, `:102-104`). Under *Security/Safety* it names "Secure
boot, TrustZone peripherals, active tamper, environmental monitors,
display secure layers, hardware accelerators" and a "Complete resource
isolation framework" (`:84-88`). Its link list names the public documents
with URLs: the reference manual RM0457, the UM3385 user manual, the MB1605
C01 schematic and the datasheet (`:195-224`). These are public; none was
read here (network refused, above).

There is no public programming manual for the GPU itself, as for the
i.MX8MP (`imx8mp-gc-driver-notes.md:32`, "Sources, and what 'no register
value without a reference' means here"). The reference manual is expected to give
only the integration (base address, clock, reset, interrupt, security
assignment). That is a question (Q4, Q5, Q6), not a fact.

### The GPU core: four candidate rows, none confirmed

Mesa 26.2.1 carries ST's own database,
`src/etnaviv/hwdb/st/gc_feature_database.h` ("Auto created on 2024-12-23
15:24", `:56`), beside NXP's. Neither the file nor anything else under
`src/etnaviv`, `src/gallium/drivers/etnaviv` or `docs` says which ST part a
row belongs to (a search for `stm32`, `mp25` and `STMicro` found nothing).
The file has six rows. Two are not candidates: `gc7000nano_0x4652` (`:996`,
model 0x400) and `gc600L_0x465x` (`:1927`, model 0x600). Four are model
0x7000 or 0x8000 and are the candidates; the ids are the entry lines:

| row (comment line) | model | revision | product | eco | customer | shader cores | streams |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `GC8000NANOULTRA31_vip1` (`:2858`) | 0x8000 (`:2860`) | 0x6207 (`:2861`) | 0xa080007 (`:2862`) | 0 (`:2863`) | 0x19 (`:2864`) | 1 (`:2870`) | 8 (`:2894`) |
| `GCNANOULTRA31_VIP2` (`:3789`) | 0x7000 (`:3791`) | 0x6204 (`:3792`) | 0x70007 (`:3793`) | 0 (`:3794`) | 0x13 (`:3795`) | 2 (`:3801`) | 16 (`:3825`) |
| `GCNANOULTRA31_VIP2` (`:4720`) | 0x8000 (`:4722`) | 0x6205 (`:4723`) | 0x80003 (`:4724`) | 0 (`:4725`) | 0x15 (`:4726`) | 2 (`:4732`) | 16 (`:4756`) |
| `GCNANOULTRA31_VIP2` (`:5651`) | 0x8000 | 0x6205 | 0x80003 | 1 | 0x15 | 2 (`:5663`) | 16 (`:5687`) |

For comparison the i.MX8MP's part is, in NXP's database
(`src/etnaviv/hwdb/nxp/gc_feature_database.h`, the two `gc7000ULN_v123`
entries, comment lines `:18839` and `:19733`, the ids `:18841-18845` and
the rows `src/vivante/backend.rs:762-771` cites): model 0x7000, revision
0x6204, product 0x70003, eco 0 and 1, customer 0x11, 2 shader cores, 16
streams (`:18875`, `:19769`). (`driver/docs/vivante-kernel-rust.md:93`
names the `gc7000UL_6200` row — our bring-up's copy of the row Linux's own
hwdb carries for the same part (`drivers/gpu/drm/etnaviv/etnaviv_hwdb.c:72-86`
at the pinned clone: model 0x7000, revision 0x6202, product 0x70003,
customer 0, eco 0, **Streams 8** at `:78`), informal in that our copy
matches on revision & 0xfff0 — which reads **Streams 8** where Mesa's
formal rows read 16. Our kernel side uses that 8; section 3.)

Linux's hwdb is an independent check on the candidate rows. It carries
model 0x8000 / revision 0x6205 / product 0x80003 / customer 0x15 / eco 0
(`etnaviv_hwdb.c:199-204`) — the id of ST's third Mesa row, exactly —
with Streams 16 (`:205`), 2 shader cores (`:208`), 2 NN cores (`:209`) and
the limits the Mesa rows carry: 64 temporary registers (`:206`), 512
threads (`:207`), 512 instructions (`:213`), 320 constants (`:214`), 1
pixel pipe (`:212`), 16 varyings (`:216`). It carries model 0x7000 /
revision 0x6204 with product and customer left open (`:136-140`, the file
uses `~0U` there), which matches ST's second row under the wildcards the
lookup allows (`:306-313`). And it has no row for model 0x8000 / revision
0x6207, the id of ST's first row: for a chip answering to that id the
lookup finds nothing (`:326`) and the driver keeps the identity the
registers gave. In open data, then, the eight-stream rows are 0x7000/0x6202
(kernel `:78`; ST's Mesa row 1) and every known 0x6204 and 0x6205 row —
Mesa's and Linux's — reads 16 streams.

How a row is found: `gcQueryFeatureDB` (st file `:6585`) matches chip id,
revision, product, eco and customer exactly among the formal rows, then a
revision masked with 0xfff0 among the informal ones, and Mesa fills the ids
from the kernel's parameters only when the DRM interface is 1.4 or newer
(`src/etnaviv/drm/etnaviv_gpu.c:254-261`). So the five ids that the GPU
reports decide which row the compiler sees.

What is common to all four rows (read from the entries, repeatable by
`grep` for the quoted names): `gcFEATURE_BIT_REG_Halti5` 1 (`:3195`,
`:4126`, `:5057`, `:5988`), `gcFEATURE_BIT_REG_UnifiedSamplers` 1,
`gcFEATURE_BIT_SECURITY_AHB` 1, `gcFEATURE_BIT_REG_BltEngine` 0,
`gcFEATURE_BIT_DEC400` 0, `gcFEATURE_BIT_REG_InstructionCache` 1, and the
limits `gcFEATURE_VALUE_InstructionCount` 0x200, `TempRegisters` 0x40,
`NumberOfConstants` 0x140, `VaryingCount` 0x10, `NumPixelPipes` 1,
`VertexOutputBufferSize` 0x400, `VertexCacheSize` 0x10 (`:2868-2898` and
the same fields in the other three rows). Mesa derives HALTI 5 from the
HALTI5 bit (`src/etnaviv/drm/etnaviv_gpu.c:210-230`, `determine_halti`).

The four rows also carry NPU fields: `gcFEATURE_VALUE_NNCoreCount` is 2 in
each (`:2910`, `:3841`, `:4772`, `:5703`; the i.MX8MP's GPU row has 0), so
Mesa marks them `ETNA_FEATURE_CORE_NPU` as well as `CORE_GPU`
(`src/etnaviv/hwdb/etna_hwdb.c:39-40`). Whether that is the MP25's separate
NPU sharing a naming scheme or part of the same core is Q2.

### Display path (partly mainline now)

What is read: the board has HDMI and LVDS connectors and a "LVDS interface
(4-lane)" (Zephyr's board document `:37-38`, `:97-100`, above). The task
brief names "the DSI or HDMI path"; no source read here names a DSI path
for this board. Zephyr's MP2 device trees describe only the M33's
peripherals: there is no display controller, LVDS or DSI node in
`dts/arm/st/mp2/*.dtsi` (a `grep` for `ltdc`, `gpu`, `lvds`, `dsi`, `hdmi`,
`gic` and `scmi` over `stm32mp2_m33.dtsi` finds none), and Zephyr has
`drivers/display/display_stm32_ltdc.c` for the families whose device tree
carries an `st,stm32-ltdc` node (`drivers/display/Kconfig.stm32_ltdc:9`
gates on `DT_HAS_ST_STM32_LTDC_ENABLED`; its framebuffer options further
depend on the H7 and F7 series, `:78`), not for the MP2. Linux mainline,
read later in this pass, names the A35-side shape: an LTDC at
`display-controller@48010000` (`stm32mp251.dtsi:1617`), an LVDS bridge at
0x48060000 (`stm32mp255.dtsi:15`) and, on this very board's tree, LTDC →
that bridge → an LVDS panel (`stm32mp257f-dk.dts:98-127`, `:274-299`). So
the controller is LTDC; the layers, formats, stride alignment and the
HDMI chain (an external bridge, its control bus, the backlight) remain
Q7, answered by UM3385, the MB1605 schematic, RM0457's LTDC and LVDS
chapters and ST's Starter Package tree for the DK.

### Boot chain (partly read, partly the brief's)

Read: the DK ships ST's OpenSTLinux; the board document says "Zephyr OS is
ported to run on the Cortex®-M33 core, as a coprocessor of the Cortex®-A35
core" (`index.rst:10-11`), that under *System Clock* the Cortex-A35 is
"Not yet supported in Zephyr" (`:124-127`), that the M33 "requires the
Cortex®-A35 to load and start the firmware using remoteproc"
(`:139-145`), and that in the current
distribution the A35 loads the M33 image into DDR or SRAM and starts it
(`:154-157`). From the brief: OP-TEE runs in the chain and serves the GPU's
clocks through SCMI (an OP-TEE panic in its PLL init when etnaviv loaded).
The expected chain is therefore Boot ROM, then ST's first-stage loader,
then OP-TEE (resident, secure), then U-Boot, then the OS, with the OS
reaching OP-TEE's SCMI server by an SMC call. That order is the ST wiki's
to confirm (Q8); it was not read here.

The Mali port met the same shape on the RK3588: there the GPU rate goes
through "SCMI clock 5 (SMC 0x82000010)" (`driver/docs/nanopc-t6-mali.md:74`)
and the SCMI interface is reached from U-Boot too (`:512`, `:1542`), yet the
measured result was that the clock unit is programmed directly
("The GPU clock: the CRU's, never SCMI", `:1233`). That is the
precedent for reading the firmware's side before assuming how the GPU is
turned on.

## 2. The open-source stack today

**Mesa.** `src/gallium/drivers/etnaviv` is the GL driver. The ST candidate
rows set `REG_UnifiedSamplers` 1, which Mesa reads as
`screen->specs.unified_samplers` (`etnaviv_screen.c:841-845`, which also
requires HALTI5), and its texture descriptor path is
`etnaviv_texture_desc.c`, which includes Mesa's rnndb
header `src/etnaviv/hw/texdesc_3d.xml.h` (`:31`). HALTI5 cores fetch
shaders from memory through the instruction cache: for halti >= 5 Mesa
sets `vs_offset`/`ps_offset` to 0 and "Do not program shaders manually"
(`etnaviv_screen.c:901-906`, "GC7000 - this core must load shaders from
memory"). Mesa's own CI runs dEQP-GLES2 on a GC7000 revision 0x6204
(`src/etnaviv/ci/deqp-etnaviv-gc7000-r6204.toml`, with
`etnaviv-gc7000-r6204-fails.txt`, 22 lines, and
`etnaviv-gc7000-r6204-flakes.txt`, 3 lines): the i.MX8MP's part. The other
GC7000 CI entry is revision 0x6214 (`deqp-etnaviv-gc7000-r6214.toml`). No
CI file names an ST part. For ES 2.0 work the driver is complete enough
that the i.MX8MP port already passes dEQP-GLES2 with zero failures on the
board (`tools/cts/STATUS.md`, "ひとことで": three boards zero failures,
10/15-16); the open question is only whether the ST row differs in a way
that breaks that (section 3).

**Kernel.** Linux `drivers/gpu/drm/etnaviv` is the kernel driver, GPL,
read now at the pinned clone named in the read table. It binds only
through a device tree node with compatible `vivante,gc`
(`etnaviv_drv.c:33-43`); the binding requires compatible, `reg`, one
interrupt and named clocks, `bus` / `core` / `shader` / `reg`
(`vivante,gc.yaml:16`, `:21`, `:40`, `:52`), and allows a reset and a
power domain (`:46`, `:49`). Identity is read from the GPU's own
registers — model, revision, date, customer, product, eco
(`etnaviv_gpu.c:372-376`, `:383-384`) — and the driver then looks the ids
up in its own hwdb (`etnaviv_hwdb.c:297-327`, called at
`etnaviv_gpu.c:438`); on a match it uses the row's feature words instead
of the feature registers, "as they might be wrong"
(`etnaviv_gpu.c:434-437`). No ST part is named in the driver or the hwdb,
and mainline's ST device trees carry no gpu node (section 1), so mainline
cannot probe this GPU today; ST's Starter Package tree is where such a
node exists (Q9). Per `driver/docs/vivante-kernel-rust.md:16` our kernel
side is original Rust built from permissive sources only; Linux etnaviv is
read for facts and never copied, and this note keeps that rule.

**Firmware.** The Vivante front end has no firmware: the host writes
command streams into memory and the front end fetches them
(`imx8mp-gc-driver-notes.md`, "The target"). Nothing read suggests the ST
row differs. Whether ST's secure side must program anything before a
non-secure client may use the GPU is Q4.

**ISA and command-stream knowledge we would rely on** (Mesa 26.2.1, MIT):

- Register and state descriptions: `src/etnaviv/hw/state.xml.h`,
  `state_3d.xml.h`, `state_blt.xml.h`, `common.xml.h`, `common_3d.xml.h`,
  `cmdstream.xml.h`, `texdesc_3d.xml.h`. The i.MX8MP backend already uses
  them (`driver/src/vivante/hw`, `driver/docs/vivante-kernel-rust.md`).
- Instruction set: one description for all models, `src/etnaviv/isa/`
  (`etnaviv.xml` with `assembler.c`, `disasm.c`, `encode.c`). The only
  chip-dependent input to the assembler is the flag
  `has_no_oneconst_limit`: `etna_assemble(uint32_t *out, const struct
  etna_inst *inst, bool has_no_oneconst_limit)` and its oneuniform check
  (`src/gallium/drivers/etnaviv/etnaviv_asm.c:62-71`).
- Compiler back end: `src/gallium/drivers/etnaviv/etnaviv_compiler_nir.c`
  and friends, the reference our online compiler is checked against
  (`tools/vivante`).
- Feature extraction: `src/etnaviv/hwdb/etna_hwdb.c` (the map from
  database bits and values to Mesa's features and limits).

## 3. What yakogl already has that applies

**Decision: the existing Vivante back end extends to this GPU; a new back
end is not needed.** `src/vivante` (the draw translation),
`src/shaderc/online/vivante` (our compiler for the GC ISA, held to Mesa's
output), `driver/rust/src/gpu/vivante` (front end ring, MMU v2, events,
reset) and `tools/vivante` (the reference tools) all target model 0x7000
revision 0x6204 of one family, and the ST rows are the same family.
`src/v3d`, `src/pvr` and `src/mali` are different hardware and do not
apply.

### Differences read from Mesa's hwdb, against the i.MX8MP's `gc7000ULN_v123`

Compared field by field (ST candidates against NXP `gc7000ULN_v123`, eco 0,
the `:18839` entry; every value in both files was re-read in this pass).
All four candidate rows share the results unless a column says otherwise.

| what | i.MX8MP | ST rows | effect on us |
| --- | --- | --- | --- |
| `HALTI5`, `UNIFIED_SAMPLERS`, `INSTRUCTION_CACHE`, `SECURITY_AHB`, no `BLT_ENGINE`, no `DEC400` | same | same | none: the clear is still the resolve engine, shaders still come from memory, the MMU enable is still the secure-AHB sequence |
| limits: instructions 512, temps 64, constants 320, varyings 16, pixel pipes 1 | same (`nxp :18849-18876`) | same (`st :2868-2895`) | `src/vivante/state.rs:12-24` and `src/vivante/regs.rs:909` (`GC_NUM_CONSTANTS` 320) hold |
| shader cores | 2 | 2; **1** in the 0x8000/0x6207 row (`:2870`) | the count reaches our identity struct (`driver/rust/src/gpu/vivante/hw.rs:184`, `mod.rs:107`) but no GPU path branches on it; the row's thread count is 0x200 in all four candidates |
| streams (`gcFEATURE_VALUE_Streams`) | Mesa 16 (`nxp :18875`, `:19769`); **our kernel table says 8** (`driver/rust/src/gpu/vivante/hw.rs:74`, from the informal `gc7000UL_6200` row, `driver/docs/vivante-kernel-rust.md:93`; Linux's own copy of that row reads Streams 8 too, `etnaviv_hwdb.c:78`, and its 0x6204 and 0x6205 rows read 16, `:142`, `:205`) | 16; **8** in the 0x6207 row (`st :2894`) | `GC_MAX_VERTEX_ELEMENTS` is 16 *elements*, not streams (`src/vivante/state.rs:14`). The stream plumbing is `GcElem.stream` (per-element index, `state.rs:28-30`), the array `streams: [GcStream; GC_MAX_VERTEX_ELEMENTS]` (`state.rs:74`, `GcStream` at `state.rs:48-53`) and the per-stream registers `nfe_vertex_streams_base_addr` / `_control` / `_vertex_divisor` (`src/vivante/regs.rs:152-161`, written in `emit_streams`, `src/vivante/backend.rs:1297-1304`). Mesa turns the row value into `info->gpu.stream_count` (`etna_hwdb.c:125`). Our GL core reports `MAX_VERTEX_ATTRIBS` 16, which its own note calls "the chip's own `stream_count`" (`src/vivante/VIVANTE_NOTES.md:1044-1047`); the kernel table's 8 (`hw.rs:74`) reaches the identity struct (`:181`) but "nothing in the driver reads it" (`driver/docs/vivante-kernel-rust.md:144`). So: a 16-stream row keeps today's `MAX_VERTEX_ATTRIBS`; an eight-stream row (the 0x6207 candidate, and the i.MX8MP's own galcore-style DB) must become table data and be re-queried (item 5.3, Q15) |
| `SH_NO_ONECONST_LIMIT` | 0 (`nxp :19246`) | **1** (`st :3269`, `:4200`, `:5131`, `:6062`) | Mesa then lets one instruction read two different uniforms (`etnaviv_asm.c:62-71`, `etnaviv_compiler_nir.c:851`, `etnaviv_screen.c:893-894`); the i.MX8MP does not. Our compiler's output obeys the stricter rule (it runs on the i.MX8MP), so it stays valid. Relaxing it for this core is an optimisation, not a port item |
| `WIDELINE_TRIANGLE_EMU` | 0 (`nxp :19297`) | **1** (`st :3320`, `:4251`, `:5182`, `:6113`) | Mesa reports a maximum line width of 1.0 instead of 8192 (`etnaviv_screen.c:366-368`). We program the wide-line registers (`GC_PA_CONFIG_WIDE_LINE` and `GC_PA_WIDE_LINE_WIDTH0`/`WIDTH1`, `src/vivante/backend.rs:1376`, `:1425-1426`; the names `src/vivante/regs.rs:291-293`), and the GL core reports `GL_ALIASED_LINE_WIDTH_RANGE` as 1.0 to 16.0 (`src/gl/state.rs:993-997`): wide lines on an emulating core need a different path or a reported range that fits it (Q11) |
| `gcFEATURE_VALUE_NNCoreCount` and the NN/TP/VIP fields | 0 | 2 | Mesa marks the core `CORE_NPU` too (`etna_hwdb.c:39-40`) and maps `VIP_V7` (`st :3466`, `:4397`, `:5328`, `:6259`) and `NN_XYDP0` (the 0x6207 row only, `:3500`, 1; `:4431`, `:5362`, `:6293`, 0). No effect on the GPU path; check `etna_shc` and the shim with such a row (item 5.2) |
| `SH_NO_INDEX_CONST_ON_A0` | 0 (`nxp :19245`) | **1** (`st :3268` and the other three) | Mesa does not read this bit (no `ETNA_FEATURE` for it in `etna_hwdb.c`). By its name, constants cannot be read through the address register. Since 2026-10-04 no table row indexes a uniform array through `movar` at all (`tools/vivante/README.md:67-78`: the array read became a UBO 0 load; "reads element 0 on the i.MX8MP" is what the old form did), so this agrees; an inference, Q12 |
| `SH_MOVAI_MOVAR_UNUSED_COMPONENTS_WRITE_DIRTY_DATA_FIX` | 1 (`nxp :19645`) | **0** (`st :3691` and the other three) | By its name, the fix for partial-component address-register writes is absent. Whether our compiler ever emits one is unchecked (Q12) |
| `REG_Evis`, `SH_TEXLD_U_FIX`, `TX_BORDER_CLAMP_FIX`, `FE_DRAW_DIRECT`, `MULTIVIEW_RENDER`, `PE_ADVANCE_BLEND_PART0`, `TX_UNNORMALIZED_COORD` and other fix bits | 0 (`nxp :19173-19317`) | 1 (`st :3196-3340` and the other rows) | more bug fixes or extensions in hardware; Mesa maps none of them (no `ETNA_FEATURE` for any of the seven in `etna_hwdb.c`). No port work, but a candidate for changed behaviour on a conformance case, so the first CTS run is diffed against the i.MX8MP's list |
| `ChipID`, `ChipRevision`, `ProductID`, `CustomerID`, `EcoID` | 0x7000, 0x6204, 0x70003, 0x11, 0 | see the table in section 1 | our kernel side refuses it, below |

The Mesa users that make these bits matter to a Mesa-derived design are
`etnaviv_screen.c:366-368` (line width), `:841` (unified samplers),
`:859-870` (`CORE_NPU`, `VIP_V7`, `NN_XYDP0`), `:893-894`
(`has_no_oneconst_limit`), `etna_hwdb.c:39-40`, `:104-117`, `:125`,
`etnaviv_asm.c:62-71` and `etnaviv_compiler_nir.c:851`. The bits Mesa does
not map are listed because they are in the row, not because anything is
known about them.

### Where our code is pinned to the i.MX8MP

- **Identity.** `driver/rust/src/gpu/vivante/hw.rs` carries one table row:
  `chip_id: 0x7000`, `revision_mask: 0xfff0`, `revision: 0x6200`,
  `product_id: 0x70003`, `board_revision: 0x6204` (`:63-67`); the row's own
  comment says other revisions are "refused as 'not in the database'"
  (`:42-43`). `driver/docs/vivante-kernel-rust.md` ("Identity: what
  changed", `:146-158`) states the rule and its escape: the table matches
  on model, masked revision and product "and also requires revision
  0x6204, the one revision the init has run on. Model 0, all ones, or the
  absent pattern is -ENODEV. Another part is identified from its registers
  alone, with a warning". An absent or non-answering block is -ENODEV in
  the code too (`hw.rs:108-126`). The row's `streams: 0x8` (`:74`) is
  handed to `id.stream_count` (`:181`).
  The table becomes data keyed on all five ids, with the limits of the row.
- **Constants** in `src/vivante/state.rs`, `regs.rs` and `backend.rs`
  (`GC_NUM_CONSTANTS` 320 at `regs.rs:909`, `NON_POWER_OF_TWO` with its
  comment citing the NXP rows at `backend.rs:762-771`, `GLES_VS_TEX_UNITS`
  in `src/backend.rs:665`, the renderer string). The limits match; the
  comments cite the NXP rows and need the ST row beside them.
- **Power.** `driver/rust/src/gpu/vivante/power.rs` is built over
  `soc::imx8mp::ccm` and `soc::imx8mp::gpc` (`:39-40`). The MP25 side is a
  different module (SCMI), section 4.
- **Reference tables.** `tests/reference/vivante_blobs.rs` is Mesa's
  reference for the i.MX8MP's ids: `SHC_BLOB_TABLE` has 234 rows
  (`:5877`), compiled by `etna_shc` from the `dir`-role `gc7000` sources of
  `tools/shaders/precompiled.list` (`:101-105`; `tools/vivante/README.md`).
  A different ISA flag (`has_no_oneconst_limit`) can change instruction
  words, so an ST table is its own artefact (item 5.2); the product
  compiles with our own compiler either way.
- **A runtime or a build choice.** The GL core selects the back end by
  Cargo feature (`gpu-vivante`, `Cargo.toml:88`). A chip profile (limits,
  line width, quirk flags) chosen at run time from the identity is the
  smaller change than a second feature, because the limits agree; this is a
  design decision for the first item, not made here.

## 4. The board on Zephyr

**Does Zephyr have this SoC and board?** Partly. Zephyr 4.4.99 has the SoC
series `stm32mp2x` (`soc/st/stm32/stm32mp2x/Kconfig.soc`:
`SOC_STM32MP215FXX_M33` `:16`, `SOC_STM32MP257FXX_M33` `:20`) and the
boards `stm32mp257f_dk` and `stm32mp257f_ev1` under `boards/st/`, built as
`stm32mp257f_dk/stm32mp257fxx/m33` (board document `:162`). **Only the
Cortex-M33 is a target:** the board document says of the A35 "Not yet
supported in Zephyr" (`index.rst:124-127`), and `dts/arm64/` has no `st`
directory. The M33's device tree (`dts/arm/st/mp2/stm32mp2_m33.dtsi` and
the per-SoC files it chains through, `stm32mp257_m33.dtsi` `:7` including
`stm32mp255`, `stm32mp253`, `stm32mp251`) has the CPU as `arm,cortex-m33`
(`stm32mp2_m33.dtsi:24-28`), an RCC clock controller `st,stm32mp2-rcc` at
0x44200000 with a `st,stm32-rcc-rctl` reset controller (`:42-49`) — for
the M33's peripherals, no GPU, display, LVDS, GIC or SCMI node — and two
8 MiB DDR windows, `memory0@80100000` and `memory1@80a00000`
(`stm32mp2_m33.dtsi:31-39`, sized 8 MiB in `stm32mp251_m33.dtsi:211-219`
through the include chain). The M33 is started by Linux through remoteproc
(board document `:139-145`). The M33 is not our target (the three finished
ports run on application cores, and a GPU clock, a framebuffer and a
carveout need more than 8 MiB windows). So:

- **A35 SoC and board definition** are new and ours, as the i.MX8MP's A53
  was (`imx8mp-gc-bringup.md` step 3: Zephyr ships `verdin_imx8mp` for the
  Cortex-M7 only, so the A53 became a board extension in the platform
  repository) and the AM62P's (`driver/boards/toradex/verdin_am62p`,
  `driver/soc/ti`). Needed: an arm64 SoC (the board document calls the
  A35 "64-bit dual-core Arm® Cortex®-A35", `index.rst:60`; Zephyr's
  `dts/arm64/armv8-a.dtsi` exists), the interrupt controller, the arch
  timer, the console UART, and the PSCI or secure-monitor interface the
  firmware provides. All Q13.
- **Clocks, resets, power domains.** Zephyr has an SCMI subsystem
  (`drivers/firmware/scmi`: `clk.c`, `reset.c`, `power.c`, `smc.c`,
  `shmem.c`, among others) with the bindings `arm,scmi`, `arm,scmi-smc`,
  `arm,scmi-shmem`, `arm,scmi-clock`, `arm,scmi-reset`, `arm,scmi-power`
  (`dts/bindings/firmware/`). Mainline's own model of the MP257's clocks is
  a mix: a plain `&rcc` `clock-controller@44200000` — the block Zephyr's
  M33 trees already name (above) — gates peripheral clocks such as
  `CK_BUS_GPU` 91 and `CK_KER_GPU` 340
  (`include/dt-bindings/clock/st,stm32mp25-rcc.h:114`, `:362`), while that
  controller's own input clocks are SCMI clocks (`stm32mp251.dtsi:1900` and
  following, among them `CK_SCMI_ICN_VID` at `:1913`), so gates and parents
  are separate problems. If OP-TEE's server is the only way to enable
  the GPU clock, our image is an SCMI agent over SMC: it needs the SMC
  function id, the shared-memory channel for a non-secure agent, and the
  clock and reset ids of the GPU. All Q3. This replaces `power.rs`'s CCM
  and GPC programming; the brief's OP-TEE panic is the first thing to
  reproduce or rule out with our own request (Q3).
- **Memory map, interrupt, security.** The GPU's base address and size, its
  interrupt line, the free RAM for the carveout and the display buffers,
  and whether the GPU and the display belong to a resource-isolation
  compartment the non-secure A35 may use: Q4, Q5, Q6, Q14. The i.MX8MP port
  needed the same facts and found several by measurement
  (`imx8mp-gc-driver-notes.md`, the numbered list of what only the board
  can answer). Mainline's ST trees show the carveout idea already exists:
  the DK tree reserves a 64-MiB `no-map` shared-dma-pool for firmware
  (`stm32mp257f-dk.dts:139-143`), and the DHCoM SoM tree (shared by the
  MP23x and MP25x variants) reserves 32 MiB for OP-TEE and 64 MiB named
  `gpu-reserved` (`stm32mp2xxx-dhcos-som.dtsi:178-186`); neither is a GPU
  register window, and the GPU's own base stays Q5.
- **Display.** A Zephyr driver for the MP2's display controller and its
  LVDS or HDMI bridge does not exist (above); the i.MX8MP's display was a
  driver of ours (`driver/docs/imx8mp-display-cold.md`). Q7.

**Boot route for sign-off.** The finished ports are signed off from the
board's own bootloader, never kexec (`imx8mp-netboot.md`, goal;
`docs/standalone-boot.md`). For the DK that is U-Boot (ST's, from the
OpenSTLinux boot media) loading our image over the network or from the boot
media, with OP-TEE staying resident so the SCMI server answers (the Mali
port ran its SCMI clock request from U-Boot, `driver/docs/nanopc-t6-mali.md`
"The GPU clock without Linux"). A jump from Linux (kexec) would be
development only. A direct route that replaces U-Boot proper, as
`notes/am62p-direct-boot.md` and `notes/imx8mp-direct-boot.md` did with the
second-stage loader `driver/samples/zephyr/boot_stage2`, is a later item
and needs the ST chain read first (Q8). Whether the image may start at the
exception level and with the state U-Boot leaves, and whether U-Boot leaves
the display on, are Q8 and Q13.

## 5. What can be done before the board arrives

Each item has a check. All are host-side except where Zephyr is built.

1. **Pin the hwdb comparison.** A small script under `tools/vivante` that
   reads the two Mesa headers and prints the comparison of section 3 (it
   would replace the throw-away greps of this pass). Check: its output
   equals the table in this note; a gate row like the existing `tools` rows
   (`tools/check.sh:2472` "tools   shader-table generator guards"). Needs a
   Mesa tree, so it is a gap, not a pass, where the tree is missing.
2. **A shim entry per candidate row, and the reference tables.** Add the
   four ST rows' ids to the etnaviv drm-shim next to the `GC7000UL` entry
   that `tools/vivante/mesa-cs-golden.patch` adds (the entry carries the
   five ids and no feature words — `.hwdb_ids = true`, patch lines `:61`,
   `:71-77` — so the driver looks the row up in the hwdb, over a DRM 1.4
   interface). Then, for each row: (a) `cs_golden` produces the golden
   command stream for the gl_test shapes; diff it against the i.MX8MP's
   golden (`tools/vivante/README.md`, "The golden command stream"). Check:
   the diff is exactly the expected one (line-width state, no
   `SH_NO_ONECONST_LIMIT` effect on the stream, nothing else); anything
   else is a finding. (b) `etna_shc` with the ST ids compiles the whole set
   the GC7000 table takes — today the committed reference is
   `tests/reference/vivante_blobs.rs`, `SHC_BLOB_TABLE` 234 rows (`:5877`),
   built from the `dir`-role `gc7000` lines of
   `tools/shaders/precompiled.list` (`:101-105`) — and the table generator
   can write an ST table. Check: how many instruction words differ from the
   i.MX8MP's table; that count is the size of the `ONECONST` optimisation
   and proves our i.MX8MP-valid output is also valid on ST. (c) That
   `etna_shc` and the shim run at all with a row that is also `CORE_NPU`
   (`etnaviv_screen.c:859-870`). The method is the Mali port's: Mesa's own
   driver on the host drm-shim as the reference (`docs/notes/mali-*.md`,
   `tools/mali/drm_shim_ref.sh`).
3. **Table-driven identity in the kernel side.** Replace the single i.MX8MP
   row in `driver/rust/src/gpu/vivante/hw.rs` (`:62-80`) by rows keyed on
   the five ids, with the four ST rows (and a fallback that still
   identifies from registers and warns). Check: the existing model-bus tests
   (`driver/rust/src/gpu/vivante/tests.rs`) cover each row; the i.MX8MP's
   replay (`the_c_scenario_comes_out_identical`, `:167`) stays
   byte-identical. This is `cargo test` in the gate, no board.
4. **The chip profile in `src/vivante`.** Limits and quirk flags from the
   row, line-width range from `WIDELINE_TRIANGLE_EMU`. Check: the host-test
   configurations `host-test,gpu-vivante[,shaderc-online]` pass unchanged
   for the i.MX8MP profile; a new test pins the ST profile;
   `tests/es3_caps.rs` gets the profile (grep `YakoglBackend {` in `tests/`
   and `src/` first, as CLAUDE.md says, for any new table field).
5. **The Zephyr A35 SoC and board, built not run.** An arm64 SoC and a
   `stm32mp257f_dk` A35 board with a sample that runs the identify step
   (`yd_gc_identify`, `driver/rust/src/ffi/vivante.rs:69`), built by the
   gate's Zephyr rows. Check: the build exits 0, image size and the linked
   entry points are logged. Needs `ZEPHYR_WS`; without it the row is a gap.
6. **An SCMI client with a model.** Use Zephyr's SCMI subsystem (section 4)
   or a small Rust client over the model bus (`driver/rust/src/model.rs`)
   and test the message bytes of "clock enable" and "reset deassert"
   against the SCMI specification (Arm DEN0056). Check: byte-exact host
   tests. The agent id, channel and ids are filled in when Q3 is answered.
7. **Read the remaining unread sources** and update this note: RM0457
   (GPU, RCC, RIFSC, LTDC/LVDS chapters; URL in `index.rst:211-212`),
   UM3385, the MB1605 schematic, ST's wiki pages on the MP25 GPU and on
   the SCMI server, and ST's Starter Package sources — its own kernel
   device tree, its `etnaviv` fork, its U-Boot, and the OP-TEE and TF-A
   sources of its SCMI server and the PLL init. Linux mainline was read
   in this pass at the pinned clone: it answered the interrupt controller
   partly (Q6), the display controller and the DK panel chain (Q7), the
   SCMI transport partly (Q3), the hwdb half of Q9, and the candidate-row
   cross-check in section 1. It also showed mainline cannot drive this
   GPU today: no ST tree in mainline carries a gpu node. Check: each
   question in section 7 gets a citation or stays an unknown.
8. **A bring-up order record.** The i.MX8MP's record names the order: the
   samples `gc_fence`, `gc_clear` and `gc_triangle` "check each layer with
   pixel read-back ... Run them in that order after `gc_identify`"
   (`imx8mp-gc-bringup.md`, step 4, `:131-135`); that C harness is deleted
   (same file, `:5`), and what carries the steps today is the Rust driver's
   FFI
   (`driver/rust/src/ffi/vivante.rs`: `yd_gc_identify`, `yd_gc_reset`,
   `yd_gc_hw_init`, `yd_gc_mmu_*`, `yd_gc_fe_*`) with the model-bus tests
   of item 5.3. Check: the item 5.5 sample builds for the new board and
   calls those exports.

## 6. What needs the board

First three things, in order:

1. **Under ST's Linux, read, do not drive.** ST's Starter Package Linux,
   not mainline: mainline's ST device trees carry no gpu node (section 1).
   The GPU's identity as etnaviv
   prints it (model, revision, product, eco, customer: the row, Q1;
   `etnaviv_gpu.c:430-431`), the
   device tree node of the GPU (base, interrupt, clocks, resets, power
   domains, IOMMU), `/proc/iomem` and `/proc/interrupts`, and the free RAM
   for a carveout. This settles Q1, Q3 (ids), Q5, Q6, Q9 in one sitting.
2. **From U-Boot, power the GPU through SCMI and read the identity** with
   the Zephyr image (the `yd_gc_identify` sample of item 5.5). Alive if
   `CHIP_MODEL` and `CHIP_REV` match one of the four rows, `HI_INTR_ENBL`
   (0x014, `driver/rust/src/gpu/vivante/regs.rs:53`) reads back the
   0xffffffff written to it (the i.MX8MP instrument,
   `imx8mp-gc-driver-notes.md:98`; the Rust hw init writes all bits and
   reads back, `driver/rust/src/gpu/vivante/hw.rs:379-383`), and the reset
   reaches idle. Do not read the 0x800+ state space from the CPU: two
   builds that did rebooted the i.MX8MP board
   (`driver/docs/vivante-kernel-rust.md`, "Hardware findings that still
   hold"); assume the same here until shown otherwise.
3. **The old order, `gc_fence`, `gc_clear`, `gc_triangle`**, then
   `gc_gl_triangle`: the recorded order of the i.MX8MP work
   (`imx8mp-gc-bringup.md:131-135`, `driver/docs/vivante-kernel-rust.md:125`).
   Its C harness is deleted (item 5.8), so the order is what our new
   samples must re-establish.

What says the port is alive: the identity matches a row; a fence retires
and its interrupt arrives; the first clear reads back through the resolve
engine; the first triangle reads back (the i.MX8MP's three-apex read-back
check, `imx8mp-gc-bringup.md:233-238`); the first
`gl_test` cases pass (case 1 is "fragment uniform",
`driver/tools/pvrlab/gl_test.c:1449`); then the regression set against the
i.MX8MP's list.

## 7. Risks and unknowns

An unknown stays an unknown. Each is a question, with the source that would
answer it.

| id | question | answered by |
| --- | --- | --- |
| Q1 | Which row is the MP25's GPU: model 0x7000 or 0x8000, revision 0x6204, 0x6205 or 0x6207, which eco and customer id? | the chip's own identity registers (board day one); Linux reads the same five and looks them up in its own hwdb, which carries three of the four candidate ids (section 1; `etnaviv_gpu.c:372-384`, `etnaviv_hwdb.c:297-327`); RM0457 GPU chapter |
| Q2 | Why do all candidate rows carry NPU fields? Is the NPU a separate block and the row naming shared? | RM0457 (NPU and GPU chapters); the ST wiki |
| Q3 | How is the GPU turned on: which ids, which rate, and does OP-TEE's PLL init panic on our request as it did when etnaviv loaded (the brief)? | partly mainline now: the SCMI transport is an SMC to OP-TEE (`stm32mp251.dtsi:64-66`), SCMI on OP-TEE channel 0 (`:71-75`), the clock / reset / voltage-domain protocols at 0x14 / 0x16 / 0x17 (`:77-88`); mainline's clock map names `CK_BUS_GPU` 91 and `CK_KER_GPU` 340 (`include/dt-bindings/clock/st,stm32mp25-rcc.h:114`, `:362`), gates on the `&rcc` `clock-controller@44200000` (`stm32mp251.dtsi:1895`) whose own input clocks are SCMI clocks (`:1900` and following, `CK_SCMI_ICN_VID` at `:1913`); open: the SMC function id, which of gate, rate and reset a non-secure agent may ask OP-TEE for, and the panic, which needs OP-TEE and TF-A sources (not read) and board day one |
| Q4 | Is the GPU (and its DMA master) assigned to a resource-isolation compartment that a non-secure A35 image may use, or does the secure side have to set it? Does anything else have to be programmed by the secure side first? | RM0457 (RIFSC, security chapters); OP-TEE `stm32mp2` platform code (not read); board day one. Mainline models the mechanism: a `rifsc` bus at 0x42080000 (`stm32mp251.dtsi:301`), and peripherals carrying `access-controllers = <&rifsc n>` — the LVDS bridge is 84 (`stm32mp255.dtsi:22`), the video decoder 89 (`:32`); the GPU's compartment is unknown because mainline has no gpu node |
| Q5 | The GPU's base address and size, interrupt number, and power domain, if any. | RM0457 memory map; ST's Starter Package device tree — mainline's ST arm64 trees carry no gpu node (checked over `arch/arm64/boot/dts/st/` at the pinned clone) |
| Q6 | Interrupt controller version, how the GPU's interrupt is routed to the A35, and the arch timer rate. | partly mainline now: GIC-400 at 0x4ac00000 (`stm32mp251.dtsi:123-124`); the GPU's irq routing and the timer rate from RM0457 and ST's tree; Zephyr (no A35 node exists) |
| Q7 | The display path: controller, number of layers, formats and stride alignment it scans out, LVDS and the HDMI chain on the DK (a bridge and its control bus), the backlight. | partly mainline now: the LTDC at `display-controller@48010000` (`stm32mp251.dtsi:1617`), the LVDS bridge at 0x48060000 (`stm32mp255.dtsi:15`) and the DK's LTDC → LVDS → panel wiring (`stm32mp257f-dk.dts:98-127`, `:274-299`); the formats, stride alignment, layers and the HDMI chain stay with UM3385, MB1605, ST's Starter Package tree for the DK and RM0457's LTDC and LVDS chapters |
| Q8 | The exact boot chain and what state U-Boot hands over (exception level, MMU, caches, display on or off), and whether the SMC interface to OP-TEE stays open after U-Boot. | ST wiki boot chapter; ST's U-Boot and TF-A; board day one |
| Q9 | Does Linux etnaviv carry the MP25 ids, and does anything in ST's Linux depend on a vendor blob? | answered for mainline at the pinned clone: the hwdb carries a row matching 0x7000/0x6204 (`etnaviv_hwdb.c:136-140`, product and customer wildcards) and one exact row 0x8000/0x6205 (`:199-204`), and none for 0x8000/0x6207; the driver, the hwdb and the identity read are all mainline GPL, and mainline has no gpu node for this board; the vendor-blob half is a Starter Package question (`index.rst:217-218`) |
| Q10 | Is the GPU behind an IOMMU, and what physical address width reaches the GPU? The i.MX8MP port identity-maps a carveout in the GPU's own MMU v2. | ST device tree (iommu property of the GPU); RM0457. For the record: mainline's ST arm64 trees name no iommu at all (checked over `arch/arm64/boot/dts/st/` at the pinned clone) |
| Q11 | What do wide lines do on a `WIDELINE_TRIANGLE_EMU` core when we program the wide-line registers the way the i.MX8MP backend does (`src/vivante/backend.rs:1376`, `:1425-1426`)? What should the build report as the line width range? | the golden stream (item 5.2); board test; Mesa `etnaviv_screen.c:366-368` |
| Q12 | Does our compiler ever use the address-register paths or partial-component address writes that the ST rows' `SH_NO_INDEX_CONST_ON_A0` and missing `SH_MOVAI_MOVAR...FIX` suggest are unavailable or unsafe? | a scan of the online compiler's output (`src/shaderc/online/vivante`) over the item 5.2(b) program set for `movar` and `movai`; the board's gl_test and dEQP runs |
| Q13 | The A35 SoC definition: cache and MMU state at entry, PSCI or the monitor interface, console UART, and whether Zephyr's arm64 core runs on this part without changes. | RM0457; TF-A's stm32mp2 platform; Zephyr `arch/arm64` and `dts/arm64/armv8-a.dtsi` |
| Q14 | The GPU's coherence with the A35 cluster, and the right memory attributes for the carveout. The i.MX8MP uses a non-cacheable mapping and no flushes (`imx8mp-gc-driver-notes.md`, "M3: the ring and one submit"). | RM0457 interconnect chapter; board test |
| Q15 | Does the 0x6207 row (one shader core, eight streams) apply, and if so does our stream use stay within eight: `GcElem.stream` stays under the row's stream count, `emit_streams` (`src/vivante/backend.rs:1297-1304`) writes no `nfe_vertex_streams_*` register past it, and should `MAX_VERTEX_ATTRIBS` drop from 16 to the row's eight (`src/vivante/VIVANTE_NOTES.md:1044-1047`)? | Q1; the model-bus test of item 5.3 for an eight-stream row; Mesa's reachability of 13 vertex inputs (`etnaviv_compiler_nir.c:1157`, `VIVANTE_NOTES.md:1039-1047`); Linux's hwdb streams value per row (`etnaviv_hwdb.c:78`, `:142`, `:205`) |

Risks that are not questions: the finished ports took the board work
(power, display, boot) as the long part, and the GPU work as the short one;
this port adds SCMI as a new dependency on a secure-side component we do
not own. The OP-TEE panic the brief names is a reason to put the SCMI
request first on the day-one list and to log every message.

## 8. Proposed item list

Sized against the i.MX8MP port, the closest of the three: first power-up to
the first triangle on the board took from 2026-09-16 (start,
`imx8mp-gc-bringup.md:12`) to 2026-09-20 (steps 3 and 4 written, M1 to M5
run on the board, `imx8mp-gc-bringup.md:5`, `:150`); the three boards'
dEQP-GLES2 runs with zero failures were in by 2026-10-16 and the i.MX8MP's
common set rows by 2026-10-20 (`tools/cts/STATUS.md`, "ひとことで"). "S"
is a part of a milestone, "M" about one i.MX8MP milestone, "L" several.
This port starts with the GPU code already written, so the GPU-side items
are smaller than they were then.

| # | item | check | size |
| --- | --- | --- | --- |
| P1 | Read the unread documents (section 5, item 7) and update this note; record the answers to Q1 to Q15 that sources give. Open GitHub issues for what stays open | each question cited or left | S |
| P2 | Pin the hwdb comparison (5.1) and the shim rows with goldens and an ST table (5.2) | gate rows; goldens diffed | M |
| P3 | Table-driven identity (5.3) and the chip profile (5.4) | host tests, both profiles | M |
| P4 | A35 SoC and board in Zephyr, identify sample built (5.5) | Zephyr build row | M |
| P5 | The SCMI client and the ST power module with model tests (5.6) | byte-exact host tests | M |
| P6 | Board day one (section 6, items 1 and 2): identity under Linux, power and identity from U-Boot | the five ids read; `HI_INTR_ENBL` read-back | S, board |
| P7 | `gc_fence`, `gc_clear`, `gc_triangle`, `gc_gl_triangle` | read-backs | M, board |
| P8 | The display: controller, LVDS or HDMI bridge, backlight; scan-out of the GPU's buffer | picture on the panel, a record like `imx8mp-display-cold.md` | L |
| P9 | `gl_test` regression set, then the dEQP-GLES2 sweep, diffed against the i.MX8MP's failures; the line-width decision (Q11) | RESULTS rows | L, board |
| P10 | Netboot or direct boot from the board's bootloader (`imx8mp-netboot.md` shape), sign-off from the bootloader | a run from U-Boot; `tools/board.sh` | M |

P2 to P5 can run before the board arrives; P1 only needs documents.
