# rpi5_display

The Raspberry Pi 5 display probe: U3 of `docs/notes/pre-study-rpi5.md`
(section 8, P11). It asks what the display path gives a `kernel=` image; the
routes and the checks that decide among them are in
`driver/docs/rpi5-display.md`.

This is the skeleton: it builds, arms the watchdog, prints a banner and the
placeholder line `U3 skel: ok`, then stops. `src/main.c` has a step table the
display steps are added to; `src/verdict.h` is the print helper.

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
