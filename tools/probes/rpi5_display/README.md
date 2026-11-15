# rpi5_display

The Raspberry Pi 5 display probe: U3 of `docs/notes/pre-study-rpi5.md`
(section 8, P11). It asks what the display path gives a `kernel=` image; the
routes and the checks that decide among them are in
`driver/docs/rpi5-display.md`.

It runs checks Z0 to Z4 of the note's section 6 and prints the `u3 ...` lines
`verdicts.sh` reads: clocks and the HVS stepping, the HVS channels and the
first plane of the running one, pixel valve 0 and `hd` (only when the HSM
clock runs), a pattern written into that plane (route A), the Pi 4's
mailbox allocation (`rpi_fb_alloc_virtual`) with a pattern written and read
back (route B), whether the HVS list then names that buffer, and the split
screen (console above, pattern below). It prints `GET_THROTTLED` before and
after. Built with `-Werror`; not yet run on a board.

## Watchdog first

The board layer (`driver/zephyr/lib/board/rpi4/rpi_wdog.c`,
`CONFIG_YAKOGL_RPI5_WDOG`) arms the BCM2712 PM watchdog in the first
`PRE_KERNEL_1` hook, before `main()`. The board has no remote power: the reset
brings Linux back through tryboot (`docs/standalone-boot.md`). `main()` never
feeds it. If the board does not come back, it needs a person.

## Build

```
west build -p always -b rpi_5 -d build/rpi5_display tools/probes/rpi5_display -- -DYAKOGL_WERROR=ON
```

The image is `build/rpi5_display/zephyr/zephyr.bin`; `tools/board.sh pi5`
boots such an image. `boot_x0.S` from `tools/probes/rpi_5/common/boot` keeps
the DTB pointer the firmware hands over (referenced, not copied).
