<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_dtb

Prints the device tree the Raspberry Pi 5 firmware hands to a `kernel=` image
in x0, and the nodes of it whose compatible names `v3d`, `2712-pm`, `vc4`,
`hvs`, `vc6` or `firmware-clocks` (path, compatible, reg, and `reg-names`,
`interrupts`, `clocks`, `resets`, `power-domains` where the node has them).
The BCM2712's display and clock nodes are `brcm,bcm2712-hvs`,
`brcm,bcm2712-vc6` and `raspberrypi,firmware-clocks`: the first board run
matched only the `pm` and V3D nodes, so the last three were added. Part of ROADMAP stage 5 (issue #7); see
`docs/notes/pre-study-rpi5.md`.

```
west build -p always -b rpi_5 -d build/rpi5_dtb yakogl/tools/probes/rpi_5/rpi5_dtb
```

x0 comes from `../common/boot` (`boot_x0.S`, linked with `--wrap=__start`);
the blob is checked by `../common/fdt/fdt_walk.c` before any token is trusted.

The console lines start `rpi5_dtb:`: the build, x0 / EL / entry, the mapped
range, the walk result, then one `node`/`compatible`/`reg` group per match and
`done`. A line starting `FAIL` says what went wrong.

## Verdicts

`verdicts.sh <console.log>` prints one `rpi5_dtb verdict:` line each for the
boot record, the walk, the node count, the GPU (v3d) node and the end line,
then `PASS` or `FAIL (<reason>)`. Exit 0 done, 1 no end line, 2 usage, 3 FAIL.
`verdicts_selftest.sh` checks it against inline logs (good, FAIL, truncated,
no GPU node); the gate runs it.

## Limits

- Zephyr's rpi_5 MMU map covers only the image RAM and the device MMIO, and
  the DTB sits above it. `main.c` therefore `device_map()`s the header page,
  reads `totalsize` (refused above 4 MiB), maps the whole blob rounded to
  pages as normal cacheable memory (`K_MEM_CACHE_WB`, not device memory: the
  string functions make unaligned accesses) and prints the mapped range.
  `CONFIG_MAX_XLAT_TABLES=16` leaves tables for it.
- A node with child nodes is not reported (GPU nodes are leaves).
- Run on the board on 2026-11-19 through `tools/board.sh pi5` (verdict
  `done`, Linux back by itself): the firmware loaded the image at 0x200000
  and entered it at EL2 through TF-A's BL31 with x0 = 0x2efec600, the DTB
  (80201 bytes, 322 nodes); the nodes it printed are in
  `docs/standalone-boot.md` ("Raspberry Pi 5", measured).
