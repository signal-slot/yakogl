<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# check71.d: the V3D 7.1 host checks

`tools/v3d/check71.sh` runs every executable `*.sh` here, in sorted order,
from the repository root, and prints one line per script. A new 7.1 host check
is one file in this directory; nothing else (`tools/check.sh` included) names it.

## The convention

Each script:

- is standalone bash (`#!/usr/bin/env bash`, SPDX header, executable bit),
  run from the repository root (`cd "$(dirname "$0")/../../.."` first, so it
  also runs by hand from anywhere);
- builds only into `build/v3d71/<name>/`, `<name>` being the file name
  without `.sh`;
- compares against the vendored Mesa reference build, `tools/v3d/mesa_ref.sh`
  (`build/v3d-check/mesa`), and honours `MESA_V3D_TREE` as the v3d rows of
  `tools/check.sh` do: unset, it builds and uses the vendored reference; set
  to a Mesa tree with `build-v3d`, it uses that tree; set to anything else, it
  is a gap naming `MESA_V3D_TREE`;
- on success prints, as its last line, `<name>: <summary>, 0 failed` and exits 0;
- when an input is missing exits 77 with the last line `gap: <ENV NAME> ...`
  (the names to set); a gap is not a pass;
- anything else (any other exit status) is a failure.

## The runner's result

`check71.sh` ends with `check71: N ok, F failed, G gap(s)` and exits 0 (all
passed), 77 with `gap: <script names>` (some gap, no failure) or 1 (some
failure; it wins over a gap).

Not here: the 7.1 Rust lib tests (`isa71`, `codec71`, `listing71`, `golden71`,
`roundtrip71`) run in the gate's `--lib` rows already.

## Scripts

| script | checks |
| --- | --- |
| `cl_golden.sh` | `driver/src/v3d/v3d_cl.h`'s checked packers built at 7.1 (`-DV3D_CL_VER=71`, over the committed `generated/v3d_packet_v71_pack.h`) against the packers of Mesa's own 7.1 build, with `tools/v3d/cl_golden.c`, the source of the gate's 4.2 row: for every packet and record `v3d_cl_packets.h` lists at 7.1 (the clear frame's and the 4.2 cases that exist at 7.1) the struct size, 20,002 value sets never refused and packed to Mesa's bytes, the refusals (7.1's log2 tile size, layer count, render-target stride, number and base included), `v3d_cl_emit` and the 7.1 sub-ids |
| `clear71.sh` | the lists `tools/probes/rpi_5/rpi5_clear` writes (`src/frame71.c`) equal, byte for byte, the same clear frame packed with Mesa's 7.1 packers, for sizes on and next to the tile edges; the 64x64 frame is 19 / 96 / 27 bytes; refused frames are refused |
| `clif_cmp.sh` | self-test of `tools/v3d/clif_cmp.py` on `shim71/clear.clif`: equal to itself; a changed field, a missing packet and a changed address are found; the field and the addresses are equal once ignored |
| `counts71.sh` | `qpu_counts_71.tsv` is the per-program count table of `qpu_corpus_71.txt` (header totals, rows, sorted unique words) |
| `frame_check.sh` | `driver/src/v3d/v3d_frame.c`'s clear frame built at 7.1 (`-DV3D_CL_VER=71`): `tools/v3d/frame_check.c` at 7.1, Mesa's CLE decoder for a version 71 device reads the three lists in full (the 7.1 census, no other packet, no byte left over) at the 4.2 run's sizes; the 64x64 clear is 19 / 96 / 27 bytes and equals `shim71/clear.clif` through `clif_cmp.py` with only the store's `Memory Format` and `Height in UB or Stride` ignored (raster against Mesa's UIF); the lists equal `rpi5_clear`'s `frame71.c` byte for byte, refusals included; each `tools/v3d/pi5/*clear*.clif` (D3's board dump) equals the frame of its size and clear colour with those fields and the named address fields ignored. Without such a dump: a gap naming it, after the rest |
| `gen71.sh` | `driver/src/v3d/generated/v3d_packet_v71_pack.h` is, byte for byte, Mesa's `gen_pack_header.py` run on the vendored `v3d_packet.xml` for 71 (the 4.2 header's check is the gate's `gen driver/src/v3d/generated` row); needs no Mesa build |
| `mbox_mock.sh` | `driver/zephyr/lib/board/rpi4/rpi_mbox.c`, unchanged, for the BCM2711 and the BCM2712 against a mocked mailbox and firmware model (`driver/zephyr/lib/board/test/rpi_mbox_mock.c`): the mailbox base, the MAIL1 word (bus address without the 0xc0000000 alias on the BCM2712), cache maintenance, the clock tags, the framebuffer's address and mapping, -ERANGE above 4 GiB, -EIO, -ETIMEDOUT |
| `shim71.sh` | every packet in `tools/v3d/shim71/*.clif` is a command of Mesa's V3D 7.1 packet spec |

Planned (other items): `hw_mock.sh` (7.x register map, SMS, power).
