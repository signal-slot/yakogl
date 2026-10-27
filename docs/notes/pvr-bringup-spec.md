<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR BXS-4-64 MC1 (Rogue "Volcanic", BVNC 36.53.104.796) Zephyr Bring-up Spec

Status: finished record, moved here from `driver/docs/gpu-recipes/gpu-bringup-spec.md` on 2026-10-11. A pre-bring-up spec written from the mainline drm/imagination driver. The driver boots the DDK firmware rgx.fw.36.53.104.796 instead (driver/README.md); the boot code is driver/src/pvr_bringup.c and driver/src/pvr_fw_boot.c.

Source: mainline `drivers/gpu/drm/imagination` (torvalds/linux master), extracted via WebFetch.
Target: TI AM62P SoC, bare-metal Zephyr (no DRM/GEM). Firmware: RISC-V 32-bit ELF
`powervr/rogue_36.53.104.796_v1.fw`.

All register names are `ROGUE_CR_*` from `pvr_rogue_cr_defs.h`. Offsets marked **(confirmed)**
were extracted verbatim from the header; offsets marked **(verify)** could not be pulled from
the (very large, truncated-in-fetch) header and MUST be re-read from your local generated
`pvr_rogue_cr_defs.h` before you hardcode them — but the macro *names* are authoritative.

---

## 0. Architecture summary / file map

`Makefile` (`powervr-y`) confirms three FW-processor back-ends are compiled:
`pvr_fw_meta.o` (META), `pvr_fw_mips.o` (MIPS), **`pvr_fw_riscv.o` (RISC-V)**, plus
`pvr_fw_startstop.o`, `pvr_fw.o`, `pvr_fw_util.o`, `pvr_ccb.o`, `pvr_cccb.o`, `pvr_mmu.o`,
`pvr_vm.o`, `pvr_vm_mips.o`, `pvr_power.o`, `pvr_device.o`, `pvr_device_info.o`.

The back-end is selected at runtime by a `struct pvr_fw_defs` ops table. For BXS-4-64
(Volcanic, RISC-V FW core) the table is **`pvr_fw_defs_riscv`** in `pvr_fw_riscv.c`.

---

## 1. Firmware processor type — RISC-V

**Confirmed: BXS-4-64 uses a RISC-V firmware micro-controller.** It is driven by
`pvr_fw_riscv.c`, whose ops table is:

```
static const struct pvr_fw_defs pvr_fw_defs_riscv = {
    .init                    = pvr_riscv_init,
    .fw_process              = pvr_riscv_fw_process,
    .vm_map                  = pvr_riscv_vm_map,
    .vm_unmap                = pvr_riscv_vm_unmap,
    .get_fw_addr_with_offset = pvr_riscv_get_fw_addr_with_offset,
    .wrapper_init            = pvr_riscv_wrapper_init,
    .irq_pending             = pvr_riscv_irq_pending,
    .irq_clear               = pvr_riscv_irq_clear,
};
```

Note there is **no `.start`/`.stop`** in the ops table — start/stop is common code in
`pvr_fw_startstop.c` (`pvr_fw_start`/`pvr_fw_stop`), which branches on the RISC-V feature only
for a couple of writes (`ROGUE_CR_FWCORE_BOOT`, the FWCORE MMU base).

### FW ELF / segment layout
- `pvr_riscv_fw_process()` (pvr_fw_riscv.c) calls the common
  `pvr_fw_process_elf_command_stream()` (pvr_fw.c) to parse the FW image and copy segments.
- Segments come from the FW **info header** and **layout table**, not raw ELF program headers.
  `struct pvr_fw_layout_entry { enum pvr_fw_section_id id; enum pvr_fw_section_type type;
   u32 base_addr; u32 max_size; u32 alloc_size; u32 alloc_offset; }` (pvr_fw_info.h).
  Section types: **`FW_CODE`, `FW_DATA`, `FW_COREMEM_CODE`, `FW_COREMEM_DATA`**.
  `base_addr` = the FW-processor device-virtual address the segment must live at;
  `alloc_offset` = offset of the bytes inside the FW image file.
