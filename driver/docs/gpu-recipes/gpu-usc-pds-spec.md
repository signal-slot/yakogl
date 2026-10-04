# USC EOT + PDS + ISP + PBE — concrete artifacts for a TQ_3D FILL on BXS-4-64

Target: bare-metal Zephyr PowerVR UMD, **BXS-4-64, BVNC 36.53.104.796** (TI AM62P), RISC-V FW.
Job: TRANSFER / `TQ_3D` in **FILL** mode — GPU writes a solid RGBA8888 color to a linear buffer.

Scope of THIS document: the four compiled/packed blobs the kernel does not build for you —
(1) the USC "EOT emit" program, (2) the PDS pixel-event program, (3) the ISP control stream,
(4) the PBE state words + clear registers — plus (5) how they map into `rogue_fwif_cmd_transfer`.
It supplements `docs/research/gpu-render-spec.md` (submit path, heaps, CCCB/KCCB, fences).

Sources:
- `mesa-src/…` = Mesa `main`, `src/imagination/…` (fetched to scratchpad; see per-file paths).
- `pvr-src/…` = the ported kernel driver (local).
- Mesa raw files are behind the Anubis bot-wall; the working fetch route is the
  `https://r.jina.ai/` reader proxy in front of the gitlab raw URL (retry on the occasional
  bot-wall passthrough). **The PowerVR PDS code moved**: it is now under
  `src/imagination/vulkan/pds/` (the old `src/imagination/pds/` 404s).

---

## 0. TL;DR of the three answers the task asks for

**(A) Can the USC EOT be hand-assembled reliably, or must it be captured?**
The encoding is 100% open (pco_isa.py / pco_ops.py / pco_map.py / pco_binary.c), and for 1 RT the
program is a *single* instruction group: `emitpix.freep.end` reading shared regs `sh[0],sh[1]`.
I can pin the op byte (`0xC2`) and header bytes 1–2 (`0xA0 0x80`) exactly. **BUT** the group's
first header byte (`da`+`length`) and the variable-length source/ISS bytes are produced by pco's
runtime instruction-group *length allocator* and *source packer* (`pco_srcs_map_encode`,
`pco_iss_map_encode`, the igrp length fields), which are non-trivial to reproduce by hand — and one
wrong bit faults the GPU. **Verdict: CAPTURE is the reliable route** (build a ~60-line host tool that
calls `pvr_usc_eot()` for this BVNC and dumps `pco_shader_binary_data()`; see §1.4 and
`scratchpad/capture_eot_usc.c`). Hand-assembly is documented in §1.3 as a best-effort recipe, not a
guaranteed blob.

**(B) Are the PDS + ISP generators portable to standalone C?**
- **PDS pixel-event: YES, fully.** Extracted, deps-free, byte-exact against Mesa:
  `scratchpad/pvr_pds_pixel_event_gen.{c,h}` (verified by `scratchpad/selftest.c`). For 1 RT the
  **code segment is the constant pair `{0xF0040101, 0xF4000002}`**; only the data segment varies.
- **ISP control stream: YES for a fixed W×H** — it is small register-packing (csbgen), see §3 and
  `scratchpad/pvr_isp_ctrl_stream_gen.c`.

**(C) Ordered list of concrete byte-blobs/values you still must produce** — see §6.

---

## 1. USC EOT program (the crux)

### 1.1 What the program actually is (for 1 RT)
`pvr_usc_eot()` (`mesa-src/pvr_usc.c:87-181`) builds NIR. Driven by the props the transfer context
uses (`mesa-src/pvr_arch_job_context.c:1229-1270`, `pvr_transfer_eot_shaders_init`):

```c
rt_pbe_regs[i] = i * PVR_STATE_PBE_DWORDS;   /* PVR_STATE_PBE_DWORDS == 2 (pvr_common.h) */
props = { .emit_count = 1, .shared_words = true, .state_regs = rt_pbe_regs };
/* msaa_samples, num_output_regs, tile_buffer_addrs left 0 */
```

