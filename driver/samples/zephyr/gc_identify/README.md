# gc_identify

The first Zephyr program for the i.MX8MP GPU: it runs the Vivante driver's
milestone 1 (power the GPU domains through the GPC, clock the core through the
CCM, read the identity registers, reset to idle), prints the result, services
the watchdog Linux left running for a bounded time and resets the SoC.

```sh
# in a west workspace made from yakogl's west.yml (see ../README.md)
west build -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_identify \
    yakogl/driver/samples/zephyr/gc_identify
# on the board (Torizon): kexec_zephyr zephyr.bin 0x42000000
```

Expected console lines: `GC7000 rev 0x6204`, `reset to idle`, `gc_identify: PASS`.
This sample has not run on a board yet (2026-09-16): the Verdin iMX8M Plus is
the target, docs/imx8mp-backend-plan.md has the status.
