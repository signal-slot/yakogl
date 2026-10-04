# PowerVR BXS-4-64 (RISC-V FW) — M3/M4 bare-metal implementation spec

Target: BXS-4-64, BVNC 36.53.104.796, TI AM62P. RISC-V firmware processor.
FW heap = 32 MiB (shift 25) at GPU dev-vaddr `ROGUE_FW_HEAP_BASE = 0xE1C0000000`.
MMU context for all FW mappings = `MMU_CONTEXT_MAPPING_FWPRIV = 0x0` (pvr_rogue_defs.h:93).

All citations are `file:line` into the mainline source under `pvr-src/`.

Assumption used throughout: you place ALL GPU-visible memory in one physically
contiguous, CPU-non-cached DDR carveout. "phys" below = the CPU/bus physical
address of a page in that carveout (this is what the mainline driver calls the
`dma_addr`). The GPU MMU translates GPU-dev-vaddr -> phys; you fill the page
tables with these phys addresses.

Two header files were NOT present locally (rate-limited placeholders):
`pvr_rogue_riscv.h` and `pvr_rogue_meta.h`. The RISC-V region macros were
recovered verbatim from mainline and are reproduced in §4. (meta is not needed.)

--------------------------------------------------------------------------------
## 1. GPU MMU page tables
--------------------------------------------------------------------------------

Source: pvr_mmu.c, pvr_rogue_mmu_defs.h. 3-level walk:
L2 = Page Catalogue (PC), L1 = Page Directory (PD), L0 = Page Table (PT).

### 1.1 Device page size
The driver maps only at `PVR_DEVICE_PAGE_SIZE = PAGE_SIZE` granularity
(pvr_mmu.h:51). Use **4 KiB** device pages (`ROGUE_MMUCTRL_PAGE_SIZE_4KB = 0`,
mmu_defs.h:37). This gives the simplest, exact-driver-parity encoding.

### 1.2 40-bit GPU-vaddr index decomposition (mmu_defs.h:21-26)
For a device virtual address `va` (bits [39:0]):

| Level | field       | shift | bits    | #entries | source |
|-------|-------------|-------|---------|----------|--------|
| L2/PC | PC_INDEX    | 30    | [39:30] | 1024 (0x400) | mmu_defs.h:21, :28 |
| L1/PD | PD_INDEX    | 21    | [29:21] | 512  (0x200) | mmu_defs.h:23, :29 |
| L0/PT | PT_INDEX    | 12    | [20:12] | 512  (0x200) | mmu_defs.h:25, :30 |
| page  | offset      | 0     | [11:0]  |          |        |

Index extraction (pvr_mmu.c:1674, :1693, :1712):
```
l2_idx = (va >> 30) & 0x3FF;   /* & ~PC_INDEX_CLRMSK then >>30 */
l1_idx = (va >> 21) & 0x1FF;
l0_idx = (va >> 12) & 0x1FF;   /* for 4KB pages, shift == PAGE_4KB_RANGE_SHIFT == 12 */
```

Each L1 entry spans 2 MiB (512 * 4 KiB). Each L2 entry spans 1 GiB.

### 1.3 Table sizes / alignment
Backing page for every table = 4 KiB, `PVR_MMU_BACKING_PAGE_SIZE = SZ_4K`
(pvr_mmu.c:218). All three table levels are a single 4 KiB page, 4 KiB-aligned:

| Level | entry size | #entries | table bytes | source |
|-------|-----------|----------|-------------|--------|
| L2/PC | 4  (u32)  | 1024     | 4096 | ENTRY_SIZE_PC=0x20 bits (mmu_defs.h:32); pvr_mmu.c:425-429 |
| L1/PD | 8  (u64)  | 512      | 4096 | ENTRY_SIZE_PD=0x40 bits (mmu_defs.h:33); pvr_mmu.c:548-552 |
| L0/PT | 8  (u64)  | 512      | 4096 | ENTRY_SIZE_PT=0x40 bits (mmu_defs.h:34); pvr_mmu.c (4KB) |

### 1.4 Entry encodings

**L2/PC entry (u32)** — pvr_mmu.c:447-457, fields mmu_defs.h:125-134
```
pc_entry = ((u32)(l1_table_phys >> 12) << 4) | 0x1;
           /* PD_BASE at shift 4 (ALIGNSHIFT 12), VALID bit0 */
```
- PD_BASE_SHIFT=4, PD_BASE_ALIGNSHIFT=12, PD_BASE_CLRMSK=0x0000000F (field=bits[31:4]).
- VALID_EN=0x1 (bit0). ENTRY_PENDING=0.
- Because L1 table is 4 KiB-aligned, `(phys>>12)<<4 == (phys & 0xFFFFF000) >> 8`.

**L1/PD entry (u64), 4 KiB pages** — pvr_mmu.c:570-586, fields mmu_defs.h:108-123
```
pd_entry = (l0_table_phys & 0x000000FFFFFFF000ULL) | 0x1;
           /* PT_BASE (4KB) = phys bits[39:12] in place; PAGE_SIZE=4KB(=0); VALID bit0 */
```
- VALID_EN=0x1 (bit0).
- PAGE_SIZE field bits[3:1], value `ROGUE_MMUCTRL_PAGE_SIZE_4KB = 0` (so nothing set).
- PT_BASE for 4KB: `child & ~PT_BASE_4KB_RANGE_CLRMSK`, CLRMSK=0xFFFFFF0000000FFF ->
  mask = 0x000000FFFFFFF000 = bits[39:12] (pvr_mmu.c:585, mmu_defs.h:62-63).