Because `shared_words == true` and `tile_buffer_addrs[0] == 0`, the loop body in `pvr_usc.c:99-171`
reduces to (for u=0):
```
state0 = nir_load_preamble(base = props->state_regs[0]     = 0)   // USC shared reg 0
state1 = nir_load_preamble(base = props->state_regs[0] + 1 = 1)   // USC shared reg 1
last_emit = nir_emitpix_pco(state0, state1)                        // pvr_usc.c:169
nir_intrinsic_set_freep(last_emit, true)                           // pvr_usc.c:173
nir_jump(return)                                                   // pvr_usc.c:176
```
`pco_data.fs.uses.olchk_skip = true` (`pvr_usc.c:179`).

`pco_end()` (`mesa-src/pco/pco_end.c:49-56`) sees the last instr (emitpix) *supports* the END
modifier (`O_EMITPIX = hw_op('emitpix', OM_ALU_RPT1 + [OM_FREEP], 0, 2)`, and
`OM_ALU_RPT1` contains `OM_END`, `mesa-src/pco/pco_ops.py:364,444`), so it sets `end=1` on the
emitpix itself — **no trailing `nop.end` is added**. So the compiled shader is exactly **one
instruction group**: `emitpix` with `freep=1`, `end=1`, `src0=sh[0]`, `src1=sh[1]`, `rpt=1`.

The two "state words" it emits are the PBE state pair; the PDS pixel-event program (§2) loads them
into USC shared regs `sh[0],sh[1]` (Common Store) just before DOUTU-launching this shader.

### 1.2 The emitpix instruction byte — EXACT (high confidence)
Encoding tables: `mesa-src/pco/pco_isa.py`.

Backend op enum `F_BACKEND_OP` (`pco_isa.py:1652-1663`): `emit = 0b110`.
Backend byte layout (`I_BACKEND` bit_set, `pco_isa.py:1859-1975`) + emitpix struct
(`I_EMITPIX`, `pco_isa.py:2359-2367`), byte 0 fields:

| bits | field | value |
|---|---|---|
| 7:5 | `backend_op` | `emit` = `0b110` |
| 4:2 | `rsvd0_emitpix` | 0 |
| 1   | `freep` | **1** (`nir_intrinsic_set_freep(...,true)`) |
| 0   | `rsvd0_emitpix_` | 0 |

⇒ **emitpix backend byte = `(0b110<<5) | (1<<1) = 0xC2`**. (With freep=0 it would be `0xC0`.)
`emitpix` occupies a single backend instruction byte; its two operands live in the group's
source/ISS bytes, not in the op byte.

### 1.3 The full instruction group — best-effort hand-encoding recipe (NOT a guaranteed blob)
Binary layout of one instruction group (`pco_encode_igrp`, `mesa-src/pco/pco_binary.c:63-105`):
```
[ hdr bytes ] [ instr bytes per phase, reversed ] [ lower_srcs ] [ upper_srcs ] [ iss ] [ dests ] [ pad ]
```
For emitpix: only the **backend** phase instr (1 byte, §1.2), **lower_srcs** (s0,s2 used; s1 empty),
an **iss** byte (internal source mux), **no dests** (`0` dests), plus header and word/align padding.

**Header** = `I_IGRP_HDR_MAIN` (from `group_map(O_EMITPIX, …)`, `mesa-src/pco/pco_map.py:3477-3493`):
```
oporg='be'(0b010), olchk=OM_OLCHK(=0, olchk_skip), w1p=0, w0p=0, cc=OM_EXEC_CND(=0 → e1_zx/always),
end=OM_END(=1), atom=0, rpt=1, alutype='main'(0b00), ext=1
```
Header bit positions (`I_IGRP_HDR`, `pco_isa.py:203-246`), 3 bytes (ext=1 ⇒ byte2 present):
- **byte1** = `ext(1)<<7 | oporg(0b010)<<4 | olchk(0)<<3 | w1p(0)<<2 | w0p(0)<<1 | cc1(0)` = **`0xA0`**
- **byte2** = `end(1)<<7 | alutype(0b00)<<5 | rsvd(0)<<4 | atom(0)<<3 | rpt(=1→enc 0b00)<<1 | ccext(0)` = **`0x80`**
- **byte0** = `da(4b)<<4 | length(4b)` — **NOT hand-derivable**: `length` is the total group length
  via `F_UINT4_POS_WRAP` (`pco_isa.py`: uint4, value 1..16, `encode = len%16`), and `da` is set by
  the pco group builder. Both depend on the final byte count (which itself depends on the source
  encoding below). This is the crux of why capture is safer.

