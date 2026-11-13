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

The worker that ran this item (host `orion`, no `target:x2` lease in the
task) could not reach the board either, so no jump was made and no image was
built for the board:

- `getent hosts armadillo` returned nothing (the name does not resolve here).
- `tools/board.sh x2 status` stopped before touching the board:
  `BOARD_HOST: set BOARD_HOST=` (the runner's own message: the variable was unset) (the task carried no
  board access).
- `ask_farm` answered that the board is reached as root over Tailscale and
  that login is accepted from the nodes Lepton, Lepton-local, hx90,
  n5max-local, nuc and system76 only; `orion` is not one of them. The
  console (CP2102N) is plugged into n5max and is read-only.

The board was not touched. The work that did not need it is on the host:
`docs/notes/gc-tiled-wire.md` W1 to W5 behind the `gc_tiled_rt` knob
(`gc_debug_set_tiled_rt`, default off), see its "Status" section. The
off/on pair of its "session A" is what the board still has to run.

## Third attempt (plan #355, 2026-11-13): still no measurement

Same worker (host `orion`), same state: `getent hosts armadillo` returned
nothing and `tools/board.sh x2 status` stopped at `BOARD_HOST` unset (no
`target:x2` lease reached the task). No jump was made, no image built, the
board was not touched. W1 to W5 were already on main (`9cdeff6d`), so there
was nothing left to build on the host for this item.

## Fourth attempt (plan #355, 2026-11-13): still no measurement

Host `orion` again: `getent hosts armadillo` returned nothing and
`tools/board.sh x2 status` stopped at `BOARD_HOST` unset (no `target:x2` or
`target:x2-console` lease reached the task). No jump, no image, the board was
not touched. W1 to W5 are on main (`9cdeff6d`); the earlier closed pull
request for this item only added case 243 probe helpers to `ts.rs` and failed
CI, and was not reused.

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
`imx8mp-glmark2-cpu.md` ("Slow scenes on the board").
