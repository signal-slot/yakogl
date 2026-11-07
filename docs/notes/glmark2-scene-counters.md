# glmark2 GM image: the per-scene counter line

Design only (plan #329, item glmark2-scene-counters-design). No code
changes. The print belongs in `driver/samples/zephyr/glmark2/src/main.cpp`,
which plan #328 owns, so the patch below is a sketch for whoever lands it
after #328 releases the file. Every name and line was checked against
`origin/main` at `2ae5add0`.

Why: `pi4-slow-terrain.md` ("Counters a board run needs") and
`am62p-slow-terrain.md` (same heading) both ask for per-scene counter
differences. Today the scene's share is read from lines printed only when an
arena grows, or from cumulative lines at frame 1.

## Semantics

- Every counter below is **cumulative since init** (`u32` unless noted; the
  `u64`s are ns). The line holds the **difference** between the value read
  at the scene's end and the value read at its start (`log_scene_info`,
  where `s_cost` is reset, `main.cpp:1043-1050`). Use `wrapping_sub` on
  `u32`.
- Exceptions that are gauges, not counters: `v3d_rt_images` `live` and
  `bytes` (current values, print the value at the scene's end, not a
  difference).
- One line per scene, printed from `log_scene_result` right after
  `print_steps()` (the `glmark2-steps:` line, `main.cpp:1068`), skipped when
  `s_cost.frames == 0` like `print_steps` (`main.cpp:1178`).
- Per scene, not per frame: the line is raw differences plus `frames N`, so
  a reader divides; the existing `glmark2-frame:` and `glmark2-steps:` lines
  already give per-frame figures and rounding would hide a rate under 1 a
  frame (re-posts, evictions).
- `key=value`, separated by spaces, no spaces inside a value. Lists inside a
  value are comma-separated `name:n` pairs, zero entries omitted. One
  prefix per back end so a parser picks by prefix: `glmark2-v3d:` and
  `glmark2-pvr:`. A build prints exactly one of them.

## Line format

V3D (`CONFIG_YAKOGL_GPU_V3D`):

```
glmark2-v3d: <scene>: frames N jobs J waits W copies=plain:n,first-sample:n,... mip_refused=tfu:n,wait:n,... mip_queued=Q mip_queue_full=F rt_direct=D rt_cpu_writes=C rt_no_memory=M rt_chains=H rt_chains_returned=R rt_live=L rt_bytes=B tile_ns=U,K,A tile_untimed=X
```

`copies` and `mip_refused` list names from `COPY_REASON_NAMES`
(`src/v3d/backend.rs:2224-2241`) and `MIP_REFUSAL_NAMES`
(`:2278-2281`) with a nonzero difference. `tile_ns` is the three slots
upload, kept-at-draw, arena (`TILE_AT_UPLOAD`, `TILE_KEPT_AT_DRAW`,
`TILE_ARENA`, `:3987-3994`).

PowerVR (`CONFIG_YAKOGL_DRV_GPU_PVR`):

```
glmark2-pvr: <scene>: frames N kicks K waits W mip_chains=made reposts=P draws=D tiles=T quads=Q splits=S wait_timeouts=O
```

`kicks` here is the backend's kicks submitted, same word as
`kicks/frame` in `am62p-slow-terrain.md` (52 a frame at terrain); `waits` is
its CPU waits on a fence.

## Counters that exist

