<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 flat shading: the provoking vertex on the PowerVR, V3D and GC7000UL

Design, no code. The rows are the nine
`dEQP-GLES3.functional.rasterization.flatshading.*` cases: `lines`,
`line_strip`, `line_loop`, their `_wide` forms, `triangles`,
`triangle_strip` and `triangle_fan`. All nine fail on the AM62P whole tree at
`ad2b5ed` with "Found invalid pixel values"
(`docs/es3/am62p-gles3-notpass.tsv:1568-1576`). This note gives, per back end
and per topology, what yakogl does today, what Mesa 26.2.1 does, the cause,
the fix and the host test, and lists the checks only a board can make. Every
path a fix touches is held by another plan. The fix items name them and wait.

## 1. What GL and dEQP ask for

- ES 3.0 2.18 ("Flatshading") and table 2.12: a `flat` varying takes its
  value from the provoking vertex, which is the last vertex of the primitive.
  Numbering vertices and primitives from 1:

  | mode | provoking vertex of primitive i |
  |---|---|
  | `GL_LINES` | 2i (the line's second vertex) |
  | `GL_LINE_STRIP` | i + 1 (the segment's second vertex) |
  | `GL_LINE_LOOP` | i + 1, except the closing segment (n, 1), whose provoking vertex is vertex **1** |
  | `GL_TRIANGLES` | 3i |
  | `GL_TRIANGLE_STRIP` | i + 2 |
  | `GL_TRIANGLE_FAN` | i + 2 (not the hub, vertex 1) |

- In 0-based terms, the provoking vertex is the primitive's last vertex in
  GL order. For a strip triangle with odd i the order is (i+1, i, i+2), for
  a fan it is (0, k, k+1), and the loop's closing segment is (n-1, 0), so its
  provoking vertex is vertex 0. `restart_to_lists` (`src/gl/draw.rs:4719-4797`)
  writes primitive-restart pieces in this order, so the restart path keeps
  the provoking vertex.
- dEQP's flatshading cases draw with a `flat out`/`flat in` colour, one
  colour per vertex. Each primitive must be one solid colour: the colour of
  its last vertex. The `_wide` forms draw the same lines at a width above 1.
  The GL core reports `GL_ALIASED_LINE_WIDTH_RANGE` [1, 16] for every back end
  (`src/gl/state.rs:993-997`), so the cases run rather than reporting
  NotSupported. (The dEQP source is not on this host. The expectation above
  is the spec's table. A board run is the check.)
- The GL core does not convert any mode. `map_prim` (`src/gl/draw.rs:1317-1328`)
  hands all seven modes to the back end as they are, and each back end draws
  them natively (sections 2-4). The core does no line-loop closing, no fan
  split and no wide-line expansion. A wide line is a width the back end
  programs into its rasteriser. So the provoking vertex is a back-end
  register question on every topology. The GL core needs no change.

## 2. PowerVR (AM62P, BXS-4-64), plan #325, `src/pvr/**`

**Today (main since `7b39917c`, 2026-11-09).** Three words choose the vertex.
All three say VERTEX_2 for every topology:

- `TA_STATE_PPP_CTRL.flatshade_vtx = PVR_TA_FLATSHADE_VERTEX_2` (`src/pvr/backend.rs:8223`).
- `VDM_STATE0.flatshade_control = PVR_VDM_FLATSHADE_CONTROL_VERTEX_2` (`src/pvr/backend.rs:8468`).
- The DOUTI shademodel `FLAT_VERTEX2` of each flat varying in the coefficient
  PDS program (`driver/src/pvr_pds_render_gen.c:373`).

Topologies are native (`src/pvr/backend.rs:7255-7263`): LINE_LIST,
LINE_STRIP, LINE_LOOP, TRI_LIST, TRI_STRIP, TRI_FAN, with TA object type LINE
for the three line modes. A wide line is only `ISPA.pointlinewidth` (4.4 fixed
point, `src/pvr/backend.rs:8010-8024`). Nothing expands it into triangles in
software. A long draw is cut into pieces by `cut_unit` (`src/pvr/backend.rs:6579-6588`).
A line strip repeats one vertex, so each segment keeps its own second vertex.
Fans and loops are never cut.

**Mesa 26.2.1** (`src/imagination/vulkan/pvr_arch_cmd_buffer.c`, in the deps
cache `mesa-pan/mesa-26.2.1`). With `VK_PROVOKING_VERTEX_MODE_LAST_VERTEX_EXT`,
Mesa writes VERTEX_2 to all three words for every topology, line lists and
line strips included: DOUTI at `:7289-7290`, PPP at `:7515-7516`, VDM at
`:8161-8162`. VERTEX_1 is used only for a triangle fan in first-vertex mode
(`:7291-7292`, `:7517-7518`, `:8163-8164`). The device reports
`provokingVertexLast = true` (`pvr_physical_device.c:453`). So in Mesa's
design, VERTEX_2 means "the primitive's last vertex" for a line too, where
that vertex is vertex 1. The field is not a literal vertex index.

**Cause at `ad2b5ed`.** All three words said VERTEX_0, the first vertex,
which is D3D's convention. That explains all nine rows. The smooth line rows
of the same run pass (`rasterization.primitives.lines`,
`interpolation.basic.line_loop` are not in the not-pass list, and
`interpolation.*.lines` are only QualityWarning), so line geometry, loops
and widths are right. Only the flat value is wrong.

**Per topology after `7b39917c`:**

| row | expected after `7b39917c` | why |
|---|---|---|
| `triangles`, `triangle_strip`, `triangle_fan` | Pass | VERTEX_2 is Mesa's last-vertex mode for every triangle topology |
| `lines`, `line_strip` | Pass | Mesa uses the same VERTEX_2 for its line lists and strips |
| `line_loop` | open: **board** | Mesa never draws a LINE_LOOP topology (Vulkan has none). Whether the hardware gives the closing segment (n-1, 0) vertex 0 under VERTEX_2 is not documented anywhere we can read |
| `lines_wide`, `line_strip_wide`, `line_loop_wide` | open: **board** | the TA widens the line itself. Mesa's Vulkan has no wide lines on this part, so no Mesa path shows that the provoking vertex survives the widening |

**Fix.** None known to be needed. The design is Mesa's. If the board shows
`line_loop` failing only on its closing segment, the fallback is in the GL
core and keeps the back end as it is: draw a loop with flat inputs as an
indexed `GL_LINES` list (i, i+1) plus (n-1, 0), as `restart_to_lists` already
writes it. That would edit `src/gl/draw.rs`, held by #335 and #339. If the
board shows the wide lines failing, there is no register fallback in Mesa.
That becomes a new item (wide lines as triangles in the back end,
`src/pvr/backend.rs`, #325).

**Host test** (an addition to `tests/es3_pvr_flatshade.rs` under #325, or a
new `tests/es3_flatshade_pvr.rs` if #325 prefers). It checks what that file
does not check today:

1. `GL_LINE_LOOP` gives topology `PVR_VDM_TOPOLOGY_LINE_LOOP` with PPP and
   VDM VERTEX_2. `words_of`/`check_prim` cover five modes; the loop is
   missing.
2. A wide line (`glLineWidth(5)`) on `GL_LINES`, `GL_LINE_STRIP` and
   `GL_LINE_LOOP` keeps VERTEX_2 and writes `point_line_width` 79
   (5 * 16 - 1).
3. A line-loop draw longer than one piece is not cut (`cut_unit` (0, 0)), so
   the closing segment stays in the draw.

A host test cannot show which vertex the TA picks. That is the board check.

## 3. V3D (Pi 4, V3D 4.2), plan #330, `src/v3d/**`, `driver/src/v3d/`

**Today.**

- Per-varying flat flags. The online compiler sets bit i of
  `flat_shade_flags` for each flat fragment input component
  (`src/shaderc/online/v3d/chain.rs:414-431`). The job builder emits them as
  `FLAT_SHADE_FLAGS` packets (`driver/src/v3d/v3d_job.c:733-735`, macro at
  `:475`). The frame path (`driver/src/v3d/v3d_frame.c:596-599`) only handles
  draws with no flat input.
- Provoking vertex. `CFG_BITS` is written in `emit_cfg_bits`
  (`driver/src/v3d/v3d_job.c:547-567`) and in `v3d_frame.c:526-534`. Neither
  sets `direct3d_provoking_vertex`, so the bit (`v3d_packet.xml:832`, bit 21;
  `driver/src/v3d/generated/v3d_packet_v42_pack.h:2831`) stays 0, which is
  GL's last vertex.
- Topologies are native (`src/v3d/state.rs:216-227`, including
  `V3D_PRIM_LINE_LOOP` and `TRIANGLE_FAN`). Wide lines are the `LINE_WIDTH`
  packet from the draw's width (`src/v3d/state.rs:555`,
  `driver/src/v3d/v3d_job.c:682-684`).

**Mesa 26.2.1.** `config.direct3d_provoking_vertex =
rasterizer->base.flatshade_first` (`src/gallium/drivers/v3d/v3dx_emit.c:336-337`),
which is 0 for GL. The flags come from `prog_data.fs->flat_shade_flags`
through `emit_varying_flags`/`emit_flat_shade_flags` (`v3dx_emit.c:127`,
`:178`, `:621-627`). Mesa's design is the same as ours.

**Board evidence.** All nine rows **pass** on the Pi 4 at `0ff58329`
(image R1, `tools/cts/log/2026-10-31-pi4-gles3-a/R1.console.txt:437-478`),
and none is in `docs/es3/pi4-gles3-fails.tsv`.

**Cause.** None. The V3D has no flatshading failure.

**Fix.** None. Item `fix-flat-v3d` reduces to a regression guard, a host
test only.

**Host test** (new `tests/es3_flatshade_v3d.rs`. `tests/es3_*_v3d.rs` is
held by #330, so it waits for #330 or is written under it). For each of the
seven modes, a `flat out vec4` program drawn through the real V3D back end
on the host rig must give:

1. A `CFG_BITS` with bit 21 (`direct3d_provoking_vertex`) clear.
2. A `FLAT_SHADE_FLAGS` packet whose word-0 mask is the flat components.
3. The native primitive (`V3D_PRIM_LINE_LOOP` for a loop, `TRIANGLE_FAN`
   for a fan).
4. For `glLineWidth(5)`, `LINE_WIDTH` 5.0.

`src/shaderc/online/v3d/chain.rs:3256-3300` already proves the flag mask in
the compiler, so the new test covers only the job-stream half.

## 4. GC7000UL (i.MX8MP), plans #333/#328/#335, `src/vivante/**`

**Today.**

- `PA_SYSTEM_MODE` carries `PROVOKING_VERTEX_LAST` on every draw
  (`src/vivante/backend.rs:1372-1380`).
- `PA_CONFIG.SHADE_MODEL` is FLAT when any fragment input is flat
  (`flat_shade_model`, `src/vivante/backend.rs:537-545`, `:1384-1389`).
  The comment there records why: PROVOKING_VERTEX_LAST applies only under the
  flat model, and under the smooth model a flat varying takes the first
  vertex.
- Each component of a flat input is marked `GC_INTERP_FLAT` (2) in
  `GL_HALTI5_SHADER_ATTRIBUTES` (`src/vivante/shader.rs:268-270`, `:377-378`),
  so the smooth inputs of the same program still interpolate.
- Topologies are native (`prim_type`, `src/vivante/backend.rs:1207-1217`,
  including `GC_PRIM_LINE_LOOP` and `GC_PRIM_TRIANGLE_FAN`).
- Wide lines are native: `PA_CONFIG.WIDE_LINE` is always set, and
  `PA_LINE_WIDTH`, `PA_WIDE_LINE_WIDTH0/1` get half the width
  (`src/vivante/backend.rs:1370`, `:1389`, `:1438-1439`). The profile has
  `wideline_triangle_emu: false` and a range of [1, 16]
  (`src/vivante/profile.rs:117`, `:128-130`).

**Mesa 26.2.1** (etnaviv, deps cache):

- `PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST` unless `flatshade_first`
  (`etnaviv_rasterizer.c:59-61`).
- SHADE_MODEL from `rasterizer->flatshade` (`:48`), and switched to FLAT when
  the linked shader has any flat varying (`etnaviv_shader.c:260-267`).
- Per-varying interpolation from the NIR variable (`etnaviv_compiler_nir.c:1474-1486`).
- `WIDE_LINE` when the part has the feature (`etnaviv_rasterizer.c:53`).
- LINE_LOOP native when `ETNA_FEATURE_LINE_LOOP` is present (`etnaviv_screen.c:344-345`).
- Max line width 8192 unless `WIDELINE_TRIANGLE_EMU` (`:366-368`).

The NXP hwdb row `gc7000ULN_v123` has `REG_WideLine` 1, `REG_LineLoop` 1,
`PA_WIDELINE_FIX` 1 and `WIDELINE_TRIANGLE_EMU` 0
(`src/etnaviv/hwdb/nxp/gc_feature_database.h:19044`, `:19047`, `:19268`,
`:19297`). So Mesa takes the same native paths on this part. Mesa never
rewrites indices to move the provoking vertex: the GC takes the last vertex
in hardware.

**Board evidence.** Triangles only. gl_test case 89 `flatlast` (one triangle
with a flat vec4 and a flat uint) is recorded as measured on silicon
(`src/vivante/backend_tests.rs:1615-1627`). The i.MX8MP dEQP-GLES3 whole tree
has not run (#316), so no flatshading row has a GC verdict.

**Cause.** None known. The register design equals etnaviv's for every
topology.

| row | expected | open question for the board |
|---|---|---|
| `triangles` | Pass | none (case 89) |
| `triangle_strip`, `triangle_fan` | Pass | that PROVOKING_VERTEX_LAST picks i+2 and not the fan hub on this part (Mesa relies on it, no measurement here) |
| `lines`, `line_strip` | Pass | that the flat model applies to line primitives |
| `line_loop` | Pass | the closing segment's provoking vertex (vertex 0) with the native LINE_LOOP |
| `*_wide` | Pass | that the native wide-line path (PA_WIDELINE_FIX part) keeps the flat value |

**Fix.** None. Item `fix-flat-gc` reduces to a regression guard plus the
board rows. If the board fails `line_loop` only, the fallback is the same
GL-core loop-as-`GL_LINES` rewrite as in section 2 (`src/gl/draw.rs`, #335).
If it fails wide lines, the fallback is the triangle path Mesa uses on
`WIDELINE_TRIANGLE_EMU` parts (`src/vivante/**`, #333/#328/#335).

**Host test** (new `tests/es3_flatshade_gc.rs`; the GC tests and
`src/vivante/**` are held by #333/#328/#335). Today
`a_flat_varying_selects_the_flat_shade_model` and
`the_provoking_vertex_is_the_last_one` (`src/vivante/backend_tests.rs:1615`,
`:1663`) test a triangle list only. For each of the seven modes, a real
ES 3.00 `flat out vec4` program compiled by the online compiler and drawn
through the real GC back end must give:

1. `PA_SYSTEM_MODE` with PROVOKING_VERTEX_LAST.
2. `PA_CONFIG` with SHADE_MODEL_FLAT (and SMOOTH for the same program without
   `flat`).
3. `HALTI5_SHADER_ATTRIBUTES` nibbles of 2 for the flat components.
4. The native primitive type in the draw command.
5. For `glLineWidth(5)`, `PA_CONFIG.WIDE_LINE` and 2.5 in
   `PA_WIDE_LINE_WIDTH0/1`.

## 5. Board checks no host test replaces

Board runs are for #313 (AM62P), #320 (Pi 4) and #316 (i.MX8MP), not this plan.

1. **AM62P**, at a commit that contains `7b39917c`: the nine
   `rasterization.flatshading.*` rows. Expected 9 Pass. If `line_loop` or a
   `_wide` row fails while `lines` passes, use the fallbacks in section 2.
2. **i.MX8MP**: the same nine rows (in the whole tree #316 runs, or alone),
   plus gl_test 89. Expected 9 Pass.
3. **Pi 4**: none new. The nine rows passed at `0ff58329`. They are rerun
   with the whole tree.

A gl_test case could make these cheaper (one image, no dEQP). One flat colour
per vertex, drawn as a 4-vertex `GL_LINE_LOOP` at width 1 and width 5, with
one pixel read in the middle of each segment. The closing segment must be
vertex 0's colour. That belongs to the plan's `gltest-cases` item.

## 6. Paths and holders

| change | path | holder |
|---|---|---|
| PowerVR loop/wide host test | `tests/es3_pvr_flatshade.rs` or new `tests/es3_flatshade_pvr.rs` | #325 |
| PowerVR wide-line fallback (only if the board fails) | `src/pvr/backend.rs` | #325 |
| V3D regression guard | new `tests/es3_flatshade_v3d.rs` | #330 (`tests/es3_*_v3d.rs`) |
| GC regression guard | new `tests/es3_flatshade_gc.rs` | #333/#328/#335 (GC tests) |
| GC wide-line fallback (only if the board fails) | `src/vivante/backend.rs` | #333/#328/#335 |
| Line-loop-as-lines fallback, any back end (only if the board fails) | `src/gl/draw.rs` | #335, #339 |
| inventory row (`flatshading.*` 9: "PowerVR VERTEX_0, fixed `7b39917c`; V3D pass; GC board") | `docs/es3-cts-failures.md` | #330 (draft in the plan's `drafts` item) |
