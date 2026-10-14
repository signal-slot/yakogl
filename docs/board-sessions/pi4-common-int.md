<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session, common set from the integration image (GM SO SV CL GT)

HEAD8: `d64e995b` (`d64e995ba5e9e6866af070bb75273bcbd10ca1ee`). That is the
integration tip `79c73ea2` plus three commits that touch only this note
(`git diff --stat 79c73ea2 d64e995b`: this file, nothing else), so the
images are the `79c73ea2` code. Board: Raspberry Pi 4 `elegance-1`, resource
pi4-elegance. Run on 2026-10-14, 22:10-22:32 JST.

**Result: all five rows `verdict=done`, `resets=0`.** glmark2 has score 183,
33 scenes, 0 failed, 0 GL errors and 0 incomplete-texture draws. The boot line
says `carve-out 96 MiB`, and refract and terrain print no memory refusal.
Slint and the cluster pass their probes with no GL error line. gl_test passes
12 of 12. Findings (section "Differences from the baseline"): the Slint
fingerprint changed, terrain and refract are now 3 and 2 fps where they were
10 and 5, and a bring-up uniform dump prints during terrain.

## Commands

The bridge (`usb1`, `100.124.238.76:35931`) had answered
`usb: cannot open the interface: open failed (esp_err 261)` in earlier passes
of this item. At 22:02 JST it still did. At 22:09 a 15 s capture got
`MARK-pi4ci` back after an ssh `echo MARK-pi4ci > /dev/serial0` on the board,
so the bridge could reach the Debug Probe again.

```sh
ssh -o BatchMode=yes signal-slot@elegance-1 true            # rc 0; board idle (load 0.06, no yakogl-*.bin, tryboot.txt 1 byte)
RUSTUP_TOOLCHAIN=nightly tools/cts/common-images.sh pi4 tools/cts/common-tree.txt build/pi4ci/img
# BOARD_HOST=signal-slot@elegance-1 BOARD_UART=100.124.238.76:35931
# BOARD_HDMI_TIMINGS='1920 1 50 44 50 720 1 10 20 50 0 0 0 60 0 99070000 1'
#   (exported by an untracked wrapper, removed after the run)
tools/board.sh --dry-run pi4 run build/pi4ci/img/GM/zephyr/zephyr.bin build/pi4ci/rec-dry/GM GM-d64e995b
for l in GM SO SV CL GT; do
  tools/board.sh pi4 run build/pi4ci/img/$l/zephyr/zephyr.bin build/pi4ci/rec/$l $l-d64e995b
done
python3 tools/cts/common-tally.py pi4 --row GM=<rec>/GM/<run> --row SO=… --row SV=… --row CL=… --row GT=…
```

The environment's `BOARD_HDMI_TIMINGS` on this worker was `1920x720`, which is
not an `hdmi_timings` line. The run used the full line from
`tools/cts/README.md` (above) instead.

`common-images: build d64e995b (d64e995ba5e9e6866af070bb75273bcbd10ca1ee) board pi4`:

```
GM rc=0 sha256=641004450e21ba7a0a8a9053d622f36f49ab638aba04786732abfd84d5fb0b79 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
SO rc=0 sha256=f3435760645663f9ea8130b5d93733c97a3f10d0bd594a137cd2793df4c780ad build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
SV rc=0 sha256=b253cb6b945dd01b62146c7f972b01d89212f8051bc156c1baa95201c726ff35 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
CL rc=0 sha256=6d5a7c1d216f21e05bc9c02a5274410ddc6ba5ae30d5e1fd78f1b258c54b1f86 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
GT rc=0 sha256=3510cfdc89990b3c48bb22910d4938eeef502852965e09a68dbee9cf96f8a3f6 build=V3D_BUILD=d64e995b
common-images: 0 row(s) not built
```

## Verdict lines (verbatim)

```
board.sh pi4 GM-d64e995b rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00b7,6184003f,00258320 back_after=434s unsupported=0
board.sh pi4 SO-d64e995b rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:PROBES resets=0 record=594b0601,08420862,00210862,312b0883 back_after=138s
board.sh pi4 SV-d64e995b rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:PROBES resets=0 record=594b0601,08420862,00210862,312b0883 back_after=143s
board.sh pi4 CL-d64e995b rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:PROBES resets=0 record=594b0601,00a710c4,10c40086,7b932967 back_after=183s
board.sh pi4 GT-d64e995b rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:gl_test_record_end resets=0 record=absent back_after=265s
```

