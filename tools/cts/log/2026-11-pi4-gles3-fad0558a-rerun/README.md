<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rerun at fad0558a: the b1b5b845 images main has changed since

commit: fad0558a7def0e5677761608eec56b12e9855d18

Plan #380, step "the Pi 4's remaining GLES3 images" (issue #1261). The images of `tools/cts/pi4-gles3-tree.txt` whose `b1b5b845` result main has changed since (`../2026-11-18-pi4-gles3-ac2fe51f-map/README.md`), run again on the Raspberry Pi 4 from one commit, `fad0558a`. Every session of this rerun builds from this commit and adds its section below. Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line. "before" is the `b1b5b845` record (`../2026-11-pi4-gles3-b1b5b845-rows00-15/`, `-rows16-30/`, `-rows31-45/`, `-rows46-60/`, `-rows61-75/`), in the same view.

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

## Session 2 (G2)

Eighteen images of the table's S32a..F3S2 part: S32a S32b S32c1 S32c2 S32c3 S32c4 S32d T03 F3C1 F3C2 F3C3 F3C4 F3F1 F3F2 F3F3 F3F4 F3S1 F3S2 (rows 53-59, 62, 64-71, 73, 74 counted from 1; `TREE_ROWS=52-58,61,63-70,72,73`). S32c ran as the four one-case images S32c1-S32c4 of the table; no image of this session was re-split.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict | before (b1b5b845) P / F / N.S. / other / without | after P / F / N.S. / other / without | moved |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 53 | S32a | 469 | 53 | 0 | 0 | 39.9 s | 0 | none | DONE | 262 / 260 / 0 / 0 / 0 | 469 / 53 / 0 / 0 / 0 | 207 Fail to Pass |
| 54 | S32b | 0 | 34 | 0 | 0 | 7.3 s | 0 | none | DONE | 0 / 34 / 0 / 0 / 0 | 0 / 34 / 0 / 0 / 0 | none (18 change cause, below) |
| 55 | S32c1 | 0 | 1 | 0 | 0 | 0.2 s | 0 | none | DONE | 0 / 0 / 0 / 1 / 0 (the refused case of the abort) | 0 / 1 / 0 / 0 / 0 | aborted to Fail |
| 56 | S32c2 | 0 | 1 | 0 | 0 | 0.2 s | 0 | none | DONE | 0 / 0 / 0 / 0 / 1 (never began) | 0 / 1 / 0 / 0 / 0 | no result to Fail |
| 57 | S32c3 | 0 | 1 | 0 | 0 | 0.2 s | 0 | none | DONE | 0 / 0 / 0 / 0 / 1 (never began) | 0 / 1 / 0 / 0 / 0 | no result to Fail |
| 58 | S32c4 | 0 | 1 | 0 | 0 | 0.2 s | 0 | none | DONE | 0 / 0 / 0 / 0 / 1 (never began) | 0 / 1 / 0 / 0 / 0 | no result to Fail |
| 59 | S32d | 118 | 0 | 0 | 0 | 5.1 s | 0 | none | DONE, all passed | 116 / 2 / 0 / 0 / 0 | 118 / 0 / 0 / 0 / 0 | 2 Fail to Pass |
| 62 | T03 | 122 | 0 | 12 | 0 | 26.2 s | 0 | none | DONE, none failed | 0 / 122 / 12 / 0 / 0 | 122 / 0 / 12 / 0 / 0 | 122 Fail to Pass |
| 64 | F3C1 | 14 | 0 | 0 | 0 | 2.1 s | 0 | none | DONE, all passed | 0 / 14 / 0 / 0 / 0 | 14 / 0 / 0 / 0 / 0 | 14 Fail to Pass |
| 65 | F3C2 | 14 | 0 | 0 | 0 | 2.1 s | 0 | none | DONE, all passed | 0 / 14 / 0 / 0 / 0 | 14 / 0 / 0 / 0 / 0 | 14 Fail to Pass |
| 66 | F3C3 | 14 | 0 | 0 | 0 | 2.0 s | 0 | none | DONE, all passed | 0 / 14 / 0 / 0 / 0 | 14 / 0 / 0 / 0 / 0 | 14 Fail to Pass |
| 67 | F3C4 | 14 | 0 | 0 | 0 | 2.0 s | 0 | none | DONE, all passed | 0 / 14 / 0 / 0 / 0 | 14 / 0 / 0 / 0 / 0 | 14 Fail to Pass |
| 68 | F3F1 | 15 | 0 | 0 | 0 | 2.7 s | 0 | none | DONE, all passed | 0 / 15 / 0 / 0 / 0 | 15 / 0 / 0 / 0 / 0 | 15 Fail to Pass |
| 69 | F3F2 | 15 | 0 | 0 | 0 | 3.7 s | 0 | none | DONE, all passed | 0 / 15 / 0 / 0 / 0 | 15 / 0 / 0 / 0 / 0 | 15 Fail to Pass |
| 70 | F3F3 | 15 | 0 | 0 | 0 | 3.7 s | 0 | none | DONE, all passed | 0 / 15 / 0 / 0 / 0 | 15 / 0 / 0 / 0 / 0 | 15 Fail to Pass |
| 71 | F3F4 | 14 | 0 | 1 | 0 | 2.5 s | 0 | none | DONE, none failed | 0 / 14 / 1 / 0 / 0 | 14 / 0 / 1 / 0 / 0 | 14 Fail to Pass |
| 73 | F3S1 | 13 | 0 | 0 | 0 | 1.6 s | 0 | none | DONE, all passed | 0 / 13 / 0 / 0 / 0 | 13 / 0 / 0 / 0 / 0 | 13 Fail to Pass |
| 74 | F3S2 | 13 | 0 | 0 | 0 | 1.4 s | 0 | none | DONE, all passed | 0 / 13 / 0 / 0 / 0 | 13 / 0 / 0 / 0 / 0 | 13 Fail to Pass |

