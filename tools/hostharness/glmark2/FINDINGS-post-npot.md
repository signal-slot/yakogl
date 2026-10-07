# glmark2 host harness after `npot` merged

Result in one line: `GL error 0x502 in glGenerateMipmap` is **gone on
powervr, vivante and v3d** (terrain and refract), and the incomplete-texture
lines went with it. One **new problem on the V3D**: refract now refuses a
draw, because its NPOT mip chain no longer fits the tiled-copy budget
(section 2.1).

## 1. Commit and commands

* Tree: `fdff1147b151a4a240126371a06f45f69a310601` (ccfarm/integration tip).
  The npot commits are in it: `2673197d` (core), `77693ff4`, `be938366`.
  `tools/check.sh --quick` on this tree: 99 ok, 0 failed, 0 gaps.
* glmark2: `22c527cb0556f3a1ac4445aaa52cc532760928d5`, cloned and checked out
  in the deps cache:

```sh
git clone https://github.com/glmark2/glmark2 "$GLMARK2_SRC"
git -C "$GLMARK2_SRC" checkout 22c527cb0556f3a1ac4445aaa52cc532760928d5
tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-post-npot
```

  (`GLMARK2_SRC=/home/tasuku/.local/share/ccfarm/deps/yakogl/glmark2`.)
  This runs powervr, vivante and v3d, 33 entries, 3 frames, 800x600. The V3D
  uses `YAKOGL_V3D_CARVEOUT_BYTES=100663296` (96 MiB, set by `run.sh` from the
  Pi 4 board file). `run.sh` was run with its default `GM2_WRITE=1`, so it
  rewrote `FINDINGS.md` and `FINDINGS-v3d.md`. I diffed them against HEAD for
  the table below, then restored both with `git checkout`. The shader
  corpus came out identical (69 texts). The logs are in
  `build/glmark2-post-npot/logs` (untracked).

## 2. 0x502 in glGenerateMipmap, and what changed

All 33 entries ended `ok` on all three GPUs.

| entry | GPU | line | before | after |
|---|---|---|---|---|
| 22 terrain | powervr | `GL error 0x502 in glGenerateMipmap` | 11 | **0** |
| 22 terrain | vivante | `GL error 0x502 in glGenerateMipmap` | 11 | **0** |
| 22 terrain | v3d | `GL error 0x502 in glGenerateMipmap` | 11 | **0** |
| 24 refract | powervr | `GL error 0x502 in glGenerateMipmap` | 3 | **0** |
| 24 refract | vivante | `GL error 0x502 in glGenerateMipmap` | 3 | **0** |
| 24 refract | v3d | `GL error 0x502 in glGenerateMipmap` | 3 | **0** |
| 22 terrain | powervr, vivante | incomplete texture, unit 0, 800x600 | 1, 1 | 0, 0 |
| 22 terrain | v3d | incomplete texture, unit 0, 800x600 | 1 | 0 |
| 24 refract | powervr, vivante | incomplete texture, unit 1, 1600x1200 | 1, 1 | 0, 0 |
| 24 refract | v3d | incomplete texture, unit 1, 1600x1200 | 1 | 0 |
| 24 refract | v3d | `draw refused: a texture larger than the tiled-copy budget …` | 0 | **1 (new)** |
| 24 refract | v3d | `glDrawArrays was not recorded (-22); the frame is missing it` | 0 | **3 (new)** |
| 24 refract | v3d | `GL error 0x505 in glDrawArrays` | 0 | **3 (new)** |
| 21 jellyfish | v3d | `tiled copies high-water 1064960 of 8388608 bytes` | 0 | 1 (new, info only) |

No other line changed on any GPU. No new compile or link failure, and refused
draws stay 0 on powervr and vivante. On the V3D the scene table still shows
`refused=0` for refract: the refusal is the backend's `-22`, not the
`yakogl_draws_refused` counter.

### 2.1 New: V3D refract draw refused

* Line: `yakogl/v3d: draw refused: a texture larger than the tiled-copy budget
  (CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES) and the texture arena (8 MiB
  tiled)`, then `glDrawArrays was not recorded (-22)` and `GL error 0x505`
  once per frame (3 frames).
* Cause: refract's 1600x1200 RGBA8 texture now has a mip chain
  (about 7.68 MB for level 0, about 10.2 MB with the chain) before npot it
  had one level. The tiled image is bigger than both the per-job arena
  (8 MiB, `TEX_BYTES`) and the kept-copy budget, so `src/v3d/backend.rs:2909`
  (`image_bytes + 4096 + 64 > TEX_BYTES`) reaches the `OverBudget` message at
  `src/v3d/backend.rs:2913`.
* The budget is the default 8388608: `zephyr/Kconfig:305-307`, and the host
  harness reads it from `src/osal/linux.c:585`. The Pi 4 glmark2 board file
  `driver/samples/zephyr/glmark2/boards/rpi_4b.conf` does not set
  `YAKOGL_V3D_TILED_COPIES_BYTES` (no match in `driver/samples/zephyr/glmark2`).
  The Kconfig help (`zephyr/Kconfig:309-316`) says a 16.8 MiB image needs about
  24 MiB here.
* Not fixed here. It is a board-config or backend question for the Pi 4
  items (for example `pi4-mem`, `v3d-hh-48m`). It means refract on the Pi 4
  would draw with a missing draw call, not a black texture. Check it on the
  board.

## 3. Follow-up lines (still there)

| entry | GPU | line | before | after |
|---|---|---|---|---|
| 23 shadow | powervr | `the backend did not resolve its depth (-16)` | 3 | 3 |
| 24 refract | powervr | `the backend did not resolve its depth (-16)` | 3 | 3 |
| 23 shadow | v3d | `a stale tiled copy of 7684096 bytes given back before the job opened` | 1 | 1 |
| 24 refract | v3d | `a stale tiled copy of 7684096 bytes given back before the job opened` | 1 | 1 |

Vivante logs no depth-resolve line. `pvr-depth-resolve` (still `[ready]`) and
`v3d-rt-sample` own the other two.

## 4. V3D GPU-memory peak, 96 MiB carve-out

The pool is 100,134,912 allocatable bytes. Refused allocations: 0 in all 33
entries. Highest, with the earlier values from FINDINGS-v3d.md:

| entry | before | after | share of 100,134,912 |
|---|---|---|---|
| 22 terrain | 54,360,064 | **59,829,248** (the run's peak) | 59.7% |
| 24 refract | 55,527,424 | 58,087,424 | 58.0% |
| 23 shadow | 40,908,544 | 40,908,544 | 40.9% |
| 21 jellyfish | 29,829,888 | 30,096,128 | 30.1% |

Terrain grew by about 5.5 MB for its NPOT chain. Refract grew by 2.6 MB, and
would grow more if its draw were not refused (2.1). The carve-out still has
about 40 MB spare at the peak. Longest free run at exit is 84,758,016 for
terrain and refract, as before.
