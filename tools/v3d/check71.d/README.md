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
| `counts71.sh` | `qpu_counts_71.tsv` is the per-program count table of `qpu_corpus_71.txt` (header totals, rows, sorted unique words) |
| `shim71.sh` | every packet in `tools/v3d/shim71/*.clif` is a command of Mesa's V3D 7.1 packet spec |

Planned (other items): `cl_golden.sh` (7.1 packet encoders against Mesa),
`hw_mock.sh` (7.x register map, SMS, power), `frame_check.sh` (7.1 frame
builder against Mesa's CLE decoder).
