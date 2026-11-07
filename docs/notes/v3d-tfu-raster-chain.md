<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D TFU: a mip chain from a raster level 0, and `tfu_tile` level by level

Study, no code change, nothing run on the board. It closes two questions
left open by `v3d-terrain-copies.md` (design 3) and `pi4-slow-terrain.md`
(candidate 3b).

Mesa is `mesa-26.2.1`, commit `da14d65e4499e66468094be52bff9ea0915a695e`
(the version `tools/v3d/README.md` names; `third_party/mesa-26.2.1` is not
the whole tree and holds no TFU file, so the sources were read in a shallow
clone, `$CONNECTED_DEPS_DIR/mesa-v3d`). Mesa paths below are that commit's.
yakogl line numbers are `e2a4e86b`; `src/v3d`, `src/gl`, `driver/src/v3d`
and `src/osal` are byte-identical at origin/main `a8e79b52`, the tip when
this was written. Running items (`v3d-rt-evict`, `pi4-target-mark-clear`)
will move the `src/v3d/backend.rs` numbers; the names stay.

## Answers

1. **Does the V3D 4.2 TFU take a raster source with NUMMM > 0?** The
   register packing allows it, no Mesa path does it, and no source I could
   read says what the hardware does. So: *by the fields, yes; by hardware,
   unmeasured.* A raster level 0 can be the source of a chain job (and the
   same job can write level 0 into the tiled image) if one board probe,
   below, says the levels come out right. Until then it is a design, not a
   feature.
2. **Can `tfu_tile` tile a chain one TFU job a level?** Yes, with no new
   hardware behaviour: every job is the single-level raster-to-tiled job
   that `tfu_tile` already runs and gl_test case 140 measured byte-exact,
   only with the level's own size, source rows and destination slice. It
   needs a per-level pack function (the existing one is level-0 only) and a
   decision about the host stand-in. The result is a byte copy of the GL
   core's levels, so it cannot change a picture.

## The register fields

Mesa `src/broadcom/common/v3d_tfu.h:24-52` (V3D 4.2 names, `V3D33_*`):

| register | field | meaning | Mesa |
|---|---|---|---|
| `IOS` | 15:0 width, 31:16 height | level 0 size, in texels | `v3dx_tfu.c:47-48,98` |
| `IIA` | address | source (level 0) | `:106-108` |
| `IIS` | raster: stride in texels; UIF: padded height in UIF blocks; LT and UB-linear: 0 | source stride | `:114-127` |
| `ICFG` bits 18+ | input format: 0 raster, 11 LINEARTILE, 12/13 UBLINEAR 1/2 column, 14/15 UIF no-XOR/XOR | **the source's layout** | `:130-137`, `v3d_tfu.h:36-43` |
| `ICFG` bits 9+ | `TTYPE`: the texel data type the job moves/filters as | | `:138` |
| `ICFG` bits 5+ | `NUMMM`: mip levels made after the first | | `:147` |
| `ICFG` bits 22+ | `OPAD`: UIF block rows of padding of the *level 0 destination* | | `:154-162` |
| `IOA` bit 0 | `DIMTW`: level 0 is **not** written, only levels 1.. | set iff `last_level != base_level` | `:140-141`, `v3d_tfu.h:27` |
| `IOA` bits 3+ | output format: 3 LT .. 7 UIF XOR (no raster on 4.2) | the destination's layout | `:143-145` |
| `IOA` bits 6+ | destination address | | `:110-112` |

