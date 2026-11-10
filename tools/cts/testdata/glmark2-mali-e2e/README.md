<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/cts/testdata/glmark2-mali-e2e/`

The fixture of `tools/cts/glmark2-mali-e2e-selftest.sh`, which runs plan
#332's NanoPC-T6 glmark2-es2 chain end to end on a host without a board:
`mali-glmark2-build.sh --dry-run`, then `mali-glmark2-run.sh` over all 33
scenes on the fake board of `tools/cts/testdata/mali-glmark2-run/` (its
`bin/`, `board-bin/` and the scenarios `clean.tsv` and `faults.tsv` used by
path, `fixture-build/` copied and stamped with the dry run's commit), then
`glmark2-mali-tally.py` on each run directory in its runner-directory mode.

- `pictures.txt`: the tally's `--table`, a `mali` column of three scenes, so
  the judgement stays put when `tools/cts/glmark2-pictures.txt` gains its own
  `mali` column. Against the fake program's one picture: scene 1 matches,
  scene 2 is off in probe c and the mean (a mismatch), scene 5 is within the
  tolerance (a match in `clean.tsv`; absent in `faults.tsv`, where scene 5
  is the job timeout and prints no picture).

## The scene-name gap

The runner's `scenes.tsv` names each scene as `-b` takes it, options in the
runner's order (`pulsar:quads=5:texture=false:light=false`); the tally looks
the scene up as glmark2 prints it, options sorted by name
(`[pulsar] light=false:quads=5:texture=false`, `glmark2-host-ref.sh --list`),
and refuses the run directory at the first row that differs (row 15, pulsar;
14 of the 33 scenes have more than one option in another order). The
self-test prints the tally's answer on the run directory as it is as a
`note:` line, and checks the tally on a copy whose `scenes.tsv` has each
scene's options sorted, nothing else changed.
