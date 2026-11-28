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

## Stop, 2026-11-24 (plan #355, item x2-before-now)

**No X2 measurement was made. No jump, nothing was staged or copied to the
board, the board was left as found, up on Linux.** The rows below are stop
rows, not measurements.

This session held `target:x2` only (no console, as `x2-glmark2-scenes.sh`
needs none). Commits: before `72972ba4`, tip `ecd9743e` (`origin/main` as
this task's clone shows it; `git fetch origin` failed here, access denied).

What held:

- Reachability, the cause of the 2026-11-11 stop, is cleared: from the
  leased worker, ssh as root logs in, and `tools/board.sh x2 status` (with
  `BOARD_HOST` from the lease) reads `board lease: none`, the runlevel
  `default`, `kexec_loaded 0`, `kexec_crash_loaded 0`, crash window
  `268435456`, uptime about 80 450 s.
- The 72972ba4 terrain image built (`armadillo_x2/mimx8ml8/a53`, sha256
  `0a20645a...`, the marker `gc_wdog_arm: armed` present, footprint
  157057024 bytes).

What stopped it: **an application container runs on the board.** The
status read counts one podman container: `slint-launcher` (image
`at-debian-image`), started at boot and up for 22 h. Its config on the board
sets `set_autostart yes`, so it comes back at every boot. `board.sh x2 run`'s
readiness check (`x2_kexec_ready`, "nobody's application running": a
crash-route jump ends a container without a shutdown) refuses the jump.
Stopping the container or changing its autostart was not this item's to do
(leave the board as found, persist nothing), so the board cannot take a
kexec-dev run until its owner decides.

The run, `tools/cts/x2-glmark2-scenes.sh --commit 72972ba4
records/x2/20261124-1035-before-72972ba4 terrain shadow refract`, built the
terrain image and stopped at its first `board.sh x2 run` (no selector), in
the readiness read, before the tools, the record or the image were copied:

```
board.sh x2: something is running (containers 1)
scene	commit	selector	verdict line	record words	decoded	ms/frame
terrain	72972ba4	none	no verdict line, rc=3	-	-	-
x2-glmark2-scenes: a run was refused, stopped (nothing was retried)
```

The run directory's `meta.txt` (redacted):
`board_state=openrc settled uptime 82110s kexec=0 containers=1 crash_window=268435456 crash_loaded=0`,
`refused_rc=3`, `refused=something is running (containers 1)`. After it,
`board.sh x2 status` read `board lease: none`, runlevel `default`,
`kexec_loaded 0`, `kexec_crash_loaded 0`, uptime 82118 s (no reboot): the
board as found.

On the way (not causes of the stop): `kexec_zephyr` and `snvs_lpgpr`, which
`board.sh` refuses to run without, are gitignored and must be built on the
worker first (README, "freestanding, so any aarch64 GCC works"). Here they
were built with `clang --target=aarch64-none-elf -O2 -static -nostdlib
-ffreestanding -fno-stack-protector` and the Zephyr SDK's `ld.bfd`.
`board.sh`'s ssh must also be the connected ssh on `PATH`: a run with `PATH`
reset to the system ssh got `rc 255` from the lease step and never reached
the board. That run was stopped by hand before any lease.

The tip run was not started: it would stop at the same readiness check.

Stop rows (ms a frame `-`):

| commit | gc_tiled_rt | scene | ms a frame | FPS | verdict line |
|---|---|---|---|---|---|
| 72972ba4 | - | terrain | - | - | verdict= stop: no verdict line, rc=3 (`something is running (containers 1)`) |
| 72972ba4 | - | shadow | - | - | verdict= stop: not run (the command stopped at terrain) |
| 72972ba4 | - | refract | - | - | verdict= stop: not run (the command stopped at terrain) |
| ecd9743e | off | terrain | - | - | verdict= stop: not run |
| ecd9743e | off | shadow | - | - | verdict= stop: not run |
| ecd9743e | off | refract | - | - | verdict= stop: not run |

To run it next time: the board with no container running. Every run ends
with a reset and the launcher autostarts at each boot, so stopping it once
is not enough. A session (4 or 5 runs a scene, 3 scenes, 2 commits) needs
the launcher's autostart off for the session, the owner's decision. Then
build the two helpers and run the commands of "What is left" above, without
the console lease.

## Session A: stop, 2026-11-25 (plan #355, item x2-session-a)

**No X2 measurement was made. No image was built for the board, nothing was
staged or copied to it, no jump was made, the board was left as found, up on
Linux.** The rows below are stop rows, not measurements, and carry no trust
mark because they hold no number.

Commit: `3c3fe33b` (`origin/main` as this task's clone shows it,
`refs/connected/origin/remotes/origin/main`; `git fetch origin` failed here,
access denied). It carries W1-W5 (`9cdeff6d` is its ancestor). The work
started from the integration tip `ab919063`.

Leases held: `target:x2`, `target:x2-console` and `target:x2-display`, the
full set the run sheet names (`gc-tiled-wire.md`, "Session A: the run sheet").

What held, run sheet step 2 (reachability from the leased worker, first):

- ssh as root logs in from the leased worker (`uptime`: up 2 days 7 h).
- `tools/board.sh x2 status` (with `BOARD_HOST` from the lease) read:
  `board lease: none`, uptime 198066 s, runlevel `default`, `kexec_loaded 0`,
  `kexec_crash_loaded 0`, crash window `268435456`, containers `1`.
- The console resource opened (connected, 115200 8N1, read only; no character
  was sent to it).

What stopped it, unchanged since "Stop, 2026-11-24" above: **the podman
container `slint-launcher` still runs on the board** (`podman ps`: `Up 2
days`). `board.sh x2 run`'s readiness check (`x2_kexec_ready`,
`tools/board.sh:2518-2526`) refuses a jump while a container runs
(`something is running (containers 1)`, rc 3), and the status read's
`containers 1` is that check's input. Stopping the container or turning its
autostart off is the board owner's decision (leave the board as found,
persist nothing), so no run was started: it would end at rc 3 before staging,
as the 2026-11-24 terrain run did.

