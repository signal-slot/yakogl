<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 17-31 (LT M1 PR R1 SQ S01 S02 S03 S04 S05 S06 S07 S08 S09 S10) at b1b5b845

commit b1b5b8456ec69247c64d1bd07e42461266dc5136

Session `2026-11-pi4-gles3-b1b5b845-rows16-30`: rows 17-31 of `tools/cts/pi4-gles3-tree.txt` (15 images: LT M1 PR R1 SQ S01 S02 S03 S04 S05 S06 S07 S08 S09 S10).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 17 | LT | 55 | 1 | 0 | 0 | 1.5 s | 0 | none | DONE |
| 18 | M1 | 438 | 11 | 58 | 0 | 18.3 s | 0 | none | DONE |
| 19 | PR | 504 | 0 | 0 | 0 | 22.4 s | 0 | none | DONE, all passed |
| 20 | R1 | 257 | 16 | 26 | 4 | 17.1 s | 0 | none | DONE |
| 21 | SQ | 911 | 26 | 9 | 0 | 27.9 s | 0 | none | DONE |
| 22 | S01 | 861 | 0 | 0 | 0 | 46.9 s | 0 | none | DONE |
| 23 | S02 | 392 | 0 | 0 | 0 | 12.6 s | 0 | none | DONE, all passed |
| 24 | S03 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 25 | S04 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 26 | S05 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 27 | S06 | 0 | 0 | 0 | 1 | 0.3 s | 0 | none | incomplete (no complete deqp summary) |
| 28 | S07 | 0 | 0 | 0 | 0 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 29 | S08 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 30 | S09 | 1156 | 0 | 0 | 0 | 43.2 s | 0 | none | DONE, all passed |
| 31 | S10 | 605 | 85 | 0 | 0 | 39.3 s | 0 | none | DONE |

