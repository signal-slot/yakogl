<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES2 regression rows (D FB1 T1 T3 TF) at b1b5b845

commit: b1b5b8456ec69247c64d1bd07e42461266dc5136

The ES 2.0 regression rows of `docs/es3/board-recipe-pi4.md` section 8, run
on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot) at the commit of the
one-commit Pi 4 dEQP-GLES3 sessions, item pi4-triage of plan #320 (issue
#1261). Rows 1, 4, 19, 21 and 24 (counted from 0) of
`tools/cts/pi4-full-tree.txt`, the gles2 stage 1 as `DEQP_WORK`, no
`DEQP_MODULE`, `BOARD_MUSTPASS` unset (the GLES2 list).
Per-image files, relative to this directory: `<LABEL>.console.txt`,
`<LABEL>.tally.txt`. Counts are the mustpass view (`gles2-mustpass.txt`) of
the tally's TOTAL line.

| row | image | mustpass | Pass | Fail | NotSupported | other | begun | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | D | 292 | 292 | 0 | 0 | 0 | 305 | 12.1 s | 0 | none | DONE, all passed |
| 4 | FB1 | 390 | 390 | 0 | 0 | 0 | 392 | 11.7 s | 0 | none | DONE, all passed |
| 19 | T1 | 181 | 181 | 0 | 0 | 0 | 224 | 27.1 s | 0 | none | DONE, all passed |
| 21 | T3 | 304 | 304 | 0 | 0 | 0 | 304 | 43.1 s | 0 | none | DONE, all passed |
| 24 | TF | 17 | 17 | 0 | 0 | 0 | 20 | 2.0 s | 0 | none | DONE, all passed |
| | total | 1,184 | 1,184 | 0 | 0 | 0 | 1,245 | | 0 | | |

("begun" is the image's own counter: its filter also matches cases outside
`gles2-mustpass.txt`; every one of the 1,245 begun cases passed, the image's
own `done:` lines read `0 failed, 0 not supported, 0 warnings, complete=1`.)

## Result

All five images ran to the end (`complete=1`, `N cases begun, N with a
result, 0 without`), and every mustpass case in their filters passed: 1,184 of
1,184, 0 Fail, 0 NotSupported, 0 without a result. `pi4-console-scan.py` over
the five consoles: `5 image run(s): 5 finished, 0 without a finish line;
recoveries 0, job timeouts 0, hung 0, MMU faults 0, resets 0, faults 0`; every
image printed `slow jobs: 0 ...; MMU faults: 0; recoveries: 0; hung: none`.
The longest run is T3's 43.1 s, far inside the 210 s record deadline.

