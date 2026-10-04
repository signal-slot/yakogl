# Minimal "GPU writes a solid color to a buffer" — code-level spec

Target: bare-metal Zephyr port of the mainline `drivers/gpu/drm/imagination` driver, PowerVR
**BXS-4-64 (BVNC 36.53.104.796)**, RISC-V firmware. FW is up (`firmware_started=1`), KCCB works.
Goal: submit one job that makes the GPU write a constant color into a DDR buffer, with **no Mesa,
no Vulkan/GLES** — all job content hand-built.

Path conventions below:
- `pvr-src/…` = the ported kernel driver (local).
- `mesa-src/…` = Mesa userspace (`gitlab.freedesktop.org/mesa/mesa`, `src/imagination/vulkan/…`),
  used only to learn what the *content* of the streams must be. The kernel never builds this content;
  it copies a user-supplied "stream" into a FW struct (see §3).

---

## 0. TL;DR verdict

- **Chosen job type: TRANSFER (`DRM_PVR_CTX_TYPE_TRANSFER_FRAG` → `ROGUE_FWIF_CCB_CMD_TYPE_TQ_3D`), in FILL mode.**
  It is the only "fill" path that needs **no HWRT dataset and no free lists** (unlike FRAGMENT/3D)
  and **no compute kernel** (unlike COMPUTE). It reuses the 3D ISP+PBE hardware but with a tiny,
  self-contained ISP control stream.
- **Is a USC shader unavoidable? Almost, but only a trivial fixed one.**
  A *solid fill* needs **no per-pixel / compiled fill shader** — the color is delivered by the
  hardware **`CR_USC_CLEAR_REGISTER0..3`** registers, and the background-object PDS (`pds_bgnd*`)
  is left null (`mesa-src/pvr_arch_job_transfer.c:2828-2875`, `:651-694`).
  **BUT** the Pixel Back End (PBE) only writes to memory what a USC **`emitpix`** instruction hands
  it, kicked by a PDS "pixel-event"/end-of-tile (EOT) program. So you need exactly **one tiny fixed
  USC EOT program** (`emitpix(pbe_word0, pbe_word1); end` — one `emitpix` per render target) plus its
  PDS kick program (`mesa-src/pvr_usc.c:87-175` `pvr_usc_eot`, `mesa-src/pvr_arch_job_transfer.c:1152-1242`
  `pvr_pbe_setup_emit`). This EOT program is **color-independent and geometry-independent** — build it
  once. There is no way to make the PBE write without a USC emit.
- **Ordered build list** is in §7.

---

## 1. Which job type is minimal?

The kernel exposes four DM (data-master) job types. Selection + which FW command struct is built:

| UAPI job type | ctx type required | `fw_ccb_cmd_type` | FW cmd struct built | needs HWRT? | init site |
|---|---|---|---|---|---|
| `DRM_PVR_JOB_TYPE_GEOMETRY` | RENDER | `ROGUE_FWIF_CCB_CMD_TYPE_GEOM` (0x2ABC80C9) | `rogue_fwif_cmd_geom` | **yes** | `pvr-src/pvr_job.c:142-169` |
| `DRM_PVR_JOB_TYPE_FRAGMENT` | RENDER | `ROGUE_FWIF_CCB_CMD_TYPE_FRAG` (0x2ABC80CB) | `rogue_fwif_cmd_frag` | **yes** (`:184-185`) | `pvr-src/pvr_job.c:171-200` |
| `DRM_PVR_JOB_TYPE_COMPUTE` | COMPUTE | `ROGUE_FWIF_CCB_CMD_TYPE_CDM` (0x2ABC80CD) | `rogue_fwif_cmd_compute` | no | `pvr-src/pvr_job.c:215-238` |
| **`DRM_PVR_JOB_TYPE_TRANSFER_FRAG`** | **TRANSFER_FRAG** | **`ROGUE_FWIF_CCB_CMD_TYPE_TQ_3D` (0x2ABC80CA)** | **`rogue_fwif_cmd_transfer`** | **no** | **`pvr-src/pvr_job.c:251-274`** |

Constant derivation: `ROGUE_CMD_MAGIC_DWORD = 0x2ABC` → `_SHIFTED = 0x2ABC0000`
(`pvr-src/pvr_rogue_fwif.h:~790`); task DMs also OR `ROGUE_CCB_TYPE_TASK = BIT(15) = 0x8000`
(`pvr-src/pvr_rogue_fwif.h:1506,1511-1531`). So `TQ_3D = 202 | 0x2ABC0000 | 0x8000 = 0x2ABC80CA`.

