<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_gpu_scanout

The STM32MP257F-DK's GPU-rendered frame on its HDMI (issues #7 and #1281,
plan #353): `gc_triangle`'s triangle, drawn by the GC8000 at the display
mode's size, copied by the resolve engine into a linear XRGB8888 buffer, and
that buffer scanned out by the LTDC (LTDC -> DSI -> ADV7535 -> HDMI).

```
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_gpu_scanout \
  tools/probes/stm32mp257f_dk/mp2_gpu_scanout
```

(from the top of this repository, with `ZEPHYR_BASE` the workspace's
`zephyr` of `west.yml`'s pinned revision and `ZEPHYR_SDK_INSTALL_DIR` the
SDK; `-- -DYAKOGL_WERROR=ON` makes warnings errors.)

## What it does

The board layer powers the GPU (`gc_power_mp2.c`) and the display path
(`display_power_mp2.c`) as for `gc_triangle` and `mp2_display`; every SCMI
request is on the console (`gc_power: ...`, `display_power: ...`).
`src/main.c` then:

1. asks the LTDC device (`display_yakogl_stm32_ltdc.c`, the chain owner) for
   its capabilities, which reads HPD and the EDID through the ADV7535 and
   chooses the mode: 1920x1080@60 when the EDID prefers it, else
   1280x720@60. The buffer's pitch is `ROUND_UP(W * 4, 64)`, which the probe
   requires to be the LTDC driver's pitch (`stm32_ltdc_set_fb` keeps it);
2. does `gc_triangle`'s bring-up and draw at that size (no Kconfig size):
   the GPU device, `gc_render_init`, a W x H render target with two
   poisoned guard rows, `gc_draw_clear` to `0x00102030`, one Gouraud
   triangle (red top, green bottom left, blue bottom right, clip
   coordinates +-0.8), and the three-apex CPU read-back with `gc_triangle`'s
   lines and thresholds;
3. does `gc_triangle`'s "linear scan-out copy" at the same size:
   `gc_draw_copy` into a second buffer from `gc_render_alloc` (4096-byte
   aligned, in the GPU carveout at 0xfa800000, RISAF4 region 10), poisoned
   first, and the same checks on the copy plus a pixel-by-pixel compare;
4. cleans the buffer's range (`gc_render_cache_flush`, item 3 of "Cache
   maintenance" in `docs/notes/stm32mp257f-dk-display-bringup.md`), checks
   it with the board's `stm32_ltdc_board_fb_check` (RIF/RISAF), brings the
   chain up (`display_blanking_off`: LTDC on the driver's own buffer at
   0xb0000000, DSI, ADV7535) and flips the layer to the buffer's `bo.phys`
   (`stm32_ltdc_set_fb`, which waits for the reload). No address comes from
   Kconfig. The format needs no conversion: the PE's X8R8G8B8 is the word
   `0x00RRGGBB`, the LTDC's format 0 (ARGB8888) reads it unchanged ("GPU
   buffer format" in the note);
5. waits 60 frames, each with no transfer or underrun error
   (`stm32_ltdc_wait_frame`). The LTDC's frame CRC is not in the driver's C
   ABI (`yakogl_drv/stm32_ltdc.h`), so there is no CRC check: PASS rests on
   the read-back, the flip's read-back of the scanned address and the frame
   flags; the picture on the KVM.

`PASS` comes only after both the read-back and the scan-out succeeded. The
probe then keeps scanning out for `CONFIG_MP2_GPU_SCANOUT_HOLD_S` seconds
(default 60; one frame checked a second), also after a FAIL once the chain
is up, so the runner's `MP2_HOLD_S` and the KVM can see the picture.
`prj.conf` raises the layer's `CONFIG_YAKOGL_STM32MP2_LIFETIME_S` to 150; a
`BUILD_ASSERT` keeps the two consistent. OP-TEE's watchdog is armed and
petted by the layer (`CONFIG_YAKOGL_STM32MP2_WDOG`).

The overlay (`boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay`) is
`mp2_display`'s display nodes unchanged plus `&gpu` as the board layer's
`app.overlay` has it for `gc_triangle`.

## Expected lines

At 1280x720 (the pixel values only show the shape; they are not a
measurement):

