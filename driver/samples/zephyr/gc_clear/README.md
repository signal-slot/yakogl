# gc_clear

Milestone 4 of `docs/imx8mp-backend-plan.md`: fill a linear XRGB8888 surface
in the GPU carveout with a colour, then read the pixels back on the CPU.

The clear goes through the **resolve engine**, not a full-screen quad. This
core has no BLT engine (the hardware database's `BltEngine` bit is 0), so the
resolve engine is what Mesa uses for a clear here, and it touches none of the
3D state. That makes it the right first picture: it depends on the carveout,
the MMU, the submit path and one fixed-function block, and on nothing else.
If `gc_clear` passes and `gc_triangle` does not, the fault is in the 3D pipe.

The surface is poisoned with `0x00a55a3c` first and then cleared **twice**
with different colours, so neither "the allocator zeroed it" nor "only the
first clear landed" can pass. Five probes -- the four corners and the middle
-- catch a clear that covered only part of the surface, which is what a wrong
stride or window size looks like.

```sh
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_imx8mp/mimx8ml8/a53 -d build/gc_clear \
    yakogl/driver/samples/zephyr/gc_clear
# on the board (Torizon): kexec_zephyr zephyr.bin 0x42000000
```

Expected console lines:

```
target 256x128 stride 1024 at GPU 0x580600xx (CPU 0x...)
cleared to 0x2080c0 (fence 1):
  top left      (  0,  0) = 0x2080c0
  ... five probes ...
cleared to 0xff0040 (fence 2):
  ... five probes ...
gc_clear: PASS
```

`CONFIG_GC_CLEAR_WIDTH` must stay a multiple of 16: the resolve engine
refuses any other width, and the driver rejects it rather than letting the
GPU do something undefined.

Not run on hardware yet (2026-09-20).
