<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_dtb

Prints the device tree the Raspberry Pi 5 firmware hands to a `kernel=` image
in x0, and the nodes of it whose compatible names `v3d`, `2712-pm` or `vc4`
(path, compatible, reg). Part of ROADMAP stage 5 (issue #7); see
`docs/notes/pre-study-rpi5.md`.

```
west build -p always -b rpi_5 -d build/rpi5_dtb yakogl/tools/probes/rpi_5/rpi5_dtb
```

x0 comes from `../common/boot` (`boot_x0.S`, linked with `--wrap=__start`);
the blob is checked by `../common/fdt/fdt_walk.c` before any token is trusted.

The console lines start `rpi5_dtb:`: the build, x0 / EL / entry, the mapped
range, the walk result, then one `node`/`compatible`/`reg` group per match and
`done`. A line starting `FAIL` says what went wrong.

## Limits

- Zephyr's rpi_5 MMU map covers only the image RAM and the device MMIO, and
  the DTB sits above it. `main.c` therefore `device_map()`s the header page,
  reads `totalsize` (refused above 4 MiB), maps the whole blob rounded to
  pages as normal cacheable memory (`K_MEM_CACHE_WB`, not device memory: the
  string functions make unaligned accesses) and prints the mapped range.
  `CONFIG_MAX_XLAT_TABLES=16` leaves tables for it.
- A node with child nodes is not reported (GPU nodes are leaves).
- Built, not run on the board; the first run also shows whether the
  watchdog (`CONFIG_YAKOGL_RPI5_WDOG`, as `rpi5_wdog`) works.