| Back end | Name (C) | file:line | Arguments (type, unit) | Cumulative | Guard |
|---|---|---|---|---|---|
| V3D | `v3d_counters` | `src/v3d/backend.rs:6355` | `kicks`, `waits`: `u32*`, jobs submitted and CPU waits for a job (`V3dState.kicks` `:1246`, `.waits` `:1255`) | yes | `CONFIG_YAKOGL_GPU_V3D` |
| V3D | `v3d_copy_counters` | `:2269` | `copies`: `u32[V3D_COPY_REASONS=16]` (`:2261`), tiled copies and stores by reason | yes | same |
| V3D | `v3d_mip_counters` | `:5034` | `refused`: `u32[V3D_MIP_REFUSALS=14]` (`:5024`), `queued`, `queue_full`: `u32*` | yes | same |
| V3D | `v3d_rt_images` | `:4990` | `live` `u32` and `bytes` `u64` (gauges); `direct`, `cpu_writes`, `no_memory`, `chains`, `chains_returned`: `u32*` | all but `live`, `bytes` | same |
| V3D | `v3d_tiling_time` | `:5157` | `ns`: `u64[3]` ns of the OSAL clock; `untimed`: `u32*` | yes; the times are valid only while the `untimed` difference is 0 (needs `OSAL_QUERY_CLOCK_NS`, `src/osal/osal_internal.h:103`) | same |
| PowerVR | `pvr_backend_counters` | `src/pvr/backend.rs:9200` (decl. `src/pvr/pvr_backend.h:43`) | `kicks`, `waits`: `u32*` (`:1855-1856`) | yes | `CONFIG_YAKOGL_DRV_GPU_PVR` |
| PowerVR | `pvr_backend_mip_chains` | `:6382` (no C declaration; declare it in the sample) | `made`, `refused`: `u32*` (`:1632-1633`), GPU mipmap chains made and left to the CPU | yes | same |
| PowerVR | `pvr_backend_workload` | `:9222` (decl. `pvr_backend.h:44`) | `draws`, `tiles`: `u32*` (`:1859-1860`), objects and 16x16 tiles over all kicks | yes | same |
| PowerVR | `pvr_backend_overhead` | `:9243` (decl. `pvr_backend.h:47`) | `quads`, `splits`: `u32*` (`:1864-1865`), driver-added full-screen quads and frame splits | yes | same |
| PowerVR | `pvr_backend_wait_timeouts` | `:9269` | returns `u32`, fence waits that timed out (`:1858`) | yes | same |
| PowerVR | `pvr_backend_ds_refused` | `:9278` | returns `u32`, draws/clears refused (GL_OUT_OF_MEMORY) because the depth/stencil buffer could not be allocated or was lost; not in the line above, add `ds_refused=` if wanted | yes | same |
| PowerVR driver | `pvr_render_reposts` | `driver/src/pvr_render.h:250` (defined `pvr_render.c:348`, set `:748`, `:784`) | `uint32_t`, kicks re-posted by the 20 ms wait mitigation since boot | yes; counts only while `pvr_render_wait_repost` is on (`pvr_render.h:247`, default on) | `CONFIG_YAKOGL_DRV_GPU_PVR`; header `<pvr_render.h>` is already included (`main.cpp:147`) |
| PowerVR driver | `pvr_render_fl_counters` | `pvr_render.h:225`: `u32[9]`, word 6 is the same re-post count (`driver/rust/src/gpu/pvr/render.rs:2237-2249`) | | yes | same; use `pvr_render_reposts` instead, it needs no array |

No C header declares any `v3d_*` function above (`osal_internal.h:100` only
names `v3d_tiling_time` in a comment). `main.cpp` must declare them in its own
`extern "C"` block, guarded by `CONFIG_YAKOGL_GPU_V3D`, with the signatures
in the table. The host harness does the same with weak symbols
(`tools/hostharness/glmark2/shim.c:325`, `:448`).

`pvr_render_wait_repost` is on by default (`pvr-waited-kick.md`, citing
`pvr_render.c:347`, `:597`), so `reposts` is live on the usual image; on a
build that turns it off a 0 means "mitigation off", not "none needed". The
log line for re-posts stops counting at 8; this counter is the true total.

## Counters that do not exist yet

| Wanted | For | Owning item | Notes |
|---|---|---|---|
| CPU time summed in `wait_job` (`src/v3d/backend.rs:3051-3057`), ns | V3D: waits W says how many, not how long (`pi4-slow-terrain.md`, `v3d-wait-poll.md`) | `v3d-wait-time` | export beside `v3d_counters`; add `wait_ns=` after `waits` in the V3D line |
| Render-image evictions (`rt_register`, `:1862`) | V3D candidate 2 | `v3d-rt-evict` (in review) | add `rt_evictions=`; if it lands as a field of `v3d_rt_images`, the argument list above grows by one `u32*` |
| CPU mipmap filter time (ns) and bytes | V3D candidate 3; PowerVR path B | `mip-filter-wide` | the GL core counts them, so they are back-end independent: print on both lines as `mip_filter_ns=` and `mip_filter_bytes=` |
| PowerVR frame-set waits (kick N waits for kick N-4, `src/pvr/backend.rs:5288-5291`), their count and CPU time | `am62p-slow-terrain.md` (about 47 a frame) | none of the plan's items adds a counter: `pvr-frameset-wait-study` is a study, `pvr-repost-trace` names the wait site in a log line only | needs an item; see the handoff |
| PowerVR CPU time in `wait_fence`, by call site (frame set, swap, finish) | same | `pvr-repost-trace` names the site for the 20 ms re-post only | same |

