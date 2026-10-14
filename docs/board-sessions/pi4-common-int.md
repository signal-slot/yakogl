<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session, common set from the integration image

Status: **GM, SO, SV, CL and GT all ran on the Pi 4 and every verdict is `done`.**
The 96 MiB carve-out is live, glmark2 refract and terrain print no memory
refusal, and Slint, the cluster and gl_test are clean. Two findings (below):
terrain and refract are slower than at baseline, and the glmark2 record words
differ from baseline. The tally ran after the policy allowed it (`--row` form,
the table has no `tree.txt`); the checks below also come from the consoles
(`$CCFARM_DEPS_DIR/scratch/pi4int-rec/<LABEL>/<run>/console.txt`).

HEAD8: `86be8aeb` (workspace snapshot of main, `86be8aebea46f0add9711ba0113b1762c0ce7907`;
board.sh printed `build c9de578`, the note-only commits on top). Main holds the
Pi 4 fixes under test: `rpi_4b.conf` has `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES=100663296`
and `tools/cts/common-tree.txt` has the CL row. The integration ref (`39bb67dc`)
was not used. Board: Raspberry Pi 4, `$BOARD_HOST`, boot tryboot,
1920x720 HDMI timings, UART bridge `$BOARD_UART`. Date 2026-10-14.

## Built images (`common-images.sh`, all rc=0)

Zephyr `af17c0c8aa2a`, board `rpi_4b`, SDK 1.0.1. Built without
`RUSTUP_TOOLCHAIN=nightly` (the prefixed command was refused; it was enough).

| label | sample | sha256 of zephyr.bin | build ids |
|---|---|---|---|
| GM | glmark2 | `97e398044bdebc5b28878e18adc83de66863b747c03afe3a6689e972d9160251` | PVR_SAMPLE_TABLE=c95547a1, PVR_SAMPLE_TREE=86be8ae |
| SO | slint | `7e7187968b520867a4d8c0c4747f6ce971eafa8dcbb3c2f9c26dfa5a86f9b9bd` | same |
| SV | slint, view on | `a5de6b53280a95ce1b0c911dfcddcf537eab3cb10009cae22df0f18aaec78c0f` | same |
| CL | cluster | `aa2426bd51d8059c46a9fd40d50fdd982943b30c172c8a0cde3e7e3b96a8ba7b` | same |
| GT | v3d_gl_test | `52bbf5512aac418905d60e9fa29fcf655baac45ce267eeccc0454df434110051` | V3D_BUILD=86be8ae |

## Commands

`BOARD_HOST` could not be set with a command prefix, so the runs went through a
wrapper outside the repo (`scratch/pi4int-run.sh`: exports
`BOARD_HOST=$BOARD_HOST`, then runs the `board.sh` line below per label).

```sh
tools/cts/common-images.sh --dry-run pi4 tools/cts/common-tree.txt $CCFARM_DEPS_DIR/scratch/pi4int-img
tools/cts/common-images.sh pi4 tools/cts/common-tree.txt $CCFARM_DEPS_DIR/scratch/pi4int-img
tools/board.sh --dry-run pi4 run $IMG/GM/zephyr/zephyr.bin $REC/GM GM-86be8aeb
tools/board.sh pi4 run $IMG/<LABEL>/zephyr/zephyr.bin $REC/<LABEL> <LABEL>-86be8aeb   # GM SO SV CL GT, one at a time
```

Run notes: the first SV attempt was cut off when the worker restarted; the
board was back in Linux, with `yakogl-SV-86be8aeb.bin` and `tryboot.txt` left
in `/boot/firmware`. I removed both (`ssh ... sudo -n rm -f`) and re-ran SV;
only the second SV run (`20261014-0034-SV-...`) is a result. The board was idle
(`uptime` 5 min, no other session) before the re-run.

## Verdict lines (board.sh, verbatim)

```
board.sh pi4 GM-86be8aeb rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00b7,6184003f,00258320 back_after=426s unsupported=0
board.sh pi4 SO-86be8aeb rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:PROBES resets=0 record=594b0601,08420862,00210842,c5920883 back_after=129s
board.sh pi4 SV-86be8aeb rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:PROBES resets=0 record=594b0601,08420862,00210842,c5920883 back_after=127s
board.sh pi4 CL-86be8aeb rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:PROBES resets=0 record=594b0601,00a710c4,10c40086,7b932967 back_after=182s
board.sh pi4 GT-86be8aeb rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:gl_test_record_end resets=0 record=absent back_after=261s
```

Record decode printed by board.sh for GM: `GLMARK2 (18) seq 1: score 183 status 1 (pass)
scenes ran 33 failed 0 unsupported 0 size 800x600 on-screen; first failed: none;
unsupported: none, none; GL errors raised 0, draws that sampled an incomplete texture 0`.

## Tally (common-tally.py pi4 --row LABEL=<rundir> x5, verbatim, run dirs shortened)