("before" of S32c1-S32c4 is the one `S32c` record of `../2026-11-pi4-gles3-b1b5b845-rows46-60/`, case by case: S32c aborted at its first case, a refused draw, and the other three never began.)

### Result

Run 2026-11-20 05:01 to 07:12 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item pi4-g2 of plan #380. 954 mustpass cases in the eighteen images, 954 with a result: **850 pass, 91 fail, 13 NotSupported, 0 without a result**. At `b1b5b845` the same cases read 378 pass, 559 fail, 13 NotSupported, 1 refused in an aborted run and 3 without a result: **472 cases moved from Fail to Pass, the four S32c cases now have a result (Fail), and none moved from Pass.** All eighteen images ended `complete=1` with `0 without` a result; none aborted, so the table was not re-split. 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 slow jobs, 0 resets before the end of any run (`pi4-console-scan.py`: `18 image run(s): 18 finished, 0 without a finish line; recoveries 0, job timeouts 0, hung 0, MMU faults 0, resets 0, faults 0`). The longest run is S32a's 39.9 s of the 210 s record deadline. Every console prints `v3d_deqp: build fad0558a` and ends with the image's own end line.

What moved, by image (`table.txt` names every case that is not Pass):

- **S32a**: 207 pass (the texelFetch, offset, 2D-array / 3D and shadow look-ups that the compiler refused at `b1b5b845`). Still not Pass 53, two causes:
  - 32 draws refused by the job builder (`v3d_deqp` letter R, `glGetError() returned GL_INVALID_OPERATION at glsShaderRenderCase.cpp:742`): `texturegrad`, `texturegradoffset`, `textureprojgrad`, `textureprojgradoffset` of `sampler3d_fixed`, `sampler3d_float`, `isampler3d`, `usampler3d`, vertex and fragment (8 each). These are the 32 of the uniform words 46/47 that the rerun map expected to stay (its issue draft 1).
  - 21 Fail with no refusal (`Fail (Fail)`, a wrong image): shadow look-ups with an explicit level or gradient, and the 2D-array shadow `texture`: `texturegrad.sampler2dshadow_*` 2, `texturegrad.sampler2darrayshadow_*` 2, `texturegradoffset.sampler2dshadow_*` 2, `texturegradoffset.sampler2darrayshadow_*` 2, `texturelod.sampler2dshadow_*` 2, `texturelodoffset.sampler2dshadow_*` 2, `textureprojgrad.sampler2dshadow_*` 2, `textureprojgradoffset.sampler2dshadow_*` 2, `textureprojlod.sampler2dshadow_*` 2, `textureprojlodoffset.sampler2dshadow_*` 2, `texture.sampler2darrayshadow_fragment` 1. Outside the mustpass the image also fails `texture.sampler2darrayshadow_vertex` and `texturegrad.samplercubeshadow_{fragment,vertex}` the same way. New on the board: at `b1b5b845` these cases failed to compile (`Failed to compile shader program at glsShaderRenderCase.cpp:447`); they now compile and draw and read wrong. The cause is not read here.