- `struct pvr_fw_info_header` (pvr_fw_info.h): `info_version`, `layout_entry_num`,
  `layout_entry_size`, `bvnc`, `fw_page_size`, `fw_version_major/minor/build`,
  `device_info_size`. `pvr_fw_validate()` (pvr_fw.c) checks the header's `bvnc` against the
  live GPU id — this must match 36.53.104.796.
- Segments are copied into GPU-visible buffers held as four objects in `fw_mem`:
  `fw_code_ptr`, `fw_data_ptr`, `fw_core_code_ptr`, `fw_core_data_ptr` (core code/data are the
  RISC-V "coremem" tightly-coupled regions; optional if present in the layout table).
- The FW **boot data** for RISC-V is a `struct rogue_riscv_fw_boot_data` overlaid at the start
  of the FW data segment (`fw_data_ptr`), carrying the device-virtual/firmware addresses of the
  code/coremem regions the bootloader needs.
- Entry point / boot address is **not** a PC written to a register; the RISC-V core resets to
  its bootloader whose code/data are exposed through the FWCORE address-remap windows (see §5).
  The kick is a single `ROGUE_CR_FWCORE_BOOT = 1` write.
- FW heap for RISC-V: `pvr_fw_heap_info_init(pvr_dev, ROGUE_FW_HEAP_RISCV_SHIFT, 0)` → 32 MiB
  heap.

---

## 2. Power / clock / reset

### What the platform (Zephyr) must do before the driver touches registers
On Linux the GPU node uses `pm_runtime` + genpd; the actual TISCI power-domain/clock work is
done by TI's k3 firmware driver, invoked through DT. For Zephyr you replicate that manually
**before** any register access:
- Enable **TISCI power-domains 237 and 242** (device power) via the AM62P DMSC/TIFS mailbox.
- Enable **clock id 237 output 3 @ 800 MHz** (GPU core clock).
- Deassert the GPU reset.

This mirrors the driver's manual power op `pvr_power_sequence_ops_manual` (pvr_drv.c/pvr_power.c),
which for `compatible = "img,img-rogue"` / `"ti,am62-gpu"` does:
```
clk_prepare_enable(core_clk);   // "core" — mandatory
clk_prepare_enable(sys_clk);    // optional (AM62 has none)
clk_prepare_enable(mem_clk);    // optional (AM62 has none)
udelay(1);
reset_control_deassert(reset);
```
`pvr_device_clk_init()` (pvr_device.c) gets `core` (required), `sys`, `mem` (optional).
`pvr_device_init()` calls `pm_runtime_resume_and_get(dev)` **before** the first register read.

DT (`img,powervr-rogue.yaml`): AM62/AM62P uses `compatible = "ti,am62-gpu", "img,img-rogue"`
(BXS variant adds `"img,img-bxs-4-64"`), `clock-names = "core"` (single clock),
`power-domains`/`power-domain-names` **required** for `img,img-bxs-4-64`. Note the current
`dt_match` in `pvr_drv.c` only lists `img,img-rogue`, `img,img-axe`, `thead,th1520-gpu` — the
`ti,am62-gpu` compatible falls through to `img,img-rogue` → `pvr_device_data_manual`.

### GPU-internal reset done by the driver (`pvr_fw_start`, pvr_fw_startstop.c)
Order (all writes to control-register BAR):
1. If `sys_bus_secure_reset` feature: `ROGUE_CR_SYS_BUS_SECURE = 0`.
2. If RISC-V: `ROGUE_CR_FWCORE_BOOT = 0`  (hold FW core in reset).
3. **Assert full reset:** `ROGUE_CR_SOFT_RESET = soft_reset_mask` (the full mask),
   and if `xe_tpu2` feature: `ROGUE_CR_SOFT_RESET2 = ROGUE_CR_SOFT_RESET2_MASKFULL`.
4. **Read back** `ROGUE_CR_SOFT_RESET` (+SOFT_RESET2) to fence the writes.
5. **Staged deassert (critical order):**
   a. Take Rascal+Dust out first: `SOFT_RESET = mask ^ ROGUE_CR_SOFT_RESET_RASCALDUSTS_EN`.
   b. Everything except the FW processor: `SOFT_RESET = ROGUE_CR_SOFT_RESET_GARTEN_EN`
      (i.e. leave only Garten/FW-core asserted).
   c. Release all: `SOFT_RESET = 0x0`.
