<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rerun at fad0558a, session 4 (G4): T17 UB1 UB2 UB3 UA1 UA2 UA3

commit: fad0558a7def0e5677761608eec56b12e9855d18

Plan #380, step "the Pi 4's remaining GLES3 images" (issue #1261): the last
session of `../2026-11-pi4-gles3-fad0558a-rerun/README.md` ("Later sessions ...
T17, UB1-UB3, UA1-UA3 (session 4)"), built from the same commit. It has a
directory of its own so it does not touch the README of sessions 1 to 3.
"before" is `../2026-11-pi4-gles3-b1b5b845-rows76-90/` in the mustpass view.

| row | image | mustpass | Pass | Fail | NotSupported | other | begun | run | GPU recoveries | hangs | verdict | before (b1b5b845) P / F / N.S. / other / never began | moved |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 80 | T17 | 504 | 432 | 0 | 72 | 0 | 504 | 39.2 s (repeat 39.2 s) | 0 | none | DONE, none failed | 36 / 0 / 216 / 1 / 251 | aborted image runs to its end: 396 Pass, 72 NotSupported (ASTC 8x8 and 8x8 sRGB) |
| 83 | UB1 | 968 | 968 | 0 | 0 | 0 | 968 | 31.6 s (repeat 31.6 s) | 0 | none | DONE, all passed | 968 / 0 / 0 / 0 / 0 | equal |
| 84 | UB2 | 755 | 755 | 0 | 0 | 0 | 757 | 39.9 s (repeat 39.8 s) | 0 | none | DONE, all passed | 754 / 1 / 0 / 0 / 0 | 1 Fail to Pass (`ubo.random.all_per_block_buffers.28`) |
| 85 | UB3 | 632 | 632 | 0 | 0 | 0 | 632 | 52.3 s (repeat 52.2 s) | 0 | none | DONE, all passed | 628 / 4 / 0 / 0 / 0 | 4 Fail to Pass (`single_struct_array` `shared_instance_array_both`, `std140_instance_array_both`, each `per_block_buffer` and `single_buffer`) |
| 86 | UA1 | 759 | 759 | 0 | 0 | 0 | 760 | 31.5 s | 0 | none | DONE, all passed | 758 / 1 / 0 / 0 / 0 | 1 Fail to Pass (`uniform_api.random.83`) |
| 87 | UA2 | 1014 | 1014 | 0 | 0 | 0 | 1014 | 42.9 s | 0 | none | DONE, all passed | 1014 / 0 / 0 / 0 / 0 | equal |
| 88 | UA3 | 408 | 408 | 0 | 0 | 0 | 408 | 14.9 s | 0 | none | DONE, all passed | 408 / 0 / 0 / 0 / 0 | equal |
| | total | 5,040 | 4,968 | 0 | 72 | 0 | | | 0 | | | 4,566 / 6 / 216 / 1 / 251 | |

("begun" is the image's own counter: UB2's and UA1's filters match one or two
cases outside `gles3-mustpass.txt`, all passed.)

## Result

All seven images ran to the end: `complete=1`, `N cases begun, N with a result,
0 without`, 0 Fail, 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU
faults, 0 slow jobs. **T17, which aborted at `b1b5b845` on
`texture.wrap.etc2_eac_rgba8.clamp_clamp_linear_npot` (251 cases never
began), now runs all 504 cases: 432 Pass, 72 NotSupported** (the ASTC 8x8 and
8x8 sRGB cases, `ASTC not supported`, as in T15 and T16); the ETC2/EAC wrap
cases all pass. The six Fails of UB2, UB3 and UA1 at `b1b5b845` pass; nothing
moved from Pass. 4,968 of the 5,040 mustpass cases pass, 72 are NotSupported,
0 fail, 0 are without a result. The end lines:

```
T17  v3d_deqp: build fad0558a DONE, none failed: 504 cases, P 432 F 0 R 0 N 72 other 0   (504 cases in 39168 ms)
UB1  v3d_deqp: build fad0558a DONE, all passed: 968 cases, P 968 F 0 R 0 N 0 other 0     (968 cases in 31567 ms)
UB2  v3d_deqp: build fad0558a DONE, all passed: 757 cases, P 757 F 0 R 0 N 0 other 0     (757 cases in 39890 ms)
UB3  v3d_deqp: build fad0558a DONE, all passed: 632 cases, P 632 F 0 R 0 N 0 other 0     (632 cases in 52277 ms)
UA1  v3d_deqp: build fad0558a DONE, all passed: 760 cases, P 760 F 0 R 0 N 0 other 0     (760 cases in 31530 ms)
UA2  v3d_deqp: build fad0558a DONE, all passed: 1014 cases, P 1014 F 0 R 0 N 0 other 0   (1014 cases in 42851 ms)
UA3  v3d_deqp: build fad0558a DONE, all passed: 408 cases, P 408 F 0 R 0 N 0 other 0     (408 cases in 14930 ms)
```

## What is recorded, and what is not

**All seven images have a console and a tally** (`<LABEL>.console.txt`,
`<LABEL>.tally.txt`, below). In the first run the leased serial console
(1 MiB) dropped T17's, UB1's, UB2's and UB3's console before it could be saved
to disk; only their end-of-run summary was read as it arrived (the table's
first figures and the first four end lines above). Those four images were
then run once more from the same commit (the repeat below), and each console
was saved to disk as soon as its end line arrived, before the next cycle: the
T17, UB1, UB2 and UB3 consoles and tallies in this directory are the repeat's.
The repeat gives the same result as the first run, case for case in the
counters: T17 432 Pass, 72 NotSupported (all 72 `ASTC not supported`, the
`astc_8x8` and `astc_8x8_srgb` groups, 36 each), UB1 968, UB2 757 (755 of them
in the mustpass), UB3 632, all with `complete=1`, `N cases begun, N with a
result, 0 without`, 0 Fail, 0 GPU recoveries, 0 MMU faults, 0 slow jobs, no
hung job. `cycles.txt` has the per-image sha256 and boot ids of both runs.

The repeat's end lines:

```
T17  v3d_deqp: build fad0558a DONE, none failed: 504 cases, P 432 F 0 R 0 N 72 other 0   (504 cases in 39172 ms)
UB1  v3d_deqp: build fad0558a DONE, all passed: 968 cases, P 968 F 0 R 0 N 0 other 0     (968 cases in 31574 ms)
UB2  v3d_deqp: build fad0558a DONE, all passed: 757 cases, P 757 F 0 R 0 N 0 other 0     (757 cases in 39843 ms)
UB3  v3d_deqp: build fad0558a DONE, all passed: 632 cases, P 632 F 0 R 0 N 0 other 0     (632 cases in 52223 ms)
```

## The repeat (T17, UB1, UB2, UB3)

Built on another worker in a worktree of `fad0558a` (`git describe --always
--dirty`: `fad0558a`), the same way as below (VK-GL-CTS 93bca01 with its
externals, the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 in
285 s, then `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh
tools/cts/pi4-gles3-tree.txt <dir> T17 UB1 UB2 UB3`: four `rc=0
build=fad0558a` lines; every console prints `v3d_deqp: build fad0558a`). The
images' sha256 are not the first run's: each image carries the absolute paths
of the build (151 strings in T17 name the stage-1 directory or the Rust
toolchain's sources, from assertion and panic messages), so the same commit
built under other paths gives other bytes. Each repeat image was read back on
the board equal to the sha256 built (`cycles.txt`). Four tryboot cycles, the
steps of "The board" below in the same order. Before the first one the board
again held a leftover `tryboot.txt` of an earlier session (it named a
`yakogl-E.bin` that was not there, with an `hdmi_timings` line; the board had
rebooted since); it was set to the one-newline stock before anything was
staged. After the last cycle: Linux up (boot_id `85816105`), no
`yakogl-*.bin`, `pi4run-*.bin` or `config-linux.txt` under `/boot/firmware`,
`tryboot.txt` one newline (1 byte, read back), `config.txt` unchanged
(sha256 `6ea8eaef...de46`, never written). Consoles were cut by the same rule
(first line after `reboot: Restarting system` to the end line, CRs and NULs
dropped); tallies by `tools/cts/pi4-session-tally.sh` (`T17: 432 pass, 0
fail, 72 notsupported, 0 other`; UB1 968; UB2 755; UB3 632; UA1-UA3's
tallies came out byte for byte as committed).

## The board

Seven tryboot cycles, one image each, Pi 4 (V3D 4.2, Zephyr af17c0c8),
`tools/board.sh pi4`'s steps done by hand in its order (the UART bridge has one
client and the leased serial console held it): the image checks of
`nb_check_image` (AArch64 header, size inside the 256 MiB window, the `board: PM
watchdog resets` marker), prechecks (no leftover `yakogl-*.bin`, no
`config-linux.txt`, `tryboot.txt` the one-newline stock), the image sent
xz-compressed and unpacked on the board into `/boot/firmware/yakogl-<LABEL>.bin`,
`tryboot.txt` written with `pi4_stage`'s lines (no `hdmi_timings`: the
panel's value is not in the environment), both read back (sha256, text) before
`sudo reboot "0 tryboot"`; the end line awaited on the leased serial console;
Linux back (a new boot_id), the staged image removed and `tryboot.txt` put
back to one newline, read back. `board.sh`'s lease file was not written.

Before the first cycle the board held a leftover from an earlier session: a
staged `yakogl-FF.bin` and a `tryboot.txt` that named it (with an
`hdmi_timings` line), written 70 minutes earlier, the board already rebooted
since. Both were removed and `tryboot.txt` set to the one-newline stock before
anything of this session was staged. After the last cycle: Linux up (boot_id
`cce265f9`), no `yakogl-*.bin`, `pi4run-*.bin` or `config-linux.txt` under
`/boot/firmware`, `tryboot.txt` one newline, `config.txt` unchanged (sha256
`6ea8eaef...de46`, never written). The board's clock was not synchronised
when read after a cycle, so no reboot-to-Linux seconds are recorded.

## Method

Built in a worktree of `fad0558a` (`git describe --always --dirty`:
`fad0558a`): VK-GL-CTS 93bca01 with its externals and the gles2 stage 1
(`tools/cts/deps-env.sh --build --board rpi_4b DEQP_WORK`), the gles3 stage 1
of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1), then the seven
images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` from
`tools/cts/pi4-gles3-tree.txt` (7 `rc=0 build=fad0558a` lines; every console
prints `v3d_deqp: build fad0558a`). Consoles of UA1-UA3 were read with
`serial_read`, saved to disk by the tool, cut by a script from the first line
after `reboot: Restarting system` to the end line, CRs and NULs dropped, never
copied by hand; tallies by `tools/cts/pi4-session-tally.sh` (`UA1: 759 pass, 0
fail, 0 notsupported, 0 other`; UA2 1014; UA3 408).

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the
tally reads past them.

- `T17`, `UB1`, `UB2`, `UB3` (the repeat), `UA1`, `UA2`, `UA3` (the first
  run) `.console.txt`: the image's console, from the first line after Linux's
  reboot to the `v3d_deqp: build fad0558a DONE ...` end line.
- the same seven `.tally.txt`: `pi4-session-tally.sh`.
- `cycles.txt`: per image the sha256 of the image (also read back on the
  board), the boot id of the Linux that came back and the end line; the first
  run's seven lines, then the repeat's four.
