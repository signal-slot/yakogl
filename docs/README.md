<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Documentation

What each place holds. Every file here describes the current state or a
design the code follows; there are no run logs or session diaries.

| Where | What |
| --- | --- |
| `es2-cts-failures.md`, `es3-cts-failures.md` | dEQP-GLES2 and dEQP-GLES3: what is accepted, what fails now and why. |
| `es2/` | The NanoPC-T6 (Mali-G610) dEQP-GLES2 board recipe, `es2/board-recipe-mali.md`: library, deqp-gles2 build, tree table, runner, kernel messages, tally and board rules. |
| `es3/` | ES 3.0 designs per feature, the board recipes, and the current per-case failure lists (`*.tsv`). |
| `online-shaderc/` | The on-board shader compiler: front end, IR, back ends, ISAs and ABIs. |
| `notes/` | Designs and findings that the code relies on (comments cite them), one file per subject. Each file's title line says what it covers: `grep -m1 '^# ' docs/notes/*.md` lists them. This row names no file. |
| `release/` | Release stage: building guide, checklist, licence and redaction audits. |
| `ci.md`, `source-form.md`, `standalone-boot.md`, `v3d-noseamless-cube.md`, `notes/v3d-texture-size-words.md` | The CI gate, the source-form rule, standalone boot, two V3D designs: the non-seamless cube, and the texture size uniform words 44-47. |
| `notes/v3d71-cube-seamless.md` | Mesa's 7.1 cube-map and seamless reading (packets, driver, CI): what the 7.1 table sets for a cube, the U8 prediction, and the case board71-tex runs. |
| `notes/pre-study-rpi5.md` | The Pi 5 pre-study (BCM2712, VideoCore VII, V3D 7.1): the host reading of Mesa, Linux and the firmware, and the P and U steps stage 5 cites from it. |
| `notes/pre-study-imx95.md` | The Toradex Verdin iMX95 on Mallow pre-study (NXP i.MX 95, Mali-G310): the module's variants, the boot chain and the U-Boot sign-off route, the System Manager's rights for our agent, the firmware, and the host and board items in order. |
| `notes/imx95-direct-boot.md` | The Verdin iMX95 direct boot (pre-study H5, B7): the boot chain and its containers as Toradex's imx-boot has them, what checks what on an open part, where our image can replace U-Boot proper (the recommended BL33 slot), the boot partition fallback and the way back, `tools/imx95-mkboot.py`, and the board steps. |
| `notes/v3d71-backend.md` | The 7.1 online-compiler back end (P8): the design that fixes the file names, types and function signatures the later items implement. |
| `notes/v3d71-clear-frame.md` | The 7.1 clear-only frame packet by packet and the 7.1 tile sizes: the specification the frame builder follows for the first clears on the board (H5, M1-M4). |
| `notes/v3d71-triangle-frame.md` | The 7.1 clear-then-triangle frame (M5): what the job holds packet by packet, and the BCM2712 D0 shader record it needs. |
| `notes/v3d71-bin-submit.md` | The bin and render submission for the first triangle with a binning pass on the board (M5): the facts for the 7.1 frame, the `rpi5_triangle` probe and the board session. |
| `notes/v3d71-tri-ours-record.md` | The D0 shader record Mesa wrote for its Pi 5 triangle against the one our 7.1 pass-through programs need: the input of the probe's "ours" option. |
| `notes/v3d71-gl-table.md` | The P9 figures for the 7.1 `src/v3d` back-end table (the 7.1 shader record and draw index, the limits, `Clear Render Targets`, TFU `IOC`) and how a build selects 7.1. |
| `notes/rpi5-display.md` | The Pi 5 display design (P11, U3): which Pi 4 pieces carry over, what the HVS 6.x and pixel valves make new, one implementation order for a Zephyr image that draws GL on the board, and what only the board can still answer. |
| `notes/mali-compiler-road-to-es2.md` | The Mali online compiler's road to zero ES 2 refusals: its stages (general or shape-matched), the measured tail at `48f6017cb` (refusals by reason class, distinct shaders behind them, what a lowered family exposes next), Mesa's passes against ours, and the options (per family, general mechanisms, a mix) with cost, order and the three hardest problems. |
| `notes/mali-g310-vs-g610.md` | The i.MX 95 port's host measurement: what Mesa 26.2.1 generates for a Mali-G310 against the Mali-G610 (every reference's shader words, FAU and descriptors, frame probes), the Mesa lines that tell the two apart, and which of our files carry a G610 assumption. |
| `notes/imx95-linux-panthor.md` | The Linux route on the Verdin iMX95: the stock images drive the G310 with `mali_kbase` and have no panthor (Toradex and mainline trees read), the ways to a panthor Linux on the module and the recommended one (mainline v7.1+ with the Mallow DT and Debian trixie on a microSD card), the day-one command sequence with `MALI_BOARD=verdin-imx95` (gl_test sweep, then the dEQP-GLES2 tree), and what each result settles for the Zephyr side. |
| `../driver/docs/imx95-display.md` | The Verdin iMX95 on Mallow display design: the i.MX 95 DPU, LDB and DSI with their addresses, clocks and SCMI domains, what the Mallow puts behind them (the owner's Toradex 10.1" DSI display on X4, LVDS on X12 unused, no HDMI with this module), the permissive sources (hal_nxp, Zephyr), three routes (adopt, cold DSI, off-screen), the cold DSI path to implement from (1280x800 at 70 MHz, 420 Mbit/s a lane, the combo D-PHY PLL, the SN65DSI83, expander, backlight and touch), the day-one board reads and the unknowns. |

Elsewhere: board results are `tools/cts/RESULTS.md` (current results only)
and its Japanese summary `tools/cts/STATUS.md`; the stages are `ROADMAP.md`;
the driver's own documents are under `driver/docs/`; component notes sit next
to their code (`src/*_NOTES.md`, `src/*/*_NOTES.md`, `tools/*/README.md`).

## Where a new finding goes

- A board run: its row in `tools/cts/RESULTS.md` (replacing the old one), the
  summary in `tools/cts/STATUS.md`, and a comment on its GitHub issue. No
  session file.
- A design, or a finding the code depends on: `notes/` (or next to the code),
  cited from the code.
- Open work and known defects: GitHub issues, never a file here.
- Anything else (how a run went, what was tried): the commit message.

## Removed records

Board-session records, investigations and diaries that only recorded past
runs were removed on 2026-10-16 (`docs/board-sessions/`,
`docs/investigations/`, the old per-session logs in `tools/cts/log/`, and
the old measurement log in `tools/cts/RESULTS.md`). `tools/cts/log/` has
held files again since e839242b (2026-10-27): the slices that RESULTS.md
rows cite, as `tools/cts/log/README.md` defines them. Comments that still name one write it as
`ab43a20c5:<path>`. The Mali board captures (`tools/mali/board-ref/`) and
the last two session records were removed on 2026-10-26; comments name
them as `0dce654a8:<path>`. The excerpts the tests read are kept in
`tools/mali/board-dump/`.

These hashes, and every commit hash the docs, records and code comments
cite from before the first public commit, name commits of the non-public
working repository the project was developed in. They cannot be read in
the public repository (CONTRIBUTING.md, "Branches and commits").
