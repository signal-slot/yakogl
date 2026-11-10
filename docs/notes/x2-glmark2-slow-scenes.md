<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# X2 board session: terrain, shadow and refract (plan #328, 2026-11-10)

Development measurements on the Armadillo-X2 (kexec-dev, never a row of
`tools/cts/RESULTS.md`). Tool: `tools/cts/x2-glmark2-scenes.sh`, images
built for `armadillo_x2/mimx8ml8/a53` with the watchdog marker
`gc_wdog_arm: armed` (the dry run found it in the image).

## Result: no measurement, the board did not come back

| scene | commit | run | verdict line |
|---|---|---|---|
| terrain | `de737051` (origin/main tip, "after") | no selector | `rc=5 verdict=not_back boot_verdict=not_back result=none record=absent` |

- The dry run of all five runs of terrain at `de737051` built the image,
  found the marker and printed what each run would do (rc 0).
- The first real run (terrain, no selector, the jump itself) did not bring
  the board back within the runner's limit. The script stopped the whole
  command by its rule (exit 5), without a retry. The next ssh from the
  session was refused (publickey), so the board is not in the state the
  session found it in.
- No row exists, for either commit: the "before" commit `72972ba4` was not
  run, and the "after" table has no number. Nothing here is a ms a frame.
- **The board needs a person** (no remote power control). Whether the
  image hung with its watchdog not expiring, or the board rebooted into a
  state without the farm's ssh key, is not known: no console was open.

## What is left

Rerun, on the board once it is back:

```sh
tools/cts/x2-glmark2-scenes.sh --commit 72972ba4 <outdir> terrain shadow refract
tools/cts/x2-glmark2-scenes.sh --commit origin/main <outdir> terrain shadow refract
```

The per-scene expectations from the host profile stay in
`imx8mp-glmark2-cpu.md` ("Slow scenes on the board").
