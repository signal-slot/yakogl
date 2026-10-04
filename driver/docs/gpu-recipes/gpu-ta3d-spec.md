# GEOM(TA)+3D render — code-level reproduction spec (PowerVR BXS-4-64, BVNC 36.53.104.796)

Target: one screen-covering primitive filling a 64x64 RGBA8888 linear surface, produced by a
**fence-chained GEOM(TA)+3D pair** sharing one HWRTData — the structure the on-board known-good
`rgx_blit_test`/`rgx_triangle_test` use (both traces show `Kick TA` (DM=2) then `Kick 3D` (DM=3)).

Sources cited as `pvr-src/<file>:<line>` (mainline drm/imagination) and
`mesa:<path>:<line>` (Mesa 26.2.1 `src/imagination/...`), plus the HW capture (`fwtrace_blit.txt`).

## 0. THE STRUCTURAL GAP (read first)

Your current fill uses `ROGUE_FWIF_CCB_CMD_TYPE_TQ_3D` (`pvr-src/pvr_job.c:264`) — the **transfer
context** (`rogue_fwif_fwtransfercontext.tq_context`, `pvr-src/pvr_rogue_fwif.h:779`). Mesa's
transfer path (`mesa:vulkan/pvr_arch_job_transfer.c`) is **TQ-only: it emits ZERO VDM geometry**
(`grep -c VDMCTRL` = 0) and rasterises a hard-coded ISP-vertex background object. On this core a
lone TQ/3D over a render target whose region headers were never written by a TA tiles nothing → PBE
never emits (your exact symptom).

The known-good `rgx_blit_test` is NOT that path. Its trace is a **render-context GEOM+FRAG pair**:
`ROGUE_FWIF_CCB_CMD_TYPE_GEOM` (`pvr-src/pvr_job.c:158`, DM=2) followed by
`ROGUE_FWIF_CCB_CMD_TYPE_FRAG` (`pvr-src/pvr_job.c:188`, DM=3), both on one
`rogue_fwif_fwrendercontext` (`pvr-src/pvr_rogue_fwif.h:734`) that has a **`geom_context`** and a
**`frag_context`** common-context (`:736`,`:738`), sharing one **HWRTData**.

So: **switch from the transfer context to a render context, and add the whole GEOM side.** The
Mesa reference to mirror is the **static-clear path** (`mesa:vulkan/rogue/pvr_clear.c`) run through
the **render job** (`mesa:vulkan/pvr_arch_job_render.c`) — that path builds a real VDM control stream
+ PPP state (a full-screen rect), which is exactly a minimal TA+3D fill.

Single most important missing requirement: **a valid HWRTData shared by a GEOM kick and the 3D kick,
with the GEOM actually tiling one full-screen primitive (writing TE region headers) before the 3D**.
Freelists, VHeap/TPC and the VDM stream are all in service of that. Details per section.

---

## 1. HWRTData / RTData dataset

### 1.1 Structs (`pvr-src/pvr_rogue_fwif.h`)

- `struct rogue_fwif_hwrtdata_common` (`:2117`) — **one per dataset**, shared by both RTData copies
  and by GEOM+3D. Holds the tiling/screen registers.
- `struct rogue_fwif_hwrtdata` (`:2141`) — **`ROGUE_FWIF_NUM_RTDATAS` = 2** copies
  (`pvr-src/pvr_rogue_fwif_shared.h:10`); the kernel makes 2 (`pvr-src/pvr_hwrt.c:473`). A single
  render only needs one to be valid; the 2nd exists for double-buffered/parallel renders. In the
  trace the two live datasets are `0xc0104800` and `0xc0104500`.
- `struct rogue_fwif_rta_ctl` (`:2042`) — embedded in hwrtdata; all-zero for a single-layer render
  (`args->layers == 1`, so no SRTC/RAA objects — `pvr-src/pvr_hwrt.c:389`).

The GEOM and 3D commands reference the dataset by FW address in their shared header:
`struct rogue_fwif_cmd_geom_frag_shared` (`pvr-src/pvr_rogue_fwif_shared.h:81`) field
`hwrt_data_fw_addr` (`:90`). **Both cmd_geom and cmd_frag put the SAME hwrt_data_fw_addr here.**

### 1.2 `rogue_fwif_hwrtdata_common` — field values for 64x64, 1 sample, 1 layer

BXS-4-64 features (`mesa:common/device_info/bxs-4-64.h`): `tile_size = 16x16`,
`isp_samples_per_pixel = 4`, `simple_parameter_format_version = 2`, `roguexe`, `num_clusters = 1`,
`num_raster_pipes = 1`. So 64x64 → `num_tiles = 4x4` (matches trace "4x4 tiles"). The kernel derives
these in `hwrt_init_common_fw_structure()` (`pvr-src/pvr_hwrt.c:228-343`), SIPF-v2 branch
(`:249-270`). Register bitfields: `pvr-src/pvr_rogue_cr_defs_client.h`.

