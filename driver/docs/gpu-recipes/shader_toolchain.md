# Offline GLSL/SPIR-V → PowerVR Rogue USC toolchain for BXS-4-64

**Device:** BXS-4-64, BVNC **36.53.104.796** (`pvr_device_info_36_53_104_796`), Mesa arch ROGUE, TI AM62P (Toradex Verdin).
**Goal:** compile arbitrary GLSL/SPIR-V vertex/fragment shaders to USC machine code **plus the
driver-integration metadata** a hand-written bare-metal driver needs — no Mesa runtime on target.

**Result:** a reusable harness (`pco_shc`) that takes `<stage> <in.spv>` and emits USC + metadata,
built on the *production* Mesa PCO backend for this exact device. All six requested shaders
(F1–F4, V1–V2) compile to **disassembly-verified** USC. Everything lives under the scratchpad.

---

## 1. How to compile a shader (reproducible)

### Route chosen: (a) GLSL → SPIR-V → NIR → PCO backend

Route (a) works and is what the harness uses. It runs the *same pipeline the real PVR Vulkan
driver runs* (`src/imagination/vulkan/pvr_arch_pipeline.c`): `spirv_to_nir` with
`pco_spirv_options()`, the common `vk_spirv_to_nir` lowering, then
`pco_preprocess_nir → pco_lower_nir → pco_postprocess_nir → pco_trans_nir → pco_process_ir →
pco_encode_ir`. The driver's register-allocation helpers (`pvr_alloc_vs_attribs`,
`pvr_alloc_vs_varyings`, `pvr_alloc_fs_varyings`, push-const/meta allocation) are **copied
verbatim** into the harness, so the vtxin / coefficient / shared-register layout is identical to
the real driver. Route (b) (hand-built NIR, à la `capture_smp_fs.c`) remains available for
one-off intrinsics but was **not needed** for any of these shaders.

### Files (all under `scratchpad/`)

| File | What |
|---|---|
| `mesa-build/mesa-26.2.1/src/imagination/vulkan/pco_shc.c` | the harness (route a) |
| `mesa-build/.../src/imagination/vulkan/meson.build` | adds the `pco_shc` target |
| `shaders/src/*.frag,*.vert` | GLSL sources |
| `shaders/compile.sh` | one-shot driver: GLSL→SPV→USC + metadata + disasm + `shaders.h` |
| `shaders/out/<name>.{spv,bin,meta.txt,dis.txt}` | per-shader outputs |
| `shaders/out/shaders.h` | all USC dword arrays as C |

### Build the harness (once)

```sh
cd scratchpad/mesa-build/mesa-26.2.1/build
ninja src/imagination/vulkan/pco_shc
```

### Compile one shader

```sh
glslangValidator -V -S frag shader.frag -o shader.spv
./src/imagination/vulkan/pco_shc fs shader.spv > shader.bin    # raw USC bytes on stdout
                                                               # metadata on stderr
# disassemble (production PCO disassembler; keep stdout clean with NOBIN):
PCO_SHC_NOBIN=1 PCO_DEBUG_PRINT=fs,vs,internal ./.../pco_shc fs shader.spv
```

Or compile the whole set and regenerate `shaders.h`:

```sh
scratchpad/shaders/compile.sh
```

### Two codegen modes

* **lean (default)** — marks the NIR `internal`, so the backend skips the runtime-gated Vulkan
  fragment epilogue (alpha-to-coverage / colour-write-enable / alpha-test / ISP feedback that a
  full FS reads from a per-draw *meta* control word) and the VS implicit point-size/clip-cull
  outputs. Produces minimal shaders in the exact style of the driver's existing `ddk_interp_fs` /
  `ddk_vs`. **Use this for the hand driver.**
* **full (`PCO_SHC_FULL=1`)** — the faithful full-Vulkan-pipeline output; larger, and the FS reads
  a *meta* control word from a shared register. The harness **auto-selects full mode for any FS
  that uses `discard`** (the coverage kill is only emitted on that path — see F2).

Output format: FS default target is `R8G8B8A8_UNORM` packed to one `po0` dword (matches
`ddk_interp_fs`); `PCO_SHC_OUT=f32x4` emits four f32 to `po0..3`.

### How output was verified

The production PCO disassembler (`pco_print.c`, same ISA tables as `pco_isa.py`, the same
mechanism used to verify `smp_tex_fs`) prints correct mnemonics for every shader — e.g. VS varying
emit `uvsw.write`, FS iteration `ditrp.pixel cf..`, mat4×vec4 as an `fmul`/`fmad` chain, and the
`discard` coverage kill as `savmsk.vm` + `alphaf`. Per-shader disassembly is in
`shaders/out/<name>.dis.txt`.

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

Raw dword arrays are also in `shaders/out/shaders.h` as `static const uint32_t <name>_usc[]`.
GLSL sources are in `shaders/src/`.

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

Array: `shaders/out/shaders.h` → `F2_discard_usc[104]` (also `out/F2_discard.meta.txt`).

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

1. Write GLSL in `shaders/src/<name>.<frag|vert>`.
   * Uniforms → `layout(push_constant) uniform { … };` (they land at `sh0..`, DOUTD).
   * Inter-stage varyings → `layout(location=N)` in/out (they get `VARYING_SLOT_VAR0+N`).
   * Vertex attributes → `layout(location=N) in …` (they get `vi` regs in order).
2. `glslangValidator -V -S <frag|vert> src/<name>.<ext> -o out/<name>.spv`
3. `pco_shc <fs|vs> out/<name>.spv > out/<name>.bin`  — USC bytes on stdout, metadata on stderr.
4. Read the metadata block for the driver wiring: `push_consts` (DOUTD sh layout), `fs_in`
   (DOUTI cf layout), `attrib`/`vs_out` (vtxin/UVSW), `temps`, `po` output.
5. Disassemble to sanity-check: `PCO_SHC_NOBIN=1 PCO_DEBUG_PRINT=fs,vs,internal pco_shc …`.
6. Or just run `shaders/compile.sh` (add a row to its `SHADERS` list) to regenerate `shaders.h`.

**Gotchas**
* `discard` ⇒ full mode (auto). The FS then reads a `meta` control word at `sh[meta]`; the driver
  must supply it. All other shaders here are lean (no meta).
* Textures/samplers: use the proven `capture_smp_fs.c` route (b) — combined image/sampler state at
  `sh0..3`/`sh4..7`, coords iterated from `cf`. `pco_shc` covers attributes/varyings/push-const
  uniforms/colour output; it does **not** yet wire Vulkan descriptor-set UBOs/SSBOs/samplers
  (those need `pvr_init_descriptors` + a descriptor layout). Push-constant uniforms are the
  bare-metal-friendly path and are fully supported.
* Attribute formats default to float32 vecN (inferred from the input type). For integer/packed
  vertex formats, set `data.vs.attrib_formats[...]` in `pco_shc.c`.
* Perspective (`smooth`) varyings pull in the w iterator at `cf0..3`; flat varyings would not.
  Everything here uses default `smooth`.

## 5. Status / confidence

* **Encoding correctness: very high** — emitted by the production PCO encoder for the exact BVNC and
  decoded back by the production disassembler; the alloc layout is byte-identical to the shipping
  Vulkan driver's own helpers.
* **Not yet silicon-tested here.** Each shader is compiler- and disassembler-verified; a single
  on-board draw per shader (solid-colour F1, transformed triangle V2+F4) would close the loop, as
  noted for `smp_tex_fs`.
* **Flagged:** none failed. F2 needed full mode for `discard` (handled automatically).