**Regressions against `tools/cts/RESULTS.md`: none.** RESULTS.md's Pi 4 ES 2.0
row ("Pi 4, 29 rows", re-run at `46f93767`, 2026-10-24) reads 17,126 pass,
0 fail, 34 NotSupported over the whole tree, and its 34 NotSupported
(`clip_control`, `negative_api.texture.compressedtexsubimage2d_*`,
`multisampled_render_to_texture.readpixels`, `multisample.*`) lie outside
these five images' filters, so the expected outcome of each row is all of
its mustpass cases Pass (recipe section 8: "each row's tally equal to the
whole tree ..., with T3 and TF as on `1cf946a9` (all pass), and nothing moved
from Pass"). That is what each row reads at `b1b5b845`: nothing moved from
Pass, no Fail, no NotSupported, no recovery.

## The verdict lines `tools/board.sh` printed

```
board.sh pi4 D rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=302s
board.sh pi4 FB1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=307s
board.sh pi4 T1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=299s
board.sh pi4 T3 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=305s
board.sh pi4 TF rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=306s
```

each followed by `=== <LABEL> rc=0`, then `TREE-EXIT`; `tools/board.sh pi4
tally records/pi4/es2-b1b5b845` (rc 0) printed the same five lines and

```
== mustpass: 1184 cases in these records' case filters (15981 outside them, not run, left out); 1184 with a result (1245 case lines seen)
group                           pass   fail   n.s.     QW  other   none
fbo                              390      0      0      0      0      0
shaders                          309      0      0      0      0      0
texture                          485      0      0      0      0      0
TOTAL                           1184      0      0      0      0      0
```

with an empty `== not Pass, by name`.

## The cycles

Run 2026-11-14 02:17-02:44 UTC, one image at a time, each staged only after
Linux was back and the previous one's files were gone. Per image (from
`board.sh`'s `meta.txt` and the console):

| image | zephyr.bin sha256 | tryboot (UTC) | Linux back after | clean-up | `cases in` line |
|---|---|---|---|---|---|
| D | `480623678df5c6ca867d09ef3f31ea6b352805258a146d573a43d7bd0c3b2652` | 02:17:19 | 302 s | staged image removed, tryboot.txt put back (read back) | 305 cases in 12093 ms, record at 12 s after boot |
| FB1 | `c43489671b34df93019218467b9a6c2566d3cc8edd6441587e5aa5092644a7aa` | 02:23:13 | 307 s | the same | 392 cases in 11714 ms, record at 11 s after boot |
| T1 | `955fa153eb33003f6b73d51239602f21737d9185173c2faeb7c272fd3f8f3e71` | 02:28:35 | 299 s | the same | 224 cases in 27060 ms, record at 27 s after boot |
| T3 | `f622549600508690febda4100b973f0ce3ad72c5e1906466f17673dfb3c8a3a8` | 02:33:49 | 305 s | the same | 304 cases in 43102 ms, record at 43 s after boot |
| TF | `c720915e447c29f166725b1b1b5c26fd237e8621270be4d0a47a753fc38688e4` | 02:39:13 | 306 s | the same | 20 cases in 2013 ms, record at 2 s after boot |

Every image printed `board: watchdog armed` before Zephyr started and
`v3d_deqp: build b1b5b845` at boot; the PM watchdog reset the board at about
250 s and Linux came back on its normal configuration. After the session
`/boot/firmware` held no staged image and `tryboot.txt` as before the session
(a single newline); `config.txt` was untouched throughout.

## Method

Everything was built from a detached worktree of `b1b5b845` outside the
repository: the gles2 stage 1 through `tools/cts/deps-env.sh --build --board
rpi_4b DEQP_WORK` (VK-GL-CTS 93bca01, Zephyr SDK 1.0.1; 418 ninja steps,
rc 0), then the recipe's first command, `DEQP_WORK=<gles2 stage 1>
tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt
build/deqp/pi4-b1b5b845 D FB1 T1 T3 TF`: five `<LABEL> rc=0 sha256=...
build=b1b5b845` lines (the sha256 above). The runs used the recipe's second
and third commands as written, `TREE_IMAGE_DIR=build/deqp/pi4-b1b5b845
TREE_ROWS=1,4,19,21,24 tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt
records/pi4/es2-b1b5b845` and `tools/board.sh pi4 tally
records/pi4/es2-b1b5b845`: tryboot, the runner's own staging, read-back,
reboot, end-line wait, return check and clean-up, `BOARD_HOST` and
`BOARD_UART` from the environment. D ran alone first (`TREE_ROWS=1`), then
`TREE_ROWS=4,19,21,24`.

Two things differ from a session at a desk, both outside the board's side:

- The console. The Pi 4's UART bridge takes one client at a time and the
  leased serial console (the MCP server connected forwards) held it
  (`tools/am62p_netboot_uart.py probe` answered `busy`). `BOARD_UART` named a
  one-client relay on the runner host instead, and the console bytes were
  read with the console's `serial_read` (offset-addressed, contiguous from
  the first tryboot to TF's end line), saved to disk by the tool and appended
  to the relay's stream by a script, never copied by hand. Each image's bytes
  reached `board.sh`'s capture 115-301 s after its tryboot, inside its 420 s
  bound, so the capture's time stamps are the relay's arrival times, not the
  board's (the consoles here carry none). Each reply that held a reboot
  decoded to 9 bytes fewer than the console's offsets counted (in the
  reboot and firmware stretch; the reply that held only dEQP output decoded
  byte for byte); every case line is whole: the tallies find every result
  line the images' own summaries count.
- The session check. `who` on the board lists non-pty ssh sessions too, and
  `board.sh`'s own `ssh <board> who` came from the lease forwarder's address,
  which is not one of the runner host's own, so `common_prechecks` refused
  with rc 3 (the pi4-es2reg attempt's failure). Before each run `who` showed
  only that forwarder session (`SSH_CLIENT` of the session itself) and the
  board's own seat0/tty1 logins, and the runner was told the forwarder
  address is its own (an `ip` on `PATH` that adds it to `ip -o addr`);
  `board.sh` itself was not changed. One earlier start stopped before any
  board command (`BOARD_UART` unset in that shell); nothing was staged or
  booted and its scratch record is not part of this one.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the
tally reads past them.

- `<LABEL>.console.txt`: `board.sh`'s (redacted) `console.txt` of the image's
  run from the firmware's first line (the line after Linux's `reboot:
  Restarting system`) to the `v3d_deqp: build b1b5b845 ...` end line, CRs and
  NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `deqp-uart-tally.py --label <LABEL> --mustpass
  ../../gles2-mustpass.txt <LABEL>.console.txt`, run from inside this
  directory on relative names.
