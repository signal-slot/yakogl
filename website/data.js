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
//             {pass, fail, notSupported, warnings, noResult, total}
//   unit      what the value counts
//   commit    the commit the image was built from (null: not ours, or not recorded)
//   onMain    true only for an ancestor in this public repository
//   provenance.kind   repository | pre-publication | external | unrecorded-build
//   provenance.recordCommit   public commit containing the cited record
//             Pre-publication hashes are identifiers, not public Git links.
//   date      YYYY-MM-DD (or YYYY-MM / YYYY for a public reference); null if unknown
//   source    repository path of the record the figure comes from
//   section   where in that file
//   status    "pass" | "warn" | "fail" | "info"
//   note      what a reader must know to read the figure honestly
//   unverified  present and true when part of the provenance could not be checked
window.YAKOGL_DATA = {
  "repo": "https://github.com/signal-slot/yakogl",
  "checkedAgainst": {
    "commit": "381b0f09e9c8e7b3d437946c888224f12a27b367",
    "date": "2026-10-27",
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
      "glTestSample": "gl_test",
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
      "glTestSample": "gl_test",
      "panel": "1920x720",
      "storage": "SD card (firmware)"
    }
  ],
  "figures": {
    "deqp2.am62p": {
      "label": "dEQP-GLES2, whole tree at 1x",
      "board": "am62p",
      "value": {
        "pass": 17330,
        "fail": 0,
        "notSupported": 34,
        "warnings": 7,
        "total": 17371
      },
      "unit": "cases begun at 1x (17,165 mustpass cases plus 206 outside the list)",
      "commit": "996bc35f",
      "onMain": false,
      "date": "2026-10-26",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "0 GPU resets. Within the mustpass list: 17,124 pass, 34 not supported, 7 warnings. The 16 multisample cases require the separate 4x image below; that image has not been rerun since 2026-10-15.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
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
      "onMain": false,
      "date": "2026-10-15",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "A separate 4x run from 2026-10-15. These 16 passes replace the unsupported multisample cases only when reporting a combined 1x/4x result; they are not part of the newer 1x result above.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
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
      "commit": "d0c5dc48",
      "onMain": false,
      "date": "2026-10-23",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "0 GPU resets. The 34 not-supported cases include the 16 multisample cases at 1x. The 19 quality warnings are also reported with NXP's driver.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
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
      "commit": "46f93767",
      "onMain": false,
      "date": "2026-10-24",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 2.0: dEQP-GLES2 (stage 1, met)",
      "status": "pass",
      "note": "0 GPU recoveries in 29 records. Three quality warnings and two timing compatibility warnings. The 34 not-supported cases include the 16 multisample cases at 1x.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "deqp3.am62p": {
      "label": "dEQP-GLES3, recorded full-tree baseline",
      "board": "am62p",
      "value": {
        "pass": 36552,
        "fail": 2880,
        "notSupported": 2195,
        "warnings": 185,
        "total": 42810,
        "noResult": 998
      },
      "unit": "cases of the ES 3.0 mustpass list; baseline from 2026-10-14",
      "commit": "ad2b5ed",
      "onMain": false,
      "date": "2026-10-14",
      "source": "tools/cts/RESULTS.md",
      "section": "ES 3.0: dEQP-GLES3 (stage 4)",
      "status": "warn",
      "note": "This full-tree baseline has not been replaced by a complete rerun. Later targeted runs pass several groups; five images of a revised 31-image set completed on 2026-10-25. Other images remain unrun, and 3D texture filtering can hang the GPU. Product builds report ES 2.0 by default.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "gltest.am62p": {
      "label": "gl_test regression set",
      "board": "am62p",
      "value": "50 / 50",
      "unit": "cases passed, 0 GPU resets",
      "commit": "445c5653",
      "onMain": false,
      "date": "2026-10-20",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": "The common regression set, run from the same build as the AM62P benchmark below.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "gltest.imx8mp": {
      "label": "gl_test regression set",
      "board": "imx8mp",
      "value": "49 / 49",
      "unit": "cases run passed; case 16 skipped (4x multisampling)",
      "commit": "445c5653",
      "onMain": false,
      "date": "2026-10-20",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "warn",
      "note": "This run passes every case that was run. Earlier repeated boots expose intermittent failures in cases 10 and 17; the record does not establish that those defects are fixed.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "gltest.pi4": {
      "label": "gl_test regression set",
      "board": "pi4",
      "value": "12 / 12",
      "unit": "cases passed, 0 GPU resets",
      "commit": "445c5653",
      "onMain": false,
      "date": "2026-10-20",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "pass",
      "note": "The gl_test result was not rerun with the newer Pi 4 glmark2 build below.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "glmark2.am62p": {
      "label": "glmark2-es2",
      "board": "am62p",
      "value": 461,
      "unit": "score at 800x600; 33 scenes, 0 failed, 0 GL errors",
      "commit": "445c5653",
      "onMain": false,
      "date": "2026-10-20",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "warn",
      "note": "All 33 scene picture values are recorded; none is black. Twenty-nine match the earlier run inspected on the panel, with four differences in animated scenes. Heavy scenes remain slow: terrain about 94 ms, shadow 353 ms, refract 376 ms per frame.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "glmark2.imx8mp": {
      "label": "glmark2-es2",
      "board": "imx8mp",
      "value": 42,
      "unit": "score at 800x600; 33 scenes, 0 failed, 0 GL errors",
      "commit": "a917d123",
      "onMain": false,
      "date": "2026-10-20",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "warn",
      "note": "Default one-buffer configuration. All 33 picture values match the earlier 445c5653 run; they have not been visually reviewed on the panel. Terrain is 4 FPS, shadow and refract 2 FPS. The texture/mipmap picture change remains unexplained. Extra display-buffer configurations are separate experiments.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "glmark2.pi4": {
      "label": "glmark2-es2",
      "board": "pi4",
      "value": 195,
      "unit": "score at 800x600; 33 scenes, 0 failed, 0 GL errors",
      "commit": "c4af4917",
      "onMain": false,
      "date": "2026-10-22",
      "source": "tools/cts/RESULTS.md",
      "section": "Common set (stage 2 input)",
      "status": "warn",
      "note": "All 33 scenes recorded; none is black. Thirty-one picture values match the earlier column; two animated scenes differ. Terrain is 161.79 ms, shadow 7.34 ms and refract 17.27 ms per frame. This is a newer build than the common-commit 193 score in the README.",
      "provenance": {
        "kind": "pre-publication",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
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
      "note": "Not our measurement; conditions differ. The source does not state its date.",
      "provenance": {
        "kind": "external",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
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
      "note": "Not our measurement; off-screen, while ours is on-screen.",
      "provenance": {
        "kind": "external",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
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
      "note": "Not our measurement; conditions differ.",
      "provenance": {
        "kind": "external",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
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
      "note": "The records are not in git, and the image commit is not stated. The product form (default forever) has not run on any board.",
      "provenance": {
        "kind": "unrecorded-build",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      },
      "unverified": true
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
      "note": "The records are not in git, and the image commit is not stated. The counted default was set but its cycle is not proved; default forever has not run.",
      "provenance": {
        "kind": "unrecorded-build",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      },
      "unverified": true
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
      "note": "The public record does not identify an image commit. The literal config.txt swap has not run on the board.",
      "provenance": {
        "kind": "unrecorded-build",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      },
      "unverified": true
    },
    "size.compiler": {
      "label": "The on-board shader compiler's cost",
      "board": "am62p",
      "value": 884201,
      "unit": "bytes of flash (plus 5,244,035 bytes of RAM for its scratch and keep regions)",
      "commit": null,
      "onMain": false,
      "date": "2026-10-07",
      "source": "zephyr/Kconfig",
      "section": "YAKOGL_PRECOMPILED_SHADERS",
      "status": "info",
      "note": "AM62P gl_test image, measured 2026-10-07. The public record gives these sizes but does not identify the image commit.",
      "unverified": true,
      "provenance": {
        "kind": "unrecorded-build",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    },
    "size.table": {
      "label": "A precompiled table for one app (gl_test_es100, 176 sources)",
      "board": "am62p",
      "value": 2173648,
      "unit": "bytes of flash",
      "commit": null,
      "onMain": false,
      "date": "2026-10-07",
      "source": "zephyr/Kconfig",
      "section": "YAKOGL_PRECOMPILED_SHADERS",
      "status": "info",
      "note": "Measured 2026-10-07 for one application; the table grows with its shader set. The public record does not identify the image commit.",
      "unverified": true,
      "provenance": {
        "kind": "unrecorded-build",
        "recordCommit": "381b0f09e9c8e7b3d437946c888224f12a27b367"
      }
    }
  }
};
