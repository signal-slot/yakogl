<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Multi-draw and stencil on BXS-4-64 (BVNC 36.53.104.796)

> **Verified on silicon (2026-09-14):** yakogl drives ISPA.sref / ISPB (compare, ops,
> masks) and ISPCTL.tagwritedisable per draw from GL stencil state and glColorMask;
> `driver/tools/pvrlab/gl_test.c` case 9 passes mask/test (EQUAL, NOTEQUAL), INCR x2, the
> glClearStencil background value and the stencil write mask. The mask pass (no colour,
> TRANSLUCENT pass type as every object) is not discarded; no PUNCH_THROUGH was needed.
> Two-sided stencil (case 12): header `pres_ispctl_ba`/`pres_ispctl_bb` (bits 3/4),
> `ISPCTL.two_sided` (bit 19) and a second ISPA/ISPB pair after the front pair; the
> front pair applies to the TA's clockwise triangles (the winding `CULLMODE_CW` culls),
> so with GL's default `glFrontFace(GL_CCW)` on the y-flipped window surface the GL
> front state goes into the back pair, and the other way round on an FBO.

READ-ONLY analysis. Sources cited by `file:line`:

- Mesa VDM/PPP field layout: Mesa 26.2.1 `src/imagination/csbgen/rogue/vdm.xml`, `.../ppp.xml`
- Mesa Vulkan cmd-buffer emit: Mesa 26.2.1 `src/imagination/vulkan/pvr_arch_cmd_buffer.c`, `.../pvr_arch_job_render.c`, `.../pvr_csb_enum_helpers.h`
- Our builder: `driver/src/pvr_vdm_ppp_gen.c`

The single most important structural fact, established from Mesa:

> **State (PPP + VDM) is a *latched* stream state. A draw is *only* an INDEX_LIST block.
> The STREAM_TERMINATE is emitted once for the whole render, not per draw.**

Mesa proves this cleanly:
- `pvr_emit_vdm_index_list()` (`pvr_arch_cmd_buffer.c:8655-8754`) emits **only** `VDMCTRL_INDEX_LIST0..N` — no PPP, no VDM_STATE, no terminate.
- State is emitted **only when dirty**: `pvr_emit_dirty_ppp_state()` early-returns unless `pvr_ppp_state_update_required()` (`:7972`, predicate `:7920-7952`); `pvr_emit_ppp_state()` writes one `VDMCTRL_PPP_STATE0/1` pair (`:7846-7853`).
- Terminate is emitted once at sub-command end: `pvr_arch_csb_emit_terminate()` (`:2470`).

So consecutive `vkCmdDraw` with identical pipeline/state produce: **one** state block, then **N back-to-back INDEX_LIST blocks**, then **one** terminate. A state change between draws inserts a fresh PPP_STATE_UPDATE (and/or VDM_STATE_UPDATE) before the next INDEX_LIST.

Our trailing "2nd PPP_STATE_UPDATE (3 words @ ppp+0x4c)" is **not** a per-draw item. It is a
`TA_STATE_HEADER` with `pres_terminate` (bit25) set followed by `TA_STATE_TERMINATE0/1` (tile clip
bounds): the **PPP-state-stream terminator for the whole render** (`ppp.xml:104-107` header,
`:236-253` STATE_TERMINATE0/1). It goes
**once, at the very end, after the last INDEX_LIST, before VDMCTRL_STREAM_TERMINATE** — for both the
same-state and state-change cases.

---

## (a) Exact VDM stream layout

### Field references (vdm.xml)
- `INDEX_LIST0` (`vdm.xml:204-220`): block_type[29:31]=INDEX_LIST(3); index_addr_present[28];
  index_count_present[27]; index_instance_count_present[26]; index_offset_present[25];
  start_present[24]; index_size[17:18] (B8=0/B16=1/B32=2); primitive_topology[8:11];
  index_base_addrmsb[0:7] (addr>>32).