```
common-tally: build 96e5437
common-tally: pi4, 5 rows from --row
GM GM-86be8aeb rc=0 verdict=done result=record:GLMARK2 record=594b1201,fc3f00b7,6184003f,00258320; resets (console count) 0; image=97e398044bdebc5b build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=86be8ae
    glmark2 score 183; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none; GL errors raised 0, draws that sampled an incomplete texture 0
SO SO-86be8aeb rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets (console count) 0; image=7e7187968b520867 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=86be8ae
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
SV SV-86be8aeb rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210842,c5920883; resets (console count) 0; image=a5de6b53280a95ce build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=86be8ae
    slint probes pass (status pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592 (RGB565))
CL CL-86be8aeb rc=0 verdict=done result=record:PROBES record=594b0601,00a710c4,10c40086,7b932967; resets (console count) 0; image=aa2426bd51d8059c build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=86be8ae
    cluster probes pass (status pass p0 10c4 p1 00a7 p2 0086 p3 10c4 p4 2967 crc16 7b93 (RGB565)); throughput: 1920x720: 8373 frames in 60019 ms, 139.5 fps, 192.9 Mpx/s; draw 5.5 ms mean, 3.98 ms/Mpx
GT GT-86be8aeb rc=0 verdict=done result=legacy:gl_test_record_end record=absent; resets (console count) 0; image=52bbf5512aac4189 build=V3D_BUILD=86be8ae
    gl_test passed 12 of 12 (0 failed, 0 timed out); failing: none
common-tally: rows 5, with a run 5, without 0; verdict done 5, other 0
```

## The expected checks

- Boot line (quoted): `00:17:47 osal(v3d): ready: carve-out 96 MiB at phys 0x2fdd000, GPU VA 0x181000..0x6100000`.
- Memory refusals: the GM console has **no** `out of GPU memory` and no
  `no GPU memory for a tiled copy` line (grep over the whole console: 0).
  Refract: `yakogl/v3d: tiled copies high-water 19742720 of 25165824 bytes`;
  the carve-out high-water over the run is 69,610,240 of 100,134,912 bytes.
- Terrain not black: no `GL error 0x502` and no incomplete-texture line
  (`glmark2: GL errors raised 0, draws that sampled an incomplete texture 0`).
  That is the strongest evidence the console and record give; they cannot show
  the pixels themselves, so "not black" is **not looked at on the panel**.
- Slint: SO and SV console `slint: check PASS`, `slint: PASS`; 0 GL error or
  incomplete-texture lines on both; SV: `the 1024x608 window is a view into the framebuffer, pitch 5120 (no copy)`.
- Cluster: `probes: 34 of 34 within 12 of the host reference (worst 1)`,
  `cluster: check: PASS`, animation `8373 frames presented in 60019 ms, 139.5 fps`,
  `1920x720: ... 192.9 Mpx/s; draw 5.5 ms mean`.
- gl_test: `v3d_gl_test: 12 of 12 cases passed (2700 ms)`, `slow jobs: 0 ...; MMU faults: 0; recoveries: 0; hung: none`.

## Before / after

"Before" is origin/main `8abeaabb`, `docs/benchmarks-baseline/pi4-common.md` on
the baseline2-pi4 branch (`9c357d77`, not merged into this tree); its cluster
image was built by hand.

| label | before (baseline2-pi4) | after (this run) |
|---|---|---|
| GM | `done`; score 183, 33 ran, 0 failed; terrain 8 `out of GPU memory` + 1 `no GPU memory for a tiled copy`, 16 `GL error 0x502`, 10 fps (107 ms/frame); refract 2 `out of GPU memory`, 5 fps (249 ms/frame); record `594b1201,fc3f00b7,2184003f,03200258` | `done`; score 183, 33 ran, 0 failed; no memory refusal; GL errors 0; terrain 447.6 ms/frame (23 frames in the scene); refract 720.8 ms/frame (14 frames); record `594b1201,fc3f00b7,6184003f,00258320` |
| SO | `done`; `594b0601,08420862,00210842,c5920883`; crc32 `0xfaa46995` | `done`; same record; crc32 `0xfaa46995`, 2 frames, 1024x600 |
| SV | same record and crc32 as SO | `done`; same record `594b0601,08420862,00210842,c5920883` |
| CL | `done`; `594b0601,00a710c4,10c40086,7b932967`; 34/34; 139.5 fps | `done`; same record; 34/34, worst 1; 139.5 fps, 192.9 Mpx/s |
| GT | `done`; 12 of 12 passed (2693 ms) | `done`; 12 of 12 passed (2700 ms) |

## Findings (recorded, nothing changed)

1. **terrain and refract are slower than at baseline**: terrain 10 fps (107 ms)
   to about 2 fps (448 ms), refract 5 fps (249 ms) to about 1.4 fps (721 ms).
   Per frame, `gl` time is 442 ms and 711 ms while v3d render is 2.8 and 4.1 ms,
   so the time is on the CPU side of GL calls (the scenes now do the work that
   was refused or errored before: NPOT mipmaps, 7.6 MB refract textures).
   The score is unchanged at 183. I did not investigate further.
2. **GM record words 3 and 4 differ from baseline** (`6184003f,00258320` against
   `2184003f,03200258`). board.sh's decoder reads both as the same pass result,
   and the record now carries the GL-error and incomplete-texture counts (0, 0),
   so this is most likely a changed packing; not checked in the code.
3. No automatic row-by-row diff: the tally was run without an earlier common run.