### (a) TRANSFER / TQ_3D (the "TDM" question)
Note: the kernel's `TRANSFER_FRAG` context maps to **`TQ_3D`** (the 3D-based transfer queue), **not**
`TQ_TDM` (`pvr-src/pvr_job.c:264`). The true 2D/TDM data master (`ROGUE_FWIF_CCB_CMD_TYPE_TQ_TDM`,
`rogue_fwif_fwtdmcontext`) exists in the FWIF (`pvr-src/pvr_rogue_fwif.h:1521`, `:769-777`) but the
mainline kernel driver **has no context type or stream def that submits it** — you cannot reach TDM
without adding a new context/stream path. So on this driver, "transfer" = ISP+PBE (3D pipe), driven
by an ISP control stream instead of a VDM geometry stream.

Why TRANSFER is minimal: `create_job()` forbids HWRT on anything but GEOM/FRAG
(`pvr-src/pvr_job.c:414-417`); transfer context creation forbids static context state
(`init_transfer_fw_objs()` requires `static_context_state_len==0`, `pvr-src/pvr_context.c:127-135`).
The FILL sub-path builds no shader at all except the shared EOT emit
(`mesa-src/pvr_arch_job_transfer.c:2828-2875`).

### (b) FRAGMENT / 3D "background clear"
Heaviest. `pvr_frag_job_fw_cmd_init()` hard-requires `job->hwrt` (`pvr-src/pvr_job.c:184-185`).
A HWRT dataset requires **two free lists** (local+global) with a minimum page count
(`pvr-src/pvr_hwrt.c:52-63`), plus a `rogue_fwif_hwrtdata` with region-header memory, TPC buffer,
16-macrotile config, parameter-management buffers (`pvr-src/pvr_hwrt.c:249-370`). It also needs the
PM/parameter buffer and the VDM/TA to have run first (geometry) unless you carefully set up a
"background-object-only" render. Far more hand-built state than a fill. Reject for a first milestone.

### (c) COMPUTE / CDM
No HWRT, but requires a real **CDM control stream** + a **USC compute kernel** that does a global
`store` to the buffer, plus the CDM PDS programs and `cdm_ctrl_stream_base`
(`rogue_fwif_cmd_compute.regs`, `pvr-src/pvr_rogue_fwif_client.h:265-315`;
stream def `pvr-src/pvr_stream_defs.c:177-199`). This means writing and encoding a genuine USC kernel
(more instructions than the fixed EOT emit). More content than TRANSFER-FILL, but it is the second
choice if you would rather write a compute kernel than build an ISP control stream. It also needs a
"static compute context state" (`ctxswitch_regs`, `pvr-src/pvr_context.c:107-125`).

**Decision: TRANSFER_FRAG / TQ_3D, FILL mode.**

---

## 2. Context + CCCB setup (objects required to submit ONE transfer job)

### 2.1 The FW transfer context
`get_fw_obj_size(TRANSFER_FRAG)` → `sizeof(struct rogue_fwif_fwtransfercontext)`
(`pvr-src/pvr_context.c:62-63`), which is just:

```c
struct rogue_fwif_fwtransfercontext {          /* pvr_rogue_fwif.h:779-782 */
    struct rogue_fwif_fwcommoncontext tq_context;
} __aligned(8);
```

So `queue->ctx_offset = offsetof(fwtransfercontext, tq_context) = 0`
(`pvr-src/pvr_queue.c:100-101`). One `rogue_fwif_fwcommoncontext` is the whole thing; **no static
context state** is copied (`init_transfer_fw_objs`, `pvr-src/pvr_context.c:127-135`).

`struct rogue_fwif_fwcommoncontext` layout (fields the kernel MUST fill —
`pvr-src/pvr_rogue_fwif.h:668-730`, filled by `init_fw_context()` `pvr-src/pvr_queue.c:1087-1108`):

