<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1: line and point rasterization as Mesa states it, and what the 7.1 path sets

Scope: the fields that decide how a GL ES 2 line or point is rasterized on
V3D 7.1 (`LINE_WIDTH`, `POINT_SIZE`, `CFG_BITS` line rasterization,
`CLIPPER_XY_SCALING`, the shader record's point-size flag, the sprite
coordinate origin), what Mesa 26.2.1's v3d driver sets for them, and what
`draw71`/`prim71` and the C job set today. Every difference is named as a
gap (`R1`...). Read only: no code changes follow from this note by itself.

Citation prefixes:

* `X:` `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml` (vendored).
* `L:` `third_party/mesa-26.2.1/src/broadcom/common/v3d_limits.h` (vendored).
* `G:` the gate's full Mesa checkout, tag `mesa-26.2.1` (the `gallium/drivers/v3d`,
  `mesa/state_tracker` and `broadcom/compiler` sources are not vendored; the
  line numbers are that tag's). Paths are relative to its `src/`.
* repository paths as written.

## 1. The hardware fields

| Field | Packet (code) | Layout, 4.2 and 7.1 | Source |
|---|---|---|---|
| `Point Size` | `POINT_SIZE` (104) | one float, 32 bits; no `min_ver`/`max_ver`: same at 4.2 and 7.1 | `X:886-888` |
| `Line width` | `LINE_WIDTH` (105) | one float, 32 bits; same at both | `X:890-892` |
| `Line Rasterization` | `CFG_BITS` (96) | **4.2:** 2 bits at 4-5, enum `Line Rasterization` (0 diamond exit, 1 perp end caps). **7.1:** 1 bit at 4 (type `uint`); bit 5 is `Depth Bounds Test Enable`, bit 4 keeps the same meaning (0 diamond exit, 1 perpendicular end caps) | `X:317-320`, `X:842` (4.2), `X:861` (7.1) |
| `Viewport Half-Width/Height` | `CLIPPER_XY_SCALING` (110) | **4.2:** in 1/256th of a pixel; **7.1:** in 1/64th. Same opcode, same 8-byte body, different unit | `X:919-927`; granularity `G:broadcom/common/v3d_device_info.c:87,91` |
| `Point size in shaded vertex data` | GL Shader State Record, bit 0 | the vertex shader's output word carries gl_PointSize (the VPM header slot after Zs and 1/Wc) | `X:1198` (4.2 record), `X:1257` (7.1), `X:1315` (7.1 draw-index record); slot `G:broadcom/compiler/v3d_nir_lower_io.c:427-428` |
| limits | | `V3D_MAX_POINT_SIZE` 512.0, `V3D_MAX_LINE_WIDTH` 32 (version-independent) | `L:46-47` |

There is **no** hardware point-sprite-origin bit and no hardware `gl_PointCoord`
flip: point coordinates are two fragment varyings the compiler asks for
(`point_x`, `point_y`, section 2.4). The 7.1 `CFG_BITS` differs from 4.2 for
rasterization only in the 1-bit line rasterization field (above).

## 2. What Mesa sets (GL ES 2)

### 2.1 Line rasterization mode (diamond exit versus perpendicular end caps)