```
mp2_gpu_scanout: build <commit>, hold 60 s
mp2_gpu_scanout: mode 1280x720, buffer pitch 5120
mp2_gpu_scanout: GPU model 0x8000 revision 0x6205 product 0x... customer 0x... eco N
mp2_gpu_scanout: target 1280x720 stride 5120 at GPU 0x........
mp2_gpu_scanout: red apex   PASS (640,110) = f31010ff
mp2_gpu_scanout: green left PASS (179,628) = 10f010ff
mp2_gpu_scanout: blue right PASS (1100,628) = 1010f0ff
mp2_gpu_scanout: outside     PASS (background kept outside the box and in the guard rows)
mp2_gpu_scanout: covered     PASS 294... pixels (235929..353894 expected)
mp2_gpu_scanout: linear: red apex   PASS (640,110) = f31010ff
mp2_gpu_scanout: linear: green left PASS (179,628) = 10f010ff
mp2_gpu_scanout: linear: blue right PASS (1100,628) = 1010f0ff
mp2_gpu_scanout: linear: outside     PASS (background kept outside the box and in the guard rows)
mp2_gpu_scanout: linear: covered     PASS 294... pixels (235929..353894 expected)
mp2_gpu_scanout: linear: copy       PASS (0 of 1280x720 pixels differ from the target)
mp2_gpu_scanout: linear: PASS 1280x720 pitch 5120 at GPU 0x........ phys 0x........ (0 failing checks)
display_power: fb 0x........ + 0x384000: region 10 verdict ... rc 0
mp2_gpu_scanout: chain step none err 0, hpd 1, edid 1 (header 00 ff, ext ...), choice ...
mp2_gpu_scanout: mode 1280x720 74250000 Hz h 110/40/220 v 5/5/20 flags 0x..., hdmi vic 4
mp2_gpu_scanout: ltdc hwver 0x...... bus . layers . on 1, pitch 5120, fb 0xb0000000, clock 74250000 Hz
mp2_gpu_scanout: dsi on 1 lanes 4 lane ... kbit/s, failed step 0 errno 0
mp2_gpu_scanout: adv7535 rev 0x.. hpd 1 started 1
mp2_gpu_scanout: layer at phys 0x........ (GPU 0x........)
mp2_gpu_scanout: PASS: 1280x720 pitch 5120 phys 0x........ 60 frames
mp2_gpu_scanout: hold: 60 frames ok, 0 failed
```

At 1920x1080 the samples are red (960,165), green (268,943), blue
(1651,943), covered 530841..796262, pitch 7680, buffer 0x7e9000 bytes.

The verdict line has `tools/board-stm32mp2.sh`'s `<prog>: PASS: ` /
`<prog>: FAIL: ` shape (its hold and its `result=` read it):

- `mp2_gpu_scanout: PASS: <w>x<h> pitch <p> phys 0x... <n> frames`
- `mp2_gpu_scanout: FAIL: <reason> <err>`, reason one of `ltdc-ready`,
  `format`, `pitch`, `size`, `gpu-device`, `gpu-render-init`, `gpu-alloc`,
  `gpu-clear`, `gpu-wait-clear`, `gpu-triangle`, `gpu-wait-triangle`,
  `linear-alloc`, `linear-copy`, `linear-wait`, `fb-check`, `chain`,
  `set-fb`, `frame`, or the chain's own step (`board`, `fb`, `adv-init`,
  `mode`, `clock`, `ltdc-probe`, `ltdc-configure`, `ltdc-enable`,
  `dsi-mode`, `dsi`, `adv-start`, `i2c`); or
  `mp2_gpu_scanout: FAIL: readback (<n> failing checks)` when a check line
  above says `FAIL`. A fence that times out dumps the GPU's state first
  (`gc_render_stall_dump`).

## The run and the KVM check

On `resource:stm32mp257f-dk` with its console and reset resources, through
the runner (`BOARD_HOST`, `BOARD_UART` from the environment only):

```
MP2_HOLD_S=45 tools/board-stm32mp2.sh run build/mp2_gpu_scanout/zephyr/zephyr.bin <logdir> mp2_gpu_scanout
```

While the runner prints `hold: result seen at ..., holding 45s`, take a
screenshot from `resource:kvm-stm32mp257f-dk` and judge it. The triangle
fills the screen as in `gc_triangle`: its apexes are at (0.5, 0.1),
(0.1, 0.9) and (0.9, 0.9) of the width and height, on the background
`0x102030`, whatever the mode:

```
python3 tools/probes/stm32mp257f_dk/display_check/colorbars.py --triangle \
  --background 16,32,48 \
  --apex 0.5,0.1:255,0,0 --apex 0.1,0.9:0,255,0 --apex 0.9,0.9:0,0,255 \
  shot.png
```

which answers `triangle: PASS: triangle at 3 apexes #...... #...... #......`
(the samples sit 20% of the way from each apex to the centroid, where the
own channel is about 0xdd and the others about 0x11).

Built for the DK's A35 with `-DYAKOGL_WERROR=ON`; not run on a board yet.