- **S32b**: unchanged, 34 Fail. The 16 `textureSize` cases of 2D and cube samplers (`isampler2d`, `usampler2d`, `sampler2d_{fixed,float}`, `isamplercube`, `usamplercube`, `samplercube_{fixed,float}`, vertex and fragment) draw with no refusal and fail `Got unexpected texture size`, as at `b1b5b845`. The 18 of 3D and 2D-array samplers (`sampler2darrayshadow_*` included) are now draws refused by the job builder (letter R, 324 refused draws, `the job builder refused the draw (a uniform, program or primitive it does not write)`) where at `b1b5b845` they were `Shader failed`: the rerun map's "the cause has moved" (issue draft 1). The verdict text of all 34 is `Got unexpected texture size`.
- **S32c1-S32c4**: the RET16_OF_32 refusal is gone (`080240bc`): each of `texturesize.sampler2dshadow_{fragment,vertex}` and `samplercubeshadow_{fragment,vertex}` draws with no refusal and ends `complete=1`. Each fails `Got unexpected texture size`, the verdict of S32b's 16 non-refused 2D and cube cases: 20 `textureSize` cases of 2D, cube and their shadow samplers now read a wrong size on the board with no refusal.
- **S32d**: the two `long_instance_name` Fails pass (reflection names of 63+ characters); the image has no Fail.
- **T03**: the 122 Fails pass (`texture.filtering.2d_array`: 2D-array look-ups the compiler refused at `b1b5b845`); the 12 NotSupported (`formats.srgb_r8_*`, `srgb_rg8_*`: `GL_EXT_texture_sRGB_R8` / `_RG8` not advertised) stay.
- **F3C1-F3C4, F3F1-F3F4, F3S1, F3S2**: every `texture.filtering.3d` case of these images passes (141; F3F4's `formats.srgb_r8_linear` stays NotSupported).

No case moved from Pass. The two Fail classes left without a refusal (S32a's 21 shadow explicit-level look-ups, the 20 `textureSize` cases of S32b and S32c1-S32c4) are named in the report's issue drafts; their causes are not read on the board here.

### The board

Eighteen tryboot cycles, one image each. Before the first cycle the board was in Linux with `tryboot.txt` holding one newline (sha256 `01ba4719...`), `config.txt` sha256 `6ea8eaef...`, and no staged image. After every cycle Linux came back with a new boot_id (298-340 s after the reboot), the staged `yakogl-<LABEL>.bin` was removed and `tryboot.txt` put back to its one-newline content, both read back, and the sha256 of `tryboot.txt` and `config.txt` read again equal to the values before the first cycle. After the last cycle (F3S2) the board was read once more: Linux up, no `yakogl-*.bin`, `pi4run-*.bin` or `config-linux.txt` under `/boot/firmware`, `tryboot.txt` one newline and `config.txt` with the sha256 read before the first cycle. `config.txt` was never written. Nothing else on the board was changed: no partition, no U-Boot variable, no kernel module, no service. No ssh connection timed out and no cycle was repeated.

### Method

Built in a worktree of `fad0558a` (`git describe --always --dirty`: `fad0558a`): VK-GL-CTS 93bca01 with its externals (a shallow fetch of that commit), the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores, 453 ninja steps), then the eighteen images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` from a table holding only these images' `# image` and filter lines, copied from `tools/cts/pi4-gles3-tree.txt`. 18 `rc=0 build=fad0558a` lines, every image `RAM: 166356-166364 KB 256 MB 63.46%`, each `.config` passed by the builder's `check-deqp-images.py --config --gles3` line.

As in session 1, the leased serial console held the UART bridge's one client slot, so the steps of `tools/board.sh pi4` were done by hand in its order, one image at a time, by one script per step: the image checks of `nb_check_image` (the AArch64 header, `image_size` inside the 256 MiB window, the `board: PM watchdog resets` marker) and the `.built` line's sha256 and build; `sudo`, free-space and leftover prechecks over ssh (no `yakogl-*.bin` / `pi4run-*.bin`, no `config-linux.txt`, `tryboot.txt` the one-newline stock); the undo printed; the image sent xz-compressed, unpacked on the board and installed to `/boot/firmware/yakogl-<LABEL>.bin`; `tryboot.txt` written with `pi4_stage`'s lines (its comment line, `[all]`, `kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`, the panel's value was not in the environment); both read back (sha256, text) before the reboot; `sudo reboot "0 tryboot"`; the end line awaited on the leased serial console (`serial_expect`), the console read from it (`serial_read`) and cut by a script from the first firmware line after the reboot to the end line, CRs and NULs dropped, never copied by hand; Linux back (a new boot_id), the staged image removed and `tryboot.txt` put back, read back. `board.sh`'s lease file was not written; there are no `board.sh pi4 <LABEL> rc=... verdict=...` lines, the verdicts are the end lines in `cycles.txt` and the scan's.