`CFG_BITS.line_rasterization` is `PERP_END_CAPS` when
`v3d_line_smoothing_enabled()` is true, else `DIAMOND_EXIT`
(`G:gallium/drivers/v3d/v3dx_emit.c:369-372`, "nicer line caps when line
smoothing is enabled"). Smoothing is on when the rasterizer has `line_smooth`,
the job is not MSAA, the rasterizer is not multisample, there is a colour
buffer 0 and it is not a pure integer format
(`G:gallium/drivers/v3d/v3d_context.c:177-203`). `line_smooth` comes from
`GL_LINE_SMOOTH` without multisampling (`G:mesa/state_tracker/st_atom_rasterizer.c:242-243`).

GL ES 2 has no `GL_LINE_SMOOTH`: the core ES 2 enable list does not include
it, so for ES 2 contexts the rasterizer's `line_smooth` is 0 and the mode is
always **diamond exit**. Perpendicular end caps matter only to desktop GL
(and to ES contexts that expose a line-smooth extension).

### 2.2 Wide lines

* `LINE_WIDTH` is emitted when the rasterizer state is dirty, with
  `v3d_get_real_line_width()` (`G:gallium/drivers/v3d/v3dx_emit.c:440-442`).
  Without smoothing the value is the rasterizer's `line_width`; with smoothing
  it is `floorf(sqrt(2) * width) + 3` (`G:gallium/drivers/v3d/v3d_context.c:206-219`).
* The gallium `line_width` is the GL width clamped to
  `[MinLineWidth, MaxLineWidth]` (the AA range when smoothing)
  (`G:mesa/state_tracker/st_atom_rasterizer.c:243-250`). The screen reports min 1,
  granularity 0.1, max `V3D_MAX_LINE_WIDTH` = 32 for both ranges
  (`G:gallium/drivers/v3d/v3d_screen.c:374-383`, `L:47`).
* There is no software wide-line path in v3d: the hardware draws the wide line
  as a rectangle of the packet's width (the driver never converts a wide line
  to triangles).
* Culling never discards lines or points: `Enable Forward Facing Primitive` is
  forced on for `LINES`/`POINTS` reduced primitives
  (`G:gallium/drivers/v3d/v3dx_emit.c:300-312`).

### 2.3 Point size and its clamp

* `POINT_SIZE` is emitted when the rasterizer is dirty, and on **4.2** with
  every draw (a binner FIFO workaround, `G:gallium/drivers/v3d/v3dx_emit.c:424-437`;
  the 7.1 value is therefore emitted on rasterizer changes only). Its value is
  `rasterizer->point_size`.
* That value is `MAX2(cso->point_size, .125f)` (workaround HW-2726, the PTB
  does not handle zero-size points; `G:gallium/drivers/v3d/v3dx_state.c:90-93`).
* `cso->point_size` is `ctx->Point.Size`; only when the vertex shader does
  **not** write the point size is it clamped to `[MinSize, MaxSize]`
  (`G:mesa/state_tracker/st_atom_rasterizer.c:197,232-238`). With a per-vertex
  size the packet carries the unclamped `Point.Size` and the shader's value is
  what the hardware uses.
* The point-size flag of the shader record is
  `mode == POINTS && rasterizer->point_size_per_vertex`
  (`G:gallium/drivers/v3d/v3dx_draw.c:479-481` and `:556-558`, the two record
  layouts). Mesa compiles the vertex shader with `per_vertex_point_size` from
  the same condition (`G:gallium/drivers/v3d/v3d_program.c:783-785`,
  `:864-866`; `G:broadcom/compiler/vir.c:1111`). So a program drawn as lines or
  triangles is a **variant without** the point-size output.
* The screen's limits: min point size 1, granularity 0.1, max
  `V3D_MAX_POINT_SIZE` = 512 (`G:gallium/drivers/v3d/v3d_screen.c:374-386`).
  The compiler does not clamp a shader-written gl_PointSize; the hardware does
  not either at the packet level (nothing in `X` states a clamp).