- ENTRY_PENDING (bit40)=0.

**L0/PT entry (u64), 4 KiB pages** — pvr_mmu.c:734-743 + flags :768-781, fields mmu_defs.h:80-106
```
pt_entry = (page_phys & 0x000000FFFFFFF000ULL) | flags | 0x1;
```
- Address field: `phys & ~PAGE_4KB_RANGE_CLRMSK`, CLRMSK=0xFFFFFF0000000FFF ->
  mask = 0x000000FFFFFFF000 = bits[39:12] (mmu_defs.h:44-45).
- VALID_EN = 0x1 (bit0, mmu_defs.h:106). ENTRY_PENDING (bit5) = 0.
- `flags` from `pvr_page_flags_raw_create(read_only, cc, slc_bypass, pm_fw_protect)`
  (pvr_mmu.c:768-781). Bit positions (mmu_defs.h):
  - READ_ONLY        bit1  0x0000000000000002 (:101-103)
  - CC (cache-coh)   bit2  0x0000000000000004 (:98-100)
  - SLC_BYPASS_CTRL  bit3  0x0000000000000008 (:95-97)
  - PM_META_PROTECT  bit62 0x4000000000000000 (:80-82)

**Flag values the driver actually uses for FW memory** (pvr_mmu.c:2597-2599):
`pvr_page_flags_raw_create(false /*ro*/, false /*cc*/, slc_bypass, pm_fw_protect)`
where
- `slc_bypass = (bo_flags & DRM_PVR_BO_BYPASS_DEVICE_CACHE)`, and every FW config
  struct / CCB / code / data object here is created `PVR_BO_FW_FLAGS_DEVICE_UNCACHED`
  which == `DRM_PVR_BO_BYPASS_DEVICE_CACHE` (pvr_gem.h:78) -> **slc_bypass = 1**.
- `pm_fw_protect = (bo_flags & DRM_PVR_BO_PM_FW_PROTECT)`, and ALL FW objects get
  this flag OR'd in automatically (pvr_fw.c:1272-1273) -> **pm_fw_protect = 1**.

So the concrete L0 flag word for the FW heap is:
```
flags = PM_META_PROTECT | SLC_BYPASS_CTRL = 0x4000000000000000 | 0x8
      = 0x4000000000000008
pt_entry(4KB) = (page_phys & 0x000000FFFFFFF000) | 0x4000000000000009   /* +VALID */
```
(If you ever map a *cached* region set slc_bypass=0 -> drop the 0x8.)

### 1.5 Mapping a GPU vaddr -> phys (concrete algorithm, 4 KiB)
```
for each 4KiB page i in [0, 32MiB):
    va   = 0xE1C0000000 + i*0x1000;
    phys = carveout_phys_of(va);            /* your allocator */
    l2 = (va>>30)&0x3FF; l1=(va>>21)&0x1FF; l0=(va>>12)&0x1FF;
    if (!L2[l2] valid) { alloc 4KiB L1 table T1 (4KiB-aligned, zeroed);
                         L2[l2] = ((T1_phys>>12)<<4) | 1; }
    T1 = L1 table pointed by L2[l2];
    if (!T1[l1] valid) { alloc 4KiB L0 table T0 (4KiB-aligned, zeroed);
                         T1[l1] = (T0_phys & 0x000000FFFFFFF000) | 1; }
    T0 = L0 table pointed by T1[l1];
    T0[l0] = (phys & 0x000000FFFFFFF000) | 0x4000000000000009;
```
For a 32 MiB heap starting at 0xE1C0000000 this needs: 1 L2, 1 L1 (16 entries
used), 16 L0 tables (each covers 2 MiB). All memory must be visible to the GPU
(place the tables themselves in the carveout too). If your carveout is CPU-non-
cached you do not need explicit CPU cache flushes of the tables; otherwise flush.

### 1.6 Optional 2 MiB large pages
The HW supports up to 2 MiB pages, but the L1 entry ALWAYS points to an L0 table
(the L0 table just shrinks: 2 MiB -> 1 entry of 8 B; PT_BASE_2MB_RANGE_SHIFT=5,
mmu_defs.h:77). To use them: PD PAGE_SIZE field = `ROGUE_MMUCTRL_PAGE_SIZE_2MB=5`
(bits[3:1]) and the L0 entry address field uses `~PAGE_2MB_RANGE_CLRMSK =
0x000000FFFFE00000` (bits[39:21], mmu_defs.h:59-60). This is NOT simpler than 4 KiB
and diverges from the driver — **recommend 4 KiB uniform pages.**

