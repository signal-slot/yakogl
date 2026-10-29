# Partial-render board configurations (Kconfig fragments)

The AM62P partial-render sessions (issue #3) build two configurations that
were prose in `docs/notes/pvr-render-pool.md`. Each is a Kconfig fragment
here, with the knob in a third fragment so every configuration runs knob off
and knob on from one image recipe. Built images stay in `build/`, never
committed.

| fragment | sample | what it is |
|---|---|---|
| `case175.conf` | `driver/samples/zephyr/gl_test` | gl_test case 175 (refusehang): the refused grow with no 3D pending |
| `glmark2-d320.conf` | `driver/samples/zephyr/glmark2` | glmark2 as image d320c8a5 ran (one kick in flight) |
| `partial-render-on.conf` | either | the knob; absent = knob off |

## Provenance, line by line

Note sections are in `docs/notes/pvr-render-pool.md`; "Kconfig" is
`driver/zephyr/Kconfig` unless named.

### case175.conf

Note section "The AM62P glmark2 cell: the refused-grow hang's minimal
reproducer (gl_test 175 refusehang)" (line 3975). It says "Image 0ea4c598
(`build/rfh-am62p`, GL_TEST_CASES="175", `NO_RESERVE`, nothing else)"
(line 3989-3990) and "every grow knob off, `NO_RESERVE`" (line 4060).

- `CONFIG_GL_TEST_CASES="175"`: note line 3990. Option:
  `driver/samples/zephyr/gl_test/Kconfig:68`.
- `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y`: note line 3990 (also
  3981 and the probe section, line 968). Option: Kconfig:126. It
  `depends on !YAKOGL_DRV_GPU_PVR_FL_GROW_ON_DEMAND`, so it cannot be layered
  with the d320 fragment (which sets the grow knob).

Not in the fragment: `FWCCB_CHECK`, `FW_UNCACHED` and the grow knobs are not
set ("nothing else"). Differences from the note's image, none renamed or
removed:
- `YAKOGL_DRV_GPU_PVR_FW_UNCACHED` is `default y` today (Kconfig:139), and
  the note's case 175 image was built when it was off (the opt-in, line
  3762, 147d27f8). The fragment keeps today's default: the knob-off and
  knob-on arms must differ only in the partial render. For the note's literal
  image add `CONFIG_YAKOGL_DRV_GPU_PVR_FW_UNCACHED=n`.
- The `case 175` itself is `driver/tools/pvrlab/gl_test.c`, not a Kconfig
  matter.

### glmark2-d320.conf

Note section "The one-kick-in-flight run: d320c8a5" (line 2544): "Image
d320c8a5 (ae18b42d's tree + `CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y`)"
(line 2546, 2528). ae18b42d is "615b0839: knob on, ready pages on, grow
scene on, landed frame sets" (line 2431, "Image ae18b42d").

- `CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y`: line 2546. Option:
  `driver/samples/zephyr/glmark2/Kconfig:27` (needs `YAKOGL_DRV_GPU_PVR`).
- `CONFIG_YAKOGL_DRV_GPU_PVR_FL_GROW_ON_DEMAND=y` ("knob on", line 2431):
  Kconfig:36. The note names it "the grow knob" (line 3769).
- `CONFIG_YAKOGL_DRV_GPU_PVR_FL_READY_PAGES=y` ("ready pages on", line 2431):
  Kconfig:51.
- `CONFIG_YAKOGL_DRV_GPU_PVR_GROW_SCENE=y` ("grow scene on", line 2431, and
  "The instrument" line 2413): Kconfig:87.

"Landed frame sets" is code in the tree, not an option. The d320c8a5 console
shows `PVR_BACKEND_OPT_ONE_IN_FLIGHT=1` (note, run section). The fragment does not set
`KICK_TIMES` or `FWCCB_CHECK`: the note names neither for this image.

## Layering

`common-images.sh` and `board-stage.sh` pass no extra conf file, and they are
not edited. Zephyr's own `-DEXTRA_CONF_FILE=` takes a `;`-separated list
merged after `prj.conf` and the board's `.conf` (the repository already uses
it: `tools/cts/pi4-deqp-images.sh:81`, `tools/cts/imx8mp-gltest-build.sh:50`,
`driver/samples/zephyr/gl_test/CMakeLists.txt:19-34`). Use absolute paths.
`OVERLAY_CONFIG` is not used anywhere in the repository. The arguments are
`tools/cts/common-images.sh --dry-run am62p tools/cts/common-tree.txt <dir>`
plus the fragments:

```sh
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr PATH=$WEST_VENV/bin:$PATH
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
P=$PWD/tools/cts/pr-configs
# gl_test case 175, knob off; add ;$P/partial-render-on.conf for knob on
(cd "$ZEPHYR_WS" && west build -p always -b verdin_am62p/am62p54/a53 -d build/c175 \
  "$OLDPWD/driver/samples/zephyr/gl_test" -- "-DZEPHYR_MODULES=$MODS" -DYAKOGL_DIR=$OLDPWD \
  -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n \
  "-DEXTRA_CONF_FILE=$P/case175.conf")
# glmark2, d320 configuration; add ;$P/partial-render-on.conf for knob on
#   ... driver/samples/zephyr/glmark2 -- ... -DGLMARK2_SRC=$GLMARK2_SRC \
#       "-DEXTRA_CONF_FILE=$P/glmark2-d320.conf"
```

(`-DCONFIG_GL_TEST_CASES=...`, which `common-images.sh --cases 175` passes, is
the same as the fragment's first line; `--cases 175` plus the NO_RESERVE
fragment line is equivalent.)

Host check (2026-10-29, `west build --cmake-only`, board
`verdin_am62p/am62p54/a53`): case175, case175 + on, glmark2-d320 and
glmark2-d320 + on all configured with rc=0, and each `.config` carried every
fragment line as set (no "assigned but got n" error).

Order matters only for the knob: list `partial-render-on.conf` last.