6. `rogue_slc_init()` — program `ROGUE_CR_SLC_CTRL_MISC` (hazard checks, address-decode mode,
   burst-combiner bypass when cache line < 1024 bits).
7. `defs->wrapper_init()` → `pvr_riscv_wrapper_init()` (see §5).
8. `rogue_axi_ace_list_init()` — `ROGUE_CR_AXI_ACE_LITE_CONFIGURATION` (coherency/ACE-lite).
9. `rogue_bif_init()` — write the kernel page-table root: `ROGUE_CR_BIF_CAT_BASE0`, and for
   RISC-V also `ROGUE_CR_FWCORE_MEM_CAT_BASE0`.
10. `udelay(3)`.
11. If RISC-V: **`ROGUE_CR_FWCORE_BOOT = 1`**  ← this releases the RISC-V core to run.

### Register offsets involved in reset (from header)
- `ROGUE_CR_SOFT_RESET`  = **0x0100** (confirmed).
  `ROGUE_CR_SOFT_RESET_MASKFULL` = `0x00E7FFFFFFFFFC3D`,
  `ROGUE_CR_SOFT_RESET_GARTEN_EN` = `0x0000000100000000`,
  `ROGUE_CR_SOFT_RESET_RASCALDUSTS_EN` (verify value in header).
- `ROGUE_CR_SOFT_RESET2` = **0x0108** (confirmed), MASKFULL `0x00000000001FFFFF`.
- `ROGUE_CR_SYS_BUS_SECURE`, `ROGUE_CR_SLC_CTRL_MISC`, `ROGUE_CR_AXI_ACE_LITE_CONFIGURATION`,
  `ROGUE_CR_BIF_CAT_BASE0`, `ROGUE_CR_FWCORE_MEM_CAT_BASE0`, `ROGUE_CR_FWCORE_BOOT` — names
  authoritative, offsets **(verify)** in local header.

### Higher-level power/reset flow (pvr_power.c)
- `pvr_power_device_resume()` → `pwr_ops->power_on()` then `pvr_power_fw_enable()`.
- `pvr_power_fw_enable()` → `pvr_fw_start()` then `pvr_wait_for_fw_boot()`; arms watchdog
  (`WATCHDOG_TIME_MS` = 500 ms).
- `pvr_power_device_suspend()` → `pvr_power_fw_disable(false,true)` then `pwr_ops->power_off()`.
- `pvr_power_fw_disable()` sends `ROGUE_FWIF_POW_FORCED_IDLE_REQ` then `ROGUE_FWIF_POW_OFF_REQ`
  via KCCB, disables IRQ, then `pvr_fw_stop()`.
- `pvr_power_reset(hard_reset=true)`: pre-reset queues, `pvr_power_fw_disable(true,false)`,
  `pvr_fw_hard_reset()`, full suspend/resume cycle, post-reset queues; on failure escalate,
  then `pvr_device_lost()`.

---

## 3. Register map / core identification (MILESTONE 1)

Decoder: **`pvr_gpuid_decode_reg()`** (pvr_device.c). Storage: `struct pvr_gpu_id { u16 b,v,n,c; }`
in `pvr_device.gpu_id` (pvr_device.h).

### The read
Primary (PBVNC-capable cores — this includes BXS):
```
u64 bvnc = pvr_cr_read64(pvr_dev, ROGUE_CR_CORE_ID__PBVNC);
b = (bvnc & ~..BRANCH_ID_CLRMSK)              >> ..BRANCH_ID_SHIFT;              // bits 63:48
v = (bvnc & ~..VERSION_ID_CLRMSK)             >> ..VERSION_ID_SHIFT;             // bits 47:32
n = (bvnc & ~..NUMBER_OF_SCALABLE_UNITS_CLRMSK)>> ..NUMBER_OF_SCALABLE_UNITS_SHIFT; // 31:16
c = (bvnc & ~..CONFIG_ID_CLRMSK)              >> ..CONFIG_ID_SHIFT;              // bits 15:0
```
If `b == 0` (old core, no PBVNC), fall back to:
```
u32 core_rev = pvr_cr_read32(pvr_dev, ROGUE_CR_CORE_REVISION);
u32 core_id  = pvr_cr_read32(pvr_dev, ROGUE_CR_CORE_ID);
b = PVR_CR_FIELD_GET(core_rev, CORE_REVISION_MAJOR);
v = PVR_CR_FIELD_GET(core_rev, CORE_REVISION_MINOR);
n = (core_id & 0xFF00) >> 8;
c = (core_id & 0x00FF);
```

