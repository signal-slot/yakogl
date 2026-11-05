<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Case 217 (forcedpr): the layer crossing on the AM62P, and both knob states at it

Run 2026-11-05 (04:37-04:52 UTC), AM62P (Verdin), item board-pr-first of plan
#198 (issue #2/#3). The directory is named for the item, not the run date.

**Result in one paragraph.** The crossing is between **976 and 1220 layers**:
976 of the case's 512x512 rectangles fit the 512-page list, 1220 do not. The
design note's prediction (2013 to 2514) was about twice too high. The
committed default `CONFIG_GL_TEST_FORCEDPR_LAYERS` is now **1220**, and at it
the first kick runs out of parameter memory. With the knob off that is the
known negative, stop cause 2 (`pr-verdict: OK` under `--expect stop`). With
the knob on **and the cap at 512 the run is byte for byte the same stop**:
the firmware sends no grow request at a list that is at its maximum, it sends
the out-of-memory notice alone, and the driver's at-maximum stop
(`driver/rust/src/gpu/pvr/ccb.rs`, `at_maximum`, not gated by the knob) ends
the run before any partial render can happen (`partial renders 0`,
`pr-verdict: FAIL`). So the contract's knob-on run at the cap cannot pass
without a driver change, which is outside this item. A third run, the knob on
with case 175's refusal configuration (no grow reserve, cap off), shows what
the firmware does with this frame when it is allowed to ask: one grow request,
refused, **one partial render, the kick complete, 0 GPU recoveries, no stop,
the depth read-back equal, the colour mostly the clear** (`FAIL pixels`, the
frame side is not in: pr-frame-colour, bgload-pds).

## The crossing (image S, `sweep-off.log`)

The case now sweeps as case 175 does, up by a quarter a step from the Kconfig
start, and ends the sweep at the first step the firmware ran out of parameter
memory in: a grow request, or an out-of-memory notice alone (the capped list).
From 256:

| step | layers | result |
|---|---|---|
| 1-7 | 256, 320, 400, 500, 625, 781, 976 | `no grow request`, kick complete |
| 8 | 1220 | `FWCCB: UPDATE_STATS element=2 (out of memory)`, `render: stopped: ... at the maximum (list 1, 0 KiB)`, cause 2 |

So a layer costs between 2048/1220 = 1.68 and 2048/976 = 2.10 KiB of the
2048 KiB list, about twice the note's 0.82 to 1.02 KiB (which scaled case
175's full-screen quad by the tiles a 512x512 rectangle touches). The same
sweep with the knob on (image T, `sweep-on.log`) is identical line for line
but for the HWRT words the knob writes (`hrt[32..34]`).

The default is fixed at the first step that asked, 1220. At the default the
first kick asks (images A, B, C: `step 1: 1220 layers`), so the committed
case is deterministic; the sweep above it is kept for a board or a list size
that differs. The expected image at 1220 layers (sha256
`8a0aee698cbd38f3c83f576b7612d37374c74da63bf67f299efa0bf724794ccf`, 100 of the
1220 layers win a pixel, 1120 win none) is pinned by
`tools/cts/forcedpr-expected-selftest.sh`.

## Images

All five images are the gl_test sample for `verdin_am62p/am62p54/a53` built
from this checkout: origin/main `93f9c958` plus this item's change to
`driver/tools/pvrlab/gl_test.c` and `driver/samples/zephyr/gl_test/Kconfig`
(the sweep by a quarter, the notice as the ask, the default 1220); the images
print `build tree=93f9c958-dirty`. The commit that carries the change is the
one this directory lands in. `driver/src/fw/pvr_fw_blob.h` came from the deps
cache (untracked, never committed).

- S, T: `driver/tools/pvrlab/forcedpr-images.sh <out>` with the default
  still 256 (S = `off`, T = `on`).
- A, B: `driver/tools/pvrlab/forcedpr-images.sh <out>` with the default 1220
  (A = `off`, B = `on`): `CONFIG_GL_TEST_CASES="217"`,
  `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512`,
  `CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`,
  `CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`, and the knob
  `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER` n (A) or y (B); the two
  `.config` files differ in that one line. Resulting:
  `CONFIG_GL_TEST_FORCEDPR_LAYERS=1220`, `FL_PROBE_NO_RESERVE` unset,
  `FL_GROW_ON_DEMAND` unset, `YAKOGL_AM62P_WDT_SECONDS=150`.
