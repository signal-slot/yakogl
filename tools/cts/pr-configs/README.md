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

| fragment | sample | what it is for | record it reproduces |
|---|---|---|---|
| `gl_test-175-off.conf` | gl_test | case 175 (refusehang), the knob off: the known negative, the refused grow at A = 0 ends in the 10 s stop | `tools/cts/log/2026-10-31-pr-neg175` image A (`pr-session.sh <name> --knob off`) |
| `gl_test-175-on.conf` | gl_test | case 175, the knob on: the kick completes through the firmware's partial render | `tools/cts/log/2026-10-31-pr-neg175` image B (`pr-session.sh <name> --knob on`) |
| `glmark2-d320c8a5-off.conf` | glmark2 | d320c8a5's configuration (one kick in flight), the knob off: the stop of 2026-10-04 | image d320c8a5, docs/notes/pvr-render-pool.md "The one-kick-in-flight run: d320c8a5" (`pr-session.sh <name> --knob off --glmark2 --one-in-flight`) |
| `glmark2-d320c8a5-on.conf` | glmark2 | d320c8a5's configuration, the knob on: the refused grow must complete, partial renders >= 1, no stop | the note's "first board measurement" step 2 (`pr-session.sh <name> --knob on --glmark2 --one-in-flight`) |

Every fragment sets `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=0` (the
probe off, the lists free to ask for a grow): that is `pr-session.sh`'s
default cap and what the neg175 session built with (`FL_PROBE_MAX_PAGES=0`
in its README). Case 175 is not run with the cap at 512.

## Building with a fragment

From the west workspace (`ZEPHYR_WS`), with the modules list and the
AM62P's watchdog arguments of `tools/cts/common-images.sh`, exactly as
`pr-session.sh --dry-run` prints them, the fragment taking the place of
the `-DCONFIG_...` options:

```sh
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
F=$PWD/tools/cts/pr-configs/gl_test-175-off.conf      # or any fragment above
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

- The neg175 images also had `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y`
  (the `forcedpr-images.sh` flow, image 0ea4c598's refusal config);
  `pr-session.sh` does not pass it, and the fragments follow
  `pr-session.sh`. A rebuild of neg175's exact `.config` adds it on the
  command line.
- Image d320c8a5 was ae18b42d's tree of 2026-10-04 (the ready-pages and
  grow-scene options of that tree) plus one kick in flight. Those options
  are not re-set here: the fragments give d320c8a5's configuration on the
  current tree's defaults, which is what the partial-render session runs.