```c
struct rogue_fwif_fwcommoncontext {
    u32 ccbctl_fw_addr;      // = cccb->ctrl_fw_addr      (CCCB control FW vaddr)
    u32 ccb_fw_addr;         // = cccb->cccb_fw_addr      (CCCB base FW vaddr)
    struct rogue_fwif_dma_addr ccb_meta_dma_addr;         // {u64 dev_addr; u32 fw_addr; u32 pad} — 0 for non-META DMA
    u32 context_state_addr __aligned(8); // = FW addr of queue->reg_state_obj (frag ctx-switch state)
    u32 fw_com_ctx_flags;    // 0
    u32 priority;            // = ctx->priority (e.g. PVR_CTX_PRIORITY_MEDIUM)
    u32 priority_seq_num;    // 0
    u32 rf_cmd_addr __aligned(8);          // 0 (reset framework unused)
    bool stats_pending __aligned(4); s32 stats_num_stores; s32 stats_num_out_of_memory;
    s32 stats_num_partial_renders;
    u32 dm;                  // = PVR_FWIF_DM_FRAG (3)  — get_dm_type(): TRANSFER_FRAG→FRAG (pvr_queue.c:1063-1073, pvr_rogue_fwif_common.h)
    aligned_u64 wait_signal_address;       // 0
    struct rogue_fwif_dllist_node wait_signal_node __aligned(8);   // {0,0}
    struct rogue_fwif_dllist_node buf_stalled_node __aligned(8);   // {0,0}
    aligned_u64 cbuf_queue_ctrl_addr;      // 0
    aligned_u64 robustness_address;        // 0
    u32 max_deadline_ms;     // = MAX_DEADLINE_MS (30000)  (pvr_queue.c:1101)
    bool read_offset_needs_reset;
    struct rogue_fwif_dllist_node waiting_node __aligned(8);       // {0,0}
    struct rogue_fwif_dllist_node run_node __aligned(8);           // {0,0}
    struct rogue_fwif_ufo last_failed_ufo;                          // {0,0}
    u32 fw_mem_context_fw_addr; // = FW addr of the VM's FW mem-context obj (pvr_vm_get_fw_mem_context)
    u32 server_common_context_id; // = ctx->ctx_id
    u32 pid;                 // any nonzero id
    bool geom_oom_disabled __aligned(4);
} __aligned(8);
```

The whole FW context object is created as **device-uncached** and the FW address is obtained with
`pvr_fw_object_get_fw_addr(ctx->fw_obj, …)` (`pvr-src/pvr_context.c:335-336`). The
`context_state_addr` points at `queue->reg_state_obj`, a **frag context-switch state** buffer whose
size is `get_xfer_ctx_state_size()` (`pvr-src/pvr_queue.c:27-43`): for a core with
`xe_memory_hierarchy`, `= struct_size(struct rogue_fwif_frag_ctx_state, frag_reg_isp_store, 1)` ≈
**16 bytes** (3×u32 + 1×u32). Contents can be all-zero (`reg_state_init` only special-cases GEOMETRY,
`pvr-src/pvr_queue.c:1233-1243`).

Supporting objects that must also exist for the context/queue (all in your FW-mapped space):
- **VM context** with a **FW memory context** object (`fw_mem_context_fw_addr`) — the page-catalog the
  GPU MMU is already using. (`pvr_vm_get_fw_mem_context`, `pvr-src/pvr_queue.c:1090,1105`.)
- **timeline UFO** — one u32, device-uncached, one per queue; this is the completion signal (§4).
  (`pvr-src/pvr_queue.c:1358-1366`.)

### 2.2 The Client CCB (CCCB) — separate from the KCCB
The CCCB is a per-queue ring in FW-mapped memory; the KCCB is the single kernel→FW mailbox.
Size = `1 << CTX_TRANSFER_CCCB_SIZE_LOG2 = 1<<15 = 32 KiB` (`pvr-src/pvr_queue.c:25`,
`pvr_cccb_init` `pvr-src/pvr_cccb.c:45-84`). Two FW objects, both **device-uncached**:

```c
struct rogue_fwif_cccb_ctl {        /* pvr_rogue_fwif_shared.h */
    u32 write_offset;   // host-owned, 16-byte aligned; bumped after writing cmds
    u32 read_offset;    // FW-owned
    u32 dep_offset;     // FW-owned
    u32 wrap_mask;      // = size-1 (0x7FFF)
    u32 read_offset2, read_offset3, read_offset4;  // AGP only, 0
    u32 padding;
} __aligned(8);                     // 32 bytes
```
Init sets `write=read=dep=0`, `wrap_mask=size-1` (`pvr-src/pvr_cccb.c:22-32`).

**Writing a command** into the CCCB (`pvr_cccb_write_command_with_header`, `pvr-src/pvr_cccb.c:147-195`):
each command is `[header][payload]`, header:

```c
struct rogue_fwif_ccb_cmd_header {  /* pvr_rogue_fwif.h:1564-1578 */
    u32 cmd_type;      // ROGUE_FWIF_CCB_CMD_TYPE_*
    u32 cmd_size;      // ALIGN(payload_size, 8)
    u32 ext_job_ref;   // debug (job->id)
    u32 int_job_ref;   // debug (job->id)
    struct rogue_fwif_workest_kick_data work_est_kick_data __aligned(8); // leave 0
};
```
Total on-ring size = `sizeof(header) + ALIGN(cmd_size,8)`
(`pvr_cccb_get_size_of_cmd_with_hdr`, `pvr-src/pvr_cccb.h:80-85`). If a command would wrap, a
`ROGUE_FWIF_CCB_CMD_TYPE_PADDING` (0x2ABC00DD) header is written to the tail and `write_offset`
resets to 0 (`pvr-src/pvr_cccb.c:166-190`).

