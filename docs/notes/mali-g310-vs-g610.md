<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali-G310 (i.MX 95) against Mali-G610 (RK3588): what Mesa generates differently

Date 2026-11-20, base `1d9022e7`. This is the i.MX 95 port's host
measurement. It does not need the board. The question: how much of what
Mesa 26.2.1 generates for a Mali-G310 differs from what it generates for the
Mali-G610, which our Mali back end (`src/mali/`) and compiler
(`src/shaderc/online/mali/`) are built and byte-compared against.

**Answer.** The shader side does not differ. Over every reference this
measurement reached, the G310's shader words, FAU words, facts and
descriptors are identical to the G610's (869 of 869 references, 2381 of 2381
program regions), and so is the whole pandecode trace. The one generated
difference is the frame's tile-buffer budget. It reaches the framebuffer,
render-target and tiler-context descriptors, only on the G310 variants with
the smaller tile buffer (v1-v3), and only for frames of 12 bytes a pixel or
more (three or four RGBA8 targets, 4x MSAA). The G310's GPU-side work is
therefore the frame builder's tile-buffer budget and its GPU identity, not
the compiler.

**The variant is measured, not assumed.** The Verdin iMX95's GPU reads
`GPU_ID 0xac740001` and `CORE_FEATURES 0x1`: Mesa's G310v2 (product (10, 12,
4), variant 1), one of the v1-v3 variants with the smaller tile buffer
(`tools/cts/RESULTS.md`, "Mali: Verdin iMX95 on Zephyr", row `imx95_ident`;
the 12 consoles of `tools/cts/log/2026-11-24-imx95-gles2-S1a-e63ab28e/` print
`mali: gpu_id 0xac740001 (G310v2, core_features 0x1), tiler_features
0x809`). The frame difference below therefore applies to the board.