### 1.7 Root pointer registers (CAT_BASE) — pvr_fw_startstop.c:36-60
Let `pc_phys` = physical address of the L2/PC table (4 KiB-aligned).
Register value packing (startstop.c:46-56):
```
cat_base = ((pc_phys >> ADDR_ALIGNSHIFT) << ADDR_SHIFT) & ~ADDR_CLRMSK
```
For both registers ADDR_ALIGNSHIFT=12, ADDR_SHIFT=12, ADDR_CLRMSK=0xFFFFFF0000000FFF,
so effectively:
```
cat_base = pc_phys & 0x000000FFFFFFF000;   /* 4KiB-aligned phys, bits[39:12] */
```
Write it to BOTH (context index n = MMU_CONTEXT_MAPPING_FWPRIV = 0):
- BIF (host/BIF view):   `ROGUE_CR_BIF_CAT_BASE0        = 0x1200` (cr_defs.h:2380)
  offset = `BIF_CAT_BASEX(0)` = 0x1200 (stride BASE1-BASE0 = 0x1208-0x1200 = 8; defs.h:99).
  fields: ADDR_SHIFT=12, ADDR_ALIGNSHIFT=12, ADDR_CLRMSK=0xFFFFFF0000000FFF,
  MASKFULL=0x000000FFFFFFF000 (cr_defs.h:2381-2385).
- FWCORE (RISC-V core view): `ROGUE_CR_FWCORE_MEM_CAT_BASE0 = 0x30F0` (cr_defs.h:3731)
  offset = `FWCORE_MEM_CAT_BASEX(0)` = 0x30F0 (stride BASE1-BASE0 = 0x30F8-0x30F0 = 8; defs.h:103).
  fields identical (cr_defs.h:3732-3735). Only written for RISC-V (startstop.c:53-59).

Both get the SAME `pc_phys`. Write as 64-bit.

--------------------------------------------------------------------------------
## 2. FW heap layout (32 MiB)
--------------------------------------------------------------------------------

Source: pvr_fw.c:31-45, :888-902; pvr_fw_riscv.c:104.

Constants:
- `raw_size            = 1<<25 = 0x2000000` (32 MiB)              (fw.c:896)
- `offset_mask         = raw_size-1 = 0x1FFFFFF`                  (fw.c:897)
- `CONFIG_HEAP_GRANULARITY = SZ_64K = 0x10000`                    (fw.c:32)
- `CONFIG_HEAP_SIZE    = 3*64K = 0x30000` (192 KiB)               (fw.c:33)
- `config_offset       = raw_size - 0x30000 = 0x1FD0000`          (fw.c:898)
- `heap.size (main)    = raw_size - 0x30000 = 0x1FD0000`          (fw.c:900; reserved_size=0 for riscv, fw_riscv.c:104)
- `heap.gpu_addr = fw_mm_base = ROGUE_FW_HEAP_BASE = 0xE1C0000000`(fw.c:893, :963)

Address helpers:
- GPU dev-vaddr of a heap object: `gpu_addr = 0xE1C0000000 + heap_offset`
  (pvr_fw_obj_get_gpu_addr, fw.c:1463-1470; heap_offset == fw_addr_offset == mm_node.start - fw_mm_base, fw.c:1216).

### 2.1 Main region (bottom of heap): code/data
Allocated bottom-up from offset 0 up to `heap.size` (0x1FD0000):
- **code_obj** MUST be at heap offset 0 (fw.c:651-660): dev-vaddr 0xE1C0000000.
  size = `code_alloc_size` = sum of alloc_size of all FW_CODE layout entries.
- **data_obj**: RISC-V has `has_fixed_data_addr = false` (fw_riscv.c:164), so the
  driver lets the allocator pick the offset (fw.c:674-678). You control placement:
  pick any 4 KiB-aligned offset >= code_alloc_size, size = `data_alloc_size`.
  Record its dev-vaddr (= 0xE1C0000000 + data_offset) for the wrapper (§4).
- Everything else (KCCB, FWCCB, sysdata, osdata, fault page, runtime_cfg, trace,
  gpu_util, hwrinfobuf, power_sync, kccb rtn) is allocated from the same main
  region (offsets in [0, 0x1FD0000)); exact offsets are your choice.

### 2.2 Config region (top 192 KiB) — FIXED offsets, FW discovers these by offset
The FW bootloader looks for these three structures at FIXED heap offsets (they are
NOT pointed to by anything — that is why they are placed by offset). Offsets from
start of config area (fw.c:39-43):
- `CONNECTION_CTL_OFFSET = 0`
- `OSINIT_OFFSET         = 0x10000` (1*granularity)
- `SYSINIT_OFFSET        = 0x20000` (2*granularity)

Absolute heap offsets and dev-vaddrs (config_offset = 0x1FD0000):

| struct           | heap offset | GPU dev-vaddr   | FW-addr (uncached, §5) |
|------------------|-------------|-----------------|------------------------|
| connection_ctl   | 0x1FD0000   | 0xE1C1FD0000    | 0xD1FD0000 |
| osinit           | 0x1FE0000   | 0xE1C1FE0000    | 0xD1FE0000 |
| sysinit          | 0x1FF0000   | 0xE1C1FF0000    | 0xD1FF0000 |

(created via `pvr_fw_object_create_and_map_offset(dev_offset = config_offset + X)`,
fw.c:287-294, :539-558; dev_offset -> dev_addr = fw_mm_base + dev_offset, fw.c:1416.)

--------------------------------------------------------------------------------
## 3. Firmware load
--------------------------------------------------------------------------------

Source: pvr_fw_util.c (ELF stream), pvr_fw.c:226-279 (segment router),
pvr_fw_riscv.c:66-99 (riscv fw_process + boot_data).