Checks: `tools/cts/pi4-session-tally.sh` (31 tallies; session 1's thirteen read back unchanged), `pi4-console-scan.py` over the eighteen consoles (above), `check-pi4-gles3-log.py --commit fad0558a7def0e5677761608eec56b12e9855d18` on this directory (`100 images, 31 with a tally, 69 missing, 0 duplicated, 0 aborted, commit fad0558a`: the missing are the images of sessions 3 and 4, plan #376's and those not rerun), `check-gles3-tree.py tools/cts/pi4-gles3-tree.txt` (`100 images, 42810 matched once`).

## Session 3 (G3)

Twelve images of the table's T05a..T13 part: T05a T05b T05c T05d T06 T07 T08 T09 T10 T11 T12 T13 (rows 76-87 counted from 1; `TREE_ROWS=75-86`), every image of that part that the rerun map marks "rerun" (T05d included: its 8 ETC2 cube cases were NotSupported at `b1b5b845`). No image of this session aborted, so the table was not re-split.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict | before (b1b5b845) P / F / N.S. / other / without | after P / F / N.S. / other / without | moved |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 76 | T05a | 190 | 2 | 4 | 0 | 11.2 s | 0 | none | DONE | 142 / 34 / 20 / 0 / 0 | 190 / 2 / 4 / 0 / 0 | 32 Fail to Pass, 16 NotSupported to Pass |
| 77 | T05b | 12 | 0 | 0 | 0 | 0.5 s | 0 | none | DONE, all passed | 0 / 4 / 8 / 0 / 0 | 12 / 0 / 0 / 0 / 0 | 4 Fail to Pass, 8 NotSupported to Pass |
| 78 | T05c | 4 | 0 | 0 | 0 | 0.5 s | 0 | none | DONE, all passed | 0 / 0 / 0 / 1 / 3 (aborted) | 4 / 0 / 0 / 0 / 0 | aborted to Pass (1 ResourceError, 3 never began) |
| 79 | T05d | 8 | 0 | 0 | 0 | 1.0 s | 0 | none | DONE, all passed | 0 / 0 / 8 / 0 / 0 | 8 / 0 / 0 / 0 / 0 | 8 NotSupported to Pass |
| 80 | T06 | 308 | 6 | 12 | 0 | 38.3 s | 0 | none | DONE | 106 / 208 / 12 / 0 / 0 | 308 / 6 / 12 / 0 / 0 | 202 Fail to Pass |
| 81 | T07 | 196 | 28 | 0 | 0 | 37.8 s | 0 | none | DONE | 140 / 84 / 0 / 0 / 0 | 196 / 28 / 0 / 0 / 0 | 56 Fail to Pass |
| 82 | T08 | 0 | 288 | 0 | 0 | 40.3 s | 0 | none | DONE | 0 / 288 / 0 / 0 / 0 | 0 / 288 / 0 / 0 / 0 | none (288 change cause, below) |
| 83 | T09 | 72 | 150 | 0 | 0 | 97.5 s | 0 | none | DONE | 72 / 150 / 0 / 0 / 0 | 72 / 150 / 0 / 0 / 0 | none (144 change cause, below) |
| 84 | T10 | 327 | 7 | 0 | 0 | 84.6 s | 0 | none | DONE | 212 / 122 / 0 / 0 / 0 | 327 / 7 / 0 / 0 / 0 | 115 Fail to Pass |
| 85 | T11 | 319 | 1 | 0 | 0 | 34.4 s | 0 | none | DONE | 184 / 136 / 0 / 0 / 0 | 319 / 1 / 0 / 0 / 0 | 135 Fail to Pass |
| 86 | T12 | 194 | 3 | 0 | 0 | 64.0 s | 0 | none | DONE | 119 / 78 / 0 / 0 / 0 | 194 / 3 / 0 / 0 / 0 | 75 Fail to Pass |
| 87 | T13 | 306 | 6 | 0 | 0 | 32.8 s | 0 | none | DONE | 180 / 132 / 0 / 0 / 0 | 306 / 6 / 0 / 0 / 0 | 126 Fail to Pass |