**The exact command sequence** the kernel writes for one job (`pvr_queue_submit_job_to_cccb`,
`pvr-src/pvr_queue.c:661-738`) — replicate this:
1. (optional) `FENCE_PR` (0x2ABC00D7) commands, one per unsignaled native dep, each an array of
   `rogue_fwif_ufo{addr,value}`. **For a standalone fill with no deps: skip.**
2. **The job command**: `cmd_type = TQ_3D (0x2ABC80CA)`, payload = the built
   `struct rogue_fwif_cmd_transfer` (see §3), `cmd_size = ALIGN(sizeof,8)`.
3. **`UPDATE` (0x2ABC00D5)**: payload = one `rogue_fwif_ufo{ addr = timeline_ufo FW addr,
   value = done_fence seqno }`. This is what signals completion (§4).

### 2.3 Kicking the job (CCCB → KCCB KICK)
After bumping `cccb->write_offset`, send **one** KCCB command (`pvr_cccb_send_kccb_kick`,
`pvr-src/pvr_cccb.c:224-239`):

```c
struct rogue_fwif_kccb_cmd {                       /* pvr_rogue_fwif.h:1245-1259 */
    u32 cmd_type;  // = ROGUE_FWIF_KCCB_CMD_KICK (101|0x2ABC0000 = 0x2ABC0065)
    u32 padding;
    union { struct rogue_fwif_kccb_cmd_kick_data cmd_kick_data; ... } cmd_data;
};

struct rogue_fwif_kccb_cmd_kick_data {             /* pvr_rogue_fwif.h:871-888 */
    u32 context_fw_addr;         // = pvr_context_get_fw_addr(ctx) + queue->ctx_offset  (ctx_offset=0)
    u32 client_woff_update;      // = cccb->write_offset  (new)
    u32 client_wrap_mask_update; // = cccb->wrap_mask     (0x7FFF)
    u32 num_cleanup_ctl;         // 0 for transfer (no hwrt) — fill_cmd_kick_data only adds one if hwrt!=NULL
    u32 cleanup_ctl_fw_addr[ROGUE_FWIF_PRBUFFER_MAXSUPPORTED+1]; // unused → 0
    u32 work_est_cmd_header_offset; // 0
};
```
`fill_cmd_kick_data()` (`pvr-src/pvr_cccb.c:197-211`): for transfer, `hwrt==NULL` so
`num_cleanup_ctl=0`. A `wmb()` precedes the kick (`:236`).

The KCCB send itself (`pvr_kccb_send_cmd_reserved_powered`, `pvr-src/pvr_ccb.c:261-303`):
memcpy the 88-ish-byte `rogue_fwif_kccb_cmd` into the KCCB ring at `write_offset`, `mb()`, bump the
KCCB `write_offset`, then **kick MTS**:
```c
pvr_fw_mts_schedule(pvr_dev, PVR_FWIF_DM_GP & ~ROGUE_CR_MTS_SCHEDULE_DM_CLRMSK);  // DM=GP(0)
    → pvr_cr_write32(pvr_dev, ROGUE_CR_MTS_SCHEDULE, val);   // pvr_fw.c:1076-1083
```
The GP data master in FW processes the KCCB, sees KICK, reads `client_woff_update` into the CCCB
control, and schedules the TQ_3D command on the FRAG DM.

---

## 3. The actual GPU command/control stream for the fill

### 3.1 What the kernel forwards vs. what it builds
The kernel does **not** understand register semantics. `pvr_fw_cmd_init()`
(`pvr-src/pvr_job.c:87-102`) copies a user "stream" (byte blob) and runs it through
`pvr_stream_process()` using the per-DM **stream definition**, scattering values into the FW struct.
For transfer the definition is `rogue_fwif_cmd_transfer_stream` (`pvr-src/pvr_stream_defs.c:234-270`).
**You build the stream in exactly this order** (each entry = little-endian value of the given width;
the process step also validates/ör-pads). The resulting `struct rogue_fwif_cmd_transfer` is
(`pvr-src/pvr_rogue_fwif_client.h:317-369`):