Not run, and so not answered: the knob-on picture match (the trust rule),
gl_test cases 3, 44 and 169, the RS rate for an 800x600 tiled to linear copy,
early depth under a TILED colour target, and whether TS works with the MMU
off (the W7-W9 gate stays closed). Case 243 was not run (a later session).

Stop rows (ms a frame `-`):

| commit | gc_tiled_rt | scene | ms a frame | FPS | verdict line | trust |
|---|---|---|---|---|---|---|
| 3c3fe33b | gc_tiled_rt off | terrain | - | - | verdict= stop: not run (`something is running (containers 1)`, board.sh x2 status) | no number |
| 3c3fe33b | gc_tiled_rt off | shadow | - | - | verdict= stop: not run (same cause) | no number |
| 3c3fe33b | gc_tiled_rt off | refract | - | - | verdict= stop: not run (same cause) | no number |
| 3c3fe33b | gc_tiled_rt on | terrain | - | - | verdict= stop: not run (same cause) | no number |
| 3c3fe33b | gc_tiled_rt on | shadow | - | - | verdict= stop: not run (same cause) | no number |
| 3c3fe33b | gc_tiled_rt on | refract | - | - | verdict= stop: not run (same cause) | no number |
| 3c3fe33b | gc_tiled_rt on | gl_test 3, 44, 169 | - | - | verdict= stop: not run (same cause) | no CASE record |

Re-check, same day, from a second leased worker (the task moved machines;
`origin/main` there reads `d4f95c29`): ssh as root logs in, `uptime` up 2
days 7 h 24 min, runlevel `default`, and `podman ps` still lists
`slint-launcher  Up 2 days`. The stop holds unchanged; again no image was
built, nothing was staged and no jump was made.

