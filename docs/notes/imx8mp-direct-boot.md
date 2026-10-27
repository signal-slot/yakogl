<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Direct boot on the Verdin iMX8M Plus: Zephyr in place of U-Boot proper

Goal (2026-10-21, the owner): the board powers on into Zephyr and the Slint
cluster (route 3, the M7 with it) with no U-Boot proper and no Linux,
as fast as the chain allows; the owner's wish is 100 ms from power-on to the
first frame, the number to measure against. Everything the board itself runs
before Zephyr is Toradex's and NXP's: the Boot ROM, U-Boot SPL (DDR
training), TF-A.

Status (2026-10-21): designed from the board's own boot image, built and
checked offline. **Nothing has run on the board.** The board was read only
(`dd` of its boot partitions, `sfdisk -d`, `/etc/fw_env.config`); the backups
are on the build host, outside the repository.

The Armadillo-X2 route (docs/notes/x2-direct-boot.md) is the same idea on
Atmark's imx-boot; the Verdin AM62P's (docs/notes/am62p-direct-boot.md)
shares the second-stage loader.

## The chain

```
Boot ROM ── eMMC boot partition (boot0 today; PARTITION_CONFIG 0x48)
  └─ U-Boot SPL 2024.07-7.7.0 (Toradex), IVT at 0: DDR training, PMIC
       └─ loads the FIT at image offset 0x58000 through the ROM API:
            uboot  = the second-stage loader   -> 0x40200000 (no entry)
            atf    = TF-A BL31                 -> 0x970000, entry
            fdt-1  = Toradex's DTB             -> appended after uboot (unused)
       └─ jumps to the first loadable with an entry point: BL31
            └─ BL31: no OP-TEE in this image, so straight to
               BL33 = 0x40200000, non-secure EL2, MMU/caches off, DAIF masked
                 └─ boot_stage2: EL2 -> EL1, inflates its payload to 0x48000000,
                    MMU and caches off, jumps
                      └─ the direct image (verdin_imx8mp/mimx8ml8/a53/direct):
                         __start, the display from cold, the GPU, the sample
```

U-Boot proper is gone. Its slot in the FIT holds the loader, which carries
the real image compressed. Every other part of Toradex's image is copied
unchanged (`tools/x2-mkflash.py build`, which reads the default
configuration's `firmware` name, so it serves this FIT too).

## What the board's boot image is

Read from the board on 2026-10-21 (Torizon OS 7.7.0+build.40, kernel
6.6.142-7.7.0): both eMMC boot partitions are 4 MiB (8192 sectors), boot0
holds the image and boot1 is all zero. U-Boot's environment is the last
0x2200 bytes of boot0 (`/etc/fw_env.config`: `/dev/emmc-boot0 -0x2200
0x2000`), so the image may use boot0 up to 0x3FDE00.

Layout (`tools/x2-mkflash.py info` on the dump):

```
IVT @0x0: entry 0x920000 self 0x91ffc0 boot_data 0x91ffe0 csf 0x946fc0
boot data: start 0x91ffc0 length 0x29000 (SPL + DDR firmware)
FIT @0x58000: header 1068 bytes, "Configuration to load ATF before U-Boot"
  uboot  "U-Boot (64-bit)"      standalone pos 0x003000 size 0x0d7408 load 0x40200000 entry -
  atf    "ARM Trusted Firmware" firmware   pos 0x0da408 size 0x00aa68 load 0x970000 entry 0x970000
  fdt-1  "freescale/imx8mp-verdin-wifi-dev" flat_dt pos 0x0e4e70 size 0x014408
config-1 (default): firmware=uboot, loadables=atf, fdt=fdt-1
```

No `tee` image: Toradex's TF-A is built without OP-TEE, so BL31 enters BL33
directly (the X2 chain has to keep OP-TEE in the FIT). The SPL's strings
show no gzip or LZMA support, so the FIT payload is loaded as is: whatever
sits in the `uboot` slot runs uncompressed, and the slot is about 3.5 MiB
(0x3FDE00 minus the FIT's data start and the atf and fdt payloads).

## Why a second stage

| image (verdin_imx8mp, Zephyr af17c0c8) | zephyr.bin | deflate (zlib 9) |
|---|---|---|
| triangle (direct variant, 2026-10-21) | 1.24 MB | 0.4 MB |
| slint (the cluster, direct variant, 2026-10-21) | 4.21 MB | 1.53 MB |
| gc_slint (the cluster, 2026-09-23 build, kexec variant) | 6.36 MB | 1.27 MB |
| gc_slint_ring (route 3, 2026-09-23 build) | 6.35 MB | 1.27 MB |

The cluster does not fit the slot uncompressed; compressed it fits with
room to spare, and so does the AM62P's (its slot is 1.49 MiB; the triangle
is 1.81 MB and 0.87 MB deflated). So the boot image is one piece, the
loader with the payload linked in (`BOOT_STAGE2_SOURCE_EMBEDDED`), and
nothing is written to the eMMC user area. The loader's eMMC source (the
AM62P design of 2026-10-02) stays for an image whose compressed form
outgrows the slot.

