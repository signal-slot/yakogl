# glmark2 host profile: v3d

Run: glmark2 `fixture`, `--size 800x600`, 11 frames per entry, one process per entry (run.sh GM2_PROFILE=1, README.md "Profile"). Host time on this machine, not the board's: linux.c's submits complete at once, so no GPU wait is in it. Counted frames: frames 2 to 11 (the first frame is in setup). Wall = CLOCK_MONOTONIC, cpu = the thread's CPU time. The GPU work linux.c models on the CPU (the V3D's TFU jobs) is the board GPU's, not the CPU's: it is not in the wall and cpu figures, and its own column gives its host time.

## 0 `ideas:speed=duration`

10 counted frames: 3.000 ms wall, 3.000 ms cpu a frame; in GL and EGL calls 2.200 ms (73%), in glmark2 itself 0.800 ms.

| entry point | calls a frame | wall ms a frame | cpu ms a frame | share of GL wall | modelled GPU jobs, ms a frame |
|---|---|---|---|---|---|
| `glDrawElements` | 200.0 | 2.000 | 2.000 | 90.9% | - |
| `eglSwapBuffers` | 1.0 | 0.200 | 0.200 | 9.1% | - |

Draw steps (src/draw_steps.rs laps over the counted frames): 2000 timed draws, 200.0 a frame; 5.000 us a draw in all.

| step | us a draw | share |
|---|---|---|
| `GlProgram` | 1.000 | 20.0% |
| `GlUniforms` | 0.500 | 10.0% |
| `GlAttribs` | 2.000 | 40.0% |
| `GlState` | 1.500 | 30.0% |

Setup and first frame: 2.0 ms wall in GL and EGL calls; the top entry points: `glDrawElements` 2.0 ms (2 calls), `eglSwapBuffers` 0.0 ms (1 calls).

