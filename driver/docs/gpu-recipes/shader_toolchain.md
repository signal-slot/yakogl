<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Offline GLSL/SPIR-V → PowerVR Rogue USC toolchain for BXS-4-64

**Device:** BXS-4-64, BVNC **36.53.104.796** (`pvr_device_info_36_53_104_796`), Mesa arch ROGUE, TI AM62P (Toradex Verdin).
**Goal:** compile arbitrary GLSL/SPIR-V vertex/fragment shaders to USC machine code **plus the
driver-integration metadata** a hand-written bare-metal driver needs — no Mesa runtime on target.

**Result:** a reusable harness (`pco_shc`) that takes `<stage> <in.spv>` and emits USC + metadata,
built on the *production* Mesa PCO backend for this exact device. The harness and its build
are `tools/pco_shc/` (its README has the route, the commands and the two code generation
modes). This file keeps the register model the metadata describes (section 2) and six
worked examples from the bring-up (section 3).

---

## 1. How to compile a shader

`tools/pco_shc/README.md`: the route (GLSL -> SPIR-V -> NIR -> PCO, with the driver's own
allocation helpers), building the harness, compiling one shader by hand, the lean and full
modes, and the switches. `tools/glsl2usc.sh` compiles a whole shader list into the offline
table.

---

## 2. Register model the driver must feed

| Shader resource | USC location | Driver mechanism |
|---|---|---|
| VS vertex attributes | vertex-input regs `vi0..` (`vtxins`) | vertex-fetch DMA (PDS), one dword per attrib dword |
| VS outputs (position + varyings) | vertex-output store, offsets `vtxout` | UVSW emit (`uvsw.write <val>, <offset>`); `vtxouts` = `vs_output_size` |
| FS input varyings | coefficient regs `cf0..` (`coeffs`) | DOUTI/coefficient (iteration) PDS program |
| FS/VS uniforms (push constants) | shared regs `sh0..` (`shareds`) | DOUTD DMA into the USC common store |
| FS colour output | pixel-output reg `po0..` | PBE/EOT |
| FS *meta* (full mode only) | shared reg `sh[meta]` | driver-supplied ISP-feedback control word |

**Coefficient layout rule (FS):** perspective-correct (`smooth`) varyings make the shader read
`1/w`, so slot `POS` (the w iterator) is allocated **first** at `cf0..3`
(`ROGUE_USC_COEFFICIENT_SET_SIZE = 4` dwords/component-set), then the user varyings `VAR0, VAR1, …`
in ascending location. Each scalar varying component takes one 4-dword coefficient set
(A,B,C,pad). The DOUTI program must iterate exactly this layout.

**Attribute/output rule (VS):** attributes are packed into `vi` registers in ascending location,
one dword per attribute dword. Outputs are emitted with `uvsw.write val, offset`; position is
`VARYING_SLOT_POS`, user varyings are `VARYING_SLOT_VAR0+n`. `f32_smooth/flat/npc` report how many
f32 varying components fall in each interpolation class (drive the fragment varying/iterator
config from these).

**Uniform rule:** a GLSL `layout(push_constant)` block lands at `sh[start..]` (start = 0 here).
`load_push_constant` at byte offset *O* reads `sh[start + O/4]`. The driver DMAs the uniform bytes
to those shared dwords with a DOUTD program.

---

## 3. The shaders (USC + metadata)

Six shaders compiled during the bring-up (September 2026), kept as worked examples of the
register model and of the code size and temps each shape costs. Their sources and outputs
were in a scratchpad and are not in the tree.

### F1 — uniform/constant colour  *(lean, 16 dwords, temps=5)*

GLSL: `layout(push_constant) uniform U { vec4 u_color; }; o_color = u_color;`
Disasm: `mbyp sh0..sh3 → r0..3; rpt4 pck.scale.u8888; mov.olchk po0`.

