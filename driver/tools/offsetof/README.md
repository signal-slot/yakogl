# FWIF ABI checks

The TI/Img DDK 25.2 kernel headers are vendored unmodified in
`third_party/ti-img-rogue-ddk-25.2/`, together with the build configuration
the firmware matches (`config/am62p_linux_release/config_kernel.h`). The
layouts must be read with that configuration, not the DDK's
`copy-to-kernel-tc` one: the two disagree (`RGXFWIF_KCCB_CMD` 160 against 80
bytes, `RGXFWIF_FWCOMMONCONTEXT` 456 against 336), and the firmware reads the
160/456 layout.

## ddk_abi_check (a `tools/check.sh` row)

`ddk_abi_pins.h` lists every firmware-interface number the driver depends on
(structure sizes and offsets, KCCB/FWCCB/CCB command numbers, flags, the MH
tag encodings and the BIF fault-status fields the render service decodes)
with the value the board runs. `ddk_abi_check.c` turns each line into a
`_Static_assert` against the vendored headers, and compares the boot
alignment-check table with `RGXFW_ALIGN_CHECKS_INIT_KM` at run time.
`ddk_abi_fork_check.c` checks the driver's own copies of the same numbers
(`driver/src/pvr_ddk_abi.h`, the `PVR_DDK_FW` parts of `pvr_fwif.h`) against
the pinned values, for as long as those copies exist. A `MISMATCH` line
records a place where the driver's copy differs from the DDK, with both values
asserted and the reason.

```sh
cc -std=gnu11 -Wall -Wextra -Werror $(driver/tools/offsetof/ddk-cflags.sh) \
   driver/tools/offsetof/ddk_abi_check.c -o ddk_abi_check && ./ddk_abi_check
cc -std=gnu11 -Wall -Wextra -Werror -DPVR_DDK_FW -Idriver/src -Idriver/tools/offsetof \
   driver/tools/offsetof/ddk_abi_fork_check.c -o ddk_abi_fork_check && ./ddk_abi_fork_check
```

## dumpcc

`dumpcc.c` prints `offsetof`/`sizeof` for a few FWIF structs, for looking at a
layout by eye:

```sh
cc -std=gnu11 $(driver/tools/offsetof/ddk-cflags.sh) -o dumpcc driver/tools/offsetof/dumpcc.c && ./dumpcc
```

Expected (abridged):

```
sizeof(RGXFWIF_KCCB_CMD)=160
sizeof(RGXFWIF_FWCOMMONCONTEXT)=456
```
