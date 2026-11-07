<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: STM32MP257F-DK (VeriSilicon Vivante GPU) before the board arrives

Date 2026-11-06, main `1673798c`. Fourth pass (third pass below). The first two passes (`720c8224`
and the second, main `6a9fe4c0`, both 2026-10-28) were host research before
the board arrived; the board is on the bench now. This pass applies the
day-one readings taken by hand under ST's Linux on 2026-11-05 and 2026-11-06
(issue #1281; cited below as "#1281") and reads ST's own kernel device tree
for the DK (section "What was read", row *ST Linux*). The fourth pass folds
in the read-only studies of plan #322 that finished since: OP-TEE's and
TF-A's stm32mp2 code, ST's U-Boot, a second read of the board by hand
("board-read, 2026-11-06"), the galcore power-up order, the RCC and RIFSC
offsets, the Zephyr arm64 entry state, the GPU memory, and Q11 and Q12.
No file under `src/`, `driver/` or `tools/` changed, no image ran on the
board. Where a finding is cited to a study without a line, the file and line
are in that study's report; the open sources are cited by tag.
Repository lines cited below were read against main `6a9fe4c0` (pass two) and
are marked where this pass re-read them. Written for the port that would
follow the i.MX8MP's (`imx8mp-gc-bringup.md`, `imx8mp-gc-driver-notes.md`,
`driver/docs/vivante-kernel-rust.md`).

## Verdict in six lines

1. The GPU is the same family as the i.MX8MP's GC7000UL: every ST hardware
   database row Mesa carries for a model 0x7000 or 0x8000 NanoUltra core has
   HALTI5, unified samplers, the secure AHB, no BLT engine, and the same
   limits (512 instructions, 64 temporaries, 320 constants, 16 varyings, one
   pixel pipe). **`src/vivante` extends; no new back end.** (Section 3.)