### 3.1 Where segment bytes come from
`pvr_fw_process_elf_command_stream` (fw_util.c:28-66) walks the **ELF32 program
headers** and for each `PT_LOAD` segment copies `p_filesz` bytes from
`fw + p_offset`, then zero-fills to `p_memsz`. It routes the destination by
`p_vaddr` through the **layout table** (fw.c:226-279):
```
find layout entry E with E.base_addr <= p_vaddr < E.base_addr+E.alloc_size;
dst_base = { FW_CODE:code_ptr, FW_DATA:data_ptr,
             FW_COREMEM_CODE:core_code_ptr, FW_COREMEM_DATA:core_data_ptr }[E.type];
dst = dst_base + (p_vaddr - E.base_addr) + E.alloc_offset;   /* fw.c:266-273 */
memcpy(dst, fw + p_offset, p_filesz); memset(dst+p_filesz, 0, p_memsz-p_filesz);
```
So the destination heap offset of a section is `E.alloc_offset` within its
allocation (code_obj at heap 0, data_obj at your chosen data_offset), PLUS
`(p_vaddr - E.base_addr)` (which is 0 when a whole section is loaded at its base).

**You must parse the layout table to get `alloc_offset`.** It is embedded in the FW
binary footer (see pvr_fw_info.h): the last 4 KiB region holds a
`pvr_fw_info_header` at `file_size - 4K` (info.h:75-100), followed by
`layout_entry_num` * `pvr_fw_layout_entry` (24 bytes each: id,type,base_addr,
max_size,alloc_size,alloc_offset — all u32/enum-u32, info.h:106-119).

### 3.2 Section -> allocation for BXS-4-64 (RISC-V), with your known numbers
Layout section IDs (info.h:31-35) and types (info.h:38-44):
- `RISCV_UNCACHED_CODE` -> type `FW_CODE` -> **code_obj**
- `RISCV_CACHED_CODE`   -> type `FW_CODE` -> **code_obj**
- `RISCV_PRIVATE_DATA`  -> type `FW_DATA` -> **data_obj**
(No coremem sections in your 3-LOAD image -> core_code/core_data = NULL,
fw.c:686-714, fw_riscv.c:83-93 skipped.)

Your three ELF LOAD segments map as (dev-vaddr == layout base_addr):

| ELF p_vaddr | size (p_filesz) | section id          | dest alloc | dest heap offset |
|-------------|-----------------|---------------------|------------|------------------|
| 0xC0000000  | 8192            | RISCV_UNCACHED_CODE | code_obj   | code_base(0) + alloc_offset_uncached |
| 0x40000000  | 112320          | RISCV_CACHED_CODE   | code_obj   | code_base(0) + alloc_offset_cached   |
| 0x50000000  | 15808           | RISCV_PRIVATE_DATA  | data_obj   | data_base + alloc_offset_priv        |

Read the actual `alloc_offset`/`alloc_size` from the layout table; sizes are
rounded to `FW_BLOCK_SIZE = SZ_4K` (info.h:15). `code_alloc_size` =
sum(alloc_size of the two code entries); `data_alloc_size` = alloc_size(priv).
Note `0xC0000000`, `0x40000000`, `0x50000000` are RISC-V FW *region* addresses
(§4), not GPU dev-vaddrs; the layout base_addr equals the ELF p_vaddr.

### 3.3 RISC-V boot_data (MUST be written into data_obj at offset 0)
After the ELF copy, `pvr_riscv_fw_process` overlays a `rogue_riscv_fw_boot_data`
struct at the **start of the private-data allocation** (`fw_data_ptr`, fw_riscv.c:81):
```
struct rogue_riscv_fw_boot_data {           /* fw_riscv.c:55-64 */
    u64 coremem_code_dev_vaddr;   /* 0 (no coremem) */
    u64 coremem_data_dev_vaddr;   /* 0 */
    u32 coremem_code_fw_addr;     /* 0 */
    u32 coremem_data_fw_addr;     /* 0 */
    u32 coremem_code_size;        /* 0 */
    u32 coremem_data_size;        /* 0 */
    u32 flags;                    /* 0 */
    u32 reserved;                 /* 0 */
};                                /* 40 bytes */
```
For BXS-4-64 with no coremem all fields are 0, but the struct still occupies the
first 40 bytes of the private-data section — this is loaded AFTER the ELF priv
segment (which reserves this space), so it overwrites the first 40 bytes of the
ELF-loaded private data. Just ensure the first 40 bytes of data_obj are these
zeros (i.e. loading the priv ELF segment then zeroing the first 40 bytes is
equivalent when no coremem).

### 3.4 Copy path
The driver stages into kmalloc buffers then memcpy into the mapped FW objects
(fw.c:716-742). For bare metal you can memcpy ELF segment bytes directly into the
carveout at the computed dest offsets. Order does not matter except boot_data must
land last on data_obj[0..40).

--------------------------------------------------------------------------------
## 4. RISC-V wrapper init  (pvr_fw_riscv.c:22-53)
--------------------------------------------------------------------------------