**Sources**: `srcs=[('s[0]', SRC0, 's0'), ('s[2]', SRC1, 's2')]` (`pco_map.py:3489-3492`) — emitpix
src0→hw slot **s0**, src1→hw slot **s2**; both are USC **shared** registers (`F_REGBANK.shared =
0b100`, `pco_isa.py:331-341`), indices 0 and 1. The lower-source word encoding (`I_SRC` bit_set +
one of the `I_*_LO_*` variants, `pco_isa.py:359-593`) is chosen at runtime by pco's source packer
(`pco_srcs_map_encode`) based on how many sources and their bank widths — and the `iss`
(internal-source-selector / mux) byte comes from `pco_iss_map_encode`. Reproducing the exact variant
selection + bit packing by hand is the error-prone part.

**Conclusion for §1.3:** you can lock `0xC2` (op) and `0xA0 0x80` (header bytes 1–2), but the source
bytes, ISS byte, `da`/`length` and padding must come from the encoder. Do not ship a hand-guessed
blob to the GPU.

### 1.4 The reliable route — CAPTURE (recommended)
Build a tiny host tool that reproduces `pvr_transfer_eot_shaders_init` for rt_count=1 and dumps the
bytes. Full source: **`scratchpad/capture_eot_usc.c`**. Essentials:
```c
pco_ctx *ctx = pco_ctx_create(&pvr_device_info_36_53_104_796, &(struct pvr_device_runtime_info){0}, mem);
/* pco_ctx_setup_usclib(ctx, pco_usclib_data, pco_usclib_size);  // if your build requires it */
unsigned rt_pbe_regs[3] = {0,2,4};
struct pvr_eot_props props = { .emit_count=1, .shared_words=true, .state_regs=rt_pbe_regs };
pco_shader *eot = pvr_usc_eot(ctx, &props, &pvr_device_info_36_53_104_796);
fwrite(pco_shader_binary_data(eot), 1, pco_shader_binary_size(eot), stdout);   // the EOT bytes
unsigned temps = pco_shader_data(eot)->common.temps;                            // -> DOUTU temps (§2)
```
API: `mesa-src/pco/pco.h:35-62`; `pvr_usc_eot`: `mesa-src/pvr_usc.c:87`; feature table:
`mesa-src/bxs-4-64.h` (`pvr_device_info_36_53_104_796`).
Build against Mesa's PowerVR compiler lib (meson `-Dvulkan-drivers=imagination-experimental`), or,
with zero custom code, run any Vulkan clear/blit with **`PCO_DEBUG=binary`** and read the bytes
pco prints (`pco_binary.c:128-131` → `pco_print_binary`).

You need to capture this **once per BVNC** (color- and size-independent) for rt_count=1 (and
optionally 2,3). Record: the byte blob, its length, and `temps`.

---

## 2. PDS pixel-event program (portable C — DONE, byte-exact)

Generator: `pvr_pds_generate_pixel_event()` (`mesa-src/vulkan/pds/pvr_pds.c:534-624`); caller
`pvr_pbe_setup_emit()` (`mesa-src/pvr_arch_job_transfer.c:1152-1242`). No `_data_segment`/
`_code_segment` functions — those names are `gen_mode` macros (`pvr_pds.h:759-766`). **No HALT**: the
DOUTU carries END=1, which both kicks the USC and terminates the PDS program.

### 2.1 Inputs
| input | value / source |
|---|---|
| `num_emit_word_pairs` | `rt_count` = **1** (`transfer.c:1164`) |
| `emit_words[0..1]`    | the two **PBE state words** for the RT (`PVR_STATE_PBE_DWORDS==2`) — §4 |
| USC EOT code offset   | `usc_eot_bo.dev_addr − usc_heap.base_addr`, 4-byte aligned (`transfer.c:1176-1177`) |
| `usc_temps`           | `pco_shader_data(eot)->common.temps` (from §1.4 capture) |
| `sample_rate`         | `ROGUE_PDSINST_DOUTU_SAMPLE_RATE_INSTANCE = 0` (`transfer.c:1179-1183`) |

