glmark2 host profile, v3d: time in GL and EGL calls, ms per frame

| scene | before min ms/frame | n | CV | after min ms/frame | n | CV | delta ms | ratio |
|---|---|---|---|---|---|---|---|---|
| `terrain` | 60.000 | 2 | 5% | 19.000 | 3 | 5% | -41.000 | 0.32x |
| `shadow` | 50.000 | 3 | 7% | 51.000 | 3 | 6% | +1.000 ~ | 1.02x |

Top entry points by wall ms a frame in the before run:

| scene | entry point | before ms/frame | after ms/frame | delta ms |
|---|---|---|---|---|
| `terrain` | `glDrawElements` | 36.000 | 11.400 | -24.600 |
| `shadow` | `glDrawElements` | 30.000 | 30.600 | +0.600 |

Minimum of the repeats of the chosen metric; CV = sample standard deviation / mean of the repeats kept; '~' marks a delta smaller than the before CV * minimum (within noise).

Left out as disturbed (wall time more than 5% above cpu time): 1 before repeat of `terrain`.
