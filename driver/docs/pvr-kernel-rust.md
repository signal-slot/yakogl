<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The PowerVR kernel side in Rust: inventory and plan

docs/source-form.md, group A, the remainder (branch `form/7-pvr-kernel-rust`).
After step 6 the firmware start, stop and image load are the vendored DDK's
(`rgxstartstop.c`, `rgxfwimageutils.c`) on our `rgxlayer.h` in Rust. What is
left in C is the code that began as a port of Linux drm/imagination (the MMU
page-table writer, the CCB and kick code, the FWIF structure setup), the
reset staging `pvr_bringup.c` copies from the DDK, and the render service,
which is ours. The rule (docs/source-form.md): upstream code stays in upstream
form, original code is Rust. The mainline-derived C is not upstream code we
carry (mainline's firmware ABI is not the one the board runs, and its
`pvr_mmu.c` / `pvr_ccb.c` / `pvr_fw.c` need DRM), so it is rewritten against
the vendored DDK types and becomes ours, in Rust.

The DDK's own implementations (`mmu_common.c`, `rgxccb.c`, `rgxfwutils.c`,
`rgxta3d.c`) are used as documentation of the formats only. They are MIT, but
they need the DDK's environment layer (devicemem, PMR, OS functions, sync,
about 170k lines with what they pull in); vendoring them is not proposed.

## Where things go

| Rust module (`driver/rust/src/gpu/pvr/`) | what | C left |
|---|---|---|
| `km.rs` | `struct yd_pvr_km`: the carveout, the OS hooks (cache clean/invalidate, full barrier, log), the register bus; all state of the modules below lives in it (C-owned, one per GPU) | `pvr_fw_boot.c` owns the instance and fills the hooks |
| `pool.rs` | the render pool's free lists per heap (best fit, merge on both sides, overflow chain with checked headers, double-free refusal, opt-in rollback) and the pool's summary lines (tools/cts/log/investigations/pvr-render-pool.md) | `pvr_render_alloc`, `pvr_render_free`, `pvr_render_pool_report` wrappers |
| `mem.rs` | the carveout's allocators: FW-heap bump, page-table pages, render pool (bump, PM-heap 16 KiB variant, newest-block rollback, placed spans) and FW-heap addressing | thin `pvr_*` wrappers in `pvr_fw_boot.c` (pvr_drv.h API) |
| `mmu.rs` | the 3-level page-table writer on the formats of `rgxmmudefs_km.h`: map, lookup, audit, dirty flag | wrappers |
| `ccb.rs` | KCCB post and MTS doorbell (with the release barrier before the doorbell), slot probe, FWCCB service | wrappers |
| `freelist.rs` | freelist build (PFN array, `RGXFWIF_FREELIST`), the grow pool, grow / reconstruction answers, audit | wrappers |
| `fwinit.rs` | the FWIF structures: KCCB/FWCCB, sysinit, osinit, sysdata/osdata, runtime cfg, trace buffer, HWR info, align checks | `pvr_fw_boot()` sequencing, the firmware blob, RGXStart/RGXStop calls |
| `fwif.rs` | the DDK layout numbers the modules use (offsets, sizes, enum values), each pinned against the vendored headers by the host check | — |
| `render.rs` | the render service's firmware side: the render context, the per-size HWRT datasets (build, reuse, recycle, cleanup), the client CCBs, the submit (commands, the pre-kick MMU cache command, the kicks), the fence wait (re-post, HWR decode, stall dumps) | `pvr_render.c`: the static PDS/USC areas (they call group B's generators), the parameter buffer and freelist sizing, the pool's API (`pvr_render_alloc`/`_free`/`_pool_report` over `pool.rs`), the run-time switches, the `pvr_render.h` API |

## Inventory

Origin: **mainline** = began as a port of Linux drm/imagination (GPL-2.0-only
OR MIT, used under MIT; NOTICE entry 3), its definitions since replaced by
the DDK's; **DDK-seq** = follows a DDK 25.2 sequence by hand; **ours** =
written for this project. Fate: the Rust module, **C shim** (stays C because
Zephyr/the OS needs it there), **C lab** (bring-up rig, not ported), or
**deleted**.

### driver/src/pvr_fw_boot.c

