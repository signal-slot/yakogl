// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Every number the website shows, in one place. The pages render from this
// object (site.js); a new measurement changes only this file.
//
// Below the assignment the object is strict JSON (tools/website-check.sh
// parses it with Python's json module): double quotes, no comments, no
// trailing commas.
//
// figures.<id>:
//   label     what was measured
//   board     "am62p" | "imx8mp" | "pi4" | null
//   value     a number, a string, or for a dEQP tree an object
//             {pass, fail, notSupported, warnings, total}
//   unit      what the value counts
//   commit    the commit the image was built from (null: not ours, or not recorded)
//   onMain    true if that commit is an ancestor of main; false for the
//             development history before publication or an integration branch
//   date      YYYY-MM-DD (or YYYY-MM / YYYY for a public reference); null if unknown
//   source    repository path of the record the figure comes from
//   section   where in that file
//   status    "pass" | "warn" | "fail" | "info"
//   note      what a reader must know to read the figure honestly
//   unverified  present and true when part of the provenance could not be checked
window.YAKOGL_DATA = {
  "repo": "https://github.com/signal-slot/yakogl",
  "checkedAgainst": { "commit": "1cf946a9", "date": "2026-10-11", "source": "docs/HANDOFF.md" },
  "boards": [
    {
      "id": "am62p",
      "name": "Toradex Verdin AM62P",
      "short": "AM62P",
      "soc": "TI AM62P",
      "gpu": "Imagination PowerVR BXS-4-64",
      "feature": "gpu-powervr",
      "zephyrBoard": "verdin_am62p/am62p54/a53",
      "glTestSample": "gl_test",
      "panel": "1024x600",
      "storage": "eMMC (U-Boot)"
    },
    {
      "id": "imx8mp",
      "name": "Toradex Verdin iMX8M Plus",
      "short": "i.MX8MP",
      "soc": "NXP i.MX 8M Plus",
      "gpu": "Vivante GC7000UL",
      "feature": "gpu-vivante",
      "zephyrBoard": "verdin_imx8mp/mimx8ml8/a53",
      "glTestSample": "gc_gl_test",
      "panel": "1280x800",
      "storage": "eMMC (U-Boot)"
    },
    {
      "id": "pi4",
      "name": "Raspberry Pi 4",
      "short": "Pi 4",
      "soc": "Broadcom BCM2711",
      "gpu": "Broadcom V3D 4.2",
      "feature": "gpu-v3d",
      "zephyrBoard": "rpi_4b",
      "glTestSample": "v3d_gl_test",
      "panel": "1920x720",
      "storage": "SD card (firmware)"
    }
  ],
  "figures": {
    "deqp2.am62p": {
      "label": "dEQP-GLES2, whole tree",
      "board": "am62p",
      "value": { "pass": 17070, "fail": 54, "notSupported": 34, "warnings": 7, "total": 17165 },
      "unit": "cases of the mustpass list",
      "commit": "32778b57",
      "onMain": true,
      "date": "2026-10-10",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: the whole dEQP-GLES2 tree on origin/main 32778b57",
      "status": "fail",
      "note": "The 54 are 9 shaders.invariance cases (fixed in e17fb6a7; image 1 alone on that commit passes, row below) and 45 texture.vertex.2d cases (a 1 MiB draw arena refused dEQP's grid; fixed on the host, not yet confirmed on the board). 34 not supported include the 16 multisample cases, which run on the 4x image (row below). 7 warnings: 5 line-interpolation quality warnings and 2 flush_finish timing warnings. Compared with the 2026-10-02 tree (0 fail) these are regressions."
    },
    "deqp2.imx8mp": {
      "label": "dEQP-GLES2, whole tree",
      "board": "imx8mp",
      "value": { "pass": "17,112–17,116", "fail": 0, "notSupported": "30–34", "warnings": 19, "total": 17165 },
      "unit": "cases of the mustpass list",
      "commit": "32778b57",
      "onMain": true,
      "date": "2026-10-10",
      "source": "tools/cts/RESULTS.md",
      "section": "i.MX8MP: the whole dEQP-GLES2 tree on Zephyr at origin/main 32778b57",
      "status": "pass",
      "note": "17,371 cases began (17,318 pass, 0 fail, 34 not supported, 19 quality warnings), every selector identical to the 2026-10-01 tree. The range: the run covered 206 cases outside the mustpass list, four of them in the not-supported group. The 19 warnings appear on NXP's own driver too."
    },
    "deqp2.am62p.ms4": {
      "label": "dEQP-GLES2, multisample on the 4x image",
      "board": "am62p",
      "value": { "pass": 14, "fail": 2 },
      "unit": "cases (the 16 multisample.* cases)",
      "commit": "32778b57",
      "onMain": true,
      "date": "2026-10-10",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: multisample on the ms4 image at 32778b57",
      "status": "fail",
      "note": "Two runs, identical. constancy_alpha_to_coverage and constancy_both_inverted fail; the 2026-10-02 tree had 16 / 16. Alpha to coverage goes through the fragment shader's sample_kill variant; the cause is not found and no issue exists yet."
    },
    "deqp2.am62p.shaders": {
      "label": "dEQP-GLES2, image 1 (shaders) after the invariance fix",
      "board": "am62p",
      "value": { "pass": 1379, "fail": 0 },
      "unit": "cases",
      "commit": "e17fb6a7",
      "onMain": true,
      "date": "2026-10-10",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: image 1 on e17fb6a7",
      "status": "pass",
      "note": "Image 1 alone, not a whole tree. The 9 shaders.invariance common_subexpression cases pass."
    },
    "gltest.am62p": {
      "label": "gl_test regression set",
      "board": "am62p",
      "value": "48 / 48",
      "unit": "cases passed, 0 timed out, 0 GPU resets",
      "commit": "32778b57",
      "onMain": true,
      "date": "2026-10-10",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: gl_test default set on 32778b57",
      "status": "pass",
      "note": "Compiler-only image (precompiled hits 0, compiler runs 197)."
    },
    "gltest.imx8mp": {
      "label": "gc_gl_test regression set",
      "board": "imx8mp",
      "value": "45–46 / 48",
      "unit": "cases passed in three runs (45, 46, 46)",
      "commit": "32778b57",
      "onMain": true,
      "date": "2026-10-10",
      "source": "tools/cts/RESULTS.md",
      "section": "i.MX8MP: gc_gl_test on 32778b57, three runs",
      "status": "warn",
      "note": "Run 1: first failure case 10 (intermittent, #9). Runs 2 and 3: first failure case 16 (#8: the skipped 4x half is counted as a failure). Same as the 8abeaabb baseline (46 / 48). Case 44 (es3hdr, #6) alone on f2a13877 (2026-10-11) still fails: check 3 got 0x40042814, wanted 0x40043904, the colour of texel (5,0) stored where (7,0) belongs in an 8-wide linear RGBA16F target."
    },
    "deqp2.am62p.previous": {
      "label": "dEQP-GLES2, last whole tree",
      "board": "am62p",
      "value": { "pass": 17141, "fail": 0, "notSupported": 19, "warnings": 5, "total": 17165 },
      "unit": "cases of the mustpass list",
      "commit": "d60b9d76",
      "onMain": false,
      "date": "2026-10-02",
      "source": "docs/es2-cts-failures.md",
      "section": "Summary; tools/cts/RESULTS.md 1z2",
      "status": "pass",
      "note": "Superseded by the 2026-10-10 tree on 32778b57 (the row above), which found 54 failures that this tree did not have."
    },
    "deqp2.am62p.integration": {
      "label": "dEQP-GLES2, shaders groups (images 1 to 7)",
      "board": "am62p",
      "value": { "pass": 10092, "fail": 9 },
      "unit": "cases",
      "commit": "b83216d4",
      "onMain": true,
      "date": "2026-10-08",
      "source": "tools/cts/log/board-sessions/am62p-plan1.md",
      "section": "Result",
      "status": "fail",
      "note": "An integration image, not a landing proof. Nine shaders.invariance common_subexpression cases that passed at d60b9d76 fail. Fixed on main in e17fb6a7 (2026-10-10, no fusion on the way to an invariant output): image 1 of the AM62P tree on e17fb6a7 gives 1,379 pass / 0 fail."
    },
    "deqp2.imx8mp.previous": {
      "label": "dEQP-GLES2, last whole tree",
      "board": "imx8mp",
      "value": { "pass": "17,112–17,116", "fail": 0, "notSupported": "30–34", "warnings": 19, "total": 17165 },
      "unit": "cases of the mustpass list",
      "commit": "d8583f20",
      "onMain": false,
      "date": "2026-10-01",
      "source": "docs/es2-cts-failures.md",
      "section": "A. i.MX8MP",
      "status": "pass",
      "note": "Superseded by the 2026-10-10 tree on 32778b57, which has the same counts in every selector. The range: the run covered 206 cases outside the mustpass list, four of them in the not-supported group. The 19 warnings appear on NXP's own driver too."
    },
    "deqp2.pi4": {
      "label": "dEQP-GLES2, whole tree",
      "board": "pi4",
      "value": { "pass": 17126, "fail": 0, "notSupported": 34, "warnings": 5, "total": 17165 },
      "unit": "cases of the mustpass list",
      "commit": "a27d69c0",
      "onMain": true,
      "date": "2026-10-11",
      "source": "tools/cts/RESULTS.md",
      "section": "Pi 4: the whole dEQP-GLES2 tree, 0 fail",
      "status": "pass",
      "note": "Composed of three records on main builds, not one image set: the whole tree on a27d69c0 (2026-10-10: 17,083 pass, 9 fail, 34 not supported, 3 quality and 2 compatibility warnings, 34 cases lost by the serial capture), the 9 cube cases passing on 1cf946a9 (the six rows whose fragment cube look-ups changed: 934 of 934 pass), and the 34 lost cases passing in a rerun of image F1. The 25 cube failures (#12) of the 2026-10-01 tree are 0. 34 not supported: 16 multisample, 9 compressed-texture negative tests, 8 clip_control, 1 multisampled_render_to_texture. A single-commit tree is not run."
    },
    "deqp2.pi4.previous": {
      "label": "dEQP-GLES2, last whole tree",
      "board": "pi4",
      "value": { "pass": 17075, "fail": 25, "notSupported": 62, "warnings": 3, "total": 17165 },
      "unit": "cases of the mustpass list",
      "commit": "fe5e8e73",
      "onMain": false,
      "date": "2026-10-01",
      "source": "docs/es2-cts-failures.md",
      "section": "A. Raspberry Pi 4 (sweep 30)",
      "status": "info",
      "note": "Superseded by the 2026-10-11 reading (the row above): the 25 cube-map failures (#12) are 0 on the board, and the 27 fbo and fragdata cases that were not supported pass."
    },
    "deqp3.am62p": {
      "label": "dEQP-GLES3, whole tree (28 images)",
      "board": "am62p",
      "value": { "pass": 32669, "fail": 5549, "notSupported": 2318, "warnings": 185, "total": 42810 },
      "unit": "cases of the mustpass list; 2,089 more have no result",
      "commit": "f2a13877",
      "onMain": true,
      "date": "2026-10-11",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: the whole dEQP-GLES3 tree on f2a13877, 28 images",
      "status": "info",
      "note": "The previous tree (c4ccc966, 2026-10-10) had 30,431 pass, 7,011 fail and 3,088 without a result. The 2,089 without a result: the tails of two images that stopped early (transform-feedback capture missing; a draw too big for the 1 MiB arena), the tails of two images whose GPU hangs on a 3D linear_mipmap_linear case (the board returns only with the reset), and 51 cases the serial capture lost (counted as run, not as failures). ES 3.0 work since then is host-only."
    },
    "gltest.am62p.previous": {
      "label": "gl_test regression set",
      "board": "am62p",
      "value": "48 / 48",
      "unit": "cases passed, 0 GPU resets",
      "commit": "05f8f749",
      "onMain": true,
      "date": "2026-10-06",
      "source": "tools/cts/RESULTS.md",
      "section": "Precompiled shaders on the AM62P without a compiler (origin/main 05f8f749)",
      "status": "pass",
      "note": "Both the compiler-only image and the precompiled + compiler image."
    },
    "gltest.imx8mp.previous": {
      "label": "gc_gl_test regression set",
      "board": "imx8mp",
      "value": "46 / 48",
      "unit": "cases passed",
      "commit": "8abeaabb",
      "onMain": true,
      "date": "2026-10-08",
      "source": "tools/cts/log/benchmarks-baseline/imx8mp.md",
      "section": "Tally",
      "status": "warn",
      "note": "First failure case 16, 4x MSAA (#8). Cases 2 and 10 fail intermittently (#25, #9)."
    },
    "gltest.pi4": {
      "label": "v3d_gl_test",
      "board": "pi4",
      "value": "12 / 12",
      "unit": "cases passed, 0 resets",
      "commit": "1cf946a9",
      "onMain": true,
      "date": "2026-10-11",
      "source": "tools/cts/RESULTS.md",
      "section": "Pi 4: gl_test on 1cf946a9 and the six cube rows",
      "status": "pass",
      "note": "Run on the Pi 4 through the tryboot product configuration."
    },
    "glmark2.am62p": {
      "label": "glmark2-es2",
      "board": "am62p",
      "value": 465,
      "unit": "score, 800x600 on-screen, 33 of 33 scenes ran, 0 GPU resets",
      "commit": "f2a13877",
      "onMain": true,
      "date": "2026-10-11",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: glmark2 (GM image) on f2a13877: the depth pass resolves",
      "status": "warn",
      "note": "Shadow and refract no longer sample stale depth: no 'did not resolve its depth' line in the capture (one per frame before; pvr-depth-resolve confirmed on the board). The earlier reading was 457 on 8e7257f8 (2026-10-04). The capture has no picture, so no scene was looked at: whether terrain still samples black (#1, fixed on the host) is not read from this run."
    },
    "glmark2.imx8mp": {
      "label": "glmark2-es2",
      "board": "imx8mp",
      "value": 42,
      "unit": "score, 800x600 on-screen, 33 of 33 scenes ran",
      "commit": "8abeaabb",
      "onMain": true,
      "date": "2026-10-08",
      "source": "tools/cts/log/benchmarks-baseline/imx8mp.md",
      "section": "Tally",
      "status": "warn",
      "note": "The board has no console, so there are no per-scene scores. README.md shows 37, the earlier round on 85c8d206 (2026-10-04, tools/cts/RESULTS.md, i.MX8MP common set round 4). Terrain samples black (#1; fixed on the host, not board-run)."
    },
    "glmark2.pi4": {
      "label": "glmark2-es2",
      "board": "pi4",
      "value": 183,
      "unit": "score, 800x600 on-screen, 33 of 33 scenes ran",
      "commit": "1fa941e3",
      "onMain": false,
      "date": "2026-10-04",
      "source": "tools/cts/RESULTS.md",
      "section": "Pi 4 common set round 3",
      "status": "warn",
      "note": "Refract and terrain did not fit the 48 MiB GPU carve-out (#4, #5); the glmark2 image now takes 96 MiB (ca4074a), not board-run. Terrain samples black (#1; fixed on the host)."
    },
    "glmark2ref.am62p": {
      "label": "glmark2, public reference (Linux, vendor driver)",
      "board": "am62p",
      "value": 306.4,
      "unit": "glmark2-DRM score, TI Processor SDK Linux 12.01, size not stated",
      "commit": null,
      "onMain": false,
      "date": null,
      "source": "driver/docs/glmark2.md",
      "section": "Verdin AM62P / AM62P (TI Linux Performance Guide, table 2.15)",
      "status": "info",
      "note": "Not our measurement; conditions differ. The source does not state its date."
    },
    "glmark2ref.imx8mp": {
      "label": "glmark2, public reference (Linux, vendor driver)",
      "board": "imx8mp",
      "value": 382,
      "unit": "glmark2-es2-wayland score, Vivante galcore 6.4.11, off-screen 800x600",
      "commit": null,
      "onMain": false,
      "date": "2025",
      "source": "driver/docs/glmark2.md",
      "section": "Verdin iMX8M Plus (NXP community thread)",
      "status": "info",
      "note": "Not our measurement; off-screen, while ours is on-screen."
    },
    "glmark2ref.pi4": {
      "label": "glmark2, public reference (Linux, Mesa)",
      "board": "pi4",
      "value": 697,
      "unit": "glmark2-es2 score, Mesa 23.2.1, 800x600 windowed",
      "commit": null,
      "onMain": false,
      "date": "2023-01",
      "source": "driver/docs/glmark2.md",
      "section": "Raspberry Pi 4 (geerlingguy/sbc-reviews issue 4)",
      "status": "info",
      "note": "Not our measurement; conditions differ."
    },
    "slint.am62p": {
      "label": "Slint sample",
      "board": "am62p",
      "value": "pass",
      "unit": "picture probes; 705 frames in 10,007 ms",
      "commit": "7dada1ff",
      "onMain": true,
      "date": "2026-10-05",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: Slint and cluster after the join removal",
      "status": "pass",
      "note": "Same records with the precompiled shader tier (17605c36)."
    },
    "slint.imx8mp": {
      "label": "Slint sample",
      "board": "imx8mp",
      "value": "pass",
      "unit": "picture probes, with the copy and through the scan-out view",
      "commit": "8abeaabb",
      "onMain": true,
      "date": "2026-10-08",
      "source": "tools/cts/log/benchmarks-baseline/imx8mp.md",
      "section": "Tally",
      "status": "pass",
      "note": "No frame rate: the board has no console."
    },
    "slint.pi4": {
      "label": "Slint sample",
      "board": "pi4",
      "value": "pass",
      "unit": "picture probes, with the copy and through the scan-out view",
      "commit": "1fa941e3",
      "onMain": false,
      "date": "2026-10-04",
      "source": "tools/cts/RESULTS.md",
      "section": "Pi 4 common set round 3",
      "status": "pass",
      "note": "Not run since."
    },
    "cluster.am62p": {
      "label": "Instrument cluster (agl-slint-cluster)",
      "board": "am62p",
      "value": 56.4,
      "unit": "Mpx/s (91.8 fps at 1024x600), probes pass",
      "commit": "7dada1ff",
      "onMain": true,
      "date": "2026-10-05",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: Slint and cluster after the join removal",
      "status": "pass",
      "note": "60 s drive cycle; fixed points of the picture checked against a PC reference."
    },
    "cluster.imx8mp": {
      "label": "Instrument cluster (agl-slint-cluster)",
      "board": "imx8mp",
      "value": "pass",
      "unit": "picture probes at 1280x800",
      "commit": "0f608491",
      "onMain": true,
      "date": "2026-10-08",
      "source": "tools/cts/log/benchmarks-baseline/imx8mp.md",
      "section": "What was built (CL row)",
      "status": "pass",
      "note": "No frame rate: the board has no console. Built with the integration branch's script, since main had no cluster row then."
    },
    "cluster.pi4": {
      "label": "Instrument cluster (agl-slint-cluster)",
      "board": "pi4",
      "value": 193.4,
      "unit": "Mpx/s (139.8 fps at 1920x720), probes pass",
      "commit": "031de9a2",
      "onMain": false,
      "date": "2026-10-04",
      "source": "tools/cts/RESULTS.md",
      "section": "Cluster acceptance run, one sample on three boards",
      "status": "pass",
      "note": "031de9a2 is the branch tree that brought the sample to main, before it landed. Not run since."
    },
    "boot.am62p": {
      "label": "Standalone boot",
      "board": "am62p",
      "value": "eMMC",
      "unit": "U-Boot ext4load + go; the boot-twice-then-Linux default ran end to end",
      "commit": null,
      "onMain": false,
      "date": "2026-10-05",
      "source": "docs/standalone-boot.md",
      "section": "State",
      "status": "pass",
      "note": "The records are not in git, and the image commit is not stated. The product form (default forever) has not run on any board."
    },
    "boot.imx8mp": {
      "label": "Standalone boot",
      "board": "imx8mp",
      "value": "eMMC",
      "unit": "U-Boot ext4load + go; a one-boot run from the eMMC proved",
      "commit": null,
      "onMain": false,
      "date": "2026-10-05",
      "source": "docs/standalone-boot.md",
      "section": "State",
      "status": "pass",
      "note": "The records are not in git, and the image commit is not stated. The counted default was set but its cycle is not proved; default forever has not run."
    },
    "boot.pi4": {
      "label": "Standalone boot",
      "board": "pi4",
      "value": "SD card",
      "unit": "the firmware's kernel= line; every test run boots the product configuration through tryboot.txt",
      "commit": null,
      "onMain": false,
      "date": "2026-10-05",
      "source": "docs/standalone-boot.md",
      "section": "State",
      "status": "pass",
      "note": "The literal config.txt swap has not run on the board."
    },
    "size.compiler": {
      "label": "The on-board shader compiler's cost",
      "board": "am62p",
      "value": 884201,
      "unit": "bytes of flash (plus 5,244,035 bytes of RAM for its scratch and keep regions)",
      "commit": "67488fec",
      "onMain": true,
      "date": "2026-10-07",
      "source": "docs/HANDOFF.md",
      "section": "Open work 1 (step 6)",
      "status": "info",
      "note": "AM62P gl_test image, release build. The commit is the one that recorded the numbers; the image's own commit is not stated.",
      "unverified": true
    },
    "size.table": {
      "label": "A precompiled table for one app (gl_test_es100, 176 sources)",
      "board": "am62p",
      "value": 2173648,
      "unit": "bytes of flash",
      "commit": "67488fec",
      "onMain": true,
      "date": "2026-10-07",
      "source": "docs/HANDOFF.md",
      "section": "Open work 1 (step 6)",
      "status": "info",
      "note": "It grows with the app's shader set; the compiler does not. So the tier is off by default. The commit is the one that recorded the numbers.",
      "unverified": true
    }
  }
};