- C: the same sample configured with cmake directly (the arguments
  `pr-session.sh <name> --knob on --cap-pages 0 --cases 217` would pass, plus
  `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y`): case 175's refusal
  configuration of `tools/cts/log/2026-10-31-pr-neg175` image B
  (`FL_PROBE_MAX_PAGES=0`, no reserve, the list at 448 + 64 of 4096 pages),
  `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y`, `CONFIG_GL_TEST_FORCEDPR_LAYERS=1220`.
  Not one of the contract's two runs.

zephyr.bin sha256:

| image | knob | cap | layers | sha256 | size |
|---|---|---|---|---|---|
| S | off | 512 | 256 (sweep) | `4cb49e3a2c576253fd50bfe4f74a27b122c7928a2e0de27c70cbecca8e679be4` | 3128624 |
| T | on | 512 | 256 (sweep) | `2ca41ef9f0f876845d98dbcd4e325db57445b13c56c4c96f384c36de30e8c601` | 3128632 |
| A | off | 512 | 1220 | `43bad0dd16cb23c904826e3eacd6e1cd33183461fe41583ffa3008e456427eb4` | 3128624 |
| B | on | 512 | 1220 | `a39481442258c653b3a850fcd229492812500bc26cd1a3b66d8da6aca0ce65a6` | 3128632 |
| C | on | 0, no reserve | 1220 | `ceb6dba331809e1394b7c30212d6d236e6e1edd9f8d449ddecc591474e4db862` | 3128632 |

Watchdog: every image self-arms the RTI (60 s expiry, fed for 150 s,
`rti_arm: armed` in each log). No run needed it: each image reset the SoC
itself after its 20 s hold (`sample done (-1): resetting the SoC`) and Linux
was back within about 60 s. One GL client ran at a time.

## Commands

`tools/cts/pr-session.sh` was not used to run: its `tools/board.sh am62p run`
needs the serial bridge, which the leased MCP console holds (`probe: busy`),
and the feed bridge needs a relay this task machine could not run. The
`uboot-mmc` one-shot was done step by step as `board.sh` does it, from this
host over ssh: the image written to `/var/yakogl/once-<token>.bin`, read back
by sha256 with the page cache dropped, armed with `fw_setenv -s`
(`yako_once=<token>`, `yako_src=mmc`, `yako_file=<U-Boot path>`), read back,
`systemctl reboot`; afterwards `yako_once` read back empty and the staged
image was removed. The console was read through the leased serial console.
`yako_file` is left naming the last token's path (the hook sets it on every
run; `yako_once` and `yako_default` are empty).

Verdicts: `python3 -I tools/cts/pr-verdict.py --expect stop <log> 217` (S, A)
and `--expect pr <log> 217` (T, B, C).

## Runs

| run | image | case 217 | last step | stop / cause | grow requests | partial renders | HWR | pr-verdict |
|---|---|---|---|---|---|---|---|---|
| S | off, sweep | TIMEOUT | 8: 1220 layers, out of memory at the maximum | cause 2 | 0 (1 notice) | 0 | 0 | OK (`--expect stop`) |
| T | on, sweep | TIMEOUT | 8: 1220 layers, out of memory at the maximum | cause 2 | 0 (1 notice) | 0 | 0 | FAIL (`--expect pr`) |
| A | off, 1220 | TIMEOUT | 1: 1220 layers, out of memory at the maximum | cause 2 | 0 (1 notice) | 0 | 0 | OK (`--expect stop`) |
| B | on, 1220 | TIMEOUT | 1: 1220 layers, out of memory at the maximum | cause 2 | 0 (1 notice) | 0 | 0 | FAIL (`--expect pr`) |
| C | on, 1220, no reserve | FAIL pixels | 1: 1220 layers, asked for a grow | none (`stop 0`) | 1 (refused 1) | 1 | 0 | FAIL (`--expect pr`), class 2 F8 |

The two contract runs are A (knob off) and B (knob on). pr-verdict lines,
verbatim:

```
A (off.log, --expect stop):
pr-verdict: case 217 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause2
pr-verdict: grow requests 0 served 0 unserved 0
pr-verdict: OK

B (on.log, --expect pr):
pr-verdict: case 217 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause2
pr-verdict: grow requests 0 served 0 unserved 0
pr-verdict: class 8 stop cause 2, not a partial-render failure (knob probably off)
pr-verdict: FAIL case 217 is TIMEOUT; partial renders 0 (want >= 1); render stopped, cause 2

S (sweep-off.log, --expect stop):
pr-verdict: case 217 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause2
pr-verdict: grow requests 0 served 0 unserved 0
pr-verdict: OK

T (sweep-on.log, --expect pr):
pr-verdict: case 217 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause2
pr-verdict: grow requests 0 served 0 unserved 0
pr-verdict: class 8 stop cause 2, not a partial-render failure (knob probably off)
pr-verdict: FAIL case 217 is TIMEOUT; partial renders 0 (want >= 1); render stopped, cause 2

C (nr.log, --expect pr):
pr-verdict: case 217 FAIL partial_renders=1 oom=1 hwr=0 stopped=no
pr-verdict: grow requests 1 served 0 unserved 1
pr-verdict: class 2 F8 corrupt frame
pr-verdict: FAIL case 217 is FAIL
```

The case's own lines:

```
A: gl_test: forcedpr: step 1: 1220 layers: out of memory at the list's maximum (no grow request)
   gl_test: forcedpr: FAIL stopped: 1220 layers (1 step): requests 0 (served 0, refused 0), notices 1, stop 2, partial renders 0, GPU recoveries 0
B: (the same two lines)
C: gl_test: forcedpr: step 1: 1220 layers: asked for a grow
   gl_test: forcedpr: FAIL pixels: 1220 layers (1 step): requests 1 (served 0, refused 1), notices 1, stop 0, partial renders 1, GPU recoveries 0
   gl_test: forcedpr: 1001186 of 1048576 pixels differ; first at (48,0) from the bottom left: got 0xff000000 want 0xff076511
```

## What B shows: the at-maximum stop is not gated by the knob

In A and B the firmware sends one `UPDATE_STATS element=2 (out of memory)`
and no `FREELIST_GROW` (the list is at its maximum, 512 of 512; the Kconfig
help of `FL_PROBE_MAX_PAGES` says so). `ccb.rs` `at_maximum` then stops the
render service in the same FWCCB batch: one notice, no grow in the batch, a
list with less than one grow of room. The knob does not enter that rule, so
with the cap the knob-on image cannot reach a partial render: every wait
returns at once and the run ends (`kick 1 did not complete (-110)`, the
20 s hold, the reset). Whether the firmware would have rendered partially
after the notice at the maximum is not measured here: the host stops reading
the FWCCB with the service.

For the knob-on run at the cap to pass, the driver needs (outside this item):
with `partial_render` on, an out-of-memory notice at the maximum is the start
of the firmware's partial render, not a stop; the 10 s kick-timeout stop
(cause 4) stays as the bound. Until then the forced-PR case at the cap is
the knob-off negative in both knob states.

## What C shows: the partial render, the depth kept, the colour lost

With the list free to ask and the answer zero pages (the configuration of
case 175's positive, neg175 image B), the firmware rendered partially once,
the kick completed, no stop, no HWR, and the case read the whole target back:

- **depth:** the 64x64 depth sampling pass ran (`a 1024x1024 depth texture
  is sampled from its depth buffer`, the second HWRT) and no "depth samples
  differ" line was printed: all 4096 window samples equal the expected
  winner's depth within 2 of 255. The expected depths span 13 to 242 of
  255, so neither a cleared nor an unwritten depth buffer matches; the
  depth of the layers tiled before the partial render survived it.
- **colour:** 1001186 of 1048576 pixels differ; the first mismatch (48,0) is
  the clear where layer 0 should be (`forcedpr-expected.py --explain`: "got
  is the clear colour: ... a partial render that reloaded the clear"). About
  47000 pixels are right, more than the 18000-odd the clear should cover:
  the layers after the partial render were drawn over a re-cleared colour
  buffer. That is the frame side the plan has not landed (the PR's colour
  scratch, PR EOT and background load: pr-frame-colour, bgload-pds), recorded
  here, not fixed.

## Files

`sweep-off.log` (S), `sweep-on.log` (T), `off.log` (A), `on.log` (B),
`nr.log` (C): excerpts of the serial console from U-Boot's
`yakogl-netboot: once=` line to the image's reset. Board identity lines
(U-Boot's banner, Linux) are left out; addresses and pointer dumps are
replaced by `<addr>` / `<addresses dropped>`; the `hc dwords` and `hrt[]`
dumps are dropped (the knob's `hrt[32..34]` values are noted in place); the
GPU bring-up lines of T, A, B and C, identical to S's, are replaced by one
line. No other lines were removed. No board address, IP, port or credential
was in the captured text.