2. The MP25's GPU is the third candidate row, **model 0x8000, revision
   0x6205, product 0x80003, eco 0** (read under ST's Linux, #1281); Linux's etnaviv hwdb carries exactly that id. The
   customer id was not read (the row's is 0x15). The other three rows matter
   only as far as a table-driven identity costs nothing. (Section 1, Q1.)
3. The register-level and ISA differences from the i.MX8MP's row that Mesa
   reads are three: `SH_NO_ONECONST_LIMIT` (an instruction may read two
   different uniforms), `WIDELINE_TRIANGLE_EMU` (Mesa caps the line width at
   1.0), and NPU fields in the row. The golden stream against the i.MX8MP's
   (`tools/vivante/README.md`, "Another part: the ST row" (commit `3dd64152`)) adds one that is not a feature word: Mesa
   turns tile-status compression off on this row (section 5.2). The 0x8000/0x6205 row has two shader
   cores and 16 streams like the i.MX8MP's Mesa row (the eight-stream row
   was the 0x6207 one, which does not apply). (Section 3.)
4. What is new is everything around the GPU: the power (an SCMI performance
   domain and an SCMI power domain served by OP-TEE, plus plain RCC gates
   and a reset, #1281), the display (LTDC to DSI to an ADV7535 to HDMI, not
   LVDS, #1281), the Zephyr A35 SoC and board (Zephyr has only the M33), and
   the identity table in the kernel side, which today refuses any part but
   the i.MX8MP's. (Sections 4 and 5.)
5. Most of the GPU work can be done on the host before the board arrives,
   with the drm-shim method the i.MX8MP and Mali work already use.
   (Section 5.)
6. The first board day (identity under Linux) is done (#1281). What is next
   is power-on through SCMI from our own image, then the identity from the
   chip, then the three Vivante samples in their old order. (Section 6.)

## What was read, and what was not

Read for this pass, all on this host:

| short name | what | version |
| --- | --- | --- |
| Mesa | `src/etnaviv/` (hwdb, hw headers, isa, drm, ci) and `src/gallium/drivers/etnaviv/` | 26.2.1 (`VERSION` 26.2.1; the tag the Vivante tools name: `tools/vivante/README.md:5`, `src/vivante/backend.rs:765-767`) |
| Linux | `drivers/gpu/drm/etnaviv/`, `arch/arm64/boot/dts/st/`, `include/dt-bindings/clock/st,stm32mp25-rcc.h`, `Documentation/devicetree/bindings/gpu/vivante,gc.yaml` | master `65ba8f34ec886ec72b2c07ddc8cf3d815c32869d` (dated 2026-10-27), a depth-1 clone fetched 2026-10-28 |
| Zephyr | `boards/st/stm32mp257f_dk`, `soc/st/stm32/stm32mp2x`, `dts/arm/st/mp2`, `dts/bindings/firmware`, `drivers/firmware/scmi`, `drivers/display` | 4.4.99 (`VERSION`, the lab workspace `ZEPHYR_WS`) |
| ST Linux | `arch/arm64/boot/dts/st/` (`stm32mp257f-dk.dts`, `stm32mp257f-dk-resmem.dtsi`, `stm32mp251.dtsi`, `stm32mp255.dtsi`, `stm32mp25-pinctrl.dtsi`) and `include/dt-bindings/{clock,reset,power}/st,stm32mp25-*.h` | ST's `STMicroelectronics/linux`, branch `v6.6-stm32mp` (Linux 6.6.129, `Makefile:1-3`), commit `548f960c059bc4b9165cb69895fd67551f6061ca`, a depth-1 clone fetched 2026-11-06; GPL, cited as facts only. The OpenSTLinux `scarthgap` release tag was not looked up; the branch head is what was read |
| OP-TEE, TF-A | OP-TEE `stm32mp2` platform, its SCMI server and SMC watchdog service; TF-A `plat/st/stm32mp2` (PSCI, BL2) | tags `4.0.0-r3.1` and `v2.10-r3.1`, the newest for the versions the board prints; the board's exact release is unknown. The `stm32mp257f-dk` `ca35tdcid` configuration and TF-A's defaults are taken to describe the board's plain variant. Both are open source; cited by tag |
| ST U-Boot | the DK's boot flow: `boot.scr.uimg`, the distro loop, extlinux, the environment, `booti` | ST's U-Boot 2023.10; GPL, cited as facts only |
| galcore | the platform code for the STM32MP25: power-up order, clocks, reset, memory | galcore 6.4.21 (GPL, facts only) |
| RCC, RIFSC offsets | cross-checked in TF-A, OP-TEE, Zephyr, U-Boot and Linux | the trees named above |
| ours | `src/vivante`, `src/shaderc/online/vivante`, `driver/rust/src/gpu/vivante`, `driver/rust/src/ffi/vivante.rs`, `driver/rust/src/model.rs`, `tools/vivante`, `tools/shaders`, `tests/`, the notes named in each section | main `6a9fe4c0` |

The Mesa hwdb was compared field by field with `grep` over both
`gc_feature_database.h` files and the entry ranges; every value quoted in
sections 1 and 3 is a line in those files, at the line given or greppable
by the quoted name. The repository has no tool called `ETNA_HWDB`
(`git grep ETNA_HWDB` finds nothing); `tools/vivante/etna_shc.c` and
`tools/vivante/mesa-cs-golden.patch` read the same hwdb through Mesa
itself.

**Read late.** The first writing of this note could not reach the network.
A fetch grant arrived later in pass one: Linux mainline was cloned on
2026-10-28 and read at the pinned commit named in the table above. In this
pass ST's kernel tree was cloned and read (table row *ST Linux*), and the
board answered by hand (#1281).

**Not read, and why.** ST's reference manual RM0457, the user manual
UM3385, the board schematic MB1605 and the datasheet (all four with public
URLs, listed in Zephyr's board document at `index.rst:199-212`), the ST
wiki, and the ST community thread about the OP-TEE panic were not read.
RM0457 was tried by the plan's study and could not be fetched (the refusal
comes from st.com; whether the URL is at fault was not confirmed): **every
RCC and RIFSC offset below is unconfirmed from the manual** and rests on the
cross-check of open sources in section 4. The panic path and the OP-TEE
message ABI are folded in at the end of section 4 and in section 7.
Everything that would need one of them is written as a question in section
7 with the source that answers it; a question stays a question, and no
verdict above depends on one. Where this note repeats something the task
brief said (the OP-TEE panic when etnaviv loaded, ST's `scarthgap` release),
it says so; that thread was not re-read.

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
Camera* (`:99-100`; the DK's own device tree drives the HDMI connector
through DSI and an ADV7535, see *Display path* below), and an on-board STLINK-V3EC debugger with a virtual
COM port (`:42-44`, `:102-104`). Under *Security/Safety* it names "Secure
boot, TrustZone peripherals, active tamper, environmental monitors,
display secure layers, hardware accelerators" and a "Complete resource
isolation framework" (`:84-88`). Its link list names the public documents
with URLs: the reference manual RM0457, the UM3385 user manual, the MB1605
C01 schematic and the datasheet (`:195-224`). These are public; none was
read here (not read; see "Not read").

There is no public programming manual for the GPU itself, as for the
i.MX8MP (`imx8mp-gc-driver-notes.md:32`, "Sources, and what 'no register
value without a reference' means here"). The reference manual is expected to give
only the integration (base address, clock, reset, interrupt, security
assignment). Q5 and Q6 are answered from the device tree (#1281, and ST's tree below); Q4 is
answered in part.

### The GPU core: four candidate rows, the third one is the part

The board answered (#1281): **model 0x8000, revision 0x6205, product
0x80003, eco 0**, which is the third row of the table below and the one
Linux's etnaviv hwdb carries exactly. The customer id was not read (the
row's is 0x15; rows 3 and 4 differ only in eco, and eco 0 was read). Plan #322
therefore builds the identity, the shim entry, the golden stream and the ST
shader table for this row; the other three rows are needed only as far as
a table-driven identity costs nothing. The 0x7000/0x6204 row is the
i.MX8MP's neighbour in the same family and the 0x8000/0x6207 row (Q15) does
not apply to this board.

Mesa 26.2.1 carries ST's own database,
`src/etnaviv/hwdb/st/gc_feature_database.h` ("Auto created on 2024-12-23
15:24", `:56`), beside NXP's. Neither the file nor anything else under
`src/etnaviv`, `src/gallium/drivers/etnaviv` or `docs` says which ST part a
row belongs to (a search for `stm32`, `mp25` and `STMicro` found nothing);
the board's identity registers do (#1281), as above.
The file has six rows. Two are not candidates: `gc7000nano_0x4652` (`:996`,
model 0x400) and `gc600L_0x465x` (`:1927`, model 0x600). Four are model
0x7000 or 0x8000 and are the candidates; the ids are the entry lines:

| row (comment line) | model | revision | product | eco | customer | shader cores | streams |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `GC8000NANOULTRA31_vip1` (`:2858`) | 0x8000 (`:2860`) | 0x6207 (`:2861`) | 0xa080007 (`:2862`) | 0 (`:2863`) | 0x19 (`:2864`) | 1 (`:2870`) | 8 (`:2894`) |
| `GCNANOULTRA31_VIP2` (`:3789`) | 0x7000 (`:3791`) | 0x6204 (`:3792`) | 0x70007 (`:3793`) | 0 (`:3794`) | 0x13 (`:3795`) | 2 (`:3801`) | 16 (`:3825`) |
| **`GCNANOULTRA31_VIP2` (`:4720`), the MP25 (#1281)** | 0x8000 (`:4722`) | 0x6205 (`:4723`) | 0x80003 (`:4724`) | 0 (`:4725`) | 0x15 (`:4726`) | 2 (`:4732`) | 16 (`:4756`) |
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
(kernel `:78`; ST's Mesa row 1, the 0x6207 one, which the board's 0x6205
excludes) and every known 0x6204 and 0x6205 row — Mesa's and Linux's —
reads 16 streams. The MP25's row therefore has 16.

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

### Display path (read from the board's device tree, #1281)

The display is **LTDC → DSI → ADV7535 → HDMI**, not the LVDS panel pass one
assumed (#1281). ST's own tree for the DK says the same: the LTDC's output
endpoint goes to the DSI (`stm32mp257f-dk.dts:536-547`), the DSI's second
port goes to an `adi,adv7535` bridge on an I2C bus with four DSI lanes
(`:328-350`, `:428-433`), and the bridge's output goes to the `hdmi-connector`
node (`:129-137`, `:452-460`). The LVDS bridge exists in the SoC tree
(`stm32mp255.dtsi:137-145`, RIFSC entry 84) but the DK board file does not
enable it. The controller addresses are mainline's and ST's alike: an LTDC at
`display-controller@48010000` (mainline `stm32mp251.dtsi:1617`, ST
`stm32mp251.dtsi:2150`) and a DSI at 0x48000000 (ST `stm32mp255.dtsi:123-133`,
RIFSC entry 81). Zephyr's MP2 device trees describe only the M33's peripherals:
there is no display controller, LVDS or DSI node in `dts/arm/st/mp2/*.dtsi`,
and Zephyr has `drivers/display/display_stm32_ltdc.c` for families whose
device tree carries an `st,stm32-ltdc` node (`drivers/display/Kconfig.stm32_ltdc:9`),
not for the MP2. The layers, formats, stride alignment, the DSI host and the
ADV7535's configuration over its control bus remain Q7. No item of plan #322
needs the display.

### Boot chain (read from the board, #1281; sources of the chain unread)

The DK ships ST's OpenSTLinux; the board document says "Zephyr OS is
ported to run on the Cortex®-M33 core, as a coprocessor of the Cortex®-A35
core" (`index.rst:10-11`), that under *System Clock* the Cortex-A35 is
"Not yet supported in Zephyr" (`:124-127`), that the M33 "requires the
Cortex®-A35 to load and start the firmware using remoteproc"
(`:139-145`), and that in the current distribution the A35 loads the M33
image into DDR or SRAM and starts it (`:154-157`).

The board's console prints (#1281): **TF-A v2.10 (BL2, BL31), OP-TEE 4.0,
U-Boot 2023.10 (ST's), then Linux 6.6.129 through extlinux from the
microSD**; PSCI is 1.1 over SMC. OP-TEE stays resident and serves SCMI to
the OS. The boot media is a microSD (the boot pins select it; with no card
the ROM is silent) with the partitions fsbla1, fsbla2, metadata1, metadata2,
fip-a, fip-b, u-boot-env (512 KiB), bootfs (64 MiB ext4, mounted on `/boot`:
`Image.gz`, the device trees, `boot.scr.uimg`, `mmc0_extlinux/`,
`mmc1_extlinux/`), vendorfs, rootfs and userfs (#1281). The eMMC carries ST's
partition layout, whose contents were never examined. The watchdog Linux uses
is `arm,smc-wdt` (OP-TEE's), 32 s; ST's tree has the node with
`arm,smc-id = <0xbc000000>` and the DK file enables it with
`timeout-sec = <32>` (`stm32mp251.dtsi:78-82`, `stm32mp257f-dk.dts:216-219`,
confirming #1281). OP-TEE's and TF-A's code answers the rest (r-optee-tfa, tags above): BL2
starts the IWDG, so the watchdog is already armed and running when U-Boot
hands over; OP-TEE's SMC service serves it, and a hang resets the board in
about 32 s (after an OP-TEE panic at about 27 s). A client calls INIT first,
uses timeouts of 6 s or more and never sends ENABLE with 0. The return trial
therefore needs no arming of its own. The board's U-Boot has no `wdt`
support (board-read, 2026-11-06).

**U-Boot's flow** (r-uboot, board-read, 2026-11-06). The environment lives in
the `u-boot-env` partition as a CRC32, a flag byte and the strings, at offset
0x7e000 in the 512 KiB area (board-read; writing it by hand is possible but
was not tried). U-Boot's `bootcmd` re-runs `env_check`, which saves a default
environment when the stored one is not valid. The distro boot loop tries
extlinux and leaves `boot.scr.uimg` alone while extlinux succeeds; if it
falls through, `boot.scr` runs and does `env save`. Partitions are named by
`mmcblk1pN` or PARTUUID, never by partition label (the eMMC carries the same
labels). A key at "Hit any key to stop autoboot" (a window of about 1 s) or
Ctrl-C at "Enter choice:" stops at a prompt; whether a key in the first
second works was not checked.

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

**Kernel.** Linux `drivers/gpu/drm/etnaviv` is the mainline kernel driver,
GPL, read at the pinned clone named in the read table. It binds only through
a device tree node with compatible `vivante,gc` (`etnaviv_drv.c:33-43`); the
binding requires compatible, `reg`, one interrupt and named clocks, `bus` /
`core` / `shader` / `reg` (`vivante,gc.yaml:16`, `:21`, `:40`, `:52`), and
allows a reset and a power domain (`:46`, `:49`). Identity is read from the
GPU's own registers (`etnaviv_gpu.c:372-376`, `:383-384`) and looked up in
etnaviv's hwdb (`etnaviv_hwdb.c:297-327`, called at `etnaviv_gpu.c:438`); on
a match the row's feature words are used "as they might be wrong"
(`etnaviv_gpu.c:434-437`). Mainline's ST trees carry no gpu node.

**What the board runs (#1281).** ST's image drives the GPU with its
out-of-tree **galcore 6.4.21** and its own GL user space (OpenGL ES 3.1),
not etnaviv. So "the identity as etnaviv prints it" and any etnaviv
reference from the board do not exist; the host references stay Mesa on the
drm-shim (section 5). The kernel has etnaviv as a module that is not loaded;
the brief reports an OP-TEE panic when it loaded, and plan #322 never loads
it. ST's own tree carries the node galcore binds: a `gpu@48280000` with
compatible `vivante,gc` (`stm32mp255.dtsi:169-183`), disabled in the SoC file
and enabled on the DK with a `contiguous-area` property pointing at the
64 MiB reserved area (`stm32mp257f-dk.dts:378-381`). `contiguous-area` and
`throttle,max_state` (`stm32mp255.dtsi:181`) are properties the etnaviv
binding read in pass one does not list.
Per `driver/docs/vivante-kernel-rust.md:16` our kernel side is original Rust
built from permissive sources only; Linux is read for facts and never
copied, and this note keeps that rule.

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
All four candidate rows share the results unless a column says otherwise;
the MP25's is the third (#1281).

| what | i.MX8MP | ST rows | effect on us |
| --- | --- | --- | --- |
| `HALTI5`, `UNIFIED_SAMPLERS`, `INSTRUCTION_CACHE`, `SECURITY_AHB`, no `BLT_ENGINE`, no `DEC400` | same | same | none: the clear is still the resolve engine, shaders still come from memory, the MMU enable is still the secure-AHB sequence |
| limits: instructions 512, temps 64, constants 320, varyings 16, pixel pipes 1 | same (`nxp :18849-18876`) | same (`st :2868-2895`) | `src/vivante/state.rs:12-24` and `src/vivante/regs.rs:909` (`GC_NUM_CONSTANTS` 320) hold |
| shader cores | 2 | 2; **1** in the 0x8000/0x6207 row (`:2870`), which does not apply to this board (#1281) | the count reaches our identity struct (`driver/rust/src/gpu/vivante/hw.rs:184`, `mod.rs:107`) but no GPU path branches on it; the row's thread count is 0x200 in all four candidates |
| streams (`gcFEATURE_VALUE_Streams`) | Mesa 16 (`nxp :18875`, `:19769`); **our kernel table says 8** (`driver/rust/src/gpu/vivante/hw.rs:74`, from the informal `gc7000UL_6200` row, `driver/docs/vivante-kernel-rust.md:93`; Linux's own copy of that row reads Streams 8 too, `etnaviv_hwdb.c:78`, and its 0x6204 and 0x6205 rows read 16, `:142`, `:205`) | 16 in the MP25's row (`st :4756`); **8** in the 0x6207 row (`st :2894`), which does not apply (#1281) | `GC_MAX_VERTEX_ELEMENTS` is 16 *elements*, not streams (`src/vivante/state.rs:14`). The stream plumbing is `GcElem.stream` (per-element index, `state.rs:28-30`), the array `streams: [GcStream; GC_MAX_VERTEX_ELEMENTS]` (`state.rs:74`, `GcStream` at `state.rs:48-53`) and the per-stream registers `nfe_vertex_streams_base_addr` / `_control` / `_vertex_divisor` (`src/vivante/regs.rs:152-161`, written in `emit_streams`, `src/vivante/backend.rs:1297-1304`). Mesa turns the row value into `info->gpu.stream_count` (`etna_hwdb.c:125`). Our GL core reports `MAX_VERTEX_ATTRIBS` 16, which its own note calls "the chip's own `stream_count`" (`src/vivante/VIVANTE_NOTES.md:1044-1047`); the kernel table's 8 (`hw.rs:74`) reaches the identity struct (`:181`) but "nothing in the driver reads it" (`driver/docs/vivante-kernel-rust.md:144`). So: the MP25's 16-stream row keeps today's `MAX_VERTEX_ATTRIBS`. An eight-stream row (the 0x6207 candidate, which the board excluded, and the i.MX8MP's own informal row) would have to become table data; the table-driven identity of item 5.3 carries the stream count per row at no cost, and nothing else is needed (Q15 is dropped) |
| `SH_NO_ONECONST_LIMIT` | 0 (`nxp :19246`) | **1** (`st :3269`, `:4200`, `:5131`, `:6062`) | Mesa then lets one instruction read two different uniforms (`etnaviv_asm.c:62-71`, `etnaviv_compiler_nir.c:851`, `etnaviv_screen.c:893-894`); the i.MX8MP does not. Our compiler's output obeys the stricter rule (it runs on the i.MX8MP), so it stays valid. Relaxing it for this core is an optimisation, not a port item |
| `WIDELINE_TRIANGLE_EMU` | 0 (`nxp :19297`) | **1** (`st :3320`, `:4251`, `:5182`, `:6113`) | Mesa reports a maximum line width of 1.0 instead of 8192 (`etnaviv_screen.c:366-368`). We program the wide-line registers (`GC_PA_CONFIG_WIDE_LINE` and `GC_PA_WIDE_LINE_WIDTH0`/`WIDTH1`, `src/vivante/backend.rs:1376`, `:1425-1426`; the names `src/vivante/regs.rs:291-293`), and the GL core reports `GL_ALIASED_LINE_WIDTH_RANGE` as 1.0 to 16.0 (`src/gl/state.rs:993-997`): wide lines on an emulating core need a different path or a reported range that fits it (Q11) |
| `gcFEATURE_VALUE_NNCoreCount` and the NN/TP/VIP fields | 0 | 2 | Mesa marks the core `CORE_NPU` too (`etna_hwdb.c:39-40`) and maps `VIP_V7` (`st :3466`, `:4397`, `:5328`, `:6259`) and `NN_XYDP0` (the 0x6207 row only, `:3500`, 1; `:4431`, `:5362`, `:6293`, 0). No effect on the GPU path; check `etna_shc` and the shim with such a row (item 5.2) |
| `SH_NO_INDEX_CONST_ON_A0` | 0 (`nxp :19245`) | **1** (`st :3268` and the other three) | Mesa does not read this bit (no `ETNA_FEATURE` for it in `etna_hwdb.c`). By its name, constants cannot be read through the address register. Since 2026-10-04 no table row indexes a uniform array through `movar` at all (`tools/vivante/README.md:67-78`: the array read became a UBO 0 load; "reads element 0 on the i.MX8MP" is what the old form did), so this agrees; an inference, Q12 |
| `SH_MOVAI_MOVAR_UNUSED_COMPONENTS_WRITE_DIRTY_DATA_FIX` | 1 (`nxp :19645`) | **0** (`st :3691` and the other three) | By its name, the fix for partial-component address-register writes is absent. Whether our compiler ever emits one is unchecked (Q12) |
| `REG_Evis`, `SH_TEXLD_U_FIX`, `TX_BORDER_CLAMP_FIX`, `FE_DRAW_DIRECT`, `MULTIVIEW_RENDER`, `PE_ADVANCE_BLEND_PART0`, `TX_UNNORMALIZED_COORD` and other fix bits | 0 (`nxp :19173-19317`) | 1 (`st :3196-3340` and the other rows) | more bug fixes or extensions in hardware; Mesa maps none of them (no `ETNA_FEATURE` for any of the seven in `etna_hwdb.c`). No port work, but a candidate for changed behaviour on a conformance case, so the first CTS run is diffed against the i.MX8MP's list |
| `ChipID`, `ChipRevision`, `ProductID`, `CustomerID`, `EcoID` | 0x7000, 0x6204, 0x70003, 0x11, 0 | 0x8000, 0x6205, 0x80003, 0, customer 0x15 in the row (#1281 read the first four) | our kernel side refuses it, below |

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
  firmware provides. All Q13, answered in part by ST's tree and the studies
  (below): the CPUs are
  `arm,cortex-a35` with `enable-method = "psci"` and the PSCI node is
  `arm,psci-1.0` over `smc` (`stm32mp251.dtsi:21-28`, `:195-198`; the
  board reports PSCI 1.1, #1281); the interrupt controller is a GIC-400
  node at `interrupt-controller@4ac00000` whose register windows are
  0x4ac10000 (distributor), 0x4ac20000, 0x4ac40000 and 0x4ac60000
  (`:166-175`; the node's compatible is `st,stm32mp2-cortex-a7-gic`,
  `arm,cortex-a7-gic`, `:168`); the arch timer lists PPIs 13, 14, 11 and 10
  and has no `clock-frequency` (`:261-268`; its rate stays unread, Q6); the
  DK's console is `usart2` (`stdout-path = "serial0:115200n8"`,
  `stm32mp257f-dk.dts:24`, `:31`) at 0x400e0000, compatible
  `st,stm32h7-uart`, RIFSC entry 32 (`stm32mp251.dtsi:861-873`), TX on PA4
  (AF6) and RX on PA8 (AF8) (`stm32mp25-pinctrl.dtsi:845-856`). The boot-time facts (r-optee-tfa, board-read, r-zephyr-a35; TF-A and
  OP-TEE by tag): the image enters at **EL2** at its load address (from
  `booti` or `go`); the GIC is a **GICv2**, and TF-A keeps the secure SGIs 8
  to 15 and the secure PPI 29, so the board layer leaves SGIs 8 to 15 and
  PPIs 29 and 31 alone; the counter runs at 40 MHz (board-read; `CNTFRQ_EL0`
  from a CPU was not read, so the image prints it); the watchdog is armed
  from BL2 and has to be served from the first tick (32 s). The study of
  Zephyr's arm64 path (its report holds the errata list) finds that the
  GICv2 path needs no change; the SoC definition is still to be built and
  run (item P4).
- **Clocks, resets, power domains.** Zephyr has an SCMI subsystem
  (`drivers/firmware/scmi`: `clk.c`, `reset.c`, `power.c`, `smc.c`,
  `shmem.c`, among others) with the bindings `arm,scmi`, `arm,scmi-smc`,
  `arm,scmi-shmem`, `arm,scmi-clock`, `arm,scmi-reset`, `arm,scmi-power`
  (`dts/bindings/firmware/`). The board's answer (#1281, confirmed by ST's
  tree): SCMI runs **over OP-TEE** (`linaro,scmi-optee`, channel 0:
  `stm32mp251.dtsi:103-108`) with protocols 0x11 (power domain), 0x13
  (performance), 0x14 (clock), 0x16 (reset) and 0x17 (voltage domain)
  (`:110-136`). The GPU node uses the **performance (0x13) and power domain
  (0x11)** protocols, plus the plain `d1_pd` domain: `power-domains =
  <&scmi_perf 1>, <&scmi_devpd PD_SCMI_GPU>, <&d1_pd>` (`stm32mp255.dtsi:176`,
  `PD_SCMI_GPU` is 0, `include/dt-bindings/power/st,stm32mp25-power.h:9`;
  `d1_pd` is the node `power-domain-d1`, compatible `st,stm32mp-pm-domain`, `stm32mp251.dtsi:189-193`).
  Its **clocks and reset are plain `&rcc` entries**, not SCMI:
  `<&rcc CK_BUS_GPU>` (91) as `bus`, `<&rcc CK_KER_GPU>` (340) as `core`, and
  `<&rcc GPU_R>` (75) as the reset (`stm32mp255.dtsi:173-175`,
  `include/dt-bindings/clock/st,stm32mp25-rcc.h:114`, `:362`;
  `include/dt-bindings/reset/st,stm32mp25-rcc.h:85`). So a non-secure
  agent has to ask OP-TEE for the two domains and write the RCC gates and
  the reset itself, as far as RIFSC lets the non-secure side. **The transport is not an
  SMC mailbox with shared memory**: the DT node is `linaro,scmi-optee`
  (channel 0, no SMC id and no `shmem` node for the non-secure side,
  board-read), and OP-TEE's server is reached through its message protocol:
  register a shared-memory buffer, open a session to the SCMI pseudo-TA,
  invoke command 4 with the SCMI message (r-optee-tfa, OP-TEE 4.0.0-r3.1).
  Zephyr's `scmi-smc` driver does not apply as it stands. Galcore's order
  (r-galcore): the performance level is set first, the power domain 0 is
  turned on, then the two GPU clocks are one gate bit and the reset is one
  bit of the same RCC register, `RCC_GPUCFGR` at offset 0x83C, bit 1 the
  clocks gate, bit 0 the reset (cross-checked in the open sources; RM0457
  would confirm and could not be read). Galcore sets no clock rate and no
  performance level of its own, so our image reads the current level before
  it sets one. The `CK_SCMI_PLL3` clock and the PLL3 registers are never
  touched. Every message is logged. Our own request has not been tried on
  the board. This replaces `power.rs`'s CCM and GPC programming.
- **The OP-TEE message ABI our transport must speak** (note-optee, read
  from ST's `optee_os` tag `4.0.0-stm32mp-r3.1`, commit `0063536697f1`; the
  tag the board's OP-TEE 4.0 is nearest to, not checked against the running
  binary). Paths are in that tree.
  - *SMC function ids* (`core/arch/arm/include/sm/optee_smc.h:36-47`, owner
    `OPTEE_SMC_OWNER_TRUSTED_OS` 50, `:55`; owner shift 24): a fast call is
    `0x80000000 | 50 << 24 | n`, a standard call `50 << 24 | n`. So
    `GET_SHM_CONFIG` (n 7, `:230`) is 0xb2000007, `EXCHANGE_CAPABILITIES`
    (n 9, `:323`) 0xb2000009, `CALL_WITH_ARG` (n 4, `:199`, via
    `optee_msg.h:343`) 0x32000004, `CALL_WITH_RPC_ARG` (n 18, `:566`)
    0x32000012, `CALL_WITH_REGD_ARG` (n 19, `:569`) 0x32000013. The
    probes use owner 63: `CALLS_UID` 0xbf00ff01 (`:89-93`), UID
    384fb3e0-e7f8-11e3-af63-0002a5d5c51b (`optee_msg.h:251-254`), revision
    2.0 (`:262-264`). The `NSEC_NOIRQ` variants (n 56 to 58, `:641-659`) are
    for the PTA's OCALL thread and are not needed. Return codes in a0:
    OK 0, ETHREAD_LIMIT 1, EBUSY 2, ERESUME 3, EBADADDR 4, EBADCMD 5,
    ENOMEM 6, ENOTAVAIL 7, unknown function 0xFFFFFFFF (`:814-824`); an
    RPC return has the prefix 0xFFFF0000 (`:686-688`), with the function in
    the low half, and the call must be resumed (`ALLOC`, `FREE`,
    `FOREIGN_INTR`, `CMD`, `:723-787`).
  - *Capability exchange* (`core/arch/arm/tee/entry_fast.c:67-123`): a1 in
    may only be `NSEC_CAP_UNIPROCESSOR` (bit 0), any other bit returns
    ENOTAVAIL (`:85-88`). The reply sets `DYNAMIC_SHM` (bit 2) when the
    non-secure DDR is defined (`:99-101`), `MEMREF_NULL` (bit 4, always),
    `ASYNC_NOTIF` (bit 5; the stm32mp2 platform forces it on,
    `plat-stm32mp2/conf.mk:113`), `PM_NOIRQ` (bit 15, always) and `RPC_ARG`
    (bit 6, always), and a3 = the RPC argument's parameter count
    (`:119-122`); `HAVE_RESERVED_SHM` (bit 0) only if
    `CFG_CORE_RESERVED_SHM`, which the platform leaves `n` (`conf.mk:157`).
    From the source the reply value is therefore **0x8074** (bits 2, 4, 5, 6,
    15; bit 3 is off unless `CFG_NS_VIRTUALIZATION`). That is derived, not
    read from the board: the confirming read is the capability exchange of
    the return trial's image (item 5), which logs a1 to a3.
  - *Message layout* (`core/include/optee_msg.h`): `struct optee_msg_arg`
    (`:204`) is eight 32-bit words, `cmd`, `func`, `session`, `cancel_id`,
    `pad`, `ret`, `ret_origin`, `num_params`, then the parameters; a
    `struct optee_msg_param` (`:176`) is 32 bytes, a 64-bit `attr` and three
    64-bit words (`a`, `b`, `c` for a value; `buf_ptr`, `size`, `shm_ref`
    for temporary memory; `offs`, `size`, `shm_ref` for registered memory).
    The message is 32 + 32 × `num_params` bytes. Commands: OPEN_SESSION 0,
    INVOKE_COMMAND 1, CLOSE_SESSION 2, CANCEL 3, REGISTER_SHM 4,
    UNREGISTER_SHM 5 (`:341-348`); attribute types VALUE_INPUT 1, OUTPUT 2,
    INOUT 3, RMEM_INPUT 5, OUTPUT 6, INOUT 7, TMEM_INPUT 9, OUTPUT 0xa,
    INOUT 0xb (`:21-33`); META is bit 8, NONCONTIG bit 9 (`:43`, `:71`).
    OPEN_SESSION carries the TA UUID in the meta `param[0]` (a, b) and the
    client UUID and login class in the meta `param[1]` (a, b, c)
    (`:295-308`); the session id comes back in `session`.
  - *Shared memory.* The platform has no reserved shared memory, so the
    message and the SCMI buffers are in registered memory: REGISTER_SHM with
    one `TMEM_INPUT` parameter (physical address, size, `shm_ref`; with
    NONCONTIG the address names a page list of 4 KiB pages,
    `optee_msg.h:306-318`, `:97`). How to place the message so it can sit in
    one page (the arg is at most `OPTEE_MSG_MAX_NUM_PARAMS` parameters,
    `:233-236`) and whether OP-TEE wants the RPC-argument copy appended
    (`CALL_WITH_RPC_ARG`, given capability bit 6) are open: the answer is
    the board's reply to our first call, and the OP-TEE source for the
    message-parsing path (`core/arch/arm/kernel/thread.c` family, not read).
  - *The SCMI pseudo-TA* (`core/pta/scmi.c`, `lib/libutee/include/pta_scmi_client.h`):
    UUID a8cfe406-d4f5-4a2e-9f8d-a25dc754c099 (`pta_scmi_client.h:8`); the
    commands are CAPABILITIES 0 (value out: a = capability mask),
    PROCESS_SMT_CHANNEL 1, PROCESS_SMT_CHANNEL_MESSAGE 2, GET_CHANNEL_HANDLE
    3 and **PROCESS_MSG_CHANNEL 4** (`:19`, `:32`, `:47`, `:59`, `:69`), the
    OCALL2 threads 2048 and 2049 (`:81`, `:93`). The dispatcher is
    `scmi.c:400-424`. Command 4 takes exactly three parameters: `VALUE_INPUT`
    with the channel in a, `MEMREF_INPUT` the SCMI request, `MEMREF_OUTPUT`
    the reply buffer, and any other parameter type returns
    BAD_PARAMETERS (`scmi.c:131-172`); the reply length comes back in the
    output memref's size. The message is the SCMI one with the 32-bit message
    header, no shared-memory-transport (SMT) header: `PTA_SCMI_CAPS_MSG_HEADER`
    (bit 1, `pta_scmi_client.h:103`) is what the `_CFG_SCMI_PTA_MSG_HEADER`
    build option enables (`mk/config.mk:784-787`). The platform builds the
    server with `CFG_SCMI_SCPFW` (`plat-stm32mp2/conf.mk:205`), so the
    message goes to `scmi_server_msg_process_thread`
    (`core/lib/scmi-server/scmi_server.c:62-74`) and an unknown channel is
    BAD_PARAMETERS. Opening the session requires the caller's login to be
    `TEE_LOGIN_REE_KERNEL` (0x80000000, `scmi.c:381-396`,
    `lib/libutee/include/tee_api_defines_extensions.h:138`), else
    ACCESS_DENIED. Whether a non-secure agent that is not Linux can present
    that login, and how OPEN_SESSION's login class (`optee_msg.h:87-92`
    lists only the GlobalPlatform classes) maps to it, is **open**: it is the
    first thing the board's reply to our OPEN_SESSION shows, and `scmi.c`
    would show the same in a `DMSG` ("Expecting TEE_LOGIN_REE_KERNEL") on a
    debug OP-TEE, which the board's is not known to be. The channel number
    (0, `stm32mp251.dtsi:103-108`) and CAPABILITIES (command 0) can be
    asked first, harmlessly.
  - *The perf domain is OP-TEE's too.* The GPU is perf domain 1 because the
    CPU's is registered first and the GPU's second
    (`plat-stm32mp2/scmi_server_scpfw.c:360-381`), and OP-TEE probes the
    `vivante,gc` node itself for it (`plat-stm32mp2/main.c:495-500`). A
    level change goes to `stm32_gpu_opp_set_rate` and is **refused with
    ACCESS_DENIED while the GPU's supply regulator is on**
    (`core/drivers/stm32_gpu_opp.c:50-58`, "Change OPP not allowed when GPU
    is used"); it sets the voltage, then the clock rate (`:60-72`). So the
    level is requested before power domain 0 is turned on (the order
    galcore uses), and a failed request leaves the GPU off, which is
    harmless. The same driver `panic()`s at OP-TEE's own boot if the
    default OPP cannot be set (`:337-340`), a boot-time failure that is not
    reachable from a request.
- **Memory map, interrupt, security.** The board reports (#1281, confirmed
  by ST's tree): the GPU registers at **0x48280000, size 0x800**
  (`stm32mp255.dtsi:171`; the CPU must not read above 0x800 anyway,
  section 6), the interrupt **GIC SPI 183**, level high (`:172`), **no
  `iommus`**, and `access-controllers = <&rifsc 79>` (`:178`). The memory is
  4 GiB; ST's tree maps `memory@80000000` 0x1_0000_0000 bytes
  (`stm32mp257f-dk.dts:142-145`), and its reserved-memory include marks
  everything below 0x84000000 as `no-map` (TF-A BL31 at 0x0a000000 in
  internal RAM, the M33 and TF-M windows from 0x80000000, the IPC buffers,
  `op-tee@82000000` of 32 MiB, `stm32mp257f-dk-resmem.dtsi:20-170`), so
  **System RAM is 0x84000000 to 0xfa7fffff** on the running board (#1281).
  Above it come a 64 MiB `no-map` **`gpu-reserved@fa800000`** (0xfa800000,
  size 0x4000000, `:170-173`) and two LTDC secure-layer areas at 0xfe800000
  and 0xff000000 (`:175-183`). A CMA pool of 128 MiB is declared inside
  0x80000000 to 0xffffffff (`:186-193`). Whether the GPU's RIFSC entry
  is open depends on the image variant (#1281 has the table); the board runs
  the plain `stm32mp257f-dk` variant, where entry 79 is open (#1281). OP-TEE's
  `ca35tdcid` configuration for the DK agrees: the GPU's entry is
  non-secure and nothing is to be programmed for it (r-optee-tfa, tag
  above). That settles Q4 for this variant only; others would close it.
  The memory (gc-mem-mp2, r-galcore): the 64 MiB at 0xfa800000 lies below
  4 GiB, the GPU's MMU maps it flat (GPU address equals physical address),
  and our image maps it as device or write-combine memory, with a barrier
  before the GPU is kicked. The interrupt handler reads the register at
  0x10 once, which acknowledges it (zero means the interrupt is not ours).
  Coherence (Q14) is in no tree.
- **Display.** A Zephyr driver for the MP2's display controller and its
  DSI host and ADV7535 bridge (LTDC → DSI → ADV7535 → HDMI, #1281; not LVDS)
  does not exist (above); the i.MX8MP's display was a driver of ours
  (`driver/docs/imx8mp-display-cold.md`). Q7; no item of plan #322 needs it.

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
and needs the ST chain; its facts are now in section 1 (U-Boot's flow) and
the return trial records the rest (Q8). Whether the image may start at the
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
2. **A shim entry per candidate row, and the reference tables.** (Plan
   #322 narrows this to the 0x8000/0x6205 row, #1281.) Add the
   four ST rows' ids to the etnaviv drm-shim next to the `GC7000UL` entry
   that `tools/vivante/mesa-cs-golden.patch` adds (the entry carries the
   five ids and no feature words — `.hwdb_ids = true`, patch lines `:61`,
   `:71-77` — so the driver looks the row up in the hwdb, over a DRM 1.4
   interface). Then, for each row: (a) `cs_golden` produces the golden
   command stream for the gl_test shapes; diff it against the i.MX8MP's
   golden (`tools/vivante/README.md`, "The golden command stream"). Check:
   the diff is exactly the expected one (line-width state, no
   `SH_NO_ONECONST_LIMIT` effect on the stream, nothing else); anything
   else is a finding. **Done for the 0x8000/0x6205 row (shim-golden, `tools/vivante/README.md`, "Another part: the ST row" (commit `3dd64152`)):**
   `ETNA_SHIM_GPU=GCNANOULTRA31` (customer id 0x15 from the hwdb, the
   board's was not read); Mesa 26.2.1's driver runs on it in every
   `cs_golden` mode, the i.MX8MP goldens stay byte-identical, and the
   streams have the same shape (same submits, commands and `LOAD_STATE`
   counts) as the GC7000UL's. The differences are not the expected ones:
   (1) `PE_MEM_CONFIG` (0x014bc) and `TS_MEM_CONFIG` (0x01654): the i.MX8MP
   gets colour/depth tile-status modes and compression, the ST row gets the
   mode bits 0 and only colour fast clear (0x02, or 0 for mipmapped
   textures), so Mesa uses no tile-status compression on this row;
   (2) mode 25 (a `vec4` array read at a uniform index): three shader
   words differ (`SH_NO_ONECONST_LIMIT`) and `SH_HALTI5_UNIFORMS[0]` is
   `0x2 0xffef7000 0 0` against `0x2 0x10 0xffef7000 0`; modes 2 and 23
   (the one-constant cases) do not move; (3) mode 28, `glLineWidth(4)`:
   `ALIASED_LINE_WIDTH_RANGE` 1..1 against 1..8192, and `PA_LINE_WIDTH` /
   `PA_WIDE_LINE_WIDTH0/1` hold 0.5 against 2.0 (the registers hold half
   the width; Mesa clamps to 1.0). Nothing else in the common state moves. (b) `etna_shc` with the ST ids compiles the whole set
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
3. **Table-driven identity in the kernel side.** (Plan #322: the row
   needed is 0x8000/0x6205, #1281; the others only where a table costs
   nothing.) Replace the single i.MX8MP
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
6. **An SCMI client with a model, in two halves.** The transport is OP-TEE's
   message protocol (section 4), so the client is an SCMI codec (the
   messages of the performance and power-domain protocols, against the SCMI
   specification, Arm DEN0056) under an OP-TEE transport (shared memory, the
   session, invoke command 4). The two halves are byte-tested separately on
   the host. Zephyr's SMC transport is not used. The channel is 0 and the ids
   are those of section 4.
7. **Read the remaining unread sources** and update this note: RM0457
   (GPU, RCC, RIFSC, LTDC/DSI chapters; URL in `index.rst:211-212`), UM3385,
   the MB1605 schematic, ST's wiki pages on the MP25 GPU and the SCMI server,
   and the OP-TEE, TF-A and U-Boot sources of ST's chain (the last three are
   done in the fourth pass; RM0457 could not be fetched and has to be
   supplied as a local file or by a route st.com accepts). Done so far:
   Linux mainline (pass two: the SCMI transport, the display controller,
   the hwdb half of Q9, the candidate-row cross-check), the board's
   day-one readings (#1281: Q1, Q5, Q6, Q9, Q10 and parts of Q3, Q4, Q7,
   Q8) and ST's kernel device tree for the DK (this pass; it confirms the
   device-tree values of #1281, section 4). Check: each question in section
   7 gets a citation or stays an unknown.
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

1. **Under ST's Linux, read, do not drive. Done** (2026-11-05 and
   2026-11-06, by hand, #1281). It settled Q1, Q5, Q6, Q9 and Q10 and parts
   of Q3, Q4, Q7 and Q8, and showed that ST's Linux drives the GPU with
   galcore 6.4.21, not etnaviv, so the "identity as etnaviv prints it" the
   first drafts of this item asked for does not exist; the identity was read
   under Linux by hand (#1281). The unread list is in #1281. What is still to
   read from the board is `CNTFRQ_EL0` from a CPU (40 MHz is expected,
   board-read), the customer id, and what U-Boot hands over (Q6, Q1, Q8).
   **Before any GPU step**, the first board fact is the return: an image that
   prints a banner and ends, hangs or faults must give the board back by
   itself. The watchdog is already armed from BL2, so the smallest image
   needs only a PSCI SYSTEM_RESET at the end and a one-shot boot so that
   the next boot is ST's Linux (section 1).
2. **From U-Boot, power the GPU through SCMI and read the identity** with
   the Zephyr image (the `yd_gc_identify` sample of item 5.5), in the order
   of section 4: performance level, power domain 0, `RCC_GPUCFGR` (offset
   unconfirmed from RM0457), then the identity registers. A result that
   looks like a hardware fault is first checked against the supply (the
   USB-C cable is the only one). Alive if
   `CHIP_MODEL` and `CHIP_REV` read 0x8000 and 0x6205 (#1281), `HI_INTR_ENBL`
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

| id | question | status and what answers it |
| --- | --- | --- |
| Q1 | Which row is the MP25's GPU? | **Answered (#1281):** model 0x8000, revision 0x6205, product 0x80003, eco 0, the third candidate row (section 1); Linux's etnaviv hwdb carries exactly that id (`etnaviv_hwdb.c:199-204`). Open: the customer id was not read (the row's is 0x15) |
| Q2 | Why do all candidate rows carry NPU fields? Is the NPU a separate block and the row naming shared? | open: RM0457 (NPU and GPU chapters; could not be fetched, so unchanged); the ST wiki |
| Q3 | How is the GPU turned on: which ids, which rate, and does OP-TEE's PLL init panic on our request as it did when etnaviv loaded (the brief)? | **In part (#1281, ST tree):** SCMI over OP-TEE (`linaro,scmi-optee`, channel 0, `stm32mp251.dtsi:103-108`) with protocols 0x11, 0x13, 0x14, 0x16, 0x17; the GPU node uses performance (0x13, domain 1) and power domain (0x11, domain 0), and its clocks (`CK_BUS_GPU` 91, `CK_KER_GPU` 340) and reset (`GPU_R` 75) are plain `&rcc` entries (`stm32mp255.dtsi:173-176`). **Corrected (r-optee-tfa, OP-TEE 4.0.0-r3.1; board-read, 2026-11-06):** the transport is OP-TEE's message protocol to the SCMI pseudo-TA (invoke command 4), not an SMC mailbox; the board has no SMC id and no shmem node for the non-secure side. The order is performance level, power domain 0, then `RCC_GPUCFGR` (0x83C: bit 1 the clocks gate, bit 0 the reset; r-galcore; offsets unconfirmed from RM0457); galcore sets no rate and no level, and `CK_SCMI_PLL3` and the PLL3 registers are never used. **ABI (note-optee, OP-TEE `4.0.0-stm32mp-r3.1`, section 4):** SMC ids 0xb2000009 (capabilities, derived reply 0x8074), 0x32000004/12/13 (calls), SCMI pseudo-TA a8cfe406-d4f5-4a2e-9f8d-a25dc754c099, command 4 with value(channel), memref in, memref out; a level change is refused while the GPU supply is on. Open: our own request has not been tried; the 0x8074 value is derived from the source, not read from the board (the capability exchange of the return trial's image); whether a non-secure agent that is not Linux can open the session (the pseudo-TA wants `TEE_LOGIN_REE_KERNEL`); the message parsing path of OP-TEE (not read); the cause of the brief's panic (not established, section 7; the console text, `clk-stm32mp25.c`, OP-TEE's `scmi_pd_consumer.c`/`scmi_clock_consumer.c`; the board) |
| Q4 | Is the GPU (and its DMA master) in a resource-isolation compartment that a non-secure A35 image may use, or does the secure side have to set it? Does anything else have to be programmed by the secure side first? | **In part (#1281):** it depends on the image variant; the board runs the plain `stm32mp257f-dk` variant, where RIFSC entry 79 (the GPU's, `access-controllers = <&rifsc 79>`, `stm32mp255.dtsi:178`) is open; #1281 has the table of variants. OP-TEE's `ca35tdcid` configuration agrees: entry 79 is non-secure and nothing is needed for this variant (r-optee-tfa). Open: whether anything else must be set first (the DMA master's compartment, the reserved area): RM0457 (RIFSC; not fetchable so far); the board |
| Q5 | The GPU's base address and size, interrupt number, and power domain. | **Answered (#1281, confirmed by ST's tree `stm32mp255.dtsi:169-183`):** `vivante,gc` at 0x48280000, size 0x800; GIC SPI 183, level high; three power domains: SCMI performance 1, SCMI power domain 0 (`PD_SCMI_GPU`), and `power-domain-d1` |
| Q6 | Interrupt controller version, how the GPU's interrupt is routed to the A35, and the arch timer rate. | **In part (#1281):** GIC-400 at 0x4ac00000 (ST `stm32mp251.dtsi:166-175`), the GPU's interrupt is SPI 183 (above). The arch timer node has no `clock-frequency` (`:261-268`); the rate is **40 MHz** (board-read, 2026-11-06) and `CNTFRQ_EL0` is expected to read the same (r-optee-tfa). The GIC is a GICv2 whose secure SGIs 8 to 15 and PPI 29 TF-A keeps. Open: `CNTFRQ_EL0` read from a CPU (the return trial prints it) |
| Q7 | The display path: controller, layers, formats and stride alignment, and the HDMI chain on the DK. | **In part (#1281, ST tree):** LTDC → DSI → ADV7535 → HDMI (`stm32mp257f-dk.dts:536-547`, `:328-350`, `:428-460`), not LVDS. Open: layers, formats, stride alignment, the DSI host and the ADV7535's setup, the backlight: UM3385, MB1605, RM0457's LTDC and DSI chapters. Not needed by plan #322 |
| Q8 | The exact boot chain and what state U-Boot hands over (exception level, MMU, caches, display on or off), and whether the SMC interface to OP-TEE stays open after U-Boot. | **In part (#1281):** TF-A v2.10 (BL2, BL31), OP-TEE 4.0, U-Boot 2023.10 (ST's), Linux 6.6.129 via extlinux from the microSD; PSCI 1.1 over SMC; Linux's watchdog is `arm,smc-wdt` (OP-TEE's), 32 s (`stm32mp251.dtsi:78-82`, `stm32mp257f-dk.dts:216-219`). **Folded in (r-optee-tfa, r-uboot, board-read, 2026-11-06):** BL2 starts the IWDG, so the watchdog is armed at hand-over and a hang resets in about 32 s; the image enters at EL2; U-Boot's distro loop reaches extlinux and leaves `boot.scr.uimg` alone while extlinux succeeds (section 1); the environment is a CRC32, a flag byte and strings at offset 0x7e000 of its partition. Open: the MMU and cache state and the DTB pointer actually handed over, and whether a key in the first second stops autoboot: the return trial's image records the hand-over |
| Q9 | Does Linux etnaviv carry the MP25 ids, and does anything in ST's Linux depend on a vendor blob? | **Answered (#1281):** mainline etnaviv's hwdb carries the exact row (`etnaviv_hwdb.c:199-204`), but ST's image does not use etnaviv: it runs **galcore 6.4.21** (out of tree) and its own OpenGL ES 3.1 user space. The kernel has etnaviv as an unloaded module; loading it reportedly panicked OP-TEE, so no etnaviv reference exists from the board and the host references are Mesa on the drm-shim |
| Q10 | Is the GPU behind an IOMMU, and what physical address width reaches the GPU? | **Answered in part (#1281, confirmed by ST's tree):** no `iommus` on the GPU node (`stm32mp255.dtsi:169-183`), so our carveout stays identity-mapped in the GPU's own MMU v2 as on the i.MX8MP. The 64 MiB carveout at 0xfa800000 lies below 4 GiB and the GPU's MMU maps it flat, GPU address equal to physical address (gc-mem-mp2, r-galcore). The address width that reaches the GPU is not in any tree: RM0457 |
| Q11 | What do wide lines do on a `WIDELINE_TRIANGLE_EMU` core when we program the wide-line registers the way the i.MX8MP backend does (`src/vivante/backend.rs:1376`, `:1425-1426`)? What should the build report as the line width range? | **Answered for the reported range (q11-wideline, Mesa 26.2.1 `etnaviv_screen.c:366-368`):** the ST profile reports `GL_ALIASED_LINE_WIDTH_RANGE` as 1.0 to 1.0. Open: what the hardware does when the wide-line registers are programmed anyway; the golden stream shows only what Mesa writes (mode 28: `PA_LINE_WIDTH` 0.5, clamped, against 2.0 on the i.MX8MP; `tools/vivante/README.md`, "Another part: the ST row" (commit `3dd64152`)); a board test |
| Q12 | Does our compiler ever use the address-register paths or partial-component address writes that the ST rows' `SH_NO_INDEX_CONST_ON_A0` and missing `SH_MOVAI_MOVAR...FIX` suggest are unavailable or unsafe? | open: the scan of the online compiler's output (`src/shaderc/online/vivante`) for `movar` and `movai` has run (q12-scan); its verdict is in that study's report and is not folded in here. The golden stream adds one data point: with the one-constant limit lifted, Mesa's indexed-uniform shader (mode 25) changes three words and `SH_HALTI5_UNIFORMS[0]` (`tools/vivante/README.md`, "Another part: the ST row" (commit `3dd64152`)); our compiler's output is the stricter form. The board's gl_test and dEQP runs settle any remainder |
| Q13 | The A35 SoC definition: cache and MMU state at entry, PSCI or the monitor interface, console UART, and whether Zephyr's arm64 core runs on this part without changes. | **In part (ST tree):** PSCI 1.0-style over SMC (the board reports 1.1, #1281), console `usart2` at 0x400e0000 on PA4/PA8, GIC-400 at 0x4ac00000 (section 4 for the lines). **Folded in (r-optee-tfa, r-zephyr-a35, board-read):** entry at EL2 at the load address, GICv2 with SGIs 8 to 15 and PPIs 29 and 31 left alone, a 40 MHz counter, USART2 at 0x400e0000, the arm64 GICv2 path unchanged. Open: cache and MMU state at entry (the return trial records it), the UART's kernel clock and whether U-Boot left it set up; whether the SoC definition runs (item P4) |
| Q14 | The GPU's coherence with the A35 cluster, and the right memory attributes for the carveout. The i.MX8MP uses a non-cacheable mapping and no flushes (`imx8mp-gc-driver-notes.md`, "M3: the ring and one submit"). | open: RM0457 interconnect chapter (not fetchable so far); board test. The area for it is the 64 MiB `gpu-reserved` at 0xfa800000 (#1281; ST tree `stm32mp257f-dk-resmem.dtsi:170-173`) |
| Q15 | ~~Does the 0x6207 row (one shader core, eight streams) apply?~~ | **Dropped, not applicable (#1281):** the board's revision is 0x6205, so the 0x8000/0x6207 row is not this part. The table-driven identity (item 5.3) still carries the stream count per row, which costs nothing |

Risks that are not questions: the finished ports took the board work
(power, display, boot) as the long part, and the GPU work as the short one;
this port adds SCMI as a new dependency on a secure-side component we do
not own. The OP-TEE panic the brief names is a reason to put the SCMI
request first on the day-one list and to log every message.

**The etnaviv panic and what our image avoids** (note-optee). The panic
itself is the brief's report (etnaviv loaded on the board, OP-TEE panicked);
its console text was not read and the cause is **not established** here. The
earlier study of the path (optee-panic) did not leave a report this item
could read, so what follows is re-derived and marked by how well it is
known.

- *Read from the sources.* ST Linux `v6.6-stm32mp-r3.1` (tag object
  29611595b9dc, `drivers/gpu/drm/etnaviv/etnaviv_gpu.c`): the driver's
  runtime resume enables its clocks one by one with `clk_prepare_enable`
  (reg, bus, core, shader; **approximately** lines 1590-1610,
  `etnaviv_gpu_clk_enable`), and the hardware bring-up then writes
  `HI_CLOCK_CONTROL` and polls `HI_IDLE_STATE` in `etnaviv_hw_reset`
  (**approximately** 506-575), reads the identity registers in
  `etnaviv_hw_identify` (**approximately** 332-360) and enables interrupts
  in `etnaviv_gpu_hw_init` (**approximately** 716). Those line numbers come
  from one counting pass over a web copy of the file, because git access to
  the tag was refused on the machine that wrote this; they are to be rechecked
  with `git show v6.6-stm32mp-r3.1:drivers/gpu/drm/etnaviv/etnaviv_gpu.c`
  before another document quotes them. etnaviv uses no reset control in that
  file (no `reset_control_*` call was found by that pass), unlike galcore's
  explicit reset bit; which level request, if any, the kernel's power-domain
  layer sent before it is not known. OP-TEE side
  (`optee_os` `4.0.0-stm32mp-r3.1`): the level change is refused while the
  regulator is on (`core/drivers/stm32_gpu_opp.c:50-58`), the driver `panic()`s
  only at OP-TEE's own boot (`:337-340`); a runtime panic from a request
  must therefore come from another path (the RCC clock handling, the power
  domain, or a bus fault from the GPU registers), which this item did not
  find. **Open:** the cause. The sources that would answer it: the console
  text of the panic (the brief's report; the signature is the line OP-TEE
  prints with `E/TC:` and the call stack), `drivers/clk/stm32/clk-stm32mp25.c`
  and the `scmi_pd_consumer.c`/`scmi_clock_consumer.c` handlers in OP-TEE's
  SCMI server (`core/lib/scmi-server/`, not read), and a board session that
  loads nothing and only logs. Plan #322 never loads etnaviv.
- *What our image avoids, whatever the cause.* It follows galcore's order,
  not etnaviv's: the level first, power domain 0 second, then
  `RCC_GPUCFGR`, never the clock gates before the domain; it sets no clock
  rate; it touches no PLL and no `CK_SCMI_PLL3`; it reads nothing of the
  GPU above offset 0x800; it requests only the two domains named in
  section 4 and logs every request and reply before the next is sent, with
  the reply status, so a panic leaves the last request on the console; and
  it leaves the GPU on a failed request as it found it. The watchdog
  (about 27 s after a panic to the reset, 32 s without one) gives the
  board back; the return trial proves that before any request is made.

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
| P1 | Read the unread documents (section 5, item 7) and update this note; record the answers to Q1 to Q15 that sources give. Open GitHub issues for what stays open. **Fourth pass 2026-11-06: #1281, ST's kernel tree and the OP-TEE, TF-A, U-Boot, galcore, RCC/RIFSC, Zephyr A35 and memory studies applied; RM0457 is still unread** | each question cited or left | S |
| P2 | Pin the hwdb comparison (5.1) and the shim rows with goldens and an ST table (5.2) | gate rows; goldens diffed | M |
| P3 | Table-driven identity (5.3) and the chip profile (5.4) | host tests, both profiles | M |
| P4 | A35 SoC and board in Zephyr, identify sample built (5.5) | Zephyr build row | M |
| P5 | The SCMI codec, the OP-TEE message transport and the ST power module with model tests (5.6; section 4 for the order) | byte-exact host tests | M |
| P6 | Board day one (section 6, items 1 and 2): identity under Linux (**done, #1281**), the return trial (banner, PSCI reset, back in Linux without a person), power and identity from our image | the board returns; the ids read; `HI_INTR_ENBL` read-back | S, board |
| P7 | `gc_fence`, `gc_clear`, `gc_triangle`, `gc_gl_triangle` | read-backs | M, board |
| P8 | The display: LTDC, DSI, ADV7535 to HDMI, backlight; scan-out of the GPU's buffer | picture on the panel, a record like `imx8mp-display-cold.md` | L |
| P9 | `gl_test` regression set, then the dEQP-GLES2 sweep, diffed against the i.MX8MP's failures; the line-width decision (Q11) | RESULTS rows | L, board |
| P10 | Netboot or direct boot from the board's bootloader (`imx8mp-netboot.md` shape), sign-off from the bootloader; the one-shot off the microSD follows U-Boot's distro loop (section 1) | a run from U-Boot; `tools/board.sh` | M |

P2 to P5 can run without the board; P1 only needs documents. The board is on the bench (#1281); plan #322 narrows P2 and P3 to the 0x8000/0x6205 row.