* **Uniform:** `u_color` at **shared regs sh0..sh3** (`push_consts`: start=0, count=4 dwords).
  DOUTD must load the 4 colour dwords (RGBA float) to common store dwords 0..3.
* **Varyings:** none. **Output:** `po0` (RGBA8-UNORM packed). **temps=5, shareds=4.**

```c
static const uint32_t F1_uniform_usc[16] = {
  0x87008235,0x00000880,0x82354000,0x08818700,0x41000000,0x87008235,0x00000882,0x82354200,
  0x08838700,0x43000000,0x9c069258,0xa0408020,0x2c001040,0xfff1ff44,0x87808a34,0x20000044,
};
```

### F2 — discard/kill  *(full mode — auto-forced, 104 dwords, temps=9)*

GLSL: `if (v_uv.x < 0.5) discard; o_color = vec4(v_uv,0,1);`
Kill verified as `savmsk.vm` (+ `alphaf` ISP feedback), `uses.discard=1`.

* **Varyings:** `v_uv` (VAR0, vec2) at **cf4..11** (2 coeff sets); the **w iterator at cf0..3**
  (`uses.w=1`). DOUTI must iterate w into cf0..3 and v_uv into cf4..11.
* **Meta:** reads a driver-supplied **ISP-feedback control word at sh0** (`meta`: start=0,count=1).
  Supply it so alpha-to-coverage/alpha-test are off and colour writes enabled (all-features-off =
  a benign value; the kill from `discard` is independent and always active).
* **Output:** `po0`. **temps=9, coeffs=12, shareds=1.**
* **Note:** discard *requires* full mode — in lean mode the coverage kill is silently dropped.


### F3 — branch + loop  *(lean, 80 dwords, temps=13)*

GLSL: data-dependent `if (v_in.x>0.5) { for(i<4) acc+=v_in*i; } else acc=v_in*2;`
Disasm shows real control flow (`br`/`if(p0)`) plus an `fmul`/`fmad` accumulation.

* **Varyings:** `v_in` (VAR0, vec4) at **cf4..19** (4 coeff sets); w iterator at **cf0..3**.
* **Output:** `po0`. **temps=13** (higher, as expected for control flow). **coeffs=20.**

Array: `shaders.h` → `F3_branchloop_usc[80]`.

### F4 — multi-varying  *(lean, 52 dwords, temps=10)*

GLSL: `o_color = v_color * vec4(v_uv,1,1);` — two input varyings.
Disasm: `ditrp.pixel cf4..19` (v_color) and `ditrp.pixel cf20..27` (v_uv), then `fmul` → pack → po0.

* **Varyings (coeff/DOUTI layout — the multi-varying reference):**
  * w iterator: **cf0..3**
  * `v_color` (VAR0, vec4): **cf4..19** (4 coeff sets)
  * `v_uv` (VAR1, vec2): **cf20..27** (2 coeff sets)
* **Output:** `po0`. **temps=10, coeffs=28.**

```c
static const uint32_t F4_multivary_usc[52] = {
  0xf300c057,0x8040f2b0,0x00000040,0x8104ff00,0x00080060,0x81040000,0x00080061,0x80040000,
  0x00050478,0xc0570040,0xf2b0f300,0x00408040,0xff000000,0x00608104,0x00000008,0x00618104,
  0x00000008,0xff6a8002,0xf300c057,0x8040f2b0,0x00000040,0x8104ff00,0x00080060,0x81040000,
  0x00080061,0x80040000,0x00150878,0xc0570020,0xf2b0f300,0x00408040,0xff000000,0x00608104,
  0x00000008,0x00618104,0x00000008,0xff6a8002,0x40008235,0x0000a8c4,0x8235ff40,0xa9c54000,
  0xff410000,0x87008234,0x42000046,0x87008234,0x43000047,0x9c06925a,0xa0408020,0x2c001040,
  0xfff3ff44,0xffffffff,0x87808a34,0x20000044,
};
```