Third re-check, same day, from a third leased worker (`origin/main` there
also reads `d4f95c29`): ssh as root logs in, `uptime` up 2 days 7 h 41 min,
runlevel `default`, `podman ps` still lists `slint-launcher Up 2 days`.
`tools/board.sh x2 status` was not approved on that worker, so the ssh read
is the evidence. The stop holds; no image, no staging, no jump.

Fourth re-check, same day, from a fourth leased worker (`origin/main` there
reads `3454235c`): ssh as root logs in, `uptime` up 2 days 7 h 58 min,
runlevel `default`, `podman ps` still lists `slint-launcher Up 2 days`.
`tools/board.sh` was refused by the tool layer on that worker, so the ssh
read is the evidence. The stop holds; no image, no staging, no jump.

To run it: the board with no container running for the whole session (each
run ends with a SoC reset and the launcher autostarts at boot), then the run
sheet in `gc-tiled-wire.md` from step 1.

## Stop, 2026-11-25: session A not run (slint-launcher container)

**Session A did not run, and there is no measurement.** Three task machines
found the board up over ssh on 2026-11-25 (not rebooted, kexec not loaded)
with the podman container `slint-launcher` running and set to autostart.
`tools/board.sh x2 run` refuses a jump while any container runs
(`something is running (containers 1)`), and the plan's rules forbid an agent
from stopping it or changing its autostart. The owner did not answer. It is
the same stop as 2026-11-24 (record
`records/x2/20261124-1035-before-72972ba4`, no measurement) and the same cause
that stopped the look by eye (`x2-glmark2-pictures-by-eye.md`, commit
`fe72e2a9`). The 2026-11-25 attempts left no record, because no run started.
The "Session A: stop, 2026-11-25" section above holds its ssh reads; its
readiness-check citation is now `tools/board.sh:2518-2526`.

Commits that would have been measured, ms per frame for terrain, shadow and
refract: `72972ba4` (before), and `3c3fe33b` (`9cdeff6d`, W1 to W5, is its
ancestor) with `gc_tiled_rt` off and with `CONFIG_GLMARK2_GC_TILED_RT=y`. All
cells stay `-`.

The host prediction it would have been compared with
(`gc-slow-host-counts.md`, "Predicted effect of gc_tiled_rt"):

- terrain: about -14 to +3 ms of about 250. At current main the host counts
  51 terrain draws, not 71 (RS jobs per frame off/on: terrain 11/14, shadow
  3/3, refract 4/4).
- shadow and refract: unchanged. The depth-resolve cause is **unconfirmed**:
  it rests on a 350 to 1000 ms estimate taken from an AM62P figure, and the
  resolve takes about 1 ms on the host, so the host cannot decide whether it
  dominates. See `gc-slow-shadow-refract.md` (`gc-shadow-refract-cause.md`
  does not exist). A session should note whether its commit includes
  `25fa1a96` (wide loads in the resolve).

