<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_scanout32

Stage 2 of `docs/notes/rpi5-display.md` (section 3, question Q4): our own
32 bpp XRGB8888 plane placed after word 32 of the firmware's display list,
channel 0's list pointer swapped at vblank between two buffers, then the
firmware's list restored byte-equal. Part of ROADMAP stage 5 (issue #7), step
P11 of `docs/notes/pre-study-rpi5.md`.

This directory is the build skeleton. Its image only reads and writes
nothing: it prints the build line, the HVS `VERSION` and stepping, channel 0's
`CTRL0`, `LPTRS`, `STATUS` (`FRCNT`) and `DL` words decoded, the firmware's
display list word by word from its start through word 32 and on to its END,
and the first plane (address, pitch, size, format). Every read prints a line,
also when it cannot be made. It ends with `rpi5_scanout32: skeleton done`.
`snap_dlist()` in `src/main.c` is the list snapshot the byte-equal restore
check will reuse.

```sh
west build -p always -b rpi_5 -d build/rpi5_scanout32 yakogl/tools/probes/rpi_5/rpi5_scanout32 -- -DYAKOGL_WERROR=ON
```

## On the board

Not run yet.
