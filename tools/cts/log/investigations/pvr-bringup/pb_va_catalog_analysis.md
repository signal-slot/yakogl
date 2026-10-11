<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Parameter-Buffer VA space & catalog routing — DDK 25.2 analysis

Status: finished record, moved here from `driver/docs/gpu-recipes/pb_va_catalog_analysis.md` on 2026-10-11. A 2026-09 analysis of the parameter-buffer fault. The fault is resolved; the PM and freelist setup is driver/rust/src/gpu/pvr/freelist.rs and render.rs.

Scope: why the 3D/ISP faults reading the TA-tiled parameter buffer on our bare-metal BXS-4-64
driver. All paths under `.../scratchpad/ti-img-rogue-driver-25.2`.

## TL;DR

On Rogue the parameter buffer is **not** mapped in the app's normal page catalog. It lives in a
**private PM virtual address space** (0 .. `RGX_PM_MAX_PB_VIRT_ADDR_SPACE` = 16 GiB) that is
translated by the **PM's own MMU**, whose page tables are the **MList** and whose catalog-base
registers are the dedicated **`RGX_CR_BIF_PM_CAT_BASE_{VCE,TE,ZLS,ALIST}{0,1}`** (catalog slots
**8..0xF**, distinct from the 8 normal contexts 0..7). Those PM cat-base register values are held in
the **`RGXFWIF_HWRTDATA`** struct (`ui64VCECatBase`/`ui64TECatBase`/`ui64AlistCatBase` + `psPMMListDevVAddr`
+ `psVHeapTableDevVAddr`) and are **programmed by the firmware**, and **saved at TA-finish / restored
at 3D-kick** so the 3D reads back exactly the PM mapping the TA wrote.

Our fault is `BIF_FAULT_BANK0`, `cat_base=1` — a **normal render-context** catalog slot (0..7), **not**
a PM catalog (8..0xF). So the faulting access is a *normal-context* fetch by a pixel-side unit
(requester tag 0x14 = `MCU_PDS_USCA`, the USC/MCU coefficient/PDS fetch), to an address
(`0x4c4000000`) that is not mapped in the app context. That address is *derived from* what the 3D
read out of the tiled PB. The root cause is that the **PM-MMU state (PM cat bases + MList + VHeap)
is not correctly carried from the TA phase into the 3D phase**, so the 3D reads wrong tiled data and
the USC then dereferences a garbage pointer in the normal context.

---

## 1. What VA space do TA-written region-header / primitive pointers use?

The **PM parameter-buffer virtual address space**, base 0, size
`RGX_PM_MAX_PB_VIRT_ADDR_SPACE = 0x400000000` (16 GiB).

- `hwdefs/rogue/km/rgxdefs_km.h:239-248`
  - `RGX_NUM_PM_ADDR_SPACES (3U)  /* VCE, TE and Alist */`
  - `RGX_PM_MAX_PB_VIRT_ADDR_SPACE (0x400000000)  /* PM Maximum addressable limit */`
  - `RGX_BIF_PM_PHYSICAL_PAGE_ALIGNSHIFT (12U)` → PB backing pages are 4 KiB
  - `RGX_BIF_PM_VIRTUAL_PAGE_ALIGNSHIFT (14U)` → PM *virtual* pages are 16 KiB
- This is a **separate address space**, not one of the heaps in `include/rogue/rgxheapconfig.h`.
  The freelist backing pages get **no device VA in the app context**: they are allocated as a bare
  PMR (`PhysmemNewRamBackedPMR`, `PHYS_HEAP_HINT(GPU_PRIVATE)`, `GPU_READABLE` only —
  `rgxta3d.c:948-957`) and referenced only by **PFN** in the freelist array.
- The freelist array entries are `phys >> 12` (PFN), confirmed in
  `services/server/common/pmr.c:4366-4367`:
  `paui32AddrShifted[i] = TRUNCATE_64BITS_TO_32BITS(DevPAddr >> uiLog2PageSize)` with
  `uiLog2PageSize = RGX_BIF_PM_PHYSICAL_PAGE_ALIGNSHIFT (12)`. **Our PFN=phys>>12 encoding matches.**
- The FW freelist struct calls `psFreeListDevVAddr` the *"Freelist page table base"* and
  `ui64CurrentDevVAddr` the *"Freelist page table entry for current free page"*
  (`include/rogue/rgx_fwif_shared.h:944-948`). The PM pops PFNs from this array as it needs pages.