| field | formula (`pvr_hwrt.c`) | value @64x64 |
|---|---|---|
| `screen_pixel_max` | `(w-1)<<PIXXMAX(0) \| (h-1)<<PIXYMAX(16)` (`:308`) | `0x003F003F` |
| `te_screen` | `tile_max_x<<XMAX(0) \| tile_max_y<<YMAX(12)`, `tile_max = round_up(n_tiles,2)-1 = 3` (`:314`,`:269`) | `0x00003003` |
| `te_mtile1` | `mtile_x[0]<<X1(18)`, `mtile_x[0]=DIV_ROUND_UP(4,8)*2=2` (`:319`,`:263`) | `0x00080000` |
| `te_mtile2` | `mtile_y[0]<<Y1(18)`, `mtile_y[0]=2` (`:323`,`:264`) | `0x00080000` |
| `isp_mtile_size` | `x<<X(16) \| y<<Y(0)`, x=y=2 (single sample, spp=4 → no shift) (`:328`,`get_cr_isp_mtile_size_val:93`) | `0x00020002` |
| `mtile_stride` | `mtile_x[0]*mtile_y[0]` (`:302`) | `4` |
| `teaa` | single sample, spp=4 → 0 (`get_cr_te_aa_val:176`) | `0` |
| `multi_sample_ctl` | 1 sample → pos (8,8): `8 \| 8<<4` (`get_cr_multisamplectl_val:130`) | `0x0000000000000088` |
| `flipped_multi_sample_ctl` | same, y flipped (16-8=8) | `0x0000000000000088` |
| `tpc_stride` | from userspace `geom_data_args->tpc_stride` (`:333`) | `0` (1 layer) |
| `tpc_size` | `geom_data_args->tpc_size` (`:334`) | `128` (see §1.3) |
| `rgn_header_size` | `args->region_header_size` (`:336`) | `16` (region-header **count**, SIPF v2) |
| `isp_merge_lower/upper/scale_*` | from userspace; SIPF+16px tile → guard false → all 0 (`mesa:pvr_arch_job_render.c:555`) | `0` |
| `geom_caches_need_zeroing` | `false` (`:283`) | `false` |

### 1.3 `rogue_fwif_hwrtdata` — per-copy dev-addr fields and the buffers they point at

Filled in `hwrt_data_init_fw_structure()` (`pvr-src/pvr_hwrt.c:353-426`). All addresses are **GPU
virtual** in the FW/PM heaps. Values below are the buffer **sizes/alignments** you must allocate
(sizes from `mesa:vulkan/pvr_arch_job_render.c`, evaluated for 64x64/1x/1 layer). "INVALID" = leave
the field 0 / do not allocate.

| hwrtdata field | src | buffer to allocate | size @64x64 | align |
|---|---|---|---|---|
| `tail_ptrs_dev_addr` | `geom_data_args->tpc_dev_addr` (`:373`) | **TPC (tail-pointer cache)** | **128 B** (`get_tail_ptr_stride_size`: 8x8 mtiles → 64, v2 /4 → 16, ×`TAIL_POINTER_SIZE=8`) | 64 (`TE_TPC_ADDR_BASE`) |
| `vheap_table_dev_addr` | `geom_data_args->vheap_table_dev_addr` (`:374`) | **VHeap table** | **1536 B** (`VHEAP_TABLE_SIZE=384 × 4`) | 64 |
| `rtc_dev_addr` | `geom_data_args->rtc_dev_addr` (`:375`) | **RTC** (render-target cache) | **0 / INVALID** (only if layers>1) | — |
| `pm_mlist_dev_addr` | `rt_data_args->pm_mlist_dev_addr` (`:377`) | **MList** (PM linked-list) | **~4096 B/RT** (freelist-size dependent; see below) | 16 |
| `macrotile_array_dev_addr` | `rt_data_args->macrotile_array_dev_addr` (`:378`) | **Macrotile array (MTA)** | **0 / INVALID** (SIPF returns 0) | — |
| `rgn_header_dev_addr` | `rt_data_args->region_header_dev_addr` (`:379`) | **ISP region headers** | **128 B/RT** (stride; alloc 256 B for 2 RTs) | 64 (`TE_PSGREGION_ADDR_BASE`) |
| `freelists_fw_addr[0..1]` | FW addrs of the 2 freelists (`:368-371`) | see §2 | — | — |
| `hwrt_data_common_fw_addr` | FW addr of the common struct (`:365`) | the common struct BO | `sizeof(hwrtdata_common)` | uncached |
| `pm_mlist_stack_pointer`, `vce_*`, `te_*`, `alist_*`, `pm_alist_stack_pointer` | **zero-init** (PM fills at runtime) | — | 0 | — |
| `state` | `ROGUE_FWIF_RTDATA_STATE_NONE` (0) | — | 0 | — |