### 4.1 RISC-V FW region macros (recovered verbatim from mainline pvr_rogue_riscv.h)
```
#define ROGUE_RISCVFW_REGION_SIZE   SZ_256M                     /* 0x10000000 */
#define ROGUE_RISCVFW_REGION_SHIFT  __ffs(ROGUE_RISCVFW_REGION_SIZE)   /* = 28 */
#define ROGUE_RISCVFW_REGION_BASE(r)  ((u32)(ROGUE_RISCV_REGION_##r) << 28)
#define ROGUE_RISCVFW_REGION_REMAP_CR(r) \
        (ROGUE_CR_FWCORE_ADDR_REMAP_CONFIG0 + (u32)(ROGUE_RISCV_REGION_##r) * 8U)
```
Region indices (16 regions of 256 MiB; the ones that matter):
| region name                | index | REGION_BASE (idx<<28) | REMAP_CR (0x3000 + idx*8) |
|----------------------------|-------|-----------------------|---------------------------|
| SOCIF                      | 2     | 0x20000000            | CONFIG2  0x3010 |
| BOOTLDR_DATA               | 5     | 0x50000000            | CONFIG5  0x3028 |
| SHARED_CACHED_DATA         | 6     | 0x60000000            | CONFIG6  0x3030 |
| COREMEM                    | 8     | 0x80000000            | CONFIG8  0x3040 |
| BOOTLDR_CODE               | 12    | 0xC0000000            | CONFIG12 0x3060 |
| SHARED_UNCACHED_DATA       | 13    | 0xD0000000            | CONFIG13 0x3068 |
(CONFIG5=0x3028, CONFIG12=0x3060, CONFIG13=0x3068 confirmed cr_defs.h:3383,3530,3551.)
Note these match your ELF dev-vaddrs: uncached_code 0xC0000000 = BOOTLDR_CODE,
private_data 0x50000000 = BOOTLDR_DATA. (cached_code 0x40000000 is region 4, loaded
into code_obj via the layout table; it does not get its own host remap.)

### 4.2 FWCORE ADDR_REMAP_CONFIG register field layout (cr_defs.h:3277-3317; identical for every CONFIGn)
```
CONFIG0 = 0x3000, CONFIG1 = 0x3008, stride 8, MASKFULL = 0x7FFFF7FFFFFFF000
TRUSTED        bit62  EN=0x4000000000000000   (:3280-3282)
LOAD_STORE_EN  bit61  EN=0x2000000000000000   (:3283-3285)
FETCH_EN       bit60  EN=0x1000000000000000   (:3286-3288)
SIZE           shift44  CLRMSK=0xF0000FFFFFFFFFFF   (:3289-3290)
CBASE          shift40  CLRMSK=0xFFFFF8FFFFFFFFFF   (:3291-3292)  <- MMU context
DEVVADDR       shift12  CLRMSK=0xFFFFFF0000000FFF ALIGNSHIFT12    (:3293-3296)
```
Aliases used by the code (pvr_rogue_defs.h:111-115):
- `FWCORE_ADDR_REMAP_CONFIG0_MMU_CONTEXT_SHIFT = CBASE_SHIFT = 40`
- `FWCORE_ADDR_REMAP_CONFIG0_SIZE_ALIGNSHIFT   = 12`

### 4.3 common_opts and the two writes (fw_riscv.c:25-46)
```
common_opts = ((ROGUE_FW_HEAP_RISCV_SIZE >> 12) << 44)          /* SIZE field */
            | ((u64)MMU_CONTEXT_MAPPING_FWPRIV << 40);          /* CBASE=0 */
```
Concrete for 32 MiB heap, context 0:
```
(0x2000000 >> 12) = 0x2000;  0x2000 << 44 = 0x0200000000000000;
common_opts = 0x0200000000000000
```
DEVVADDR is OR'd in directly (the code asserts DEVVADDR_SHIFT==ALIGNSHIFT==12 and
WARNs if `addr & 0xFFFFFF0000000FFF` is nonzero, i.e. addr must be 4 KiB-aligned
and < 2^40; fw_riscv.c:35-39). Both `code_addr` and `data_addr` are full GPU
dev-vaddrs from §2 (`pvr_fw_obj_get_gpu_addr`).

**Write 1 — BOOTLDR_CODE** (fw_riscv.c:41-42), reg `ROGUE_RISCVFW_REGION_REMAP_CR(BOOTLDR_CODE)` = **0x3060**:
```
val = code_addr | common_opts | FETCH_EN_EN
    = code_addr | 0x0200000000000000 | 0x1000000000000000
    = code_addr | 0x1200000000000000
```
With code at heap offset 0 (code_addr = 0xE1C0000000):
```
= 0x120000E1C0000000
```

**Write 2 — BOOTLDR_DATA** (fw_riscv.c:44-46), reg `ROGUE_RISCVFW_REGION_REMAP_CR(BOOTLDR_DATA)` = **0x3028**:
```
val = data_addr | common_opts | LOAD_STORE_EN_EN
    = data_addr | 0x0200000000000000 | 0x2000000000000000
    = data_addr | 0x2200000000000000
```
(data_addr = 0xE1C0000000 + your data_offset.)

**Write 3 — Garten wrapper** (fw_riscv.c:49-50), reg `ROGUE_CR_MTS_GARTEN_WRAPPER_CONFIG` = **0x0B50** (cr_defs.h:1971):
```
val = ROGUE_CR_MTS_GARTEN_WRAPPER_CONFIG_IDLE_CTRL_META = 0x0   (cr_defs.h:1989)
```
(IDLE bit controlled by RISC-V; IDLE_CTRL is bit0, META=0.)

All three are 64-bit writes. The SHARED_CACHED/SHARED_UNCACHED/COREMEM/SOCIF
region remaps are configured by the FW bootloader itself (from boot_data and its
own tables) — the host programs ONLY these two bootloader remaps plus the FWCORE
page-catalogue base (§1.7). FW data accesses (KCCB etc. at 0xD0000000+off /
0x60000000+off) are translated by the FWCORE MMU via FWCORE_MEM_CAT_BASE0.