### V1 — passthrough, 2 attributes → 2 varyings  *(lean, 10 dwords, temps=0)*

GLSL: `gl_Position=a_pos; v_color=a_color; v_uv=a_uv;`
Disasm: `mov vi0..3/vi4..7/vi8..9`, then `uvsw.write a_color,0x4`, `uvsw.write a_uv,0x8`,
`uvsw.write pos,0x0`.

* **Vertex inputs (vtxin, `vtxins=10`):**
  * `a_pos` (loc 0, vec4): **vi0..3**
  * `a_color` (loc 1, vec4): **vi4..7**
  * `a_uv` (loc 2, vec2): **vi8..9**
  (attrib formats inferred float32 vecN; override in the harness if you need packed formats.)
* **Vertex outputs / UVSW emit (`vtxouts = vs_output_size = 10`):**
  * `POS`: offset **0** (vtxout0..3)
  * `v_color` (VAR0): offset **4** (vtxout4..7)
  * `v_uv` (VAR1): offset **8** (vtxout8..9)
  * `f32_smooth = 6` (v_color 4 + v_uv 2, both perspective).
* **Matching FS coeff layout** (what a consumer FS like F4 expects): w@cf0..3,
  VAR0@cf4..19, VAR1@cf20..27.

```c
static const uint32_t V1_passthrough_usc[10] = {
  0x0806a055,0x00048404,0xa0553000,0x88080802,0x30000004,0x0806a056,0x00048000,0xfff13000,
  0x0580a044,0xff000000,
};
```

### V2 — uniform MVP transform  *(lean, 92 dwords, temps=16)*

GLSL: `layout(push_constant) uniform U { mat4 u_mvp; }; gl_Position = u_mvp * a_pos;`
Disasm: reads `sh0..sh15` (u_mvp) and `vi0..3` (a_pos), 16-term `fmul`/`fmad` chain, `uvsw.write`.

* **Uniform:** `u_mvp` (mat4) at **shared regs sh0..sh15** (`push_consts`: start=0, count=16
  dwords, column-major). DOUTD loads 16 dwords to common store 0..15.
* **Vertex input:** `a_pos` (loc 0, vec4): **vi0..3** (`vtxins=4`).
* **Vertex output:** `POS` at offset 0 (`vtxouts = vs_output_size = 4`); no user varyings.
* **temps=16.**

Array: `shaders.h` → `V2_mvp_usc[92]`.

---

## 4. How to add a new shader later

Add the GLSL to the shader list `tools/glsl2usc.sh` reads, or run `pco_shc` by hand
(`tools/pco_shc/README.md`). Read the metadata block for the driver wiring: `push_consts`
(the DOUTD shared-register layout), `fs_in` (the DOUTI coefficient layout), `attrib` and
`vs_out` (vertex inputs and UVSW), `temps`, and the `po` output.

**Gotchas**
* `discard` forces full mode. The FS then reads a `meta` control word at `sh[meta]`, and
  the driver must supply it.
* Combined image samplers go to shared registers by binding: `sh[0..19]` for binding 0
  (image, image meta, sampler, sampler meta, gather; 4 dwords each), `sh[20..39]` for
  binding 1, and so on (`tools/pco_shc/pco_shc.c`, "descriptor sets").
* Attribute formats are float32 vecN, inferred from the input type.
* Perspective (`smooth`) varyings pull in the w iterator at `cf0..3`; flat varyings do not.

## 5. Status / confidence (September 2026)

* **Encoding correctness: very high** — emitted by the production PCO encoder for the exact BVNC and
  decoded back by the production disassembler; the alloc layout is byte-identical to the shipping
  Vulkan driver's own helpers.
* **Not yet silicon-tested here.** Each shader is compiler- and disassembler-verified; a single
  on-board draw per shader (solid-colour F1, transformed triangle V2+F4) would close the loop, as
  noted for `smp_tex_fs`.
* **Flagged:** none failed. F2 needed full mode for `discard` (handled automatically).
