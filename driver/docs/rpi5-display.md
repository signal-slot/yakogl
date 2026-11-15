<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5 (BCM2712): the picture on HDMI from Zephyr

Status: design (2026-11-14). No Pi 5 display code exists; nothing here was run
on a board. The goal is the Pi 4's screen path on the Pi 5: a Zephyr image
started with `kernel=` puts a picture on HDMI0 (text on top, a pattern or the
GL window below), so that `gl_test`, dEQP and glmark2 runs on the Pi 5 have
the console screen and the scan-out view the Pi 4 has
(`driver/zephyr/lib/board/rpi4/board_rpi4.c`). This note is item P11 ("Display
(U3)") of `docs/notes/pre-study-rpi5.md`, section 8. It names three routes in
order of cost, what each needs, and the board checks that decide among them.
The hardware facts it rests on are `driver/docs/rpi5-v3d71-facts.md` sections
9, 10 and 13 (the facts file); this note adds what those sections did not read.

## Sources and rules

| Prefix | Tree | Commit |
|---|---|---|
| `rpi:` | `https://github.com/raspberrypi/linux.git`, branch `rpi-6.12.y` | `5bd564aaa8dc4b1b8ed11733c19ec8ece63a72cc` (the facts file's commit; a depth-1 sparse checkout of the cited directories in the dependency cache) |
| `repo:` | this repository | `cf9dc425` |

The Linux files are GPL-2.0: only facts and `file:line` citations are kept
here, never code, never a comment and never a constant table
(`docs/source-form.md`). Every `rpi:` line number was read with `grep -n` or
`sed -n` on that checkout. All `vc4_*` paths are under `drivers/gpu/drm/vc4/`,
all `bcm2712*` device-tree paths under `arch/arm64/boot/dts/broadcom/`, and
`raspberrypi-firmware.h` is `include/soc/bcm2835/raspberrypi-firmware.h`.
Where the facts file already cites a line, this note repeats it only when the
argument needs it; those lines were re-read and agree.

## 1. What the Pi 4 does today, and what must carry over

| Pi 4 step | Where | Citation |
|---|---|---|
| Ask the firmware for a 32 bpp framebuffer through the property mailbox: physical size, virtual size, depth 32, pixel order RGB, virtual offset 0,0, allocate (4096 alignment), get pitch, in one request | `rpi_fb_alloc_virtual`, `rpi_fb_alloc` | `repo:driver/zephyr/lib/board/rpi4/rpi_mbox.c:165-225,251-294` |
| The answer's bus address becomes the ARM address (`& ~0xc0000000` on the BCM2711, `& ~0` on the BCM2712); the buffer is mapped Normal non-cacheable | same | `rpi_mbox.c:42-51,207,222,287-291` |
| The pixel order the firmware answered picks the window format (RGB: RGBA8888, BGR: XRGB8888); a window is a view into the framebuffer when it fits; a half of a double-height buffer is shown by a pan (`SET_VIRTUAL_OFFSET`) | the pure decisions and the C that applies them | `repo:driver/rust/src/board/rpi4.rs:28-91`; `repo:driver/zephyr/lib/board/rpi4/board_rpi4.c:212-274,281-322` |
| The console draws `printk` into the framebuffer with an 8x16 font; it moves beside or under the window | `fbcon.c`, `console_beside` | `repo:driver/zephyr/lib/board/rpi4/fbcon.c:67-80`; `board_rpi4.c:197-210` |
| The record screen freezes the last lines until the watchdog resets the board | `freeze_record_screen` | `board_rpi4.c:336-368` |
| The mailbox client is already built for the BCM2712 (base 0x10_7c013880, no bus alias) and its mock runs for both SoCs; the framebuffer calls are "untried on the Pi 5" | | `rpi_mbox.c:22-30,42-51`; `repo:driver/zephyr/lib/board/rpi4/rpi_mbox.h:4-6`; `repo:driver/zephyr/lib/board/test/rpi_mbox_mock.c:9-20` |

What the Pi 5 must keep for the GL path: a linear 32 bpp buffer with a known
pitch at a known ARM physical address below 4 GiB, its format named by the
board layer, a double buffer shown by a pan or a flip, and a console that
shares the screen. Which block puts it on the wire is the board layer's
business alone.

## 2. The Pi 5 chain

```
 RAM (the image's sram0, 0x200000.., below 4 GiB)
  |  AXI, through IOMMU4 (0x10_0000_5200; addresses below its aperture pass through)
  v
 HVS 6 "scaler6" 0x10_7c580000 (0x1a000): display list in its own memory at +0x4000,
   channel 0 (FIFO 0) --> pixelvalve0 0x10_7c410000 (0x100) --> hdmi0 (nine regions, below)
   channel 1 (FIFO 1) --> pixelvalve1 0x10_7c411000        --> hdmi1
 clocks: firmware 4 (core) and 16 (disp) for the HVS; 13 (hdmi, the HSM) and 14 (bvb) for HDMI;
         the DVP block 0x10_7c700000 gates a 108 MHz clock and holds the HDMI resets
```

| Block | ARM address (40-bit), size | Interrupt | Citation |
|---|---|---|---|
| HVS, `brcm,bcm2712-hvs` | 0x10_7c580000, 0x1a000; `iommus = <&iommu4>` | `disp_intr` 2, 9, 16 (`ch0-eof`, `ch1-eof`, `ch2-eof`) | `rpi:bcm2712.dtsi:612-623` |
| `disp_intr` (`brcm,bcm2711-l2-intc`) | 0x10_7c502000, 0x30 | GIC SPI 97 | `bcm2712.dtsi:318-324` |
| `pixelvalve0`, `pixelvalve1` | 0x10_7c410000, 0x10_7c411000; 0x100 each | SPI 101, SPI 110 | `bcm2712.dtsi:292-302` |
| `hdmi0`: `hdmi` 0x10_7c701400 (0x300), `dvp` 0x10_7c701000 (0x200), `phy` 0x10_7c701d00 (0x300), `rm` 0x10_7c702000 (0x80), `packet` 0x10_7c703800 (0x200), `metadata` 0x10_7c704000 (0x800), `csc` 0x10_7c700100 (0x80), `cec` 0x10_7d510800 (0x100), `hd` 0x10_7c720000 (0x100); `resets = <&dvp 1>` | | `aon_intr` 1-3 (CEC), 7-8 (hot-plug) | `bcm2712.dtsi:370-397` |
| `hdmi1`: the same names; `hdmi` 0x10_7c706400, `dvp` 0x10_7c706000, `phy` 0x10_7c706d00, `rm` 0x10_7c707000, `packet` 0x10_7c708800, `metadata` 0x10_7c709000, `csc` 0x10_7c700180, `cec` 0x10_7d511000, `hd` 0x10_7c720000 (the same `hd` block as hdmi0); `resets = <&dvp 2>` | | `aon_intr` 11-15 | `bcm2712.dtsi:399-426` |
| `aon_intr` | 0x10_7d510600, 0x30 | GIC SPI 239 | `bcm2712.dtsi:284-290` |
| `dvp` clock and reset block, `brcm,brcm2711-dvp` | 0x10_7c700000, 0x10; parent clock 108 MHz | | `bcm2712.dtsi:326-330` |
| DVP registers: resets at +0x04 (6 lines, `reset-simple`), clock gates at +0x08 bit 3 (hdmi0 108 MHz) and bit 4 (hdmi1) | | | `rpi:drivers/clk/bcm/clk-bcm2711-dvp.c:10-14,49-52,64,73` |
| IOMMU4, "for HVS, MPL/TXP", `brcm,bcm2712-iommu` | 0x10_0000_5200, 0x80 (under `axi`, two address cells) | | `rpi:bcm2712-ds.dtsi:292-299` |
| Clocks the Pi 5 board file gives: HVS `core` = firmware clock 4, `disp` = 16; HDMI `hdmi` = 13 (`RPI_FIRMWARE_M2MC_CLK_ID`), `bvb` = 14 (`RPI_FIRMWARE_PIXEL_BVB_CLK_ID`), `audio` = DVP gate, `cec` = 27 MHz | | | `rpi:bcm2712-rpi-5-b.dts:276-291` (repeated at `:704-716`); ids `raspberrypi-firmware.h:169,178,179,181` |
| All of it is `status = "disabled"` in the base tree and enabled by the `vc4-kms-v3d-pi5` overlay, which also attaches the `vc4` node to IOMMU4 | | | `rpi:bcm2712-ds.dtsi:436-487`; `rpi:arch/arm/boot/dts/overlays/vc4-kms-v3d-pi5-overlay.dts:12-130` (`&vc4` with `iommus` at `:125-129`) |

Two things in this table were not in the facts file and matter for every route:

- **The HVS is behind an IOMMU.** Linux's driver leaves addresses below its
  IOVA aperture untranslated ("requests to lower addresses pass straight
  through to SDRAM", default aperture base 40 GiB, size 2 GiB;
  `rpi:drivers/iommu/bcm2712-iommu.c:96-100,115-116,182-202`). Its control
  register is `MMMU_CTRL` at +0x00 with `BYPASS` bit 8 and `ENABLE` bit 0,
  the bypass range is +0x1c/+0x20, the address cap +0x14 and `DEBUG_INFO`
  +0x38 (`bcm2712-iommu.c:24-81`). Without Linux the IOMMU is in whatever
  state the firmware leaves it; an image's buffers at 0x200000 are below any
  plausible aperture, but the state is read on the board (check Z0 below)
  before a plane address is trusted as a physical address.
- **The HVS 6 has two register layouts.** `SCALER6_VERSION` (+0x00, bits 7:0)
  reads 0x53 for the C0 stepping and 0x54 for D0; Linux switches to the D0
  layout when it reads 0x54 (`rpi:vc4_regs.h:543-546`, `rpi:vc4_hvs.c:2083-2092`).
  The per-channel registers sit at +0x30 (C0) or +0x100 (D0), see section 3.
  The board's stepping is the first word any Pi 5 display code reads.

## 3. Route A: adopt the firmware's running scanout

The cheapest route if it is open: find the plane the firmware is scanning,
write into its buffer, change nothing else.

### What Linux shows about the firmware's scanout

| Fact | Citation |
|---|---|
| The display-list allocator starts at word 32 (`HVS_BOOTLOADER_DLIST_END`) on every generation, "never overwrite the setup from the bootloader"; on the 6.x HVS the list memory size is read from `SCALER6_CXM_SIZE` | `rpi:vc4_hvs.c:1570-1596` (gen 6 at `:1580-1596`, `:1589`); `rpi:vc4_regs.h:1056` |
| `vc4_hdmi_bind` enables the pixel, HSM and BVB clocks when `HD_VID_CTL.ENABLE` (bit 31) is already set on a `brcm,bcm2712-hdmi0/1` node: the firmware may hand over a running HDMI | `rpi:vc4_hdmi.c:3391-3399`; bit `rpi:vc4_regs.h:1007` |
| `vc4_drm_bind` removes conflicting framebuffers and, when the firmware's own KMS driver is not in use, sends `RPI_FIRMWARE_NOTIFY_DISPLAY_DONE` (0x00030066) to stop the firmware's display driver | `rpi:vc4_drv.c:306-312,390,397-400`; tag `raspberrypi-firmware.h:100` |
| `vc4_crtc_disable_at_boot` finds a pixel valve the firmware left running by `PV_CONTROL.EN` (bit 0) and `PV_V_CONTROL.VIDEN` (bit 0), maps it to its HVS channel through `vc4_hvs_get_fifo_from_output`, and disables it; it runs for the `brcm,bcm2712-pixelvalve0/1` nodes | `rpi:vc4_crtc.c:553-610` (compatibles `:568-575`, the two bits `:576-580`, channel `:582`) |
| Pixel valve 0 drives HVS output 0 and the HDMI0 encoder, pixel valve 1 output 1 and HDMI1; outputs 0 and 1 are FIFOs (channels) 0 and 1 on the 6.x HVS | `rpi:vc4_crtc.c:1331-1357`; `rpi:vc4_hvs.c:916-930` |
| Linux's debugfs dump of a running 6.x channel: `DISPX_STATUS.MODE` not `DISABLED` (0) or `EOF` (3), then the words from `DISPX_DL.LACT` until a word with `END` (bit 31) | `rpi:vc4_hvs.c:282-320` |

So Linux expects a firmware scanout and has code for both adopting its state
and stopping it. Whether one runs when a `kernel=` image is Zephyr, and on
which output, only the board says (check Z1).

### Which registers and display-list words give the plane

All offsets are from the HVS base 0x10_7c580000. `X` is the channel (0 for
HDMI0); the C0 stride between channels is 0x20, the D0 stride 0x40.

| Register | C0 | D0 | Fields | Citation |
|---|---|---|---|---|
| `VERSION` | 0x00 | 0x00 | bits 7:0: 0x53 C0, 0x54 D0 | `rpi:vc4_regs.h:543-546` |
| `CXM_SIZE` | 0x04 | 0x04 | display-list memory size in words | `:547`; `rpi:vc4_hvs.c:1589` |
| `CONTROL` | 0x20 | 0x20 | `HVS_EN` 31, `PF_LINES` 22:18, `MAX_REQS` 7:4 | `:553-558,682` |
| `DISPX_CTRL0` | 0x30 + 0x20·X | 0x100 + 0x40·X | `ENB` 31, `RESET` 30, `FWIDTH` 28:16 (width − 1), `ONESHOT` 15, `LINES` 12:0 (height − 1) | `:564-573,701,718,729` |
| `DISPX_CTRL1` | 0x34 + 0x20·X | 0x104 + 0x40·X | `BGENB` 8, `INTLACE` 0 | `:575-580` |
| `DISPX_LPTRS` | 0x3c + 0x20·X | 0x110 + 0x40·X | `HEADE` 11:0: the word index where the channel's next list starts | `:587-591,704` |
| `DISPX_COB` | 0x40 + 0x20·X | 0x114 + 0x40·X | `TOP` 31:16, `BASE` 15:0: the output FIFO's range (its size is `top − base + 4`) | `:593-598,705`; `rpi:vc4_crtc.c:93-104` |
| `DISPX_STATUS` | 0x44 + 0x20·X | 0x118 + 0x40·X | `EMPTY` 22, `FRCNT` 21:16 (frame count), `MODE` 14:13 (0 disabled, 1 init, 2 run, 3 EOF), `YLINE` 12:0 (the line being composed) | `:600-612,706`; `rpi:vc4_crtc.c:135,148`, `rpi:vc4_hvs.c:794-812` |
| `DISPX_DL` | 0x48 + 0x20·X | 0x11c + 0x40·X | `LACT` 11:0: the word index of the list being scanned now | `:614-619,714` |
| display-list memory | 0x4000.. | 0x4000.. | `CXM_SIZE` 32-bit words | `:541`; `rpi:vc4_hvs.c:2150-2153` |

The list at `LACT` (or `HEADE`) is a chain of plane entries; each entry is the
words below in this order, and `CTL0.NEXT` is the entry's length in words
(`rpi:vc4_plane.c:2143-2217,2296-2299`):

| Word | Fields | Citation |
|---|---|---|
| `CTL0` | `END` 31 (the list's end marker, an entry of its own), `VALID` 30, `NEXT` 29:24, `RGB_TRANS` 23, `ADDR_MODE` 22:20 (0 linear, 1 128B, 2 256B, 3 MAP8, 4 UIF), `ALPHA_MASK` 19:18, `UNITY` 15 (no scaling: then `POS1` is absent), `ORDERRGBA` 14:13, `SCL1_MODE` 10:8, `SCL0_MODE` 7:5, `PIXEL_FORMAT` 4:0 | `rpi:vc4_regs.h:1359-1377`; `rpi:vc4_plane.c:2143-2152` |
| `POS0` | `START_Y` 28:16, `HFLIP` 15, `START_X` 12:0 | `:1379-1381`; `:2154-2159` |
| `CTL2` | alpha mode 31:30, premultiplied 29, mix 28, background fill 26, CSC enable (C0 bit 25, D0 bit 19), alpha 15:4 | `:1383-1389,1215-1216,1212`; `:2161-2166` |
| `POS1` (only when not `UNITY`) | `SCL_LINES` 28:16, `SCL_WIDTH` 12:0 (scaled size − 1) | `:1391-1392`; `:2168-2174` |
| `POS2` | `SRC_LINES` 28:16, `SRC_WIDTH` 12:0 (source size − 1) | `:1394-1395`; `:2176-2182` |
| `POS3` | context, written by the hardware | `:2184-2185` |
| `PTR0` (per colour plane) | `VFLIP` 31, `UPM_BASE` 28:16, `UPM_HANDLE` 14:10, `UPM_BUFF_SIZE` 9:8, **`UPPER_ADDR` 7:0 = address bits 39:32** | `:1397-1405`; `:2196-2205`, UPM fields `:1136,1193-1197` |
| `PTR1` | **address bits 31:0** | `:2207-2208`; the flip rewrites `PTR0` and `PTR1` only, `:2451-2463` |
| `PTR2` | **`PITCH` 16:0** (bytes per line, linear planes) | `:1415`; `:2210-2217` |

The 32 bpp formats: `PIXEL_FORMAT` 7 is RGBA8888 and `ORDERRGBA` says which
byte is which (0 RGBA, 1 BGRA, 2 ARGB, 3 ABGR); Linux maps `DRM_FORMAT_XRGB8888`
and `ARGB8888` to order 2, `XBGR8888`/`ABGR8888` to order 3, `RGBX8888`/`RGBA8888`
to order 0 and `BGRX8888`/`BGRA8888` to order 1; `PIXEL_FORMAT` 4 is RGB565
with its own order codes (`rpi:vc4_regs.h:1065,1070,1097-1106`;
`rpi:vc4_plane.c:42-95`). The Pi 4 board layer's two window formats are
therefore order 2 (`XRGB8888`, B first in memory) and order 0 (`RGBA8888`, R
first); any other order the firmware uses is a format the GL core does not
offer today and a refusal by name, not a guess.

### What route A needs and risks

| Needs | Detail |
|---|---|
| `device_map()` (read, then one word written for nothing) | HVS 0x10_7c580000 size 0x1a000; pixelvalve0 0x10_7c410000 size 0x100; `hd` 0x10_7c720000 size 0x100; IOMMU4 0x10_0000_5200 size 0x80. All above 4 GiB: `CONFIG_ARM64_PA_BITS_40` (the `rpi_5` defconfig has it, facts section 11) and enough `CONFIG_MAX_XLAT_TABLES` (the probes use 16) |
| A mapping of the found buffer | `PTR0.UPPER_ADDR:PTR1` as a 40-bit address, `PTR2.PITCH` × (`POS2.SRC_LINES` + 1) bytes, mapped Normal non-cacheable as the Pi 4's (`rpi_mbox.c:208-222`) |
| No mailbox tag, no clock, no reset | the firmware owns the chain; the image only writes pixels |
| Mailbox reads for safety | `GET_CLOCK_RATE` (0x00030002) of clocks 16 (`disp`) and 13 (`hdmi`): Linux refuses to touch the HDMI registers while the HSM clock reads 0 because the access "will lead to a silent CPU stall", and the firmware leaves it at 0 when no monitor was plugged at boot (`rpi:vc4_hdmi.c:3251-3262`; `rpi:drivers/clk/bcm/clk-raspberrypi.c:95-106`). The probe reads the rate first and skips `HD_VID_CTL` when it is 0 |

| Risk | What it would look like | Then |
|---|---|---|
| No scanout runs for a `kernel=` image (the firmware draws nothing when it boots a kernel without a display request, or only its splash) | `DISPX_CTRL0.ENB` clear or `STATUS.MODE` 0 on both channels, `PV_CONTROL.EN` clear | route A is closed; B or C |
| The plane is not a linear 32 bpp RGBA8888 (a 565 splash, a tiled image, a scaled plane with `POS1`) | `CTL0.PIXEL_FORMAT` ≠ 7 or `ADDR_MODE` ≠ 0 or `UNITY` clear | A as a verification target only (readback), B for the GL window |
| The plane's address is an IOVA, not a physical address | IOMMU4 `MMMU_CTRL.ENABLE` set and the address inside its aperture (above the bypass end) | the buffer cannot be found from the ARM side without the IOMMU's tables; B or C |
| The firmware keeps drawing into the same buffer (a boot message) or moves the plane | the pattern written in Z3 is overwritten or the `PTR` words change between reads | B, which gives the image a buffer of its own |
| The firmware's size is the monitor's (EDID) and cannot be chosen | `FWIDTH`/`LINES` are what they are | the board layer takes the size it finds; the Pi 4's fixed `YAKOGL_BOARD_RPI4_FB_WIDTH/HEIGHT` become a request, not a promise |
| Reading HDMI registers with the HSM clock at 0 stalls the CPU (the watchdog then resets the board) | no further console line after the `hd` read | read clock 13 first; never touch `hd` when it is 0 |

Size: small. A header-only decoder of the list words with mock tests (planned
as the plan's HVS6 decoder), and in the probe about a dozen register reads and
one buffer write. Nothing of it is wasted even if route B is taken: the same
readback is how route B is verified without a capture device (section 7).

## 4. Route B: the mailbox framebuffer

The Pi 4's route, unchanged in kind: the firmware allocates and scans the
buffer, the image draws into it and pans it.

### The tags, as `rpi_mbox.c` sends them

| Tag | Id | Request words / answer | Citation |
|---|---|---|---|
| `SET_PHYSICAL_WIDTH_HEIGHT` | 0x00048003 | w, h / the granted w, h | `raspberrypi-firmware.h:137`; `repo:rpi_mbox.c:171,256` |
| `SET_VIRTUAL_WIDTH_HEIGHT` | 0x00048004 | w, virtual h / granted | `:138`; `:172,257` |
| `SET_DEPTH` | 0x00048005 | 32 / granted | `:139`; `:173,258` |
| `SET_PIXEL_ORDER` | 0x00048006 | 1 (RGB) / the order set, 0 BGR or 1 RGB | `:140`; `:174,259`; `repo:rpi4.rs:16-36` |
| `SET_VIRTUAL_OFFSET` | 0x00048009 | x, y / echoed | `:143`; `:260,308` |
| `FRAMEBUFFER_ALLOCATE` | 0x00040001 | alignment / bus address, size | `:105`; `:175,261` |
| `GET_PITCH` | 0x00040008 | / bytes per row | `:112`; `:176,262` |
| `GET_VIRTUAL_OFFSET` | 0x00040009 | / x, y | `:113`; `:330` |
| `SET_VSYNC` | 0x0004800e | returns at the next vsync | `:149`; `:344` |
| Not used by the Pi 4 code, available: `FRAMEBUFFER_RELEASE` 0x00048001, `BLANK` 0x00040002, `SET_PITCH` 0x00048008, `GET_NUM_DISPLAYS` 0x00040013, `SET_DISPLAY_NUM` 0x00048013, `GET_DISPLAY_SETTINGS` 0x00040014 (a 64-byte answer), `GET_DISPLAY_ID` 0x00040016, `NOTIFY_DISPLAY_DONE` 0x00030066 | | | `:121,106,142,124,123,125,185,122,100` |

Buffer rules (the facts file, section 9, re-read): 16-byte aligned, whole
words, below 1 MiB in size, a 32-bit address; channel 8
(`rpi:drivers/firmware/raspberrypi.c:21,24,53-58,100-114`); the response
word is 0x80000000 and anything else is an error
(`raspberrypi-firmware.h:15-16`; `raspberrypi.c:121`); each answered tag
has bit 31 of its code word set (`repo:rpi4.rs:21-22,29`). On the BCM2712
**the answer's bus address is the ARM address**: `soc` has no `dma-ranges`,
the `firmware` node's is empty, the `axi` bus maps the first 64 GiB one to
one (`rpi:bcm2712.dtsi:189-193,440`; `rpi:bcm2712-rpi.dtsi:14`), and
`rpi_mbox.c` already masks nothing on the BCM2712 (`repo:rpi_mbox.c:42-47`).
The firmware's answer is a 32-bit word, so the buffer it hands out is below
4 GiB by construction.

### Who still speaks these tags for the BCM2712

| Fact | Citation |
|---|---|
| The Pi 5 base device tree keeps the firmware framebuffer node `fb` (`brcm,bcm2708-fb`, `firmware = <&firmware>`) and the alias `fb = &fb`; the `vc4-kms-v3d-pi5` overlay is what disables it | `rpi:bcm2712-rpi.dtsi:37-40,128`; `rpi:arch/arm/boot/dts/overlays/vc4-kms-v3d-pi5-overlay.dts:12-17` |
| The Pi 5 kernel configuration builds that driver in (`CONFIG_FB_BCM2708=y`) and the KMS driver as a module | `rpi:arch/arm64/configs/bcm2712_defconfig:1118,1099` |
| That driver's mode set is one request of `SET_PHYSICAL_WIDTH_HEIGHT`, `SET_VIRTUAL_WIDTH_HEIGHT`, `SET_DEPTH`, `SET_VIRTUAL_OFFSET`, `ALLOCATE`, then `SET_PITCH` with an ARM-allocated buffer, or `GET_PITCH` with base 0 ("the old scheme", the firmware allocates), the same six tags `rpi_mbox.c` sends, in the same order | `rpi:drivers/video/fbdev/bcm2708_fb.c:349-369,389-432,434-455` |
| It first sends `SET_DISPLAY_NUM` when the firmware supports several displays (the two HDMI outputs) | `bcm2708_fb.c:215-226,123` |
| It ORs 0x40000000 into the answered base and masks bits 31:30 away for the CPU address: a BCM2711 habit (the 0xc0000000 alias) that would truncate a BCM2712 address at or above 1 GiB, so on the Pi 5 the firmware's answer is either below 1 GiB or the ARM-allocated buffer | `bcm2708_fb.c:464-467`; CMA is placed below 1 GiB, `rpi:bcm2712.dtsi:180-186` |
| The fork also carries a firmware-KMS driver for the 2712 (`raspberrypi,rpi-firmware-kms-2712`, a `disabled` node) that drives planes and timings through the mailbox (`SET_PLANE` 0x00048015, `SET_TIMING` 0x00048017, `GET_DISPLAY_CFG`, `SET_DISPLAY_POWER`): the firmware's display service exists on the BCM2712 | `rpi:bcm2712-ds.dtsi:68-74`; `rpi:vc4_firmware_kms.c:1374-1379,390,913,937`; tags `raspberrypi-firmware.h:156,158-160` |
| The Pi 4's counterparts, for contrast: `brcm,bcm2708-fb` in `bcm270x-rpi.dtsi`, `rpi-firmware-kms-2711` | `rpi:arch/arm/boot/dts/broadcom/bcm270x-rpi.dtsi:59`; `rpi:arch/arm/boot/dts/broadcom/bcm2711-rpi-ds.dtsi:177` |

So under Linux the BCM2712 firmware answers `FRAMEBUFFER_ALLOCATE` and the
rest of the set, and the BCM2712 display service is the same service the
Pi 4's firmware has. **Open:** whether it answers when the `kernel=` image is
Zephyr and nothing of Linux has run (on the Pi 4 it does, measured
2026-09-24, `docs/notes/pi4-v3d-bringup.md`), and what size and format it
grants when `tryboot.txt` has no `hdmi_*` keys (`docs/standalone-boot.md`,
"What differs from the Pi 4": the Pi 5 run writes none). The size is the
answer's business: the board layer reads back the granted size, pitch and
order and refuses a window that does not fit, as the Pi 4 does.

### What route B needs and risks

| Needs | Detail |
|---|---|
| The mailbox client as it is | `CONFIG_YAKOGL_RPI5_MBOX` (`repo:driver/zephyr/lib/board/Kconfig:27-32`); Zephyr's `mbox0` stays disabled (the Pi 4 lesson, `docs/notes/pi4-v3d-bringup.md`, the mailbox paragraph) |
| A message buffer and a framebuffer below 4 GiB | the image's `sram0` is 32 MiB at 0x200000 (`tools/probes/rpi_5/*/boards/rpi_5.overlay`); the firmware's buffer is wherever it says, below 4 GiB |
| The Pi 4 decisions with the board as a parameter | pixel order to format, view, pan (`repo:rpi4.rs:28-91`), unchanged for the Pi 4 |
| A Pi 5 board layer | the Pi 4's `board_rpi4.c` shape with the BCM2712 mailbox: console, window, show, record screen, watchdog first |
| Verification without a capture device | route A's readback: after the allocation, the plane in the HVS display list must name the answered address, pitch and size (section 7) |

| Risk | What it would look like | Then |
|---|---|---|
| The firmware does not answer the framebuffer tags for a non-Linux kernel (a Pi 5 firmware that starts the display service only on request) | `-ETIMEDOUT`, or 0x80000001, or tags without bit 31 | route A if Z1 found a scanout, else C |
| The firmware allocates but scans nothing (no HDMI running) | an address and pitch, but `DISPX_CTRL0.ENB` clear and `HD_VID_CTL.ENABLE` clear | the buffer is usable as the GL window's memory, but there is no picture: C for the wire, or `SET_DISPLAY_POWER` 0x00048019 as a read-only experiment first |
| The granted order is BGR, or a size other than the request | the Pi 4 code already handles both: `window_format` and `-ERANGE` | nothing new |
| A double-height virtual buffer is refused | the Pi 4's fallback: one screen, windows copied (`board_rpi4.c:136-153`) | nothing new |
| A `SET_PIXEL_ORDER` answer the Pi 4 never saw | `pixel_order()` returns `None`, no window | a refusal by name |

Size: medium-small. `rpi_mbox.c` is unchanged. New: the board-parameterised
decisions and their host tests, a Pi 5 board layer of
about the Pi 4's size, a Kconfig entry, a mock run beside
`rpi_mbox_mock.c`. The plan's items `fb-by-board` and `rpi5-screen` are
exactly this.

## 5. Route C: program HVS 6, the pixel valve and HDMI ourselves

The route of last resort: Zephyr owns the whole chain. What Linux does, in
the order it does it (`rpi:vc4_crtc.c:680-700`: HVS enable, encoder
pre-configure, pixel valve configure and enable, encoder pre-enable, video
enable, encoder post-enable).

### HVS 6

| Step | Registers and values | Citation |
|---|---|---|
| Clocks | firmware clocks 4 (`core`) and 16 (`disp`) on through the mailbox (`SET_CLOCK_STATE` 0x00038001); Linux also reads the maximum core rate (HDMI 2.0 above 550 MHz) | `rpi:vc4_hvs.c:2111-2135` |
| Global init | `CONTROL` = `HVS_EN` with `PF_LINES` 8 and `MAX_REQS` 15; `PRI_MAP0/1` all ones; six CSC coefficient sets (C0 through the `CFC1` registers, D0 through `SCALER_PI_CMP_CSC_*`) | `rpi:vc4_hvs.c:1909-1960` |
| Channel init | `DISPX_CTRL0` = `RESET`; `DISPX_CTRL1.INTLACE` as the mode; `DISPX_CTRL0` = `ENB` with `FWIDTH` = width − 1 and `LINES` = height − 1 (`ONESHOT` for a writeback) | `rpi:vc4_hvs.c:1004-1037` |
| The list | written into the list memory from word 32 up (section 3's words), then `DISPX_LPTRS.HEADE` = its first word | `rpi:vc4_plane.c:1899-2300`; `rpi:vc4_hvs.c:1151-1175` |
| The UPM | each plane needs an allocation in the "UPM" line memory, named in `PTR0` (`UPM_BASE`, `UPM_HANDLE`, `UPM_BUFF_SIZE` 2 lines for a linear plane) | `rpi:vc4_plane.c:1136,1193-1197` |
| The COB | Linux only reads `DISPX_COB` to size the pixel valve's FIFO level: the output FIFO ranges are inherited from the firmware | `rpi:vc4_crtc.c:85-104` |
| Stop | `DISPX_CTRL0` with `RESET`, then without `ENB`, `STATUS.MODE` must read `DISABLED` | `rpi:vc4_hvs.c:1072-1097` |
| End of frame | `disp_intr` line 2 for channel 0; the handler toggles `CTRL1.BGENB` and frees the old list | `rpi:vc4_hvs.c:1484-1510` |

### Pixel valve 0 (0x10_7c410000)

| Register | Offset | Fields | Linux's value | Citation |
|---|---|---|---|---|
| `PV_CONTROL` | 0x00 | `FORMAT` 23:21 (0 = 24-bit), `FIFO_LEVEL` 20:15, `CLR_AT_START` 14, `TRIGGER_UNDERFLOW` 13, `WAIT_HSTART` 12, `PIXEL_REP` 5:4, `CLK_SELECT` 3:2 (1 = DPI/SMI/HDMI), `FIFO_CLR` 1, `EN` 0 | reset: `EN` clear then `FIFO_CLR`; configure: `FIFO_CLR`, the FIFO level from the COB size and `fifo_depth` 64, format 24, `CLR_AT_START`, `TRIGGER_UNDERFLOW`, `WAIT_HSTART`, the clock select; then `EN` | `rpi:vc4_regs.h:145-169`; `rpi:vc4_crtc.c:329-343,471-480,688,1331-1343` |
| `PV_V_CONTROL` | 0x04 | `ODD_TIMING` 29, `INTERLACE` 4, `CONTINUOUS` 1, `VIDEN` 0 | progressive: `CONTINUOUS`, and `ODD_TIMING` on the 6.x HVS with one pixel per clock; `VIDEN` last; disable clears `VIDEN` and waits 1 ms | `:171-180`; `:442-446,696-697,516-519` |
| `PV_HORZA` | 0x0c | `HBP` 31:16, `HSYNC` 15:0 | from the mode, divided by pixels per clock (1 progressive, 2 interlaced on the Pi 5) | `:184-188`; `:387-391,1338-1339` |
| `PV_HORZB` | 0x10 | `HFP` 31:16, `HACTIVE` 15:0 | | `:190-194`; `:393-397` |
| `PV_VERTA`, `PV_VERTB` | 0x14, 0x18 | `VBP` 31:16, `VSYNC` 15:0; `VFP` 31:16, `VACTIVE` 15:0 | | `:196-206`; `:450-455` |
| `PV_VSYNCD_EVEN`, `PV_VERTA_EVEN`, `PV_VERTB_EVEN` | 0x08, 0x1c, 0x20 | interlace only | 0 | `:182,208-209`; `:447` |
| `PV_INTEN`, `PV_INTSTAT` | 0x24, 0x28 | `VFP_START` bit 7 is the vblank | | `:211-222` |
| `PV_MUX_CFG` | 0x34 | `RGB_PIXEL_MUX_MODE` 5:2 | 8 (no swap) | `:228-231`; `:460-463` |
| `PV_PIPE_INIT_CTRL` | 0x94 | `PV_INIT_WIDTH` 11:8, `PV_INIT_IDLE` 7:4, `PV_INIT_EN` 0 | 1, 1, set (6.x HVS only) | `:233-236`; `:465-469` |

### HDMI0

Register names are Linux's; the region is the device tree's and the offset
is from that region's base (`rpi:vc4_hdmi_regs.h:431-529`, the
`vc6_hdmi_hdmi0_fields` table; hdmi1's `VID_CTL` is at `hd` + 0x48,
`:531-538`).

| Region | Register | Offset | Citation |
|---|---|---|---|
| `hd` 0x10_7c720000 | `DVP_CTL` 0x00, `VID_CTL` 0x44, `FRAME_COUNT` 0x60 | | `:432,438,439` |
| `hdmi` 0x10_7c701400 | `FIFO_CTL` 0x7c, `RAM_PACKET_CONFIG` 0xc4, `SCHEDULER_CONTROL` 0xe8, `HORZA` 0xec, `HORZB` 0xf0, `VERTA0` 0xf4, `VERTB0` 0xf8, `VERTA1` 0x100, `VERTB1` 0x104, `MISC_CONTROL` 0x114, `DEEP_COLOR_CONFIG_1` 0x18c, `GCP_CONFIG` 0x194, `GCP_WORD_1` 0x198, `HOTPLUG` 0x1c8 | | `:441-471` |
| `dvp` 0x10_7c701000 | `CLOCK_STOP` 0xbc, `VEC_INTERFACE_CFG` 0xf0, `VEC_INTERFACE_XBAR` 0xf4 | | `:474-476` |
| `phy` 0x10_7c701d00 | `RESET_CTL` 0x000 .. `PLL_POWERUP_CTL` 0x194 (the facts file's section 10 sequence) | | `:478-499` |
| `rm` 0x10_7c702000 | `RM_CONTROL` 0x000, `RM_OFFSET` 0x018, `RM_FORMAT` 0x01c | | `:501-503` |
| `packet` 0x10_7c703800 | `RAM_PACKET_START` 0x000 (the infoframe RAM) | | `:505` |
| `cec` 0x10_7d510800 | `CEC_CNTRL_1` 0x010 .. | | `:507-519` |
| `csc` 0x10_7c700100 | `CSC_CTL` 0x000, the coefficient pairs 0x004-0x018, `CSC_CHANNEL_CTL` 0x02c | | `:521-528` |

`VID_CTL` bits: `ENABLE` 31, `UNDERFLOW_ENABLE` 30, `FRAME_COUNTER_RESET` 29,
`VSYNC_LOW` 28, `HSYNC_LOW` 27, `CLRSYNC` 24, `CLRRGB` 23, `BLANKPIX` 18,
`BLANK_INSERT_EN` 16 (`rpi:vc4_regs.h:1007-1015`). `SCHEDULER_CONTROL`:
`MANUAL_FORMAT` 15, `IGNORE_VSYNC_PREDICTS` 5, `HDMI_ACTIVE` 1, `MODE_HDMI`
0; `FIFO_CTL.MASTER_SLAVE_N` 0; `RAM_PACKET_CONFIG.ENABLE` 16 (`:839-843,836,796`).

| Step | What Linux does | Citation |
|---|---|---|
| Resources | regions `hdmi`, `hd`, `cec`, `csc`, `dvp`, `phy`, `packet`, `rm`; clocks `hdmi` (HSM), `bvb`, `audio`, `cec`; the reset (DVP bit 1). **No `pixel` clock on the BCM2711 and BCM2712**: only the BCM2835 variant asks for one, so the TMDS clock is the PHY PLL's alone | `rpi:vc4_hdmi.c:3089-3186` (`:3162,3168,3186`); `:3047-3075` (`:3070`); variants `:3555,3583` |
| Power-up | HSM clock on and its rate checked non-zero (else a silent CPU stall), audio clock on, then `reset`: the DVP reset line pulsed and `DVP_CTL` = 0 | `rpi:vc4_hdmi.c:3239-3268,208-225` |
| Pre-configure | HSM minimum = max(120 MHz, 1.01 × TMDS rate); the BVB clock 75, 150 or 300 MHz by the TMDS rate; PHY init (section 10 of the facts file: reset, nine PLL constants, 54 MHz reference, VCO 8 to 12 GHz with its divider, lane settings per rate, power-up, PLL reset); `SCHEDULER_CONTROL` ∣= `MANUAL_FORMAT`, `IGNORE_VSYNC_PREDICTS`; timings | `:1481-1595` (`:1523-1526,1546-1559,1566,1570-1573,1578`); `rpi:vc4_hdmi_phy.c:954-1198,605-627` |
| Timings | `HORZA` (sync polarities, active, front porch), `HORZB` (back porch, sync), `VERTA0/1`, `VERTB0/1`, `DEEP_COLOR_CONFIG_1`, `GCP_WORD_1`, `GCP_CONFIG`, `MISC_CONTROL` pixel repetition, `CLOCK_STOP` = 0 | `:1332-1437` (`:1365-1386,1414-1432`) |
| Pre-enable | CSC: RGB output uses the crossbar word 0x354021 and the full-range RGB-to-RGB coefficient table; `VEC_INTERFACE_CFG`, `VEC_INTERFACE_XBAR`, `CSC_CHANNEL_CTL`, `CSC_CTL`; then `FIFO_CTL` = `MASTER_SLAVE_N` | `:1204-1265` (`:1249-1251,1258-1261`), `:1597-1625` (`:1618`) |
| Post-enable | `VID_CTL` = `ENABLE`, `CLRRGB`, `UNDERFLOW_ENABLE`, `FRAME_COUNTER_RESET`, `BLANK_INSERT_EN`, the sync polarities; clear `BLANKPIX`; HDMI sink: `SCHEDULER_CONTROL` ∣= `MODE_HDMI` and wait for `HDMI_ACTIVE` (1000 ms), then `RAM_PACKET_CONFIG.ENABLE` and the infoframes; DVI sink: packets off | `:1627-1700` (`:1648-1660,1663-1672,1690-1696`) |
| Disable | `RAM_PACKET_CONFIG` = 0, `CLRRGB`, `BLANKPIX`; clearing `VID_CTL.ENABLE` is skipped on the BCM2712 because it "result[s] in a system lockup" (Linux's own TODO); power-down blanks, disables the PHY and the clocks | `:882-960` (`:912-921`) |

### What route C needs and risks

| Needs | Detail |
|---|---|
| `device_map()` | every block of section 2 (HVS, pixel valve 0, the nine hdmi0 regions, the DVP block, `disp_intr` if the EOF interrupt is used, IOMMU4), all with 40-bit addresses |
| Mailbox | `SET_CLOCK_STATE`/`SET_CLOCK_RATE` for clocks 4, 16, 13, 14 (ids `raspberrypi-firmware.h:169,181,178,179`); `GET_CLOCK_RATE` to confirm |
| DVP | the 108 MHz gate for hdmi0 (+0x08 bit 3) and the hdmi0 reset (+0x04, line 1) |
| A mode | the HDMI timing of the sink: from the EDID over DDC (`ddc0`, an I2C bus Zephyr does not drive today) or a fixed mode as the Pi 4's `hdmi_timings` config gave the firmware |
| The PHY's constants | the nine PLL words and the per-rate lane tables are GPL source constants; the project's rule is to take them from the board (the register dump of the running Linux, check L2) and not from the file |
| Infoframes | an AVI infoframe in the packet RAM for an HDMI sink; or DVI mode without |

| Risk | Note |
|---|---|
| The HSM-at-zero stall and the BCM2712 disable lock-up: two documented ways to hang the SoC, which here ends in a watchdog reset and a lost session | `rpi:vc4_hdmi.c:3251-3262,912-921` |
| The UPM and COB allocations: Linux allocates the UPM itself and inherits the COB from the firmware; without a running firmware setup both must be chosen by us from `LBM_SIZE`/`UBM_SIZE`/`COB_SIZE` (+0x08, +0x0c, +0x14, `rpi:vc4_regs.h:548-551`) with no documentation | |
| The PHY lane settings per TMDS rate are a table the Linux driver carries for the BCM2712 alone (`rpi:vc4_hdmi_phy.c:658-911`); a wrong value gives no picture with every register reading fine | |
| Hot-plug and EDID are not needed for a bench with one fixed monitor, but a Pi 5 that boots without a monitor must not touch the HDMI blocks | |

Size: large. The Linux paths involved are about 1,800 lines (the HVS 6 init
and list writer, the pixel valve configuration, the HDMI configure, timings,
CSC and enable, the PHY init with its tables), to be re-derived as facts and
rewritten, plus a mode source and the mock tests; the x2 and T6 cold-display
notes (`driver/docs/x2-hdmi-cold.md`, `driver/docs/nanopc-t6-display.md`) are
the shape. Weeks, not days. It is taken only if A and B are both closed.

## 6. The board checks, in the order one session runs them

Rules of every Pi 5 session apply (`tools/probes/rpi_5/README.md`): the image
arms the BCM2712 PM watchdog first and comes back to Linux by itself through
`tryboot`; `vcgencmd get_throttled` is recorded with every record; one client
holds `target:rpi5` and `target:rpi5-console` together; if the board does not
come back, stop and report that it needs a person. Records go to
`records/rpi5/<YYYYMMDD-HHMM>-<name>-<sha8>/`, gitignored and redacted; no
address, host or UART path in any file.

### Under Linux first, over ssh, read-only (the references)

Raspberry Pi OS runs the KMS driver with the `vc4-kms-v3d-pi5` overlay, so
what it shows is Linux's own setup, not the firmware's: a reference for the
register layout, the stepping and the mode, and the proof that the decoders
of section 3 read real words.

| Check | What is read | What it gives |
|---|---|---|
| L1 | `/proc/device-tree`: the `status` of `hvs`, `pixelvalve0`, `hdmi0`, `fb`, `firmwarekms`, the `iommus` of `hvs` and `vc4`, `compatible` of `hvs` | which driver owns the display under Linux; whether `fb` (the firmware framebuffer) is disabled by the overlay as the source says |
| L2 | debugfs of the `vc4` DRM device: `hvs_regs`, `hvs_dlists`, `hvs_upm`, `crtc0_regs`, `hdmi0_regs` (names: `rpi:vc4_hvs.c:1529-1541`, `rpi:vc4_crtc.c:1333`, `rpi:vc4_hdmi.c:3540`); `modetest`-style mode and plane listing if the tool is present | the HVS stepping (`VERSION`), a decoded live display list with a known plane (address, pitch, format) to test the decoder against, the pixel valve timing words, the HDMI timing, CSC and PHY register values of a working mode (the only lawful source of the PHY constants for route C) |
| L3 | `vcgencmd measure_clock core`, `hdmi`, `pixel`, `v3d`; `vcgencmd get_throttled`; `vcgencmd get_config int` (any `hdmi_*` keys) | the clock rates a working display runs at; the supply state |
| L4 | raw reads of the scanout blocks (`/dev/mem` or an equivalent, read-only): HVS +0x00..+0x4c and the D0 block +0x100..+0x120, pixel valve 0 +0x00..+0x34, `hd` +0x44, IOMMU4 +0x00, +0x14, +0x1c, +0x20, +0x38; only after L3 showed the `hdmi` clock non-zero | the same words the probe will read, from the same addresses, to compare the probe's reader word for word |

### Under Zephyr through `tryboot` (the probe image `rpi5_display`)

| Check | What the image does | Answers |
|---|---|---|
| Z0 | watchdog armed; mailbox: `GET_CLOCK_RATE` for clocks 4, 16, 13, 14 and 5, printed; IOMMU4 `MMMU_CTRL`, bypass start/end, address cap, `DEBUG_INFO`, printed; HVS `VERSION`, `CXM_SIZE`, `CONTROL`, printed | whether the HSM clock runs (if 0, every HDMI read below is skipped and said so); whether the IOMMU translates or passes low addresses; C0 or D0 |
| Z1 | HVS channels 0..2: `CTRL0`, `CTRL1`, `LPTRS`, `COB`, `STATUS`, `DL`; the display-list words from `DL.LACT` to `END` (at most 64) for every channel with `STATUS.MODE` 1 or 2, decoded (format, order, address mode, unity, source size, 40-bit address, pitch); pixel valve 0 `CONTROL`, `V_CONTROL`, `HORZA/B`, `VERTA/B`; `hd` `VID_CTL` only when Z0 allowed it | **(1) does a scanout run, at what address and format?** |
| Z2 | the Pi 4's one-request allocation (`rpi_fb_alloc_virtual`: physical w×h, virtual w×2h, depth 32, order RGB, offset 0,0, allocate 4096, get pitch) at the Pi 4's size; every tag's response code and value printed raw; then `GET_NUM_DISPLAYS` and `GET_DISPLAY_SETTINGS` as read-only extras; then Z1's registers again | **(2) does the mailbox allocate, and what does it answer?** And whether the firmware put the new buffer into the HVS list (the plane's `PTR0`/`PTR1` now equal the answer) |
| Z3 | a known pattern (row index in one channel, column in another, a constant in the third, as the Pi 4's clear probes) written into the buffer of Z2, or into Z1's plane when Z2 gave none; the plane address read back from the list and compared with the buffer's physical address; `STATUS.FRCNT` read twice 100 ms apart; the bytes read back through the CPU mapping and compared | **(3) the scanout reads our buffer**: address equal, frame count advancing, bytes as written |
| Z4 | the Pi 4's split screen: `fbcon` text on the top half, the pattern on the bottom half, the record screen at the end; then the feeding stops and the watchdog returns the board to Linux | **(4) the first picture** (by eye if someone looks; by readback otherwise) |

### Which result picks which route

| Z1: scanout runs | Z2: mailbox allocates | Route | Why |
|---|---|---|---|
| yes | yes | **B**, A's readback as its verification | the Pi 4's path works unchanged; the firmware's list shows our buffer |
| no | yes | **B**; and a read-only `GET_DISPLAY_SETTINGS` and `SET_DISPLAY_POWER` experiment if Z3 shows no scanout of the allocated buffer | the firmware allocates on request; whether it also starts the wire is Z3's frame count |
| yes | no | **A**: write into the found plane, take its size and format as found; B dropped | the firmware scans but does not serve the tags to a non-Linux kernel |
| no | no | **C**, with L2's register values as the reference | nothing of the firmware's display is available |

Z0 decides nothing by itself but can stop the session: an HSM clock of 0
means no `hd` read, and an IOMMU that translates means Z3's address
comparison is against an IOVA (then recorded, not trusted).

### The first route coded

**Route B is the first route coded, before the board session.** The reading
does not show it closed: the BCM2712 firmware answers the same six tags for
Linux's framebuffer driver, the Pi 5 base tree keeps that driver's node, the
mailbox client already runs for the BCM2712, and the Pi 4's decisions
generalise by board. What the session can show is that the firmware does not
answer a Zephyr `kernel=` image (Z2 "no"), and then route A is the next
cheapest; route A's reader is built anyway, as route B's verification. If the
session shows A and B both closed, this note is updated with the Z-records
and route C is scheduled as its own plan.

## 7. Verification

The farm lists no HDMI capture device as a resource of its own for the Pi 5
(no `display-observation` target): the board's resource description says
its HDMI0 feeds a USB capture device on a farm node, verified under Linux on
2026-11-06 with the boot loader's diagnostics screen and the kernel's
consoles, but a session cannot lease it and the probe cannot read it. So no
check here needs one. The picture is verified as the Pi 4 did it: by the
registers and by the bytes. A route counts as working when the HVS display
list names the image's buffer (`PTR0.UPPER_ADDR:PTR1` equal to the buffer's
physical address, `PTR2.PITCH` equal to the pitch the board layer uses,
`CTL0.PIXEL_FORMAT` 7 with the order the board layer announced), the
channel's `STATUS.FRCNT` advances while the image runs, pixel valve 0 has
`EN` and `VIDEN` set, `HD_VID_CTL.ENABLE` is set (read only with a non-zero
HSM clock), and every byte of the pattern reads back as written through the
Normal non-cacheable mapping. The console log carries the words, the record
directory keeps them, and the plan's parser turns them into verdicts. A
frame from the capture device, when someone with access to that node takes
one during Z4, or a person at a monitor, is a confirmation, never the proof.

The pixel valve and HDMI0 registers of the checks above decode through
`driver/zephyr/lib/board/rpi5_scanout/pv_hdmi.h` (pure functions over values
the caller read; host test `tools/v3d/check71.d/pv_hdmi.sh`).

## 8. Open

- **U3 on the board:** does a Zephyr `kernel=` image find a running firmware
  scanout (Z1), and does the BCM2712 firmware answer `FRAMEBUFFER_ALLOCATE`
  and its companions to it (Z2)? Both are open until the session; the
  reading says neither is closed.
- **The firmware's mode without `hdmi_*` keys:** the size, order and pitch
  the firmware grants on this board's sink (the Pi 4 asked for a custom
  1920x720 mode in `config.txt`; the Pi 5 run writes none). The sink is the
  capture device: its EDID makes Linux pick 3840x2160 at 30 Hz unless
  `cmdline.txt` pins 1920x1080 at 60 Hz, which it does (the resource
  description), and Zephyr reads neither file. A 3840x2160 32 bpp screen is
  about 32 MiB per buffer, as large as the image's `sram0`: the board layer
  must take the granted size from the answer and refuse, by name, a window
  that does not fit, and a `SET_PHYSICAL_WIDTH_HEIGHT` request the firmware
  does not honour is a result of Z2, not a failure.
- **The IOMMU4 state the firmware leaves** (Z0): bypass, or an aperture with
  pass-through below it, or translation of everything.
- **Stepping:** C0 or D0 (`SCALER6_VERSION`), which chooses the register
  layout the readers use.
- **The disp clock (16) at 0:** whether HVS registers stall like the HDMI's
  with the HSM at 0 is not stated anywhere read; Z0 reads the rate first and
  the probe treats a 0 as "do not read the HVS" until a run shows otherwise.
- **HDMI0 or HDMI1:** which connector the firmware drives for a `kernel=`
  boot when only one has a monitor; Z1 reads all channels and the second
  `VID_CTL` (`hd` + 0x48) under the same clock rule.
- **Pan and vsync on the Pi 5:** whether `SET_VIRTUAL_OFFSET` and `SET_VSYNC`
  behave as measured on the Pi 4 (a pan of about 90 us, a vsync wait of about
  32 ms, `repo:rpi_mbox.h:68-96`); a later measurement of the Pi 5 board
  layer, not of this probe.
- **Route C's licence question:** the PHY constants may be taken only from
  the board's own register dump (L2), never from the GPL source; whether a
  dump of a running mode is enough for a second mode is unknown.
- **The firmware's own drawing:** whether the firmware writes into its
  scanout buffer after the kernel started (a boot message, a splash update);
  Z3 reads the pattern back twice.
