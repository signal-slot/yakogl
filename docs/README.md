<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Documentation

What each place holds. Every file here describes the current state or a
design the code follows; there are no run logs or session diaries.

| Where | What |
| --- | --- |
| `es2-cts-failures.md`, `es3-cts-failures.md` | dEQP-GLES2 and dEQP-GLES3: what is accepted, what fails now and why. |
| `es2/` | The NanoPC-T6 (Mali-G610) dEQP-GLES2 board recipe, `es2/board-recipe-mali.md`: library, deqp-gles2 build, tree table, runner, kernel messages, tally and board rules. |
| `es3/` | ES 3.0 designs per feature, the board recipes, the shader and texture failure maps (`es3/shader-failure-map.md`, `es3/texture-failure-map.md`) with their host repros, `es3/zephyr-env.md` (the Zephyr build environment for the gate rows), the HOST-ONLY GC7000UL dEQP-GLES3 run in `es3/host-gc-gles3-5454be1b/` (its `README.md`), and the current per-case failure lists (`*.tsv`). |
| `online-shaderc/` | The on-board shader compiler: front end, IR, back ends, ISAs and ABIs. |
| `notes/pi4-slow-terrain.md`, `notes/v3d-terrain-shaders.md` | glmark2 terrain on the Pi 4 (V3D 4.2): the frame's breakdown, the ranked list of what is left with the measured scheduler result (34.758 -> 31.906 ms at `3e950f0f`) and rows 2 and 3 waiting on `src/gl`; and terrain's shader word counts, ours against Mesa's measured 4.2 counts, with the per-job render model. |
| `notes/` | Designs and findings that the code relies on (comments cite them), one file per subject. Each file's title line says what it covers: `grep -m1 '^# ' docs/notes/*.md` lists them. The files that need a longer description have their own rows below. |
| `release/` | Release stage: `release/building.md` (building guide), `release/checklist.md`, `release/license-audit.md` and `release/redaction-audit.md` (the audits), `release/snapshot.md` (the public snapshot commit) and `release/readme-walkthrough.md` (the README's instructions walked on a fresh clone). |
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
| `notes/v3d71-table71.md` | How the 7.1 GL table `yakogl_backend_v3d71` maps the GL core onto fill71: its entries against the 4.2 table, `GlesDraw` and the clears field by field onto `fill71::Op`, `Framebuffer` and `History`, the `state.rs` reuse and the shared-file fixes it names, memory, submit and readback, the host-test mock, and each gl_test P9 case's expected verdict and path. |
| `notes/rpi5-display.md` | The Pi 5 display design (P11, U3): which Pi 4 pieces carry over, what the HVS 6.x and pixel valves make new, one implementation order for a Zephyr image that draws GL on the board, and what only the board can still answer. |
| `notes/rpi5-gl-scanout.md` | The plan for `rpi5_gl_scanout` (after P9): a 1920x1080 V3D 7.1 frame rendered into our own XBGR8888 HVS6 plane, with the memory layout, the cache rules, the Y flip, the vblank swap and restore, what the 2026-11-25 `rpi5_scanout32` run does and does not answer, the verdict lines, and the order of items G1-G5. |
| `notes/rpi5-gate-rows.md` | The Pi 5 gate rows (P12): the rows in `tools/check.sh` on the tip, the rows P9 to P13 still need, and the order in which the open items add them, for 0 gaps. |
| `notes/rpi5-deqp-glmark2.md` | What P13 (dEQP-GLES2 and glmark2 on the Pi 5) needs: how the Pi 4 does the same two things, what the Pi 5 images lack, and what the 7.1 table must have. |
| `notes/v3d71-refusals.md` | Every named refusal of the V3D 7.1 path (host modules, C builders, online compiler) and the gl_test cases that meet it, for the board session to report against. |
| `notes/mali-compiler-road-to-es2.md` | The Mali online compiler's road to zero ES 2 refusals: its stages (general or shape-matched), the measured tail at `48f6017cb` (refusals by reason class, distinct shaders behind them, what a lowered family exposes next), Mesa's passes against ours, and the options (per family, general mechanisms, a mix) with cost, order and the three hardest problems. |
| `notes/mali-g310-vs-g610.md` | The i.MX 95 port's host measurement: what Mesa 26.2.1 generates for a Mali-G310 against the Mali-G610 (every reference's shader words, FAU and descriptors, frame probes), the Mesa lines that tell the two apart, and which of our files carry a G610 assumption. |
| `notes/pvr-vertex-outputs-64.md` | The src/gl change that lets the PowerVR answer GL_MAX_VERTEX_OUTPUT_COMPONENTS 64 (now 60): the patch for `context.rs` and `state.rs`, why 64 is safe (`tests/pvr_vertex_outputs_64.rs`), what moves on the other GPUs and in dEQP, and the tests that pin it. |
| `notes/pvr-npot-cube-levels.md` | The src/gl change that keeps every level of a NPOT PowerVR cube map (cause K3, 56 AM62P `texture.specification` fails): the cause through `src/gl` and `src/pvr`, the patch against main (rebased from `2d215408`), the host test `tests/pvr_npot_cube.rs`, and what else moves. |
| `notes/pvr-npot-array-levels.md` | The src/gl change that keeps every level of a NPOT PowerVR 2D array (cause K4, 76 AM62P `texture.specification` fails): why the native array alone does not fix it, the cause through `src/gl` and `src/pvr`, the patch against main (and a variant after the K3 cube patch), the stacked-array draw rule, the host test `tests/pvr_npot_array_chain.rs`, and the K4 case groups. |
| `notes/pvr-refract-draw.md` | PowerVR host, glmark2 refract and shadow: 94 to 97% of the draw call was the CPU depth resolve (before fix 1); a ranked fix list. |
| `notes/pvr-depth-sample.md` | PowerVR: the TPU samples the ZLS depth buffer as an X8U24 stride image, no CPU resolve, with its TPU state words. |
| `notes/gc-slow-terrain.md`, `notes/gc-terrain-shaders.md`, `notes/gc-terrain-shaders-rest.md` | GC7000: glmark2 terrain on the i.MX8MP (measured on the Armadillo-X2 in place of the Verdin): the frame's host breakdown and ranked causes, the compiler changes of plan #421 with what each removed and which ranks did not land (the ten draw, blur and overlay shaders 475 -> 234 instructions against etnaviv's 279, the noise shader 840 -> 471 against 414, at `3df3adfc9`), and the X2 rows (162.0 ms a frame at `85afd0cf`; the after row not measured). |
| `notes/gc-depth-sample.md` | GC7000: can the texture unit sample the D24S8 depth surface instead of the CPU resolve? The board case `dtpu` (gl_test 222, knob `gc_debug_set_depth_tpu`, checks 3 to 7), its readout page at `SEL=0x10b0`; the X2 11-11 result (check 4 failed) and the X2 sessions of 2026-11-27 in place of the Verdin iMX8MP: the first stopped at the kexec loader, the second gave verdict code 3 (check 3 passed, check 7 failing), outcome (a) plus the texture-cache flush. |
| `notes/gc-depth-sample-wire.md` | GC7000: the code for each outcome of the `dtpu` verdict: (a) a depth view, (a) plus a texture-cache flush, (b) a GPU copy, (c) the CPU resolve stays (the verdict comes from the Armadillo-X2: code 3, check 7 failing, so (a) plus the flush, knob mode 4, the product default), whose three CPU fixes are on main as `25fa1a96`, `e320630d` and `e58be17b`. |
| `notes/gc-slow-shadow-refract.md` | GC7000: the host breakdown of one glmark2 `shadow` and `refract` frame (GL calls, CPU passes over GPU memory, waits, kicks), the ranked causes of the 2 FPS on the i.MX8MP (measured on the Armadillo-X2 in place of the Verdin iMX8MP) with a fix for each, and the PERF values that confirm them (the before rows hold the 2026-11-27 X2 attempt: the loader refused the jump, no figures). |
| `notes/imx95-linux-panthor.md` | The Linux route on the Verdin iMX95: the stock images drive the G310 with `mali_kbase` and have no panthor (Toradex and mainline trees read), the ways to a panthor Linux on the module and the recommended one (mainline v7.1+ with the Mallow DT and Debian trixie on a microSD card), the day-one command sequence with `MALI_BOARD=verdin-imx95` (gl_test sweep, then the dEQP-GLES2 tree), and what each result settles for the Zephyr side. |
| `notes/stm32mp257f-dk-bringup.md` | The STM32MP257F-DK's board runs in order (return trial, GPU power and identity, `gc_fence`, `gc_clear`, `gc_triangle`, `gl_test` case 1): each step's expected lines, the board verdicts with their records, and the findings (the watchdog catch, GPU 0x8000/0x6205 customer 0x15, the supply-fault guide keyed on BL2's reset reason). |
| `notes/stm32mp257f-dk-p10.md` | The STM32MP257F-DK's P10: what direct boot from ST's own U-Boot means here (the env-armed one-shot as the sign-off row, the counted default as the product proof, the FIP never written), the board constraints, what `tools/board.sh` needs for a DK row (steps, verdict line, record layout, reset by resource), the gap list with the file each gap lives in, the board-session order and what is not verified. |
| `../driver/docs/imx95-display.md` | The Verdin iMX95 on Mallow display design: the i.MX 95 DPU, LDB and DSI with their addresses, clocks and SCMI domains, what the Mallow puts behind them (the owner's Toradex 10.1" LVDS display on X12 since 2026-11-24; the X4 DSI path kept; no HDMI with this module), the permissive sources (hal_nxp, Zephyr), three routes (adopt, cold, off-screen), the cold X12 LVDS path as built (the LDB PLL, the LDB and LVDS PHY 0, the DPU, the TPM5 backlight) with the board's Linux dump decoded and every disagreement between hal_nxp and Linux named, the cold X4 DSI path (the combo D-PHY PLL, the SN65DSI83, expander, backlight), the day-one board reads with what they returned and the unknowns. |
| `notes/imx95-gpu-power.md` | The i.MX 95 GPU power over SCMI (Verdin iMX95): the System Manager's ids, Linux's path reduced to messages, the A55 agent's permissions in Toradex's SM configuration, what Zephyr's SCMI drivers lack, and what day one on the board confirms. |
| `notes/mrt4-dbi-wiring.md` | Wiring the per-draw-buffer blend state (`GL_EXT/OES_draw_buffers_indexed`) into the V3D and PowerVR back ends: what `GlesDraw` carries per slot, V3D per-RT `BLEND_CFG` and colour masks, the PowerVR key, which plan owns each file, the item order and when the extensions may be listed. |

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