```c
struct rogue_fwif_transfer_regs {   // (only the fields present in the stream def matter)
    u32 isp_bgobjvals;
    u32 usc_pixel_output_ctrl;
    u32 usc_clear_register0, usc_clear_register1, usc_clear_register2, usc_clear_register3;
    u32 isp_mtile_size;
    u32 isp_render_origin;
    u32 isp_ctl;
    u32 isp_xtp_pipe_enable;   // only if S7_TOP_INFRASTRUCTURE
    u32 isp_aa;
    u32 event_pixel_pds_info;
    u32 event_pixel_pds_code;
    u32 event_pixel_pds_data;
    u32 isp_render;
    u32 isp_rgn;
    u32 frag_screen;           // only if GPU_MULTICORE_SUPPORT
    aligned_u64 pds_bgnd0_base, pds_bgnd1_base, pds_bgnd3_sizeinfo;
    u64 isp_mtile_base;
    u64 pbe_wordx_mrty[3*3];   // TQ_MAX_RENDER_TARGETS(3) * PBE words-per-RT(3)
};
struct rogue_fwif_cmd_transfer {
    struct rogue_fwif_cmd_common common;  // {u32 frame_num} — kernel sets 0 (pvr_job.c:271)
    struct rogue_fwif_transfer_regs regs __aligned(8);
    u32 flags;    // kernel sets from convert_transfer_flags() (pvr_job.c:272) — 0 (or SINGLE_CORE)
    u32 padding;
};
```
Stream **order** (from `pvr_stream_defs.c:234-259`), 64-bit first then 32-bit:
`pds_bgnd0_base(64), pds_bgnd1_base(64), pds_bgnd3_sizeinfo(64), isp_mtile_base(64),
pbe_wordx_mrty[9](64 each), isp_bgobjvals(32), usc_pixel_output_ctrl(32),
usc_clear_register0..3(32×4), isp_mtile_size(32), isp_render_origin(32), isp_ctl(32), isp_aa(32),
event_pixel_pds_info(32), event_pixel_pds_code(32), event_pixel_pds_data(32), isp_render(32),
isp_rgn(32)`, then `isp_xtp_pipe_enable(32)` **iff** `S7_TOP_INFRASTRUCTURE`, `frag_screen(32)`
**iff** `GPU_MULTICORE_SUPPORT`. (`common.frame_num` and `flags` are set by the kernel, not in the
stream.) `dest_size = sizeof(struct rogue_fwif_cmd_transfer)`.

### 3.2 Concrete field values for a WxH RGBA8888 linear fill (from Mesa's FILL path)
Reference: `pvr_3d_copy_blit_core()` FILL branch (`mesa-src/pvr_arch_job_transfer.c:2828-3051`),
`pvr_setup_hwbg_object()` (`:651-694`), `pvr_pbe_setup*` (`:1152-1333`), `pvr_isp_tiles()`
(`:1370-1466`), `pvr_isp_ctrl_stream()` (`:4045-4582`). All register packing uses `pvr_csb_pack`
against the `rogue_pvr_cr_defs` layouts (see `pvr-src/pvr_rogue_cr_defs.h` for bit fields).

- `common.frame_num = 0`, `flags = 0`, `padding = 0`.
- **`isp_bgobjvals = 0`** — "no background enabled" for a pure fill (`:2844`). (Contrast: a *copy*
  sets `enablebgtag=true`, `:2999-3001`.)
- **`usc_clear_register0..3`** = the packed clear color. For UNORM RGBA8888, all four channels pack
  into **register0 = 0xAABBGGRR** (byte order per `pvr_pack_clear_color`); registers 1..3 = 0 for a
  single 32-bit-per-pixel RT (`:2850-2870`). This is the ONLY place the fill color lives.
- **`pds_bgnd0_base = pds_bgnd1_base = pds_bgnd3_sizeinfo = 0`** — background-object PDS is null for a
  fill (`state->pds_shader_task_offset=0`, `common_ptr=0`, sizes 0 → `:651-694`). *No background USC.*
- **`isp_aa`**: `mode = aa_mode(sample_count=1) = 0` (`:2846-2848`).
- **`usc_pixel_output_ctrl`** and the tiles-in-flight bits of **`isp_ctl`**: from
  `pvr_arch_setup_tiles_in_flight()` (`:1441-1463`); `isp_ctl.process_empty_tiles = true`.
- **`isp_mtile_size`**: `x = width_in_tiles`, `y = height_in_tiles` (tile size is core-specific,
  typically 32×32; 64×64 → 2×2 tiles) (`:1431-1434`).
- **`isp_render_origin`** = {0,0} (`:1436-1439`).
- **`isp_render`**: final value from the ctrl-stream builder is
  `mode_type = ROGUE_CR_ISP_RENDER_MODE_TYPE_FAST_2D` (`:4563-4565`). (The earlier FAST_SCALE at
  `:3027` is overwritten.)
- **`isp_rgn`**: region-stream size / link flag (`:4569-4577`); for a single non-linked prim block
  the size field encodes the control-stream length. Use `CR_ISP_RGN`/`CR_ISP_RGN_SIPF` packing.