DOUTU `src0` build (`pvr_pds_setup_doutu` + `pvr_set_usc_execution_address64`, `pvr_pds.c:485-523`,
`pvr_pds.h:1247-1254`):
```
src0  = (exec_addr & 0xFFFFFFFC)                       // EXE_OFF, bits[31:2]
src0 |= ((DIV_ROUND_UP(usc_temps,4)) & 0x3F) << 35     // TEMPS (in 4-dword blocks)
src0 |= sample_rate << 33                               // 0
src0 |= dual_phase ? (1<<41) : 0                        // 0
```

### 2.2 Outputs for 1 RT (verified byte-exact by selftest)
- **DATA segment (5 dwords, 20 B):**
  `[ exec_addr&0xFFFFFFFC , (temps_blocks&0x3F)<<3 , emit_words[0] , emit_words[1] , DOUTW_ctrl ]`
  where `DOUTW_ctrl = 0x90000002` on BXS-4-64 (**slc_mcu_cache_controls = FALSE**, `bxs-4-64.h`).
  (It would be `0x94000002` if that feature were present.)
- **CODE segment (2 dwords, 8 B) — CONSTANT for 1 RT:** `{ 0xF0040101 , 0xF4000002 }`
  - `0xF0040101` = `DOUTW cc=0 end=0 src1=4 src0=1 dst=DOUTW(1)` — emit the PBE pair to Common Store.
  - `0xF4000002` = `DOUTU cc=0 end=1 src0=0 dst=DOUTU(2)` — kick the EOT USC + terminate PDS.
- Const layout (`pvr_pds_get_constants`, `pvr_pds.c:253-276`): control@0(64b), emit@2(64b),
  ctrlword@4(32b) ⇒ data_size=5.
- Upload: `pvr_arch_cmd_buffer_upload_pds(... data,data_size, EVENT_PIXEL_PDS_DATA_ADDR_ALIGNMENT,
  code,code_size, EVENT_PIXEL_PDS_CODE_ADDR_ALIGNMENT, ...)` (`transfer.c:1209-1218`).
  `event_pixel_pds_data/code` = returned offsets; `event_pixel_pds_info.{const_size,usc_sr_size}`
  per `transfer.c:1223-1239` (see §5).

### 2.3 PDS ISA encodings used (so you can port without Mesa)
All 32-bit. Only the **DOUT** opcode (opcode `0xF`, `DOUT_OPCODE_SHIFT=28`) with two DST selectors:
```
DOUT (pvr_rogue_pds_encode.h:514-536):
  [31:28] OPCODE = 0xF          [27] CC           [26] END
  [23:16] SRC1 (&0xFF, 32-bit const idx)          [14:8] SRC0 (&0x7F, 64-bit const idx)
  [2:0]   DST : DOUTW=1, DOUTU=2  (pvr_rogue_pds_defs.h:1564-1573)
DOUTW src1 control word (pvr_pds.c:155-178):
  bit31 LAST_EN(0x80000000) | bit28 DEST_COMMON_STORE(0x10000000) |
  bit26 CMODE_BYPASS(0x04000000, feature-gated) | [1:0] BSIZE(ALL64=0x2) | [..:13] AO(dest offset)
DOUTU src0: 64-bit value from §2.1.
HALT (reference, unused): 0xD3000000.
```
Full portable generator: **`scratchpad/pvr_pds_pixel_event_gen.{c,h}`** (public API
`pvr_pds_pe_build_doutu_src0()` and `pvr_pds_generate_pixel_event_tq()`), verified against Mesa.

---

## 3. ISP control stream (minimal FILL, 64×64)

BXS-4-64 has `simple_internal_parameter_format` + `_v2` = TRUE (`bxs-4-64.h`), so the **SIPF2** path is
used. csbgen names resolve to `src/imagination/csbgen/rogue/{ipf,ppp}.xml`. Builder:
`pvr_isp_ctrl_stream()` (`mesa-src/pvr_arch_job_transfer.c:4045`), aligned writer
`pvr_isp_ctrl_stream_sipf_write_aligned()` (`:4018`), size `pvr_isp_primitive_block_size()` (`:3795`).

