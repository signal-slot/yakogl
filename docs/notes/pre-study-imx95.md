<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: Toradex Verdin iMX95 on the Mallow carrier (NXP i.MX 95, Mali-G310)

Date 2026-11-20, main `1d9022e7`. Host research only: no board was touched.
The owner fixed the board on 2026-11-20: a Toradex Verdin iMX95 on a Mallow
carrier. This note is the pre-study for that board. NXP's EVKs and FRDM and
the other Verdin carriers appear only as a comparison.

Five topics have their own documents, written in parallel. This note stays
at survey level for them and points there:

| Topic | Document |
| --- | --- |
| the Zephyr board and its build-only gate row | `driver/boards/toradex/` (the board) and `tools/check.sh` (the row) |
| the SCMI GPU clock and power sequence, with a mock, and the permission question in depth | `docs/notes/imx95-gpu-power.md` |
| the display path in depth | `driver/docs/imx95-display.md` |
| the Mali kernel side reading its parameters from registers | `driver/rust/src/gpu/mali/` (its own change) |
| Mesa's G310 against the G610 | `docs/notes/mali-g310-vs-g610.md` |

The depth here goes to what none of them covers: the module's hardware
variants, the boot chain and the sign-off route, the firmware, and the
item list with its ordering.

## Verdict in eight lines

1. The module is the Verdin iMX95 "Hexa" in four product IDs: 0089 (8 GB,
   Wi-Fi/BT, IT), 0226 (4 GB, WB, IT), 0227 (4 GB, ET) and 0228 (16 GB,
   IT) (TU: `board/toradex/common/tdx-cfg-block.c:160, 193-195`;
   TL: `conf/machine/verdin-imx95.conf:109`). For us they differ only in
   DRAM and in the Wi-Fi DT include. GPU presence is a fuse (bank 2,
   word 3, bit 17), read on day one.
2. The GPU is the Mali-G310 at `0x4D900000`, arch 10.12. Its firmware is
   **byte-identical to the arch 10.8 file we already pin**: linux-firmware
   has `arm/mali/arch10.12/mali_csffw.bin` at 282624 bytes with sha256
   `a27847ea…405b`, the same as `arch10.8` and `arch10.10`
   (FW: the three files; repo `tools/cts/deps.toml:316-317`). The firmware
   is not a new item.
3. The System Manager on the Verdin is Toradex's build of NXP's SM, config
   `verdin-imx95` (TL: `verdin-imx95.conf:85-94`; SMT:
   `configs/verdin-imx95.cfg`). It grants the non-secure A55 agent
   "AP-NS" the GPU (`GPU_NPROT`, which brings `PD_GPU`, `PERF_GPU` and
   `CLK_GPU_CGC`), the display controller, the LDB and the DSI. That agent's
   channel is the `mu2` at `0x445B0000`, the one Linux and Zephyr already
   use. A Zephyr image in place of Linux is that agent. The permission
   question is answered "yes" from the configuration; the depth is
   `docs/notes/imx95-gpu-power.md`.
4. The boot chain is ROM (on the M33) → ELE + OEI (DDR) + SM + A55 SPL in
   one container → TF-A BL31 at `0x8A200000` (+ OP-TEE) → Toradex U-Boot
   2024.04 at `0x90200000` (TL; MK: `iMX95/soc.mak:43-44, 396-400`). The
   SM stays resident and serves SCMI in every route.
5. The sign-off route of our two Verdins carries over: Torizon (or Linux)
   on the eMMC, `ext4load mmc 0:1` and `go` from U-Boot through the
   `tools/board.sh` hook (`docs/standalone-boot.md`). Toradex's U-Boot has
   `go`, `CMD_CACHE` and `CMD_EXT4_WRITE`, and the eMMC is `mmc 0` (TU).
   What changes is the load address, the reset path (TF-A is built with
   full-system PSCI reset) and the watchdog (WDOG3 is a system reset,
   WDOG4 only an A55 reset).
6. Mallow has no console: our Verdin iMX8M Plus on Mallow already lives
   without one (`docs/notes/imx8mp-gc-bringup.md:152-170`). On the i.MX 95
   the SNVS record is gone: the battery-backed block belongs to the SM. The
   AP-NS agent owns `GPR4`..`GPR7` of the BBSM mix through SCMI, the likely
   new record (U2).
7. The display on Mallow is Verdin DSI: a DSI panel, or Toradex's
   DSI-to-HDMI adapter (Lontium LT8912B). Mainline Linux has no i.MX 95
   display driver or node at all. Toradex's 6.6 kernel and its overlays
   carry it (survey: section 1.4; depth: `driver/docs/imx95-display.md`).
8. Zephyr, at the pin and on upstream main, has no Verdin iMX95. Our board
   is out of tree in `driver/boards/toradex/`, over the pin's i.MX 95 A55
   SoC support, as our other two Verdins are.

## What was read, and what was not

Prefixes used below. Line numbers are exact at the named version.