The X2 is the same SoC (i.MX8MP) and GPU (GC7000UL) as the Verdin iMX8MP and
now stands in for it (owner's rule of 2026-11-27, below). How to run it: the
update below; the owner's decision this stop waited on is no longer needed.

## Update, 2026-11-27 (plan #417, item x2-runner-app): the runner stops the containers for a run

**The stop above is lifted in `tools/board.sh` (`c10c1936`): an x2 run can
stop the board's application containers for the run, prints the undo first,
starts them again afterwards, and leaves autostart alone. The X2 stands in
for the Verdin iMX8MP for the GC7000UL depth-sample and slow-scene sessions
(owner's rule of 2026-11-27: the Verdin is away; same SoC, same GPU),
recorded rows included. Every x2 record now carries
`board_measured=Armadillo-X2 (i.MX 8M Plus, GC7000UL), stands in for the
Verdin iMX8MP` in its `meta.txt`.**

The behaviour as implemented (`tools/board.sh`, usage header; x2 `run` and
`stage`):

- `X2_STOP_APPS=1` is the opt-in. Unset or `0` keeps the old behaviour
  exactly: while a container runs, the readiness read refuses the jump
  (`something is running (containers N)`, rc 3).
- With `1`, once every other readiness word holds (OpenRC settled, kexec
  not loaded, crash window; nothing is stopped on a board refused for
  another reason), step 2 lists the running containers
  (`podman ps --format '{{.Names}}'`), validates each name, prints one
  `X2_STOP_APPS: undo: podman start <name>` line per container and records
  `undo=podman start <name>` in `meta.txt` before anything changes, then
  runs `podman stop -t 5 <name>` for each (recording `app_stop=`) and reads
  the readiness again; that second read (`board_state_stopped=`) must say
  `containers 0` before anything is staged.
- After the jump, once ssh is back — and likewise for a refusal after the
  stop and before the jump, and at the end of `stage` (which never jumps) —
  the runner reads the containers again and records `app_back=` per
  container: running again on its own, or not running with the recorded
  undo run once (`podman start <name>`) and what it left. If Linux does not
  come back, the `undo=` lines in `meta.txt` stand as the way to restore
  them.
- Autostart (`/etc/atmark/containers/*.conf`, podman-atmark), the container
  config and the rootfs are never touched.
- `tools/board_selftest.sh` pins all of this against a fake board (its
  section 6; 553 checks, 0 failed as it landed): the refusal without the
  opt-in, the undo recorded before the board's `podman stop`, `containers
  0` on the second read, the return check with and without autostart, two
  containers, nothing running, a refusal before and after the stop,
  `stage`, `SEL`.
- `kexec_zephyr` and `snvs_lpgpr` stay prerequisites: built on the worker,
  in `X2_TOOLS` (default `driver/tools/kexec-zephyr`;
  `driver/tools/kexec-zephyr/README.md`).

The read-only probe of 2026-11-27 found the board with no container running
and the containers' autostart off (`set_autostart no`), so the sessions can
run as the board is; with nothing running, `X2_STOP_APPS=1` does nothing but
one extra readiness read. With a container up, prefixing the script carries
the opt-in to every `board.sh` call it makes (it unsets only
`NETBOOT_NO_WATCHDOG`, `BOARD_UART` and `SEL`):

```sh
X2_STOP_APPS=1 tools/cts/x2-glmark2-scenes.sh --commit origin/main <outdir> terrain shadow refract
```

In each record, check `board_measured=`, and when the opt-in acted, the
`undo=`, `app_stop=` and `app_back=` lines of `meta.txt`.

## The crash route's interrupt check refused the jumps, 2026-11-27 (plan #417, items dtpu-x2 and gm-before-x2)

Plan #417's first two X2 sessions — case 222 `dtpu` (item dtpu-x2) and the
glmark2 before run (item gm-before-x2) — reached the board and were refused
at the jump by the crash route's interrupt check: `crash_irq_check` and its
measured-chip list `crash_irq_chips` in
`driver/tools/kexec-zephyr/crash_irq.h` (`84e7adff`, wired into
`kexec_zephyr --crash` by `4c0626d1`, both 2026-11-15, measured on the
NanoPC-T6). The X2 keeps handlers on two chips the T6 does not: `gpio-mxc`
(PWROFF, pca9450, REBOOT, rv8803, SW1, mmc cd) and `irqsteer` (dw-hdmi-cec,
imx-lcdifv3-crtc.1), so those rows of its `/proc/interrupts` were outside the
list. Readiness was fine (`containers 0`), the images built and the dry runs
read right; then the loader on the board printed "an interrupt is left on a chip
the crash route was not measured with ... not triggering", brought the CPUs
back and unloaded the crash image. Linux never went down, so the runner's
`not_back` line (rc 5) reports a jump that never happened: no verdict page,
no `gc_wdog_arm: armed` on the console, no ms a frame. The glmark2 run
stopped at its first scene (shadow); refract and terrain were not reached.
The full session records are `gc-depth-sample.md` ("The X2 session of
2026-11-27") and `gc-slow-shadow-refract.md` (its before table); until the
loader accepts the X2, no cell of those tables could fill.

**Fixed in `fa8dcf85`: `crash_irq.h` takes a per-board chip list.** The list
is picked at run time from the device tree's compatible string
(`crash_irq_board_imx8mp`): `fsl,imx8mp` adds `gpio-mxc` and `irqsteer` to
the measured chips; any other board, and an unreadable compatible, keeps the
T6 list alone. The two chips are MMIO chips that set no irq bus lock, so the
chip bus lock the panic path takes is the kernel's no-op and never sleeps;
the evidence for them is the X2's own crash-route jumps of 2026-11-06 and
2026-11-11, made with loaders from before the check existed. `crash_irq_test.c`
covers the parsing on the host. With the X2 list picked, the loader prints
`crash: i.MX 8M Plus (fsl,imx8mp): gpio-mxc and irqsteer rows are let
through`, and the first jump with it is watched on the X2 console: that line
is what shows the X2 list was in use. The next X2 sessions run with it: the
dtpu rerun of `gc-depth-sample.md` unchanged, and the glmark2 before rerun
can reuse its three images (built from `02c5c9af`).

## Stop, 2026-11-27 (plan #355, item x2-before-rerun): the board came back, but not to this worker's ssh key

**No board.sh row was measured: the first run ended `verdict=not_back` and
the command stopped with no retry.** Measured on the Armadillo-X2 (i.MX 8M
Plus, GC7000UL), standing in for the Verdin iMX8MP; the record's `meta.txt`
says `board_measured=Armadillo-X2 (i.MX 8M Plus, GC7000UL), stands in for the
Verdin iMX8MP`. Record: `records/x2/20261127-0954-before-72972ba4/`
(gitignored, redacted).

Commits: before `72972ba4`; tip `b947cc22` (`origin/main` as this task's
clone shows it, `refs/connected/origin/remotes/origin/main`; `git fetch
origin` failed here, access denied). The tip run was not started. Leases:
`target:x2` and `target:x2-console` (read only, no character sent).

What held this time:

- Reachability: ssh as root from the leased worker logged in, and
  `tools/board.sh x2 status` read `board lease: none`, runlevel `default`,
  `kexec_loaded 0`, `kexec_crash_loaded 0`, crash window `268435456`.
- `kexec_zephyr` and `snvs_lpgpr` built from the tip (`b947cc22`, which has
  `fa8dcf85` and `c10c1936`) with `clang --target=aarch64-none-elf -O2 -static
  -nostdlib -ffreestanding -fno-stack-protector` and the Zephyr SDK's
  `ld.bfd` (the SDK's `aarch64-zephyr-elf-gcc` fails the link: its default
  script's `ram` region overflows by 47648 bytes).
- The command was `X2_STOP_APPS=1 tools/cts/x2-glmark2-scenes.sh --commit
  72972ba4 records/x2/20261127-0954-before-72972ba4 terrain shadow refract`.
  A dry run of the terrain image first returned rc 0 for all four runs. The
  terrain image (sha256 `0a20645ad2108716...`, footprint 157057024, marker
  `gc_wdog_arm: armed` present) ran with `board_state=openrc settled uptime
  3498s kexec=0 containers=0`, so `X2_STOP_APPS=1` stopped nothing (no `undo=`,
  `app_stop=` or `app_back=` line in `meta.txt`).
- **The crash list held.** The loader printed (`kexec.log`):
  `crash: i.MX 8M Plus (fsl,imx8mp): gpio-mxc and irqsteer rows are let through`
  and `crash: /proc/interrupts: every row with a handler is on a measured chip
  (...; i.MX 8M Plus also gpio-mxc, irqsteer)`. `jump=ok`: this is the first
  jump with `fa8dcf85`'s loader.
- **The image ran terrain to the end.** The console (read only) showed the
  crashdump kernel start, `sample: build tree=72972ba4`, glmark2
  `[terrain] <default>: FPS: 5 FrameTime: 247.588 ms`, `glmark2-frame: ... 41
  frames; us per frame: gl 224580, swap 22081, show 136, other 0, frame
  246797`, `glmark2-steps: ... gc frame 213117 ...`, `yk-record: 594b120e
  fc3f0004 6104003f 00258320`, `glmark2: PASS`, then `board: resetting the
  SoC`, U-Boot's `reset cause: watchdog` and Linux to `armadillo login:`
  about a minute after the jump. The image adopted the LCDIF at 1920x1080.
  `gc_wdog_arm: armed` does not show on the console (printed before the
  console starts, as the resource notes say); the build check found it in
  the image.

What stopped it: **after the reboot, the board refused this task machine's
ssh key** (`Permission denied (publickey,...)`). The key connected installs
for a task machine goes into `/root/.ssh` on the board's tmpfs overlay; only
the farm nodes' keys are persisted, so every reboot drops it. `board.sh`
waited its 1500 s for ssh, then wrote:

```
board.sh x2 terrain-nosel rc=5 verdict=not_back boot=kexec-dev DEV boot_verdict=not_back result=none resets=absent record=absent back_after=?s (kexec-dev: a development run, never a row)
x2-glmark2-scenes: the board did not come back (rc 5): stopped, no retry; it needs a person
```

(`meta.txt`: `jump=ok`, `lost=10`, `timeline=jump 0s, ping lost 10s, ping
back ?s, ssh back ?s, route --crash`, `rc_cycle=5`.) Under the session rule,
there was no retry. The tip run and the selector runs (`0x1000`, `0x1001`,
`0x1003`) were not started, so no PERF page was read and no record words
came from LPGPR. The board does **not** need a power cycle. Its console shows
Linux at the login prompt. What it needs is a login after the reboot:
either a session from a worker whose key survives a reboot (a farm node), or
the task machine's key kept across the run's reboot. Both are owner
decisions, and this item persists nothing. The board was found and left with
no container running. The staged files in `/tmp/yakogl-kexec` went with the
reboot. LPGPR1..3 hold what the run wrote. After the stop, an ssh login could
not check anything.

Rows (`x2-scene-table.py --before 72972ba4` on the record prints the
not_back row, with the image sha8 in its commit column, and no pair). The
console figure is an observation, not a board.sh row:

| commit | gc_tiled_rt | scene | ms a frame | FPS | verdict line | record words |
|---|---|---|---|---|---|---|
| 72972ba4 | - | terrain | - (console only: FrameTime 247.588, frame 246.797 us/1000) | - (console: 5) | verdict= `board.sh x2 terrain-nosel rc=5 verdict=not_back boot=kexec-dev DEV boot_verdict=not_back result=none resets=absent record=absent back_after=?s` | record=absent (console: `yk-record: 594b120e fc3f0004 6104003f 00258320`) |
| 72972ba4 | - | shadow | - | - | verdict= stop: not run (the command stopped after terrain's not_back) | - |
| 72972ba4 | - | refract | - | - | verdict= stop: not run (same) | - |
| b947cc22 | off | terrain | - | - | verdict= stop: not run (same) | - |
| b947cc22 | off | shadow | - | - | verdict= stop: not run (same) | - |
| b947cc22 | off | refract | - | - | verdict= stop: not run (same) | - |

Board checked afterwards, same day, from the same worker at its next turn.
connected installs the key again when a turn starts, so ssh logged in:
uptime 5116 s (the boot after this run's reset), `podman ps` 0 containers,
`kexec_crash_loaded 0`. The board is as found. No further jump was made: it
would lose the key the same way and end `not_back` again, a retry under the
stop rule. With the key installed once a turn, a task machine gets at most
one jump a turn, and board.sh can never read that jump's record.

To run it next time: the same commands, with ssh that survives the run's
reboot. The loader and the readiness checks no longer stop it.

Reading a jump's record in the next turn: `tools/cts/x2-glmark2-scenes.sh
--collect records/x2` prints the scene table from the record directories
(verdict line, record words, decoded page, ms a frame) and checks each
console.txt for `gc_wdog_arm: armed`, with no board, no jump and no ssh. A
task machine's key is gone after the jump's reboot, so board.sh could not read
that record in the jump's own turn; the turn after it can.