**Total = 216 bytes** = a **128-byte region array** (control stream; only 7 bytes meaningful) followed
by an **88-byte / 22-dword primitive block**. Allocated in the TRANSFER_FRAG heap.
Portable generator: **`scratchpad/pvr_isp_ctrl_stream_gen.c`** (compiles; selftest matches the bytes
below). Inputs: W,H, the heap-relative BO base, and the NOP-PDS offsets.

### 3.1 Region array (bytes 0–6; rest padding to 128)
| byte | value | field (SIPF2) | meaning | cite |
|---|---|---|---|---|
| 0 | `0x16` | `IPF_PRIMITIVE_HEADER_SIPF2` | prim_base_size=1, mask_num_bytes=1, valid_tile0=1 | `:4443-4447`, ipf.xml:154-162 |
| 1–4 | `(prim_blk_addr>>3)&0xFFFFFFFF` LE | `IPF_PRIMITIVE_BASE_SIPF2` (shift 3) | 8-byte-granular ptr to prim block | `:4451-4453`, ipf.xml:181-183 |
| 5 | `0x03` | `IPF_BYTE_BASED_MASK_ONE_BYTE_WORD_0_SIPF2` | tile0_0 \| tile0_1 (1 mapping) | `:4460-4487`, ipf.xml:188-221 |
| 6 | `0x03` | `IPF_CONTROL_STREAM_TERMINATE_SIPF2` | cs_ctrl_type = TERM(3) | `:4544`, ipf.xml:148-151 |

`prim_blk_addr = stream_base_vaddr + 128` (cs_start_offset=0 for a fill, `:3947`). `stream_base_vaddr`
is the **TRANSFER_FRAG-heap-relative** base of this allocation — the same value used for
`isp_mtile_base` / `CR_ISP_MTILE_BASE` (`:4557-4561`).

### 3.2 Primitive block (22 dwords, at byte offset 128). Order: ISP-state → index → vertices → PDS-state.
```
dw0  0x0000004C  \ VERTEX_FORMAT_WORD_SIPF2 (64b): vertex_total=3, primitive_total=1,
dw1  0x00000160  /   isp_vtx_raw=1, tsp_vtx_raw=1, isp_state_size=2         (:3440-3452, ipf.xml:108-120)
dw2  0x00000000    TA_STATE_ISPCTL (empty)                                   (:3459, ppp.xml:131-143)
dw3  0x002E0000    TA_STATE_ISPA: objtype=TRIANGLE, passtype=OPAQUE,
                     dwritedisable=1, dcmpmode=ALWAYS                        (:3463-3469, ppp.xml:145-160)
dw4  0x04430820  \ IPF_INDEX_DATA_WORDS_SIPF (64b): tris (0,1,2)(3,2,1)(4,5,6)(7,6,5)
dw5  0x14C718A4  /                                                           (:3519-3543, ipf.xml:76-105)
dw6  0x00008000  \ ISP_VERTEX_WORD_SIPF TL  x=(0+2048)<<4=0x8000, y=0x8000    (:3703-3728, ipf.xml:474-478)
dw7  0x00000001  /
dw8  0x00008400  \ TR  x=(W+2048)<<4, y=top
dw9  0x00000001  /
dw10 0x08008000  \ BL  x=left,  y=(H+2048)<<4     [see note]
dw11 0x00000001  /
dw12 0x08008400  \ BR  x=right, y=bottom
dw13 0x00000001  /
dw14 <nop_pds_data_offset>>4>   TA_STATE_PDS_SHADERBASE  (NOP PDS)           (:3357-3359, :4208-4209)
dw15 0x00000000                 TA_STATE_PDS_TEXUNICODEBASE
dw16 <pds_sizeinfo1>            TA_STATE_PDS_SIZEINFO1 (small, from unitex)   (:3367-3389)
dw17 <pds_varyingbase>          TA_STATE_PDS_VARYINGBASE (0 for a fill)       (:3392-3394)
dw18 0x00000000                 TA_STATE_PDS_TEXTUREDATABASE
dw19 0x00000000                 TA_STATE_PDS_UNIFORMDATABASE
dw20 0x00000000                 TA_STATE_PDS_SIZEINFO2                        (:3408-3416)
dw21 0x00000000                 padding (SIPF 64-bit realign)                (:3419-3420)
```
Vertex XY encoding (SIPF): `packed = (coord + 2048) << 4`; word = `z(0)<<34 | y<<17 | x`
(`pvr_int32_to_isp_xy_vtx` `:3601-3634`; `bias=true` for ≤256 tiles, `:3662-3663`).
**Note (correction):** for BL/BR the 64-bit vertex values are `0x1_0800_8000` / `0x1_0800_8400`, which
split little-endian to **lo=`0x08008000`/`0x08008400`, hi=`0x00000001`** (verified by the generator's
selftest). `y=(64+2048)<<4 = 0x8400`, and `0x8400<<17 = 0x1_0800_0000`.