| Prefix | What, at what version |
| --- | --- |
| `L:` | mainline Linux master `65e4fd942f72a9dba0e4e257290c4fa1aa756854`, `Makefile` 7.4.0-rc3 |
| `FW:` | linux-firmware `main` `f9919c7a5892be53a36100fa18d8620b5fe7d6c4` (`gitlab.com/kernel-firmware/linux-firmware`) |
| `SMT:` | Toradex's System Manager `git.toradex.com/imx-sm-toradex.git`, branch `main` at `3aa8a77a15d66701df5962e883b77ddd1db8c215`. The Yocto recipe pins `cc7a8d2d5c925dce3f1369e70d0d5dd63bb2449e` (TL: `recipes-bsp/imx-system-manager/imx-sm-toradex_1.0.0.bb:15-17`). That commit is in the history of `main`, five commits back. Between the two, `configs/verdin-imx95.cfg` is unchanged, and the generated files differ only in the M7 agent's `seenvId` (1 → 0) and `GEN_CONFIG_VER` (2U → 3U) (`git diff cc7a8d2 3aa8a77 -- configs/verdin-imx95.cfg configs/verdin-imx95/`). So the rights read here are the shipped ones |
| `SM:` | NXP's System Manager `github.com/nxp-imx/imx-sm`, `master` at `eec232878ab7dd460f5a8f3767d39dc7e9412efa` |
| `TL:` | Toradex's Yocto layer `git.toradex.com/meta-toradex-nxp.git`, branch `scarthgap-7.x.y` at `b0b9f0fd9d3e4854e3d764bcd2b383b337318f3a` |
| `TU:` | Toradex's U-Boot `git.toradex.com/u-boot-toradex.git`, branch `toradex_imx_lf_v2024.04` at `b7c747d8c084504a4096f9eeea9191e9acd6c5e2`, which is exactly the recipe's `SRCREV` (TL: `recipes-bsp/u-boot/u-boot-toradex_2024.04.bb:11-14`) |
| `TO:` | Toradex's overlays `git.toradex.com/device-tree-overlays.git`, branch `toradex_6.6-2.2.x-imx` at `e9a3ca6fbd6bd5fab7f00cdb87360be44373cf69` (the recipe pins `aa87f343`, TL: `recipes-kernel/linux/device-tree-overlays_git.bb:3-4`; read at the tip) |
| `MK:` | NXP's `imx-mkimage`, default branch `lf-6.18.20_2.0.0` at `1b577853ae1afe1f26cdef27548da52fb424af48` (which mkimage revision Toradex's build uses comes from meta-imx and was not checked) |
| `Z:` | Zephyr at the workspace pin `af17c0c8aa2a82a0b714a423915ad3610063ee68` (4.4.99) |
| `ZM:` | Zephyr upstream `main` at `4f2d5263e21e731928ffe14ca1e8b366fdaeece8` (4.5.99) |
| `HAL:` | `hal_nxp` at `1e91007b164c52b74d58d4e75a856efbc5d4de05` (the revision Zephyr's `west.yml:215-217` pins) |
| `M:` | Mesa 26.2.1 (the tarball of `third_party/mesa-26.2.1/UPSTREAM`; the cited files compared with the tarball by sha256) |
| `repo:` | this repository at `1d9022e7` |

Not read:

- Toradex's web documentation: the Verdin iMX95 data sheet, the Mallow
  data sheet and product page, and the developer pages. This host's
  session refuses web fetches and searches. The Mallow facts below come
  from its mainline device tree and from what this repository already
  recorded about the Mallow under our Verdin iMX8M Plus.
- Toradex's kernel `linux-toradex.git` (branch `toradex_6.6-2.2.x-imx`, the
  recipe's): only its overlays repository was read.
- Toradex's OEI (`imx-oei-toradex.git`) and TF-A sources. Only Toradex's
  TF-A patch in TL was read.
- NXP's reference manual and data sheet (login): which i.MX 95 part
  number the module carries, and the fuse map behind section 1.1.
- A Panthor probe log of an i.MX 95 (mailing lists): the G310's
  configuration registers stay a day-one read.

## 1. The hardware

### 1.1 The module and its variants

| Product ID | Name | Source |
| --- | --- | --- |
| 0089 | "Verdin iMX95 Hexa 8GB WB IT" | TU: `tdx-cfg-block.c:160`; enum `tdx-cfg-block.h:118` |
| 0226 | "Verdin iMX95 Hexa 4GB WB IT" | TU: `tdx-cfg-block.c:193`; `tdx-cfg-block.h:151` (`= 226`) |
| 0227 | "Verdin iMX95 Hexa 4GB ET" | TU: `tdx-cfg-block.c:194`; `tdx-cfg-block.h:152` |
| 0228 | "Verdin iMX95 Hexa 16GB IT" | TU: `tdx-cfg-block.c:195`; `tdx-cfg-block.h:153` |

The Yocto machine lists the same four IDs (TL: `verdin-imx95.conf:109`).
What differs, and what it means for us:

- **DRAM: 4, 8 or 16 GB.** U-Boot sizes it by aliasing probes at 2, 4, 8
  and 16 GB above `PHYS_SDRAM` (TU: `board/toradex/verdin-imx95/
  verdin-imx95.c:118-137`). It keeps the first 256 MiB for firmware:
  `PHYS_SDRAM = 0x80000000 + SZ_256M`, `PHYS_SDRAM_SIZE = SZ_2G - 256M`,
  and a second bank of up to 6 GB (TU: `include/configs/verdin-imx95.h:
  17-24`). Every SKU has at least 4 GB, so an image linked inside the first
  2 GB runs on all four.
- **Wi-Fi/Bluetooth ("WB").** U-Boot selects the device tree by product ID:
  0089 and 0226 are `wifi`, the others `nonwifi` (TU: `verdin-imx95.c:
  86-108`). The `wifi` include powers a MAYA-W260 module on `usdhc3` and
  Bluetooth on `lpuart6` (L: `imx95-verdin-wifi.dtsi:10-50`); `nonwifi`
  disables `usdhc3` (L: `imx95-verdin-nonwifi.dtsi:10-16`). Nothing of
  ours touches either.
- **IT/ET**: temperature grades. No effect on software.
- **"Hexa"**: six Cortex-A55. Which i.MX 95 part number Toradex fits (and
  so its fused options) is in Toradex's data sheet (not read). What the
  software sees is the fuses. NXP's code in Toradex's U-Boot reads OCOTP
  bank 2 word 2 (NPU, A55 cores 4/5, LVDS, ISP) and word 3 (PCIe, **GPU
  mix bit 17**, VPU, JPEG, MIPI CSI, **MIPI DSI bit 24**), and deletes the
  device-tree nodes of what is fused off (TU: `arch/arm/mach-imx/imx9/scmi/
  soc.c:1180-1240`; the GPU node `/soc/gpu@4d900000` at :1044-1056). The
  SM refuses `PD_GPU` when its own fuse `DEV_SM_FUSE_GPU3D_DISABLE` is set
  (SMT: `devices/MIMX95/sm/dev_sm_fuse.c:82, 201`). Toradex's U-Boot has
  `CONFIG_CMD_FUSE=y` (TU: `configs/verdin-imx95_defconfig:74`), so
  `fuse read 2 2 2` at the prompt pins GPU, DSI and LVDS presence on the
  real module before any image of ours runs (B1).

Common to every SKU (L: `imx95-verdin.dtsi`):

- Console: `stdout-path = "serial2:115200n8"` with `serial2 = &lpuart1`
  (:30-41), which is Verdin UART_3 (L: `imx95-verdin-mallow.dtsi:159-162`).
  Toradex's U-Boot names it `console=ttyLP2` (TU: `verdin-imx95.env:4`). The
  SM's own debug UART is instance 2 (`BOARD DEBUG_UART_INSTANCE=2`, SMT:
  `configs/verdin-imx95.cfg:44-45`), that is LPUART2, Verdin UART_4 (L:
  `imx95-verdin.dtsi:529-532`).
- eMMC: `usdhc1`, 8 bit, non-removable, `mmc0` (L: :25, 722-734); the SD
  slot is `usdhc2` = `mmc1`.
- Reserved memory: the M7's `0x80000000` + 16 MiB, its vrings and buffers
  at `0x88000000`..`0x88221000`, and a CMA of `0x3c000000` inside
  `0x80000000` + `0x7F000000` (L: :149-198).
- The GPU node is inherited from `imx95.dtsi` unchanged and enabled
  (L: `imx95.dtsi:2298-2311`; no Verdin file mentions `gpu`).
- Watchdog: Linux enables `wdog3` with `fsl,ext-reset-output` (L: :747-751).
- PMICs: PF09 and two PF53 (the thermal zones `pf09-thermal`,
  `pf53-arm-thermal`, `pf53-soc-thermal`, L: :629-674). They are the SM's
  (SMT: `BOARD I2C_INSTANCE=1`, cfg :47-48). We do not drive a PMIC on this
  board, unlike the T6's RK806.

### 1.2 The SoC and the GPU (survey)

- Mali-G310, `compatible = "nxp,imx95-mali", "arm,mali-valhall-csf"`,
  `reg = <0 0x4d900000 0 0x480000>`, clocks `IMX95_CLK_GPU_CGC` ("core")
  and `IMX95_CLK_GPUAPB` ("coregroup"), interrupts SPI 289/290/288
  (job/mmu/gpu), `power-domains = <&scmi_devpd IMX95_PD_GPU>`, OPPs 500,
  800 and 1000 MHz all at 920000 µV (L: `imx95.dtsi:253-270, 2298-2311`).
  The node has no `dma-coherent` and no `iommus`.