The time this costs is the inflate: RFC 1951 in Rust (`loader/src/inflate.rs`,
table-driven since 2026-10-21: the bit-by-bit form took about half a second
for 1.8 MB on an x86 host, the table form well under 50 ms) on one A53. The
loader's own number on the A53 is the first thing the board run reports
(`boot_stage2: ... unpacked in N ms`). If it is a large part of the budget,
the payload becomes LZ4 (a block decoder is a few dozen lines, decompression
runs near memory speed) or the image shrinks (the cluster image is mostly
Slint's pre-rendered glyphs and the shader table).

## The Zephyr side

Two images, two board qualifiers:

- **The loader**, `driver/samples/zephyr/boot_stage2` built for
  `verdin_imx8mp/mimx8ml8/a53` with `boards/verdin_imx8mp_mimx8ml8_a53.overlay`
  and `.conf`: its RAM is 0x40200000 + 32 MiB (TF-A's BL33 entry; SPL appends
  the FIT's devicetree right after the file, inside this RAM), one core, UART3
  console (not connected on the lab carrier: the loader is silent there), the
  GPU off. It identity-maps 0x48000000 .. 0x58000000 write-back and inflates
  the payload to 0x48000000 (`BOOT_STAGE2_LOAD`, the Kconfig defaults for
  this board), then jumps with DAIF masked, the EL1 timers stopped, the MMU
  and caches off (`loader/src/lib.rs`, `bs2_jump`).
- **The direct image**, `verdin_imx8mp/mimx8ml8/a53/direct`
  (`driver/boards/toradex/verdin_imx8mp/verdin_imx8mp_mimx8ml8_a53_direct.*`):
  the a53 (kexec) variant linked at 0x48000000 with 256 MiB of RAM up to the
  GPU carveout at 0x58000000, and the display path in the board devicetree
  (`driver/samples/zephyr/common/verdin_imx8mp_display.dtsi`: LCDIF1 ->
  MIPI DSI -> SN65DSI84 -> LVDS panel, the media block control, the
  backlight PWM), because nobody else owns the panel on this route. It is
  entered at EL1 by the loader (the kexec variant is entered at EL2 by the
  crash route); Zephyr's reset path takes both.

What the earlier stages own in DRAM (2 GiB on this module):

| range | owner |
|---|---|
| 0x40000000 + 2 MiB | SPL's saved DDR timing for TF-A's DRAM frequency service |
| 0x40200000 + 32 MiB | the loader and its payload; dead once the image runs |
| 0x48000000 + 256 MiB | the direct image (`zephyr,sram`) |
| 0x58000000 + 64 MiB | GPU carveout (the kexec variant's) |
| 0x80000000 + 64 MiB | Cortex-M7 carve-out (route 3): its firmware, status, ring, frame buffers |
| 0x970000 (OCRAM) | TF-A BL31 |

Boot time: `CONFIG_YAKOGL_BOARD_BOOT_TIME=y` prints the system counter at
the board layer's application init and at the first `board_show()`. On this
SoC SPL starts the counter early, so the value is close to the time since
power-on less the Boot ROM's part. Without a UART on the carrier the
figures reach the panel console, or the camera; the end-to-end number
(power LED to picture) is the camera's frame count.

## Building

```sh
# the direct image (linked at 0x48000000)
west build -p always -b verdin_imx8mp/mimx8ml8/a53/direct -d build/imx8mp/triangle_direct \
    examples/triangle            # or slint, with -DSLINT_NOSTD_DIR=... -DNOSTD_PATCH_DIR=...
# its payload (header, CRC-32s, raw deflate)
tools/am62p-mkboot.py stage2img --zephyr build/imx8mp/triangle_direct/zephyr/zephyr.bin \
    --load 0x48000000 --name triangle -o build/imx8mp/direct/stage2-triangle.img
# the loader with the payload linked in
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/imx8mp/loader_triangle \
    driver/samples/zephyr/boot_stage2 -- -DBOOT_STAGE2_PAYLOAD=$PWD/build/imx8mp/direct/stage2-triangle.img
# the boot image: Toradex's, with the loader in the uboot slot, below U-Boot's environment
tools/x2-mkflash.py build --base <boot0 dump> --bl33 build/imx8mp/loader_triangle/zephyr/zephyr.bin \
    --limit 0x3FDE00 -o build/imx8mp/direct/boot1-triangle.bin
```

(`west build` as `tools/check.sh`'s image rows run it: from the west
workspace, `RUSTUP_TOOLCHAIN=nightly`, `-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh $ZEPHYR_WS)`,
`-DYAKOGL_DIR=<this repository>`.)

## At the board

**Done on 2026-10-21, 23:30 JST onward (the owner at the board; no UART
on this carrier, the panel and the camera are the result channel):**

1. The loader built on the a53 board had the a53 default
   `CONFIG_MAX_XLAT_TABLES=8`, too few for its own 32 MiB, the payload
   region and the devices (the AM62P's loader failed silently the same
   way); `boards/verdin_imx8mp_mimx8ml8_a53.conf` sets 64.
2. No RAM trial: the owner chose to write boot1 directly. boot0 and boot1
   read back equal to the 2026-10-21 backups (boot0 `f9789f29...`, boot1
   all zero), PARTITION_CONFIG 0x48; `boot1-slint.bin` written from
   Torizon (`force_ro` cleared, `dd bs=1M conv=fsync`, read back equal),
   `mmc bootpart enable 2 1 /dev/mmcblk2`: PARTITION_CONFIG **0x50**.
3. **Power-cycle: the Slint cluster at power-on** (SPL -> BL31 -> the
   loader as BL33 -> inflate -> the slint image, the panel from cold
   through the C LCDIF/DSIM path). Two findings on that first picture:
   - it was the sample's 1024x600 window on the 1280x800 panel (the
     board layer copies a smaller window into a panel-sized buffer), and
     the UI's unit comes from the generated layout, which the
     sample always took for 1024x600: now `SLINT_WINDOW_WIDTH/HEIGHT`
     (the sample's Kconfig) pick the window and the layout, and this
     board's direct conf says 1280x800;
   - the first frame the viewer saw was a solid red/pink one, before the
     cluster: the cold bring-up lit the backlight at its end, about a
     second before the first rendered frame. With
     `YAKOGL_DRV_DISPLAY_IMX8MP_BACKLIGHT_ON_FIRST_SHOW` the backlight waits
     for the board layer's first `board_show()` (a 5 s fallback keeps the
     panel console reachable when no frame comes).
   The direct conf also runs the demo as long as the knobs allow
   (`ANIMATE_S=600`, `HOLD_S=3000`, `HOLD_FAIL_S=3000`, then one reset) and
   logs warnings and errors only. The boot-time marks (printk) are on the
   panel console, which this configuration never lights: the numbers are
   not readable on this route without a UART.
4. The next images were written the same way after a recovery boot of
   Toradex's boot0 image over `uuu` (its U-Boot boots Torizon from the
   eMMC, the write from there), one power-cycle each (2026-10-22, 00:20
   to 01:00 JST):
   - 1280x800 with the backlight at the first frame: the cluster filled
     the panel at once, and stood still. The sample's first-frame check
     (`check/src/lib.rs REFERENCE`, five pixel positions of the 1024x600
     design) failed on the scaled layout and the run sat in the failure
     hold. The verdict now counts only at 1024x600 (main.c); elsewhere it
     is printed and the run goes on unchecked.
   - with that: the cluster animated (camera: speed 98 -> 50, clock
     00:42 -> 01:08 between two frames), but flickered. At the panel's
     size the board layer scanned the window in place, so the LCDIF read
     the buffer the GPU was drawing the next frame into.
     `YAKOGL_BOARD_IMX8MP_ALWAYS_COPY` + `SCREEN_BUFFERS=2` (this conf):
     every frame copied into one of two screen buffers and flipped at
     VSYNC. The owner: no flicker, about 10 fps -- the 4 MiB copy per
     frame into the uncached carve-out is the cost. This is the state
     written in boot1 (`boot1-slint.bin` `20e4e9d8...`).
   - the copy-free route, tried next (01:10): `SLINT_WINDOW_BUFFERS=2`,
     the sample rendering into two panel-sized windows in turn, each its
     own EGL window surface made current before the frame, each shown
     through the in-place path (a flip by address, waiting for the
     latch). The panel came up pink and stayed so; the backlight came
     late (the 5 s fallback, so no first show ever counted), and after
     another power-cycle the panel was yellowish instead: the scanned
     buffer was never a drawn one. Cause, found in the
     code afterwards: the i.MX8MP board layer hands out ONE window
     (`board_window()`'s `taken`, -ENOMEM for the second), so
     `window()` failed before the first frame and the run never drew;
     the 5 s backlight fallback lit the panel on whatever the buffer
     held. The sample code for several windows stays (default 1, so no
     other board changes); the conf is back on the copy route
     (`SLINT_WINDOW_BUFFERS=1`, `ALWAYS_COPY`, two screen buffers,
     `boot1-slint.bin` `d76ee42c...`).
   - the copy-free route, done (01:30): the board layer hands out
     `YAKOGL_BOARD_IMX8MP_WINDOWS` windows (2 here, each its own carve-out
     buffer), the sample renders into them in turn and shows each with
     the in-place flip, `ALWAYS_COPY` off. The owner: 「起動直後は２秒くらい
     描画止まるけど、その後はさっきより早い」 -- it works and is faster
     than the copy route; the picture stands about 2 s after it appears,
     then animates. This is what boot1 holds (`boot1-slint.bin`
     `67056666...`).
   - the 2 s stand-still was the first-frame check reading the whole
     1280x800 window back through the CPU from non-cacheable memory
     (4 MiB per pixel; the copy route's memcpy of the same bytes took
     ~100 ms, a per-pixel loop an order more). Off the 1024x600 design
     size the check no longer runs (main.c); `boot1-slint.bin`
     `f5960887...`. The owner: 「起動直後に、数字は動いてる。左のゲージは
     ゆっくり動いてて２秒後くらいから早く動き出す。そのタイミングで右も
     動き出す。」 That is the demo's own script, not the presentation:
     `demo_vehicle()` (rust/src/lib.rs) holds the speed at 0 and ramps
     the rpm from 800 to 1000 for the first 3 s, then ramps the speed
     0 -> 120 km/h over 12 s with the rpm cycling 1500 -> 6000 every 3 s;
     trip, odometer and the clock count from t = 0. The same on every
     board.
5. **Route 3, RAM trial** (2026-10-22, about 10:30 JST; the recovery
   jumper, `uuu -b spl boot1-slint-route3.bin` (`2cdd15dc...`), nothing
   written; uuu's SDPS progress stopped at about 12 % as on every uuu boot
   here, and the image ran). Two camera frames about 10 s apart
   (`imx8mp-route3-ram-{1,2}-20261022-1030.jpg` in the deps directory):
   the cluster animates, 0 -> 94 km/h, D1 -> D5, the clock 03:10 -> 03:36.
   What the picture proves, against the code:
   - **The M7 composites.** The speed digits and km/h, the gear, the READY
     pill, the telltale row (the left arrow lit) and the fuel segments and
     litres are the safe-core elements, which cluster.slint draws only
     `if root.standalone`. Route 3 sets `standalone` false from the first
     draw, and the fallback keeps it false, so the A53 drew none of these.
     They are on the panel, so the M7's safety layer put them there, in
     place, at the Layout positions it shares with the A53.
   - **The A53 draws from the M7's page.** The A53's gauge arc, TRIP/ODO
     (2.82 km, 2.8 km; 2829 m both) and the clock are not
     `demo_vehicle()`'s values: that drive starts the odometer at
     123.456 km and the fuel at 27.5 L (the A53-only run showed 123.8 km
     and 26.6 L). The M7's demo model has trip equal to odometer and
     about 3.7 L. The A53's arc agrees with the M7's digits (0 with 0, about
     94 with 94). The fallback would have drawn `demo_vehicle()` without
     the safe-core elements.
   - **The M7 flips.** The A53 writes the LCDIF only once, for frame 1,
     and again only in the fallback. A moving picture with the M7's values
     is therefore the M7 flipping to the frames it took from the ring.
   - Not decided by the photos: the flat `bg-bottom` background. Both
     pictures are dark and the gradient is subtle; the code path is the
     only evidence. Not measured: frame rate, and the time from power-on
     to the first frame and to the M7's first composite (no UART).
6. **Route 3 at power-on from boot1** (2026-10-22 morning): recovery,
   Toradex's boot0 over `uuu`, Torizon, and `boot1-slint-route3.bin`
   (`2cdd15dc...`) written to boot1 and read back. `mmc bootpart enable
   2 1`, then a power-cycle. The owner: 「すごい描画が滑らか。最初の３フレーム
   くらいがちゃんと描けてない」. Route 3 runs at power-on and is visibly
   smoother than the A53-only image (one GPU render per frame, the M7's
   flips, no copy). The first frames were wrong, and the code explains
   them:
   - frame 1 was drawn with `demo_vehicle(0, 0)` (demo odometer 123 km)
     and, `standalone` being false, without the M7's elements;
   - the A53 lit the backlight at frame 1, so that picture stood on the
     panel for the whole M7 boot;
   - the M7's first composite went over that same frame 1, with the demo
     values;
   - frame 2 was rendered right after the release, before the M7's page
     existed (zeros), and only then did the M7's values arrive, with
     Slint's animations from there.
   Fixed in the next image (item 7).
7. **The first-frames fix** (built 2026-10-22 10:51, not yet run;
   `boot1-slint-route3.bin` `3d1463e2...`, superseded by item 8; the loader still from before
   `dc608ada`):
   - the backlight stays off at frame 1; the M7 turns PWM3 on when it
     adopts the LCDIF (`lcdif_m7_backlight_ensure_on`, right before its
     first flip of a frame with its layer in it). The A53 calls
     `imx8mp_display_backlight_on()` once the M7 has shown a frame (it
     then only cancels the 5 s fallback, or lights the panel if the M7
     did not), or when the M7 is absent;
   - frame 1 shows a car at rest (zeros), not `demo_vehicle(0, 0)`;
   - until the M7's first vehicle publish, the next frame waits for it
     (bounded by `..._ROUTE3_M7_WAIT_MS`; the console says after how
     long).
   The delay this adds to the first picture is the M7's own start, from
   the release (right after frame 1 is published) to its first flip: its
   Zephyr start (the `.data` copy and 3.2 MB of `.bss` cleared from DDR),
   the MPU and caches, the Slint safety layer's init, one model tick, its
   render and composite (3.7 + 2.6 ms measured on Torizon on 2026-10-03)
   and the wait for the next VSYNC (up to 17 ms). Nothing of the M7's start
   to its first frame was ever timed. The estimate is tens of milliseconds,
   well under 100 ms, and unmeasured until a camera burst or a UART reads
   the A53's line "route3: the M7's first frame on the panel within N ms
   of the release". No other wait is added: the A53 renders frame 2 in
   parallel and the M7 composites whichever frame is newest. If the M7
   never shows a frame, the panel still lights: at 3 s when no status
   block appears (the M7 is declared absent and the A53 lights it), and in
   every other case the display driver's 5 s fallback, which only the M7's
   first shown frame or the absent path cancels.
   What to look for: from power-on, a dark panel until the cluster
   appears with the M7's digits and telltales already on it, then no jump
   in the values. The backlight now comes from the M7's PWM3 programming
   (full brightness), which this board has not run from cold before; if
   the panel stays dark, that is the first suspect.
8. **Dark after about 11 minutes** (2026-10-22, boot1 `2cdd15dc`, powered
   on about 10:44, dark about 10:55 with nothing touched; Torizon not up,
   no ROM on USB). Not route 3: the image's own watchdog bound. WDOG1 is
   self-armed (60 s timeout, `WDOG_B` on) and fed only until
   `GC_BOARD_WDOG_SECONDS` = `YAKOGL_BOARD_LIFETIME_S` = 300 s of uptime,
   so every direct image resets 300-360 s after Zephyr starts, long before
   `ANIMATE_S` = 600 and its hold. The A53-only image (`f5960887`) has the
   same bound. Route 3 itself would have carried on: the hold sleeps with
   the 100 ms heartbeat still beating (a `k_timer`) and the last frame
   published, so the M7 would have kept showing it. Why the panel then
   stayed dark instead of the image booting again (`WDOG_B` reset the board
   to SPL on every kexec run) is not known: no record survives the
   recovery's power cycle, and no warm restart of a direct image was ever
   watched. A power-cycle without the jumper says whether a cold start
   still brings it up.
   Fix (`boot1-slint-route3.bin` `5206c9c6...`, includes item 7):
   the board layer's slint direct conf
   (`driver/zephyr/lib/board/imx8mp/slint_direct.conf`, so the A53-only
   image and route 3 alike) sets `SLINT_ANIMATE_FOREVER`
   (the render loop never ends) and `GC_BOARD_WDOG_UNBOUNDED` (WDOG1 fed for
   as long as the loop feeds `board_watchdog_feed()` within
   `YAKOGL_BOARD_ALIVE_S`; a hung loop still resets the board). Both default
   off; a measurement conf keeps the 300 s bound. The A53-only image with
   it: `boot1-slint-forever.bin` (`3d9ded7f...`, a spare, not written).
9. **Route 3 with both fixes at power-on** (2026-10-22, boot1
   `5206c9c6...` written from Torizon, read back `5206c9c63656c21f`, boot0
   unchanged, PARTITION_CONFIG 0x50; power-cycle). The owner:
   「１フレーム目からきれい」 -- clean from the very first frame. The panel
   now lights at the M7's first flip, with its layer and the M7's values
   already in the frame. Whether it is still running after 10-15 minutes
   (item 8) is to be recorded here.

**Back to Torizon**: from Torizon, `sudo mmc bootpart enable 1 1
/dev/mmcblk2`; without Linux, the recovery mode (the carrier's recovery
button held at power-on, the USB device-side connector to the host) and
`uuu -b spl <boot0 dump>` boots Toradex's own image from RAM, whose U-Boot
boots Torizon; the dumps are in the deps directory.

**Procedure, as it was planned:**

The board has no UART on the carrier and no SNVS record on this route, so
the picture and the camera are the result channel.

**First, without writing anything: the image from RAM.** The i.MX8MP's
serial download mode (the carrier's recovery button held at power-on, the
USB device-side connector to the host) takes a whole imx-boot image over USB and runs it as if the ROM
had read it from the eMMC: `uuu -b spl build/imx8mp/direct/boot1-triangle.bin`
(NXP's `uuu`, built from source into the build host's deps directory; it
needs USB access, so `sudo` or a udev rule). SPL -> BL31 -> loader -> the
triangle, the panel from cold, the boot-time lines on the panel console; the
eMMC is untouched and the next power-cycle boots Torizon. This is the whole
chain minus the eMMC read, so it is the real first test.

**Then the boot1 flip**, the one reversible way to boot the image at power-on:

1. **Read-only** (done 2026-10-21): boot0/boot1 sizes and PARTITION_CONFIG,
   both partitions dumped, the user-area layout (one partition from sector
   8192; sectors 64..8191 are zero).
2. **Back up** boot0 and boot1 (done; keep the copies).
3. **Write boot1 only** with the whole boot image (`echo 0 >
   /sys/block/mmcblk2boot1/force_ro; dd ... of=/dev/mmcblk2boot1`), read it
   back and compare, then enable boot1: `mmc bootpart enable 2 1 /dev/mmcblk2`
   (boot1, with the boot acknowledge bit as today's 0x48 has it). boot0
   (Toradex's bootloader and U-Boot's environment) and the user area are not
   touched.
4. Power-cycle. A picture within the hold time is the pass; a dark panel is
   a fail, and the next step is the recovery below (no information comes
   back otherwise: the first debug aid is the panel console, which needs
   the display to come up).

## Route 3: the M7 from the direct image

Design of 2026-10-22; first run on the board the same morning (RAM trial,
"At the board", item 5). Route 3 is a split: the
Cortex-M7 runs the safety firmware, owns the vehicle state and the panel
once it has adopted it. The A53 renders the rich UI with the GPU and hands
each finished frame to the M7 through a ring in shared DDR. The M7 blends its
safety layer into that frame and flips the LCDIF to it. The A53 draws only
from the vehicle state the M7 publishes, and when the A53 stops, the M7 shows
its own picture. The 2026-10-03 board runs on Torizon passed this way at
45 fps and later 58 fps. There, U-Boot and Linux brought the panel up, a
systemd unit (`route3_boot.sh`) paused
the demo container and started the M7 through remoteproc, and the A53 image
came later over kexec. On this route the direct image does all of it.

**The M7 firmware.** A separate build (commit `adf4b94`), built for
`verdin_imx8mp/mimx8ml8/m7/ddr` with the M7's take-over-at-boot option set
(Zephyr af17c0c8, Slint from the slint-zephyr tree). The result is
`m7_4e43a5dc.elf` (build id 0x4e43a5dc, sha256 `19d39ba6...`) in the
build host's deps directory, next to the script that built it. The earlier
`m7_b87c5576.elf` cannot be used: it predates the M7's page (frame
ring version 2), so it would refuse this producer's version-3 ring and
publishes no vehicle state. The board-tested `cfc7c618` build of the same
branch exists only on the board's Torizon. Its `PT_LOAD`s (`readelf -l`):

| paddr (M7 view) | A53 address | filesz / memsz | what |
|---|---|---|---|
| 0x00000000 | 0x007E0000 (ITCM) | 0x2bc | the vector table (`rom_start`): initial SP and reset vector |
| 0x80000000 | 0x80000000 | 0x16a140 | text, rodata |
| 0x8016a140 | 0x8016a140 | 0xe8 | `.data`'s load image (VMA 0x80200000; Zephyr copies it) |
| 0x8016a228 | 0x8016a228 | 0x4c | the resource table (no Linux reads it; it is not waited on) |
| 0x802000e8 | 0x802000e8 | 0 / 0x3270a4 | bss, noinit |
| 0x81001000 | 0x81001000 | 0 / 0x4000 | the M7's RAM console |

The entry point 0x8005a3fd is reached through the vector table. The i.MX8MP
M7 boots from its ITCM at address 0, as remoteproc's `imx_rproc_att_imx8mn`
maps it (dev 0x0 is sys 0x7E0000). Each segment is loaded at its physical
address, as remoteproc does: `filesz` bytes copied, the rest of `memsz`
zeroed.

**Carried in the direct image.** The ELF goes into `.rodata` of the slint
direct image (`.incbin`), stripped at build time to the ELF header, the
program headers and the segment bytes (`m7_image.py`, about 1.5 MB). The
image is deflated inside the loader's payload, so it costs the boot partition
its compressed size (measured at the build below). The loader stays generic;
the board layer parses the ELF32 headers at run time.

**The release.** The board's BL31 is NXP's `v2.10.0 (release):lf-6.6.52-2.2.1`
(the strings in boot0's `atf` image; not v2.13). Its SiP handler for
0xC2000005 (`IMX_SIP_SRC`, which Linux's `imx_rproc` calls `IMX_SIP_RPROC`)
was read from the disassembly of the board's own BL31:

- x1 = 0 (start): clears `IOMUXC_GPR22[0]` (CM7_CPUWAIT, 0x30340058) and
  returns 0;
- x1 = 1 (started): returns `!(GPR22 & 1)`;
- x1 = 2 (stop): checks GPC 0x303a00ec, sets CPUWAIT and resets the core
  through the SRC.

This is the call Torizon's `remoteproc0` made on the 2026-10-03 runs
(`fsl,imx8mp-cm7` uses `imx_rproc_cfg_imx8mn`, the SMC method), with the
same BL31. The board layer issues `smc #0` with x0 = 0xC2000005 and x1 = 0.
If x0 comes back non-zero, it clears GPR22[0] itself (the `-mmio` variant's
start). TF-A touches no clock. The M7's own SoC init (Zephyr
`soc/nxp/imx/imx8m/m7/soc.c`) moves its core root to SYS_PLL1 800 MHz. The
A53 only checks that `CCM_TARGET_ROOT1` (the M7 core, 0x30388080) is enabled
and sets bit 28 if it is not. Linux enables that clock through `m7_core`
before the SMC.

**Order.** This firmware reads the LCDIF on its first tick
(`panel_show`). A read of the LCDIF while MEDIAMIX is unpowered is a bus
fault, so the M7 can be released only once the panel is running. The order
in the A53 image is:

1. Board-layer init: map the carve-out pieces (below), load the ELF, and set
   up the ring (`cluster_frame_ring_init`, buffers in the M7's app window).
   Nothing runs on the M7 yet.
2. The windows the sample renders into are the ring buffers, not GPU
   carve-out buffers (EGL imports them; `gc_render_map_phys`).
3. The first `board_show`: `imx8mp_display_on()` with the first frame's ring
   buffer (the C cold path, unchanged), the backlight, publish frame 1, start
   the heartbeat timer (100 ms), then the SMC. Because the ring is attached
   and holds a frame before the M7 runs, its first tick composites that frame
   and does not show its own stand-in. The A53 starts no latch thread and
   never writes an LCDIF register again.
4. Later shows publish frame S and wait (bounded, 200 ms) until the ring's
   `shown` (M7-written) reaches S. A buffer holding frame S-1 is the
   producer's again once `shown` has moved past S-1 (frame_ring.h's one
   rule), so with two windows the one the GPU draws next is read by neither
   the M7 nor the LCDIF. `yakogl_zephyr_wait_vblank` reports
   -ENOTSUP in route 3.
5. On every show until it holds, the A53 checks the M7's status block
   (`SAF7`, version >= 10, `fb_app`, `ring_addr`, 1280x800). If none appears
   within `YAKOGL_BOARD_IMX8MP_ROUTE3_M7_WAIT_MS`, the M7 is declared absent:
   the ring is dropped, the latch thread starts, and the A53 flips its own
   windows as on the plain direct route. The sample then draws its demo
   drive. The picture moves but has no M7 safety elements (`standalone`
   stays false), which tells the viewer the M7 did not come up.

**Shared memory** (the M7 firmware's memory map; the A53's mappings are Normal
non-cacheable, the attribute the 2026-10-03 producer used and the one
`gc_render` gives an import; the M7 maps the carve-out cacheable except its
uncached window and invalidates before it reads):

| address | size | what |
|---|---|---|
| 0x81000000 | 0x800 | M7 status block (`SAF7`, word offsets of `struct m7_status`) |
| 0x81000800 | | M7 control block (`CTL7`; TAKEOVER_AT_BOOT sets `takeover = 1`) |
| 0x81005000 | 0x800 | the frame ring (`<cluster/frame_ring.h>` version 3, 3 slots) |
| 0x81005800 | 160 B | the M7's page: vehicle state (seqlock `gen`), background descriptor |
| 0x81408000 | 4 MiB | `fb_app`, the M7's stand-in; never an A53 buffer |
| 0x81808000, 0x81c08000 | 2 x 4 MiB | the M7's scan-out pair |
| 0x82008000 + n x 4 MiB | 4 MiB each | A53 ring buffers (n = 0..3; the sample uses 2) |
| 0x83008000 | 4 MiB | the M7's background picture (not used: the A53 clears flat) |

The 0x80000000 + 64 MiB carve-out lies outside every region the direct image
or the GPU uses (0x48000000 + 256 MiB, 0x58000000 + 64 MiB). The ELF load
window (0x80000000 .. 0x80528000) is mapped only while loading. ITCM is
written through a device mapping with 32-bit stores, and a `dsb sy` precedes
the SMC. A PSCI system reset at the end of a run resets the M7 with the SoC
(GPR22 comes back as CPUWAIT), so every boot loads it again.

**The picture.** In route 3 the sample sets `standalone = false`: the M7
draws the safety elements, and the A53 draws the rest over a flat
`Layout.bg-bottom` (cluster.slint, which was `transparent` and scanned black;
femtovg clears to it, one full-screen clear, not the gradient). The vehicle
comes from the M7's page every frame: speed, rpm, gear, fuel, tank, odometer
and trip, the indicator as switch AND flasher phase, low beam, READY, and the
fuel and engine warnings. Before the first publish (`gen` 0), the values are
zero. `demo_vehicle()` is used only standalone or when the M7 is absent.

**Built (2026-10-22 05:10, not run).** `CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3`
(board layer, default n) and the slint sample's
`imx8mp_route3.conf`:

```sh
west build -p always -b verdin_imx8mp/mimx8ml8/a53/direct -d build/imx8mp/slint_direct_route3 \
    examples/slint -- <the slint direct arguments above> \
    -DEXTRA_CONF_FILE=imx8mp_route3.conf \
    '-DCONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_M7_ELF="<deps>/directboot/m7_4e43a5dc.elf"'
```

The direct image is 6.81 MB (5.32 MB without route 3: the stripped M7 is
1.48 MB of it). `stage2-slint-route3.img` is 2.14 MB deflated, and
`boot1-slint-route3.bin` is 2.70 MB against the 0x3FDE00 (4.19 MB) limit.
Its loader is built from the tree before `dc608ada` (hardware CRC-32 in the
loader, not yet run on this board), so route 3 is the only change from the
image that ran: sha256 `2cdd15dc...`. The same image with the `dc608ada`
loader is `boot1-slint-route3-loaderhwcrc.bin` (`ce1b8c59...`).

**The board window** (the first route-3 run):

1. RAM only, nothing written. Set the recovery jumper, power on, and run
   `uuu -b spl boot1-slint-route3.bin` (the image runs as if read from
   boot1). Remove the jumper afterwards; the next power-on boots boot1's
   A53-only image (`f5960887`) again.
2. Read the panel, with the camera:
   - pass: the cluster with the M7's speed digits and telltales over it, the
     needles moving with the M7's demo drive, and a flat background (not the
     gradient);
   - the cluster moving without those elements (about 3 s after the first
     picture): no M7 status block, the fallback (release or load failed);
   - the first picture standing still: an M7 that runs and never takes the
     panel, or one that refuses the frames (the ring verdict);
   - the M7's own stand-in picture: the M7 runs and does not accept our
     frames;
   - a dark panel: the image before the panel (as before route 3).
3. On a pass, write boot1 as before (recovery, Toradex's boot0 over `uuu`,
   Torizon, the write from there, `mmc bootpart enable 2 1`), then power-cycle. The default direct image (no route 3) is built from
the same tree with the board's conf alone. It has the same size as the
`f5960887` build and draws the same picture (`standalone` stays true, so the
gradient). It is not byte-identical, because the board layer's latch thread
start moved into a function.

**Risks.**

- The new M7 build differs from the board-tested `cfc7c618` (the same
  commit) only in its Slint tree. It has not run.
- The M7 ran under Linux before: Linux had enabled its clock and the
  MEDIAMIX/LCDIF clocks were the Linux driver's (69.3 MHz pixel clock). Here
  they are the C cold path's. The M7 adopts only 1280x800, 32 bpp, pitch 5120
  (`panel_show`), which is what this image programs.
- In-place compositing means the M7 writes into the buffer the LCDIF scans
  for the first frame (the A53 set that address). The first frame's safety
  layer may tear once.
- An A53 that dies leaves the heartbeat stopped, and the M7 shows its
  stand-in after 1.5 s (the design). An A53 that resets takes the M7 down
  with the SoC.
- No channel reports what happened: no UART, no Linux, and the panel console
  is off. The result is the picture: the cluster with the M7's digits and
  telltales and needles moving means pass. A moving cluster without them is
  the fallback (no M7). A still first frame means an M7 that runs but never
  flips. The camera records it. The M7's status block is readable only by a
  later Torizon boot (`mmio_rd 0x81000000`, if Torizon's U-Boot does not
  clear DDR).

## Recovery

- **Back to boot0 from Linux**: not available (Linux never runs on this
  route).
- **Back to boot0 from the recovery mode**: the Verdin iMX8MP's USB recovery
  (the carrier's recovery button at power-on, the USB device-side connector
  to the host) takes
  Toradex's imx-boot over `uuu` and runs its U-Boot from RAM; at its prompt,
  `mmc partconf 2 1 1 0` re-enables boot0, or Toradex Easy Installer
  reinstalls Torizon. The recovery U-Boot and the Easy Installer image are
  Toradex's downloads (not in this repository; their exact file names and
  checksums are to be recorded here when they are fetched).
- **Restore a backup**: from Linux (after the flip back), `dd` the saved
  boot1 dump over `mmcblk2boot1` with `force_ro` cleared.

## Risks and open questions

- Run on 2026-10-21 (above): the chain, the loader and the slint image at
  power-on; route 3 from RAM on 2026-10-22. The triangle image is not run.
- **The display from cold.** The C LCDIF1/DSIM path (`imx8mp_display_on`)
  brought the panel up from cold on the TFTP route (docs/notes/imx8mp-netboot.md);
  the Rust path (driver/docs/imx8mp-display-cold.md) has not run on a board.
  The direct confs use the C path.
- **SPL's devicetree append** lands inside the loader's RAM (0x40200000 +
  file size, 8-aligned): the loader's `.bss` starts there and is cleared by
  Zephyr's start; nothing reads the appended DTB.
- **Route 3 (the M7)**: "Route 3" above; RAM trial passed 2026-10-22.
- **Boot time**: no number yet.