### 3.3 W/H dependence and runtime fields
- **W/H-dependent:** only the 4 vertex words — `right = (W+2048)<<4`, `bottom = (H+2048)<<4`
  (`left=top=(0+2048)<<4=0x8000`). For any full-surface fill, left/top are fixed; only right/bottom
  change. (`isp_mtile_size` = width_in_tiles×height_in_tiles is a *register*, not in this stream:
  tile=16×16 ⇒ 64×64 = 4×4.)
- **Runtime (allocator/program) fields:** `PRIMITIVE_BASE_SIPF2` = `(base+128)>>3`;
  `PDS_SHADERBASE` = `nop_program.pds.data_offset>>4`; `PDS_SIZEINFO1`; `PDS_VARYINGBASE` (0 for fill).
- **Everything else is constant** for a single-quad fill.

### 3.4 Associated CR registers set by the builder (go into the transfer stream, §5)
`CR_ISP_MTILE_BASE` = region-array base (heap-relative); `CR_ISP_RENDER.mode_type = FAST_2D`;
`CR_ISP_RGN` (SIPF variant; `cs_size_ipf_creq_pf = was_linked = false` here) (`:4557-4577`).

---

## 4. PBE state words + clear registers

Packing: `pvr_arch_pbe_pack_state()` (`mesa-src/pvr_arch_job_common.c:82`). It emits
`ROGUE_NUM_PBESTATE_STATE_WORDS = 2` **state** words (DMA'd to the PBE by the EOT pixel-event program,
= the `emit_words` of §2) and `ROGUE_NUM_PBESTATE_REG_WORDS = 3` **reg** words (the
`pbe_wordx_mrty[]` registers, §5). Surface params: `pvr_pbe_setup_codegen_defaults()` (`:790`) +
`pvr_pbe_setup_swizzle()` (`:1010`, 8888-UNORM force `:1064-1069`).

Surface: `VK_FORMAT_R8G8B8A8_UNORM`, LINEAR, identity swizzle, `packmode = U8U8U8U8 = 0x0`
(pbestate.xml:60), `source_format = 8_PER_CHANNEL = 1` (pbestate.xml:155), `norm = 1`,
`mem_layout = LINEAR = 0` (pbestate.xml:53). Transfer stride is a **pixel** stride (= 64 for a
tightly-packed 64-wide RGBA8888 image).

### 4.1 STATE words (the EOT `emit_words[0..1]` = "pbe_word0/word1")
- `PBESTATE_STATE_WORD0` (pbestate.xml:226): **`address_low = (addr>>2)&0xFFFFFFFF`**.
- `PBESTATE_STATE_WORD1` (pbestate.xml:230-243): `source_format(1)<<19 | norm(1)<<13 | packmode(0)<<7 |
  address_high((addr>>34)&0x3F)`.

### 4.2 REG words (`pbe_wordx_mrty[0..2]`)
- `PBESTATE_REG_WORD0` (pbestate.xml:245): `tilerelative(1)<<57 | linestride((stride-1)/2)<<34 |
  swizzle(identity: chan0..3 = 0,1,2,3 at [22:20],[25:23],[28:26],[31:29]) | memlayout(LINEAR=0) |
  minclip_x(0)`.
