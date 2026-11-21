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

## Second attempt (plan #355, 2026-11-12): still no measurement

The worker that ran this item (no `target:x2` lease in the
task) could not reach the board either, so no jump was made and no image was
built for the board:

- `getent hosts armadillo` returned nothing (the name does not resolve here).
- `tools/board.sh x2 status` stopped before touching the board:
  `BOARD_HOST: set BOARD_HOST=` (the runner's own message: the variable was unset) (the task carried no
  board access).
- `ask_farm` answered that the board is reached as root over Tailscale and
  accepts logins only from a fixed set of farm nodes, which did not include
  this task's machine; its serial console (CP2102N) is plugged into another
  farm node and is read-only.

The board was not touched. The work that did not need it is on the host:
`docs/notes/gc-tiled-wire.md` W1 to W5 behind the `gc_tiled_rt` knob
(`gc_debug_set_tiled_rt`, default off), see its "Status" section. The
off/on pair of its "session A" is what the board still has to run.

## Third attempt (plan #355, 2026-11-13): still no measurement

Same worker, same state: `getent hosts armadillo` returned
nothing and `tools/board.sh x2 status` stopped at `BOARD_HOST` unset (no
`target:x2` lease reached the task). No jump was made, no image built, the
board was not touched. W1 to W5 were already on main (`9cdeff6d`), so there
was nothing left to build on the host for this item. A rerun of the item on
the same worker later that day (main at `2aee4c92`) found the same three
facts and did nothing else.

## Fourth attempt (plan #355, 2026-11-13): still no measurement

The same worker again: `getent hosts armadillo` returned nothing and
`tools/board.sh x2 status` stopped at `BOARD_HOST` unset (no `target:x2` or
`target:x2-console` lease reached the task). No jump, no image, the board was
not touched. W1 to W5 are on main (`9cdeff6d`); the earlier closed pull
request for this item only added case 243 probe helpers to `ts.rs` and failed
CI, and was not reused.

## Fifth attempt (plan #355, 2026-11-13): still no measurement

The same worker again, no `target:x2` or `target:x2-console` lease in the task:
`getent hosts armadillo` returned nothing and `tools/board.sh x2 status`
stopped at `BOARD_HOST` unset. No jump, no image, the board was not touched.
W1 to W5 are on main; the earlier closed pull request (`9698dd11`, a tiled
chain layout in `tex_layout.rs` with no caller) failed CI and was not reused.

## Sixth attempt (plan #355, 2026-11-13): still no measurement

The same worker again, no `target:x2` or `target:x2-console` lease in the task:
`getent hosts armadillo` returned nothing, `tools/board.sh x2 status` stopped
at `BOARD_HOST` unset, and `ask_farm` for the board's login timed out. No
jump, no image, the board was not touched. `gc_tiled_rt` (W1 to W5) is already
on main, so no host change was made.

## Seventh attempt (plan #355, 2026-11-15): still no measurement

The same worker again, no `target:x2` or `target:x2-console` lease in the task:
`getent hosts armadillo` returned nothing and `tools/board.sh x2 status`
stopped at `BOARD_HOST` unset. No jump, no image, the board was not touched.
`gc_tiled_rt` (W1 to W5) is on main, so no host change was made. The earlier
closed pull request (`cf0709ff`, the W8/W9 fast-clear word builders in
`rs_clear.rs` with no caller) failed CI and was not reused: W7 to W9 wait for
session A.

## Eighth attempt (plan #355, 2026-11-16): still no measurement

The same worker again, no `target:x2` or `target:x2-console` lease in the task:
`getent hosts armadillo` returned nothing and `tools/board.sh x2 status`
stopped at `BOARD_HOST` unset. No jump, no image, the board was not touched.
`gc_tiled_rt` (W1 to W5) is on main, so no host change was made. The earlier
closed pull request (`9301a7d3`, `depth_dump.rs`, a pure module with no
caller) failed CI and was not reused.

## What is left

Rerun from one of the nodes that log in (see the checks above), with
`target:x2` and `target:x2-console` leased:

```sh
tools/cts/x2-glmark2-scenes.sh --commit 72972ba4 <outdir> terrain shadow refract
tools/cts/x2-glmark2-scenes.sh --commit origin/main <outdir> terrain shadow refract
# session A, on the commit that has gc_tiled_rt: the same, off and then on
tools/cts/x2-glmark2-scenes.sh --commit origin/main --conf CONFIG_GLMARK2_GC_TILED_RT=y <outdir> terrain shadow refract
```

The per-scene expectations from the host profile stay in
`imx8mp-glmark2-cpu.md` ("Slow scenes on the board"). On the host the knob
changes only terrain's streams (`tests/gc_scene_kicks_tiled_rt.rs`;
`gc-tiled-wire.md`, "Status"): shadow and refract render the same streams
off and on, so session A's on run of those two is a control, not a gain.

## Stop, 2026-11-21 (plan #355)

**No X2 measurement was made. The rows below are stop rows, not
measurements: no run, no image, no jump, the board was not touched.**

Cause: the X2 (farm resource `armadillo-x2`) has been turned off on the farm
since 2026-11-17, so no session could lease or reach it. Behind that stands
the earlier cause of the 2026-11-11 stop, which also still holds until it is
checked again from a leased worker: the task machine could not reach the board
by name and had no persisted key (see above). Plan #355's acceptance c5
accepts this written stop.

Commits that would be measured:

- before: `72972ba4`;
- tip of the day: `c3122ede` (`git rev-parse origin/main` as the refs of this
  task's clone show it; the integration tip the work started from is
  `e07d066a`). Session A uses one commit of `origin/main` that carries
  `gc_tiled_rt`, knob off and on.

Commands to run when the board returns (the order and checks of
`gc-tiled-wire.md`, "Session A: the run sheet": reachability from the leased
worker first, then `tools/board.sh x2 status`):

```sh
tools/cts/x2-glmark2-scenes.sh --commit 72972ba4 <outdir> terrain shadow refract
tools/cts/x2-glmark2-scenes.sh --commit origin/main <outdir> terrain shadow refract
# session A: the same commit, knob on
tools/cts/x2-glmark2-scenes.sh --commit origin/main --conf CONFIG_GLMARK2_GC_TILED_RT=y <outdir> terrain shadow refract
tools/cts/x2-scene-table.py <outdir>
```

`x2-glmark2-scenes.sh` runs `tools/board.sh x2 run` (kexec-dev) per scene.

Stop rows (ms a frame `-`; the form of the stop rows of `x2-scene-table.py`,
no `gc_tiled_rt` knob in the older image):

| commit | gc_tiled_rt | scene | ms a frame | FPS | verdict line |
|---|---|---|---|---|---|
| 72972ba4 | - | terrain | - | - | verdict= stop: board turned off, no verdict line |
| 72972ba4 | - | shadow | - | - | verdict= stop: board turned off, no verdict line |
| 72972ba4 | - | refract | - | - | verdict= stop: board turned off, no verdict line |
| c3122ede | off | terrain | - | - | verdict= stop: board turned off, no verdict line |
| c3122ede | off | shadow | - | - | verdict= stop: board turned off, no verdict line |
| c3122ede | off | refract | - | - | verdict= stop: board turned off, no verdict line |
