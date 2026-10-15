<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_common -- what only the i.MX8MP samples share

The board support every i.MX8MP image needs (the watchdog and the bounded way
back to Linux, the fatal handler, the interrupt guard, the SNVS result record
and the panel console) is now the driver module's `yakogl_gc_board` library,
`driver/zephyr/lib/board/imx8mp`, which has its own README. `Kconfig` here turns it
on for every sample that sources this directory's Kconfig, and
`gc_sample.cmake` stamps the sample's name and build time on it.

What stays here is used by the samples only:

- `verdin_imx8mp_display.dtsi`: the Verdin/Mallow display chain that the
  samples' overlays include.