- The SCMI ids: `IMX95_CLK_GPU_CGC = 41 + 123 + 10 = 174`,
  `IMX95_CLK_GPUAPB = 83`, `IMX95_CLK_GPU = 84` (L:
  `arch/arm64/boot/dts/freescale/imx95-clock.h:51, 95-96, 186`);
  `IMX95_PD_GPU 14`, `IMX95_PD_DISPLAY 13`, `IMX95_PERF_GPU 9` (L:
  `imx95-power.h:22-23, 42`). Zephyr's binding at the pin has 83 and 84 but
  no `GPU_CGC` (Z: `include/zephyr/dt-bindings/clock/imx95_clock.h:95-96`).
- Panthor has nothing i.MX 95-specific: the match table is
  `mediatek,mt8196-mali`, `rockchip,rk3588-mali`, `arm,mali-valhall-csf`
  (L: `drivers/gpu/drm/panthor/panthor_drv.c:1881-1883`); the binding lists
  `nxp,imx95-mali  # G310` (L: `Documentation/devicetree/bindings/gpu/
  arm,mali-valhall-csf.yaml:22`); the model name comes from
  `GPU_PROD_ID_MAKE(10, 4)` → "Mali-G310" (L: `panthor_hw.c:130-135`).
- Mesa: `PAN_PROD_ID(10, 12, 4)`, variants v1..v5 by
  `CORE_FEATURES & 0xff`, tile buffer 16 KiB (v1-v3) or 32 KiB (v4-v5)
  (M: `src/panfrost/model/pan_model.c:88-100`,
  `src/panfrost/lib/kmod/panthor_kmod.c:162`). The comparison with the
  G610, and what it means for `src/mali`, is
  `docs/notes/mali-g310-vs-g610.md`.
- The register map and the GPU blk-ctrl, from NXP's headers: GPU registers
  `0x4D900000`, doorbells `0x4D980000`, `GPU_BLK_CTRL_GPUMIX` `0x4D810000`
  with `GPURESET` (bit 0 "GPU Reset Release") (HAL: `devices/i.MX/i.MX95/
  MIMX9596/MIMX9596_cm7_COMMON.h:1606, 1969, 1979`;
  `periph/PERI_GPU_BLK_CTRL_GPUMIX.h:411-414, 446-450`). On the Verdin
  that blk-ctrl belongs to the SM, not to the A55 (SMT: cfg :214), and the
  SM reloads the GPU mix's TRDC each time `PD_GPU` comes up
  (SMT: `devices/MIMX95/sm/dev_sm.c:233-234`,
  `dev_sm_config.c:269-279`). Who releases `GPURESET` on the i.MX 95 is in
  `docs/notes/imx95-gpu-power.md`. The i.MX 952 SM writes it
  (SMT: `devices/MIMX952/sm/dev_sm_config.c:390`), the i.MX 95 SM does not
  (no hit under `devices/MIMX95`).
- The configuration registers (`GPU_ID` revision, `CORE_FEATURES`,
  `SHADER_PRESENT`, `L2_FEATURES`, `TILER_FEATURES`, `MMU_FEATURES`,
  `COHERENCY_FEATURES`) are in no source read. Our `mali::ident` reads them
  (repo: `driver/rust/src/gpu/mali/mod.rs:130-146`) on day one (B2).

### 1.3 The Mallow carrier

From mainline (L: `imx95-verdin-mallow.dtsi`, 223 lines, and the two
`imx95-verdin-{wifi,nonwifi}-mallow.dts`):

- The two board files differ only in the Wi-Fi include and the names:
  `model = "Toradex Verdin iMX95 WB on Mallow Board"`, compatible
  `toradex,verdin-imx95-wifi-mallow` (L: `imx95-verdin-wifi-mallow.dts:
  11-20`), and "Toradex Verdin iMX95 on Mallow Board",
  `toradex,verdin-imx95-nonwifi-mallow` (L: `…-nonwifi-mallow.dts:11-20`).
- Enabled: `adc1`, `enetc_port0` (Verdin ETH_1, on-module PHY), `flexcan1/2`,
  `lpuart1` (Verdin UART_3, "used as the Linux console"), `lpuart2`
  (UART_4), `lpuart7` (UART_1), `lpuart8` (UART_2), `pcie0`, `tpm4/5/6`
  (PWM_1, PWM_2, PWM_3_DSI), `usb2`, `usb3`, `usdhc2` (SD_1), the wake key
  (L: :60-223).
- I²C: `i3c2` "Verdin I2C_3_HDMI", `lpi2c3` "Verdin I2C_2_DSI", `lpi2c4`
  "Verdin I2C_1" with a TMP1075 at 0x4f and the carrier EEPROM at 0x57,
  `lpi2c5` "Verdin I2C_4_CSI" (:107-136). A TPM (`infineon,slb9670`) on
  SPI_1 (:138-157). Four user LEDs on `gpio5` (:18-57).
- **No display node.** The carrier exposes the Verdin DSI signals (the
  file enables the DSI I²C bus and the DSI PWM), and the display itself is
  an overlay (section 1.4).
- **No console on USB.** For the same carrier under our Verdin iMX8M Plus
  this repository recorded: "The Mallow's debug UARTs are only on its
  unassembled X11 header (1.8 V), and its USB-C ports carry no serial
  converter" (repo: `docs/notes/imx8mp-gc-bringup.md:167-169`), and the
  ES3 recipe reaches Verdin UART_3 "through a serial bridge" when one is
  fitted (repo: `docs/es3/board-recipe-imx8mp.md:335-340`). The carrier is
  the same, so the i.MX 95's A55 console (UART_3) and the SM's console
  (UART_4) are on that header too. Whether the lab's Mallow has a bridge on
  it is a farm question, not a document one.
- The lab already has a Toradex 10-inch DSI capacitive-touch panel on a
  Mallow (repo: `docs/notes/imx8mp-gc-bringup.md:152-153`). Toradex ships
  the matching overlay for this module (TO:
  `overlays/verdin-imx95_panel-cap-touch-10inch-dsi_overlay.dts`).

The other carriers, one line each (L: their `imx95-verdin-*.dtsi`):

| Carrier | What differs from Mallow, for us |
| --- | --- |
| Dahlia | audio codec WM8904 and a USB hub (`usb424,5744`/`2744`) (L: `imx95-verdin-dahlia.dtsi:137, 244-251`); our Verdin AM62P runs on one (repo: `docs/notes/am62p-direct-boot.md:6`) |
| Verdin Development Board | audio NAU8822, GPIO expander, an Ethernet PHY for ETH_2 (L: `imx95-verdin-dev.dtsi:127-191`); U-Boot's default device tree (TU: `configs/verdin-imx95_defconfig:13`, `fdt_board=dev` `verdin-imx95.env:5`) |
| Ivy | industrial analogue I/O (ADS1119, muxes), TPM, FRAM (L: `imx95-verdin-ivy.dtsi:21-434`) |
| Yavia | small: sensors and EEPROM only (L: `imx95-verdin-yavia.dtsi:137-142`) |
| Zinnia | analogue inputs, TPM (L: `imx95-verdin-zinnia.dtsi:99-337`) |

None of them changes the GPU, the boot chain or the SM. The console header
and the display connector are the carrier-specific parts.

NXP's boards, for comparison (Z: board docs; L: device tree names):