("before" of T05c is `../2026-11-pi4-gles3-b1b5b845-rows61-75/T05c.tally.txt`: dEQP stopped at `etc2_eac_rgba8_cube_npot`, a refused draw counted as ResourceError, `complete=0`, and the other three cases never began. T09's 6 `size.2d.2048x2048_*` are letter R, as at `b1b5b845`.)

### Result

Run 2026-11-20 12:42 to 14:18 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item pi4-g3 of plan #380. 2,443 mustpass cases in the twelve images, 2,443 with a result: **1,936 pass, 491 fail, 16 NotSupported, 0 without a result**. At `b1b5b845` the same cases read 1,155 pass, 1,236 fail, 48 NotSupported, 1 refused in an aborted run and 3 without a result: **781 cases moved to Pass (745 from Fail, 32 from NotSupported, T05c's 4 from the abort) and none moved from Pass** (case by case over the result lines of the two consoles). All twelve images ended `complete=1` with `0 without` a result; none aborted. 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 slow jobs, 0 resets before the end of any run (`pi4-console-scan.py`: `12 image run(s): 12 finished, 0 without a finish line; recoveries 0, job timeouts 0, hung 0, MMU faults 0, resets 0, faults 0`). The longest run is T09's 97.5 s of the 210 s record deadline. Every console prints `v3d_deqp: build fad0558a` and ends with the image's own end line.

What moved, by image (`table.txt` names every case that is not Pass):

- **T05a**: 32 Fail and 16 NotSupported pass (the EAC and ETC1 formats, `455c3bb5`, and the sized 2D formats). Still Fail: `sized.2d.rgba32f_{npot,pot}`; still NotSupported: `sized.2d.srgb_r8_*`, `srgb_rg8_*` (`GL_EXT_texture_sRGB_R8` / `_RG8` not advertised).
- **T05b, T05c, T05d**: every ETC2 / EAC 2D and cube case passes (ETC2 on the V3D, plan #354). T05c no longer aborts at `etc2_eac_rgba8_cube_npot`. T05c and T05d can be merged back into one image.
- **T06**: 202 pass (2D-array, 3D and cube look-ups the compiler refused at `b1b5b845`). Still Fail 6: `sized.{2d_array,3d,cube}.rgba32f_*` (`Image comparison failed`); the 12 sRGB R8/RG8 NotSupported stay.
- **T07**: 56 pass. Still Fail 28, all `Image verification failed`: `mipmap.2d.{base_level,min_lod,max_lod}` 12 and `mipmap.cube.{base_level,min_lod}` 8, as at `b1b5b845`; and `mipmap.3d.{min_lod,max_lod}` 8, new on the board: at `b1b5b845` they failed to compile (`glsTextureTestUtil.cpp:411`); they now draw and read wrong. (`mipmap.3d.base_level` and `mipmap.cube.max_lod`, 4 cases each, fail too, with `Image verification failed`; they are outside `tools/cts/gles3-mustpass.txt`, so the mustpass tally does not count them. Same cause: the texture record carries base level 0 and the sampler LOD -1000/1000, `src/v3d/texture.rs`.) The rerun map expected 64 to move; 56 did.
- **T08**: unchanged count, 288 Fail, cause changed: every `texture.shadow.2d.*` and `shadow.2d_array.*` case compiled to `Program(NotYet("a shadow look-up"))` at `b1b5b845`; it now compiles and draws with no refusal and fails `Image verification failed`, the `always` and `never` compare functions included. The rerun map expected all 288 to pass (the host passes them).
- **T09**: unchanged count, 150 Fail, cause changed the same way for the 144 `texture.shadow.cube.*` cases (now `Image verification failed`, no refusal). The 6 `size.2d.2048x2048_*` cases stay letter R (`Image comparison failed`), as at `b1b5b845`.
- **T10**: 115 pass. Still Fail 7: `basic_teximage2d.rgba32f_{2d,cube}`, `basic_teximage3d.rgba32f_{2d_array,3d}`, `basic_texsubimage2d.rgba32f_{2d,cube}`, `basic_texsubimage3d.rgba32f_3d`.
- **T11**: 135 pass. Still Fail 1: `teximage2d_pbo.rgba32f_cube`.
- **T12**: 75 pass. Still Fail 3: `texstorage2d.format.rgba32f_{2d,cube}`, `texstorage3d.format.rgba32f_3d` (`rgba32f_2d_array` fails outside the mustpass view too).
- **T13**: 126 pass. Still Fail 6: `texsubimage2d_pbo.rgba32f_{2d,cube}`, `texsubimage2d_unpack_params.rgba32f_complex`, `texsubimage3d_pbo.rgba32f_{2d_array,3d}`, `texsubimage3d_unpack_params.rgba32f_complex`.

The 25 `rgba32f` Fails of T05a, T06, T10-T13 are one class: GL_RGBA32F stored as RGBA16F halves on the V3D path at `fad0558a`, the class `tests/es3_rgba32f_v3d.rs` pins at this commit (its header names T05a's and T06's eight; T10-T13's 17 are the same format through the specification paths). The 432 shadow Fails of T08 and T09 and T07's 8 `mipmap.3d` LOD Fails are new on the board: refusals at `b1b5b845`, wrong images now, with no refusal line; they join S32a's 21 shadow explicit-level Fails of session 2. Their causes are not read here (issue drafts in the report of item pi4-g3).

### The board

Twelve tryboot cycles, one image each. Before the first cycle the board was in Linux with `tryboot.txt` holding one newline (sha256 `01ba4719...`), `config.txt` sha256 `6ea8eaef...`, and no staged image. After every cycle Linux came back with a new boot_id (308-413 s after the reboot), the staged `yakogl-<LABEL>.bin` was removed and `tryboot.txt` put back, and `ls` / `sha256sum` read back no `yakogl-*.bin`, `pi4run-*.bin` or `config-linux.txt` under `/boot/firmware` and `tryboot.txt` and `config.txt` equal to the values before the first cycle. After the last cycle (T13) the board was in Linux, stock. `config.txt` was never written. Nothing else on the board was changed: no partition, no U-Boot variable, no kernel module, no service. No ssh connection timed out and no cycle was repeated.

### Method

Built in a worktree of `fad0558a` (`git describe --always --dirty`: `fad0558a`): VK-GL-CTS 93bca01 with its externals (`deps-env.sh --build --board rpi_4b DEQP_WORK`), the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores), then the twelve images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` from a table holding only these images' `# image` and filter lines, copied from `tools/cts/pi4-gles3-tree.txt`. 12 `rc=0 build=fad0558a` lines, every image `RAM: 166356 KB 256 MB 63.46%`, each `.config` passed by the builder's `check-deqp-images.py --config --gles3` line.

As in sessions 1 and 2, the leased serial console held the UART bridge's one client slot, so the steps of `tools/board.sh pi4` were done by hand in its order, one image at a time, by one script per step: the image checks of `nb_check_image` (the AArch64 header, `image_size` inside the 256 MiB window, the `board: PM watchdog resets` marker) and the `.built` line's sha256 and build; `sudo`, free-space and leftover prechecks over ssh (no `yakogl-*.bin` / `pi4run-*.bin`, no `config-linux.txt`, `tryboot.txt` and `config.txt` at their stock sha256); the undo printed; the image sent xz-compressed, unpacked on the board and installed to `/boot/firmware/yakogl-<LABEL>.bin`; `tryboot.txt` written with `pi4_stage`'s lines (its comment line, `[all]`, `kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`, the panel's value was not in the environment); both read back (sha256, text) before the reboot; `sudo reboot "0 tryboot"`; the end line awaited on the leased serial console; the console read from it in 256 KiB pieces (`serial_read`, offsets contiguous) and cut by a script from the first line after the kernel's `reboot: Restarting system` line to the end line, the console's `\xNN` escapes decoded, CRs and NULs dropped, never copied by hand (the script also checks that the firmware loaded `yakogl-<LABEL>.bin`); Linux back (a new boot_id), the staged image removed and `tryboot.txt` put back, read back. `board.sh`'s lease file was not written; there are no `board.sh pi4 <LABEL> rc=... verdict=...` lines, the verdicts are the end lines in `cycles.txt` and the scan's. The raw reads were written outside the tree (`records/pi4/`, gitignored) in the task's own workspace. No copy was found after the turn ended (the next turn searched its workspace and the deps cache), so, as for sessions 1 and 2, the committed consoles are the only record of these runs.

Checks: `tools/cts/pi4-session-tally.sh` (43 tallies; sessions 1 and 2's thirty-one read back unchanged), `pi4-console-scan.py` over the twelve consoles (above), `check-pi4-gles3-log.py --commit fad0558a7def0e5677761608eec56b12e9855d18` on this directory (`100 images, 43 with a tally, 57 missing, 0 duplicated, 0 aborted, commit fad0558a`: the missing are the images of session 4, plan #376's and those not rerun), `check-gles3-tree.py tools/cts/pi4-gles3-tree.txt` (`100 images, 42810 matched once`).

## Not in this session

- The last session of this rerun, from the same commit: T17, UB1-UB3, UA1-UA3 (session 4).
- Plan #376's images: S03-S08, T14, TF1, TF2.
- No change expected (their `b1b5b845` result stands): the other images of the table, among them B1, C1, D1, FO1-FO6, PR, S01, S02, S09, S11-S15, S17-S27, S5f, S5v, S30, S31 of the A1..S31 part and T01, T02, T04, F3F5 of the S32a..F3S2 part (the rerun map's verdict; session 1 had listed T02 and T04 under session 2, and session 2 did not run them).

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of its boot to the `v3d_deqp: build fad0558a ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over each session's consoles, in the table's order: session 1's thirteen, then (after a `# Session 2 (G2)` line) session 2's eighteen, then (after a `# Session 3 (G3)` line) session 3's twelve.
- `cycles.txt`: per session (`== cycles, session 2 (G2)` starts the second, `== cycles, session 3 (G3)` the third) and per image the sha256 of `zephyr.bin`, the reboot time (unix seconds), the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line.
