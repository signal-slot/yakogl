<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# display_check

Host-side judge for the DK display probe's picture, read from a NanoKVM
screenshot. Python 3, standard library only; no board needed to run it.

```
colorbars.py SHOT.png|SHOT.ppm
colorbars.py --triangle --apex 0.5,0.15 --apex 0.2,0.8 --apex 0.8,0.8 \
             [--color R,G,B] [--background R,G,B] SHOT.png
selftest.sh
```

Input: PNG (8-bit gray/RGB/RGBA, non-interlaced) or binary PPM (P6). The
stdlib has no JPEG decoder: convert a JPEG first (`convert shot.jpg shot.ppm`).

## Colour bars

`mp2_display` fills 8 equal vertical bars: white, yellow, cyan, green,
magenta, red, blue, black. The tool classifies every column of the middle
half of the picture by the nearest of those colours (`--tol`, default 100 of
441), measures the runs and prints one line; exit 0 = PASS, 1 = FAIL,
2 = unreadable input.

| Output | Meaning |
|---|---|
| `colorbars: PASS: 8 bars white yellow ... black` | order right, widths equal (a black border left of the picture is ignored) |
| `FAIL: no signal: the picture is black` / `neither black nor colour bars` | the link is down, or the KVM's no-signal screen |
| `FAIL: red and blue swapped (...)` | the order is W C Y G M B R K: a pixel-format mistake (RGB vs BGR in LTDC or the layer format) |
| `FAIL: partial: N of 8 bars` | fewer bars: a cut-off timing or a wrong active width/stride |
| `FAIL: scaled: uneven bar widths` | the picture is scaled or the stride is wrong |
| `FAIL: wrong order: ...` | any other order |

## GPU triangle

`--triangle` takes three `--apex X,Y` (fractions of width/height; `:R,G,B`
after one sets that apex's expected colour, `--color` sets it for all). The
tool samples a small window 20% of the way from each apex to the centroid
(the exact apex pixel is anti-aliased) and the centroid: each must differ
from `--background` (default black) and, when a colour is given, match it
within `--tol`. Prints `triangle: PASS|FAIL: ...`.

## Self-test

`selftest.sh` writes synthetic PNG/PPM files with the tool's own writer
(correct bars as RGB, RGBA and PPM, a letterboxed picture, R/B swapped,
black, a no-signal screen, half the bars, uneven widths, reversed order, a
triangle and an empty one), runs the tool on each and checks verdict and exit
code. It prints each case and `selftest: N ok, M failed`; exit 0 when none
failed. It needs only `python3` and `mktemp`.

Possible gate row (tools/check.sh is another plan's): `tools  display_check
selftest` running `tools/probes/stm32mp257f_dk/display_check/selftest.sh`.
