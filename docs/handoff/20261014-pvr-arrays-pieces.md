<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: glDrawArrays of client arrays cut into pieces, 2026-10-14, formats-pvr

## Known defects

- **AM62P GLES3 image 7**: fixed on the host, waiting for the board to confirm.
  The 24,384-vertex `glDrawArrays` of
  `shaders.builtin_functions.precision.clamp.highp_fragment.vec3` has four
  client arrays: a vec4 position and three vec3 inputs, 52 bytes a vertex,
  1.27 MB. Until now it was refused whole ("does not fit an empty frame: the
  general arena (1048576 bytes) is too small"), because only an indexed draw
  was cut by its staged size. Now `draw_dispatch` (`src/pvr/backend.rs`)
  sends every draw whose staged data is more than half an empty GENERAL arena
  into pieces. An indexed draw goes to `draw_indexed_pieces`, as before. A
  sequential draw of at most 65,535 vertices goes to the new
  `draw_sequential_pieces`. Each piece is a run of vertices that reads the
  sequential index list from its first vertex, so its indices are absolute
  and gl_VertexID stays right. The piece copies only its own window of each
  staged array (`record_draw`'s `win`). VBO-resident attributes are fetched
  as for the whole draw.
  - Cut rule: `cut_unit`, now shared by all three cutters. Triangles cut at a
    multiple of 3. Strips repeat 2 vertices and keep an even number of
    triangles per piece. Lines cut at 2. Line strips repeat 1. A sequential
    piece must also start on an **even** vertex, because
    `VDMCTRL_INDEX_LIST1` has no address bits 1:0 (`index_base_addrlsb` must
    be 0). So a triangle list steps by 6, and a line strip or a point list
    by 2. The first host run without this rule refused every odd start by
    name (283 ResourceErrors). The rig's fake index-list emitter
    (`src/pvr/backend_tests.rs`) now refuses a misaligned address as the
    real one does.
  - Fans, line loops, draws with a second index list, and draws one step of
    which is more than half an arena still go whole and are refused by name
    (`-ENOMEM`, the same "does not fit an empty frame" line). Past
    `MAX_PIECES` (64) a draw is `-E2BIG` with its own line.
  - Draws over 65,535 vertices still take `draw_chunked`, unchanged.
  - Host evidence (HOST-ONLY; `tools/cts/host-deqp-run.sh --module=gles3
    --gpu=powervr 'dEQP-GLES3.functional.shaders.builtin_functions.precision.'`,
    1,147 cases):
    - before, 79c73ea2: 48 ResourceError, 130 Pass, 915 QualityWarning, 54 Fail
    - after: 0 ResourceError, 138 Pass, 955 QualityWarning, 54 Fail
    - after again on main 641c8b2f plus this change (2026-10-15): the same,
      1,147 of 1,147 cases began, 0 ResourceError
    - The 54 Fail are the `GL_COLOR_ATTACHMENT3` / `CAP_COLOR_ATTACHMENTS` 3
      cause in `docs/es3-host-run.md`. They are unrelated and unchanged.
  - Host tests, `src/pvr/backend_tests.rs`:
    - `a_24384_vertex_draw_arrays_of_client_arrays_is_drawn_in_pieces`:
      3 pieces of whole triangles, every vertex once, each index fetching
      its own data from all four streams.
    - `a_sequential_strip_or_line_draw_of_client_arrays_is_cut_on_primitive_boundaries`
    - `a_sequential_draw_reading_gl_vertexid_is_cut_with_absolute_indices`
    - `a_sequential_fan_or_loop_of_client_arrays_too_large_for_the_arena_is_refused_by_name`
    - `a_fitting_draw_records_the_same_frame_as_before_the_sequential_pieces`:
      four fitting draws whose frame fingerprints (VDM words, index lists,
      streams, and the arena bytes) were taken on 79c73ea2's `backend.rs`.
  - Behaviour change: a sequential draw of 512 KiB to 1 MiB of staged data
    used to fit an empty frame whole. Now it is drawn as pieces. Indexed
    draws had the same threshold already. The result is the same; only the
    command stream differs.

## Open work

- formats-pvr Part B, **RGB10_A2 sampled in its own class on the PowerVR**,
  is not done. The TPU half has been in place since 25267d17: `es3_tex_format`
  maps `GLES_TEXFMT_RGB10A2` to `A2R10B10G10` with PCK_INFO 0x148, pinned in
  the `R10G10B10A2_UNORM` row of the texture-format test in
  `src/pvr/backend.rs`. Adding `GLES_TEXFMT_RGB10A2` to
  `PVR_SAMPLED_TEXFMTS` alone would regress ES 3.0 behaviour, because
  GL_RGB10_A2 is colour-renderable and mipmappable and the class would be
  neither. In order:
  1. A render-target class for the 10/10/10/2 unorm word. There is
     `GLES_SURF_RGB10_A2UI` (24) but no unorm surface in `src/backend.rs`.
     Its PowerVR PBE/accumulation format (A2R10B10G10 packmode, F16 accum,
     the RGBA16F shader output) is in `docs/es3-pvr-formats.md` §2.5, plus a
     host test of its PBE words.
  2. GL core, `src/gl/framebuffer.rs` texture-attachment guard (about line
     430): accept `GLES_TEXFMT_RGB10A2` (today the guard takes only RGBA8,
     sRGB, RGBA16F, R16F and R32F, and `t.es3_fmt != 0 && t.texfmt !=
     RGBA8` is refused), and map its internalformat to GL_RGB10_A2.
  3. GL core, `glGenerateMipmap` (`src/gl/texture.rs`, about line 3745):
     allow GL_RGB10_A2 in the `RGB10A2` class, with a CPU box filter for the
     packed word (the backend's `YakoglBackend::generate_mipmap` entry,
     `src/backend.rs`, is `None` on the PowerVR, so the CPU filter is the
     path there). glCopyTexImage into the class and glReadPixels from it go
     with this.
  4. Then add the bit to `PVR_SAMPLED_TEXFMTS` and update the comment
     there, `docs/es3-sized-formats.md` and `docs/es3-pvr-formats.md` §10.1
     ("Deviation: RGB10_A2 stays in RGBA8").

  Steps 2 and 3 are in GL-core files that plan #2 also touches
  (`src/gl/texture.rs`), so they should go behind its merges.

## Next board session

- AM62P: re-run image 7 (`TREE_ROWS=6`, line 14 of
  `tools/cts/am62p-gles3-tree.txt`). It should now run to its end and give
  its 1,397 missing cases a result. Check the shell log for any
  "does not fit an empty frame" or `index_base_addrlsb` line.