(The table's "row" column counts from 1; the rows of this session are rows 16-30 counted from 0, the `TREE_ROWS=16-30` of the recipe.)

## Result

Run 2026-11-13 22:59 to 2026-11-14 00:19 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item pi4-s2 of plan #320 (issue #1261), session 2 of the one-commit Pi 4 dEQP-GLES3 record: rows 16-30 of `tools/cts/pi4-gles3-tree.txt` (LT M1 PR R1 SQ S01 ... S10). 6,562 mustpass cases in the fifteen images, 5,420 with a result: **5,179 pass, 139 fail, 93 NotSupported, 4 QualityWarning, 5 ResourceError / other, 1,142 never began**. Nine images ended `complete=1` with `0 without` a result (LT M1 PR R1 SQ S01 S02 S09 S10); six aborted (S03-S08, below). 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 slow jobs, 0 resets before the end of any run (`pi4-console-scan.py`). No run came near its window: the longest is S01's 46.9 s of the 210 s record deadline. Every console ends with the image's own end line (`v3d_deqp: build b1b5b845 ...`), Linux was back 301-314 s after each reboot (`cycles.txt`) and every cycle read back as before.

**M1 (`pbo`, `multisample`, `negative_api`, `occlusion_query`, ...; row 17) ran through**: at `0ff58329` dEQP aborted it on a scissored or masked integer clear (`pbo.renderbuffer.rg16i_clears`); at `b1b5b845` it ends `DONE`, 524 cases begun, 524 with a result, `complete=1` (454 passed, 12 failed, 58 not supported in the image's own counters; `pbo.renderbuffer.rg16i_clears` passes). LT (`lifetime`) ends `DONE`: 55 pass, 1 Fail (`lifetime.attach.deleted_output.buffer_transform_feedback`), as at `0ff58329`.

**S03-S08 aborted on their first case**, as they did at `8990a5cc`. Each stops at its first `precision.<function>.highp_fragment.scalar` case (S03 `abs`, S04 `cosh`, S05 `fract`, S06 `mix`, S07 `reflect`, S08 `sub`): the back end prints `yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty`, the GL core reports `GL_OUT_OF_MEMORY` (`GL error 0x505 in glDrawArrays`, `yakogl: glDrawArrays was not recorded (-22)`), dEQP records a ResourceError (`glsShaderExecUtil.cpp:684`), `Test run was ABORTED!`, `complete=0`, and every later case of the image never began (1 case begun in each; the 1,142 `none` cases of `table.txt`). All six read `rc=0 verdict=done` on the board runner's terms (the end line is `STOPPED EARLY by dEQP (complete=0; later cases never began)`), so only the tally's `complete=0` shows the abort (`check-pi4-gles3-log.py` reports `aborted, no complete deqp summary (6): S03 S04 S05 S06 S07 S08`). No image overran or hung, so nothing was re-split here (recipe section 6, *for time*); isolating the refusing `highp_fragment` cases is a re-split *at the refusing case* and is left to the plan (the cases behind each abort run in no image yet). S07's refusing case is outside `gles3-mustpass.txt`, hence its 0 in the table.

### The non-Pass cases, by group

`table.txt` names every one. By group (mustpass view): `multisample` 4 Fail / 48 NotSupported (`default_framebuffer`, `fbo_4_samples`, `fbo_8_samples` all NotSupported, `fbo_max_samples` 4 Fail), `negative_api` 7 Fail / 4 NotSupported (`buffer.blit_framebuffer*`, `read_pixels_fbo_format_mismatch`, `shader.uniformui*_incompatible_type`, `texture.texsubimage2d/3d`; `fragment.*i` indexed blend), `pbo` 6 NotSupported (`renderbuffer` float formats), `rasterization` 2 Fail / 12 NotSupported / 4 QualityWarning (`rbo_multisample_4`, `rbo_multisample_max`, `interpolation.*.lines`), `samplers` 14 Fail (`single_tex_3d`, `multi_tex_3d`), `shader_api` 14 NotSupported (`program_binary`), `state_query` 26 Fail / 9 NotSupported (`integers` 17, `integers64` 6, `rbo` 1, `shader` 2; `indexed` 9), `lifetime` 1 Fail, and in `shaders` 85 Fail (S10: `derivate.*` on `fbo_msaa2/4` and float `texture` targets) plus the six aborts. S01 begun 1,309 cases, mustpass view 861 (its own 2 Fail, `constant_expressions.other.complex_fragment` / `complex_vertex`, are outside `gles3-mustpass.txt`). `occlusion_query` 62, `polygon_offset` 20, `primitive_restart` 504, `rasterizer_discard` 51, `read_pixels` 48, `vertex_array_objects` 10 and S02 / S09 (`builtin_functions.common` ..., `conversions`) pass in full. The causes are not read here: the failure groups go to the triage item.

## Method

A shared clone of the repository at `b1b5b845` outside the checkout (`git describe` there: `b1b5b845`). VK-GL-CTS 93bca01 with its externals, the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores), then the fifteen images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` (section 3): 15 `rc=0 build=b1b5b845` lines, every one `RAM: 161844 KB 256 MB 61.74%`, each `.config` passed by `check-deqp-images.py --config --gles3`.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time and the leased serial console (an MCP server that connected forwards) held that slot. As in `../2026-11-pi4-gles3-c/README.md`, the steps of `tools/board.sh pi4` were done by hand, in its order, one image at a time (execution order LT PR M1 R1 SQ S01 ... S10; one GL client on the board at a time): `sudo` and free-space prechecks over ssh, no leftover `yakogl-*.bin` and no `config-linux.txt`; the image installed to `/boot/firmware/yakogl-<LABEL>.bin` and `tryboot.txt` written with `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`, the panel's value was not in the environment), both read back (sha256, text) before the reboot; `sudo reboot "0 tryboot"`; the console through the leased serial console (`serial_expect` on the end line, then `serial_read` of the whole run, converted by a script, never copied by hand); Linux back (a new boot_id), the staged image removed and `tryboot.txt` put back to its previous one-newline content, read back. `config.txt` was not touched. The images' local window check (`board.sh pi4 run --dry-run`) was not run by hand; every image linked at 61.74 % of the 256 MiB window like the 54 that passed it in the earlier records.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of its boot to the `v3d_deqp: build b1b5b845 ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over the fifteen consoles, in the table's order.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the reboot time (unix seconds), the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line.