- `PBESTATE_REG_WORD1` (pbestate.xml:275): `maxclip_x(W-1)<<32 | maxclip_y(H-1)` (LINEAR ⇒ size_x/y=0,
  minclip_y=0, zslice=0).
- `PBESTATE_REG_WORD2` = **0** (pair_tiles off, `sw_bytemask=0` for a plain fill; `:1119-1150`).
  Note ERN 42064 is **absent** on this BVNC, so `pbe_regs[2]` is not force-zeroed elsewhere; pack_state
  sets it (=0 here).

### 4.3 Concrete values — 64×64 red fill at dev-vaddr **0x80_1000_0000** (stride 64 px)
Address: `0x8010000000>>2 = 0x2004000000` ⇒ low32 `0x04000000`; `0x8010000000>>34 = 0x20`.

| item | value |
|---|---|
| **pbe_word0** (STATE_WORD0 = `emit_words[0]`) | **`0x04000000`** |
| **pbe_word1** (STATE_WORD1 = `emit_words[1]`) | **`0x00082020`** (=addr_hi `0x20` \| norm `0x2000` \| src_fmt `0x80000`) |
| `pbe_wordx_mrty[0]` (REG_WORD0) | **`0x0200_007C_6880_0000`** (tilerel + linestride `31=0x1F` + swizzles) |
| `pbe_wordx_mrty[1]` (REG_WORD1) | **`0x0000_003F_0000_003F`** (maxclip_x=63, maxclip_y=63) |
| `pbe_wordx_mrty[2]` (REG_WORD2) | **`0x0`** |

(If the driver is handed a *byte* stride 256 instead of pixel stride 64, `linestride=(256-1)/2=0x7F`
and REG_WORD0=`0x0201_FC00_6880_0000`. Mesa uses a **pixel** stride, so `0x1F` is correct.)

### 4.4 Clear color — `pvr_pack_clear_color()` (`:2372`, U8U8U8U8 branch `:2390-2402`)
Packing order is **`0xAABBGGRR`**: `pkd[0] = R | (G<<8) | (B<<16) | (A<<24)`, with each channel
`pvr_float_to_ufixed(chan,8)`. For opaque red (float R=1,G=0,B=0,A=1 → `0xFF,0,0,0xFF`):
```
usc_clear_register0 = 0xFF | (0x00<<8) | (0x00<<16) | (0xFF<<24) = 0xFF0000FF
usc_clear_register1 = usc_clear_register2 = usc_clear_register3 = 0
```
Written to `regs->usc_clear_register0..3` (`:4184-4206`). The EOT USC moves these to the PBE output;
the PBE packs them per §4.1/§4.2. (Numerically `0xFF0000FF` equals the RGBA literal here, but the byte
order is AABBGGRR — e.g. opaque green `0x00FF00FF`→ `0xFF00FF00`.)

---

## 5. `rogue_fwif_cmd_transfer` assembly (field map)

Kernel forwards a user "stream" through `pvr_stream_process()` using the transfer stream def; it does
not understand register semantics (`pvr-src/pvr_job.c:87-102`). Stream order
(`pvr-src/pvr_stream_defs.c:234-259`) and the target struct
(`pvr-src/pvr_rogue_fwif_client.h:317-365`, `rogue_fwif_transfer_regs` / `rogue_fwif_cmd_transfer`):

**64-bit section first**, then 32-bit:
```
pds_bgnd0_base(64), pds_bgnd1_base(64), pds_bgnd3_sizeinfo(64), isp_mtile_base(64),
pbe_wordx_mrty[9](64 each  = 3 RT × ROGUE_PBE_WORDS_REQUIRED_FOR_TQS(3)),
isp_bgobjvals(32), usc_pixel_output_ctrl(32), usc_clear_register0..3(32×4),
isp_mtile_size(32), isp_render_origin(32), isp_ctl(32), isp_aa(32),
event_pixel_pds_info(32), event_pixel_pds_code(32), event_pixel_pds_data(32),
isp_render(32), isp_rgn(32),
frag_screen(32)   // GPU_MULTICORE_SUPPORT = TRUE on BXS-4-64 → PRESENT
// isp_xtp_pipe_enable is ABSENT (S7_TOP_INFRASTRUCTURE = FALSE on BXS-4-64)
```
`common.frame_num` and `flags` are set by the kernel, not in the stream (`pvr-src/pvr_job.c:271-272`).
`dest_size = sizeof(struct rogue_fwif_cmd_transfer)` (`pvr_stream_defs.c:267`).

