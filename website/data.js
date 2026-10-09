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
  "checkedAgainst": { "commit": "32778b57", "date": "2026-10-10", "source": "docs/HANDOFF.md" },
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
      "note": "No whole tree has run since. A later integration image found 9 regressions (below)."
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
    "deqp2.imx8mp": {
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
      "note": "The range: the run covered 206 cases outside the mustpass list, four of them in the not-supported group. The 19 warnings appear on NXP's own driver too."
    },
    "deqp2.pi4": {
      "label": "dEQP-GLES2, last whole tree",
      "board": "pi4",
      "value": { "pass": 17075, "fail": 25, "notSupported": 62, "warnings": 3, "total": 17165 },
      "unit": "cases of the mustpass list",
      "commit": "fe5e8e73",
      "onMain": false,
      "date": "2026-10-01",
      "source": "docs/es2-cts-failures.md",
      "section": "A. Raspberry Pi 4 (sweep 30)",
      "status": "warn",
      "note": "The 25 failures are cube-map cases (#12). They are fixed on the host (ec5ba4f) and not yet run on the board."
    },
    "deqp3.am62p": {
      "label": "dEQP-GLES3, the only board run",
      "board": "am62p",
      "value": { "pass": 31278, "fail": 8835, "notSupported": 2636, "warnings": 5, "total": 42810 },
      "unit": "cases of the mustpass list",
      "commit": "396d667a",
      "onMain": false,
      "date": "2026-09-26",
      "source": "tools/cts/RESULTS.md",
      "section": "1r",
      "status": "info",
      "note": "56 lifetime cases had no result (transform feedback was missing). ES 3.0 work since then is host-only."
    },
    "gltest.am62p": {
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
    "gltest.imx8mp": {
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
      "unit": "cases passed",
      "commit": "afa12e9b",
      "onMain": true,
      "date": "2026-10-05",
      "source": "docs/HANDOFF.md",
      "section": "State of main (results table)",
      "status": "pass",
      "note": "Not run since. The date is the commit's; the run date is not in the committed records.",
      "unverified": true
    },
    "glmark2.am62p": {
      "label": "glmark2-es2",
      "board": "am62p",
      "value": 457,
      "unit": "score, 800x600 on-screen, 33 of 33 scenes ran",
      "commit": "8e7257f8",
      "onMain": false,
      "date": "2026-10-04",
      "source": "tools/cts/RESULTS.md",
      "section": "AM62P: the reclaim default, common round",
      "status": "warn",
      "note": "Refract's depth pass was refused every frame (#2; fixed on the host, 541c9d3). Terrain samples black (#1; fixed on the host). Shadow and refract sample stale depth (pvr-depth-resolve, open). None of the fixes has run on the board."
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