- `INDEX_LIST1` (`:222-224`): index_base_addrlsb[0:31] (VA low 32 bits, 4-byte aligned).
- `INDEX_LIST2` (`:226-228`): index_count[0:31].
- `INDEX_LIST3` (`:230-232`): instance_count (= instanceCount-1; only if bit26).
- `INDEX_LIST4` (`:234-236`): index_offset[0:31] (base-vertex add; only if bit25).
- A minimal indexed draw is therefore the 3-word block LIST0/LIST1/LIST2 exactly as our
  `pvr_vdm_build_clear_stream` already emits (`pvr_vdm_ppp_gen.c:420-425`).

### Layout 1 — N draws, SAME state (one kick)

```
VDMCTRL_PPP_STATE0            ; block_type=0, word_count=<ppp main dwords, e.g. 19>, addrmsb(ppp)
VDMCTRL_PPP_STATE1            ; addrlsb(ppp)
VDMCTRL_VDM_STATE0            ; block_type=2, vs_data_addr_present, vs_other_present, cam_size, ...
VDMCTRL_VDM_STATE2            ; vs_pds_data_base
VDMCTRL_VDM_STATE3            ; vs_pds_code_base
VDMCTRL_VDM_STATE4            ; vs_output_size
VDMCTRL_VDM_STATE5            ; vs_max_instances, sizes
                             ; ---- draw 0 ----
VDMCTRL_INDEX_LIST0          ; INDEX_LIST(3)|addr_present|count_present|B16|topology|base0_msb
VDMCTRL_INDEX_LIST1          ; base0_lsb
VDMCTRL_INDEX_LIST2          ; count0
                             ; ---- draw 1 ----
VDMCTRL_INDEX_LIST0          ; base1_msb  (new header per draw)
VDMCTRL_INDEX_LIST1          ; base1_lsb
VDMCTRL_INDEX_LIST2          ; count1
   ... (repeat per draw) ...
VDMCTRL_PPP_STATE0           ; 3-word trailing PPP: word_count=3, addr = ppp_base + 0x4c
VDMCTRL_PPP_STATE1           ; addrlsb(ppp_base + 0x4c)   -> STATE_HEADER(pres_terminate)+TERMINATE0/1
VDMCTRL_STREAM_TERMINATE     ; block_type=6
```

Rules:
- **Each draw is its own full INDEX_LIST0/1/2 block** — the header is re-emitted per draw (Mesa
  re-packs `list_hdr` and emits it every draw; `pvr_arch_cmd_buffer.c:8725-8739`). You cannot share
  one header across draws.
- VDM_STATE and PPP_STATE are **not** repeated between same-state draws.
- The trailing 3-word PPP (STATE_TERMINATE) and STREAM_TERMINATE appear **once**, at the end.

Selecting different geometry per draw with a shared sequential index buffer `{0,1,2,3,...}`:
- Concatenate all draws' vertices into one vertex buffer (draw0 verts, then draw1 verts, ...).
  Then draw *k*'s INDEX_LIST1 = `idx_va + (first_index_k * 2)` and INDEX_LIST2 = `count_k`. Because
  the sequential index values at that offset are `{first_k, first_k+1, ...}`, they naturally index
  the concatenated vertex buffer. (The vertex-fetch PDS in VDM_STATE2/3 is unchanged.)
- Equivalent alternative: keep INDEX_LIST1 fixed and set `index_offset_present` (bit25) +
  INDEX_LIST4 = base-vertex add. Prefer the base-address bump; it needs no extra word.

### Layout 2 — 2 draws with a STATE CHANGE between (the stencil 2-pass)

Only the PPP (ISP stencil) changes; the vertex program is the same, so VDM_STATE is emitted once.

```
VDMCTRL_PPP_STATE0/1         ; -> PPP block A (pass-1 ISPA/ISPB/ISPCTL), word_count = A main dwords
VDMCTRL_VDM_STATE0,2,3,4,5   ; vertex program (shared by both passes)
                             ; ---- pass 1 (mask) ----
VDMCTRL_INDEX_LIST0/1/2      ; mask geometry: base=idx_mask, count=n_mask
                             ; ---- state change ----
VDMCTRL_PPP_STATE0/1         ; -> PPP block B (pass-2 ISPA/ISPB/ISPCTL), word_count = B main dwords
                             ; ---- pass 2 (test) ----
VDMCTRL_INDEX_LIST0/1/2      ; full quad: base=idx_quad, count=n_quad
                             ; ---- terminate ----
VDMCTRL_PPP_STATE0/1         ; 3-word trailing STATE_TERMINATE (once), addr = <ppp>+0x4c
VDMCTRL_STREAM_TERMINATE
```