- **Region headers** (`rgn_header_dev_addr`) is the buffer the TA writes ("TE Region headers base
  0x...2bad00 (RGNHDR Init:1)" in the trace, `fwtrace_blit.txt:16`). stride/size:
  `mesa:pvr_arch_job_render.c:349` — `single_rgn_header_size=6` × grouped tiles → 96, `ALIGN_POT(...,64)
  = 128 B` per RT. **This is the single most important data buffer**: without a TA writing it, the 3D
  has nothing to rasterise.
- **MTA and RTC are NOT allocated** on this SIPF/single-layer core (both formulas return 0). The
  trace never references a macrotile array separately for these small renders.
- **MList** size: `mesa:pvr_arch_job_render.c:317` — `ALIGN_POT((pce+pde+pte)*NUM_PM_AS(2)*STRIDE(4),
  4096)`; with a minimal freelist this is one 4096-B page per RT.
- The three FW-object BOs (`common`, and each `hwrtdata` copy) are created
  `PVR_BO_FW_FLAGS_DEVICE_UNCACHED` (`pvr-src/pvr_hwrt.c:338`,`:407`).

Minimal buffer set to allocate for ONE working render (1 RTData copy is enough to kick):
TPC 128 B, VHeap 1536 B, MList 4096 B, region-headers 128 B, + the FW structs (`hwrtdata_common`,
one `hwrtdata`). All in a PM/FW-protected GPU-mapped heap.

---

## 2. Free list(s)

### 2.1 Struct + backing (`pvr-src/pvr_free_list.c`, `pvr-src/pvr_rogue_fwif.h:2061`)

`struct rogue_fwif_freelist` (`:2061`), FW-uncached, filled by `free_list_fw_init()`
(`pvr-src/pvr_free_list.c:168-191`):

| field | init | note |
|---|---|---|
| `freelist_dev_addr` | GPU VA of the freelist **array** BO (`:186`) | the u32 page-number stack |
| `current_dev_addr` | `freelist_dev_addr + (max-current)*4`, 16-aligned (`:187`) | FW stack cursor |
| `current_stack_top` | `current_pages - 1` (`:185`) | trace: `stacktop=0x19` = 25 |
| `max_pages` | `args->max_num_pages` (`:179`) | trace: `max=32` |
| `current_pages` | `initial - ready_pages` (`:180`) | trace: `26` |
| `grow_pages` | `args->grow_num_pages` (`:181`) | trace: `grow size=0` |
| `ready_pages` | `calculate_free_list_ready_pages` (`:182`) | 0 if grow=0 |
| `freelist_id` | `free_list->fw_id` (`:183`) | |
| `grow_pending` | false | |

### 2.2 Backing pages + sizing rules

- **PM physical page = 4 KB** (`ROGUE_BIF_PM_PHYSICAL_PAGE_ALIGNSHIFT=12`, `pvr-src/pvr_rogue_defs.h:67`).
- **Alignment**: page counts must be multiples of 4 pages
  (`FREE_LIST_ALIGNMENT = (16/4)-1 = 3`, `pvr-src/pvr_free_list.c:17`, since
  `ROGUE_BIF_PM_FREELIST_BASE_ADDR_ALIGNSIZE=16`, `pvr-src/pvr_rogue_defs.h:74`).
- **Minimum pages**: **roguexe → 25** (`FREE_LIST_MIN_PAGES_ROGUEXE`, `pvr-src/pvr_free_list.c:22`;
  BXS-4-64 has `roguexe`). Checked against `free_lists[LOCAL].current_pages`
  (`pvr-src/pvr_hwrt.c:60`). If BRN66011 present → 40; else default 50.
- **Backing store**: a GEM/carveout BO of `num_pages << 12` bytes, whose per-4K-page PFNs are written
  into the freelist array (`pvr_free_list_insert_pages_locked:222`).
- **The freelist array BO** (the `freelist_gpu_addr`): must be `>= max_num_pages * 4` bytes,
  PM_FW_PROTECT, no CPU-userspace access (`pvr-src/pvr_free_list.c:82-84`).

Ground truth (`fwtrace_blit.txt:12`): `FL Total Pages 26 (max=32, grow=0), FL-addr=0x...b010,
stacktop=0x19`. So the known-good uses **initial≈28 (mult of 4, ≥25) backed, max 32**, no grow. Use
`initial_num_pages = 28, max_num_pages = 32, grow_num_pages = 0, grow_threshold = 0` and back 28
pages (112 KB) + a 128-byte array BO. (28 satisfies the multiple-of-4 rule; the FW reports a slightly
different live "Total Pages" after its own bookkeeping.)

### 2.3 Local vs global; how HWRTData references them

- Two logical freelists per dataset: `ROGUE_FW_LOCAL_FREELIST=0`, `ROGUE_FW_GLOBAL_FREELIST=1`
  (`pvr-src/pvr_rogue_fwif_shared.h:137`). HWRT holds both
  (`free_lists[ROGUE_FWIF_NUM_RTDATA_FREELISTS=2]`, `pvr-src/pvr_hwrt.h:74`), and each `hwrtdata`
  copy records their FW addrs in `freelists_fw_addr[0..1]` (`pvr-src/pvr_hwrt.c:368-371`).
- The **local** freelist is the one that must meet the min-pages check and is grown by the TA
  (`pvr_free_list_add_hwrt(free_lists[0], ...)`, `pvr-src/pvr_hwrt.c:413`). The global can be
  minimal.
- The trace's **PMDM0 / PMDM1** are the two PM data-masters (TE/tiling vs VCE/alist) that both load
  the same local freelist during a render (`fwtrace_blit.txt:12`,`:41`); this is FW-internal — you
  supply the freelist once.
- The GEOM kick reaches the freelist **through the HWRTData** (via `hwrt_data_fw_addr` → the
  dataset's `freelists_fw_addr`); the freelist is **not** a field of `rogue_fwif_cmd_geom`.

**Minimal**: create 2 freelists (local 28/32 pages backed; global may be the same or a small 4-page
one), attach both to the HWRTData.

---

## 3. GEOM/TA context + kick

### 3.1 The FW context

The GEOM context is the `geom_context` member (`struct rogue_fwif_fwcommoncontext`,
`pvr-src/pvr_rogue_fwif.h:668`) inside the **one** `rogue_fwif_fwrendercontext`
(`:734`). It has its own CCCB (`ccbctl_fw_addr`, `ccb_fw_addr`), its own context-state BO, and
`dm = GEOM`. The FRAG context is the sibling `frag_context` (`:738`) on the **same** render context,
`dm = 3D`. (Your existing transfer context is a different object; do not reuse it.)
CCCB size: `1<<CTX_GEOM_CCCB_SIZE_LOG2 = 32 KB` (`pvr-src/pvr_queue.c:24`).

Context-switch/static state: `static_render_context_state` (`rogue_fwif_static_rendercontext_state`,
`pvr-src/pvr_rogue_fwif_shared.h:180`) — the VDM context store/resume register block. For a first
non-resuming kick these can be zero-backed but the BOs must exist.

### 3.2 `rogue_fwif_cmd_geom` register block (`pvr-src/pvr_rogue_fwif_client.h:98,136`)

Fields the client (you) supplies, in the exact order the kernel copies them
(`pvr-src/pvr_stream_defs.c:46-61`). Values from Mesa's geom-state init
(`mesa:vulkan/pvr_arch_job_render.c:720-789`; bitfields `mesa:csbgen/rogue/cr.xml`):

| `regs.` field | value | src |
|---|---|---|
| `vdm_ctrl_stream_base` (u64) | GPU VA of the VDM control stream (§3.4, word 0) | `CR_VDM_CTRL_STREAM_BASE.addr`, `job_render.c:733` |
| `tpu_border_colour_table` (u64) | GPU VA of a border-colour table BO (can be a zeroed BO) | `job_render.c:738` |
| `ppp_ctrl` (u32) | `wclampen(bit1)=1 \| fixed_point_format(bit9)=1` = **`0x00000202`** | `job_render.c:746` |
| `te_psg` (u32) | `completeonterminate(bit17)=1 \| region_stride(bits0-10)=rgn_headers_stride/4096` → **`0x00020000`** (stride 128 → 0) | `job_render.c:752` |
| `vdm_context_resume_task0_size` (u32) | `0` (no resume) | |
| `vdm_context_resume_task3_size` (u32) | present only if `VDM_OBJECT_LEVEL_LLS` — else omitted from stream | `client.h:113` |
| `view_idx` (u32) | `0` | `job_render.c:778` |
| `pds_coeff_free_prog` (u32) | present only if `TESSELLATION` — omit | `client.h:122` |
| `vdm_draw_indirect0/1` | present only if `VDM_DRAWINDIRECT` — omit | `client.h:102` |
| `tpu` (u32) | present only if BRN 49927 (ext stream) | `pvr-src/pvr_stream_defs.c:63` |

`flags` (`cmd_geom.flags`, `:150`): `ROGUE_GEOM_FLAGS_FIRSTKICK | ROGUE_GEOM_FLAGS_LASTKICK`
(`client.h:49-50`) — trace shows "First kick:1, Last kick:1" (`fwtrace_blit.txt:20`).
`cmd_shared.hwrt_data_fw_addr` = the dataset FW addr. `cmd_shared.pr_buffer_fw_addr[]` = 0.

### 3.3 VHeap + TPC buffers

These are the `geom_data_args` buffers already listed in §1.3 (VHeap 1536 B @64-align, TPC 128 B
@64-align). The trace confirms them: "TA Buffers: ... VHeap 0x...2bd200, TPC 0x...2c4000"
(`fwtrace_blit.txt:15`). They live in the PM heap; the TA writes tail pointers into TPC and uses
VHeap as the vertex-heap table. Allocate once per dataset.

### 3.4 VDM control stream for one full-viewport rect

The canonical screen-covering primitive is Mesa's **clear "background object"**: a **4-vertex
TRI_STRIP** (full-screen rect), packed by `pvr_pack_clear_vdm_state()`
(`mesa:vulkan/rogue/pvr_clear.c:853-959`). Structs from `mesa:csbgen/rogue/vdm.xml`
(BLOCK_TYPE: PPP=0, PDS=1, VDM=2, INDEX_LIST=3, LINK=4, TERMINATE=6). The stream, in order:

```
# A. PPP state pointer (points at the PPP state block built in §3.5)
VDMCTRL_PPP_STATE0 : block_type=PPP_STATE_UPDATE(0); word_count=<PPP dword count>; addrmsb=ppp_bo>>32
VDMCTRL_PPP_STATE1 : addrlsb=ppp_bo (shift 2)
# B. VDM state (binds the vertex USC + its PDS)
VDMCTRL_VDM_STATE0 : block_type=VDM_STATE_UPDATE(2); vs_data_addr_present=1; vs_other_present=1;
                     cam_size=<calc>; uvs_scratch_size_select=FIVE(0); flatshade_control=VERTEX_0
VDMCTRL_VDM_STATE2 : vs_pds_data_base_addr = <vertex PDS data offset>   (shift 4)
VDMCTRL_VDM_STATE3 : vs_pds_code_base_addr = <vertex PDS code offset>   (shift 4)
VDMCTRL_VDM_STATE4 : vs_output_size = DIV_ROUND_UP(vs_output_size_bytes,4)
VDMCTRL_VDM_STATE5 : vs_max_instances=<calc>; vs_usc_unified_size=DIV_ROUND_UP(3*4,16);
                     vs_pds_temp_size=DIV_ROUND_UP(temps,16); vs_pds_data_size=DIV_ROUND_UP(bytes,16)
# C. Index list — non-indexed, 4 verts, tri-strip
VDMCTRL_INDEX_LIST0 : block_type=INDEX_LIST(3); index_count_present=1; primitive_topology=TRI_STRIP(9)
VDMCTRL_INDEX_LIST2 : index_count = 4
# D. Terminate (appended by pvr_arch_csb_emit_terminate, mesa:vulkan/pvr_arch_csb.c:337)
VDMCTRL_STREAM_TERMINATE : block_type=STREAM_TERMINATE(6); context=0
```

The 4 vertices themselves come from the vertex PDS/USC (not an index/vertex buffer): the passthrough
VS reads a clear-rect vertex BO uploaded by `pvr_clear_vertices_upload()`
(`mesa:vulkan/rogue/pvr_clear.c:442`) — 4 corners at NDC extremes with a chosen depth. For a fill you
can set all corners to the viewport so the rect covers 0..64 in both axes.

### 3.5 PPP state block (required — sets ISP object type + full-screen region clip + output select)

Built by `pvr_emit_ppp_from_template()` (`mesa:vulkan/rogue/pvr_clear.c:165-248`); structs
`mesa:csbgen/rogue/ppp.xml` (`TA_STATE_*`). Minimal ordered contents:

```
TA_STATE_HEADER      : pres_ispctl, pres_ispctl_fa/fb, pres_region_clip, pres_wclamp,
                       pres_outselects, pres_varying_word0/1/2, pres_ppp_ctrl, pres_stream_out_size,
                       (pres_pds_state_ptr0/1/2 iff a color/fragment PDS is bound)
TA_STATE_ISPCTL      : bpres=1; tagwritedisable = !has_color   (0 for a color fill)
TA_STATE_ISPA        : objtype=TRIANGLE; passtype=TRANSLUCENT/OPAQUE; dwritedisable=1(no depth);
                       dcmpmode=ALWAYS
TA_STATE_ISPB        : scmpmode=ALWAYS; sop1/2/3=KEEP; swmask=0
[TA_STATE_PDS_SHADERBASE,_TEXUNICODEBASE,_SIZEINFO1,_SIZEINFO2,_VARYINGBASE,_TEXTUREDATABASE]
                       # 6 dwords — the FRAGMENT PDS pointers, present iff has_color (see §4)
TA_REGION_CLIP0      : mode=OUTSIDE; left=0; right=ROGUE_TA_REGION_CLIP_MAX   # full screen
TA_REGION_CLIP1      : top=0; bottom=ROGUE_TA_REGION_CLIP_MAX
TA_WCLAMP            : val = fui(1.0e-13f)
TA_OUTPUT_SEL        : vtxsize=4; rhw_pres=1
TA_STATE_VARYING0/1/2: 0
TA_STATE_PPP_CTRL    : pretransform=1; cullmode=NO_CULLING
TA_STATE_STREAM_OUT0 : 0
```

`word_count` for `VDMCTRL_PPP_STATE0` = the number of dwords written here
(`mesa:vulkan/rogue/pvr_clear.c:170`). The `TA_STATE_PDS_*` six dwords are how the FRAGMENT shader is
bound into the tiled primitive — this is where a real (non-transfer) render carries its pixel PDS.

---

## 4. Vertex/position + fragment USC shaders

Verdict table (Mesa 26.2.1 PCO):

| shader | canned or capture | artifact |
|---|---|---|
| **Vertex (position)** | **CANNED** | `VS_PASSTHROUGH_COMMON`, 8 dwords — saved to `pvr_usc_vs_passthrough_blob.h` (this dir). Src `mesa:pco/usclib/common.cl:29`, bytes `mesa:build/.../pco/uscgen/pco_uscgen_programs.c:24`. temps=0. |
| **Fragment (constant/gradient color)** | **MUST CAPTURE** | `pvr_uscgen_clear_attach()` — no source-array bytes exist. Recipe below. |
| Fragment NOP (reference) | canned | `FS_NOP_COMMON` 2 dw `0x00ee8004, 0xfffffff2` (writes no color — background-object dummy only). |

The color is **not** baked into the fragment USC: `pvr_uscgen_clear_attach`
(`mesa:vulkan/pvr_usc.c:1283`) emits, per output dword, `frag_store(load_preamble(shared_reg))` — it
reads the color from **shared registers** `PVR_CLEAR_ATTACH_DATA_DWORD0..3`
(`mesa:common/pvr_iface.h:62`). A per-render PDS "uniform-texture" program DMAs your color into those
shared regs (§4.3(c)). One captured FS blob therefore serves any solid color; a gradient additionally
needs iterated varyings (coefficient program §4.3(d)) or a bespoke compiled FS.

### 4.1 Fragment capture recipe (mirror your `capture_eot_usc.c`)

```c
#include "pco/pco.h"
#include "pvr_usc.h"           /* pvr_uscgen_clear_attach, struct pvr_clear_attach_props */
#include "pvr_device_info.h"
extern const struct pvr_device_info pvr_device_info_36_53_104_796;

void *mc = ralloc_context(NULL);
struct pvr_device_runtime_info rt = {0};
pco_ctx *ctx = pco_ctx_create(&pvr_device_info_36_53_104_796, &rt, mc);

struct pvr_clear_attach_props props = { .dword_count = 1, .offset = 0, .uses_tile_buffer = false };
pco_shader *fs = pvr_uscgen_clear_attach(ctx, &props);       /* mesa:vulkan/pvr_usc.c:1283 */
const uint8_t *bin = pco_shader_binary_data(fs);
unsigned len   = pco_shader_binary_size(fs);
unsigned temps = pco_shader_data(fs)->common.temps;          /* feed to pvr_pds_setup_doutu */
fwrite(bin, 1, len, stdout);
```

`dword_count=1, offset=0` = the index-0 variant, one 32-bit RGBA output. Record `bin/len` and
`temps`. Zero-tool alternative: `PCO_DEBUG=binary vkCmdClearAttachments` dumps the same bytes
(`pco_binary.c`). For a true gradient, build NIR and run `build_shader()` (`mesa:vulkan/pvr_usc.c:41`).

### 4.2 What "captured" means for your bring-up

You already captured the **USC EOT** the same way. You now need **one more captured USC blob** (the
clear-attach FS). The vertex shader you can copy verbatim from the header. So: **1 blob to copy
(VS) + 1 blob to capture (FS)**, plus the EOT you already have.

### 4.3 PDS programs (all portable C in `mesa:vulkan/pvr_pds.c`, protos `pvr_pds.h`; no VkDevice)

- **(a) Vertex PDS** (kicks the VS, feeds VDM_STATE2/3): `pvr_pds_vertex_shader()`
  (`pvr_pds.h:730`), struct `pvr_pds_vertex_shader_program` (`pvr_pds.h:432`). Convenience for the
  clear rect: `pvr_pds_clear_vertex_shader_program_init_base()` +
  `..._create_and_upload()` (`mesa:vulkan/rogue/pvr_clear.c:628`,`:658`).
- **(b) Fragment kick-USC PDS** (loads the FS): `pvr_pds_setup_doutu()` (`pvr_pds.h:650`;
  `execution_address` = FS USC-heap offset, `usc_temps` = the captured `temps`,
  `sample_rate = INSTANCE`) → `pvr_pds_kick_usc()` (`pvr_pds.h:675`), struct
  `pvr_pds_kickusc_program` (`pvr_pds.h:236`). The resulting data/code offsets go into the PPP
  `TA_STATE_PDS_*` block (§3.5). Usage model: `mesa:vulkan/rogue/pvr_clear.c:319-393`.
- **(c) Color shared-reg upload PDS** (DMAs the fill color into the shared regs the FS reads):
  `pvr_pds_pixel_shader_uniform_texture_data()` / `..._code()` (`pvr_pds.h:707`,`:714`), struct
  `pvr_pds_pixel_shader_sa_program` (`pvr_pds.h:193`, `num_texture_dma_kicks=1`). Used at
  `mesa:vulkan/rogue/pvr_clear.c:325-393`.
- **(d) Coefficient/varying PDS** — only for a gradient: `pvr_pds_coefficient_loading()`
  (`pvr_pds.h:781`). For a flat fill set `TA_STATE_PDS_VARYINGBASE = 0` and skip.
- **(e) Pixel-event / EOT PDS** — you already have this (`pvr_pds_pixel_event_gen.c`,
  `pvr_pds.h:769`). Its offsets go into `cmd_frag.regs.event_pixel_pds_{info,data}` (§5).

---

## 5. 3D kick WITH geometry (`rogue_fwif_cmd_frag`, `pvr-src/pvr_rogue_fwif_client.h:246`)

### 5.1 How it differs from your lone TQ_3D

Your TQ used `rogue_fwif_cmd_transfer` (`client.h:360`). The render FRAG uses
`rogue_fwif_cmd_frag` (`client.h:246`) whose `cmd_shared.hwrt_data_fw_addr` **references the same
HWRTData the GEOM tiled** (`pvr-src/pvr_rogue_fwif_shared.h:90`). That single link is what makes the
3D consume the TA-written region headers/primitive lists instead of empty tiles.

Client-supplied FRAG regs, in kernel copy order (`pvr-src/pvr_stream_defs.c:97-134`):
`isp_scissor_base, isp_dbias_base, isp_oclqry_base, isp_zlsctl, isp_zload_store_base,
isp_stencil_load_store_base, pbe_word[8][3], tpu_border_colour_table, pds_bgnd[3], pds_pr_bgnd[3],
usc_clear_register[8], usc_pixel_output_ctrl, isp_bgobjdepth, isp_bgobjvals, isp_aa, isp_ctl,
event_pixel_pds_info, view_idx, event_pixel_pds_data` (+ `isp_oclqry_stride`, `execute_count` because
`GPU_MULTICORE_SUPPORT`). Note `pixel_phantom` is **absent** (no `CLUSTER_GROUPING` feature).

Most ISP/PBE/USC values are **the ones you already derived** for the TQ fill (they were correct —
your `gpu-fill-groundtruth.md`): `usc_clear_register0 = fill color (0xFF0000FF)`,
`pbe_word[0][0..2] = {0x0200007C68800000, 0x0000003F0000003F, 0}`,
`event_pixel_pds_info = 0x202`, `isp_bgobjvals`, `pds_bgnd*=0`, EOT via `event_pixel_pds_{code,data}`.
The **new** requirements are: (a) `hwrt_data_fw_addr` set; (b) `isp_ctl` gains bit 31 (below);
(c) `pbe_word` is `[8][3]` here (renders) vs `pbe_wordx_mrty[9]` in transfer — same values, different
array shape; (d) `isp_zlsctl`/z bases point at valid (even if dummy) buffers if depth flags set (for a
color-only fill leave depth flags clear).

`flags` (`cmd_frag.flags`, `:251`): 0 for a plain color render (no `DEPTHBUFFER`/`STENCILBUFFER`/
`GET_VIS_RESULTS`). `zls_stride`/`sls_stride` = 0.

### 5.2 `ISPCtl = 0x80035000` decode (`pvr-src/pvr_rogue_cr_defs.h:2266-2315`)

| bits | field | value | meaning |
|---|---|---|---|
| 31 | `SKIP_INIT_HDRS` (`:2267`) | **1** | **the bit you are missing.** Tells ISP the region headers are already initialised **by the TA** — use them, don't re-init to empty. Only valid because a GEOM ran. |
| 17 | `PROCESS_EMPTY_TILES` (`:2304`) | 1 | still set in the known-good render (background clear covers empties) |
| 16 | `SAMPLE_POS` (`:2307`) | 1 | sample-position enable |
| 15:12 | `PIPE_ENABLE` (`:2310`) | 5 (`0x5000`) | active ISP pipes mask for this core |

So `0x80035000 = SKIP_INIT_HDRS | PROCESS_EMPTY_TILES | SAMPLE_POS | PIPE_ENABLE(0x5)`.
Your `0x25000 = PROCESS_EMPTY_TILES | PIPE_ENABLE(0x5)` — you are missing **bit31 (0x80000000)** and
**bit16 (0x10000)**. Add `0x80010000` → `0x35000 | 0x80000000 = 0x80035000`. **bit31 is the
load-bearing one**: it is only correct when a TA has written the headers; setting it on a lone 3D
would read garbage headers.

### 5.3 PartitionMask / Dusts / USCTiles (informational — FW-computed, not client fields)

The trace's "Dusts=1, PartitionMask=0x5, Phantom 0: USCTiles=6" (`fwtrace_blit.txt:51-52`) are FW
runtime values derived from `num_clusters=1` and the render, printed while updating tiles-in-flight.
You do not set them directly; they follow from a correct HWRTData + non-empty tiles. Their presence
(USCTiles>0) is your success signal that the ISP actually processed tiles.

---

## 6. Submission + fencing (`pvr-src/pvr_queue.c`, `pvr-src/pvr_cccb.c`)

### 6.1 The two contexts relate as ONE render context

GEOM and FRAG are two `pvr_queue`s on the same `pvr_context` (render context). They are kicked with a
**single combined KCCB command** `ROGUE_FWIF_KCCB_CMD_COMBINED_GEOM_FRAG_KICK`
(`pvr-src/pvr_cccb.c:241`, enum `pvr-src/pvr_rogue_fwif.h:1193`) carrying both kick-data blocks. (A
non-paired path exists — two separate `ROGUE_FWIF_KCCB_CMD_KICK`s — but the driver's normal render is
the combined kick; the FW still runs GEOM then, after the fence, 3D.)

### 6.2 Per-job CCCB command sequence (`pvr_queue_submit_job_to_cccb`, `pvr-src/pvr_queue.c:661-738`)

For **each** job the CCCB gets: `[FENCE_PR (wait UFOs)]  [job cmd]  [UPDATE (signal UFO)]`.

**GEOM CCCB** (geometry queue's CCCB):
```
(optional FENCE_PR on external deps)
CMD_TYPE_GEOM  : the rogue_fwif_cmd_geom (§3)          # pvr_queue.c:730
CMD_TYPE_UPDATE: [geom_timeline_ufo.addr] = geom_done_seqno   # pvr_queue.c:736  -> trace UFO Update
```
If paired, the GEOM cmd also carries a `partial_render_geom_frag_fence` = (frag_done_seqno - 1)
(`pvr_queue.c:718-727`) — for a first, non-partial render this is benign.

**FRAG CCCB** (fragment queue's CCCB):
```
FENCE_PR : [geom_timeline_ufo.addr] requires >= geom_done_seqno   # pvr_queue.c:698-716
CMD_TYPE_FRAG : the rogue_fwif_cmd_frag (§5)
CMD_TYPE_UPDATE: [frag_timeline_ufo.addr] = frag_done_seqno
```

This is exactly the trace: `UFO PR-Check [0x4013700c] requires >= 0x1ed` (frag waits),
`UFO Update [0x4013700c] = 0x1ed` (geom signals), then `UFO PR-Check ... is 0x1ed requires >= 0x1ed`
satisfied → 3D runs (`fwtrace_blit.txt:33-36,49-56`). **The UFO address is the GEOM queue's
timeline_ufo FW addr; the value is the GEOM job's done-fence seqno.** The 3D's PR-Check value must be
`>=` the value the TA's UPDATE writes.

### 6.3 The two KICK kick_data blocks (`fill_cmd_kick_data`, `pvr-src/pvr_cccb.c:198-211`)

`struct rogue_fwif_kccb_cmd_kick_data` (`pvr-src/pvr_rogue_fwif.h:871`), one for GEOM one for FRAG,
wrapped in `rogue_fwif_kccb_cmd_combined_geom_frag_kick_data` (`:890`):

| field | GEOM value | FRAG value |
|---|---|---|
| `context_fw_addr` | geom ctx FW addr + geom `ctx_offset` | frag ctx FW addr + frag `ctx_offset` |
| `client_woff_update` | geom CCCB write offset after the writes above | frag CCCB write offset |
| `client_wrap_mask_update` | geom CCCB `wrap_mask` | frag CCCB `wrap_mask` |
| `num_cleanup_ctl` / `cleanup_ctl_fw_addr[]` | 1 → `&hwrtdata.cleanup_state` FW addr (`pvr-src/pvr_cccb.c:205-210`, offset `offsetof(rogue_fwif_hwrtdata, cleanup_state)`) | same HWRT cleanup (unless partial-render, then NULL) |
| `work_est_cmd_header_offset` | 0 | 0 |

Both kick blocks reference the **same** HWRT (`pvr-src/pvr_cccb.c:254`,`:260`). Send with
`pvr_cccb_send_kccb_combined_kick(pvr_dev, geom_cccb, frag_cccb, geom_ctx_fw_addr, frag_ctx_fw_addr,
hwrt, frag_is_pr=false)` (`pvr-src/pvr_queue.c:794`). Memory barrier (`wmb()`) before the kick
(`pvr-src/pvr_cccb.c:263`).

---

## 7. Ordered build list (from what you have → working TA+3D fill)

You already have: power/reset/MMU/FW-boot/KCCB, GPU-MMU mapper, carveout allocator, `pvr_fwif.h`
structs, USC EOT blob + its pixel-event PDS, CCCB + UFO + KICK plumbing, and a correct TQ ISP/PBE/USC
register set. Build order, each with its trace-verifiable checkpoint:

1. **Render context.** Allocate one `rogue_fwif_fwrendercontext` (`pvr-src/pvr_rogue_fwif.h:734`)
   with two common contexts (`geom_context` DM=GEOM, `frag_context` DM=3D), each with its own CCCB
   (32 KB, `CTX_*_CCCB_SIZE_LOG2=15`) + CCCB-ctl + context-state BO + a timeline UFO.
   *Verify*: FW OS config lists "Ctx switch: ...; GEOM; 3D; ..." and both FWCtx addrs appear.

2. **Freelists (§2).** Create local (28 pages backed / max 32, 4-page aligned, ≥25) + global; write
   page PFNs into each array BO; build `rogue_fwif_freelist` FW structs.
   *Verify*: trace `Load Freelist ... FL Total Pages ~26 (max=32), stacktop=0x19`.

3. **HWRTData buffers (§1.3).** Allocate TPC(128)/VHeap(1536)/MList(4096)/region-headers(128), and
   a border-colour table BO. (No MTA/RTC on this core.)

4. **HWRTData FW structs (§1).** Fill `rogue_fwif_hwrtdata_common` with the 64x64 register values
   (§1.2) and one `rogue_fwif_hwrtdata` with the dev-addrs + `freelists_fw_addr[0..1]` +
   `state=NONE`. Attach the local freelist (`add_hwrt`).
   *Verify (after GEOM)*: `TA RTData <addr> loaded`, `TE Region headers base <addr> (RGNHDR Init:1)`.

5. **Shaders/PDS (§4).** Copy the VS passthrough blob (`pvr_usc_vs_passthrough_blob.h`); **capture
   the clear-attach FS** blob; build: vertex PDS (`pvr_pds_vertex_shader`), fragment kick-USC PDS
   (`pvr_pds_setup_doutu`+`pvr_pds_kick_usc`), color-upload PDS
   (`pvr_pds_pixel_shader_uniform_texture_*`). Upload all to USC/PDS heaps.

6. **PPP state + VDM control stream (§3.4/3.5).** Emit the PPP state block (ISPA=TRI, full-screen
   region clip, output select, and the `TA_STATE_PDS_*` fragment-PDS pointers), then the VDM stream
   (PPP_STATE0/1 → VDM_STATE0/2/3/4/5 → INDEX_LIST0(TRI_STRIP)+INDEX_LIST2(count=4) → TERMINATE).
   Upload; the stream's base VA → `cmd_geom.regs.vdm_ctrl_stream_base`.

7. **Build `rogue_fwif_cmd_geom` (§3.2).** Set regs (`ppp_ctrl=0x202`, `te_psg=0x20000`,
   `vdm_ctrl_stream_base`, `tpu_border_colour_table`), `flags=FIRSTKICK|LASTKICK`,
   `cmd_shared.hwrt_data_fw_addr = dataset`.

8. **Build `rogue_fwif_cmd_frag` (§5).** Reuse your TQ ISP/PBE/USC/EOT values, but:
   `cmd_shared.hwrt_data_fw_addr = dataset`; `isp_ctl = 0x80035000` (add bit31 SKIP_INIT_HDRS +
   bit16); pack `pbe_word[8][3]` (your 3 words in [0], rest 0); depth flags clear.

9. **Submit + fence (§6).** Into the GEOM CCCB: `GEOM cmd` + `UPDATE(geom_ufo)=g`. Into the FRAG
   CCCB: `FENCE_PR(geom_ufo)>=g` + `FRAG cmd` + `UPDATE(frag_ufo)=f`. Fill both kick-data blocks
   (context addrs, woff, wrap_mask, `num_cleanup_ctl=1`→HWRT cleanup_state), `wmb()`, send
   `COMBINED_GEOM_FRAG_KICK`.
   *Verify (GEOM)*: `Kick TA ... First kick:1 Last kick:1` → `TA finished` → `Perform TPC flush` →
   `UFO Update [geom_ufo]=g`.
   *Verify (3D)*: `UFO PR-Check ... requires >= g` satisfied → `3D Buffers RTData <addr> on ctx 1`
   → `Updating Tiles In Flight (Dusts=1, PartitionMask=0x5, ISPCtl=0x80035000)` →
   `Phantom 0: USCTiles=6` (>0!) → `Kick 3D ... Partial render:0` → `3D finished,
   HWRTData0State=7fffffff`.
   *Verify (result)*: the 64x64 destination surface now holds the fill color; PIXELBE_END_RENDER
   fires (USCTiles>0 means the ISP+PBE ran per tile, unlike your empty-tile lone-3D).

If USCTiles=0 or `HWRTData0State != 0x7fffffff`: the GEOM didn't tile a primitive — check the VDM
stream (index_count=4, tri-strip), the PPP region clip (full screen), and that `vdm_ctrl_stream_base`
+ `hwrt_data_fw_addr` are correct and the freelist met min-pages.
