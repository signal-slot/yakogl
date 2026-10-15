<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin AM62P: black panel under yakogl after a kexec (2026-10-15)

Status: **root cause not found by the registers; the panel was not seen**.
The red-fill image ran on the board twice from hx90 (section "On the
board"): every register of the display path equals the Linux dump except
the deliberate deviations, the DSI streams, the bridge reports no error
with video flowing, and the board resets back into Linux. The camera that
sees this panel is on Lepton, which hx90 cannot use, so whether the panel
was red is still unknown; it is the one step left, under "What remains".
This note records what the tree says, what changed since the last picture
seen on the panel, the instrumentation now in the tree (built with -Werror
in both sample routes), the two board runs, and the procedure. Nothing in
the display drivers was changed. It supersedes the static part of the same
investigation of 2026-10-14 (pull request closed: the diagnostics were
never compiled by the gate and no note was written).

## The report

yakogl (Zephyr, kexec'd from Torizon by `driver/tools/kexec-zephyr/kexec_run.sh`)
runs; the backlight is on; the panel stays black. The U-Boot splash shows,
so panel, backlight and the LVDS timing work. (Which image and hold time
the report was made with is not recorded: with `PVR_SAMPLE_HOLD_SECONDS=0`
the sample resets the SoC right after its draw, and the panel is black until
U-Boot's splash. A run for this question needs a hold: 40 s in
`gl_triangle`'s AM62P `.conf`, 60 s in the board layer's `board_finish`.)

## Reaching the board

The task ran on hx90 with an exclusive lease on `verdin-am62p`. hx90 has the
farm's key for the board but Tailscale DNS was off there, so
`ssh torizon@verdin-am62p` failed at name resolution (`Could not resolve
hostname`), and ccfarm's own access record marks the board unreachable from
hx90 (reachable from Lepton, Lepton-local, XPS139340, nuc, system76). The
tailnet path itself worked (`tailscale ping` answers in 10 ms; `tailscale
dns status`: "Tailscale DNS: disabled", MagicDNS enabled tailnet-wide), so
the session ran `tailscale set --accept-dns=true` on hx90 (left on: with it
every tailnet board resolves there), after which the lease's ssh form
worked. The farm's serial lease (`usb-serial-1`) was held by another task;
its bridge's other port, the one wired to main_uart0 (Linux ttyS2,
docs/boards/am62p-console-lease.md, "port-33"), was free and carried the
probe `echo ... | sudo tee /dev/ttyS2` verbatim, so the console was read
from it (a read-only TCP client; nothing was written to the bridge).

Readout after a reset, tested on the way: DDR (a pattern at a no-map
address read back as erased bits after `systemctl reboot`), the main
OCSRAM at 0x70000000 (overwritten by the boot) and the MCU SRAM at
0x79100000 (zeroed) all lose their contents, so there is no RAM channel
for a run's output; the console is the only one, as the tree already said.

## What the tree settles without the board

- **The last picture seen on this panel from yakogl was the C drivers'**:
  `driver/samples/zephyr/gl_triangle/README.md`, "Verified on the board on
  2026-09-14: the triangle renders on the panel (camera)". Every later
  record (`am62p-display-rust-2026-10-02.md`, `am62p-common-main.md`:
  "Visual check: not done") judged the display by counters, the bridge's
  IRQ_STAT and GPU read-backs only. On 2026-10-02 the C-driver image of the
  same tree was run for its counters, not looked at, so it is not known
  whether the panel was already black then with the C drivers. The
  regression window is therefore 2026-09-14 to the report: the Rust port of
  the bridge (`ip::sn65dsi8x`), of the DSS and DSI/D-PHY
  (`driver/docs/am62p-display-cold.md`), and everything else that moved
  (kexec route, GPU, samples) in between.
- **Powered-down peripherals are not it for the samples**: every AM62P
  sample overlay enables `tisci_power_on` (I2C1 103, GPIO0 77, GPIO1 78;
  `driver/samples/zephyr/common/verdin_am62p_display.dtsi` and the
  per-sample overlays), and "backlight on" is the last line of the bridge's
  self-start (`bridge_sn65dsi8x.c`, `sn_self_start`: DSI attach, EN, ID read
  over I2C1, CSRs, PLL_EN_STAT, soft reset, IRQ_STAT read, then the backlight
  GPIO). So I2C1, GPIO0 (EN, reset) and GPIO1 (backlight) work and the
  bridge answered. The DSS counters of 2026-10-02 (58.8 Hz vsyncs, 0
  underflows, 0 sync lost) show VP2 scanning.
- **The port is enabled, the planes are set**: `sample_glue.c`
  `show_on_dss()` calls `am62p_dss_set_app_buffer` (XRGB8888),
  `am62p_dss_enable_app_plane(true)` and `am62p_dss_output_enable(true)`,
  as it did in the C era; `dss_init` leaves the port off in both. The board
  layer (`board_am62p.c` `board_show`) goes through the same two calls.
- **The safety layer is transparent**: both `__noinit` safety buffers are
  zeroed and cache-flushed at init (`display_am62p_dss_rust.c` `dss_init`),
  ARGB8888 straight alpha (`dss.rs` `safety_plane`, `plane_configure` with
  PREMULTIPLYALPHA 0, GLOBAL_ALPHA 0xff): alpha 0 everywhere, so the
  application plane on layer 0 shows through layer 1.
- **Register images equal the Linux dump** (`driver/docs/am62p-display-dump-linux.txt`)
  per the host golden test, except the allow-listed, deliberate deviations
  (`driver/docs/am62p-display-allow.txt`) listed below; the bridge's CSR
  image equals Linux's i2cdump except 0x1A. The model, not the board, was
  compared: the board's own image is what `CONFIG_PVR_SAMPLE_DISPLAY_DUMP`
  prints.
- **The DSI VID_MAIN_CTL and the bridge's init order match Linux**: Linux
  enables the CRTC (DSS VP) first, then pre-enables cdns-dsi (link, clock
  lane HS), then the bridge (EN, CSRs, PLL, soft reset), then enables
  cdns-dsi (video) and the bridge (IRQ_STAT check). yakogl: DSI attach
  (link, clock HS, VID_EN, IF2_EN with no DPI stream yet), bridge, then the
  sample starts VP2. The video reaches the bridge after its soft reset in
  both, as the data sheet's init sequence asks.
- **The bridge's IRQ_STAT was never read with video flowing**: the
  `sn65dsi8x link clean (IRQ_STAT 0x00)` line of 2026-10-02 is printed by
  the self-start, before the sample enables VP2. A CHA_SYNCH_ERR or an LLP
  error that only video can raise was not visible. The dump
  (`CONFIG_PVR_SAMPLE_DISPLAY_DUMP`) now prints IRQ_STAT before and after
  the hold, with VP2 running.

## Differences between the verified C flow and the Rust flow

Each is a candidate; the board decides. Most likely first (order and
timing a register image cannot show):

| # | Where | C (seen on the panel 2026-09-14) | Rust (never seen) | Note |
|---|---|---|---|---|
| a | bridge order | reset release, **EN high, ID read, then `mipi_dsi_attach`** | `mipi_dsi_attach` first, then EN low 10 ms, EN high 10 ms, ID | Rust follows SLLSEC1I 7.4.1 "DSI clock in HS before EN"; Linux does the same (`pre_enable_prev_first`) |
| b | DSI VID_VSIZE1 | VSA 11, VBP 22, VFP 12 (= Linux's dump 0xc58b) | VSA 10, VBP 23, VFP 11 (TRM 12.9.2.7.9.1) | VSA+VBP equal (33); VFP one line short on purpose |
| c | MCTL_MAIN_PHY_CTL WAIT_BURST_TIME | 0 (Linux too) | 4 (TRM: 0 forbidden) | |
| d | lane start | DAT/CLK lanes started with PLL_START, before the D-PHY is programmed | lanes started after PLL_LCK (Figure 12-551) | lanes report READY in both |
| e | wrapper DPI_0_EN | first write of attach | just before VID_EN / IF2_EN | Linux j721e: late, as Rust |
| f | MCTL_DPHY_TIMEOUT1/2, ULPOUT | a frame / 1 ms of sys clocks (overflowing 9 bits) | maximum | |
| g | VID_DPHY_TIME REG_LINE_DURATION | 1012 | 1011 | unused in blanking-packet mode |
| h | DSS FIFO low / preload, MFLAG, IRQ_EOI | size/2, 2/3-1/3, EOI 0 | size-8 = preload, 1/2-1/4, EOI 1 | 0 underflows measured with Rust |
| i | bridge 0x1A | 0x00 | 0x01 (CHB termination left at reset; Linux 0x03) | channel B unused |

The D-PHY sequence (PSM, PWM 0x40, PLL dividers 2/8/394, LANE_RSTB_CMN,
band 5, SSM) and the three MCTL_DPHY_CFG0 values (0x7f3, 0x3f1, 0x1003f1 |
D_RSTB) are the same in both.

## Instrumentation in the tree

- `CONFIG_PVR_SAMPLE_DISPLAY_DUMP=y`: the read list of the Linux dump
  printed with the DSS counters before and after the hold, plus the
  bridge's IRQ_STAT and health while video runs
  (`driver/zephyr/lib/board/am62p/display_dump.c`, reads only).
- `CONFIG_PVR_SAMPLE_DISPLAY_FILL=<argb>`: before the hold, the DSS safety
  layer (VIDL1, above the application plane) is filled by the CPU with that
  opaque colour, flipped in, and the port enabled
  (`sample_glue.c` `fill_safety_layer`). The panel must show the colour
  whatever the GPU and the application plane did: black with the fill means
  the DSS/DSI/bridge path; the colour without the triangle means the
  application plane (buffer, format, layer 0 enable).
- Both run in both sample routes: `pvr_sample_finish` (the samples on
  `samples/zephyr/common/sample.cmake`, `gl_triangle`) and the board
  layer's `board_finish` (`CONFIG_YAKOGL_BOARD`: `triangle`, `glmark2`,
  `slint`, `cluster`), through `pvr_sample_before_hold` /
  `pvr_sample_after_hold` around the hold.

## Build (hx90, 2026-10-15)

Zephyr workspace af17c0c8aa2a, SDK 1.0.1, rustc 1.101.0-nightly, from this
tree with `-DYAKOGL_WERROR=ON -DCONFIG_PVR_SAMPLE_DISPLAY_DUMP=y
-DCONFIG_PVR_SAMPLE_DISPLAY_FILL=0xffff0000` (the build directories are
not committed):

| image | route | result |
|---|---|---|
| `gl_triangle` (`-DCONFIG_PVR_SAMPLE_HOLD_SECONDS=40`) | `samples/zephyr/common/sample.cmake`, no `CONFIG_YAKOGL_BOARD` | rc 0, zephyr.bin 1,743,472 B, RAM 37252 KB of 344 MB; `.config` has `DISPLAY_DUMP=y`, `DISPLAY_FILL=0xffff0000`, `HOLD_SECONDS=40`; `sample_glue.c.obj` and `display_dump.c.obj` in the link map; the only warning is Zephyr's own `tisci.c` (outside -Werror, as the gate notes) |
| `glmark2` (`-DGLMARK2_SRC=<checkout>`) | `zephyr/lib/board`, `CONFIG_YAKOGL_BOARD_AM62P` | rc 0, zephyr.bin 12,940,128 B; `.config` has `YAKOGL_BOARD=y`, `DISPLAY_DUMP=y`, `DISPLAY_FILL=0xffff0000`, `BOARD_HOLD_S=20`; both objects in the link map; 0 warnings |

(The board-layer `triangle` sample was not built here: its own Rust
application needs `-Z build-std=core` with `rust-src`, which this node's
nightly lacks; `glmark2` exercises the same board-layer route.)

Gate, `tools/check.sh --changed 3cc4b60acf33`: 44 ok, 0 failed, 0 gaps,
105 skipped (scopes docs, drivers, samples, tools; the three Zephyr -Werror
images with the options off, as the gate builds them, included).

Neither was run on the board.

## On the board (2026-10-15, hx90)

Torizon 6.6.138-7.6.1 (`verdin-am62p-08641335`), the LVDS-1 connector
`connected`/`enabled` under Linux (fbcon on tidss; no compositor), modules
tidss, cdns_dsi, cdns_dphy, ti_sn65dsi83, pvrsrvkm loaded; MAIN RTI0
`active`, 60 s. The bridge's i2cdump under Linux equals the one in
`driver/docs/am62p-display-dump-linux.txt` byte for byte. The kexec loader
was built from `driver/tools/kexec-zephyr/kexec_zephyr.c` with the Zephyr
SDK's aarch64 gcc (the README line) and run by hand as `kexec_run.sh` does
(rmmod of the GPU modules, `./kexec_zephyr ./zephyr.bin`, the loader's
watchdog gate: `RTI0 DWDCTRL = 0xa98559da: running`). Both jumps took at
the first try.

The image: `gl_triangle` from this tree (`sample: build tree=3cc4b60a-dirty`,
the diff of this commit before it was committed), `DISPLAY_DUMP=y`,
`DISPLAY_FILL=0xffff0000`, `HOLD_SECONDS=40`, zephyr.bin sha256
`c58e252e…54f6`. Run 1's capture started late (the port delivered nothing
until the hold's end; cause unknown, a marker test precedes every later
capture); run 2 is complete and is the record
`am62p-display-black-2026-10-15-console.txt` beside this note. Its readings:

- Init: `Cadence DSI id 0xcadd5131`, `WIZ at +0xf00 (MOD_VER 0x69924881)`,
  `VP clock source 51200000 Hz`, `DSS rev 0x64040901`, `DSS ready: VP2
  1024x600`, `D-PHY: 307812500 bit/s per lane, band 5`, `DSI link up: 4
  lanes, sync events, line 4047 B`, `sn65dsi8x: ID "58ISD" rev 0x01`, `up,
  DSI 153600 kHz (range 0x1e), LVDS 51200 kHz = /3 (range 1), fmt 0x68`,
  `link clean (IRQ_STAT 0x00)`: the same lines as 2026-10-02.
- The triangle: `centre pixel r=128 g=63 b=64 a=255 (GL error 0x0)`, record
  `594b0400 ff403f80 35706c4e 04000258`, the 2026-10-02 record.
- `safety layer 1024x600 filled with 0xffff0000, port on`; before the hold
  `vsyncs 0 app_frames 1 ... safety_flips 1 app 1`; after 40 s `vsyncs 2386
  app_frames 1 underflows 0 sync_lost 0 go_timeouts 0 go_waits 0
  safety_flips 1 app 1` (58.8 Hz, as before).
- Both dumps against the Linux dump with the allow-list (a 40-line python
  diff, not committed): **no difference outside the allow-list except the
  application plane being on** (VID_ATTRIBUTES 0x4f = ENABLE | XRGB8888,
  VID_SIZE and the scaler size 0x025703ff: Linux has VID off). The
  allow-listed ones are the buffers (VIDL1 BA 0x83f49000, the red back
  buffer after the flip; VID BA 0x83cf1000, the window), the TRM FIFO
  thresholds, our VSYNC enable, the two-layer OVR2 (ATTRIBUTES_0 0x1 = VID,
  ATTRIBUTES_1 0x3 = VIDL1), VIDL1 ARGB8888 straight alpha (0x0f), no
  gamma, WAIT_BURST_TIME 4, the maximum time-outs, VSIZE1 0xb5ca, and live
  status. VP2 CONTROL 0x341 (enabled, 24 lines) as Linux. DSI MCTL_MAIN_STS
  0x3f, MCTL_MAIN_STS_FLAG 0x3e, MCTL_LANE_STS 0x4054a, VID_MODE_STS 0x1
  (stream running, no live error): all equal to Linux's. VID_VPOS advances
  (0x32a, 0x6a). VID_MODE_STS_FLAG 0x403 (Linux 0x1): two sticky error
  flags raised once, already present in the before-hold dump, i.e. before
  the port ran (VID_EN with no DPI stream yet); none live.
