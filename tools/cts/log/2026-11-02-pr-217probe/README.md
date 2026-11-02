<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Case 217 (forcedpr): the crossing is 1216 layers; both knob states stop with cause 2

Run 2026-11-02, AM62P (Verdin), item board-pr-first of plan #177 (issue #2/#3).

**Result:**
- **Crossing: 1216 layers.** With the global lists capped at 512 pages, 1152
  layers fit the list and 1216 run the parameter manager out of memory. The
  committed default (`CONFIG_GL_TEST_FORCEDPR_LAYERS=1216`) is that count, so
  the case's first kick is the one that runs out.
- **Grow requests: 0 at the crossing, not >= 1.** At the 512-page cap the
  list is already at its maximum, so the firmware asks for no grow. The
  crossing shows as the UPDATE_STATS out-of-memory notice (`notices 1`). The
  contract's "requests >= 1" cannot happen with this cap.
- **Knob off: the known negative.** Stop cause 2, partial renders 0, HWR 0.
  `pr-verdict --expect stop`: OK.
- **Knob on: not met.** The run is the same: stop cause 2, partial renders 0,
  HWR 0. The driver's at-maximum stop (`driver/rust/src/gpu/pvr/ccb.rs`,
  `at_maximum`, cause `STOP_AT_MAXIMUM`) fires on the first out-of-memory
  notice whatever the knob says. The firmware's partial render is therefore
  never waited for. `pr-verdict --expect pr`: FAIL. There is a hint that the
  firmware went on: at close the knob-on image had `pending frees now 0`
  (fence 1 had passed), where the knob-off image still held 4 frees
  (`oldest fence 1`). The FWCCB is not serviced after the stop, so the
  partial-render count stays 0. The knob's HWRT words are present
  (`hrt[32]: 400e9e80 400e9f00 00000003`, zeros with the knob off).
- The frame side (pr-frame-colour, bgload-pds, pr-zs) has not landed, so the
  knob-on picture would not be expected to match yet. It was not reached:
  the case never read back.

## Images

All images are the `gl_test` sample for `verdin_am62p/am62p54/a53`, built by
`driver/tools/pvrlab/forcedpr-images.sh` (`ZEPHYR_WS` = the deps cache's west
workspace, `driver/src/fw/pvr_fw_blob.h` copied from the deps cache,
untracked). Common Kconfig: `CONFIG_GL_TEST_CASES="217"`,
`CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512`,
`CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y` (60 s expiry, fed for 150 s),
`CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`.

| run | source | layers | knob | zephyr.bin sha256 |
|---|---|---|---|---|
| sweep | `51e4a974` (on origin/main) | 256, doubling | off | `3be07e74cc4bb0b9f90a67498bb73558a5a9127250d5ca6bd168c2e4a21e3589` |
| probe64 | `51e4a974` + the sweep's step changed by hand to +64 (not committed; prints `51e4a974-dirty`) | 1024, +64 | off | `a1ec2cb3686d1474b010749f90ebff50b0ff92b29fbcbbbae4172f4a4e60d6bd` |
| off | `2259551f` (this branch: `51e4a974` + the case 217 change only) | 1216 (default) | off (`# CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER is not set`) | `e31d07f8524499fedcbedd4ef848968d60d7868cfa7627033309caffa8660e65` |
| on | `2259551f` | 1216 (default) | on (`CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y`) | `286f23b1061a101d218304d20db8447b64ce290a89de70dbe0aae382780f174c` |

The off and on `.config` files differ in that one line. Both have
`CONFIG_GL_TEST_FORCEDPR_LAYERS=1216`. `2259551f` is not on origin/main. It is
`51e4a974` (on origin/main when this clone was made) with only the case 217
change on top (gl_test.c, its Kconfig default, the pvrlab README,
forcedpr-expected.py and its selftest).

## Commands

`tools/board.sh` / `tools/cts/pr-session.sh` were not used: this task
machine reaches the console only through the leased serial service, not
through a TCP bridge that `board.sh` can capture. The board's `uboot-mmc`
one-shot was done by hand, as `board.sh` does it. The image was written to
`/var/yakogl/once-<token>.bin` and read back by sha256 with the page cache
dropped. Then `fw_setenv -s` set `yako_once`, `yako_src=mmc` and `yako_file`,
and the board ran `systemctl reboot`. Each image reset the SoC itself
(`sample done (-1): resetting the SoC`) and Linux came back. One GL client ran
at a time. Afterwards the staged images were removed, `yako_file` was put back
to its previous value, and `yako_once` and `yako_default` are empty.

Verdicts: `python3 -I tools/cts/pr-verdict.py --expect stop off.log 217` and
`--expect pr on.log 217`.

## Sweep lines

```
sweep:   step 1: 256 layers: no grow request
         step 2: 512 layers: no grow request
         step 3: 1024 layers: no grow request
         FAIL stopped: 2048 layers (4 steps): requests 0 (served 0, refused 0), notices 1, stop 2, partial renders 0, GPU recoveries 0
probe64: step 1: 1024 layers: no grow request
         step 2: 1088 layers: no grow request
         step 3: 1152 layers: no grow request
         step 4: 1216 layers: no grow request, out of memory
         FAIL stopped: 1216 layers (4 steps): requests 0 (served 0, refused 0), notices 1, stop 2, partial renders 0, GPU recoveries 0
```

(The sweep image predates the "out of memory" step suffix, so its step 4
says only "no grow request".)

## Verdict lines (verbatim)

Knob off (`off.log`, `--expect stop`):

```
pr-verdict: case 217 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause2
pr-verdict: grow requests 0 served 0 unserved 0
pr-verdict: OK
```

Knob on (`on.log`, `--expect pr`):

```
pr-verdict: case 217 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause2
pr-verdict: grow requests 0 served 0 unserved 0
pr-verdict: class 8 stop cause 2, not a partial-render failure (knob probably off)
pr-verdict: FAIL case 217 is TIMEOUT; partial renders 0 (want >= 1); render stopped, cause 2
```

(The sweep and probe64 logs also give `OK` with `--expect stop`.)

## Files

`sweep.log`, `probe64.log`, `off.log` and `on.log` are excerpts of the serial
console. Each runs from U-Boot's `yakogl-netboot: once=` line to the image's
reset. Display bring-up, firmware-load detail and the `render pool` sub-lines
were left out. The freelist and `init ok` addresses were replaced by
`<addresses dropped>`. The `hc dwords` / `hrt[]` dumps were dropped, except
the knob-on `hrt[32]` line. No board address, IP, port or credential was in
the captured text.
