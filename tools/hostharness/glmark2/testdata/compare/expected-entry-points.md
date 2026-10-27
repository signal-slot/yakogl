glmark2 host profile, v3d: time in GL and EGL calls, ms per frame

| scene | main ms/frame | fix ms/frame | delta ms | ratio |
|---|---|---|---|---|
| `terrain` | 63.000 | 18.000 | -45.000 | 0.29x |
| `shadow` | 50.000 | 53.000 | +3.000 | 1.06x |

Top entry points by wall ms a frame in the main run:

| scene | entry point | main ms/frame | fix ms/frame | delta ms |
|---|---|---|---|---|
| `terrain` | `glGenerateMipmap` | 50.000 | 5.000 | -45.000 |
| `terrain` | `glDrawArrays` | 10.000 | 10.000 | +0.000 |
| `shadow` | `glDrawElements` | 30.000 | 33.000 | +3.000 |
| `shadow` | `glReadPixels` | 20.000 | 20.000 | +0.000 |

Not in both runs (left out): `refract`.
