<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Case 175 (refusehang), partial render off and on (parts 1+4): off stops, on completes

Run 2026-10-31 (16:35-16:41 UTC), AM62P (Verdin), item neg175-board of plan
#148 (issue #2/#3).

**Result:**
- **Knob off (A1, A2):** the known negative, as predicted. The refused grow at
  A = 0 ends in the 10 s stop with cause 4, `partial renders 0`, case TIMEOUT.
  Identical to image 0ea4c598's three runs (15 frames, 707 quads).
- **Knob on, parts 1+4 only (B1, B2):** the prediction (still a stop or a
  failure) is **refuted**. The firmware ran one partial render
  (`UPDATE_STATS element=1 (partial renders) adjust=1`), the kick completed,
  and there was no stop, no fence timeout, no HWR, no GPU reset. Case 175
  PASSes in both runs.
- **What that PASS proves, and what it does not:** case 175 passes on "no stop
  and partial renders >= 1" (`gl_test.c`, `case_refusehang`). It reads no
  pixels. With parts 1+4 the 3D_PR is a byte copy of the frame's 3D command
  (no SPM scratch, no PR EOT, no background load), so the frame's contents
  after the partial render are not checked here. Case 217 (forcedpr) reads them
  back. The firmware-side protocol (FENCE_PR, 3D_PR, the HWRT clear) is
  measured working on the board. The frame side is not measured.

## Images

- Source: commit `d1b24389` (on origin/main; the image prints
  `build tree=d1b24389`). Zephyr board `verdin_am62p/am62p54/a53`, sample
  `driver/samples/zephyr/gl_test`. The flow is
  `driver/tools/pvrlab/forcedpr-images.sh`'s, with case 175's config.
- The refusal config is image 0ea4c598's (docs/notes/pvr-render-pool.md, "Case
  175 refusehang, 3 runs"): `NO_RESERVE`, every grow knob off, the list not at
  its maximum (`FL_PROBE_MAX_PAGES=0`). `tools/cts/pr-configs/` is not on
  origin/main, so no fragments were used.
- Kconfig (both images; the `.config` files differ in one line only):
  - `CONFIG_GL_TEST_CASES="175"`
  - `CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y`
  - `CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`
  - `CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`
  - resulting: `FL_PROBE_MAX_PAGES=0`, `FL_PROBE_SERVE_MAX=0`,
    `FL_GROW_ON_DEMAND` unset, `FL_READY_PAGES` unset,
    `YAKOGL_AM62P_WDT_SECONDS=150`
  - A: `# CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER is not set`
    (`-DCONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n`)
  - B: `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y`
- zephyr.bin sha256:
  - A `a1f913e1c830a642bc51a0c2883ebfba4cd2ee18f327922d46d1bf5a810bb820` (3124528 B)
  - B `d454bab26f7b41e2e70d4827ae28983bd35c863157eec11ae3057aa850e2e979` (3124536 B)
- Watchdog: the RTI is self-armed with a 60 s expiry and fed for 150 s
  (`rti_arm: ... fed for 150 s` in each log), so a hang ends in a reset. No run
  needed it. Each image reset the SoC itself (`sample done (N): resetting the
  SoC`), and Linux came back within about 60 s.

## Commands

Build (from the west workspace, `ZEPHYR_MODULES` = `tools/cts/zephyr-modules.sh
$ZEPHYR_WS`'s list, written out by hand because west came from the workspace's
own venv; the PowerVR firmware header was copied from the deps cache to
`driver/src/fw/pvr_fw_blob.h`, untracked):

```
west build -p always -b verdin_am62p/am62p54/a53 -d <out>/{off,on} driver/samples/zephyr/gl_test -- \
    -DZEPHYR_MODULES=<list> -DYAKOGL_DIR=<repo> '-DCONFIG_GL_TEST_CASES="175"' \
    -DCONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE=y -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y \
    -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n -DCONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER={n,y}
```

