<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 65-82 (F3F1 F3F2 F3F3 F3F4 F3F5 F3S1 F3S2 T05a T05b T05c T05d T06 T07 T08 T09 T10 T11 T12) at b1b5b845

commit: b1b5b8456ec69247c64d1bd07e42461266dc5136

Session `2026-11-pi4-gles3-b1b5b845-rows61-75`: rows 65-82 of `tools/cts/pi4-gles3-tree.txt` (18 images: F3F1 F3F2 F3F3 F3F4 F3F5 F3S1 F3S2 T05a T05b T05c T05d T06 T07 T08 T09 T10 T11 T12).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 65 | F3F1 | 0 | 15 | 0 | 0 | 1.9 s | 0 | none | DONE |
| 66 | F3F2 | 0 | 15 | 0 | 0 | 2.8 s | 0 | none | DONE |
| 67 | F3F3 | 0 | 15 | 0 | 0 | 1.7 s | 0 | none | DONE |
| 68 | F3F4 | 0 | 14 | 1 | 0 | 1.7 s | 0 | none | DONE |
| 69 | F3F5 | 0 | 0 | 11 | 0 | 0.6 s | 0 | none | DONE, none failed |
| 70 | F3S1 | 0 | 13 | 0 | 0 | 0.9 s | 0 | none | DONE |
| 71 | F3S2 | 0 | 13 | 0 | 0 | 0.8 s | 0 | none | DONE |
| 72 | T05a | 142 | 34 | 20 | 0 | 8.0 s | 0 | none | DONE |
| 73 | T05b | 0 | 4 | 8 | 0 | 0.6 s | 0 | none | DONE |
| 74 | T05c | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 75 | T05d | 0 | 0 | 8 | 0 | 0.4 s | 0 | none | DONE, none failed |
| 76 | T06 | 106 | 208 | 12 | 0 | 19.8 s | 0 | none | DONE |
| 77 | T07 | 140 | 84 | 0 | 0 | 25.6 s | 0 | none | DONE |
| 78 | T08 | 0 | 288 | 0 | 0 | 14.4 s | 0 | none | DONE |
| 79 | T09 | 72 | 150 | 0 | 0 | 18.3 s | 0 | none | DONE |
| 80 | T10 | 212 | 122 | 0 | 0 | 49.7 s | 0 | none | DONE |
| 81 | T11 | 184 | 136 | 0 | 0 | 21.0 s | 0 | none | DONE |
| 82 | T12 | 119 | 78 | 0 | 0 | 29.2 s | 0 | none | DONE |

Run 2026-11-15 on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item pi4-s5 of plan #320 (issue #1261): session 5b, the images F3F1..T12 of tools/cts/pi4-gles3-tree.txt (rows 64-78 counted from 0 in the 94-image table that pi4-s4 left; the session was planned as TREE_ROWS 61-75 of the 91-image table). T05 was re-split into T05a-d after it aborted (below), so the table is 97 images and these are rows 64-81 of it counted from 0. 2,228 mustpass cases in the eighteen images, 2,225 with a result: 975 pass, 1,189 fail, 60 NotSupported, 1 ResourceError, 3 never began. Seventeen images ended complete=1; T05c aborted on its first case. 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 slow jobs, 0 resets before the end of any run (pi4-console-scan.py; its "faults 1" for T05c is the refused draw, GL error 0x505, not a GPU fault). The longest run is T10 at 49.7 s of the 210 s record deadline: no image overran, nothing was re-split for time. Linux was back 295-343 s after each reboot (cycles.txt: T11 343 s) and every cycle read back as before.
F3F1-F3F5 and F3S1-F3S2 (texture.filtering.3d, named lists of at most 15): 0 pass in the mustpass view, every case Fail with Program(NotYet("a texture look-up other than 2D or cube")), except F3F4 1 NotSupported (sRGB R8) and F3F5 11 NotSupported (srgb_r8_* and srgb_rg8_*); 1-3 s each. T06 (format.sized 2d_array, 3d, cube): 106 pass, 208 fail (the 3D / array look-up refusal), 12 NotSupported. T07 (texture.mipmap): 140 pass, 84 fail (3d look-ups and the `Image verification failed` min/max-lod and base-level cases). T08 (texture.shadow 2d, 2d_array): 0 pass, 288 fail (`NotYet("a shadow look-up")` and the 2D-array look-up). T09 (shadow cube, size): 72 pass, 150 fail; its six `size.2d` cases ended R (a texture larger than the tiled-copy budget) but dEQP completed. T10 (specification basic_tex*): 212 pass, 122 fail. T11 (specification teximage* pbo, depth, unpack): 184 pass, 136 fail. T12 (specification texstorage*): 119 pass, 78 fail. The causes are not read here; the failure groups go to the triage item.
T05 aborted and was re-split (recipe section 6, at the refusing case). The 220-case image (texture.format compressed, unsized, sized.2d) stopped after 23 cases (16 NotSupported for EAC, 4 pass for ETC1, then etc2_eac_rgba8_2d_npot and _2d_pot refused and etc2_eac_rgba8_cube_npot ResourceError): `yakogl/v3d: draw refused: a texel format the V3D does not sample (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here)`, `GL error 0x505 in glDrawElements`, `Test run was ABORTED!`, `complete=0`, end line `STOPPED EARLY by dEQP (complete=0; later cases never began)`, 197 cases without a result. The aborted run's console is not kept (its label is no longer in the table; this paragraph holds its figures; its tryboot cycle read back normally, Linux back after 301 s). The table (94 images to 97, `check-gles3-tree.py` 42810 matched once) now has T05a (196: compressed eac and etc1, unsized, sized.2d), T05b (12: the ETC2 2D cases), T05c (4: etc2_eac_rgba8 and etc2_rgb8 cube) and T05d (8: the other ETC2 cube formats). T05a ran complete: 142 pass, 34 fail, 20 NotSupported in the mustpass view (16 EAC and 4 sRGB), the 4 ETC1 passes among them. T05b ran complete: 4 refused draws counted Fail, 8 NotSupported. T05c aborted on its first case, etc2_eac_rgba8_cube_npot, the same refusal, as S32c did; the other three never began (`none` 3 of table.txt) and `check-pi4-gles3-log.py` reports `aborted, no complete deqp summary (1): T05c` for it. The cause is one refusal (the ETC2 sampling, a V3D back-end item for plan #184), not the table. T05d ran complete: 8 NotSupported. An earlier split of the 12 cube cases as one image (T05c of 12) was run and aborted at the same first case; it is superseded by the 4 + 8 split and not kept.
Method: the 30 images of this session (S11..S25 and F3F1..T12) and the four re-split images T05a-d built with DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh from a checkout at b1b5b845 and the table of this commit (every .built line rc=0 build=b1b5b845), gles3 stage 1 of docs/es3/board-recipe-pi4.md section 2. The board runner could not capture the console (the leased serial console holds the UART bridge), so the steps of tools/board.sh pi4 were done by hand, one image at a time (one GL client on the board): staging to /boot/firmware and tryboot.txt read back (sha256, text), sudo reboot "0 tryboot", the console through the leased serial console (serial_expect on the end line, serial_read into spill files, converted by a script, never copied by hand), Linux back (a new boot_id), staged image removed and tryboot.txt put back, read back. config.txt untouched.

(The table's "row" column counts from 1 in the 97-image table; this session's labels are F3F1..T12.)