| Board | Memory | Display | Zephyr at the pin |
| --- | --- | --- | --- |
| i.MX 95 EVK 19x19 | "128-Gbit LPDDR5" | two LVDS mini-SAS, one CSI/DSI mini-SAS (Z: `boards/nxp/imx95_evk/doc/index.rst:21, 52-56`) | `imx95_evk/mimx9596/a55` (+ `smp`), M7 |
| i.MX 95 EVK 15x15 | "8-Gbit LPDDR4x" | the same connectors (Z: `boards/nxp/imx95_evk_15x15/doc/index.rst:21, 47-49`) | `imx95_evk_15x15/mimx9596/a55`, M7 |
| FRDM i.MX 95 | not read | not read | none (L has `imx95-15x15-frdm.dts`, `imx95-19x19-frdm-pro.dts`) |
| Verdin iMX95 on Mallow | 4/8/16 GB | Verdin DSI | none: ours (section 4) |

### 1.4 The display path (survey)

The depth is `driver/docs/imx95-display.md`. What the board needs:

- Mainline Linux has **no** i.MX 95 display support: no display node in
  `imx95.dtsi`, none in the Verdin files (a case-insensitive search for
  `display|dsi|lvds|ldb|hdmi|dpu` in `imx95-verdin*.dts*` finds only the
  I²C and PWM labels), no `imx95` string under `drivers/gpu/drm` or
  `Documentation/devicetree/bindings/display` (L, searches at the commit
  above).
- Toradex's kernel has it. The overlays enable `&display_pixel_link`,
  `&displaymix_irqsteer`, `&dpu`, `&mipi_dsi`, `&pixel_interleaver`, and
  for HDMI a `lontium,lt8912b` bridge at I²C address `0x48` on `lpi2c4`
  with four DSI lanes, driving an `hdmi-connector` (TO:
  `overlays/verdin-imx95_dsi-to-hdmi_overlay.dts:21-139`). That is
  Toradex's "Verdin DSI to HDMI Adapter" (U-Boot's carrier ID
  `VERDIN_DSI_TO_HDMI_ADAPTER = 157`, TU: `tdx-cfg-block.h:174-176`).
  Other overlays: the 7- and 10-inch DSI panels, a DSI-to-LVDS adapter,
  and LVDS through a mezzanine (TO: the `verdin-imx95_*` file list).
  Yocto applies `verdin-imx95_dsi-to-hdmi_overlay.dtbo` by default
  (TL: `verdin-imx95.conf:107`).
- The SM grants the display to AP-NS: `BLK_CTRL_DISPLAYMIX`, `DC` and its
  units, `LVDS`, `MIPI_DSI`, `LDB_PLL`, `CLOCK_DISP1PIX` (SMT: cfg :584,
  596, 609, 627-643, 804, 807), and `PD_DISPLAY` with full rights (SMT:
  `configs/verdin-imx95/config_scmi.h:459`).
- Public code for the controller: NXP's BSD-3 MCUX `dpu_1`, `ldb_1` and
  `mipi_dsi2_dwc` (HAL), and Zephyr's M7-only `socionext,dpu` and
  `nxp,mipi-dsi-dwc` drivers (Z: `drivers/display/
  display_socionext_dpu.c`, `drivers/mipi_dsi/dsi_nxp_dwc.c:108`).

### 1.5 The boot chain

The pieces and their order, from Toradex's machine configuration and NXP's
mkimage:

```
Boot ROM (Cortex-M33)
  └─ container 1 (flash.bin, mkimage target flash_a55):
       ELE firmware (AHAB container)
       OEI on the M33: DDR training with the LPDDR4x firmware v202409
       System Manager (m33_image.bin = Toradex's m33_image-verdin-imx95)
       A55 U-Boot SPL, loaded to 0x20480000 (OCRAM)
  └─ container 2 (u-boot-atf-container.img, appended):
       TF-A BL31 at 0x8A200000 (+ OP-TEE when present)
       U-Boot proper at 0x90200000
SM: starts LM2 "AP" (A55 core 0), then serves SCMI for the rest of the run
SPL → BL31 → U-Boot (non-secure) → bootflow scan over mmc1, mmc0, dhcp
```

Sources: MK: `iMX95/soc.mak:43-44` (`ATF_LOAD_ADDR ?= 0x8A200000`,
`UBOOT_LOAD_ADDR ?= 0x90200000`), :42 (`SPL_LOAD_ADDR_M33_VIEW ?=
0x20480000`), :396-400 (`flash_a55`: `-append $(AHAB_IMG) -c $(OEI_OPT_M33)
… -m33 $(MCU_IMG) … -ap $(SPL_A55_IMG) a55 …`, then
`append_container,u-boot-atf-container.img`), :292 (the second
container: `bl31.bin u-boot-hash.bin`, OP-TEE when `$(TEE)` exists).
TL: `verdin-imx95.conf:57-58` (`IMXBOOT_TARGETS = "flash_a55"`,
`IMX_BOOT_SOC_TARGET = "iMX95"`), :68-81 (OEI `imx-oei-toradex`, board
`toradex-verdin-imx95`, `DDR_TYPE lpddr4x`, firmware `_v202409`), :85-94
(the SM), :65-66 (TF-A `imx95`, `ATF_SYS_PWR_FULL_CTRL = "1"`), :42-55
(U-Boot 2024.04, entry and DTB addresses), :19 (`optee` in the machine
features). The SM's own description of its role (SMT: `README.md:1-8`):
it is the boot core's application, starts the other cores, then "enters a
service mode" serving SCMI.

The logical machines, in Toradex's SM configuration (SMT:
`configs/verdin-imx95.cfg`):

| LM | Name | Boot order | SCMI agents and their mailbox | Source |
| --- | --- | --- | --- | --- |
| LM0 | "SM" | 1 | none (`rpc=none`) | :115 |
| LM1 | "M7" | 2, `skip=1` | agent 0 "M7" on MU 9 | :342, :371-372 |
| LM2 | "AP", `default` | 3, `skip=1` | agent 1 "AP-S" (secure, TF-A) on MU 1; agent 2 "AP-NS" on MU 3 | :443, :496-497, :570-571 |