**Which blob feeds which field:**

| stream field | source / value (this spec) |
|---|---|
| `pds_bgnd0_base`, `pds_bgnd1_base`, `pds_bgnd3_sizeinfo` | **0** (null background PDS for a fill) |
| `isp_mtile_base` (64) | (ISP-control-stream vaddr) − TRANSFER_FRAG_HEAP base — §3 / gpu-render-spec §3.2 |
| `pbe_wordx_mrty[0..2]` (RT0) | the 3 PBE **reg** words from §4 (`pvr_arch_pbe_pack_state`); [3..8]=0 |
| `isp_bgobjvals` | **0** (no background enabled for a fill) — §3/§4 |
| `usc_pixel_output_ctrl` | from tiles-in-flight setup — §3 |
| `usc_clear_register0..3` | packed clear color (§4); reg0 holds RGBA8888, reg1..3 = 0 |
| `isp_mtile_size` | width_in_tiles / height_in_tiles (tile=16×16 → 64×64 ⇒ 4×4) — §3 |
| `isp_render_origin` | {0,0} |
| `isp_ctl` | tiles-in-flight + `process_empty_tiles=1` — §3 |
| `isp_aa` | aa_mode(samples=1) = 0 |
| `event_pixel_pds_info` | `{temp_stride=0, const_size=DIV_ROUND_UP(data_size,unit), usc_sr_size=DIV_ROUND_UP(rt_count*2,unit)}` (§2, `transfer.c:1223-1231`) |
| `event_pixel_pds_code` | PDS **code** offset in PDSCODEDATA heap (§2) |
| `event_pixel_pds_data` | PDS **data** offset in PDSCODEDATA heap (§2) |
| `isp_render` | mode_type = `ISP_RENDER_MODE_TYPE_FAST_2D` — §3 |
| `isp_rgn` | region-stream size / link flag — §3 |
| `frag_screen` | `{xmax=W-1, ymax=H-1}` (present: multicore) |

FW command header: `cmd_type = TQ_3D = 0x2ABC80CA` (see gpu-render-spec §1). Kick via one KCCB
`KICK` + MTS (gpu-render-spec §2.3).

---

## 6. Concrete byte-blobs / values you still must PRODUCE (ordered)

1. **USC EOT bytes** — CAPTURE once for BVNC 36.53.104.796, rt_count=1 (§1.4). Record blob + length
   + `temps`. (rt_count 2,3 optional; a single-RT fill needs only 1.)
2. **PDS pixel-event program** — generate with `scratchpad/pvr_pds_pixel_event_gen.c`:
   - code = `{0xF0040101, 0xF4000002}` (constant),
   - data = `{eot_off&~3, (ceil(temps/4)&0x3F)<<3, pbe_state0, pbe_state1, 0x90000002}`.
   Upload to PDSCODEDATA heap; record code_offset, data_offset, data_size(=5 dw).
3. **PBE reg words `pbe_wordx_mrty[0..2]`** and **PBE state words `emit_words[0..1]`** — §4, from the
   dest surface (base vaddr, stride=W×4, fmt 8888 UNORM, LINEAR, extent W×H).
4. **`usc_clear_register0`** = packed RGBA8888 clear color (§4); reg1..3 = 0.
5. **ISP control stream** (one full-screen prim block + TERM) — generate for fixed W×H (§3); upload to
   TRANSFER_FRAG heap; compute `isp_mtile_base` offset, `isp_ctl`, `isp_mtile_size`,
   `usc_pixel_output_ctrl`, `isp_render`, `isp_rgn`, `frag_screen`.
6. **Assemble the transfer stream** in the exact order of §5 and submit (gpu-render-spec §2–§4).
