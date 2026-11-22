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
| 93 | G310v2 | (10, 12, 4) | 1 | 16384 / 8192 | 2, 4, 16, 32, 32, 8 |
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
to 7. That is assumed, not taken from a probe log (section 5).

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
  them (section 5).

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

The G310 draws reported `gl_ref: renderer Mali-G310v2 MC4 (Panfrost)`.
The 580 include the references of plan #387's items (arith, fclane, negzero,
posvary, unif2, mixcol, loops2, varylink2, fmaloc, constprod) and the older
deqp ones. Only the G310v2 variant was drawn; v1-v3 share the tile buffer
and v4-v5 the G610's, and no deqp reference is a frame probe.

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
them.

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

### `src/mali/`

The frame builder now follows the GPU's tile buffer as Mesa 26.2.1 does
(after this note was first written):

- **The budgets come from the GPU.** The OSAL's `OSAL_QUERY_MALI_GPU`
  answer (`OsalMaliGpu`, `struct osal_mali_gpu`) carries `CORE_FEATURES`
  beside `GPU_ID`: the panthor development OSAL from
  `drm_panthor_gpu_info.core_features`, the Zephyr OSAL from the kernel
  side's own read of the register (`gpu::mali::ident`, offset 0x8;
  docs/notes/mali-gpu-model.md). The backend's `init` looks the model up as
  `pan_get_model` does (`src/mali/model.rs`: the product id from `GPU_ID`,
  the variant `CORE_FEATURES & 0xff`, against the G610 and G310v1-v5 rows of
  `pan_model.c:88-99`) and refuses a GPU with no row
  (`NOT_BUILT_GPU_MODEL`). The model's halved tile buffers
  (`pan_props.h:47-72`) go into the frame context
  (`frame::Context::tile_budget`). `TIB_BUDGET` is gone.
- **The Z bound is built.** `tile_size_of` takes the minimum of the colour
  budget's tile and the Z budget's for 4 bytes of depth a sample, as
  `pan_desc.c:615-625` does (the Z bytes are counted whether or not the
  frame has a depth buffer, `pan_desc.h:242-259`). The render targets'
  offsets and the hierarchy mask follow the tile and needed no change.
- **What it changed on the G610.** The Z bound reaches every 4x frame: 4x
  RGBA8 (16 bytes a pixel) now has a 512-pixel tile and an 8192-byte
  allocation, Mesa 26.2.1's, where the builder had followed the board's Mesa
  25.0.7 (1024 and 16384). The split-kick tests against Mesa 26.2.1 on the
  board (`src/mali/frame_split_tests.rs`) lost their named deviation and are
  word-equal, and so is the backend's split kick
  (`backend_tests.rs`, FBD w11 no longer excepted). The 25.0.7 board dumps
  of a 4x frame now name their tile word as 25.0.7's
  (`frame_msaa_tests.rs` `TILE_25_0_CAUSE`: the colour-only and the
  depth/stencil MSRTT frames, `a_4x_target_is_the_board_s_...`,
  `a_4x_depth_frame_...`, `a_4x_stencil_frame_...`; and
  `frame_preload_tests.rs`'s colour and depth preloads of a 4x target).
  The tiler hierarchy mask of a 512-pixel tile stays 254. No 1x frame
  moved: at one sample the Z bound is 2048 pixels on the G610 and 1024 on a
  G310v1-v3, never below the 1024-pixel clamp. On the board (Mesa 25.0.7
  is what the T6 runs) a 4x frame now has half the tile it had; both are
  legal tile sizes, and this is the size Mesa 26.2.1 draws with.
- **Tests on the G310.** `tools/mali/testdata/tile-budget/` holds Mesa
  26.2.1's three- and four-target frames and its 4x frame drawn as the
  G310v1, the G310v4 and the G610 (`drm_shim_ref.sh run --gpu`); the
  framebuffer parameters, the render targets and the tiler context of ours
  equal them (`frame_mrt_tests.rs`
  `three_and_four_targets_on_a_g310v1_are_mesa_s` and its G310v4 twin,
  `frame_split_tests.rs` `a_4x_frame_s_tile_follows_the_gpu_s_tile_buffer`).
- **`GL_RENDERER` names the model.** Each row of `src/mali/model.rs`
  carries its string, `Arm Mali-<Mesa's name> (YakoGL)`; the GL core's
  `glGetString` asks the backend for the string of the model `init` found
  (`mali::backend::renderer`). The NanoPC-T6 keeps
  `"Arm Mali-G610 (YakoGL)"` byte for byte; a G310 says
  `"Arm Mali-G310v1 (YakoGL)"` .. `"Arm Mali-G310v5 (YakoGL)"`, so dEQP
  results of the two boards differ in `info.renderer`. The backend table's
  name, which nothing reads, is `"Arm Mali Valhall (arch 10)"`: no board
  and no model.

What remains G610-specific in `src/mali/`:

- **The tiler levels come from the GPU too.** `MEASURED_TILER_LEVELS = 8`
  and its refusal are gone: the hierarchy mask is Mesa 26.2.1's
  `GENX(pan_select_tiler_hierarchy_mask)` (`pan_desc.c:1745-1812`,
  `frame::hierarchy_mask`) for any `TILER_FEATURES` max_levels, with the
  bin-pointer budget of the heap chunk (`pan_csf.c:979-983`,
  `Context::tiler_bin_budget`). At 8 levels and the frames built here it is
  the 25.0.7 rule the board's dumps hold (254, and 255 for a 256-pixel
  tile); only max_levels 0 is refused. The boot log quoted in
  docs/notes/mali-gpu-model.md reads 0x809 (8 levels) on an i.MX 95.
- `src/mali/backend.rs`: the query block's comment that derives
  the G610's 19 from `shader_present 0x50005` (the block is sized for 64
  core ids, `QUERY_BLOCK_BYTES`, so any core mask fits).
- The frame tests other than the ones above stay the G610's
  (`tiler_max_levels: 8`, `TileBudget::G610`).
- Which G310 variant the i.MX 95 has is not known: Mesa's v1-v3 and v4-v5
  differ in the tile buffer, and nothing public read here gives the
  board's `CORE_FEATURES`. The code does not assume one; it takes the
  variant from the register on the first boot (pre-study-imx95.md, B2), and
  the kernel side's G310 mock's variant 0 (`mock::G310_ASSUMED`) decides
  nothing but the mock.

### Not measurable on the host

The shim's G610 registers hide everything a raw register decides:

- the G310's `shader_present`, which drives the TLS/WLS sizes and the
  query size through `core_id_range`;
- `TILER_FEATURES`;
- `TEXTURE_FEATURES_0`: the compressed formats, AFRC and ASTC;
- `AFBC_FEATURES`;
- `THREAD_FEATURES` and `MEM_FEATURES`;
- `GPU_ID`'s arch revision and version fields;
- the i.MX 95's variant.

No public panthor probe log of an i.MX 95 was read for this note: this
machine had no web access. The G310 register values that Mesa does not
define are therefore not taken from a probe log: the arch revision nibble
(7) in `drm_shim_ref.sh`'s header is the one assumed value. The sweep used
variant 1 (`g310v2`). Variants 0 and 2 give the same frame bytes, because
they have the same tile buffer.

## Sample

`tools/mali/testdata/g310v2-frames/` (its `README.md` says how each was
drawn) holds the `--mrt 3` and `--samples 4` traces of the G610 and the
G310v2. They show differences 1-4.