Mesa citations are `file:line` in Mesa 26.2.1
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`). `P/` is `src/panfrost/` and
`G/` is `src/gallium/drivers/panfrost/`.

## 1. How Mesa tells the two GPUs apart

### The shim

`P/drm-shim/panfrost_noop.c` is the only knob:

- `PAN_GPU_ID` (hex) becomes `GPU_ID << 16` (`:22-31`, `:169`). The text
  after a colon becomes the variant in the low byte of `CORE_FEATURES`
  (`:37-48`, `:174`). The panthor kmod reads it back as `gpu_variant =
  core_features & 0xff` (`P/lib/kmod/panthor_kmod.c:162`). The revision is
  always 0 (`:170`).
- Every other register is a fixed G610 dump, whatever the id (`:176-191`):
  `shader_present 0x50005`, `tiler_features 0x809`, `l2_features
  0x7120306`, `mem_features 0x301`, `texture_features[0] 0xc1ffff9e`,
  `thread_features 0x4010000`, `max_threads 2048`.
- `cs_reg_count` follows the arch only (`:198-202`).

`tools/mali/drm_shim_ref.sh run` used to set `PAN_GPU_ID=a867` (the G610)
unconditionally. It now takes `--gpu g610|g310v1..g310v5`, with `g610` the
default:

- `g610` is `PAN_GPU_ID=a867`.
- `g310vN` is `PAN_GPU_ID=ac74:<N-1>`.

### The model table

`P/model/pan_model.c`:

| Line | Product | `PAN_PROD_ID` | Variant | Colour / Z tile buffer | Rates (pixel, texel, varying, cvt, fma, sfu) |
| --- | --- | --- | --- | --- | --- |
| 88 | G610 | (10, 8, 7) | 0 | 32768 / 16384 | 4, 8, 16, 64, 64, 16 |
| 91 | G310v1 | (10, 12, 4) | 0 | 16384 / 8192 | 2, 2, 8, 16, 16, 4 |
| 93 | G310v2 (the Verdin iMX95's GPU) | (10, 12, 4) | 1 | 16384 / 8192 | 2, 4, 16, 32, 32, 8 |
| 95 | G310v3 | (10, 12, 4) | 2 | 16384 / 8192 | 4, 4, 16, 48, 48, 12 |
| 97 | G310v4 | (10, 12, 4) | 3 | 32768 / 16384 | 4, 8, 16, 48, 48, 12 |
| 99 | G310v5 | (10, 12, 4) | 4 | 32768 / 16384 | 4, 8, 16, 64, 64, 16 |

Other facts of the table:

- `PAN_PROD_ID` is (arch major, arch minor, product major)
  (`P/model/pan_model.h:39-40`), read by `pan_prod_id` (`:120-127`).
- Every row has `MODEL_ANISO(ALL)` and no quirk.
- `pan_get_model` matches the product id and the variant exactly
  (`pan_model.c:139-149`).

The arch revision nibble of `GPU_ID` (bits 20-23) is in no table, and
nothing in Mesa reads it: `PAN_ARCH_REV` has no user. The script sets it
to 7, and the board agrees: bits 23:20 of its `GPU_ID 0xac740001` are 7
(`tools/cts/RESULTS.md`, row `imx95_ident`).

### Where `src/panfrost/` and `G/` branch on what the two differ in

- **Arch: the same.** Both are arch 10, `pan_arch` = `PAN_ARCH_MAJOR`
  (`pan_model.h:101-118`). Every `PAN_ARCH` / `dev->arch` branch,
  including the Valhall instruction set the compiler targets, is the same.
- **Arch minor.** It is read in only two places, and neither reaches
  arch 10:
  - `P/compiler/bifrost/bi_quirks.h:29-42` lists G71, G72 and G31
    (v6/v7) only.
  - `G/pan_fb_preload.c:1238-1239` tests `arch_major == 7`.
- **Product id.** `G/pan_blend_cso.c:114` tests `< 0x700` (Midgard);
  `G/pan_screen.c:144-146` tests T760. Neither applies here.
- **Variant.** `gpu_variant` reaches the compiler inputs
  (`G/pan_shader.c:119-120`, `G/pan_blend_cso.c:101`,
  `G/pan_fb_preload.c:531`). It is read only by `va_count_stats`
  (`P/compiler/bifrost/bifrost_compile.c:4038-4078`), which divides the
  shaderdb statistics by the model's rates. That is debug text: no word,
  no descriptor.
- **The model's tile buffer.**
  - `G/pan_device.c:67-69` takes the optimal budgets, half of each table
    size on arch 10 (`P/lib/pan_props.h:47-72`).
  - `G/pan_job.c:472-473` puts them in `tile_buf_budget` and
    `z_tile_buf_budget`.
  - `GENX(pan_select_tile_size)` (`P/lib/pan_desc.c:607-648`) sets
    `tile_size = budget >> log2_ceil(colour bytes a pixel)` (`:615`) and
    takes the minimum with the Z budget's (`:617-625`). It clamps to 32x32
    on arch 10 (`:639`, `pan_props.h:84-93`) and sets
    `cbuf_allocation = ALIGN(bytes * tile, 1 KiB)` (`:643`).
  - The render targets' `internal_buffer_offset` steps by bytes x tile
    (`pan_desc.c:1008`, `:1030`, `:1336`).
  - The tiler context's `hierarchy_mask` drops the levels below the tile
    (`pan_desc.c:1784-1790`, called from `G/pan_csf.c:980-982`).
- **GL caps from the tile buffer (not drawn).**
  - `get_max_msaa` (`G/pan_screen.c:129-151`, `pan_props.h:121-134`) gives
    a 16-byte format (RGBA32F) 8 samples on G310v1-v3 and 16 on the G610.
    RGBA8 and RGBA16F stay at 16.
  - `pan_get_max_cbufs` (`pan_props.h:105-118`, `pan_screen.c:132, 701`)
    stays at 8 on all of them.
- **`GL_RENDERER`.** It is `"%s MC%u (Panfrost)"` from the model's name
  and the core count (`G/pan_screen.c:1051-1054`).
- **What the shim does not vary.** Anisotropy is `rev >=
  min_rev_anisotropic` (`G/pan_screen.c:710-711`), true for both. Core
  count and range (`pan_props.c:41-55`) feed the TLS/WLS sizing
  (`G/pan_cmdstream.c:1678, 1688`, `G/pan_job.c:424-427`) and the query
  size (`G/pan_context.c:689`). Tiler features feed
  `pan_query_tiler_features`. Compressed formats come from
  `texture_features[0]`, AFBC from `afbc_features`, AFRC from bit 25
  (`pan_props.c:93-113`). These are raw registers, not model rows: the
  shim answers the G610's for every id, so this measurement cannot see
  them. The board's own values are in `tools/cts/RESULTS.md`, row
  `imx95_ident`: `TILER_FEATURES 0x809` and `SHADER_PRESENT 0x1` (the shim's
  0x809 and 0x50005), `L2_FEATURES 0x07110306` (the shim's 0x7120306).

## 2. Method

1. Build the shim: `tools/mali/drm_shim_ref.sh build
   $CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz
   $CONNECTED_DEPS_DIR/mesa-pan-shim`. It is the 26.2.1 tarball of
   `third_party/mesa-26.2.1/UPSTREAM`, built unmodified.
2. Set up the default check. The script's default must still be the G610.
   Eight references were redrawn with `run` and no `--gpu`, then
   `pan_ref_diff.py extract --draw` was run with `--attributes` when the
   reference has that record. The result is byte-identical to the
   committed `expect.json` (the `trace` field aside) for all eight:
   - `alu_armselect`, `alu_loopcarry`, `case121`,
     `deqp_intval_reslib_float_fs`, `deqp_loops_fs_cond_continue`: no
     options;
   - `algfold_189_half`, `deqp_unitype_fsivec4`,
     `deqp_unitype_depthrange_vs`: `--uniform-values`.
3. Sweep the references. For every `tools/mali/testdata/<name>.expect.json`
   whose pair could be found:
   - Draw `--gpu g610` with the first option set whose extract equals the
     committed reference. The sets tried were `--uniform-values`, none,
     `--prim points` with and without `--uniform-values`.
   - Draw `--gpu g310v2` with the same set.
   - Compare the two extracts per region (words, FAU, `work_reg_count`,
     `preload`, VA), plus `secondary_offset`, the attribute records and the
     whole trace with host pointers masked.

   The pair is `<name>.vert/.frag`, or the files that
   `src/shaderc/online/mali/tests.rs`'s tables name for `<name>`. The
   sweep script was scratch work, not committed. Steps 2 and 3 are
   `drm_shim_ref.sh run` plus `pan_ref_diff.py extract`.
4. Run the frame probes. `tri.vert`/`tri.frag` were drawn with each of
   the ten frame options below, on `g610` and on all five G310 variants.
   The traces were compared with host pointers masked.

## 3. Counts

The 958 committed `expect.json` files split three ways:

- 869 were reached: 841 by their own `.vert/.frag` and 28 through the test
  tables.
- 89 were not reached. Their shader text is generated by the tests
  (`cmp(...)`, `sel_*`) or they are drawn by other tools (`blend_*` by
  `pan_blend_ref.py`, `preload*`, `fsout_*`, `triangle_sample*`). The frame
  probes stand in for the frame kinds among them.

Of the 869, 866 G610 redraws equal the committed reference. The other three
(`instdiv_141_es3instdiv`, `other_63_matattr`, `vsattr_matcol`) need
options the sweep did not try (instances, divisors, matrix columns). They
were compared G310 against G610 with the same `--uniform-values` draw.

| Compared, G310v2 against G610 | Identical | Differing |
| --- | --- | --- |
| `vs.position` regions (words, FAU words, `work_reg_count`, `preload`, VA) | 869 | 0 |
| `vs.varying` regions | 643 | 0 |
| `fs.fragment` regions | 869 | 0 |
| `secondary_offset` | 869 | 0 |
| attribute and attribute-buffer records (`--attributes`; one with the instanced draw) | 102 | 0 |
| whole pandecode trace: every descriptor, the CS, the framebuffer and the tiler context (host pointers masked) | 869 | 0 |

### Recheck over the dEQP references of plan #387, tip `7b540fee`

The sample above predates the dEQP references the Mali plan added
(`tools/mali/testdata/deqp_*`). They were redrawn at tip `7b540fee` with the
section 2 method: the shim rebuilt unmodified from the 26.2.1 tarball, each
reference drawn with `drm_shim_ref.sh run --gpu g610`, the first option set
(`--uniform-values`, none, `--prim points` with and without it) whose
`pan_ref_diff.py extract --draw` (with `--attributes` where the committed
file has `attributes`) equals the committed `expect.json`, then the same
draw with `--gpu g310v2` and the two extracts compared with
`pan_ref_diff.py diff-ref` (words, FAU, `work_reg_count`, `preload`, VA,
`secondary_offset`, attributes), plus the whole pandecode trace with host
pointers masked. Scratch output was kept outside the tree.

| Family (`deqp_<family>_*`) | References |
| --- | --- |
| arith 144, loops2 127, constprod 31, constop 28, loops 26, intval 25, uniarray 23, fmaloc 22, fclane 20, unitype 20, colourpair 19, mixcol 18, posvary 18, unif2 15, misc 11, negzero 9, mixprec 8, varylink2 8, varylink 7, width 1 | 580 |

| Result | Count |
| --- | --- |
| deqp references compared (all `deqp_*.expect.json` at the tip; some overlap the first sample) | 580 |
| G610 redraw equals the committed `expect.json` | 580 |
| G310v2 extract identical to the G610 extract | 580 |
| whole trace identical (host pointers masked) | 580 |
| differing | 0 |

The G310 draws reported `gl_ref: renderer Mali-G310v2 MC4 (Panfrost)`
(the shim's four cores; the board has one, `SHADER_PRESENT 0x1`, `imx95_ident`).
The 580 include the references of plan #387's items (arith, fclane, negzero,
posvary, unif2, mixcol, loops2, varylink2, fmaloc, constprod) and the older
deqp ones. Only the G310v2 variant was drawn; v1-v3 share the tile buffer
and v4-v5 the G610's, and no deqp reference is a frame probe. G310v2 is the variant the Verdin
iMX95 reports (`imx95_ident`, S1a consoles), so the draws are those of the
board's variant.

### Recheck over the references added since `7b540fee`, tip `7e27b1eb`

Date 2026-11-25 (plan #387 item `loops-lower-1`), tip `7e27b1eb`. The
standing check `tools/mali/g310_check.sh` (README-deqp_ref.md, "Checking the
G310") redrew at this tip every committed `expect.json` reference its
`--since 7b540fee` selection names: `git diff --diff-filter=AMR` over
`tools/mali/testdata` since `7b540fee`, each `.expect.json` and each changed
`.vert`/`.frag` mapped to its reference. 387 committed references result:
320 named `deqp_*` (316 added since `7b540fee` and 4 redrawn
`deqp_unif2_*` struct references) and 67 not named `deqp_*` (45 `gm2_*`
glmark2-corpus references, 17 `route_*`, 5 `prec_rebal_*`). One added pair
maps to no `expect.json`: `tile-budget/mrt4.vert/.frag`, part of the frame
probes of `tools/mali/testdata/tile-budget/` (section 5), whose Mesa traces
are drawn per GPU (`mrt4` and `ms4` for the G610, G310v1 and G310v4) and
already record the frame difference of items 1-4; it is not a shader
reference to recheck.

Method as in section 2 and the `7b540fee` recheck: the 26.2.1 tarball
rebuilt unmodified with `drm_shim_ref.sh build` into
`$CONNECTED_DEPS_DIR/mesa-pan-shim` (tarball sha256 checked against
`third_party/mesa-26.2.1/UPSTREAM`); each reference drawn with the first
option set whose `pan_ref_diff.py extract --draw` (`--attributes` where the
committed file has attributes) equals the committed file, `trace` aside,
then the same draw with `--gpu g310v2`; the extracts compared per region
(words, FAU, `work_reg_count`, `preload`, VA) plus `secondary_offset` and
attributes. Only the G310v2 variant was drawn, as before. Nothing was
changed: the runs draw and compare.

| Run | Selected | Summary line |
| --- | --- | --- |
| `g310_check.sh --since 7b540fee` | 320 `deqp_*` | `310 identical, 0 differ, 10 skipped` |
| `g310_check.sh --since 7b540fee 'gm2_*' 'prec_rebal_*' 'route_*'` | 67 | `67 identical, 0 differ, 0 skipped` |

The 320 split by family: audit 38, boolpix 29, ctrlflow 27, vs16ref 23,
intval3 22, cfold2 19, intval4 19, idxloc 18, texmisc 17, intval5 13, gaps
10, misc2 10, irindex 9, intrnd 8, misc3 8, mixprec3 8, vsconst 8, vsunif
8, constrnd 6, fclane2 6, defects 4, unif2 4, loops3 3, structmember 3. The
67: gm2_fsop 9, gm2_nbshrf 9, gm2_cprod 6, gm2_fsarith2 6, gm2_nonfin 5,
gm2_tc425 4, gm2_fx2d 3, gm2_ideas 3, prec_rebal 5, route 17.

The 10 `NO-OPTION-SET` lines are the eight `deqp_misc3_bind_*` and the two
`deqp_defects_encode_*`: Mesa drew them with the aliasing attribute bindings
of each reference's `.bind` file (`gl_ref.py --attrib-bindings`, with
`--uniform-values` for the FAU uniform-value words), which the script's four
option sets do not try — skipped, not a pass. The same machinery drawn with
`--uniform-values --attrib-bindings <the reference's .bind file>` reproduced
each committed G610 reference and drew it on the G310v2: `10 identical, 0
differ, 0 skipped`. (Adding the `.bind` files to the script's option sets
would remove those skips; noted for the tool's owner.)

Counting the 10 in their real draw: 387 references added or changed since
`7b540fee`, 387 with a G610 redraw equal to the committed reference, 387
G310v2 extracts identical to the G610's — 0 differing, in no field. With
the 580 rechecked at `7b540fee` this covers every `deqp_*.expect.json` on
disk at the tip (896) and the new `gm2_*`, `prec_rebal_*` and `route_*`
references: the claim that the G310 needs no separate compiler work now
holds over the whole reference corpus the train compares.

### The identity reached the driver

"Identical" does not mean "the switch did nothing". Three pieces of
evidence show it:

- Every G310 draw reported `gl_ref: renderer Mali-G310v2 MC4 (Panfrost)`,
  and every G610 draw `Mali-G610 MC4` (`G/pan_screen.c:1051-1054`).
- The compiler's shaderdb line divides by the G310v2's rates. For
  `tri_varying`'s position shader, the G610 gives `0.031250 fma, 0.218750
  cvt, 0.250000 alu` and the G310v2 gives `0.062500 fma, 0.437500 cvt,
  0.500000 alu`: fma rate 32 against 64 (`bifrost_compile.c:4074-4078`,
  `pan_model.c:89, 93`).
- The frame probes below differ by product and variant.

### Frame probes

`tri.vert`/`tri.frag` on a 64x64 target. Each cell gives the effective tile
size (pixels), the colour buffer allocation (bytes) and the hierarchy mask.

| Probe (`gl_ref.py` options) | G610, G310v4, G310v5 | G310v1, v2, v3 |
| --- | --- | --- |
| none; `--mrt 2`; `--depth-stencil` (with and without `--second-frame`); `--second-frame`; `--size 1920x1080` | identical traces | identical traces |
| `--mrt 3` (12 B/px) | 1024, 12288, 254 | **512, 6144**, 254 |
| `--mrt 4` (16 B/px) | 1024, 16384, 254 | **512, 8192**, 254 |
| `--samples 4` (4x RGBA8 with a D24 plane) | 512, 8192, 254 | **256, 4096, 255** |
| `--samples 4 --second-frame` | 512, 8192, 254 | **256, 4096, 255** |

## 4. The differences, classified

1. **Tile size and colour buffer allocation** (framebuffer `Parameters` and
   the fragment job's copy). Cause: `pan_model.c:91-96` has a 16 KiB colour
   tile buffer, halved to an 8 KiB budget (`pan_props.h:47-58`), and
   `pan_desc.c:615, 643` divides by it. Example, `--mrt 3`
   (`tools/mali/testdata/g310v2-frames/mrt3.*.trace`, lines 737/744 and
   813/820): G610 `Effective Tile Size: 1024` / `Color Buffer Allocation:
   12288`; G310v2 `512` / `6144`.
2. **Render-target internal buffer offset** (each RT after the first).
   Cause: `pan_desc.c:1336`, bytes a pixel x the smaller tile. Example,
   same trace, lines 923 and 953: G610 `Internal Buffer Offset: 4096`,
   `8192`; G310v2 `2048`, `4096`.
3. **The Z budget's bound on the tile.** Cause: `pan_model.c:91-96`, an
   8 KiB Z tile buffer and so a 4 KiB budget, and `pan_desc.c:617-625`.
   Example, `--samples 4` (`ms4.*.trace`): 16 bytes of D24 a pixel gives
   G610 512 and G310v2 256. On both GPUs the Z bound is below the colour
   bound (1024 and 512), and the trace's `Z Internal Format: D24` is the
   plane.
4. **Tiler hierarchy mask.** Cause: `pan_desc.c:1784-1790`: a tile of 256
   or fewer pixels keeps level 0. Example, `ms4.*.trace`: G610 `Hierarchy
   Mask: 254`, G310v2 `255`.
   The recheck at `7b540fee` (section 3) found no further difference: none
   of the 580 deqp references differs in any field.
5. **Not generated code**: the shaderdb statistics (above), `GL_RENDERER`,
   and the RGBA32F `GL_MAX_SAMPLES` (8 against 16, section 1).

G310v4 and G310v5 have the G610's tile buffer, and none of 1-4 appears on
them. The Verdin iMX95 is a G310v2 (`imx95_ident`), so 1-4 do apply to it.

## 5. What it means for our code

### `src/shaderc/online/mali/`

No G310 path is needed. The conclusion still holds at `7b540fee`: the 580
dEQP references (plan #387's families included) have identical words, FAU,
facts, attributes and traces on the G310v2 and the G610, so each lowering
item serves both boards. The first sample's 869-of-869 count is unchanged;
the recheck is a second, overlapping count, not a sum.


- Every compiler branch that sees the product is on arch major 10,
  which the G310 shares.
- The variant reaches only the statistics.
- 2381 of 2381 regions are word-, FAU- and fact-identical.
- No file of the compiler names the G610 except in comments.

### `src/mali/` and the kernel side

The constants of this section's first draft (`TIB_BUDGET`,
`MEASURED_TILER_LEVELS`, the renderer name, the core mask) are gone: no
`src/` or `driver/` file defines `TIB_BUDGET` or `MEASURED_TILER_LEVELS`
(`grep` at the branch tip). Each value now comes from a register the board
reads, or from a table row that a register selects. Paths are at the branch
tip; "tip" commits are the ones `git log -S` finds first.

| Former constant | Now | Source | Commit |
|---|---|---|---|
| `TIB_BUDGET` | the model row's tile buffer (Mesa's `MODEL_TB_SIZES`: G610 and G310v4-v5 32768 / 16384, G310v1-v3 16384 / 8192), halved into `TileBudget`; G310v2 gets colour 8192, Z 4096. `init` finds the row from `GPU_ID` (product) and `CORE_FEATURES & 0xff` (variant) | `src/mali/model.rs:95-120` (rows, `model()`), `src/mali/backend.rs:1192` (lookup, refusal), `:1230` (`model.tile_budget()` into the context), `src/mali/frame.rs:379`, `:2017` (`tile_size_of`) | `cb8f5e38` (2026-11-21) |
| `MEASURED_TILER_LEVELS` | `TILER_FEATURES` bits 11:8 (`max_levels`); 0 is refused, any other count goes through Mesa 26.2.1's hierarchy-mask rule | `src/mali/frame.rs:495` (`tiler_max_levels`), `:2678` (refusal), `:3793` (`hierarchy_mask`), `src/mali/backend.rs:1229` | `cb8f5e38` |
| `CORE_FEATURES` reaching the backend | the OSAL's query answer carries it beside `GPU_ID`; the Zephyr side reads register 0x8 into `Ident` | `src/osal/mali.rs:57`, `driver/rust/src/gpu/mali/mod.rs:243,281`, `src/mali/backend.rs:1192` | `cb8f5e38` |
| the renderer name | per-model string `Arm Mali-<Mesa's name> (YakoGL)`, made in the row macro; `glGetString(GL_RENDERER)` asks the backend for the string of the model `init` found; `"Arm Mali (YakoGL)"` before `init` | `src/mali/model.rs:85`, `src/mali/backend.rs:336-346`, `src/gl/state.rs:937` | `21ac93ad` (2026-11-22) |
| shader core mask (firmware `core_en_mask`, group compute and fragment masks, core counts) | `SHADER_PRESENT` (register 0x100), read into `Ident` and passed on; no 0x50005 in the code but tests and comments | `driver/rust/src/gpu/mali/mod.rs:69,287`, `csf.rs:828` (`core_en_mask: cfg.shader_present`), `sched.rs:694-698` | `fa1e0c1b` (2026-10-28, firmware boot), `6b7839b5` (2026-11-03, group masks) |

What the board's registers give, and so what the code does on it:

- `GPU_ID 0xac740001` with `CORE_FEATURES 0x1` is the row `G310v2`
  (product `PAN_PROD_ID(10, 12, 4)`, variant 1): `TileBudget` colour 8192,
  Z 4096 (`src/mali/model.rs:102-109`, tested at `:130`). A GPU with no
  row (`NOT_BUILT_GPU_MODEL`, `src/mali/backend.rs:157`) is refused, not
  drawn with the G610's numbers.
- `TILER_FEATURES 0x809` is 8 levels, as on the G610, so the hierarchy
  mask is the same as on the G610 for the same frame. The boot log quoted in
  docs/notes/mali-gpu-model.md reads 0x809 on an i.MX 95.
- `SHADER_PRESENT 0x1` is one shader core: `core_en_mask`, both group masks
  and the core counts follow the register. The T6's 0x50005 survives only
  as the G610 test fixture (`csf.rs:1243`, `sched.rs:2416`) and the mock (`mock.rs:27`) and in comments.
- `GL_RENDERER` reports the G310 on a G310: `"Arm Mali-G310v2 (YakoGL)"`
  here. The T6 keeps `"Arm Mali-G610 (YakoGL)"` byte for byte, so the
  `info.renderer` of dEQP results tells the boards apart. The backend
  table's name (`"Arm Mali Valhall (arch 10)"`, `src/mali/backend.rs:3765`)
  is read by nothing.

What stays G610-specific:

- `src/mali/backend.rs:397-402`: the query block (`QUERY_BLOCK_BYTES`) is
  sized for 64 core ids, so a comment derives the G610's 19 from
  `shader_present 0x50005`; any core mask fits, the value is not used.
- The frame tests other than the tile-budget ones use the G610
  (`tiler_max_levels: 8`, `TileBudget::G610`, `src/mali/model.rs:48`).
- The hierarchy rule gives the 25.0.7 mask (the board's dumps) at 8 levels
  for the frames built here; the tile of a 4x frame follows Mesa 26.2.1,
  half the 25.0.7 tile the T6's Mesa draws (`src/mali/frame_msaa_tests.rs`
  `TILE_25_0_CAUSE`).
- The kernel side's G310 mock (`driver/rust/src/gpu/mali/mock.rs:99`,
  `G310_ASSUMED`, `shader_present: 0x3`) is an assumed variant 0 and two
  cores; it decides nothing but the mock. The board's value is 0x1.