### Exact registers/offsets (confirmed from header)
- **`ROGUE_CR_CORE_ID__PBVNC` = 0x0020**, 64-bit read. Field shifts:
  BRANCH_ID `48`, VERSION_ID `32`, NUMBER_OF_SCALABLE_UNITS `16`, CONFIG_ID `0`.
  CLRMSKs: BRANCH `0x0000FFFFFFFFFFFF`, VERSION `0xFFFF0000FFFFFFFF`,
  N-SCALABLE `0xFFFFFFFF0000FFFF`, CONFIG `0xFFFFFFFFFFFF0000`.
- `ROGUE_CR_CORE_ID` = **0x0018** (32-bit legacy: bits 31:16 = ID, 15:0 = CONFIG).
- `ROGUE_CR_CORE_REVISION` = **0x0020** (MAJOR shift 16, MINOR shift 8, MAINTENANCE shift 0).

> IMPORTANT ALIASING: In the IMG header, `ROGUE_CR_CORE_ID__PBVNC` and `ROGUE_CR_CORE_REVISION`
> are **both 0x0020**. This is intentional: on a PBVNC-capable core (BXS) offset 0x0020 is the
> 64-bit packed BVNC; on legacy cores it is the 32-bit CORE_REVISION. Since BXS-4-64 has B≠0,
> read **64-bit at 0x0020** and use the PBVNC decode.

### Expected BVNC read-back for 36.53.104.796 (liveness proof)
Packed 64-bit value at 0x0020 must be:

    B=36=0x0024  V=53=0x0035  N=104=0x0068  C=796=0x031C
    → pvr_cr_read64(0x0020) == 0x0024_0035_0068_031C

Decode: b=0x24=36, v=0x35=53, n=0x68=104, c=0x31C=796.

Minimal milestone-1 program: power domains 237/242 on, clock 237/3 @800MHz on, reset deasserted,
then a single 64-bit read of BAR+0x0020 and compare against `0x0024003500 68031C`.

---

## 4. GPU MMU (from pvr_mmu.c / pvr_fw_info.h)

**3-level page table** (naming per pvr_mmu.c):
- **L2 = Page Catalogue (root)** — `struct pvr_page_table_l2`, entry `pvr_page_table_l2_entry_raw`
  (32-bit): bit0 Valid, bit1 Pending, bits 31:4 = L1 base (phys bits 39:12).
- **L1 = Page Directory** — `struct pvr_page_table_l1`, entry `pvr_page_table_l1_entry_raw`
  (64-bit): bit0 Valid, bits 3:1 Page-size field, bits 39:5 = L0 base (alignment varies with
  page size), bit40 Pending.
- **L0 = Page Table** — `struct pvr_page_table_l0`, entry `pvr_page_table_l0_entry_raw`
  (64-bit): bit0 Valid, bit1 Read-Only, bit2 Cache-Coherency, bit3 SLC-Bypass, bit4 PM-Src,
  bit5 Pending, bits 11:6 VP-Page(low), bits 39:12 physical page (varies per page size),
  bits 61:40 VP-Page(high), **bit62 PM/FW-Protect**.

Supported page sizes: **4K / 16K / 64K / 256K / 1M / 2M** (`SZ_4K`…`SZ_2M`).
Root DMA address: `pvr_mmu_get_root_table_dma_addr()` → `ctx->page_table_l2.backing_page.dma_addr`.

Root is programmed into hardware in `rogue_bif_init()` (pvr_fw_startstop.c):
- Kernel/FW context catalogue base → **`ROGUE_CR_BIF_CAT_BASE0`**.
- RISC-V FW core catalogue base → **`ROGUE_CR_FWCORE_MEM_CAT_BASE0`**.