Until an item adds one, the line omits its key. The sketch prints no key it
cannot read, so a record never shows a made-up 0.

## Patch sketch for `main.cpp`

Under 40 lines, on top of a `main.cpp` that #328 has released. `Snap` holds
the scene-start values; fill it in `log_scene_info` next to
`s_cost.steps_known = ...` (`:1050`), print from `log_scene_result` after
`print_steps();` (`:1068`).

```cpp
#if defined(CONFIG_YAKOGL_GPU_V3D)
extern "C" { /* src/v3d/backend.rs; no C header declares them */
void v3d_counters(uint32_t *kicks, uint32_t *waits);
void v3d_copy_counters(uint32_t *copies);          /* [16] */
void v3d_mip_counters(uint32_t *refused, uint32_t *queued, uint32_t *full); /* [14] */
void v3d_rt_images(uint32_t *live, uint64_t *bytes, uint32_t *direct, uint32_t *cpu,
                   uint32_t *nomem, uint32_t *chains, uint32_t *ret);
void v3d_tiling_time(uint64_t *ns, uint32_t *untimed); /* [3] */
}
struct Snap { uint32_t k, w, cp[16], rf[14], q, f, d, c, nm, ch, rt, un; uint64_t ns[3]; };
static void snap(Snap &s) {
    uint32_t l; uint64_t b;
    v3d_counters(&s.k, &s.w); v3d_copy_counters(s.cp); v3d_mip_counters(s.rf, &s.q, &s.f);
    v3d_rt_images(&l, &b, &s.d, &s.c, &s.nm, &s.ch, &s.rt); v3d_tiling_time(s.ns, &s.un);
}
#elif defined(CONFIG_YAKOGL_DRV_GPU_PVR)
extern "C" void pvr_backend_mip_chains(uint32_t *made, uint32_t *refused);
struct Snap { uint32_t k, w, mc, rp, dr, ti, qd, sp, to; };
static void snap(Snap &s) {
    uint32_t refused; pvr_backend_counters(&s.k, &s.w); pvr_backend_mip_chains(&s.mc, &refused);
    s.rp = pvr_render_reposts; pvr_backend_workload(&s.dr, &s.ti);
    pvr_backend_overhead(&s.qd, &s.sp); s.to = pvr_backend_wait_timeouts();
}
#endif
/* print_counters(): Snap now; snap(now); then one printk per back end, e.g. the V3D
 * line: printk("glmark2-v3d: %s: frames %llu jobs %u waits %u copies=", ...); the names
 * come from the two string tables copied from backend.rs; a loop prints "name:n," for
 * each now.cp[i] - s_snap0.cp[i] != 0; tile_ns is only printed when un - un0 == 0. */
```

Assumptions of the sketch: the scene-start snapshot is a file-scope
`static Snap s_snap0`, so it does not enlarge `FrameCost`; the name tables
are duplicated in `main.cpp` (a rename in `backend.rs` then breaks the
reader, so a line that prints the table length first, `copies16`, would
catch it; or export the names, which is a code change outside this item).

## Checks that go with the print (for the item that lands it)

- `tools/cts/glmark2-ms.py` and the scripts that read `glmark2-steps:` parse
  by prefix: a new prefix must not match their patterns. Check before the
  print lands (not done here).
- Host: the harness shim already reads `v3d_counters` and
  `pvr_backend_counters` weakly (`shim.c:448`, `:325`); a host test of the
  print can feed the same functions and compare the line.
- The `README`, `RESULTS.md` and `STATUS.md` glmark2 rows stay #317's; the
  line is evidence for notes and issue comments.