The SM's MU numbers index its `MU_BASE_ADDRS` table, A and B side in turn:
1 is MU1's B side (`0x44230000`, so the A55 uses `0x44220000`), 3 is MU2's
B side (`0x445C0000`, so the A55 uses `0x445B0000`) (SMT:
`devices/MIMX95/MIMX95_MU.h:809-821, 951`; the generated channels
`configs/verdin-imx95/config_mb_mu.h:82-100`). Linux's SCMI node uses
`&mu2` with shared memory at `0x445b1000` (L: `imx95.dtsi:338-340,
1590-1616`), and so does Zephyr's (Z: `dts/arm64/nxp/nxp_mimx95_a55.dtsi:
96-107, 486-492`). **A Zephyr image in place of Linux is agent 2,
"AP-NS"**, the agent that holds the GPU and the display (section 1.2,
1.4). NXP's EVK configuration has the same layout (SM:
`configs/mx95evk.cfg:496-497, 564-565`), and NXP's tree has no Verdin
configuration (SM: `configs/` holds `mx95alt`, `mx95evk`, `mx95frdm` and
the 952/94/93x ones).

What AP-NS may touch in memory (SMT: cfg :1035-1038): the GPU window
`0x4D900000`..`0x4DD7FFFF`; DDR `0x80000000`..`0x87FFFFFF` (data),
`0x88000000`..`0x89FFFFFF` and `0x8E000000`..`0x87FFFFFFF` (execute). The
hole `0x8A000000`..`0x8DFFFFFF` is AP-S's (TF-A and OP-TEE, :547), and
U-Boot's `PHYS_SDRAM` starts above it at `0x90000000`.

Reset and watchdog, which decide how an image of ours ends:

- PSCI `SYSTEM_RESET` from the A55 resets only the AP logical machine in
  NXP's TF-A. Toradex patches TF-A with `SYS_PWR_FULL_CTRL`, so that PSCI
  uses the SM's full system reset (`IMX9_SCMI_SYS_PWR_FULL_RESET
  0x80000002`), and turns it on for this machine (TL:
  `recipes-bsp/imx-atf/files/0001-feat-imx9-add-full-system-power-control-
  option.patch:1-35`; `imx-atf_%.bbappend`; `verdin-imx95.conf:66`). Our
  images end in a PSCI reset (repo: `docs/standalone-boot.md:276-277` for
  the i.MX8MP), so on this board that is a whole-board reset, as on the
  other Verdins.
- AP-NS owns `WDOG3` and `WDOG4` (SMT: cfg :898-899). The SM's reaction to
  a WDOG3 fault is `sys_reset`, to WDOG4 `lm_reset` (:552-553). Zephyr's
  EVK board arms WDOG4 (Z: `boards/nxp/imx95_evk/
  imx95_evk_mimx9596_a55.dts:16-18, 129-131`). An LM reset restarts the
  A55 side only, and whether it comes back through U-Boot to Linux is not
  read (U4). Our image should arm WDOG3, as Linux does on the Verdin
  (L: `imx95-verdin.dtsi:747-751`), so a hang ends in the same full reset
  that the i.MX8MP's WDOG1 gives (repo: `docs/standalone-boot.md:270-277`).
  It does: section 1.6.

What can start our image, and which route signs off:

| Route | What starts the image | Status here |
| --- | --- | --- |
| **U-Boot from the eMMC** (sign-off) | Toradex's U-Boot: `ext4load mmc 0:1 <addr> <file>; dcache off; icache off; go <addr>` | all commands present: `CMD_GO` and `CMD_ELF` default y (TU: `cmd/Kconfig:475-477, 501-503`), `CONFIG_CMD_CACHE=y`, `CONFIG_CMD_EXT4_WRITE=y`, `CONFIG_CMD_MMC=y` (TU: `configs/verdin-imx95_defconfig:77, 83, 90`); eMMC is `mmc 0` (`update_uboot`'s `mmc dev 0 1`, TU: `verdin-imx95.env:18`) |
| U-Boot over TFTP (development) | the same hook's `yako_net` | as on the other Verdins |
| Direct (TF-A → Zephyr as BL33) | our image in place of U-Boot in container 2 | the i.MX8MP's design (`docs/notes/imx8mp-direct-boot.md`) carries over in form; on the i.MX 95 the container is mkimage's `u-boot-atf-container.img` (MK: `soc.mak:292`). Designed in `docs/notes/imx95-direct-boot.md`. Later, after sign-off |
| USB serial download | `uuu`/SDP into SPL (`CONFIG_SPL_USB_SDP_SUPPORT=y`, TU: `configs/verdin-imx95_defconfig:179`) | recovery only |

Zephyr's own route for the EVK, a `flash.bin` whose TF-A starts Zephyr
with no U-Boot (Z: `boards/nxp/imx95_evk/doc/index.rst:121-129`), is that
direct route built with NXP's EVK firmware. It does not apply to the
Verdin as it is (Toradex's OEI and SM differ).

### 1.6 The watchdog

Which watchdog the A55 image arms, and what its timeout does. Sources: SMT
(the configuration and the board code), L, Z, mainline U-Boot `ece349ade297`
(`UB:`, the revision of the U-Boot files in the board's directory; TU is the
U-Boot the module ships, its watchdog driver not compared).

- **Who owns what.** The "A55 non-secure EENV" section of
  `configs/verdin-imx95.cfg` (SMT, agent `AP-NS`, the one U-Boot, Linux and
  our image talk to) lists `WDOG3 OWNER` and `WDOG4 OWNER` (:898-899). The
  logical machine `LM2 name="AP"` (:443) carries the faults: `FAULT_WDOG3
  OWNER, reaction=sys_reset` and `FAULT_WDOG4 OWNER, reaction=lm_reset`
  (:552-553; generated: `configs/verdin-imx95/config_lmm.h:221-222`). The
  `WDOG_ANY` pin is the SM's own (`PIN_WDOG_ANY OWNER` in LM0, :320).
- **What a WDOG3 timeout does.** The SM takes WDOG3's interrupt as the fault
  (`devices/MIMX95/sm/dev_sm_handlers.c:396-399`, `WDOG3_IRQHandler` ->
  `FaultHandler(DEV_SM_FAULT_WDOG3)`), so the WDOG3 must be armed with
  `CS.INT` set: Linux sets it for `fsl,ext-reset-output` (L:
  `drivers/watchdog/imx7ulp_wdt.c:292, 337`, at torvalds/linux `4dd10fbb`),
  which the Verdin's `wdog3` has, and U-Boot sets it on every i.MX 9 (UB:
  `drivers/watchdog/ulp_wdog.c:96-101`). The reaction `sys_reset` is
  `LMM_SystemReset` (`sm/lmm/lmm_fault.c:79-80`): every LM is notified, the
  reason is saved (`DEV_SM_SystemShutdownRecSet`, `sm/lmm/lmm_sys.c:267`),
  and `SM_SYSTEMRESET` runs (:275), which on Toradex's boards is
  `BRD_SM_SystemReset` (`boards/tdx-verdin-imx95/sm/brd_sm.h:76`): it drives
  `WDOG_ANY` low as a GPIO "to reset PMIC", waits 2 ms, and falls back to the
  SoC's warm reset, the M33's `SYSRESETREQ` (`boards/tdx-common/sm/
  brd_sm_tdx_common.c:480-504`; `devices/MIMX95/sm/dev_sm_system.c:172-179`,
  `drivers/fsl_reset.c:675-679`). So the whole board, M33 and SM included,
  starts again as from power-on: ROM, OEI, SM, SPL, TF-A, U-Boot, and
  U-Boot's `bootcmd`, which is Linux. Nothing of the image's SCMI requests
  survives: the SM's power and clock votes (the GPU's power domain and
  clocks our image turns on, the display's later) live in the SM's RAM,
  which restarts, and the PMIC reset drops the rails. The board returns to
  its normal boot without help, as the Verdin iMX8M Plus does through
  `WDOG_B`.
- **Why not WDOG4.** Its reaction is `lm_reset` of the AP machine
  (`LMM_SystemLmReset`, `sm/lmm/lmm_sys.c:641`): the M33 and the SM keep
  running, and which A55 boot path, and which votes, an LM reset leaves is
  not read (U4). Zephyr's `imx95_evk` arms WDOG4 (Z:
  `boards/nxp/imx95_evk/imx95_evk_mimx9596_a55.dts:17, 129`), which suits an
  EVK with no Linux to return to, not this board.
- **Why not the SM.** SCMI has no watchdog protocol, and NXP's vendor
  protocols in the SM (`sm/rpc/scmi/`) have none either. AP-NS has the
  system protocol for notifications only (`SYS NOTIFY`, :603; AP-S, TF-A, has
  `SYS ALL`, :524), so even a reset request goes through PSCI to TF-A, as
  our images' normal end already does (section 1.5). `LMM_1 ALL` (:597) is
  the M7's machine, not the A55's.
- **What Linux uses.** `&wdog3 { fsl,ext-reset-output; status = "okay"; }`
  (L: `imx95-verdin.dtsi:747-751`; the node, `fsl,imx93-wdt` at
  `0x42490000`, `imx95.dtsi:791-799`, `timeout-sec = <40>`). Mainline U-Boot
  takes the same node (`bootph-all`; `CONFIG_DEFAULT_DEVICE_TREE=
  "freescale/imx95-verdin-wifi-dev"`, `CONFIG_WDT=y`, `CONFIG_ULP_WATCHDOG=y`,
  `CONFIG_WATCHDOG_TIMEOUT_MSECS=30000`, `CONFIG_SPL_WATCHDOG=y`: UB
  `configs/verdin-imx95_defconfig:13, 26, 68, 185-186`), and the driver
  model starts it unless `WATCHDOG_AUTOSTART` is off (not set there; default
  y, UB `drivers/watchdog/Kconfig:14-20`). `go` does not stop it (UB
  `cmd/boot.c` has no watchdog call). So WDOG3 is probably already counting U-Boot's
  30 s when our image starts; the image does not rely on it.
- **The counter.** The LPO (32 kHz) through the 256 prescaler: 125 ticks a
  second, `TOVAL` 16 bits, at most 524 s (L: `imx7ulp_wdt.c:408-411`,
  `imx93_wdt_hw`; UB: `ulp_wdog.c:210`). Zephyr's `nxp_mimx95_a55.dtsi`
  describes the same (Z: :175-197, `clk-source = <1>`, `clk-divider =
  <256>`).

What the image does (`driver/zephyr/lib/board/verdin_imx95/
verdin_imx95_wdog.c` over `driver/rust/src/ip/wdog32.rs`): at
`PRE_KERNEL_1` 0, before any other init hook and with interrupts off, it
unlocks WDOG3 in whichever command width it finds, writes Linux's `CS`
(32-bit commands, LPO, prescaler, `UPDATE`, `WAIT`, `STOP`, `INT`) with `EN`
and `TOVAL` for 60 s, refreshes it and reads both back; a refusal resets
through PSCI at once. A 1 s timer refreshes it until the lifetime
(`CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S`) and while the board layer's alive
window holds; the normal end stays the PSCI `SYSTEM_RESET` (section 1.5).
Zephyr's own driver for the node (`wdt_mcux_wdog32`) is not used: it arms at
`POST_KERNEL` only (its setup sleeps, Z: `drivers/watchdog/
wdt_mcux_wdog32.c:84, 111, 289`), and it sets `CS.INT` only with a callback
whose ISR on the A55 clears the flag the SM's fault is (:174, 205).

What only the board can say: that WDOG3's timeout reaches the SM's fault
with `INT` set and resets the board through `WDOG_ANY` (a proof run that
arms and never feeds); whether U-Boot left WDOG3 running at `go` (the armed
line's "was running" and `TOVAL` found, 0xea6 for U-Boot's 30 s); the
`ULK`/`RCS` poll counts and the arming's time; and that Linux comes back
after the bite.

## 2. The open-source stack (survey)

- Kernel: Panthor drives the G310 through the generic compatible (section
  1.2). There is no i.MX 95 code in it.
- Firmware: Panthor requests `arm/mali/arch%d.%d/mali_csffw.bin` from
  `GPU_ID` (L: `panthor_fw.c:811-814`) and declares `arch10.8`, `arch10.10`
  and `arch10.12` among others (:1559-1561). linux-firmware's three arch-10
  files are **the same bytes**: 282624 bytes each, sha256
  `a27847ea11f8efb3136340c3ba8aab413ae25145eeb4a7f64ff5edd829a2405b` for
  `arch10.8`, `arch10.10` and `arch10.12` (FW: `sha256sum
  arm/mali/arch10.*/mali_csffw.bin`; `WHENCE:11638-11655`, "Redistributable.
  See LICENCE.mali_csffw"). That is the hash we pin for the T6 (repo:
  `tools/cts/deps.toml:316-317`), whose interface our loader already speaks
  (1.5.0, `driver/rust/src/gpu/mali/fw.rs:102-103`). Toradex's layer
  installs stock linux-firmware (TL: `recipes-kernel/linux-firmware/
  linux-firmware_%.bbappend:1-3`, version `20250311`).
- Mesa: panfrost and panvk know the G310 (section 1.2). The comparison is
  `docs/notes/mali-g310-vs-g610.md`.
- Display: section 1.4.

## 3. What yakogl already has that applies

### 3.1 The GPU side (survey)

The Mali back end, compiler and kernel side exist for the T6's G610. What
is keyed to the G610 or the RK3588 today (repo, at `1d9022e7`):

| Where | What |
| --- | --- |
| `driver/rust/src/gpu/mali/mod.rs:97-98`, stop at `driver/rust/src/board/nanopc_t6_gpu.rs:387-388` | `G610_GPU_ID 0xa867_0005`, an exact match |
| `driver/rust/src/gpu/mali/pgtable.rs:30-35`, check `driver/rust/src/board/nanopc_t6_mmu.rs:172` | VA 48 / PA 40 as constants |
| `driver/rust/src/gpu/mali/mod.rs:93-95, 317`, `nanopc_t6_mmu.rs:38` | non-coherent: `COHERENCY_NONE` and a `MEMATTR` word copied from the T6 |
| `driver/rust/src/board/nanopc_t6_submit.rs:302-304` | heap pool for a 64-byte L2 line |
| `src/mali/frame.rs:267-268, 280-283, 2620` | tiler levels must be 8; tile budget 32768 / 2 |
| `driver/rust/src/soc/rk3588.rs`, `src/osal/zephyr_mali.c:75-84`, `zephyr/Kconfig:76-79` | the RK3588 clock, power, PMIC and QoS layer; `YAKOGL_GPU_MALI` depends on the T6 |

Their replacement is the work of the kernel-side change,
`docs/notes/imx95-gpu-power.md` and `docs/notes/mali-g310-vs-g610.md`. The
two facts this note adds: the firmware needs nothing (section 2), and the
power-on needs no PMIC step (section 1.1).

### 3.2 What the two Verdins already give the Verdin iMX95

The Verdin AM62P and the Verdin iMX8M Plus share the carrier family, the
Torizon images and the U-Boot-from-eMMC sign-off (`docs/standalone-boot.md`,
`tools/board.sh`). What carries over, item by item:

| Piece | Carries over? | What changes for the iMX95 |
| --- | --- | --- |
| The U-Boot hook (`yako_hook`, `yako_dflt`, `yako_mmc`, `yako_net`; `tools/uboot-netboot-lib.sh:49-54`) | yes, as strings | `NB_MMC=0:1` (eMMC `mmc 0`) and `MMC_PART=/dev/mmcblk0p1`, as on the AM62P (repo: `tools/board.sh:357-363`), to be confirmed on the board with `findmnt -T /var/yakogl` as `board.sh` already does (`docs/standalone-boot.md:133-137`) |
| `ext4load` + `dcache off; icache off; go` | yes | the commands are in Toradex's build (section 1.5) |
| Torizon's `/var/yakogl` staging and the env from Linux (`fw_setenv`) | yes, if the board runs Torizon | Toradex's layer builds Tezi images (TL: `verdin-imx95.conf:99-105`); the env lives in the eMMC boot partition at `CONFIG_ENV_OFFSET=0xFFFFDE00`, `CONFIG_SYS_MMC_ENV_PART=1` (TU: `configs/verdin-imx95_defconfig:11, 96`) |
| The one-shot and the counted default | yes | none |
| `board.sh <board>` entry (`BOOTS`, `NB_MMC`, `YAKO_ADDR`, `LOAD_END`, `MARKER`, record sink) | new entry | `YAKO_ADDR` must lie in AP-NS's executable DDR and outside U-Boot, TF-A and the M7's reservations (section 1.5): for example `0xD0000000`, the EVK board's link address (Z: `imx95_evk_mimx9596_a55.dts:50-52`), which is below 2 GB above DRAM start on every SKU |
| The image arms its own watchdog (`docs/standalone-boot.md:255-277`) | yes, as a rule | WDOG3 (system reset), not WDOG4 (section 1.5); whether U-Boot leaves a watchdog running is a B1 read |
| The result without a console (the i.MX8MP's SNVS record, `driver/rust/src/board/record.rs`) | the idea and the format, not the block | the BBSM is the SM's (SMT: cfg :211); AP-NS owns the BBSM GPRs 4..7 (cfg :756-759; `devices/MIMX95/configtool/bbsmmix.cfg:45-48`), reachable through SCMI. Built (H4): the same four record words in GPR4..7 over BBM_GPR_SET, read from Linux through the kernel's SCMI raw interface (`docs/standalone-boot.md`, "The record without a console"). Whether they survive the full reset, and whether the board's kernel has that interface, is U2 |
| The direct boot (`docs/notes/imx8mp-direct-boot.md`) | the design | the container layout differs (section 1.5) |
| `driver/boards/toradex/` board pattern (an `extend` of a Zephyr board, `driver/boards/toradex/verdin_imx8mp/board.yml:13-17`) | the pattern | no Zephyr board to extend (section 4): a full board over the pin's SoC |

## 4. The board on Zephyr (survey)

- At the pin and on upstream `main`, `boards/toradex/` has `colibri_imx7d`,
  `verdin_am62`, `verdin_imx8mm`, `verdin_imx8mp` and no `verdin_imx95`
  (Z, ZM: directory listings). `boards/nxp/` has `imx95_evk`,
  `imx95_evk_15x15` and `imx952_evk` and no FRDM i.MX 95 (ZM). Neither
  tree has a GPU node for the i.MX 95 A55 (ZM:
  `dts/arm64/nxp/nxp_mimx95_a55.dtsi`, no `gpu`, `mali` or `dpu`).
- So the board is ours, in `driver/boards/toradex/`, over the pin's SoC
  support: `SOC_MIMX9596_A55`, `nxp_mimx95_a55.dtsi` (six A55, GIC-v3, SCMI
  over `mu2`, the watchdogs, LPUARTs; Z: :22-133, 175-197, 468-492) and the
  SoC init that powers a mix and sets clocks over SCMI (Z:
  `soc/nxp/imx/imx9/imx95/a55/soc.c:62-99`). The board brings its
  console (`lpuart1`), its memory (section 3.2), WDOG3, and its pin
  control. The board and its build-only gate row are another change's work.
- Zephyr's SCMI clock driver returns success without acting when the SM
  denies state control (Z: `drivers/clock_control/
  clock_control_arm_scmi.c:51-59`). Our GPU path calls the SCMI layer and
  checks (depth: `docs/notes/imx95-gpu-power.md`).

## 5. What can be done before the board arrives

Items the five parallel documents do not cover:

| # | Item | Check |
| --- | --- | --- |
| H1 | The firmware: record in `tools/cts/deps.toml` that `MALI_CSFFW` serves the G310 too (the arch 10.12 file is the same bytes, section 2), so no second entry is needed | the note in `deps.toml` cites FW's commit and hash; `driver/tools/mkmalifw.sh`'s header unchanged |
| H2 | `tools/board.sh` entry `imx95` (or `verdin-imx95`): `BOOTS="uboot-tftp uboot-mmc"`, `NB_MMC=0:1`, `MMC_PART=/dev/mmcblk0p1`, `YAKO_ADDR`/`LOAD_END` from the board's DTS, `MARKER` of the WDOG3 self-arm, the record sink from H4 | `tools/board_selftest.sh` passes with the entry (it checks `board.sh` without a board). Done: `tools/board.sh imx95` (`uboot-tftp` the default, `uboot-mmc`; `0xd0000000`..`0xe0000000`, the DTS's `zephyr,sram`; `MARKER='verdin_imx95: WDOG3 armed'`; the record `bbm.txt` through `tools/imx95_bbm_record.sh`, or the console with `BOARD_UART`; install and uninstall of the other Verdins' hook); the selftest runs every verb on it and replays its verdicts |
| H3 | WDOG3 self-arm for the image (the i.MX8MP's `gc_wdog_arm` pattern, `docs/standalone-boot.md:270-277`), with PSCI reset as the refusal path | a build of the board image that links it; the arming line is the `MARKER`. Built: section 1.6; the image prints `board: watchdog armed` |
| H4 | The record without a console: the SCMI BBM GPR calls for GPR4..7 from Zephyr, and their read from Linux (Linux has `scmi_bbm`, L: `imx95-verdin.dtsi:29, 623`) | a design paragraph in `docs/standalone-boot.md` and a host test of the record format over a mocked SCMI; the board check is B3. Done: `soc::imx95::bbm` (the messages; `bbm_tests.rs` over the mock SM with AP-NS's GPR rights), the sink `verdin_imx95_record.c` (`CONFIG_YAKOGL_VERDIN_IMX95_RECORD_BBM`), the reader `tools/imx95_bbm_record.sh` (its selftest against a fake SM). Linux's BBM driver exports no GPR (only the RTC and the button), so the reader needs `CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX` in the board's kernel: a B1 read |
| H5 | The direct-boot container: how to replace `u-boot-hash.bin` in mkimage's second container with our loader (MK: `soc.mak:281-300`), against Toradex's OEI and SM | designed: `docs/notes/imx95-direct-boot.md` (the chain, the options, the fallback, the board steps D1-D6); `tools/imx95-mkboot.py` builds and checks the image, `tools/imx95_mkboot_selftest.py` tests it; nothing runs until B7 |

## 6. What needs the board

Day one, in order:

| # | Try | Done when |
| --- | --- | --- |
| B1 | At Toradex's U-Boot prompt (a person or a bridge on X11): `version`, `fuse read 2 2 2` (GPU, DSI and LVDS presence, section 1.1), `printenv` (the env to back up before the hook, `bootcmd`, the watchdog state), `mmc list`, `bdinfo`. Then boot the stock Linux once and record `/proc/iomem`, `/proc/interrupts`, Panthor's probe lines and the firmware it loads, `findmnt -T /var` | the module's fuses, DRAM and env recorded in the board's issue; U-Boot's commands confirmed |
| B2 | Our image through the hook (`BOARD_BOOT=uboot-mmc board.sh … run`): SCMI base protocol (agent name "AP-NS", protocol list), `PD_GPU` on, the GPU clocks, then **the GPU identity read by our code** (`mali::ident`) | `GPU_ID`, `CORE_FEATURES`, `SHADER_PRESENT`, `L2_FEATURES`, `TILER_FEATURES`, `MMU_FEATURES`, `COHERENCY_FEATURES` printed and stored in the record |
| B3 | The firmware (the T6's Z3 path with the same file) | **the firmware's first answer**: the GLB version (1.5.0 expected, since the bytes are the T6's) and the CS features word |

The images for B1 to B3, each with one verdict line, and B1's Linux reading
are in `tools/probes/verdin_imx95/` (its README: `imx95_wdog` first, then
`imx95_scmi`, `imx95_ident`, `imx95_fw`; `linux_day_one/` for B1's Linux
half).

Then the measurements that say the port is alive:

- M1 the first submitted frame read back (the T6's Z4).
- M2 the first `gl_test` case on read-back, then the 50 (the T6 reached
  50/50 on read-back with no display, repo: `tools/cts/STATUS.md:17`).
- M3 the first clear on the Mallow's DSI panel (after
  `driver/docs/imx95-display.md`).
- M4 the first triangle on that panel.
- M5 sign-off: the same image started by U-Boot from the eMMC as the
  product form (`default <image> N`), then the direct route (B6).

## 7. Risks and unknowns

| # | Question | What answers it |
| --- | --- | --- |
| U1 | Is the SM on the lab's module built from Toradex's `verdin-imx95` configuration as read here (the Yocto pin `cc7a8d2d` or later; the configuration is the same at both), or an older build with other rights? | B1: the SM's version line on its console (UART_4), or SCMI base `DISCOVER_AGENT` from our image (B2) |
| U2 | Is there a reset-surviving record for a console-less run? The BBSM GPRs 4..7 belong to AP-NS through SCMI; do they survive a WDOG3 system reset, and can Linux read them? | H4 (built): Linux reads them only through the SCMI raw debugfs interface, which neither `imx_v8_defconfig` nor `defconfig` of Toradex's kernel tree enables (`CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX`; linux-toradex `toradex_6.18-2.1.x-imx` 851ac0f9; Torizon's own kernel fragments not read); B1 reads the board's kernel (`board.sh imx95 status`), B3 the survival. Until then a person or a bridge on X11 (`BOARD_UART`) |
| U3 | Which i.MX 95 part does Toradex fit, and is any SKU fused without the GPU or the DSI? | Toradex's data sheet (not read); B1's `fuse read` on the lab's module |
| U4 | What does an LM reset (WDOG4, or PSCI without Toradex's patch) do on this board: does the A55 side come back through U-Boot to Linux? | not needed if the image uses WDOG3 and PSCI full reset; else B-day test with a person |
| U5 | Who releases `GPURESET` on the i.MX 95, given that the A55 cannot write the GPU blk-ctrl and the SM does not write it? | `docs/notes/imx95-gpu-power.md`; B2 |
| U6 | The G310's configuration registers (variant, cores, tiler levels, coherency) | B2; their consequences are `docs/notes/mali-g310-vs-g610.md` |
| U7 | Does the lab's Mallow have a serial bridge on X11, and is the 10-inch DSI panel free for this module? | the farm (`ask_farm`), not a document |

## 8. Proposed items, ordered

The finished work to size against: the T6's own kernel side went from Z1
(2026-10-19) to Z4 (2026-11-12) and `gl_test` 50/50 on Zephyr
(2026-11-16) (repo: `tools/cts/STATUS.md:17`); the Verdin AM62P and
iMX8M Plus standalone boot ran on 2026-10-05 (`docs/standalone-boot.md:
19-39`). The GPU code is shared, the boot tooling is shared, and the
firmware is the same file. The new parts are SCMI, the display and the
console-less record.

Host items:

| # | Item | Size | Depends on | Done when |
| --- | --- | --- | --- | --- |
| P1 | The Zephyr board in `driver/boards/toradex/` and its build-only row (parallel change) | S | — | gate row green |
| P2 | Kernel side from registers, G310 mock (parallel change) | M | — | host tests pass for both identities |
| P3 | SCMI GPU power with mock (`docs/notes/imx95-gpu-power.md`, parallel) | S | — | sequence pinned in a test |
| P4 | Mesa comparison (`docs/notes/mali-g310-vs-g610.md`, parallel) | S | — | the difference table |
| P5 | H1 firmware note in `deps.toml` | XS | — | committed |
| P6 | H3 WDOG3 self-arm (section 1.6) | S | P1 | builds; arming line defined (done: the gate row requires it) |
| P7 | H4 record over SCMI BBM GPRs | S | P1 | host test of the format; design in `docs/standalone-boot.md` (done: `bbm_tests.rs`, `tools/imx95_bbm_record_selftest.sh`; the board's kernel interface is U2) |
| P8 | H2 `board.sh` entry | S | P6, P7 | `tools/board_selftest.sh` green (done: `board.sh imx95`) |
| P9 | Display design (`driver/docs/imx95-display.md`, parallel) | M | — | the document and its probe's build row |

Board items:

| # | Item | Size | Depends on | Done when |
| --- | --- | --- | --- | --- |
| B1 | Read the module at U-Boot and under its Linux; install the hook | S | the board in the farm | fuses, env, iomem recorded; `board.sh … install` ok |
| B2 | Identity and power from our image | S | P1-P3, P6, P8, B1 | `mali::ident` stored in the record |
| B3 | Firmware boot; first record read back from Linux | S | B2, P7 | GLB answer and the record |
| B4 | First frame (Z4 analogue) | M | B3, P2, P4 | read-back pixels as predicted |
| B5 | `gl_test` on read-back to 50/50 (the image builds: `driver/samples/zephyr/gl_test` for `verdin_imx95/mimx9596/a55`, panel off, gate row `zephyr  verdin_imx95 gl_test image`) | M | B4 | row in `tools/cts/RESULTS.md` |
| B6 | Display bring-up on the Mallow's DSI panel | L | P9, B2 | first clear, then the first triangle |
| B7 | Sign-off: `default <image> N` from the eMMC, then the direct route (steps D1-D6 of `docs/notes/imx95-direct-boot.md`) | S | B5 | both routes recorded |
| B8 | dEQP-GLES2 tree as on the T6 (the images build: the T6's `mali-gles2-tree.txt` rows by `tools/cts/imx95-deqp-images.sh`, stage 1 `deps-env.sh --build DEQP_WORK --board verdin_imx95`) | L (shared compiler work) | B5 | the tally in `RESULTS.md` |

Order logic: B1 comes first on the board because the fuses and the SM
version (U1, U3) can turn the plan, and it needs nothing of ours. P6-P8
are the only host items that gate B2 beyond the parallel work: without
the watchdog and a record, a console-less board strands on the first
hang. B6 does not block B5: as on the T6, read-back carries the GL work
until the display exists.

Host state of B5 and B8 (2026-11-21): both images build with
`-DYAKOGL_WERROR=ON` and wait for the board. gl_test is the T6's sample and
main (`src/main_mali.c`) with the board layer's files and
`gl_test/verdin_imx95.conf` (X4 panel off unless
`-DCONFIG_YAKOGL_VERDIN_IMX95_DISPLAY=y`): RAM footprint 88.8 MiB of the
256 MiB at 0xd0000000, `zephyr.bin` 4.4 MiB. A dEQP-GLES2 row image (the
T6's rows as they are; T6S1R08, T6S4R01 and T6S1R06 built) takes 191.0 MiB
with a 64 MiB GL carve-out plus the window memory, `zephyr.bin` 12.1 MiB:
the memory layout holds it unchanged. What the run script has to provide
(start, end lines, bounds, the record hook) is in `tools/cts/README.md`,
section 5; the record over the BBM GPRs (P7) plugs into the board layer's
`board_sink_store()`, which both images reach through `board_finish()`.
The G310's own limits reach neither image's expectations: gl_test and dEQP
read every limit through GL. What still carries a G610 constant is the back
end (`src/mali/frame.rs` `TIB_BUDGET` and `MEASURED_TILER_LEVELS`,
`src/mali/backend.rs`'s renderer name), `docs/notes/mali-g310-vs-g610.md`
section 5.
