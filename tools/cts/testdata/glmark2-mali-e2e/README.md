<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/cts/testdata/glmark2-mali-e2e/`

The fixtures of `tools/cts/glmark2-mali-e2e-selftest.sh`, which runs plan
#332's NanoPC-T6 glmark2-es2 chain end to end on a host without a board:
`mali-glmark2-build.sh --dry-run`, then `mali-glmark2-run.sh` on the fake
board of `tools/cts/testdata/mali-glmark2-run/` (its `bin/`, `board-bin/`,
`fixture-build/` and `kmsg/`, used by path), then `glmark2-mali-tally.py`.

- `scenario.tsv`: what the fake `glmark2-es2-mali` does to the 33 default
  scenes: a GPU fault in scene 5 (`texture:texture-filter=mipmap`), a hang in
  scene 23 (`terrain`), the rest clean.
- `pictures.txt`: the tally's `--table`, a `mali` column of three scenes (a
  match, a mismatch, a match), so the drafts stay put when
  `tools/cts/glmark2-pictures.txt` gains its own `mali` column.
- `runner-to-tally.py`: the runner's run directory in the tally's layout
  (below).
- `results-row.expected.in`, `status-row.expected.in`, `causes.expected.in`:
  the tally's `--results-row`, `--status-row` and `--causes` on the run, byte
  for byte once `@COMMIT@`, `@SHA8@` (the throwaway repository's main),
  `@DATE@` (the run's day) and `@PAIR16@` (the first 16 hex of the fixture
  `libGLESv2.so.2`'s sha256) are filled in.

## The layout gap

On main the runner and the tally do not meet; the self-test pins both gaps
(the tally refuses the runner's output as it is) and bridges them with
`runner-to-tally.py`:

| The tally reads | The runner writes | The bridge |
| --- | --- | --- |
| `env.txt`: `commit=`, `board=`, `kernel=`, `pair_sha256=`, `date=` | `meta.txt`: `commit=`, `kernel=`, `started=`; no board name, no sha256 of the pair | `commit=`, `kernel=` from `meta.txt`, `date=` from `started=`, `board=NanoPC-T6`, `pair_sha256=` of the build's `lib/libGLESv2.so.2` |
| `<NN>-<slug>.log` | `log/<n>.out`, `log/<n>.err` | the two, in that order |
| `<NN>-<slug>.kmsg` with `kmsg: resets=N timeouts=N mmu_faults=N fw_faults=N gpu_faults=N other_panthor=N first=...` or its JSON | `kmsg/<n>.class`: `mali-kmsg.py`'s line, which has had `cs_faults=N` before `other_panthor=` since f5afce4a: the tally's pattern does not match it | `mali-kmsg.py --json` over `kmsg/<n>.log` first, then `.class` and the kernel lines |
| `<NN>-<slug>.status`: `exit=`, `timeout=` | no exit status in the run directory: only the runner's stdout line `scene <n>: <verdict>, ms/frame ..., kmsg ..., rc <rc>`, and the verdict `hang` | `exit=` the stdout `rc`, `timeout=1` for `hang` |
| `<slug>` of `glmark2-host-ref.sh --list` | `scenes.tsv`'s scene as `-b` takes it | the info string (`[name]` and the options sorted, as glmark2 prints it) looked up in `--list` |

The runner also prints no "/boot restored" line: it never touches `/boot`
(its header), so the self-test checks that no command it sends names `/boot`
and that the board answers with the same `boot_id` at the end.
