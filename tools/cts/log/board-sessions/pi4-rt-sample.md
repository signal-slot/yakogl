<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session, v3d-rt-sample: glmark2 33 / 0 / 0, score 188

Date: 2026-10-14. Plan #18, item v3d-rt-sample: the glmark2 image with
render targets sampled from their render image and glGenerateMipmap made by
the TFU (`6914364d742c`, "v3d: sample a render target from its render image,
stored by the TLB, and make its mip chain with the TFU").

Result: **33 scenes ran, 0 failed, 0 unsupported, score 188, 0 GPU resets,
0 GL errors, 0 draws refused, one tryboot.** refract, terrain, shadow and
desktop blur take far less CPU time per frame than on the integration image
of the same day. refract, terrain and shadow still spend about 170-190 ms of
CPU time in GL calls per frame (below), so they stay at 6 FPS.

## Build

From the workspace at `6914364d742c`, with the dependency layout's Zephyr
workspace, SDK 1.0.1 and glmark2 at `22c527cb` (clean):

```sh
west build -p always -d <out>/build -b rpi_4b driver/samples/zephyr/glmark2 -- \
    "-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")" \
    -DGLMARK2_SRC=<glmark2 at 22c527cb>
```

`zephyr.bin` is 12,387,768 bytes, sha256 `0a352b2cdc194b1b...`. Its `.config`
has `CONFIG_YAKOGL_V3D_TFU=y` and `CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES=25165824`.
The build needs `TMPDIR` on a file system with room: a full `/tmp` stops
the compiler with "Disk quota exceeded".

## Run

`BOARD_HOST` (the Pi 4's ssh login) and `BOARD_UART` (`host:port` of the
serial bridge in front of its Debug Probe) come from the environment. The
bridge takes one client, and another session held it for about 15 minutes.
One run was refused (rc 3: busy at capture, image removed, no reboot); the
next one ran:

```sh
RUN_BOUND=960 tools/board.sh pi4 run <out>/build/zephyr/zephyr.bin <rec> glmark2
```

```
board.sh pi4: glmark2: build 6914364 boot tryboot (row) image sha256 0a352b2cdc194b1b footprint 250494976
  GLMARK2 (18) seq 1: score 188 status 1 (pass) scenes ran 33 failed 0 unsupported 0 size 800x600 on-screen; first failed: none; unsupported: none, none; GL errors raised 0, draws that sampled an incomplete texture 0
board.sh pi4 glmark2 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00bc,6184003f,00258320 back_after=418s unsupported=0
```

## The scenes that sample render targets

"Before" is the integration image of the same day (`86be8aeb`, the common
GM row, score 183, 33 / 0 / 0). The numbers are the `glmark2-frame:` lines:
frames in the scene's 10 s, and `gl`, the CPU time in GL calls per frame.

| Scene | Before: frames, gl | After: frames, gl | GPU render per frame (after) |
|---|---|---|---|
| desktop blur | 106, 89,810 us | 769, 7,103 us | 1,259 us |
| desktop shadow | 461, 17,031 us | 1,472, 813 us | 1,312 us |
| terrain | 23, 442,127 us | 56, 176,747 us | 2,802 us |
| shadow | 43, 228,277 us | 57, 170,396 us | 1,488 us |
| refract | 14, 711,383 us | 52, 187,740 us | 5,440 us |

Lines the driver printed:

- `a 800x600 render target is sampled from its render image (1920000 bytes,
  1 levels): no tiled copy` (desktop, the first one only: the line is printed once).
- `the TFU made the 10-level chain of a 800x600 render image: glGenerateMipmap
  filters nothing` (terrain, the first chain only).
- `a stale tiled copy of 7684096 bytes given back before the job opened`
  (at the start of shadow) and `tiled copies high-water 19742720 of 25165824
  bytes` (at the start of refract). From this console alone it is not known
  whether the copies belong to a colour target or to a depth texture (option (b),
  not done). The shutdown line `render images: ...` is not printed: glmark2
  ends without tearing the context down.

## What is left

In terrain, shadow and refract the GPU renders a frame in 1.5-5.4 ms. The
CPU spends 170-190 ms in GL calls in the same frame. shadow and refract
sample a depth texture: the GL core resolves the rendered D24S8 depth into
R32F texels on the CPU every frame (`texture::resolve_depth`), reading GPU
memory, and the V3D then tiles a copy of those texels. That is option (b) of
RESULTS. terrain samples no depth texture. Its per-frame CPU cost is not
explained by this console. The host harness
(`tools/hostharness/glmark2/run.sh ... v3d`, one process per scene, each
ending with the backend's shutdown counters) is the next step to count what
terrain still copies or filters.
