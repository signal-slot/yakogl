<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rerun at fad0558a: the b1b5b845 images main has changed since

commit: fad0558a7def0e5677761608eec56b12e9855d18

Plan #380, step "the Pi 4's remaining GLES3 images" (issue #1261). The images of `tools/cts/pi4-gles3-tree.txt` whose `b1b5b845` result main has changed since (`../2026-11-18-pi4-gles3-ac2fe51f-map/README.md`), run again on the Raspberry Pi 4 from one commit, `fad0558a`. Every session of this rerun builds from this commit and adds its section below. Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line. "before" is the `b1b5b845` record (`../2026-11-pi4-gles3-b1b5b845-rows00-15/`, `-rows16-30/`, `-rows31-45/`), in the same view.

## Session 1 (G1)

Thirteen images of the table's A1..S31 part: A1 FB1 FB2 FB3 FB4 FR1 FR2 LT M1 R1 SQ S10 S16 (rows 1, 5-8, 15-18, 20, 21, 31, 37 counted from 1; `TREE_ROWS=0,4-7,14-17,19,20,30,36`).

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict | before (b1b5b845) P / F / N.S. / other | after P / F / N.S. / other | moved |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | A1 | 470 | 2 | 0 | 0 | 28.7 s | 0 | none | DONE | 461 / 11 / 0 / 0 | 470 / 2 / 0 / 0 | 9 Fail to Pass |
| 5 | FB1 | 561 | 12 | 7 | 0 | 17.5 s | 0 | none | DONE | 559 / 14 / 7 / 0 | 561 / 12 / 7 / 0 | 2 Fail to Pass |
| 6 | FB2 | 461 | 3 | 146 | 0 | 28.5 s | 0 | none | DONE | 445 / 19 / 146 / 0 | 461 / 3 / 146 / 0 | 16 Fail to Pass |
| 7 | FB3 | 212 | 81 | 36 | 0 | 18.6 s | 0 | none | DONE | 157 / 136 / 36 / 0 | 212 / 81 / 36 / 0 | 55 Fail to Pass |
| 8 | FB4 | 114 | 329 | 66 | 0 | 29.0 s | 0 | none | DONE | 108 / 335 / 66 / 0 | 114 / 329 / 66 / 0 | 6 Fail to Pass |
| 15 | FR1 | 372 | 0 | 48 | 0 | 13.5 s | 0 | none | DONE, none failed | 360 / 12 / 48 / 0 | 372 / 0 / 48 / 0 | 12 Fail to Pass |
| 16 | FR2 | 422 | 31 | 67 | 0 | 15.1 s | 0 | none | DONE | 422 / 31 / 67 / 0 | 422 / 31 / 67 / 0 | none |
| 17 | LT | 56 | 0 | 0 | 0 | 1.5 s | 0 | none | DONE, all passed | 55 / 1 / 0 / 0 | 56 / 0 / 0 / 0 | 1 Fail to Pass |
| 18 | M1 | 443 | 6 | 58 | 0 | 18.2 s | 0 | none | DONE | 438 / 11 / 58 / 0 | 443 / 6 / 58 / 0 | 5 Fail to Pass |
| 20 | R1 | 285 | 2 | 12 | 4 | 18.9 s | 0 | none | DONE | 257 / 16 / 26 / 4 | 285 / 2 / 12 / 4 | 14 Fail to Pass, 14 NotSupported to Pass |
| 21 | SQ | 931 | 6 | 9 | 0 | 27.9 s | 0 | none | DONE | 911 / 26 / 9 / 0 | 931 / 6 / 9 / 0 | 20 Fail to Pass |
| 31 | S10 | 620 | 70 | 0 | 0 | 39.3 s | 0 | none | DONE | 605 / 85 / 0 / 0 | 620 / 70 / 0 / 0 | 15 Fail to Pass |
| 37 | S16 | 720 | 0 | 0 | 0 | 32.0 s | 0 | none | DONE, all passed | 719 / 1 / 0 / 0 | 720 / 0 / 0 / 0 | 1 Fail to Pass |

