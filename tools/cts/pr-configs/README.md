<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Kconfig fragments of the PowerVR partial-render board sessions

The AM62P partial-render sessions (issue #2; docs/notes/pvr-render-pool.md,
"Design: the firmware's partial render") are built by
`tools/cts/pr-session.sh`, which passes its Kconfig as `-D` options (its
`OVERLAY` array and `--one-in-flight`). These fragments hold the same
settings as committed files, so that a person can rebuild exactly what a
record used with `-DEXTRA_CONF_FILE=<fragment>`. `selftest.sh` here checks
that every symbol a fragment sets is declared in `driver/zephyr/Kconfig`,
`driver/samples/zephyr/glmark2/Kconfig` or
`driver/samples/zephyr/gl_test/Kconfig`, and that the values agree with what
`pr-session.sh --dry-run` prints for the matching options.
`pr-session.sh --leg 217-off|217-on` builds the `gl_test-217-off/on` pair
from these fragments (`-DEXTRA_CONF_FILE`), and `selftest.sh` checks the leg
names exactly these files.

| fragment | sample | what it is for | record it reproduces |
|---|---|---|---|
| `gl_test-175-off.conf` | gl_test | case 175 (refusehang), the knob off: the known negative, the refused grow at A = 0 ends in the 10 s stop | `tools/cts/log/2026-10-31-pr-neg175` image A (`pr-session.sh <name> --knob off`) |
| `gl_test-175-on.conf` | gl_test | case 175, the knob on: the kick completes through the firmware's partial render | `tools/cts/log/2026-10-31-pr-neg175` image B (`pr-session.sh <name> --knob on`) |
| `gl_test-217-off.conf` | gl_test | case 217 (forcedpr, acceptance c2), the knob off, list capped at 512 pages, 1220 layers: the known negative — the out-of-memory notice at a list at its maximum ends in the at-maximum stop, cause 2 | `tools/cts/log/2026-11-02-pr-217probe` run A (`pr-session.sh <name> --leg 217-off`); the same cap, case and knob ran earlier in `tools/cts/log/2026-10-31-pr-negative`, at the then-default 256 layers, where the list was not filled |
| `gl_test-217-on.conf` | gl_test | case 217, the knob on, the same cap and layers: the acceptance run, judged `pr-verdict.py --expect pr` | `tools/cts/log/2026-11-02-pr-217probe` run B (`pr-session.sh <name> --leg 217-on`): the same cause-2 stop, measured before the driver's at-maximum wait landed (`9b2e88bc`) |
| `gl_test-217-runC-off.conf` | gl_test | case 217 in run C's fallback configuration, the knob off: no cap, no grow reserve, 1220 layers; the list is free to ask, the refused grow stops the run | the knob-off partner of 217probe run C, never run itself (`pr-session.sh <name> --knob off --cap-pages 0 --cases 217`, plus the fragment's `FL_PROBE_NO_RESERVE=y`) |
| `gl_test-217-runC-on.conf` | gl_test | run C, the knob on: the grow request refused, the firmware's one partial render, the kick complete, the pixels not yet equal (the frame side is pr-frame-colour's) | `tools/cts/log/2026-11-02-pr-217probe` run C (built with `pr-session.sh <name> --knob on --cap-pages 0 --cases 217`'s arguments plus `FL_PROBE_NO_RESERVE=y`) |
| `glmark2-d320c8a5-off.conf` | glmark2 | d320c8a5's configuration (one kick in flight), the knob off: the stop of 2026-10-04 | image d320c8a5, docs/notes/pvr-render-pool.md "The one-kick-in-flight run: d320c8a5" (`pr-session.sh <name> --knob off --glmark2 --one-in-flight`) |
| `glmark2-d320c8a5-on.conf` | glmark2 | d320c8a5's configuration, the knob on: the refused grow must complete, partial renders >= 1, no stop | the note's "first board measurement" step 2 (`pr-session.sh <name> --knob on --glmark2 --one-in-flight`) |

The 175 and glmark2 fragments set `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=0`
(the probe off, the lists free to ask for a grow): that is `pr-session.sh`'s
default cap and what the neg175 session built with (`FL_PROBE_MAX_PAGES=0`
in its README). Case 175 is not run with the cap at 512. The two
`gl_test-217-{off,on}` fragments cap the list at 512 (the forced-PR case is
built to exceed it); the two `gl_test-217-runC-*` fragments have no cap and
also set `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y` (case 175's
refusal configuration: the list free to ask, every grow answered with zero
pages). All four 217 fragments set `CONFIG_GL_TEST_FORCEDPR_LAYERS=1220`,
the case's default and the AM62P's crossing (976 layers fit the 512 pages,
1220 do not: the 217probe record), explicitly so a change of the default
does not move them. Those two settings (layers, no reserve) have no
`pr-session.sh` option; `selftest.sh` accepts them only in the 217
fragments. gl_test has no one-kick-in-flight option (it is glmark2's), so
the 217 fragments carry none. `selftest.sh` compares the 217 pair against
the `--leg` dry run's options and the runC pair against the
`--knob <off|on> --cap-pages 0 --cases 217` line. In each `-off`/`-on` pair
the knob is the only difference; `selftest.sh` checks that.

## Building with a fragment

The 217 pair has the short route: `tools/cts/pr-session.sh <name> --leg
217-off` (or `--leg 217-on`) builds the gl_test image with exactly the
pair's fragment as `-DEXTRA_CONF_FILE`, runs it on the board and judges the
log with `pr-verdict.py --expect stop` (`--expect pr`), case 217. Every
fragment, that pair included, can also be built by hand. From the west
workspace (`ZEPHYR_WS`), with the modules list and the
AM62P's watchdog arguments of `tools/cts/common-images.sh`, exactly as
`pr-session.sh --dry-run` prints them, the fragment taking the place of
the `-DCONFIG_...` options:

```sh
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
F=$PWD/tools/cts/pr-configs/gl_test-217-on.conf       # or any fragment above
(cd "$ZEPHYR_WS" && west build -p always -b verdin_am62p/am62p54/a53 \
    -d build/pr/<name>/gltest <repo>/driver/samples/zephyr/gl_test -- \
    -DZEPHYR_MODULES="$MODS" -DYAKOGL_DIR=<repo> \
    -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n \
    -DEXTRA_CONF_FILE="$F")
```

For the glmark2 fragments the sample is `driver/samples/zephyr/glmark2`,
the build directory `build/pr/<name>/glmark2`, and the glmark2 source is
named with `-DGLMARK2_SRC=$GLMARK2_SRC` (`tools/cts/deps-env.sh am62p`).
The run is `tools/board.sh am62p run <build>/zephyr/zephyr.bin <logdir>
<name>` and the verdict `python3 tools/cts/pr-verdict.py <log>`, as
`pr-session.sh` does. The watchdog arguments and `GLMARK2_SRC` are the
board's and the host's, not the session's, so they stay on the command line.

## Not in the fragments

- Run C's no reserve (`CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y`) is
  in the `gl_test-217-runC-*` fragments. `pr-session.sh` has no option that
  passes it, so a plain `--knob ... --cap-pages 0 --cases 217` build lacks
  it and reproduces run C only with the fragment; the 175 fragments follow
  `pr-session.sh` and omit it.
- The neg175 images also had `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y`
  (the `forcedpr-images.sh` flow, image 0ea4c598's refusal config);
  `pr-session.sh` does not pass it, and the fragments follow
  `pr-session.sh`. A rebuild of neg175's exact `.config` adds it on the
  command line.
- Image d320c8a5 was ae18b42d's tree of 2026-10-04 (the ready-pages and
  grow-scene options of that tree) plus one kick in flight. Those options
  are not re-set here: the fragments give d320c8a5's configuration on the
  current tree's defaults, which is what the partial-render session runs.