## 2. How is the PB VA space translated for the 3D read? (the PM MMU, not the main MMU)

There is a **dedicated PM MMU**, separate from the app's page catalog:

- **Requester → catalog-slot routing** is `RGX_CR_BIF_CAT_BASE_INDEX` (`rgx_cr_defs_km.h:3568-3588`).
  Each data master's *normal* accesses map to a **normal** catalog slot 0..7: fields `TA` (shift 0),
  `PIXEL` (shift 8), `TDM`, `CDM`, `TLA`, `HOST`, `RAY`, `RVTX`. (Slot 0 = FW,
  `MMU_CONTEXT_MAPPING_FWPRIV`, `include/rgx_common.h:77`.)
- **PM address spaces have their own catalog-base registers**, slots 8..0xF:
  `RGX_CR_BIF_PM_CAT_BASE_VCE0` (0x1248), `_TE0` (0x1250), `_ALIST0`, and the `_VCE1/_TE1/_ZLS.../_ALIST1`
  variants (`rgx_cr_defs_km.h:3591+`, dumped at `rgxdebug.c:3173-3178`). Each has `ADDR` (page catalog
  phys addr, shift 12), `INIT_PAGE` (shift 40), `WRAP`, `VALID` fields
  (`rgx_cr_defs_km.h:3593-3608`).
- The fault-status `CAT_BASE` field is 4 bits (`SHIFT=12`, `CLRMSK=0xFFFF0FFF`,
  `rgx_cr_defs_km.h:3802-3803`), values 0..0xF. The debug decoder proves the split:
  `rgxdebug.c:1511` prints `(ui32PC < 0x8) ? "" : _RGXDecodePMPC(ui32PC)`, and
  `_RGXDecodePMPC` (`rgxdebug.c:198-`) maps **0x8=PM-VCE0, 0x9=PM-TE0, 0xA=PM-ZLS0, 0xB=PM-ALIST0,
  0xC=PM-VCE1, 0xD=PM-TE1, 0xE=PM-ZLS1, 0xF=PM-ALIST1**. So a *correct* PB access shows cat_base 8..0xF.
- The PM MMU's page tables are the **MList** (3-level PCE/PDE/PTE), sized from the PB size:
  `RGXCalcMListSize()` (`rgxutils.c:296-329`) computes PTE/PDE/PCE pages from
  `(MaxLocalPB + MaxGlobalPB)/RGX_BIF_PM_PHYSICAL_PAGE_SIZE`, asserting the total PB
  `<= RGX_PM_MAX_PB_VIRT_ADDR_SPACE` (`rgxutils.c:310-311`). The MList is allocated as a reservation
  in `RGXCreateHWRTDataSet` (`rgxta3d.c:2083-2093`), one MList slice per RTData
  (`rgxta3d.c:2191-2193`).
- The main-MMU tag decode confirms the PM walks page tables through the shared MMU: SB values
  `PM PT / PM PD / PM PC / PM PC W / PM PD W` (`rgxdebug.c:305-319`), and the PM stack tags
  `TA MMUstack / 3D MMUstack` (`rgxmhdefs_km.h`, decoded `rgxdebug.c:630-635`).

**Who programs the PM cat bases:** the **firmware**, not the KM driver. The KM only fills the
`RGXFWIF_HWRTDATA` struct; the PM cat-base *values* are FW-managed state stored there:

- `include/rogue/rgx_fwif_shared.h:1242-1302` (`RGXFWIF_HWRTDATA`, *"used to hold the PM context"*):
  - `psPMMListDevVAddr` — *MList Data Store* (line 1243)
  - `ui64VCECatBase[1]`, `ui64VCELastCatBase[1]` (1245-1246)
  - `ui64TECatBase[1]`, `ui64TELastCatBase[1]` (1247-1248)
  - `ui64AlistCatBase`, `ui64AlistLastCatBase` (1249-1251)
  - `ui64PMAListStackPointer`, `ui32PMMListStackPointer` (1253-1255)
  - `psVHeapTableDevVAddr` — *VHeap table base* (1274-1275)
  - plus `sTailPtrsDevVAddr`, `sMacrotileArrayDevVAddr`, `sRgnHeaderDevVAddr`, `sRTCDevVAddr`
    (1279-1286) — these ARE normal-context device VAs (app heaps).