### What must exist before FW boot
Before `pvr_fw_start`, the driver has already (in `pvr_fw_init`/`pvr_vm_*`):
1. Created the **FW VM context** and its 3-level page tables (the FW heap is mapped here).
2. Allocated & mapped the FW **heap** (`PVR_ROGUE_FW_MAIN_HEAP_BASE`, size 1<<log2_size;
   RISC-V shift → 32 MiB). Top 192 KiB reserved as config heap
   (`PVR_ROGUE_FW_CONFIG_HEAP_SIZE = 3 * granularity`).
3. Loaded FW code/data/coremem objects (§1) into that heap and recorded their FW addresses.
4. Created the pre-boot control structures (§5) at fixed heap offsets.

---

## 5. Firmware load + boot handshake

### Load (pvr_fw.c)
- `pvr_fw_init()` → `pvr_fw_validate()` (header/BVNC check) → `pvr_fw_heap_info_init()` →
  `pvr_fw_process()` → `pvr_fw_process_elf_command_stream()` copies each layout segment to its
  `base_addr` inside the FW heap. Code at offset 0 of the code alloc; data at fixed or dynamic
  address per `has_fixed_data_addr`; core code/data if present.

### RISC-V wrapper init (`pvr_riscv_wrapper_init`, pvr_fw_riscv.c)
Three register writes set up the FWCORE address-remap windows so the RISC-V bootloader can
fetch code and load/store data out of the FW heap:
1. `ROGUE_RISCVFW_REGION_REMAP_CR(BOOTLDR_CODE)` ← bootloader-code dev-vaddr | common-opts |
   `ROGUE_CR_FWCORE_ADDR_REMAP_CONFIG0_FETCH_EN_EN`.
2. `ROGUE_RISCVFW_REGION_REMAP_CR(BOOTLDR_DATA)` ← bootloader-data dev-vaddr | common-opts |
   `ROGUE_CR_FWCORE_ADDR_REMAP_CONFIG0_LOAD_STORE_EN_EN`.
3. `ROGUE_CR_MTS_GARTEN_WRAPPER_CONFIG` ← `..._IDLE_CTRL_META`.
   common-opts = (heap size via `..._CONFIG0_SIZE_SHIFT`) | (MMU ctx via
   `..._CONFIG0_MMU_CONTEXT_SHIFT`).

### Pre-boot memory structures (`pvr_fw_create_structures`, pvr_fw.c)
Created at fixed heap offsets and pointed at each other:
- **Connection control** `fwif_connection_ctl` @ `PVR_ROGUE_FWIF_CONNECTION_CTL_OFFSET`.
- **OS init** `fwif_osinit` @ `PVR_ROGUE_FWIF_OSINIT_OFFSET` — holds KCCB/FWCCB addresses & ctls.
- **Sys init** `fwif_sysinit` @ `PVR_ROGUE_FWIF_SYSINIT_OFFSET` — fault page addr, core clock
  speed, exec base addresses, and the **`firmware_started`** flag polled at boot.
- Support objects: `power_sync`, `hwrinfobuf`, `fault_page_obj`, `gpu_util_fwcb_obj`,
  `runtime_cfg_obj`.
- Command buffers: `pvr_kccb_init()`, `pvr_fwccb_init()`.

### Kick + handshake
- FW core is released by `ROGUE_CR_FWCORE_BOOT = 1` at the end of `pvr_fw_start` (§2).
- Boot completion: **`pvr_wait_for_fw_boot()`** polls `fw_dev->fwif_sysinit->firmware_started`
  (timeout `FW_BOOT_TIMEOUT_USEC` = 5 s). On success `WRITE_ONCE(fw_dev->initialised, true)`.
- After boot, per-command signalling uses the KCCB doorbell (§below).

### KCCB / FWCCB (pvr_ccb.c)
- KCCB (kernel→FW): `pvr_kccb_init()` → `pvr_ccb_init(..., ROGUE_FWIF_KCCB_NUMCMDS_LOG2_DEFAULT,
  sizeof(rogue_fwif_kccb_cmd))`. FWCCB (FW→kernel): `pvr_fwccb_init()`.