--------------------------------------------------------------------------------
## 5. FWIF control structures
--------------------------------------------------------------------------------

### 5.1 FW-address encoding (RISC-V) — flag any GPU-addr-packed field
`get_fw_addr_with_offset` (fw_riscv.c:109-121): a FW object's **FW address** (the
32-bit value stored in every `*_fw_addr` FWIF field, type `rogue_fwif_dev_virtaddr`
= a u32) is:
```
fw_addr = heap_offset(object) | REGION_BASE;
  REGION_BASE = SHARED_UNCACHED_DATA (0xD0000000) if object is DEVICE_UNCACHED,
              = SHARED_CACHED_DATA   (0x60000000) otherwise.
```
Every config struct / CCB / CCB-ctl / sysdata / osdata / etc. here is UNCACHED, so
`fw_addr = heap_offset | 0xD0000000`. These are little-endian u32. (heap_offset =
gpu_addr - 0xE1C0000000, < 0x2000000, so the OR is clean.)

### 5.2 rogue_fwif_ccb_ctl — the CCB control block (fwif.h:805-817)
```
struct rogue_fwif_ccb_ctl {              /* __aligned(8), 144 bytes */
    u32 write_offset;      /* @0   host increments */
    u8  padding[124];      /* @4   (128 - 4)  separate cache lines */
    u32 read_offset;       /* @128 FW increments  */
    u32 wrap_mask;         /* @132 = num_cmds - 1 */
    u32 cmd_size;          /* @136 = sizeof(one command) */
    u32 padding2;          /* @140 */
};
```
Init (pvr_ccb.c:30-35, :85-88):
```
write_offset = 0; read_offset = 0;
wrap_mask    = num_cmds - 1;
cmd_size     = <command size>;
```

### 5.3 KCCB / FWCCB sizing (pvr_fw.h:22-24, pvr_ccb.c:544-553, :662-666)
- KCCB: `NUMCMDS_LOG2_DEFAULT = 7` -> **128 slots**; command =
  `sizeof(struct rogue_fwif_kccb_cmd)` (fwif.h:1245-1320, a big tagged union,
  `PVR_FW_STRUCT_SIZE_ASSERT`; compute with sizeof of your mirrored struct).
  KCCB buffer size = 128 * sizeof(kccb_cmd). Command layout (fwif.h:1245-1319):
  `{ enum cmd_type; u32 kccb_flags; union cmd_data __aligned(8); } __aligned(8);`
- KCCB return-slots object: `128 * sizeof(u32)` = 512 B (fw.c:949, :985-988);
  its fw_addr goes in `osinit.kernel_ccb_rtn_slots_fw_addr`.
- FWCCB: `NUMCMDS_LOG2 = 5` -> **32 slots**; command = `sizeof(rogue_fwif_fwccb_cmd)`.
Each CCB is two objects: a `ccb_ctl` (ctrl_fw_addr) and the command array
(ccb_fw_addr). (pvr_ccb.c:50-88)

### 5.4 rogue_fwif_osinit (fwif.h:1785-1807), at heap 0x1FE0000 / fw_addr 0xD1FE0000
```
struct rogue_fwif_osinit {                       /* __aligned(8) */
    u32 kernel_ccbctl_fw_addr;        /* KCCB ccb_ctl fw_addr           */
    u32 kernel_ccb_fw_addr;           /* KCCB command array fw_addr     */
    u32 kernel_ccb_rtn_slots_fw_addr; /* KCCB rtn u32[128] fw_addr      */
    u32 firmware_ccbctl_fw_addr;      /* FWCCB ccb_ctl fw_addr          */
    u32 firmware_ccb_fw_addr;         /* FWCCB command array fw_addr     */
    u32 work_est_firmware_ccbctl_fw_addr;  /* 0 (unused)                */
    u32 work_est_firmware_ccb_fw_addr;     /* 0 (unused)                */
    u32 rogue_fwif_hwr_info_buf_ctl_fw_addr;/* hwrinfobuf fw_addr       */
    u32 hwr_debug_dump_limit;         /* 0                              */
    u32 fw_os_data_fw_addr;           /* osdata fw_addr                 */
    struct rogue_fwif_compchecks rogue_comp_checks; /* FW fills; host 0 */
};
```
Init source: fw.c:312-339. `rogue_comp_checks` is written by the FW; host inits the
two bvnc sub-fields via `rogue_fwif_compchecks_bvnc_init` (fwif.h:1646) which
zero them — leaving the struct zeroed is fine for boot.