- The bridge with video flowing: `IRQ_STAT 0 (0x00), health 0xf` before
  and after the hold (the first such reading; 2026-10-02 read it only at
  the self-start).
- `sample done (0): resetting the SoC`, then U-Boot's splash boot and the
  Torizon login prompt: the image comes back as designed.

So by every register and counter the path is live with Linux's own values:
DSS scanning the red safety layer over the triangle, DSI streaming without
live errors, the bridge locked and error-free. What the registers cannot
say is the picture. No camera was reachable: the lab's document camera is
`/dev/video2` on Lepton (farm notes), and hx90 has no lease on it.

## What remains

One look at the panel during a run of this image, from Lepton (the camera)
or by eye: red means the DSS/DSI/bridge path is fine and the owner's black
panel is the application plane or the image/hold they ran (a `HOLD_SECONDS=0`
image resets at once); black with this image means the fault is below the
registers (D-PHY signalling, LVDS mapping, the panel's own timing window)
and the bisection of the table above starts with (a) and (b). The
procedure below is unchanged.

## Board procedure (what decides it)

From a node that reaches `torizon@verdin-am62p` (Lepton, nuc, system76; hx90
after `tailscale set --accept-dns=true`) and, for the picture, the lab's
document camera on Lepton. Build as above (docs/es3-zephyr-env.md, "Gate
rows"):

```sh
cd "$ZEPHYR_WS"; mods=$("$REPO/tools/cts/zephyr-modules.sh" "$ZEPHYR_WS")
.venv/bin/west build -p always -b verdin_am62p/am62p54/a53 -d "$REPO/build/gl-fill" \
  "$REPO/driver/samples/zephyr/gl_triangle" -- "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$REPO" \
  -DZEPHYR_TOOLCHAIN_VARIANT=zephyr "-DZEPHYR_SDK_INSTALL_DIR=$SDK" "-DYAKOGL_CARGO=$HOME/.cargo/bin/cargo" \
  -DCONFIG_PVR_SAMPLE_DISPLAY_DUMP=y -DCONFIG_PVR_SAMPLE_DISPLAY_FILL=0xffff0000 -DCONFIG_PVR_SAMPLE_HOLD_SECONDS=40
```

1. Reference, from Linux before the jump: `sudo i2cdump -f -y 2 0x2c` and
   the read list with `devmem2` (the 2026-10-01 dump already holds it);
   photograph the U-Boot splash and the Linux console.
2. Console on main_uart0 (Linux ttyS2; the farm's `usb-serial-1` may carry
   mcu_uart0, docs/boards/am62p-console-lease.md). Jump:
   `BOARD_HOST=torizon@verdin-am62p driver/tools/kexec-zephyr/kexec_run.sh build/gl-fill/zephyr/zephyr.bin`
   (retry on a silent jump; about one in six).
3. Photograph the panel 15-20 s after the jump (the V4L2 device by name,
   never the EMEET SmartCam; ffmpeg or camera4screen).
4. Read the console: `safety layer 1024x600 filled with 0xffff0000, port on`,
   the `dss stats` lines, the `display dump before hold` / `after hold`
   blocks, and `# sn65dsi8x IRQ_STAT`.
   - **Red panel**: the path works; the fault is the application plane.
     Diff VID (0x30226020..) and OVR2 layer 0 (0x30228020) against the
     intended values (ENABLE, FORMAT 0x27, BA = the window's phys, SIZE,
     ROW_INC 1) and check `app 1` in the stats.
   - **Black panel**: the path. Diff the "after hold" block against
     `driver/docs/am62p-display-dump-linux.txt` with the allow-list; look
     first at VP2 CONTROL 0x3022b004 (ENABLE), DSI MCTL_MAIN_STS 0x30500024
     and MCTL_MAIN_STS_FLAG 0x30500170, VID_MODE_STS 0x305000f0 and its
     FLAG 0x30500180 (errors once video flows), VID_VPOS 0x305000e8
     (advancing?), and the bridge IRQ_STAT (bit 7 CHA_SYNCH_ERR, 6 CRC,
     3 LLP, 0 PLL_UNLOCK). Then bisect the table above, one line at a
     time, (a) and (b) first; each change keeps its reason and the host
     tests (`cargo test -p yakogl-drivers`: the register-image and golden
     tests pin the current values, so an expectation and an allow-list
     line move with it).
5. Confirm with the triangle (`DISPLAY_FILL=0`) by camera, and record the
   photos here.