- The second `VDMCTRL_PPP_STATE_UPDATE` re-latches ISP state for all subsequent INDEX_LISTs. This is
  exactly what Mesa emits when stencil dynamic state changes between two draws
  (`pvr_ppp_dynamic_state_isp_faces_and_control_dirty`, `pvr_arch_cmd_buffer.c:7901-7918` →
  re-runs `pvr_setup_isp_faces_and_control` → re-emits PPP).
- PPP blocks A and B are two separate uploaded blocks (two BOs / two offsets). If you want the single
  trailing STATE_TERMINATE, it can live at `A_base+0x4c` or `B_base+0x4c` (its own small block); it
  is state-independent (just clip bounds), so either is fine.
- If a pass needs a *different vertex program* (it does not for mask/test — both are position-only
  screen-space triangles), you would additionally re-emit VDM_STATE0/2/3/4/5 before that pass.

---

## (b) ISPA / ISPB stencil field values for the 2-pass mask/test

### Field layout (ppp.xml) and enums
`TA_STATE_ISPA` (`ppp.xml:145-160`): objtype[28:31], passtype[24:26], dwritedisable[21],
dfbztestenable[20], dcmpmode[17:19], pointlinewidth[8:15], **sref[0:7]**.

`TA_STATE_ISPB` (`ppp.xml:162-169`): **scmpmode[25:27]**, **sop1[22:24]**, **sop2[19:21]**,
**sop3[16:18]**, **scmpmask[8:15]**, **swmask[0:7]**.

`TA_STATE_ISPCTL` (`ppp.xml:131-143`): upass[22:25], **tagwritedisable[21]**, two_sided[19],
bpres[18], dbenable[17], scenable[16].

Mesa mapping (authoritative, `pvr_arch_cmd_buffer.c:6786-6823`, casts in `pvr_csb_enum_helpers.h:158-168`):
- `ISPA.sref = stencil.reference`
- `ISPB.sop1 = op.fail` (stencil-fail op)
- `ISPB.sop2 = op.depth_fail` (z-fail op)
- `ISPB.sop3 = op.pass` (depth+stencil-pass op)
- `ISPB.scmpmode = op.compare` (stencil compare func)
- `ISPB.scmpmask = compare_mask`, `ISPB.swmask = write_mask` (write_mask only if `stencil.write_enable`)

Enums are **direct Vulkan casts** (identical numbering):
- CMPMODE (`ppp.xml:34-43`): NEVER0 LESS1 EQUAL2 LEQUAL3 GREATER4 NOTEQUAL5 GEQUAL6 ALWAYS7
- ISPB_STENCILOP (`ppp.xml:63-72`): KEEP0 ZERO1 REPLACE2 INCR_SAT3 DECR_SAT4 INVERT5 INCR_WRAP6 DECR_WRAP7

### Concrete values

Assume 8-bit stencil, mask/compare masks = 0xFF, reference = 1, on-chip background stencil = 0.

**Pass 1 — mask (write stencil=1, no color, stencil test ALWAYS):**

| word | fields | value |
|------|--------|-------|
| ISPA | objtype=TRIANGLE(0), passtype=OPAQUE(0), dwritedisable=1[21], dcmpmode=ALWAYS(7)[17], sref=1[0] | `0x002E0001` |
| ISPB | scmpmode=ALWAYS(7)[25], sop1=KEEP(0), sop2=KEEP(0), sop3=REPLACE(2)[16], scmpmask=0xFF[8], swmask=0xFF[0] | `0x0E02FFFF` |
| ISPCTL | bpres=1[18], **tagwritedisable=1[21]** (no color), upass=0 | `0x00240000` |

Derivation: ISPA = 0x00200000(dwritedisable) | 0x000E0000(dcmpmode7) | 0x00000001(sref) = 0x002E0001.
ISPB = 0x0E000000(scmpmode7) | 0x00020000(sop3=REPLACE) | 0x0000FF00(scmpmask) | 0x000000FF(swmask) = 0x0E02FFFF.

**Pass 2 — test (draw quad, stencil EQUAL 1, keep stencil, write color):**