- **`isp_mtile_base`** = **(ISP control-stream BO dev-vaddr) − TRANSFER_FRAG_HEAP_BASE**
  (`:4557-4561`). This is an **offset**, because `CR_ISP_MTILE_BASE`/the ISP reads relative to the
  transfer-frag heap. **The ISP control stream itself lives in the TRANSFER_FRAG heap
  (0xE4_0000_0000).**
- **`pbe_wordx_mrty[0..2]`** (3 u64 for RT0) = the packed **PBE state** for the destination surface,
  built by `pvr_arch_pbe_pack_state()` (`:1303-1307`). Encodes: dest **base address**, **row stride**
  (= width×4 for RGBA8888 linear), **pixel format** (8888), **memory layout** (LINEAR),
  **size/extent**, swizzle (identity). RT1/RT2 words = 0.
- **`event_pixel_pds_code/data/info`**: point at the **PDS pixel-event program** that kicks the EOT
  USC (`pvr_pbe_setup_emit`, `:1223-1239`). `event_pixel_pds_code.addr` = PDS code offset (within
  PDS heap), `event_pixel_pds_data.addr` = PDS data offset, `event_pixel_pds_info` =
  `{temp_stride=0, const_size=ceil(data_size/unit), usc_sr_size=ceil(rt_count*PVR_STATE_PBE_DWORDS/unit)}`.
- **`frag_screen`** (multicore) = `{xmax=W-1, ymax=H-1}` (`:3046-3051`).

### 3.3 The ISP control stream (IPF control stream) content
Built by `pvr_isp_ctrl_stream()` (`mesa-src/pvr_arch_job_transfer.c:4045-4582`), allocated in the
**TRANSFER_FRAG heap**. For a FILL with no source: `num_prim_blks = 1`, one **primitive block** that
covers the whole surface, then a **TERM** control word (`ROGUE_IPF_CS_TYPE_TERM`, `:4550-4554`).
The primitive block references the **NOP/empty PDS pixel program** for the fill (no texture, no
shader; `device->nop_program.pds.data_offset` is used for the empty case, `:4209`), while the pixel
data comes from the clear registers. Structure: `[region array header(s)] [primitive block(s)] [TERM]`.
Word packing via `IPF_CONTROL_STREAM` / `pvr_isp_ctrl_stream_sipf_write_aligned` (`:4018-4044`).
Size accounting: `ROGUE_IPF_CONTROL_STREAM_SIZE_DWORDS` per region array + `pvr_isp_primitive_block_size`
(`:4050-4132`). This is the one non-trivial blob you must generate by hand; it is small (a few dozen
dwords for one full-screen block) and **static for a fixed WxH**.

### 3.4 The USC EOT (emit) program — the unavoidable tiny shader
Generated by `pvr_usc_eot()` (`mesa-src/pvr_usc.c:87-175`). For 1 RT with immediate PBE words it is
essentially:
```
emitpix(state0 = pbe_word0, state1 = pbe_word1)   // nir_emitpix_pco
end / phase-end
```
(For N RTs: `emitpix; wop; emitpix; …`, `:100-169`.) It is **compiled/encoded by Mesa's PCO backend**
(`build_shader`/`pco_encode_ir`, `:41-52`) — there is **no static byte blob shipped in current
Mesa** (older DDKs shipped a fixed "TQ EOT" USC). You must either (a) hand-encode the `emitpix`+`end`
for the BXS-4-64 ISA, or (b) capture the ~2–4-instruction binary once from a Mesa build for this BVNC.
It is **color- and size-independent** (the PBE state words are passed as immediates in the program or
via shared regs), so you build it exactly once and reuse it. Upload to the **USCCODE heap**; its DOUTU
target address is `(usc_bo_vaddr − USCCODE_HEAP_BASE)`
(`pvr_pds_setup_doutu`, `mesa-src/pvr_arch_job_transfer.c:1176-1183`).

### 3.5 The PDS pixel-event program that kicks the EOT USC
`pvr_pds_generate_pixel_event_{data,code}_segment` + `pvr_pds_setup_doutu`
(`mesa-src/pvr_arch_job_transfer.c:1179-1218`). Tiny PDS program: a DOUTU that launches the EOT USC
and a DOUTA/emit of the PBE state words. Upload to the **PDSCODEDATA heap**; the code/data offsets
(relative to PDS heap base) go into `event_pixel_pds_code/data` (§3.2). The PDS `EXEC_BASE` register
is programmed globally to the PDS heap base, so these are offsets, not full vaddrs.

---

## 4. Fences / sync + completion, and readback