- Send: `pvr_kccb_send_cmd()` → `pvr_kccb_send_cmd_powered()` →
  `pvr_kccb_send_cmd_reserved_powered()`:
  `memcpy(&kccb[old_write_offset], cmd, ...)`; set rtn slot to
  `ROGUE_FWIF_KCCB_RTN_SLOT_NO_RESPONSE`; `mb()`;
  `WRITE_ONCE(ctrl->write_offset, new_write_offset)`.
- **Doorbell/kick:** `pvr_fw_mts_schedule(pvr_dev, PVR_FWIF_DM_GP & ~ROGUE_CR_MTS_SCHEDULE_DM_CLRMSK)`
  writes **`ROGUE_CR_MTS_SCHEDULE` = 0x0B00** (`DM_CLRMSK = 0xFFFFFFF0`) to wake an MTS thread.
- Ring bookkeeping via `struct rogue_fwif_ccb_ctl { write_offset; read_offset; wrap_mask; }`.
  Capacity = slots − 1 (empty ⇔ read==write).
- FWCCB drain: `pvr_fwccb_process()` loops while `read_offset != write_offset`, calling
  `process_fwccb_command()`.

---

## 6. IRQ

- RISC-V IRQ pending (`pvr_riscv_irq_pending`, pvr_fw_riscv.c):
  `pvr_cr_read32(ROGUE_CR_IRQ_OS0_EVENT_STATUS) & ROGUE_CR_IRQ_OS0_EVENT_STATUS_SOURCE_EN`.
- RISC-V IRQ clear (`pvr_riscv_irq_clear`):
  `pvr_cr_write32(ROGUE_CR_IRQ_OS0_EVENT_CLEAR, ROGUE_CR_IRQ_OS0_EVENT_CLEAR_SOURCE_EN)`.
  (`ROGUE_CR_IRQ_OS0_EVENT_*` offsets: verify in header.)
- Top half `pvr_device_irq_handler()` (pvr_device.c): if `pvr_fw_irq_pending()` →
  `IRQ_WAKE_THREAD`. Thread `pvr_device_irq_thread_handler()`: loop while pending —
  `pvr_fw_irq_clear()`, then `pvr_fwccb_process()`, `pvr_kccb_wake_up_waiters()`,
  `pvr_device_process_active_queues()`.
- Registered `request_threaded_irq(..., IRQF_SHARED | IRQF_ONESHOT)` in `pvr_device_irq_init()`.
- Separate safety IRQ path reads `ROGUE_CR_EVENT_STATUS` / writes `ROGUE_CR_EVENT_CLEAR`.
- Meaning of the FW IRQ: FW has posted FWCCB command(s) and/or KCCB responses — the driver
  drains FWCCB and wakes KCCB waiters.

---

## 7. Minimal milestone ordering

### M1 — Power on + read BVNC (liveness)
- Do: enable TISCI PD 237 & 242; enable clk 237/3 @800MHz; deassert GPU reset (mirror
  `pvr_power_sequence_ops_manual`: core clk, udelay(1), reset deassert).
- Read: **`pvr_cr_read64(BAR + 0x0020)`** (`ROGUE_CR_CORE_ID__PBVNC`).
- Verify: value == **`0x0024003500 68031C`** → decode b=36,v=53,n=104,c=796.
- Ref: `pvr_gpuid_decode_reg()`, pvr_device.c.

### M2 — GPU internal reset brought out of reset
- Do the `pvr_fw_start` reset staging up to step 5 (§2): assert `SOFT_RESET`(0x0100)+`SOFT_RESET2`
  (0x0108) full, read back, then deassert RASCALDUSTS, then GARTEN-only, then 0.
- Verify: `ROGUE_CR_SIDEKICK_IDLE` (**0x03C8**) and `ROGUE_CR_SLC_IDLE` read as idle after reset;
  no bus faults.

### M3 — MMU + FW heap + firmware load
- Do: build 3-level page tables (§4); allocate/map 32 MiB FW heap at `PVR_ROGUE_FW_MAIN_HEAP_BASE`
  (top 192 KiB config heap); copy FW image segments to their `base_addr` (§1/§5);
  create connection-ctl / osinit / sysinit / power_sync / KCCB / FWCCB structures.
