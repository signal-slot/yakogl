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
  "checkedAgainst": {
    "commit": "ab43a20c5",
    "date": "2026-10-16",
    "source": "tools/cts/RESULTS.md"
  },
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
      "value": {
        "pass": 17140,
        "fail": 0,
        "notSupported": 18,
        "warnings": 7,
        "total": 17165
      },
      "unit": "cases of the mustpass list (17 images at 1x, the 16 multisample.* cases from the 4x image)",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "0 GPU resets. Warnings: 5 quality, 2 compatibility (flush_finish timing). The 18 not supported are the accepted ES 2.0 buckets: clip_control 8, compressed-texture negative tests 9, multisampled_render_to_texture.readpixels 1."
    },
    "deqp2.am62p.ms4": {
      "label": "dEQP-GLES2, multisample on the 4x image",
      "board": "am62p",
      "value": {
        "pass": 16,
        "fail": 0
      },
      "unit": "cases (the 16 multisample.* cases)",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "Counted in the whole-tree figure above in place of the 16 multisample cases the 1x configuration cannot run."
    },
    "deqp2.imx8mp": {
      "label": "dEQP-GLES2, whole tree",
      "board": "imx8mp",
      "value": {
        "pass": 17318,
        "fail": 0,
        "notSupported": 34,
        "warnings": 19,
        "total": 17371
      },
      "unit": "cases begun (the 17,165 of the mustpass list plus 206 outside it)",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "0 GPU resets. The 34 not supported are the accepted ES 2.0 buckets (16 of them multisample at 1x). The 19 warnings appear on NXP's own driver too."
    },
    "deqp2.pi4": {
      "label": "dEQP-GLES2, whole tree",
      "board": "pi4",
      "value": {
        "pass": 17126,
        "fail": 0,
        "notSupported": 34,
        "warnings": 5,
        "total": 17165
      },
      "unit": "cases of the mustpass list; every case has a result",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-16",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "0 recoveries in 29 records. Warnings: 3 quality, 2 compatibility. The 34 not supported are the accepted ES 2.0 buckets (16 of them multisample at 1x)."
    },
    "deqp3.am62p": {
      "label": "dEQP-GLES3, whole tree (28 images)",
      "board": "am62p",
      "value": {
        "pass": 36552,
        "fail": 2880,
        "notSupported": 2195,
        "warnings": 185,
        "total": 42810
      },
      "unit": "cases of the mustpass list; 998 more have no result",
      "commit": "ad2b5ed",
      "onMain": true,
      "date": "2026-10-14",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 3.0: dEQP-GLES3 (stage 4)",
      "status": "info",
      "note": "The 998 without a result: image 7 stops at a draw too big for its arena (987 cases after it) and 11 slow 3D filtering cases did not start; two 3D filtering cases hang the GPU. Some groups were re-run later and pass in full (transform_feedback, ubo, occlusion_query, fragment_ops.blend.fbo_srgb)."
    },
    "gltest.am62p": {
      "label": "gl_test regression set",
      "board": "am62p",
      "value": "50 / 50",
      "unit": "cases passed, 0 GPU resets",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": "The ES 3.0 cases 196 and 200 to 205 pass too (on cd738b1a)."
    },
    "gltest.imx8mp": {
      "label": "gc_gl_test regression set",
      "board": "imx8mp",
      "value": "49 / 49",
      "unit": "cases run passed; case 16 skipped (4x MSAA, #8)",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": "Case 44 (#6, the RGBA16F target) passes since the 64-bit render-target fix."
    },
    "gltest.pi4": {
      "label": "gl_test regression set",
      "board": "pi4",
      "value": "12 / 12",
      "unit": "cases passed, 0 GPU resets",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-16",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": ""
    },
    "glmark2.am62p": {
      "label": "glmark2-es2",
      "board": "am62p",
      "value": 462,
      "unit": "score at 800x600; 33 scenes, 0 failed, 0 GL errors",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "warn",
      "note": "Pictures are not checked per scene yet. terrain, shadow and refract are slow (1.47, 0.38 and 2.08 s a frame), time spent in GL calls on the CPU."
    },
    "glmark2.imx8mp": {
      "label": "glmark2-es2",
      "board": "imx8mp",
      "value": 43,
      "unit": "score at 800x600; 33 scenes, 0 failed",
      "commit": "a63fcc5b",
      "onMain": true,
      "date": "2026-10-14",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "warn",
      "note": "At e9901869a the 33 scenes run with 0 failed but raise 15 or more GL errors (#814); the board has no console, so that run gives no score. The score is probably capped by a wait for the panel's VSYNC every frame."
    },
    "glmark2.pi4": {
      "label": "glmark2-es2",
      "board": "pi4",
      "value": 188,
      "unit": "score at 800x600; 33 scenes, 0 failed, 0 GL errors",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-16",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "warn",
      "note": "Pictures are not checked per scene yet."
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
      "unit": "picture probes, with the copy (SO) and through the scan-out view (SV)",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": ""
    },
    "slint.imx8mp": {
      "label": "Slint sample",
      "board": "imx8mp",
      "value": "pass",
      "unit": "picture probes, with the copy (SO) and through the scan-out view (SV)",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": "No frame rate: the board has no console."
    },
    "slint.pi4": {
      "label": "Slint sample",
      "board": "pi4",
      "value": "pass",
      "unit": "picture probes, with the copy (SO) and through the scan-out view (SV)",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-16",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": ""
    },
    "cluster.am62p": {
      "label": "Instrument cluster (agl-slint-cluster)",
      "board": "am62p",
      "value": 44.8,
      "unit": "Mpx/s (73.0 fps at 1024x600), probes pass",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": ""
    },
    "cluster.imx8mp": {
      "label": "Instrument cluster (agl-slint-cluster)",
      "board": "imx8mp",
      "value": "pass",
      "unit": "picture probes at 1280x800",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": "No frame rate: the board has no console."
    },
    "cluster.pi4": {
      "label": "Instrument cluster (agl-slint-cluster)",
      "board": "pi4",
      "value": 192.8,
      "unit": "Mpx/s (139.4 fps at 1920x720), probes pass",
      "commit": "e9901869a",
      "onMain": true,
      "date": "2026-10-16",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": ""
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
      "section": "Open work 2",
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
      "section": "Open work 2",
      "status": "info",
      "note": "It grows with the app's shader set; the compiler does not. So the tier is off by default. The commit is the one that recorded the numbers.",
      "unverified": true
    }
  }
};