| word | fields | value |
|------|--------|-------|
| ISPA | objtype=TRIANGLE(0), passtype=OPAQUE(0), dwritedisable=1[21], dcmpmode=ALWAYS(7)[17], sref=1[0] | `0x002E0001` |
| ISPB | scmpmode=EQUAL(2)[25], sop1/2/3=KEEP(0), scmpmask=0xFF[8], swmask=0x00[0] | `0x0400FF00` |
| ISPCTL | bpres=1[18], **tagwritedisable=0[21]** (write color), upass=0 | `0x00040000` |

Derivation: ISPB = 0x04000000(scmpmode2=EQUAL) | 0x0000FF00(scmpmask) | 0(all KEEP, swmask 0) = 0x0400FF00.
Hardware compares `(sref & scmpmask) EQUAL (stored & scmpmask)` → `1==1` only where pass 1 wrote 1.

Notes:
- `TA_STATE_HEADER` for both blocks keeps `pres_ispctl`, `pres_ispctl_fa`, `pres_ispctl_fb`
  (`ppp.xml:126-128`; bits 0,1,2) set so ISPCTL, ISPA (FA) and ISPB (FB) are all emitted — our
  builder already sets bits 0/1/2 (`pvr_vdm_ppp_gen.c:160`). No new header bit is needed to "enable
  stencil": stencil is enabled purely by the ISPB content being present and non-default.
- Single-face only: no back-face words (two_sided=0), so `pres_ispctl_ba/bb` stay 0.

---

## (c) On-chip stencil setup / (Q4) enables

- **There is no separate stencil-enable bit in the PPP/ISP stream or in the frag command.** Stencil
  test+write is driven entirely by the per-object ISPB (compare mode + ops + masks) plus ISPA.sref.
  (`ispctl.scenable`[16] is *scissor* enable, set unconditionally when rasterizing — Mesa `:6883` —
  **not** stencil; our builder leaves it 0 and that is fine.)
- **On-chip stencil is always available** (it shares the ISP on-chip depth/stencil tile buffer, like
  depth is always available for HSR). No stencil attachment and no ZLS are required for a self-contained
  mask→test within one TA+3D kick.
- **Background (initial) stencil value** lives in the 3D/render register block, not the VDM stream:
  `CR_ISP_BGOBJVALS.stencil = ds_clear_value.stencil & 0xFF` (`pvr_arch_job_render.c:1125-1131`).
  Keep it 0 for this recipe (background stencil = 0) so pass 2's EQUAL-1 matches only the mask region.
- `CR_ISP_STENCIL_LOAD_BASE` / `sloaden` / `sstoreen` (`pvr_arch_job_render.c:864,998,1015-1027`) and
  `CR_ISP_ZLSCTL` (`:958-1007`) are only for loading/storing the stencil buffer **to system memory**
  across renders. Leave them disabled (`has_stencil_attachment=false`) for the on-chip 2-pass.
  **Verified on silicon (2026-09-14, gl_test case 13):** yakogl now uses them to keep depth and
  stencil across kicks. Packed D24S8 (`ZLS_FORMAT_TYPE 24BITINT`, stencil in the top byte), buffer
  row-linear with the width rounded up to 16 px tiles, `zlsextent = tiles - 1`, both bases the same
  address with the separate-stencil enable bit clear, `RGXFWIF_CMD3D.flags = DEPTHBUFFER |
  STENCILBUFFER` (field positions: `coeff_base_analysis.md` section 1 and the vendored
  `RGXFWIF_CMD3D`). With this format
  `ISP_BGOBJDEPTH` is a 24-bit unorm. The store writes 0 (not the background depth) for pixels no
  object covered, so a kick that clears depth/stencil starts with a full-screen object that
  writes the clear values (colour write disabled, depth ALWAYS + write, stencil REPLACE), as
  Mesa's start-of-render attachment clears do.

---

## Which of our structs change

`pvr_vdm_ppp_gen.h` / `.c` (`pvr_vdm_ppp_gen.c`):