### Not measurable on the host

What the board measured (`tools/cts/RESULTS.md`, row `imx95_ident`, record
`records/verdin-imx95/20261125-0127-imx95_ident-9734aab3`): `GPU_ID`
0xac740001 (arch 10.12 revision 7, product 4: Mesa's G310), `SHADER_PRESENT`
0x1 (one shader core), `CORE_FEATURES` 0x1 (Mesa's G310v2), `TILER_FEATURES`
0x809 (8 levels), `L2_FEATURES` 0x07110306, `MMU_FEATURES` 0x2830,
`AS_PRESENT` 0xff, `COHERENCY_FEATURES` 0x0. The variant is therefore not
unknown and the arch revision nibble 7 is not assumed: both are the board's
reads, and the sweep's variant 1 (`g310v2`) is the board's variant.

`G310_ASSUMED` is gone from `src/` (`grep -rn G310_ASSUMED src/` finds
nothing). It remains in the kernel side's mock,
`driver/rust/src/gpu/mali/mock.rs`, with `core_features: 0` and
`shader_present: 0x3`; `imx95_ident` reports 1 of 9 mock words differing and
reads `CORE_FEATURES` 0x1, so the mock's table is not the board's. It
decides nothing but the mock.

Still not measurable on the host, because the shim's G610 registers hide
what a raw register decides and the board's reads above cover only the
registers `imx95_ident` dumps:

- what the one core's `shader_present` does to the TLS/WLS sizes and the
  query size through `core_id_range` (the register is read; the sizes the
  driver derives from it are not checked on the host);
- `TEXTURE_FEATURES_0` (the compressed formats, AFRC and ASTC),
  `AFBC_FEATURES`, `THREAD_FEATURES` and `MEM_FEATURES`: not in the
  `imx95_ident` row;
- the effect on the G310's output of the constants that still carry G610
  values (`TIB_BUDGET`, `MEASURED_TILER_LEVELS`, the renderer name);
- time: see the next subsection.

Variants 0 and 2 give the same frame bytes as variant 1, because they have
the same tile buffer.

### Measured dEQP time scale (S1a)

Session S1a ran rows T6S1R01-R12 of the dEQP-GLES2 tree on the Verdin iMX95
at `e63ab28e` (`tools/cts/log/2026-11-24-imx95-gles2-S1a-e63ab28e/`, commit
`81c25e26`; `rows.tsv`). The T6 (G610) baseline for the same rows is
`tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055/` (`chunks.tsv`, column
`seconds`, whole seconds).

Method: the iMX95's time of a row is the run time of its image up to PSCI
SYSTEM_RESET (the ms in the `exit_line` of `rows.tsv`; the `seconds` column
is the same figure truncated), so the U-Boot load is not in it. The pooled
ratio is the iMX95's seconds summed over the 12 rows over the T6's: 156.1 s
over 72 s = **2.17** (recomputed with a scratch script; the whole-second sum
157 s gives 2.18). Per row it runs 0.12 (R08, 0.36 s over 3 s) to 5.28 (R12,
42.2 s over 8 s), mean 1.88, standard deviation 1.30. R12 is the outlier.

Caveats:

- 12 rows of 76; the ratios differ by a factor of 40 between rows, so the
  pooled figure is a scale, not a forecast of one row.
- The T6's seconds are whole seconds from its chunk table, which may
  include overhead the iMX95's image time does not; for a 3 s row one
  second is a third.
- `SHADER_PRESENT` 0x1 is one shader core against the G610's several
  (0x50005 on the T6), so part of the ratio is the core count and not the
  driver; the boards also run different systems (Zephyr, Linux).
- The two sides ran at different compiler commits (`8ff01055` and
  `e63ab28e`), so the draws refused can differ.
- The earlier read-only item's forecast (not recomputed here): no
  remaining row above 300 s of the 600 s lifetime; the slowest is T6S2R05,
  34.7 s at the pooled ratio, 84.5 s at the high bound.

Redo these figures after sessions S2 and S3, when all 76 rows have run.

## Sample

`tools/mali/testdata/g310v2-frames/` (its `README.md` says how each was
drawn) holds the `--mrt 3` and `--samples 4` traces of the G610 and the
G310v2. They show differences 1-4.