### 4.1 Completion signal (the important part for bare-metal)
Each queue owns a **timeline UFO**: a single `u32` in a device-**uncached** FW object
(`pvr-src/pvr_queue.c:1358-1366`, `queue->timeline_ufo.value`). The last CCCB command of every job is
an `UPDATE` writing `{addr=timeline_ufo_fw_addr, value=job->done_fence->seqno}`
(`pvr-src/pvr_queue.c:733-737`). When the FRAG DM finishes the TQ_3D, the FW executes the UPDATE and
writes `seqno` into that u32.

**How you verify done:** poll `*queue->timeline_ufo.value` (uncached CPU mapping) until it is
`>= your seqno` — exactly what `pvr_queue_signal_done_fences()` does
(`cur_seqno = *queue->timeline_ufo.value; … (int)(cur_seqno - lower_32_bits(job->done_fence->seqno)) >= 0`,
`pvr-src/pvr_queue.c:971-990`). For a single job, set `value=1`, poll for `==1`. No `dma_fence`
infrastructure is required for a bare-metal poll.

### 4.2 IRQ
Normally the FW raises an IRQ and the host runs `pvr_queue_process()` on the event. For bare-metal you
can **ignore the IRQ and just poll the timeline UFO** (and optionally the KCCB return slot
`kccb.rtn[slot] != ROGUE_FWIF_KCCB_RTN_SLOT_NO_RESPONSE`, `pvr-src/pvr_ccb.c:290-291,413-424`). The
FW also advances the CCCB `read_offset` and the KCCB `read_offset` as it consumes commands; those are
secondary progress indicators.

### 4.3 Readback
The destination BO is an ordinary GPU-MMU mapping in the **GENERAL heap** (§5). Map it CPU-side
(or allocate it CPU-mapped), invalidate CPU cache for the range if cached, and read the WxH×4 bytes.
Expect every pixel == your packed clear color (0xAABBGGRR order per `pvr_pack_clear_color`). If the
SLC/GPU cache might hold dirty lines, an SLC flush/inval via KCCB `SLCFLUSHINVAL`
(`pvr-src/pvr_rogue_fwif.h:1160`) or the PBE's own flush guarantees coherence; the UPDATE completing
implies the PBE emits have drained to memory.

---

## 5. Memory / heaps and required placements

Heap bases (`pvr-src/pvr_rogue_heap_config.h`). `PDS_EXEC_BASE`←PDSCODEDATA base and
`USC_CODE_BASE`←USCCODE base are programmed **globally** (header note lines 17-19), so PDS/USC
addresses in commands are **offsets from those bases**; ISP/PBE/dest use full 40-bit dev-vaddrs
(except `isp_mtile_base`, which is offset-from-TRANSFER_FRAG-base, §3.2).

| What | Heap | Base | Address form in registers |
|---|---|---|---|
| Destination surface (WxH×4) | **GENERAL_HEAP** | `0x80_0000_0000` (`:42`) | full vaddr, inside PBE words |
| ISP control stream (IPF) | **TRANSFER_FRAG_HEAP** | `0xE4_0000_0000` (`:94`, 16 GiB) | `isp_mtile_base = vaddr − 0xE400000000` |
| PDS pixel-event program | **PDSCODEDATA_HEAP** | `0xDA_0000_0000` (`:61`, 4 GiB) | `event_pixel_pds_code/data = offset` |
| USC EOT emit program | **USCCODE_HEAP** | `0xE0_0000_0000` (`:79`, 4 GiB) | DOUTU addr = vaddr − 0xE000000000 |
| CCCB + ctl, timeline UFO, FW ctx, reg-state | **FIRMWARE_HEAP** | `0xE1_C000_0000` (`:87`, existing) | FW addrs via `pvr_fw_object_get_fw_addr` |

**Not needed** for a TRANSFER fill (these are the FRAGMENT/3D burden you skip): free lists
(`pvr-src/pvr_free_list.c`), HWRT dataset / region-header heap `RGNHDR` (`0xDB_F000_0000`),
parameter buffers, VDM geometry stream, `VISTEST` heap. COMPUTE would instead need a CDM ctrl stream
in GENERAL and a compute USC kernel in USCCODE.

Alignment notes: CCCB `write_offset` is 16-byte aligned (`rogue_fwif_cccb_ctl.write_offset` comment);
`cmd_size` is 8-byte aligned (`pvr-src/pvr_cccb.c:154`); PDS code/data honor
`ROGUE_CR_EVENT_PIXEL_PDS_{CODE,DATA}_ADDR_ALIGNMENT`; heap bases are 4 MiB-aligned.

---

## 6. Honest verdict