| function | lines (674ea1cf) | origin | fate | step |
|---|---|---|---|---|
| `pvr_heap_alloc`, `heap_carveout_off`, `heap_cpu/gpu/fwaddr/phys` | 142-170 | mainline (pvr_fw.c FW-heap bump) | `mem.rs` | 2 |
| `pt_alloc_page` | 175-192 | mainline (pvr_mmu.c page-table page alloc) | `mem.rs` | 2 |
| `pvr_mmu_init`, `pvr_mmu_map`, `pvr_mmu_take_dirty`, `pvr_mmu_lookup` | 220-346 | mainline (pvr_mmu.c), formats now rgxmmudefs_km.h | `mmu.rs` | 2 |
| `pvr_mmu_audit` | 282-328 | ours (lab audit) | `mmu.rs` | 2 |
| `pvr_mmu_build` (the FW heap mapping) | 348-365 | mainline (pvr_fw.c heap map) | `mmu.rs` (`yd_pvr_mmu_map_fw_heap`) | 2 |
| `rgx_layer_init`, `pvr_fw_load`, `ddk_fw_get_blob`, `pvr_ddk_fw_preload` | 392-537 | ours, calling the DDK loader (step 6) | C shim (`FW_DATA_OFF` derived in step 4); `ddk_fw_get_blob` and `pvr_ddk_fw_preload` (the Linux module's /lib/firmware load) were removed on 2026-10-04 with the Linux module | 4 |
| `obj_alloc`, `ccb_ctl_init`, `pvr_fw_build_structs` | 630-879 | mainline (pvr_fw.c `pvr_fw_create_fwif_*`), on DDK structs | `fwinit.rs` | 4 |
| `PVR_LEGACY_SIZE_*` | 130-135 | mainline struct sizes kept | deleted (sizeof the DDK structs) | 4 |
| `pm_snap` | 885-902 | ours (bring-up print) | C lab (`pvr_selftest.c`; the boot-time call is gone) | 4 |
| `pvr_fw_start`, `pvr_fw_stop` | 908-933 | ours, calling RGXStart/RGXStop | C shim | — |
| `pvr_wait_for_fw_boot` | 935-981 | mainline (pvr_fw.c wait), DDK offsets | `fwinit.rs` (`wait_started`, the poll and the dump) | 4 |
| `pvr_fw_kccb_healthcheck` (not called) | 985-1027 | mainline (pvr_ccb.c test) | deleted | 3 |
| `pvr_gpu_bo_alloc_f`, `_alloc`, `_alloc_pm`, `pvr_gpu_bo_bump_rollback`, `gpu_bo_place`, `pvr_bo_phys` | 1053-1194, 1563 | ours | `mem.rs` | 2 |
| `pvr_kccb_doorbell`, `pvr_kccb_post` | 1205-1292 | mainline (pvr_ccb.c), DDK `__MTSScheduleWrite` readback | `ccb.rs` | 3 |
| `pvr_kccb_probe_slot` | 1213-1234 | ours (print-only probe) | `ccb.rs` | 3 |
| `pvr_fwccb_service` | 1393-1547 | mainline (pvr_ccb.c `pvr_fwccb_process`), DDK grow sequence | `ccb.rs` | 3 |
| `pvr_fl_addr_list`, `pvr_fl_by_id`, `pvr_freelist_audit`, `pvr_fl_reconstruct` | 1306-1386 | ours / mainline (pvr_free_list.c reconstruct) | `freelist.rs` | 3 |
| `pvr_fl_pool_init`, `pvr_fl_refusals`, `pvr_fl_grow_reqs`, `pvr_fl_grows_done`, `pvr_fl_pool_peak_pages`, `pvr_fl_set_reserve_mult` | 1577-1625, 597 | ours | `freelist.rs` | 3 |
| `pvr_build_freelist` | 1634-1787 | ours, after the DDK's RGXCreateFreeList (DDK capture values) | `freelist.rs` | 3 |
| `pds_put`, `fwt_emit`, `pvr_render_ta3d` (2500 lines) | 1565-4323 | ours (the bring-up self-test rig) | C lab, moved to `pvr_selftest.c` (compiled always, so it keeps building; run at boot only with `CONFIG_SAFEUI_GPU_PVR_SELFTEST`) | 4 |
| `pvr_fw_boot` | 4325-4419 | ours | C shim (carveout map, sequencing) | — |
| accessors (`pvr_regs`, `pvr_heap_cpu`, ...) | 615-627 | ours | C shim over `struct yd_pvr_km` | 2-4 |

### driver/src/pvr_bringup.c

| function | origin | fate | step |
|---|---|---|---|
| `pvr_power_on` (TISCI) | ours | C shim | — |
| `pvr_read_bvnc` | mainline (pvr_device.c `pvr_load_gpu_id`), DDK defs | C shim | — |
| `pvr_reset_internal` (milestone 2) | DDK-seq (RGXResetSequence by hand) + a SOFT_RESET2 pulse the DDK does not make | deleted: RGXStart resets the core itself | 4 |
| `pvr_init`, the core-clock measurement, DT glue | ours | C shim | — |

### driver/src/pvr_render.c (ours, all of it)

| part | fate | step |
|---|---|---|
| `fw_alloc`, `kccb_cleanup`, `pvr_mh_tag_name` | `render.rs` | 5 |
| `render_context_init` (and the HWR counter's start from `pvr_render_init`) | `render.rs` (`ctx_init`) | 5 |
| `hwrt_get`, `npot2`, `multisamplectl` | `render.rs` | 5 |
| `ring_reserve`, `ring_put`, `mmu_cache_sync`, the MMUCACHE probes (`mc_*`), `pvr_render_submit` | `render.rs` (`submit`); `pvr_render_submit` a C wrapper | 5 |
| `render_wait_for`, `pvr_render_wait`, `pvr_render_completed`, the dumps (`debug_mappings`, `render_debug_after`, `dump_last_frag_cmd`, `dump_cccb_pending`, `nz_count`, `lookup_log`) | `render.rs` (`wait_for`, `completed`); C wrappers | 5 |
| the pool header (`pvr_render_pool.h`, with `driver/tools/pooltest` and `tools/pvr/render_pool_check.c`) | `pool.rs` (the free lists; a golden trace from the C proves the port, and its host tests replace both C checks); `pvr_render_alloc`, `pvr_render_free`, `pvr_render_pool_report` C wrappers | 2026-10-04 (tools/cts/log/investigations/pvr-render-pool.md) |
| `flush*`, `heap_bump`, `heap_base`, `pvr_render_map_phys` | C (ours), not ported | — |
| `static_areas_init`, `pvr_render_init`, `pvr_render_static_get` | C: they call the command-stream generators of group B (`pvr_pds_render_gen.c`, `pvr_pds_pixel_event_gen.c` over Mesa's PDS code) and size the parameter buffer from the carveout | — |
| the run-time switches (`pvr_render_combined_kick`, `_wait_repost`, `_mmu_ack`, ...), `pvr_render_debug_*`, `pvr_render_perf_read`, `pvr_render_stall_dump_rearm` | C shim (exported variables, copied into the Rust state at each call; counters read from it) | 5 |

Not in this task: `pvr_fw_trace_decode.c` (ours, on the DDK's
`rgx_fwif_sf.h`), the command-stream generators of group B (they call Mesa).

## Checks

Each step is proved on the host before the board: the C it replaces is cut
out of origin/main 674ea1cf by a recorder (`tools/pvr/km/`), run against a
recording platform (memory, cache maintenance, barriers, register bus), and its
record is committed; `tools/pvr/km/km_check.c` runs the same scenario on the
Rust modules (libyakogl_drivers.a for the host) and `tools/check.sh` requires
the two records to be identical, except where a step changes the result on
purpose (step 4's sizes), where the difference is committed and explained.

## Results

Every step was board-verified on the AM62P before the next
(tools/cts/RESULTS.md, "AM62P: the PowerVR kernel side in Rust"): the
firmware handshake as predicted at each step (step 4 moved the objects:
`objs from 0xc0000`, `heap used to 0xd8d28`), dEQP fbo + shaders.texture_functions
731/0/0/0 after every step, and after step 5 fragment_ops.blend 1060/0/0/0 and
the gl_test regression set identical per case to origin/main.

## Open

- After a freelist grew from the shared pool, FREELISTS_RECONSTRUCTION
  relists `backing_phys + i * 4 KiB` for every current page: pages past the
  list's own backing. Kept as the C had it; the host scenario's audit shows it.
- Not ported: the static PDS/USC areas (group B's
  generators), `pvr_selftest.c` (lab), `pvr_fw_trace_decode.c`.
- The C side keeps `pvr_drv.h` as wrappers; the self-test rig and YakoGL's
  OSAL call those, not the Rust entry points.