### 5.5 rogue_fwif_sysinit (fwif.h:1844-1921), at heap 0x1FF0000 / fw_addr 0xD1FF0000
Field list with the minimal required values (init source fw.c:360-407):
```
aligned_u64 fault_phys_addr;   /* PHYS addr of a 4KiB fault page (NOT a fw_addr!) */
aligned_u64 pds_exec_base;     /* = ROGUE_PDSCODEDATA_HEAP_BASE = 0xDA00000000 (heap_config.h:61) */
aligned_u64 usc_exec_base;     /* = ROGUE_USCCODE_HEAP_BASE     = 0xE000000000 (heap_config.h:79) */
aligned_u64 fbcdc_state_table_base;        /* 0 */
aligned_u64 fbcdc_large_state_table_base;  /* 0 */
aligned_u64 texture_heap_base;             /* 0 */
u64 hw_perf_filter;            /* 0 */
aligned_u64 slc3_fence_dev_addr;           /* 0 */
u32 tpu_trilinear_frac_mask[ROGUE_FWIF_TPU_DM_LAST] __aligned(8); /* 0 */
struct rogue_fwif_sigbuf_ctl sigbuf_ctl[PVR_FWIF_DM_MAX];         /* 0 */
struct rogue_fwif_pdvfs_opp  pdvfs_opp_info;                      /* 0 */
struct rogue_fwif_dma_addr   coremem_data_store;    /* .fw_addr=0 (no coremem) */
struct rogue_fwif_counter_dump_ctl counter_dump_ctl;/* .buffer_fw_addr=0,.size_in_dwords=0 */
u32 filter_flags;              /* 0 */
u32 runtime_cfg_fw_addr;       /* runtime_cfg obj fw_addr */
u32 trace_buf_ctl_fw_addr;     /* trace-buf ctl obj fw_addr */
u32 fw_sys_data_fw_addr;       /* sysdata obj fw_addr */
u32 gpu_util_fw_cb_ctl_fw_addr;/* gpu_util fwcb obj fw_addr */
u32 reg_cfg_fw_addr;           /* 0 */
u32 hwperf_ctl_fw_addr;        /* 0 */
u32 align_checks;              /* 0 (skip) */
u32 initial_core_clock_speed;  /* core clock Hz (nonzero!) */
u32 active_pm_latency_ms;      /* 0 */
bool firmware_started __aligned(4);  /* 0 on host; FW sets nonzero when booted */
u32 marker_val;                /* 1 */
u32 firmware_started_timestamp;/* 0 (FW writes) */
u32 jones_disable_mask;        /* 0 */
enum fw_perf_conf firmware_perf;      /* FW_PERF_CONF_NONE = 0 */
u32 core_clock_rate_fw_addr;   /* 0 */
enum rogue_fwif_gpio_val_mode gpio_validation_mode; /* ROGUE_FWIF_GPIO_VAL_OFF = 0 */
struct rogue_hwperf_bvnc bvnc_km_feature_flags;     /* memset 0 */
u32 tfbc_compression_control;  /* 0 */
```
GPU-addr-packed / endianness notes:
- `fault_phys_addr`, `pds_exec_base`, `usc_exec_base` are 64-bit **GPU/phys**
  addresses (aligned_u64), little-endian. `fault_phys_addr` is a raw PHYSICAL
  address of a 4 KiB page (fw.c:372-373), the others are GPU dev-vaddr heap bases.
- all `*_fw_addr` are 32-bit FW addresses per §5.1 (heap_offset | 0xD0000000).
- `initial_core_clock_speed` (fw.c:399) and runtime_cfg.core_clock_speed
  (fw.c:436) must be a real nonzero Hz value; the driver WARNs if 0.

### 5.6 rogue_fwif_connection_ctl (fwif.h:1777-1783), at heap 0x1FD0000 / fw_addr 0xD1FD0000
```
struct rogue_fwif_connection_ctl {          /* __aligned(8), 16 bytes */
    enum rogue_fwif_connection_fw_state connection_fw_state; /* u32 */
    enum rogue_fwif_connection_os_state connection_os_state; /* u32 */
    u32 alive_fw_token;
    u32 alive_os_token;
};
```
The open-source native driver does NOT write these before boot (searched: no host
writes; enums fwif.h:165-186). Leave the whole struct **zeroed**
(`FW_OFFLINE=0`, `OS_OFFLINE=0`). It is only used for the guest/OS connection
handshake and is not required to reach `firmware_started`.

### 5.7 Supporting objects the FW dereferences during boot (all UNCACHED)
Create + zero/init these and wire their fw_addrs into osinit/sysinit above
(fw.c:454-565). Minimal set:
- `power_sync` (u32), `hwrinfobuf` (rogue_fwif_hwrinfobuf), `sysdata`
  (rogue_fwif_sysdata; set config_flags per fw.c:411-424), `osdata`
  (rogue_fwif_osdata; set power_sync_fw_addr, fw.c:341-349),
- `fault_page` (4 KiB filled with 0xdeadbee0, fw.c:352-358),
- `gpu_util_fwcb` (rogue_fwif_gpu_util_fwcb; last_word = PVR_FWIF_GPU_UTIL_STATE_IDLE, fw.c:446-452),
- `runtime_cfg` (rogue_fwif_runtime_cfg; core_clock_speed, active_pm_latency_ms=0,
  active_pm_latency_persistant=true, default_dusts_num_init=num_clusters,
  wdg_period_us=0, fw.c:427-444),
- FW trace buf ctl (`trace_buf_ctl_fw_addr`),
- KCCB + KCCB rtn + FWCCB (§5.3).

### 5.8 firmware_started — the poll target
`firmware_started` is a 4-byte field inside `rogue_fwif_sysinit` (fwif.h:1894).
Poll it at:
```
CPU address  = carveout_cpu_base + 0x1FF0000 + offsetof(struct rogue_fwif_sysinit, firmware_started)
GPU dev-vaddr= 0xE1C1FF0000 + offsetof(..., firmware_started)
```
Because it comes after several sub-structs whose sizes depend on enums
(ROGUE_FWIF_TPU_DM_LAST, PVR_FWIF_DM_MAX, pdvfs/sigbuf/counter_dump layouts),
**define the identical `rogue_fwif_sysinit` in your C and use `offsetof`** rather
than a hard-coded number. Read it with a volatile/`READ_ONCE`-style 32-bit load
(driver: fw.c:875). FW writes nonzero (true) when boot completes.