On V3D 7.1 the same fields move to `IOC` and the output may be raster
(`:165-202`, `v3d_tfu.h:45-61`). The Pi 4 is 4.2; the Pi 5 plan (#321) is
the 7.1 reader of this note.

Two facts about NUMMM and the format fields follow from the packing alone:

- The input format (`ICFG` 18+) and `NUMMM` (`ICFG` 5+) are separate fields
  (13 bits apart in the word). Nothing in the packing ties "raster" to
  "NUMMM = 0".
- The output format is a *single* field for the whole job. Levels 1.. have
  no field of their own: Mesa's comment says "When filling mipmaps, the
  miplevel 1+ tiling state is inferred" (`v3dx_tfu.c:149-151`). The
  hardware picks each level's tiling from its size, which is the rule
  `v3d_setup_slices` lays the image out by (`v3d_resource.c:629-670`: sizes
  `i < 2` real, `i >= 2` power-of-two, tiling by width against the utile and
  UIF block widths; only level 0 depends on `uif_top`). That rule is what
  `v3d_tex_layout` copies (`driver/src/v3d/v3d_tex.c:214-317`).

## What Mesa does

- **Gallium `v3d_generate_mipmap`** (`v3d_blit.c:240-276`) calls `v3dX(tfu)`
  with `psrc == pdst`, `src_level == base_level`, `for_mipmap = true`
  (`:264-272`). So the source and the destination are the same slice and
  have the same tiling.
- `v3dX(tfu)` refuses on 4.2 a raster *destination* (`v3dx_tfu.c:58-62`) and
  again, for mipmaps, a raster destination (`:64-66`). Because source ==
  destination, a raster resource never reaches `NUMMM > 0`: Mesa's own
  resources get a raster base level only when they are one level
  (`v3d_resource.c`'s tiling choice), so **the gallium driver never builds a
  job with a raster source and `NUMMM > 0`.** It does support a raster source
  with `NUMMM = 0` on the blit path (`v3d_tfu_blit`, `v3d_blit.c:279-320`,
  `for_mipmap = false`, `icfg` format 0 at `v3dx_tfu.c:130-132`, `IIS` the
  stride at `:121`).
- **Vulkan `v3dv`** builds raster-source jobs too
  (`src/broadcom/vulkan/v3dvx_meta_common.c:927-1035`: `IIS` stride at
  `:1010-1011`; comment at `:1018` "The TFU can handle raster sources but
  always produces UIF results"). It never sets `NUMMM`: no hit for `NUMMM`
  or `DIMTW` outside `v3d_tfu.h` and gallium's `v3dx_tfu.c` in
  `src/broadcom/`.
- Mesa's mipmap comment at `v3dx_tfu.c:149-151` says level 0 is written when
  `DIMTW` is clear and OPAD then describes its padding, so "write level 0
  and the mip levels in one job" is a defined combination. Combined with a
  raster `IIA` it is exactly the job below. It is not what Mesa runs.
- The Linux V3D simulator, `src/broadcom/simulator/v3dx_simulator.c`, does
  not model the TFU's filter or formats (no `NUMMM` hit), so Mesa's tree has
  no executable statement of the hardware's behaviour either.

## What yakogl does today

- **Chain from a tiled level 0** (`generate_mipmap`,
  `src/v3d/backend.rs:2330-2440`; pack `v3d_tex_tfu_pack_mipmap`,
  `driver/src/v3d/v3d_tex.c:968-1000`, which cites `v3dx_tfu.c` at the
  older `b739817b`): ICFG format `11 + distance`, `TTYPE` RGBA8, `NUMMM =
  levels - 1`, `DIMTW` set, `IIS` the padded height in UIF blocks. It needs
  `r.stored` and a render image (`:2360`), so it exists only for a registered
  render target. `OSAL_QUERY_V3D_TFU_QUEUE` queues it behind the job in
  flight (`:2377-2395`).
- **Level 0 from a raster source** (`tfu_tile`, `src/v3d/backend.rs:4010-4053`;
  pack `v3d_tex_tfu_pack`, `v3d_tex.c:929-955`): ICFG format 0, `NUMMM` 0,
  `IIS = stride / cpp`, the destination's format and OPAD from `slices[0]`.
  The rule at `:4017` is `min == 0 || layout.size < min || layout.levels != 1
  || layers != 1` -> `NotTried`; the pack itself also refuses
  `l->levels != 1` (`v3d_tex.c:933`). The texel type is a byte copy of the
  texel width (`:4011-4016`: R8, R16F, R32F); an RGBA8 texture moves as R32F,
  channel order irrelevant. It runs on the synchronous
  `OSAL_QUERY_V3D_TFU_RUN` (`:4040-4045`).
- Callers: `copy_image_for` (`:4278`, `fill` closure `:4318-4327`) and the
  arena copy at `:4624-4627`. A chain (`layout.levels != 1`) is tiled on the
  CPU by `texture::tile` (`src/v3d/texture.rs:547-600`), one
  `v3d_tex_store_level` per level and layer.
- The host stand-in checks the same words: `tfu_run`
  (`src/osal/linux.c:1072-1120`): raster input only with `levels == 1`
  (`:1099`), tiled input only with `DIMTW` and a matching format
  (`:1100-1102`). It would refuse both new jobs below until extended.
- The GL core's CPU box filter is round half up, `(sum + n/2) / n` per channel
  and floor-sized levels (`src/gl/texture.rs:4391-4401`; test
  `generate_mipmap_box_filters_with_round_half_up`, `:9272`).
- gl_test case 140 (`driver/tools/pvrlab/gl_test.c:10738-10900`, header
  `:296`): packs a patterned raster source in V3D memory with the product's
  own `tfu_pack` per format/size/source variant, runs one polled TFU job
  (`osal_v3d_tfu`), invalidates the destination, and compares every texel
  against `v3d_tex_store_level`'s image of the same source
  (`tfu_case_compare`); verdict "Mesa's shape on every format and size: every
  job 0 and every texel equal". The `tfu_case` cases it uses are
  `tools/v3d/tfu_case.h`.

## Question 1: a chain from a raster level 0

### Design 1 (the job)

One job per chain, `TTYPE` the texture's real type (RGBA8 for RGBA8, R8 or RG8
for the narrow ones; **never** R32F/R16F, which would filter as floats):

```
IOS  = h0 << 16 | w0
IIA  = GL storage level 0 (4-byte aligned)
IIS  = pitch_texels                      (raster: texels, v3dx_tfu.c:121)
ICFG = 0 << 18 | ttype << 9 | (levels-1) << 5 | OPAD(slice 0) << 22
IOA  = image + slices[0].offset | (3 + tiling-3) << 3     (DIMTW clear)
```

It is `v3d_tex_tfu_pack` with `levels` and `NUMMM` taken from the layout, and
without the `levels != 1` refusal, plus the `DIMTW` bit left clear so level 0
goes into the copy too. A new pack beside it (`v3d_tex_tfu_pack_chain`) keeps
`v3d_tex_tfu_pack` and its tests as they are.

### Where it would be called, and what it saves

Two call sites, with very different value:

- **(a) in `tfu_tile`, for a chain** (`layout.levels > 1`). The copy is then
  made by the TFU from the GL storage's level 0 and the levels it makes. This
  is only right when the storage's levels 1.. *are* the box chain of level 0
  (a `glGenerateMipmap` result, no `glTexImage2D` of a level >= 1, no
  `glTexSubImage2D` into one). The bind carries no such bit today
  (`levels_written` and `level_defined` are on the texture, not on
  `GlesTextureBind`), so it needs a bind field set in `draw::texture_bind_2d`
  (`src/gl/draw.rs:690-697`, plan #325's file). And by the time a copy is
  made, the GL core has *already* filtered levels 1.. on the CPU (the 116 ms
  of `pi4-slow-terrain.md`), so (a) saves only the CPU tiling: **about 10 to
  12 ms of terrain's 130** (the 800x600 chain ~8-10 ms and the 256x256 chain
  ~1.5 ms, `pi4-slow-terrain.md`, "CPU loops over GPU memory") when the table
  thrashes, and 0 when it does not.
- **(b) in `generate_mipmap`, for a texture with no render image** (the
  `NotFound`/`NotRt` refusals, `backend.rs:2342-2343`). The backend would
  answer 1 ("filter nothing"): it makes the tiled copy image with a raster-
  source chain job, and the GL storage keeps its level 0 only. This removes
  the CPU filter (the 102 + 14 ms of the record) and the CPU tiling together.
  It is also what `v3d-terrain-copies.md` design 3 asks for at set-up
  ("the 13", 5 `setup_texture` calls and 3 JPEG uploads, ~4 ms-class once).
  It costs a new state: a *chain held only by the backend for a texture that
  is not a render target*. `texture.rs:4280-4290` already has
  `chain_in_backend` and the backend's `rt_chain_back` (`:2027`) for render
  images; this needs the same for copies: a CPU read or write of levels 1..
  must get the levels back by untiling the kept copy (`v3d_tex_load_box`), the
  kept copy must not be dropped as stale while the chain is the only holder
  (`tiled_copy`'s stamp-replaced path, `begin_job`, `:2571-2573`), and the
  copy needs the arena's memory kept across jobs. That is a design of its own
  and touches `src/gl/texture.rs`, `src/gl/draw.rs`, `src/v3d/backend.rs` (held
  by #310, #325, #308). I rank it below mark-clear + the render-image table
  fixes: after them neither (a) nor (b) runs per frame in terrain.

### Risk to pictures

- **Rounding.** The GL core rounds half up. The TFU's rounding is not
  documented in anything read here. The chain the TFU makes for render images
  has been accepted in terrain's and refract's pictures, so for those sizes
  the difference is within `glmark2-pictures.txt`'s tolerance; for an
  uploaded JPEG the same filter acts on photographic data, where a 1/255
  channel difference in levels 1.. is the expected size of the effect. The
  host stand-in's TFU chain (`tfu_mipmap`, `src/osal/linux.c:1039`) is a model,
  not the hardware, so it cannot answer this. `mip-multilevel-error` measures
  a related error on the host.
- **Odd sizes.** 800x600 reaches 75 rows at level 3. The GL core (and the
  existing chain job) drops the odd row (floor size, `texture.rs:4346-4347`).
  What a raster-source job does with an odd last row is exactly the thing the
  probe below checks, size by size.
- **Float and sRGB formats.** The CPU filter has sRGB and float paths
  (`texture.rs:4375-4412`); the TFU has neither. Restrict to the
  non-sRGB 8-bit `GLES_TEXFMT_RGBA8` the offer at `texture.rs:4271-4278`
  already limits itself to, with no ES 3 format (`es3_fmt == 0`).
- ES 2.0 answers: none depend on where a chain was made.
  `tests/es2_report_guard.rs` stays the check.

### The board record that confirms it

A new part in gl_test case 140, same method (a patterned raster source in V3D
memory, one polled `osal_v3d_tfu`, `0xee`-filled destination, every texel
compared against the CPU's, `tfu_flush` of the source before, `tfu_invd` of
the destination after): for each size of `tfu_case_sizes` that has more than
one level, plus 512x512 x 10, 256x256 x 9 and 800x600 x 10 (the three chains
terrain makes), build the job above and compare every level and every texel
against `texture::tile` of the CPU box chain (`v3d_tex_store_level` per
level). One line per format and size, each variant `ok`, `W<texels wrong>`
with `max |diff|`, or `E<errno>`. **The question is answered by:**

- all levels `ok` or `max |diff| <= 1` with no `E`, no GPU reset (the line
  already names them): the job is usable; `diff <= 1` goes to the picture
  tolerance (`glmark2-pictures.txt`);
- `E` or a reset, or the levels land wrong with `texel_unwritten > 0`: the
  hardware does not take it; (b) is dead and (a) is moot.

Add a timed part as case 140's part 2 does (1920x720 timed best/worst): the
800x600 chain by this job against `texture::tile` from V3D memory. A board
run, not done here.

## Question 2: `tfu_tile` one job a level

### Design 2

For each level `k` of the layout, one job exactly like `tfu_tile`'s today
with the level's numbers:

```
w_k = max(1, w >> k), h_k = max(1, h >> k)
src  = base_gpu + gl_level_layered(...)[offset] * bpp      (4-byte aligned)
IIS  = pitch_k (texels)         from gl_level_layered(b.width, b.height, 1, bpp, k)
IOS  = h_k << 16 | w_k
ICFG = 0 << 18 | ttype << 9 | 0 << 5 | OPAD(slices[k]) << 22
IOA  = image + slices[k].offset | (3 + slices[k].tiling - LT) << 3
```

with `ttype` the byte-copy width as today (R8, R16F, R32F). Each level has its
own tiling (`slices[k].tiling`: LT, UB-linear or UIF, `v3d_tex.c:239-300`),
so the destination format and OPAD are per level; they come from the layout
the CPU tiler already uses, so the CPU tiler and the TFU job write the same
bytes (`v3d_tex_store_level(image, layout, k, ...)` is the reference, as in
case 140).

What changes in the code:

- `v3d_tex_tfu_pack` takes `l->slices[0]` and refuses `levels != 1`
  (`v3d_tex.c:929-955`). A sibling `v3d_tex_tfu_pack_level(l, level, ...)`
  uses `slices[level]`, the level's minified width and height, and checks
  `ioa & 0x3f == 0` for that slice. Slice offsets are 64-byte aligned: LT
  levels need only the utile (64 B) alignment, UIF levels more
  (`v3d_tex.c:304-317`). A level whose address is not is `NotTried` and the
  CPU tiles the chain as today.
- `tfu_tile` (`backend.rs:4010`) loops the levels and layers; one level
  refused means the whole chain goes to the CPU (no mixture of CPU and TFU
  levels: the copy is one allocation, and a half-made one is not worth the
  bookkeeping). The `layout.levels != 1` and `layers != 1` limits at `:4017`
  become `levels > 16` and `layers != 1` (a cube or array is out of scope,
  as today).
- The count: `tfu_jobs` grows by `levels`, and `tfu_count` (`:4056`) gains a
  per-chain line so a board record can tell the chain jobs from the single-
  level ones. For terrain's 800x600 chain that is 10 jobs, for the 256x256
  chain 9.
- The size rule `min` (`g.tfu_min_bytes`, "64 KiB or more") applies to the
  chain's `layout.size`, not the level: small levels are tiny jobs, whose
  fixed cost is the poll and the register writes, not the bytes.
- **Run it on the queue, not on `OSAL_QUERY_V3D_TFU_RUN`.** `RUN` finishes
  the render in flight and then spins on the CPU for every job
  (`v3d-tfu-order.md`, section 1): 10 CPU spins for a chain. The copy is made
  while `copy_image_for` has already waited for the texture's own render
  (`:4298-4306`), so the source is complete, but `OSAL_QUERY_V3D_TFU_QUEUE`
  (`:2377-2395` uses it) would queue the 10 jobs behind whatever job is in
  flight with no CPU wait at all, and the next submit runs after them (the
  OSAL's `finish_inflight` runs queued jobs first, `backend.rs:2299-2310`).
  Whether the OSAL's queue holds ten more jobs (it answers `-EBUSY` when
  full, `:2390-2395`) must be read in `zephyr_v3d.c` before the change;
  otherwise `RUN` for the levels, as today's `tfu_tile`.
- The host stand-in (`linux.c:1072-1120`) rebuilds the layout from one job's
  `IOS` and the `NUMMM` of its `ICFG`, with level 0 at `IOA`. A level-`k` job
  is neither: its `IOA` is `base + slices[k].offset`, its size is the level's.
  The model has to learn the level: either a `level` carried in a field the
  struct has but a raster single-level job does not use (`OsalV3dTfu` is 40
  bytes pinned by `BUILD_ASSERT`, `zephyr_v3d.c:1318`; `ica`, `iua`, `coef0`
  are unused by a copy), or the test builds the whole job list and the model
  matches `IOA` against the layout's slices. Pick one when implementing; it
  is the main cost on the host side.

### Risk to pictures

None by construction: the destination bytes are the CPU tiler's bytes, level
by level. The risks are of another kind:

- **A level the TFU writes beyond the CPU tiler.** The existing job "writes
  the whole padded slice" (`tfu_tile`'s comment `:4009`); the kept copy's
  image holds the whole layout, so a padded slice stays inside its level.
  Levels share nothing but the page-aligned level 1 base
  (`v3d_tex.c:288-300`); the probe below checks no byte outside a level's
  slice changes.
- **Sources not visible to the TFU.** The TFU reads memory, not the CPU's
  cache. The GL storage on the Pi 4 is non-cacheable (`v3d-terrain-copies.md`
  header); the host test cannot show a cache bug. Case 140 `tfu_flush`es the
  source; the chain case does the same.
- **A level the layout would give to the TFU as UIF and OPAD wrong.**
  `v3d_tex_tfu_pack`'s OPAD uses `slices[0]`; the new function must use
  `slices[level]`. A wrong OPAD shifts the UIF block rows and produces a wrong
  picture on a UIF level only; the probe catches it per level.
- ES 2.0 answers: unchanged (the answer is a copy of the same texels).

### The board record that confirms it

Case 140's method, extended: for the chain layouts above (512x512 x 10,
256x256 x 9, 800x600 x 10, plus `tfu_case_sizes` x all levels), run the
per-level jobs into an image filled with `0xee`, compare **every level** with
`v3d_tex_store_level`'s image, and report per level: `ok`, `W<texels>`,
`E<errno>`, and the padding bytes written ("pad+N" as case 140 does). The
verdict is the same shape: every job 0 and every texel equal. Timed part:
the chain's N jobs against `texture::tile` from V3D memory, 3 runs, best
and worst in us, so the ms a frame of the change is a measurement, not an
estimate.

## Expected ms on terrain

All figures from `pi4-slow-terrain.md` (161.8 ms at the last record, the
130 ms of CPU time and its split) and `pi4-glmark2-cpu.md` (29.1 ms of GPU
work at frame 10, 4 TFU jobs 1.8 ms of it). They are arithmetic on those
tables, not new measurements.

| state | terrain, ms a frame | what is in it |
|---|---:|---|
| before `pi4-target-mark-clear` (today) | 161.8 | 116 ms CPU box filter, 10-12 ms CPU tiling, the rest waits |
| design 2 alone, before the mark-clear | about 150 | the CPU tiling of both chains becomes ~19 TFU jobs; 5 to 10 ms of GPU work added |
| design 1 (b) alone, before the mark-clear | about 45 to 50 | the filter and the tiling go; the TFU work of two chains (3 to 5 ms) added |
| after `pi4-target-mark-clear` | about 30 to 35 | 7 targets registered, 5 TFU chains, 0 `unknown`, 0 `first-sample` |
| after it, with design 1 or 2 | 30 to 35, no change | neither runs per frame: the copies they replace are gone |

So the study's value is *not* a gain on terrain once the mark-clear lands. It
is the fallback for the cases the render-image table cannot hold (more than 8
sampled targets, a texture not rendered since its registration, the TFU not
made), the 13 set-up refusals, and the first sample of an uploaded texture
chain. If the board after the mark-clear shows `unknown` or `first-sample`
still growing per frame, the table (`v3d-rt-evict`) is the first fix, and
design 2 the cheaper of these two (no picture risk, a day's work against a
new state machine).

## Board record that decides

From the `pi4-board-after` run, with no new print needed:

- `tiled copies for draws at frame N` (`first-sample`, `plain`) and `TFU
  mipmap at frame N ... unknown`: if both stop growing after terrain's frame
  1, neither design pays on terrain, and the note is for other scenes
  (Slint layers, the cluster).
- `v3d_tiling_time` (`backend.rs`, exported, read by nothing on the board
  today; `pi4-slow-terrain.md` candidate 3) per scene would show the CPU
  tiling's ms directly; it is the "before" number for design 2.
- The case 140 chain parts above, once, for the two questions as asked.

## What was not checked

- The TFU's rounding and its handling of an odd last row, from raster or
  from tiled input: no document read here states them. Board only.
- Whether the OSAL's TFU queue holds ten jobs (`zephyr_v3d.c`, the queue
  depth) and whether `RUN` after a queued job is ordered; `v3d-tfu-order.md`
  section 4 describes the queue's design, not this load.
- Memory: design 1 (b) keeps a chain in the backend only; the kept-copy budget
  (`TILE_KEPT_AT_DRAW`, `backend.rs:3986`) and the 96 MiB carve-out were not
  re-measured.