(R1's "other" is 4 QualityWarning, as at `b1b5b845`.)

### Result

Run 2026-11-19 13:23 to 15:54 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item pi4-g1 of plan #380. 6,662 mustpass cases in the thirteen images, 6,662 with a result: **5,667 pass, 542 fail, 449 NotSupported, 4 QualityWarning, 0 without a result**. At `b1b5b845` the same images read 5,497 pass, 698 fail, 463 NotSupported, 4 QualityWarning: **170 cases moved to Pass (156 from Fail, 14 from NotSupported) and none moved from Pass.** All thirteen images ended `complete=1` with `0 without` a result; none aborted, so the table was not re-split. 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 slow jobs, 0 resets before the end of any run (`pi4-console-scan.py`: `13 image run(s): 13 finished, 0 without a finish line; recoveries 0, job timeouts 0, hung 0, MMU faults 0, resets 0, faults 0`). The longest run is S10's 39.3 s of the 210 s record deadline. Every console prints `v3d_deqp: build fad0558a` and ends with the image's own end line.

What moved, by image (`table.txt` names every case that is not Pass):

- **A1**: 9 `implementation_limits` cases pass (`compressed_texture_formats`, `num_compressed_texture_formats`, the varying, vertex-output, fragment-input, uniform-block and combined-uniform limits). Still Fail: `max_color_attachments`, `max_draw_buffers` (`Requirement not satisfied`).
- **FB1**: 2 of the 14 `fbo.completeness.renderable` cases pass; 12 still Fail (`Framebuffer checked is incomplete, expected complete`).
- **FB2**: the 16 `fbo.blit.depth_stencil` cases pass. Still Fail: 3 `fbo.blit.default_framebuffer`.
- **FB3**: 55 pass (`fbo.color.tex2darray` 20, `tex3d` 20, `fbo.depth.depth_test_clamp` 5, `depth_write_clamp` 5, `repeated_clear` 2, and one each of `blend`, `tex2d`, `texcube`). Still Fail 81: `repeated_clear` 31, `tex2d` / `tex2darray` / `tex3d` / `texcube` 12 each, `blend` 2.
- **FB4**: 6 pass (`fbo.invalidate.sub` 3, `fbo.invalidate.whole` 3). Still Fail 329: `fbo.render` 275 (`resize` 86, `shared_colorbuffer` 78, `recreate_color` 61, `shared_colorbuffer_clear` 26, `recreate_depth_stencil` 11, `shared_depth_stencil` 10, `stencil_clear` 3), `fbo.msaa` 34, `fbo.invalidate` 20. The rerun map expected 12 to move (the six `fbo.render.*.tex2d_rgb16f*` cases as well): on the board 6 did.
- **FR1**: the 12 `fragment_out.array.float` Fails pass; the image has no Fail. The rerun map read FR1 as "no change expected".
- **FR2**: unchanged, 31 `fragment_out.random` Fails.
- **LT**: `lifetime.attach.deleted_output.buffer_transform_feedback` passes; all 58 cases begun pass (56 in the mustpass view). The four `deleted_{name,output}.{texture,renderbuffer}_framebuffer` cases that regressed on main at `4255adf1` pass (fixed by `0db66553`, in this commit).
- **M1**: 5 `negative_api` cases pass (`buffer.blit_framebuffer`, `shader.uniformui_incompatible_type`, `shader.uniformuiv_incompatible_type`, `texture.texsubimage2d`, `texture.texsubimage3d`). Still Fail 6: `multisample.fbo_max_samples` 4, `negative_api.buffer.blit_framebuffer_multisample`, `negative_api.buffer.read_pixels_fbo_format_mismatch`. `pbo.renderbuffer.rg16i_clears` passes, as at `b1b5b845`.
- **R1**: the 14 `samplers.single_tex_3d` / `multi_tex_3d` Fails pass, and 14 NotSupported cases now run and pass. Still Fail 2 (`rasterization.fbo.rbo_multisample_max`), 4 QualityWarning.
- **SQ**: 20 pass (`state_query.integers` 11, `integers64` 6, `rbo.renderbuffer_component_size_color`, `shader.program_active_uniform_types`, `shader.program_binary`). Still Fail 6: `max_color_attachments`, `max_draw_buffers` and `max_samples`, each `_getfloat` and `_getinteger64`.
- **S10**: 15 `shaders.fragdepth` cases pass (`compare` 8, `write` 7). Still Fail 70: `shaders.derivate` (`dfdx` 22, `dfdy` 26, `fwidth` 22).
- **S16**: `shaders.metamorphic.synthetic.variant_3` passes with the images' 8 MiB of compiler scratch (`f2fb64c9`); the image has no Fail.

No case fails for a cause that `b1b5b845` did not already show: every Fail above was a Fail there. The causes are not read here.

### The board

Thirteen tryboot cycles, one image each. Before the first cycle the board was in Linux with `tryboot.txt` holding one newline and no staged image. After every cycle Linux came back with a new boot_id (316-350 s after the reboot; A1's 465 s in `cycles.txt` is the time the session first asked, not the time the board answered), the staged `yakogl-<LABEL>.bin` was removed and `tryboot.txt` put back to its one-newline content, both read back. After the last cycle (S16) the board was read once more: Linux up with a new boot_id, no `yakogl-*.bin`, `pi4run-*.bin` or `config-linux.txt` under `/boot/firmware`, `tryboot.txt` one newline (the sha256 read before the first cycle), `config.txt` with the sha256 read before the first cycle. `config.txt` was never written. Nothing else on the board was changed: no partition, no U-Boot variable, no kernel module, no service.

Twice the first ssh connection after a cycle timed out (before FR1's and before LT's staging, nothing staged yet); the next attempt answered and the cycle went on. No cycle was repeated.

### Method

Built in a checkout of `fad0558a` (`git describe`: `fad0558a`, no tracked change): VK-GL-CTS 93bca01 with its externals, the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores, 453 ninja steps in 263 s), then the thirteen images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` from a table holding only these images' `# image` and filter lines, copied from `tools/cts/pi4-gles3-tree.txt`. 13 `rc=0 build=fad0558a` lines, every image `RAM: 166356 KB 256 MB 63.46%` (4,512 KB over the `b1b5b845` images: the 8 MiB compiler scratch of `f2fb64c9`), each `.config` passed by `check-deqp-images.py --config --gles3`.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time and the leased serial console held that slot. As in `../2026-11-pi4-gles3-b1b5b845-rows16-30/README.md`, the steps of `tools/board.sh pi4` were done by hand, in its order, one image at a time, by one script per step: the image checks of `nb_check_image` (the AArch64 header, the size inside the 256 MiB window, the `board: PM watchdog resets` marker); `sudo` and free-space prechecks over ssh, no leftover `yakogl-*.bin` and no `config-linux.txt`; the undo printed; the image installed to `/boot/firmware/yakogl-<LABEL>.bin` and `tryboot.txt` written with `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`, the panel's value was not in the environment), both read back (sha256, text) before the reboot; `sudo reboot "0 tryboot"`; Linux back (a new boot_id), the staged image removed and `tryboot.txt` put back, read back; then the console of that boot read from the leased serial console into files and converted by a script, never copied by hand. The image went to the board xz-compressed and was unpacked there (the link to the board was slow); the sha256 read-back is of the unpacked file in `/boot/firmware`. `board.sh`'s lease file under `/var/tmp` was not written (the board was leased to this session). Since `board.sh` did not run, there are no `board.sh pi4 <LABEL> rc=... verdict=...` lines; the verdicts are the end lines in `cycles.txt` and the scan's.

Checks: `tools/cts/pi4-session-tally.sh` (13 tallies), `pi4-console-scan.py` (above), `check-pi4-gles3-log.py --commit fad0558a7def0e5677761608eec56b12e9855d18` on this directory (no duplicate, no abort, the commit line; it lists as missing the labels this session did not run), `check-gles3-tree.py tools/cts/pi4-gles3-tree.txt` (`100 images, 42810 matched once`).

## Not in this session

- Later sessions of this rerun, from the same commit: S32a, S32b, S32c1-S32c4, S32d, T02, T03, T04, F3C1-F3C4, F3F1-F3F4, F3S1, F3S2 (session 2); T05a-T05d, T06-T13 (session 3); T17, UB1-UB3, UA1-UA3 (session 4).
- Plan #376's images: S03-S08, T14, TF1, TF2.
- No change expected (their `b1b5b845` result stands): the other images of the table, among them B1, C1, D1, FO1-FO6, PR, S01, S02, S09, S11-S15, S17-S27, S5f, S5v, S30, S31 of the A1..S31 part.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of its boot to the `v3d_deqp: build fad0558a ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over the thirteen consoles, in the table's order.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the reboot time (unix seconds), the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line.
