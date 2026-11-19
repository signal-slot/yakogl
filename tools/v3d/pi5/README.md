<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1 references taken on the Raspberry Pi 5 (D3)

Item D3 of `docs/notes/pre-study-rpi5.md` (section 6): what the Pi 5's own
Linux says about its GPU, and the control lists Mesa submits on it. They
replace the drm-shim approximations of `tools/v3d/shim71/` (H3) where they
differ; the differences are listed below. Nothing here comes from our
driver.

Taken on 2026-11-19 on the bench board (Raspberry Pi 5 Model B Rev 1.1,
8 GB, BCM2712 D0), Raspberry Pi OS Lite trixie, kernel `6.18.50+rpt-rpi-2712`
(16 KiB pages), Mesa `26.2.2-1~bpo13+0~rpt1` (the image's packages; nothing
was installed), boot loader of 2025-06-13. `vcgencmd get_throttled` read
`throttled=0x0` before, between and after the records. Linux was not
rebooted for them.

| File | What | How |
|---|---|---|
| `v3d_ident.txt` | debugfs `v3d_ident` | `sudo cat /sys/kernel/debug/dri/1002000000.v3d/v3d_ident` |
| `v3d_regs.txt` | debugfs `v3d_regs` (raw hub, MMU, GMP and core 0 registers) | the same directory, `v3d_regs` |
| `clocks.txt` | the V3D clock at idle (`clk_summary`), debugfs `measure_clock`, `vcgencmd measure_clock`, the `*_freq` keys of `vcgencmd get_config int`, core volts, `get_throttled` | as its `#` lines say |
| `clear.clif`, `clear.stdout` | Mesa's `V3D_DEBUG=clif` dump of a GL clear (64x64 RGBA8 FBO, `glClearColor(0, 0, 1, 1)`) | `V3D_DEBUG=clif MESA_SHADER_CACHE_DISABLE=true python3 tools/v3d/gl_ref.py /usr/lib/aarch64-linux-gnu clear` on the board (stderr is the `.clif`, stdout the `.stdout`) |
| `triangle.clif`, `triangle.stdout` | the same clear and one Gouraud triangle | the same with `triangle` |

`gl_ref.py` is the program `tools/v3d/drm_shim_ref.sh run` used for
`shim71/`, unchanged, on the board's real render node (no shim, no loader
override); it was copied to a tmpfs and removed afterwards.

## The GPU, decoded

`python3 tools/v3d/ident71.py tools/v3d/pi5/v3d_regs.txt`:

- HUB_IDENT0..3 `0x42554856 0x00081117 0x00001900 0x00020a10`; CTL_IDENT0..2
  `0x07443356 0x81001431 0xc0078101`; MMU_DEBUG_INFO `0x20804664`.
- V3D 7.1, IPREV 10, IPIDX 16, compat_rev 2 (Mesa's renderer string: `V3D
  7.1.10.2`); 1 core, 3 slices, 12 QPUs, 1 TMU a slice (debugfs prints
  3 TMUs, slices times IDENT1's NTMU), no semaphores, MMU yes, no TFU, MSO
  yes, no L3C; VPM 65536 bytes (IDENT1 VPM_SIZE 8); MMU VA and PA 36 bits,
  MMU version 4.
- Mesa's variant consequences (U2): no accumulators, draw index (rev >= 10:
  the BCM2712 D0 shader record, below), unpack sat and max0, 8 render
  targets, framebuffer 7680, clipper XY granularity 64, CLE readahead 1024.
- debugfs prints the revision as `7.1.10.16`: its fourth number is IPIDX
  (HUB_IDENT3 bits 7:0), not the compat_rev Mesa prints fourth (2).
- Clock: `fw-clk-v3d` reads 500 MHz with enable count 0 while idle; under
  load (debugfs `measure_clock`) the V3D ran at about 965 to 978 MHz and
  `vcgencmd measure_clock v3d` read 960 MHz (`v3d_freq=960`,
  `v3d_freq_min=500`). `core_freq` 910 (min 500); the core clock read 910
  or 500 MHz depending on load.

## Differences from `tools/v3d/shim71/`

Compared with the buffer names' addresses taken out (the offsets kept):

1. **The GPU revision.** The shim reports `V3D 7.1.7.0`; the board is
   `V3D 7.1.10.2` (BCM2712 D0; the firmware loads the `bcm2712d0` overlay
   on this board). The shim's revision is older than the board's.
2. **Buffer addresses and sizes.** The board's Mesa allocates every buffer
   in 16 KiB (the kernel's page size; Mesa 26.2.1's `v3d_bufmgr.c` aligns to
   4 KiB, the shim's dumps have 4 KiB buffers), so every buffer base differs
   (`resource_0x9000`, `CL_0x11000`, ... against `resource_0x3000`,
   `CL_0x8000`, ...) and the `@format blank` sizes with them. Field by
   field, the only value that moves is the **tile allocation size**, the
   submit's `ADD_BIN` fourth argument (qms): 16384 on the board, 12288 on
   the shim and in our builder. `tools/v3d/check71.d/frame_check.sh` (d)
   compares it apart (the board's must be ours rounded up to 16 KiB).
3. **The clear's control lists are otherwise equal**: the same 36 packets
   with the same fields (binning list 19 bytes, render list 96, generic tile
   list 27). frame_check (d): `clif_cmp: 36 packets, 0 differences`.
4. **The triangle's shader record.** Mesa packs the BCM2712 D0 layout
   (`GL Shader State Record Draw Index`, `v3d_packet.xml`: "exclusive to
   2712D0 (V3D 7.1.10)", picked by `v3d_device_has_draw_index`, rev >= 10),
   which inserts `cs_basevertex`, `cs_drawindex`, `vs_basevertex` and
   `vs_drawindex` and moves every flag from bit 5 up by 2 to 4 bits. Mesa's
   CLIF dump decodes it with the plain 7.1 layout, so the dump shows three
   flags that look changed: "uses real pixel centre W" 1 -> 0, "Insert
   Primitive ID as first varying" 0 -> 1, "Disable implicit point/line
   varyings" 1 -> 0. Read at the D0 layout the values are the shim's (real
   pixel centre W is bit 15, set; Insert Primitive ID bit 18, clear; Disable
   implicit point/line varyings bit 21, not decoded by the plain layout).
   A 7.1 shader record we write for this board must use the D0 layout.
5. **Shader code and the rest of the triangle job** (the FS/VS/CS words, the
   uniforms, both attribute records, the vertex data and every control
   list packet) are equal to the shim's.

No other line differs.