- The KM leaves the CatBase fields **zero** (it only sets VHeap, PMMList, TailPtrs, Macrotile,
  RgnHeader, RTC in `RGXCreateHWRTData_aux`, `rgxta3d.c:1840-1856`). The PM/FW seeds the cat bases
  from the MList base + VHeap + freelist at the first kick, updates them (`INIT_PAGE`/stack pointer)
  as the PM consumes pages, **saves** them back to HWRTDATA at TA-finish and **restores** them at
  3D-kick. This save/restore is what makes the 3D see the same PB mapping the TA produced.

So: **the 3D reads the tiled PB through the PM catalogs (8..0xF), driven by the FW-persisted
HWRTDATA PM state — not through the app's normal page catalog.**

## 3. Why a fault at 0x4c4000000 with cat_base=1

`cat_base=1` is a **normal render-context** slot (1 = the app context the FW loaded into a
`BIF_CAT_BASE1` slot; PIXEL is routed there by `BIF_CAT_BASE_INDEX_PIXEL`). It is **not** a PM
catalog (those are 8..0xF). The requester tag (0x14 = `RGX_MH_TAG_ENCODING_MH_TAG_MCU_PDS_USCA`,
`rgxmhdefs_km.h:228`; XE TAG_ID is bits 40..45, `rgx_cr_defs_km.h:3845`) is the **MCU fetching
PDS/USC data for the pixel shader** — i.e. the fragment stage reading coefficient/PDS pointers.

So the faulting access is a **normal-context pixel-side fetch to an unmapped VA**, not the PM MMU
failing to resolve a PB address. `0x4c4000000` (~19 GiB) is also **outside** the 0..16 GiB PB space
and matches **no heap** in `rgxheapconfig.h` — it is a **garbage pointer**, not a real PB or heap VA.

Where does a garbage pixel-side pointer come from? From data the 3D read out of the tiled PB. If the
PM-MMU view during 3D does not match what the TA wrote (stale/absent PM cat bases, MList not carried
over, VHeap wrong), the 3D/ISP reads the *wrong physical pages*, so the primitive/coefficient/PDS
pointers it recovers are garbage, and the USC/MCU then dereferences one (0x4c4000000) in the normal
context → `BIF_FAULT_BANK0 cat_base=1`.

Corollary: the user's premise ("0x4c4000000 is in the PB virtual space, cat_base 1 should map it")
is not quite right — the PB is never in a normal catalog, and this VA is out of PB range. The bug is
upstream in the TA→3D PM-state handover, surfacing as a bad normal-context fetch.

## 4. Concrete minimal changes (prioritized)

The fix is to make the **PM-MMU state persist correctly from TA into 3D** so the 3D reads the real
tiled data, and to make sure every buffer each phase touches is mapped in the right catalog.

**(A) Verify/repair the PM cat-base save→restore across TA and 3D.** This is the prime suspect.
- Our FW port must, at TA-kick, load `RGX_CR_BIF_PM_CAT_BASE_VCE0/TE0/ALIST0` from
  `RGXFWIF_HWRTDATA.ui64VCECatBase/ui64TECatBase/ui64AlistCatBase` (+ VHeap/MList), and at TA-finish
  write the HW's updated PM cat bases (VCE/TE/ALIST + their `INIT_PAGE`/stack pointers, and
  `ui32PMMListStackPointer`/`ui64PMAListStackPointer`) **back** into the same HWRTDATA fields
  (`rgx_fwif_shared.h:1245-1255`). At 3D-kick it must **restore** those into
  `RGX_CR_BIF_PM_CAT_BASE_*` (the 3D uses VCE/TE/ALIST for reading, plus ZLS for depth).
  If our port skips this save/restore, the 3D runs with stale or zero PM cat bases → reads garbage.
  Instrument: dump `RGX_CR_BIF_PM_CAT_BASE_VCE0/TE0/ALIST0` **at 3D-kick** (not just during TA) and
  compare to the values seen at TA-finish and to `HWRTDATA.ui64VCECatBase` etc. They must be equal
  and `VALID`. (The user already sees them non-zero during tiling; confirm they are non-zero **and
  identical** at 3D-kick.)