1. **`struct pvr_ppp_frag_inputs`** — add stencil params so `pvr_ppp_build_clear_block` stops
   hardcoding ISPA/ISPB:
   - `uint8_t stencil_ref;`   → ISPA.sref (currently forced 0, `:195-196`)
   - `uint8_t scmpmode;` (CMPMODE) → ISPB[25] (currently forced ALWAYS, `:204`)
   - `uint8_t sop_fail, sop_zfail, sop_pass;` (ISPB_STENCILOP) → ISPB[22]/[19]/[16] (currently forced KEEP, `:205-207`)
   - `uint8_t scmpmask, swmask;` → ISPB[8]/[0] (currently 0)
   - `int color_write_disable;` → ISPCTL.tagwritedisable[21] (currently `!has_color`, `:165`)

2. **`pvr_ppp_build_clear_block()`** (`:141`) — emit ISPA.sref, the parameterized ISPB, and
   ISPCTL.tagwritedisable from the new inputs. Nothing else in the 19-dword block changes; build it
   **twice** (block A = pass-1 values, block B = pass-2 values) into two offsets.

3. **VDM builder** — the current `pvr_vdm_build_clear_stream()` (`:347`) hardcodes one PPP block +
   one INDEX_LIST + trailing PPP + terminate. Generalize to accept an **array of draws** and an
   **array of state blocks**:
   - `struct pvr_vdm_draw { uint64_t index_buf_dev_addr; uint32_t index_count; uint32_t ppp_state_dev_addr; uint32_t ppp_word_count; /*0 = reuse previous PPP*/ };`
   - Emit VDM_STATE0/2/3/4/5 once; then for each draw emit a PPP_STATE0/1 pair *only when its
     ppp_state differs from the previous draw's*, followed by that draw's INDEX_LIST0/1/2; finally
     emit the single trailing 3-word PPP_STATE + STREAM_TERMINATE.
   - The existing single-draw call becomes the 1-draw, no-state-change case (byte-identical output),
     so the proven path is preserved.

4. Caller (upload side) — upload **two** PPP blocks for the 2-pass, plus a mask vertex set and a quad
   vertex set concatenated in one vertex buffer with a sequential u16 index buffer long enough to
   cover both (index range picked per draw via INDEX_LIST1 base offset). Set the 3D command's
   `ISP_BGOBJVALS.stencil` byte to 0.

---

## Confidence and on-silicon checks

- **Multi-draw, same state = N back-to-back INDEX_LIST blocks between one VDM_STATE and one
  terminate — HIGH confidence.** This is literally Mesa's draw model (state latched, draw = index
  list only, terminate once). Matches the HW block grammar in vdm.xml.
- **State change via an interleaved PPP_STATE_UPDATE before the next INDEX_LIST — HIGH confidence.**
  Direct consequence of Mesa's dirty-state emit + the fact PPP_STATE latches until re-issued.
- **Trailing 3-word STATE_TERMINATE emitted once at end (not per draw) — MEDIUM-HIGH.** Taken from
  the `pres_terminate` header + STATE_TERMINATE0/1 layout, i.e. a per-render terminator.
  Our proven single-draw path bundles it after the one draw; the assumption that it stays *once*
  after the last draw needs an on-silicon confirmation for the multi-draw case (verify TA_PHASE
  advances / TA_FINISHED fires with 2 index lists before it).
- **ISPB/ISPA stencil field values — HIGH confidence on layout and enum mapping** (ppp.xml +
  Mesa `pvr_setup_isp_faces_and_control` are unambiguous). The exact recipe (REPLACE on pass, EQUAL
  test, tagwritedisable for the mask) is the standard mask/test pattern; **needs silicon check** on
  two points:
  1. Whether the mask pass with `tagwritedisable=1` still needs a bound fragment PDS/USC (our proven
     path always binds one — safest to keep the fragment PDS bound and just disable tag writes).
  2. That the on-chip background stencil is genuinely 0 and that HSR/ISP does not discard the
     stencil-only mask primitives before they update stencil (punch-through/passtype interaction). If
     the mask pass gets HSR'd away, try `passtype = PUNCH_THROUGH(2)` for pass 1.
- **No stencil-enable bit needed anywhere; background stencil = ISP_BGOBJVALS.stencil in the 3D cmd —
  HIGH confidence** (Mesa never sets a stencil-enable; `pvr_arch_job_render.c:1130`).