- **A no-USC fill is not possible.** The PowerVR PBE cannot write memory on its own; pixels reach
  memory only through a USC **`emitpix`** issued by a PDS **pixel-event** program at end-of-tile.
  Evidence: even the FILL branch, which explicitly builds **no pixel shader**
  (`state->pds_temps=0; /* No shader */`, `mesa-src/pvr_arch_job_transfer.c:2837-2841`) and a **null
  background PDS** (`pds_bgnd*=0`, `:651-694`), still calls `pvr_pbe_setup()` →
  `pvr_pbe_setup_emit()` which wires `event_pixel_pds_*` to a PDS program that DOUTUs
  `ctx->usc_eot_bos[...]` (`:1176-1239`).
- **But the USC you need is trivial and fixed:** one `emitpix(pbe_word0, pbe_word1); end` per render
  target (`mesa-src/pvr_usc.c:87-169`). It carries no color and no geometry — the color is pure
  register state (`CR_USC_CLEAR_REGISTER0..3`) and the extent is ISP/PBE state. Build/encode it once
  for BVNC 36.53.104.796 and reuse it for every fill of every size/color.
- **Recommended minimal milestone:** TRANSFER_FRAG / TQ_3D **FILL** of a small (e.g. 64×64) linear
  RGBA8888 buffer in the GENERAL heap, completion by polling the timeline UFO, verified by CPU
  readback. This exercises the whole submit path (context, CCCB, KCCB KICK, MTS, ISP+PBE, UFO
  signal) with the least hand-built content: **one fixed USC EOT program, one small PDS pixel-event
  program, one static ISP control stream, and a handful of register values.**

---

## 7. Ordered build list

1. **VM + FW mem-context**: ensure the app VM context exists and its FW mem-context FW addr is known
   (`fw_mem_context_fw_addr`). Reuse the MMU/page-catalog already running.
2. **Allocate + map the destination surface** in GENERAL heap (0x80_00000000+), CPU-mappable,
   W×H×4 bytes, linear. Record its GPU dev-vaddr.
3. **Build + upload the USC EOT emit program** to USCCODE heap (0xE0_00000000+):
   `emitpix(pbe_word0,pbe_word1); end` (hand-encode or capture from Mesa for this BVNC). Record
   offset-from-USC-base and its temp count.
4. **Build + upload the PDS pixel-event program** to PDSCODEDATA heap (0xDA_00000000+)
   (DOUTU→EOT USC + emit of PBE words). Record code_offset, data_offset, data_size.
5. **Pack the PBE state** for the dest surface → `pbe_wordx_mrty[0..2]` (base=dest vaddr,
   stride=W×4, fmt=8888, layout=LINEAR, extent=W×H) and the matching `pbe_setup_words` for the EOT.
6. **Build + upload the ISP control stream** (one full-screen primitive block + TERM) to
   TRANSFER_FRAG heap (0xE4_00000000+). Record its vaddr → compute `isp_mtile_base` offset.
7. **Create the transfer context**: allocate `rogue_fwif_fwtransfercontext` (uncached) + a
   ~16-byte frag ctx-switch state (`reg_state_obj`, zeroed) + a timeline UFO (u32, uncached).
   Fill `rogue_fwif_fwcommoncontext` per §2.1 (`dm=FRAG(3)`, ccb addrs, mem-context, etc.).
8. **Create the CCCB** (32 KiB) + its `rogue_fwif_cccb_ctl` (uncached), init offsets/wrap_mask.
9. **Assemble `struct rogue_fwif_cmd_transfer`** register values per §3.2 (clear color in
   `usc_clear_register0`, `isp_bgobjvals=0`, tiles/render/rgn/mtile_base, event_pixel_pds_*,
   pbe words, `frame_num=0`, `flags=0`).
10. **Write the CCCB command sequence**: `[TQ_3D header + cmd_transfer]` then
    `[UPDATE header + ufo{timeline_addr, seqno=1}]`; bump `cccb->write_offset` (16B aligned); `wmb()`.
11. **Send the KCCB KICK**: fill `rogue_fwif_kccb_cmd_kick_data` (`context_fw_addr=ctx_fw_addr+0`,
    `client_woff_update`, `client_wrap_mask_update=0x7FFF`, `num_cleanup_ctl=0`), memcpy to KCCB,
    bump KCCB write_offset, `mb()`, write `ROGUE_CR_MTS_SCHEDULE` with DM=GP(0).
12. **Poll** `*timeline_ufo.value == 1` (uncached). Then **read back** the dest buffer and check every
    pixel == packed color. (Optionally SLCFLUSHINVAL via KCCB before readback.)

If step 3/6 (USC/ISP encoding) proves too fiddly to hand-encode, the COMPUTE/CDM path (write one
tiny USC store-kernel) is the fallback second-minimal; it avoids the ISP control stream but requires
a real compute kernel + CDM control stream + static compute ctx state.