**(B) Make sure the MList and VHeap are set up and zero-initialised, and the HWRTDATA PM CatBase
fields start at 0.**
- MList size must come from `RGXCalcMListSize()` for the actual PB size (`rgxutils.c:296`), one slice
  per RTData (`rgxta3d.c:2191-2193`), and the region must be zeroed so the PM builds valid PC/PD/PT.
- `HWRTDATA.psPMMListDevVAddr`/`psVHeapTableDevVAddr` must be the correct client device VAs
  (`rgxta3d.c:1841,1848`); the `ui64*CatBase` fields must be **0** at creation (FW seeds them). If our
  port pre-fills them wrongly, the FW will program bad PM catalogs.

**(C) Confirm the normal-context buffers the 3D touches are mapped in the render context (cat_base 1).**
The fault is a normal-context miss, so also verify these are present and correct in our single-page
catalog `g_pc`:
- `sRgnHeaderDevVAddr`, `sMacrotileArrayDevVAddr`, `sTailPtrsDevVAddr`, `sRTCDevVAddr`
  (`rgx_fwif_shared.h:1279-1286`) — region headers/macrotile/tailptrs the ISP reads.
- The global `PDSCODEDATA` (0xDA00000000) and `USCCODE` (0xE000000000) heaps, which are programmed
  into `RGX_CR_PDS_EXEC_BASE` / `RGX_CR_USC_CODE_BASE_*` (`rgxheapconfig.h:74-77`) — the MCU_PDS_USCA
  requester resolves pixel-shader PDS/USC fetches relative to these. If the app's PDS/coefficient
  pointers are correct but these base regs or mappings are wrong, the same requester faults.

**(D) Disambiguating instrumentation (to confirm A vs C before coding):**
1. At 3D-kick, log `BIF_PM_CAT_BASE_VCE0/TE0/ALIST0` and compare to TA-finish values.
2. Add a known sentinel to the tiled PB during TA and read it back at 3D start via the PM catalog to
   confirm the 3D's PM view resolves to the TA's backing pages.
3. Decode the exact faulting requester on-board with `_RGXDecodeBIFReqTags` semantics
   (`rgxdebug.c:1519-1569`): confirm tag_id and whether tag_sb points at PDS/coefficient vs a
   primitive-list read. If it is a primitive-list read at cat_base 8..0xF next time, the problem is
   purely PM-catalog (A/B); if it stays MCU/USC at cat_base 1, it is a derived-pointer/PDS-base
   issue (C) fed by bad PB data (still rooted in A/B).

### Order of work
1. (A) PM cat-base save/restore across TA→3D — most likely root cause; verify with (D1).
2. (B) MList/VHeap zero-init + HWRTDATA CatBase=0 — precondition for (A) to produce valid catalogs.
3. (C) normal-context mappings (RgnHeader/Macrotile/TailPtrs/RTC + PDSCODEDATA/USCCODE bases) — the
   surface where the fault actually lands.

## Key citations
- `hwdefs/rogue/km/rgxdefs_km.h:239-250` — PM addr spaces, PB VA limit, PM page shifts.
- `hwdefs/rogue/km/rgx_cr_defs_km.h:3570-3628` — `BIF_CAT_BASE_INDEX` requester routing; `BIF_PM_CAT_BASE_*` regs.
- `hwdefs/rogue/km/rgx_cr_defs_km.h:3802-3803,3836-3846` — fault CAT_BASE (4-bit) & XE tag fields.
- `services/server/devices/rogue/rgxdebug.c:198-` `_RGXDecodePMPC` — cat_base 8..0xF = PM catalogs.
- `services/server/devices/rogue/rgxdebug.c:275-319,603-635` — MMU/PM tag decode (PM PT/PD/PC, MMUstack).
- `services/server/devices/rogue/rgxta3d.c:948-1027` — PB backing PMR (PFN-only) + freelist fill.
- `services/server/common/pmr.c:4366-4367` — freelist entry = phys>>12 (matches ours).
- `services/server/devices/rgxutils.c:296-329` — MList sizing vs PB VA space.
- `services/server/devices/rogue/rgxta3d.c:2083-2093,2189-2199,1840-1856` — MList/HWRTData wiring.
- `include/rogue/rgx_fwif_shared.h:944-971` — FW freelist ("page table base").
- `include/rogue/rgx_fwif_shared.h:1242-1302` — `RGXFWIF_HWRTDATA` PM state (cat bases, MList, VHeap).