- Program roots in `rogue_bif_init`: `ROGUE_CR_BIF_CAT_BASE0`, `ROGUE_CR_FWCORE_MEM_CAT_BASE0`.
- Program SLC/AXI/FWCORE-remap: `rogue_slc_init` (`SLC_CTRL_MISC`), `rogue_axi_ace_list_init`
  (`AXI_ACE_LITE_CONFIGURATION`), `pvr_riscv_wrapper_init` (BOOTLDR_CODE/DATA remap +
  `MTS_GARTEN_WRAPPER_CONFIG`).
- Verify: `pvr_fw_validate` BVNC match; no MMU faults reading back mapped pages.

### M4 — FW boot handshake
- Do: `udelay(3)`, then **`ROGUE_CR_FWCORE_BOOT = 1`**.
- Verify: poll `sysinit->firmware_started` becomes true within 5 s (`pvr_wait_for_fw_boot`).
  Confirm FWCCB stays consistent; enable IRQ (`IRQ_OS0_EVENT_STATUS/CLEAR`).

### M5 — Submit a trivial job (KCCB round-trip)
- Do: write a `rogue_fwif_kccb_cmd` (e.g. a health-check / no-op / `POW` request) into the KCCB
  ring, `mb()`, bump `ctrl->write_offset`, ring doorbell
  `ROGUE_CR_MTS_SCHEDULE = PVR_FWIF_DM_GP` (BAR+0x0B00).
- Verify: FW IRQ fires; `pvr_fwccb_process`-style drain returns the response; for a POW command,
  `power_sync` transitions non-zero (`readl_poll_timeout`, 1 s). This proves the full
  kernel↔FW command path before attempting real render/compute DM jobs.

---

## Appendix — confirmed register offsets

| Register | Offset | Notes |
|---|---|---|
| ROGUE_CR_CORE_ID | 0x0018 | 32-bit legacy CORE_ID |
| ROGUE_CR_CORE_ID__PBVNC | 0x0020 | 64-bit packed BVNC (use on BXS) |
| ROGUE_CR_CORE_REVISION | 0x0020 | aliases 0x0020 on legacy cores |
| ROGUE_CR_SOFT_RESET | 0x0100 | MASKFULL 0x00E7FFFFFFFFFC3D; GARTEN_EN 0x100000000 |
| ROGUE_CR_SOFT_RESET2 | 0x0108 | MASKFULL 0x001FFFFF |
| ROGUE_CR_SIDEKICK_IDLE | 0x03C8 | idle poll |
| ROGUE_CR_MARS_IDLE | 0x08F8 | idle poll (layout_mars) |
| ROGUE_CR_MTS_SCHEDULE | 0x0B00 | doorbell; DM_CLRMSK 0xFFFFFFF0 |

Offsets to re-read from your local `pvr_rogue_cr_defs.h` (names authoritative, values not
extractable from the fetched/truncated header): `ROGUE_CR_SOFT_RESET_RASCALDUSTS_EN`,
`ROGUE_CR_SYS_BUS_SECURE`, `ROGUE_CR_SLC_CTRL_MISC`, `ROGUE_CR_SLC_IDLE`,
`ROGUE_CR_AXI_ACE_LITE_CONFIGURATION`, `ROGUE_CR_BIF_CAT_BASE0`,
`ROGUE_CR_FWCORE_MEM_CAT_BASE0`, `ROGUE_CR_FWCORE_BOOT`, `ROGUE_CR_FWCORE_WRAPPER_CONFIG`,
`ROGUE_CR_FWCORE_ADDR_REMAP_CONFIG0` (+ SIZE_SHIFT / MMU_CONTEXT_SHIFT / FETCH_EN_EN /
LOAD_STORE_EN_EN), `ROGUE_CR_MTS_GARTEN_WRAPPER_CONFIG` (+ IDLE_CTRL_META),
`ROGUE_CR_MTS_INTCTX_THREAD0_DM_ASSOC`, `ROGUE_CR_MTS_BGCTX_THREAD0_DM_ASSOC`,
`ROGUE_CR_IRQ_OS0_EVENT_STATUS/CLEAR` (+ SOURCE_EN), `ROGUE_CR_EVENT_STATUS/CLEAR`.
