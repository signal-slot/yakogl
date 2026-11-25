# Results

| Board | glmark2-es2 | gl_test | GPU resets |
|---|---|---|---:|
| AM62P | 462, 33 scenes, 0 failed, 0 GL errors, 0 out-of-memory scenes; main `aaaaaaa1`, 2026-10-18; pictures against `tools/cts/glmark2-pictures.txt` column `am62p`: 33 scenes, match 28, mismatch 5, each one probe of an animated scene (desktop blur tr, terrain c off 10, refract c), shadow a match, none black | 50 of 50 | 0 |
| i.MX8MP | 42, 33 scenes, 0 failed, 0 GL errors, 0 out of memory; main `aaaaaaa1`, 2026-10-18; all 33 pictures match the `imx8mp` column of `tools/cts/glmark2-pictures.txt`, none black, **terrain match**, refract differs in one probe | 49 of 49 | 0 |
| Pi 4 | 188, 33 of 33 scenes, 0 failed, 0 unsupported, 0 GL error lines, 0 out of GPU memory; main `aaaaaaa1b2c3d4e`, 2026-10-18; 28 of 33 pictures match, none black (terrain not looked at by eye, refract not recorded: no camera) | 12 of 12 | 0 |

Text.