Deploy: the `uboot-mmc` one-shot, done by hand as `tools/board.sh` does it
(the task machine has no TCP serial bridge for `board.sh`'s capture). The image
was written to `/var/yakogl/once-neg175{A,B}.bin`, read back by sha256 with the
page cache dropped, and armed with `fw_setenv -s` (`yako_once=neg175<run>`,
`yako_src=mmc`, `yako_file=<U-Boot path>`), then `systemctl reboot`. The
console was read through the leased serial console. One GL client ran at a
time. Afterwards the staged images were removed, `yako_file` was put back to
its previous value, and `yako_once` and `yako_default` are empty.

Verdicts: `python3 -I tools/cts/pr-verdict.py --expect stop <log> 175` (A) and
`--expect pr <log> 175` (B).

## Runs

| run | image | case 175 | stop / cause | HWR | partial renders | pr-verdict |
|---|---|---|---|---|---|---|
| A1 | A (off) | TIMEOUT | `render: stopped: a kick that did not complete in the stop's time`, cause 4 | 0 | 0 | OK (`--expect stop`) |
| A2 | A (off) | TIMEOUT | same, cause 4 | 0 | 0 | OK (`--expect stop`) |
| B1 | B (on) | PASS | none (`stop 0`) | 0 | 1 | OK (`--expect pr`) |
| B2 | B (on) | PASS | none (`stop 0`) | 0 | 1 | OK (`--expect pr`) |

pr-verdict lines:

```
A1: pr-verdict: case 175 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause4
    pr-verdict: grow requests 1 served 0 unserved 1
    pr-verdict: OK
A2: pr-verdict: case 175 TIMEOUT partial_renders=0 oom=1 hwr=0 stopped=cause4
    pr-verdict: grow requests 1 served 0 unserved 1
    pr-verdict: OK
B1: pr-verdict: case 175 PASS partial_renders=1 oom=1 hwr=0 stopped=no
    pr-verdict: grow requests 1 served 0 unserved 1
    pr-verdict: OK
B2: pr-verdict: case 175 PASS partial_renders=1 oom=1 hwr=0 stopped=no
    pr-verdict: grow requests 1 served 0 unserved 1
    pr-verdict: OK
```

(B1 with `--expect stop` gives `pr-verdict: FAIL no 'render: stopped' line
(want cause 2 or 4)`: the predicted stop did not happen.)

Lines, A (both runs the same):

```
W: FWCCB: FREELIST_GROW id=1 -> no grow (reserve 0, pool 0/0, 512/4096 pages)
I: FWCCB: UPDATE_STATS element=2 (out of memory) adjust=1 (seen 1)
I: render: at grow request 1 (...): last submitted 15, last fragment done 14; 1 kicks in flight: 0 with TA done and 3D pending, 1 with TA pending
E: render: stopped: a kick that did not complete in the stop's time (list 0, 0 KiB); no kick is waited for or taken from now on
I: render freelists: 1 grow requests, 0 served, 1 unserved, 0 KiB taken on demand; GPU recoveries 0 (HWR counter 0)
I: render freelists: ... firmware UPDATE_STATS: partial renders 0, out of memory 1, TA stores 0, 3D stores 0, CDM stores 0, TDM stores 0, other 0
gl_test: refusehang: 15 frames, the last of 707 quads asked for a grow: requests 1 (served 0, refused 1), notices 1, re-posts 15, stop 4, partial renders 0
gl_test: case 175 refusehang: TIMEOUT
```

Lines, B (both runs the same):

```
W: FWCCB: FREELIST_GROW id=1 -> no grow (reserve 0, pool 0/0, 512/4096 pages)
I: FWCCB: UPDATE_STATS element=2 (out of memory) adjust=1 (seen 1)
I: render: at grow request 1 (...): last submitted 15, last fragment done 14; 1 kicks in flight: 0 with TA done and 3D pending, 1 with TA pending
I: FWCCB: UPDATE_STATS element=1 (partial renders) adjust=1 (seen 1)
gl_test: refusehang: 15 frames, the last of 707 quads asked for a grow: requests 1 (served 0, refused 1), notices 1, re-posts 15, stop 0, partial renders 1
gl_test: case 175 refusehang: PASS
I: render freelists: 1 grow requests, 0 served, 1 unserved, 0 KiB taken on demand; GPU recoveries 0 (HWR counter 0)
I: render freelists: ... firmware UPDATE_STATS: partial renders 1, out of memory 1, TA stores 0, 3D stores 0, CDM stores 0, TDM stores 0, other 0
```

In B the HWRT dump's words 32-34 are non-zero (for example `hrt[32]: 400e9e80
400e9f00 00000003`), where A has zeros. These are words only the knob writes into the HWRT data.
They were not decoded in this session.

## Files

`A1.log`, `A2.log`, `B1.log`, `B2.log` are excerpts of the serial console, from
U-Boot's `yakogl-netboot: once=` line to the image's reset. Board identity
lines (U-Boot's banner, Linux) were left out. Addresses and pointer dumps were
replaced by `<addr>` / `<addresses dropped>`, and the `hc dwords` / `hrt[]`
dumps were dropped. No other lines were removed. No board address, IP, port or
credential was in the captured text.