Run directories (ignored, not committed): `20261014-2210-GM-d64e995b-64100445`,
`20261014-2218-SO-d64e995b-f3435760`, `20261014-2221-SV-d64e995b-b253cb6b`,
`20261014-2224-CL-d64e995b-6d5a7c1d`, `20261014-2227-GT-d64e995b-3510cfdc`.
Each console holds its own image's boot (`Loaded 'yakogl-<LABEL>-d64e995b.bin'`),
so the stale-buffer problem of the baseline's first GT run did not recur.

## Tally (verbatim)

```
common-tally: build d64e995b
common-tally: pi4, 5 rows from --row
GM GM-d64e995b rc=0 verdict=done result=record:GLMARK2 record=594b1201,fc3f00b7,6184003f,00258320; resets (console count) 0; image=641004450e21ba7a build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
    glmark2 score 183; scenes ran 33, failed 0, unsupported 0; status 1 (pass); first failed: none; unsupported: none, none; GL errors raised 0, draws that sampled an incomplete texture 0
SO SO-d64e995b rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210862,312b0883; resets (console count) 0; image=f3435760645663f9 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
    slint probes pass (status pass p0 0862 p1 0842 p2 0862 p3 0021 p4 0883 crc16 312b (RGB565))
SV SV-d64e995b rc=0 verdict=done result=record:PROBES record=594b0601,08420862,00210862,312b0883; resets (console count) 0; image=b253cb6b945dd01b build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
    slint probes pass (status pass p0 0862 p1 0842 p2 0862 p3 0021 p4 0883 crc16 312b (RGB565))
CL CL-d64e995b rc=0 verdict=done result=record:PROBES record=594b0601,00a710c4,10c40086,7b932967; resets (console count) 0; image=6d5a7c1d216f21e0 build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=d64e995b
    cluster probes pass (status pass p0 10c4 p1 00a7 p2 0086 p3 10c4 p4 2967 crc16 7b93 (RGB565)); throughput: 1920x720: 8361 frames in 60015 ms, 139.3 fps, 192.6 Mpx/s; draw 5.5 ms mean, 3.98 ms/Mpx
GT GT-d64e995b rc=0 verdict=done result=legacy:gl_test_record_end record=absent; resets (console count) 0; image=3510cfdc89990b3c build=V3D_BUILD=d64e995b
    gl_test passed 12 of 12 (0 failed, 0 timed out); failing: none
common-tally: rows 5, with a run 5, without 0; verdict done 5, other 0
```

(The `run <dir>` line under each row is left out.)

## The expected lines

| expectation | state |
|---|---|
| boot line `carve-out 96 MiB` (GM) | **present**: `22:11:16 osal(v3d): ready: carve-out 96 MiB at phys 0x2fdf000, GPU VA 0x181000..0x6100000` |
| refract: no memory refusal | **met**: 0 `out of GPU memory`, 0 `no GPU memory for a tiled copy` in the whole console. Refract prints `yakogl/v3d: tiled copies high-water 19742720 of 25165824 bytes`; the carve-out high-water after refract is `69610240 … of 100134912 bytes` |
| terrain: no memory refusal | **met**: none in the console. After terrain: `carve-out used 17833984, high-water 61725440, of 100134912 bytes` |
| terrain not black | **not shown by the record.** Indirect evidence only: 0 `GL error 0x502 in glGenerateMipmap` (the baseline had 16), 0 incomplete-texture lines (the baseline had 1), and the record says `draws that sampled an incomplete texture 0`. Nobody looked at the panel. glmark2-tally-scenes or a frame readback would settle it |
| Slint SO/SV and cluster CL: no GL errors | **met**: 0 `GL error` lines in SO, SV, CL; `slint: check PASS (tolerance 1 per channel)`, `slint: PASS` (SO, SV); `cluster: check: PASS` (CL) |
| GT as at baseline2-pi4 | **met**: `v3d_gl_test: 12 of 12 cases passed (2699 ms)`; `slow jobs: 0 …; MMU faults: 0; recoveries: 0; hung: none` |

## Before / after

"Before" is baseline2-pi4 (`docs/benchmarks-baseline/pi4-common.md` on
`ccfarm/01a115b0-f176-71cb-961d-9d0aafadb5b5`, origin/main `8abeaabb`,
2026-10-07; not merged into integration).