ES 2 has no `glPointSize`: the fixed size is 1.0 and the size comes from
`gl_PointSize` (the context's `Point.Size` defaults to 1.0).

### 2.4 Sprite coordinate origin and point coordinates

* In ES 2, `gl_PointCoord` is always available and the origin is the upper
  left. Gallium's `sprite_coord_mode` is `UPPER_LEFT` when
  `(SpriteOrigin == GL_UPPER_LEFT) ^ (fb_orientation == Y_0_BOTTOM)`
  (`G:mesa/state_tracker/st_atom_rasterizer.c:205-211`); `sprite_coord_enable`
  is `CoordReplace` (plus the PNTC generic slot when the target lacks TGSI
  texcoord, `:217-223`).
* v3d takes both into the fragment key only for point draws:
  `point_sprite_mask = sprite_coord_enable` and
  `point_coord_upper_left = (sprite_coord_mode == UPPER_LEFT)`
  (`G:gallium/drivers/v3d/v3d_program.c:704-716`). The key's `is_points` and
  `is_lines` come from the draw's mode (`:603-608`).
* The compiler then reads `point_x`/`point_y` as two extra fragment varyings
  (`G:broadcom/compiler/nir_to_vir.c:4869-4872`) and sets
  `uses_implicit_point_line_varyings` (the 7.1 shader record bit "Disable
  implicit point/line varyings" is the inverse of it). A `GL_COORD_REPLACE`
  varying gets `(point_x, upper_left ? 1 - point_y : point_y, 0, 1)`
  (`G:broadcom/compiler/nir_to_vir.c:2512-2535`); `gl_PointCoord` itself is
  fixed by `lower_wpos_pntc` before that (comment at `:2516-2519`).
* Lines with a line-coordinate system value get one varying `line_x`
  (`G:broadcom/compiler/nir_to_vir.c:4873-4878`); with smoothing the fragment
  shader is also lowered by `v3d_nir_lower_line_smooth` for lines
  (`G:broadcom/compiler/vir.c:1282-1286`).

### 2.5 The clipper scale

`CLIPPER_XY_SCALING` is emitted on a viewport change with the viewport's
half-extent times 256.0 (4.2) or 64.0 (7.1)
(`G:gallium/drivers/v3d/v3dx_emit.c:445-461`). Lines and points are clipped
against the same guardband as triangles: wide lines and large points rely on
the guardband, not on a separate clip rule in the driver.

## 3. What the 7.1 path sets today

| Field | Where | Value | Same as Mesa for ES 2? |
|---|---|---|---|
| `Line Rasterization` | `src/v3d/draw71.rs:1479` (the `CfgBits` built in the state function); the C job `driver/src/v3d/v3d_job71.c:487` | 0 (diamond exit), always | yes (ES 2 has no smoothing; **R1**: no perpendicular mode is ever set) |
| `Line Rasterization` field width | `src/v3d/draw71.rs:154` (`fl("Line Rasterization", 4, 1, "uint")`), `:654-655` | 1 bit at 4, as `X:861` | yes |
| `LINE_WIDTH` spec and packet struct | `src/v3d/draw71.rs:285-291`, `:1005-1025` | 32-bit float, code 105 | yes (the spec exists and is in the spec list, `:320`) |
| `LINE_WIDTH` value | `src/v3d/glue71.rs:649` (GL's width) -> `src/v3d/fill71.rs:733` -> `driver/src/v3d/v3d_job71.c:508-509` (`st->line_width`) | the GL width as given, no clamp | **R2** (see below) |
| `POINT_SIZE` spec and packet struct | `src/v3d/draw71.rs:277-283`, `:983-1003` | 32-bit float, code 104 | yes |
| `POINT_SIZE` value | `driver/src/v3d/v3d_job71.c:504-507` | constant 1.0, "ES has no glPointSize" | value yes; **R3**: emitted once per job state, not clamped to `.125`; fine for 1.0 |
| shader record point-size flag | `src/v3d/record71.rs:171,225,496`, `src/v3d/backend71.rs:252,383`, `src/v3d/table71.rs:682`, `driver/src/v3d/v3d_job71.c:690` (`vs_writes_psiz`) | set iff the vertex program writes gl_PointSize, **for every primitive** | **R4** |
| `CLIPPER_XY_SCALING` | `src/v3d/draw71.rs:246-255` (spec: "1/64th"), `:1504` (`scale * 64.0`); `driver/src/v3d/v3d_job71.c:28,341-344,519-521` (`CLIPPER_XY_GRANULARITY 64.0f`) | scale x 64 | yes |
| `Enable Forward Facing Primitive` for points/lines | `src/v3d/draw71.rs:1483-1484`: `!(d.polygon && cull_front)`; the C job `driver/src/v3d/v3d_job71.c:480` | forced on unless the draw is a polygon | yes (`reduced_prim` test in Mesa) |
| sprite origin / `gl_PointCoord` | no field; `src/shaderc/link.rs:362-367` links `gl_PointCoord` as a fragment input without a vertex output; the 7.1 compile path has no `point_x`/`point_y` varyings and no `point_coord_upper_left` | none set | **R5** |
| `prim71` | `src/v3d/prim71.rs:49-52` (`GL_POINTS`, `GL_LINES`, `GL_LINE_LOOP`, `GL_LINE_STRIP`), `:24-25` (mode number = `Primitive` enum) | packs the draw packets only; **no** point or line state | not its job; **R6** (line loops are not converted, see below) |

### Gaps

* **R1, perpendicular end caps.** Never set (`draw71.rs:1479`,
  `v3d_job71.c:487`). Not needed for ES 2 (no `GL_LINE_SMOOTH`); needed for
  desktop-GL smoothing or an ES context with it, together with the line-width
  rule `floor(sqrt(2) * w) + 3` and `v3d_nir_lower_line_smooth` in the
  shader. A later 7.1 item that wants smoothing needs all three.
* **R2, the line width is unclamped and passes straight through.** Mesa
  clamps the GL width to the screen range (1 to 32, `G:...v3d_screen.c:374-383`)
  in the state tracker before the packet. Nothing in `glue71`/`fill71`/the C job
  clamps (`glue71.rs:649`). ES 2 lets `glLineWidth(w)` take any `w > 0`; `src/gl/state.rs:797` stores the width as given (its comment
  "the ISP takes up to 16 px" is the PowerVR limit, not V3D's) and
  `src/gl/draw.rs:5287` copies it into the draw. A width above 32 reaches the packet as is. The
  clamp belongs to the GL layer or the glue (src/gl is held by plan #397).
* **R3, `POINT_SIZE` is a constant 1.0.** It equals Mesa's value for ES 2 when
  the shader writes the size (the packet's value is then unused), and for a
  point draw without gl_PointSize it is the fixed size. It does not copy
  `MAX2(size, .125)` (not needed at 1.0).
* **R4, one program serves every primitive.** The point-size flag follows the
  program, not the draw mode. Mesa compiles a variant without the flag for
  lines and triangles (`G:...v3dx_draw.c:479-481`; see section 2.3). Known as
  `gap_psiz_all_prims` in `tests/v3d71_glue_prims.rs:54,694-695`. What a
  record that carries the flag does on a line or triangle draw is not
  measured on the Pi 5 (open).
* **R5, `gl_PointCoord` and the sprite origin are not handled in the 7.1
  compiler path.** Mesa adds two fragment varyings (`point_x`, `point_y`) for a
  point draw that reads `gl_PointCoord` and flips Y for an upper-left origin
  (section 2.4). No such variant exists in this tree's 7.1 compile path (only
  the Vivante link table knows `gl_PointCoord`, `src/shaderc/link.rs:362-367`).
  A shader that reads `gl_PointCoord` (glmark2's point scenes, dEQP
  `rasterization.*point*` with sprites) is therefore a refusal or wrong on 7.1
  until the 7.1 compile path takes `is_points` and `point_coord_upper_left`
  into its key and adds the two varyings (a `src/shaderc` item; plan #400).
* **R6, no mode-dependent state in `prim71`.** `GL_LINE_LOOP` is a hardware
  primitive (`Primitive` enum, `prim71.rs:49-52`); the draw packet is the only
  line-specific output. Line width and point size are not derived from the
  draw's mode: `line_width` is emitted for every draw. (Mesa emits both only
  when the rasterizer is dirty; 7.1 does not need the 4.2 FIFO workaround, so
  an unconditional pair is equivalent on state, only larger. Not a defect.)

What is the same: `Point Size`/`Line width` packet layouts (shared by 4.2 and
7.1), the 1-bit line rasterization field and its 0 meaning, the 1/64 clipper
unit, and the line/point culling rule.

## 4. How the rows are tested today

`tests/v3d71_glue_prims.rs` compares glue71's points and lines draws with
Mesa's `gl_prims`/`gl_primswide` clifs (`POINT_SIZE` 1.0, `LINE_WIDTH`,
the record's flag) and pins R4 as `gap_psiz_all_prims`;
`tests/v3d71_compose5.rs` checks the `CfgBits` and clipper values. No test
exercises R1, R2 or R5.