--------------------------------------------------------------------------------
## 6. M4 boot handshake
--------------------------------------------------------------------------------

Sequence (pvr_fw_startstop.c pvr_fw_start :104-194, then pvr_wait_for_fw_boot fw.c:868-880):

Preconditions before this: page tables built (§1), FW loaded (§3), all FWIF
structures written (§5). Then:

1. If feature `sys_bus_secure_reset` (you have it): write
   `ROGUE_CR_SYS_BUS_SECURE = 0` (32-bit) and read it back to fence
   (startstop.c:116-123). Find its offset in cr_defs.h (`ROGUE_CR_SYS_BUS_SECURE`).
2. RISC-V: `ROGUE_CR_FWCORE_BOOT (0x3090) = 0` (startstop.c:125-126; cr_defs.h:3614).
3. Soft-reset dance (you have `pbe2_in_xe` and `xe_tpu2` -> use `__PBE2_XE__MASKFULL`
   and SOFT_RESET2). Sequence (startstop.c:111-155):
   - `SOFT_RESET  = ROGUE_CR_SOFT_RESET__PBE2_XE__MASKFULL`; `SOFT_RESET2 = SOFT_RESET2_MASKFULL`; read back both.
   - `SOFT_RESET ^= RASCALDUSTS_EN` (take Rascal+Dusts out); `SOFT_RESET2 = 0`; read back.
   - `SOFT_RESET  = GARTEN_EN` (everything out but FW core); `SOFT_RESET2 = 0`; read back.
4. `rogue_slc_init` (startstop.c:62-94): read-modify-write `ROGUE_CR_SLC_CTRL_MISC`
   (32-bit): keep `ENABLE_PSG_HAZARD_CHECK_EN` bit, set
   `ADDR_DECODE_MODE_PVR_HASH1`; if slc_cache_line_size_bits<1024 set
   `BYPASS_BURST_COMBINER_EN`. (quirk 71242 needs gpu_multicore off — you HAVE
   multicore, so skip LAZYWB.)
5. `wrapper_init` (§4): the 2 remap writes + Garten config.
6. `rogue_axi_ace_list_init` (startstop.c:19-33): write 64-bit
   `ROGUE_CR_AXI_ACE_LITE_CONFIGURATION` = OR of the 8 domain/cache fields shown
   there (all "outer cache"). MUST precede first BIF transaction.
7. `rogue_bif_init` (§1.7): write BIF_CAT_BASE0 (0x1200) and, for RISC-V,
   FWCORE_MEM_CAT_BASE0 (0x30F0), both = `pc_phys & 0x000000FFFFFFF000`.
8. `udelay(3)`.
9. `ROGUE_CR_SOFT_RESET = 0` (take FW core out of reset); read back (startstop.c:175-176).
10. `udelay(3)`.
11. **`ROGUE_CR_FWCORE_BOOT (0x3090) = 1`** (startstop.c:183; ENABLE_EN=0x1,
    cr_defs.h:3618). `udelay(3)`.
12. Poll `sysinit->firmware_started` (§5.8) until nonzero, timeout
    `FW_BOOT_TIMEOUT_USEC = 5_000_000` (5 s) (fw.c:29, :868-880). Nonzero = booted.

### 6.1 IRQ (OS0) registers (fw_riscv.c:141-153)
- Pending check: read 32-bit `ROGUE_CR_IRQ_OS0_EVENT_STATUS = 0x0BD8`
  (cr_defs.h:2085), test `SOURCE_EN = 0x1` (bit0, cr_defs.h:2089).
- Clear: write 32-bit `ROGUE_CR_IRQ_OS0_EVENT_CLEAR = 0x0BE8`
  (cr_defs.h:2092) = `SOURCE_EN = 0x1` (cr_defs.h:2096).
The FW raises OS0 IRQ once running; not required to observe `firmware_started`
(that is a memory flag), but use it to drain/ack FW->host events afterwards.

--------------------------------------------------------------------------------
## Quick-reference: register offsets
--------------------------------------------------------------------------------
```
ROGUE_CR_BIF_CAT_BASE0                 0x1200   (stride 8; ctx0)
ROGUE_CR_FWCORE_MEM_CAT_BASE0          0x30F0   (stride 8; ctx0)
ROGUE_CR_FWCORE_ADDR_REMAP_CONFIG0     0x3000   (stride 8)
  BOOTLDR_DATA  (region 5)  -> CONFIG5  0x3028
  BOOTLDR_CODE  (region 12) -> CONFIG12 0x3060
ROGUE_CR_MTS_GARTEN_WRAPPER_CONFIG     0x0B50
ROGUE_CR_FWCORE_BOOT                   0x3090
ROGUE_CR_IRQ_OS0_EVENT_STATUS          0x0BD8   (SOURCE bit0)
ROGUE_CR_IRQ_OS0_EVENT_CLEAR           0x0BE8   (SOURCE bit0)
ROGUE_CR_SOFT_RESET / SOFT_RESET2 / SLC_CTRL_MISC / AXI_ACE_LITE_CONFIGURATION /
ROGUE_CR_SYS_BUS_SECURE                (look up offsets in cr_defs.h as needed)
```