| benchmark | before (8abeaabb) | after (d64e995b) |
|---|---|---|
| GM glmark2 | score 183, 33 scenes, 0 failed, 0 unsupported; record `594b1201,fc3f00b7,2184003f,03200258` | score 183, 33 scenes, 0 failed, 0 unsupported, GL errors raised 0, incomplete draws 0; record `594b1201,fc3f00b7,6184003f,00258320` |
| GM terrain | 10 fps (107.2 ms); 8 `out of GPU memory` + 1 `no GPU memory for a tiled copy`; 16 x `0x502` in glGenerateMipmap; 1 incomplete-texture line | **3 fps** (439.7 ms); no refusal, no `0x502`, no incomplete texture |
| GM refract | 5 fps (248.7 ms); 2 `out of GPU memory` (7684096 B) | **2 fps** (711.7 ms); no refusal; tiled copies high-water 19742720 of 25165824 |
| GM shadow | (not listed) | 5 fps (233.2 ms); one `stale tiled copy of 7684096 bytes given back` line |
| SO slint, view off | PROBES pass p0 0862 p1 0842 p2 0842 p3 0021 p4 0883 crc16 c592; fingerprint 0xfaa46995, 2 frames, 1024x600 | PROBES pass p0 0862 p1 0842 **p2 0862** p3 0021 p4 0883 **crc16 312b**; fingerprint **0xe7113010**, 2 frames, 1024x600 (record `594b0f08 e7113010 00000002 04000258`) |
| SV slint, view on | the same as SO | the same as SO (`the 1024x608 window is a view into the framebuffer, pitch 5120 (no copy)`) |
| CL cluster 1920x720 | 34 of 34 probes, 139.5 fps, 192.9 Mpx/s, draw 5.2 ms mean; fingerprint 0xfdf7d08f; built by hand | PROBES `594b0601,00a710c4,10c40086,7b932967` (identical), `check: PASS`, 139.3 fps, 192.6 Mpx/s, draw 5.5 ms mean; fingerprint 0xfdf7d08f (identical); built from the CL row |
| GT v3d_gl_test | 12 of 12 (2693 ms), 0 MMU faults, 0 recoveries | 12 of 12 (2699 ms), 0 MMU faults, 0 recoveries |

## Differences from the baseline (findings; no code or expectation changed)

1. **Slint image changed, still within tolerance.** SO and SV give probe p2
   `0862` (it was `0842`), crc16 `312b` (it was `c592`), and window crc32
   `0xe7113010` (it was `0xfaa46995`). The check passes at tolerance 1 per
   channel, so the difference is one RGB565 step on one probe. SO and SV agree
   with each other, so the cause is the shared draw path, not the scan-out
   view. Candidates: npot (NPOT textures now sample with their mip/wrap modes),
   femtovg-pin. Not bisected. If the new image is right, the Slint reference
   fingerprint in the docs needs updating; that is a docs item.
2. **terrain 10 -> 3 fps, refract 5 -> 2 fps, overall score the same (183).**
   Before, both scenes refused allocations and drew with missing textures or
   skipped draws. Now they do the full work. The cost is CPU-side: the
   `glmark2-frame` lines give `gl 434146` us per frame for terrain against a
   V3D `total 2856` us, and `gl 702418` against `total 5142` for refract. The
   CPU time therefore goes inside the GL calls (tiled copies or mipmap
   generation), not into GPU work. This is the v3d-rt-sample /
   pi4-refract-draw area; not investigated here.
3. **Bring-up uniform dump in terrain.** 92 lines of
   `  fs uniform[i] = 0x…  int …  float …` (23 frames x 4) come from
   `dump_small_fs_block` in `src/gl/draw.rs` (fires for a fragment block of at
   most 8 dwords in a shader over 900 USC dwords, budget `FS_DUMP_BUDGET`). It
   is console noise on the 115200 baud UART and a small part of terrain's frame
   time. Also four `yakogl: program 8 has no uniform 'fog…'` lines at terrain
   start (glmark2 sets uniforms the shader optimised away; harmless).

## Board state afterwards

At 22:34 the board was back in Linux (up 2 min, load 0.23), with no
`/boot/firmware/yakogl-*` file left and `tryboot.txt` at 1 byte, as before
the run.
