<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL GPU drivers

The GPU drivers YakoGL renders through, and their Zephyr board support. There is no
Linux GPU driver underneath: these drivers own the GPU from Zephyr (or bare metal).

- **PowerVR BXS-4-64** (TI AM62P): firmware boot, MMU, command rings, free lists and the
  render service. The firmware interface headers come from the TI/Imagination DDK 25.2,
  vendored in `../third_party/ti-img-rogue-ddk-25.2` and adapted in `compat/`.
- **Vivante GC7000UL** (NXP i.MX 8M Plus): power, MMU, events and the command ring
  (`src/vivante/`).
- **Broadcom V3D 4.2** (Raspberry Pi 4): the job builder (`src/v3d/`).

## Layout

- `src/`: the drivers (C). `rust/`: the driver crate (`yakogl-drivers`): the PowerVR
  kernel side, the display IP blocks and the SoC register code.
- `boards/`, `soc/`, `dts/`, `drivers/`, `lib/`, `include/`, `zephyr/`: Zephyr board
  support for the Verdin AM62P, the Verdin iMX8M Plus and the Raspberry Pi 4.
- `samples/zephyr/`: the test and benchmark images, one sample for all the boards:
  `gl_test` (the on-silicon suite), `deqp` (the dEQP suite) and `glmark2` (the
  benchmark). `samples/zephyr/README.md` has the workspace setup. The examples that
  run on all three boards (`triangle`, `triangle-rust`, `slint`, `femtovg`) are in
  `../examples/`.
- `tools/`: `offsetof/` (the firmware ABI layouts, proved against the DDK headers),
  `sfdecode/` (the firmware trace decoder), `mkfwblob.sh` (the firmware header) and the
  development harnesses.
- `docs/`: design notes, bring-up records and the GPU recipes.

## The PowerVR firmware

The firmware `rgx.fw.36.53.104.796` is Imagination's and is not in this repository.
Take it from TI's Processor SDK (or the board's `/lib/firmware`) under its licence and
turn it into the header the build needs:

```sh
driver/tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796   # -> driver/src/fw/pvr_fw_blob.h
```

The generated header must never be committed.

## Running on a board

Build a sample with west (`samples/zephyr/README.md`) and run it with the runner shared
by all boards:

```sh
BOARD_HOST=<user>@<board> tools/board.sh am62p run <zephyr.bin> <logdir> [name]
```

`docs/one-method.md` describes the method; section 4 covers the runner.

## Licensing

Every file carries an SPDX line. Our own code is Apache-2.0. Code from outside keeps
its upstream licence; the repository's `NOTICE` lists each origin.
