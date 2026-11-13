<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# X2 board session: terrain, shadow and refract (plan #328, 2026-11-10)

Development measurements on the Armadillo-X2 (kexec-dev, never a row of
`tools/cts/RESULTS.md`). Tool: `tools/cts/x2-glmark2-scenes.sh`, images
built for `armadillo_x2/mimx8ml8/a53` with the watchdog marker
`gc_wdog_arm: armed` (the dry run found it in the image).

## Result: no measurement, no jump was made (2026-11-11)

No row exists, for either commit; nothing here is a ms a frame. The
session that follows the dtpu session checked reachability before any jump
and stopped:

- `ping -c1` by the board's name failed ("Temporary failure in name
  resolution"): the task machine does not reach the board directly.
- ssh to the board worked only through the forward connected makes for one
  turn, with a per-task key kept in a tmpfs, not with the farm node's
  persisted key. That is the same state in which the dtpu session's jump
  ended in `not_back` although the board came back in Linux.
- `tools/board.sh x2 status` ran (no lease, board clock readable); no jump
  and no image was built or run. The board was left as found.

The earlier `rc=5 verdict=not_back` runs (terrain at `de737051`, then the
dtpu session) came from the runner losing the board, not from a measured
hang; they give no ms a frame either.

## What is left

Rerun from a machine that pings the board by name and logs in with a persisted key (see the checks above):

```sh
tools/cts/x2-glmark2-scenes.sh --commit 72972ba4 <outdir> terrain shadow refract
tools/cts/x2-glmark2-scenes.sh --commit origin/main <outdir> terrain shadow refract
```

The per-scene expectations from the host profile stay in
`imx8mp-glmark2-cpu.md` ("Slow scenes on the board").

Session A (plan #355, docs/notes/gc-tiled-wire.md W5's board check) adds the
knob-on arm once W1 to W5 are on origin/main, from the same image recipe:

```sh
tools/cts/x2-glmark2-scenes.sh --commit origin/main --conf CONFIG_GLMARK2_GC_TILED_RT=y <outdir> terrain shadow refract
```

The host harness's prediction for that arm (`tests/gc_scene_kicks.rs`
`tiled_rt`): terrain keeps its kicks, waits and finishes, and adds three
800x600 RS copies a frame: the overlay's tile-in, and the terrain pass's and
the overlay's resolves. Shadow and refract are untouched by the
knob: their depth is a texture, which is not PE-private.
