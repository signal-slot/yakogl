# glmark2 host GC7000 costs: vivante

Run: glmark2 `fixture`, `--size 800x600`, 3 frames per entry, one process per entry (run.sh GM2_GCCOST=1, README.md "GC7000 costs"). Counts, not times: what the counted frames (frames 2 to 3) would make the i.MX8MP's CPU pay its GPU. GPU model: `done`, `lag` (`lag`: a kick completes only when the CPU waits for it or a later one, so every fence the back end checks is a wait, the most the board can pay; `done`: linux.c's, every kick complete at once). A wait costs the board at least one 1 ms poll (driver/src/vivante/gc_render.c). Cache maintenance is the DC range work over the carve-out and stands for a CPU pass over memory that is Normal non-cacheable on the board; foreign bytes are ranges outside every GPU allocation (the window). A draw box is the draw's viewport and scissor box in the target's samples (SE_SCISSOR_LEFT/TOP to SE_CLIP_RIGHT/BOTTOM in force at the DRAW_INSTANCED), an upper bound of the pixels it covers, which the stream alone cannot give; a target's size is its kick's largest box extent.

| # | entry | frames | waits a frame: swap, other | kicks | draws | draw box Mpx | RS blits, Mpx | invalidated KiB (calls) | flushed KiB (calls) | foreign KiB | stream KiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | `terrain` | 2 | 1.0, 3.5 | 4.5 | 6.0 | 2.03 | 1.0, 0.48 | 512.0 (1.0) | 512.0 (12.0) | 0.0 | 6.5 |
| 1 | `shadow` | 2 | 0.0, 0.0 | 2.0 | 3.0 | 4.32 | 0.0, 0.00 | 7600.0 (1.0) | 7519.5 (10.0) | 2.0 | 3.3 |
| 2 | `refract` | no gc-cost line | | | | | | | | | |

## 0 `terrain`

Kicks in the counted frames, by the entry point that submitted them and the target:

| entry point | target | stride | kicks a frame | draws a frame | draw box Mpx a frame | RS blits a frame | RS Mpx a frame | stream KiB a frame |
|---|---|---|---|---|---|---|---|---|
| `glDrawArrays` | 800x600 | 3200 | 1.0 | 3.0 | 1.440 | 1.0 | 0.480 | 4.0 |
| `eglSwapBuffers` | 800x600 | 3200 | 1.0 | 1.0 | 0.480 | 0.0 | 0.000 | 0.5 |
| `glGenerateMipmap` | 256x256 | 1024 | 1.0 | 1.0 | 0.066 | 0.0 | 0.000 | 1.2 |
| `glGenerateMipmap` | 128x128 | 1024 | 1.0 | 1.0 | 0.016 | 0.0 | 0.000 | 1.2 |
| `glDrawArrays` | 0x0 | 0 | 0.5 | 0.0 | 0.000 | 0.0 | 0.000 | 0.0 |

1 kicks could not be decoded (no CPU view of their stream).

By entry point, in the counted frames:

| entry point | waits a frame | kicks a frame | invalidated KiB a frame (calls) | flushed KiB a frame (calls) |
|---|---|---|---|---|
| `glGenerateMipmap` | 2.0 | 2.0 | 0.0 (0.0) | 16.0 (8.0) |
| `glDrawArrays` | 1.5 | 1.5 | 512.0 (1.0) | 495.5 (3.0) |
| `eglSwapBuffers` | 1.0 | 1.0 | 0.0 (0.0) | 0.5 (1.0) |

The waits by caller (the entry point in progress, then the library's frames from the wait outwards, `addr2line`'s names; every phase):

| entry point | caller | setup | frames, a frame | teardown |
|---|---|---|---|---|
| `glGenerateMipmap` | `wait_fence<draw_one<record_level<generate_mipmap<gc_generate_mipmap<glGenerateMipmap` | 1 | 2.0 | 0 |
| `glDrawArrays` | `wait_fence<draw_one<gc_draw<{closure#1}<draw<glDrawArrays` | 2 | 1.5 | 0 |
| `eglSwapBuffers` | `wait_fence<{closure#1}<eglSwapBuffers<p_eglSwapBuffers<+0x1234<+0x0` | 1 | 1.0 | 1 |

## 1 `shadow`

Kicks in the counted frames, by the entry point that submitted them and the target:

| entry point | target | stride | kicks a frame | draws a frame | draw box Mpx a frame | RS blits a frame | RS Mpx a frame | stream KiB a frame |
|---|---|---|---|---|---|---|---|---|
| `glDrawArrays` | 1600x1200 | 0 | 1.0 | 2.0 | 3.840 | 0.0 | 0.000 | 2.1 |
| `glDrawArrays` | 800x600 | 3200 | 1.0 | 1.0 | 0.480 | 0.0 | 0.000 | 1.2 |

By entry point, in the counted frames:

| entry point | waits a frame | kicks a frame | invalidated KiB a frame (calls) | flushed KiB a frame (calls) |
|---|---|---|---|---|
| `glDrawArrays` | 0.0 | 2.0 | 7600.0 (1.0) | 7519.5 (10.0) |

