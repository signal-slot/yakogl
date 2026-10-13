<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 45 dEQP-GLES2 texture.vertex.2d failures (host diagnosis)

**Status (2026-10-12): finished record; the fix proposed below was not
taken.** Re-landed from integration (`225d1ec8b`, ccfarm
`01a12a3d-3d08-71f6-915b-e03cdf1dcc78`). Main reached the same cause and
fixed it differently in `a6fc6f14`: `draw_dispatch` cuts an indexed draw
whose staged bytes pass half an empty GENERAL arena into index pieces
(`draw_indexed_pieces`, `src/pvr/backend.rs`), so the arena stays 1 MiB and
no extra staging memory exists. Section "The fix" describes that unwritten
alternative (a per-set `extra` arena) and is history. Its board
confirmation is HANDOFF's (image 9 of the AM62P tree). The host tests
`tests/es2_pvr_vertex_texture.rs` stay: they pin the GL core's side (the
staged bytes, the vertex-stage binding, the compiled program).

Checked again on main `40dd2107` (2026-10-12, host only): the four tests
of `tests/es2_pvr_vertex_texture.rs` pass, with and without `report-es2`,
and none is `#[ignore]`. The recorder side of the fix is pinned by the rig
test `a_deqp_vertex_texture_grid_of_client_arrays_is_drawn_in_index_pieces`
(`src/pvr/backend_tests.rs`, `--features host-test,gpu-powervr --lib`): the
same 127 x 127 grid is drawn in three index pieces, with no "not drawn"
line. It passes. The failing run at `89d518ee` in evidence 2 is from before
the fix. The board check is still open: image 9 of
`tools/cts/am62p-full-tree.txt` should give 452 passed, 0 failed.

Item `pub-license-audit` of plan #20 (the id is reused; the licence audit is
plan 18's `release-license-audit`). Host work only, at the integration tip
`89d518ee`. No board run and no source change. The fix below is for a
follow-up item.

## Fixed in

`a6fc6f14` ("pvr: an indexed draw too big for the GENERAL arena is drawn in
pieces of its index list"). `draw_dispatch` calls `draw_indexed_pieces`
(`src/pvr/backend.rs`) for such a draw. The host test is
`a_deqp_vertex_texture_grid_of_client_arrays_is_drawn_in_index_pieces`
(`src/pvr/backend_tests.rs`); it covers the 2D cell (127 x 127 quads, two
vec2 client arrays, 193,548 bytes of indices). The board check is image 9
of `tools/cts/am62p-full-tree.txt` in plan #20's AM62P final tree: expect
452 passed, 0 failed and no "does not fit an empty frame" line. This item
does not run it.

## Result in short

- **Cause: the draws are refused, not sampled wrongly.** Each cell of a
  `texture.vertex.2d` case is one `glDrawElements` of 127 x 127 quads from
  client arrays: **1,225,804 bytes** of client data (193,548 bytes of u16
  indices and two vec2 arrays of 516,128 bytes). The PowerVR back end copies
  all of it into the frame's GENERAL arena. Since the frame-set default
  (`dcaafa41`, 2026-10-04) that arena is **1,048,576 bytes**. The draw does
  not fit an empty frame, so the back end refuses it with -ENOMEM and logs
  `does not fit an empty frame ... not drawn`. The picture keeps the clear
  colour and dEQP reports "Image comparison failed". All 45 cases draw only
  such cells.
- **Why the cube cases pass:** a `texture.vertex.cube` cell has 63 x 63
  quads, 365,148 bytes. It fits.
- **Why the last clean tree passed:** at `d60b9d76` (2026-10-02, section A
  of `docs/es2-cts-failures.md`) the GENERAL arena was 4096 KiB. The
  2026-10-04 A/B that cut it to 1024 KiB ran only `vertex_arrays`, `draw`,
  `buffer` and `fragment_ops` (`driver/docs/pvr-render-pool.md:562-579`), so
  it did not see `texture.vertex`.
- **The suspect `cc78859` is not the cause.** It changed the layout of
  non-power-of-two chains only. This test's textures are 128x128. Its
  PowerVR back-end change was a comment and a test (`git show cc78859 --
  src/pvr/backend.rs`). The vertex-stage binding and the compiled vertex
  program are both right on the host (tests 3 and 4 below).
- **The fix (follow-up, `src/pvr/backend.rs`):** a draw whose staged copies
  are larger than an empty GENERAL arena gets a frame of its own and its
  own memory for the copies, as the GC7000 back end already does for this
  same dEQP grid (`src/vivante/backend.rs:2264-2286`, `ensure_extra`).
  Details in "The fix".

## What the cases do (VK-GL-CTS source)

`modules/gles2/functional/es2fVertexTextureTests.cpp` (the checkout in the
dependency cache, `VK_GL_CTS`):

- `GRID_SIZE_2D = 127`, `GRID_SIZE_CUBE = 63` (:78-79).
- `Vertex2DTextureCase::init` (:540-634): the vertex shader samples
  `texture2DLod(u_texture, a_texCoord, u_lod)`. Two textures, each 128x128
  `GL_RGB` / `GL_UNSIGNED_BYTE` with eight levels (power of two, so
  mipmapped; :585-626).
- `iterate` (:658-741): four cells (min and mag cells of each texture). Each
  cell builds a `PosTexCoordQuadGrid` of `GRID_SIZE_2D` and draws it.
- `PosTexCoordQuadGrid` (:246-354): four vertices a quad (so the texture
  coordinate is constant over a quad) and six u16 indices a quad, in client
  memory. For 127: 64,516 vertices and 96,774 indices.
- `renderCell` (:790-794): `glVertexAttribPointer` on the client arrays,
  then `glDrawElements(GL_TRIANGLES, 96774, GL_UNSIGNED_SHORT, indices)`.
  The case never calls `glGetError`, so a refused draw shows only as the
  wrong picture.
- 45 cases: `filtering` = 6 min filters x 2 mag filters x 3 wraps (36),
  `wrap` = 3 x 3 (9). Every one of them draws only these cells, so every
  one fails, whatever its filter or wrap. That matches the board record
  (`tools/cts/log/board-sessions/am62p-es2-r1c.md`): all 45, nothing else in image 9.

The same grid broke the same 45 cases on the i.MX8MP before: the GC7000's
1 MiB frame arena refused the draw (`tools/cts/RESULTS.md`, the row
"texture.vertex.2d.* (45) were REFUSED draws ... 1,225,804 bytes"), fixed
there by `7af5e841`.

## Evidence

### 1. The GL core hands the PowerVR 1,225,804 bytes to stage (host test)

`tests/es2_pvr_vertex_texture.rs` replays one cell with the case's shaders,
textures and grid, against a stand-in back end with the PowerVR's flags,
and records the `GlesDraw`.

- `a_2d_cell_stages_more_than_an_empty_general_arena_holds`:
  `index_bytes` 193,548; two client arrays (`cpu` set, so the back end
  copies them), each `components` 2, `stride` 8, `cpu_bytes` 516,128. With
  `arena_put`'s 16-byte alignment that is 1,225,808 bytes, plus the draw's
  own state. The default arena, read from `zephyr/Kconfig`
  (`YAKOGL_PVR_ARENA_GEN_KB`, default 1024), is 1,048,576 bytes.
- `a_cube_cell_fits_an_empty_general_arena`: the cube cell stages 365,148
  bytes.

### 2. The PowerVR recorder refuses such a draw (rig run)

The back end's code path, at the tip:

- `src/pvr/backend.rs:4195-4202` `draw_dispatch`: only a *sequential* draw
  longer than one u16 list is cut into pieces (`draw_chunked`). An indexed
  draw goes to `draw_part` whole.
- `src/pvr/backend.rs:4602-4610` (`record_draw`): staged indices are copied
  whole into `b.gen`.
- `src/pvr/backend.rs:4651-4673`: every staged (client) array is copied
  whole into `b.gen` (only a piece of a split sequential draw copies part).
- `src/pvr/backend.rs:4220-4260` `draw_part`: when the copies do not fit,
  the frame is submitted and the draw tried once more in a fresh frame. If
  it still does not fit, the line `pvr_backend: a draw (N vertices/indices)
  does not fit an empty frame: the general arena (B bytes) is too small;
  not drawn` is logged once, and -ENOMEM is returned (`public_rc`,
  :4205-4211).
- `src/pvr/backend.rs:265-276`: `ARENA_GEN_BYTES` = 1024 KiB, and the
  comment says the backend built 4096 KiB before the measurement.
- The library's rig already pins the cost:
  `src/pvr/backend_tests.rs:1018-1043`
  `the_default_arenas_are_1024_and_768_kib_and_bound_a_draws_client_indices`
  ("1.5 MB of client indices do not fit a 1 MiB arena ... where the 4 MiB
  arena drew it").

The dEQP-shaped draw through the real recorder. The two tests below were
added to `src/pvr/backend_tests.rs` **temporarily** (this item must not
change `src/**`; they were removed again before the commit) and run with
`cargo test --no-default-features --features host-test,gpu-powervr --lib
a_deqp_vertex_texture_2d_cell`. Output, verbatim:

```
running 2 tests
test pvr::backend::rig_tests::a_deqp_vertex_texture_2d_cell_is_drawn_at_the_default_arenas ... FAILED
test pvr::backend::rig_tests::a_deqp_vertex_texture_2d_cell_is_drawn_at_the_old_4096_kib_arena ... ok

failures:

---- pvr::backend::rig_tests::a_deqp_vertex_texture_2d_cell_is_drawn_at_the_default_arenas stdout ----

thread 'pvr::backend::rig_tests::a_deqp_vertex_texture_2d_cell_is_drawn_at_the_default_arenas' (251075) panicked at src/pvr/backend_tests.rs:4125:5:
assertion `left == right` failed: ["pvr_backend: a draw (96774 vertices/indices) does not fit an empty frame: the general arena (1048576 bytes) is too small; not drawn"]
  left: -12
 right: 0

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 1906 filtered out; finished in 0.03s
```

So at the tip the cell is refused with -ENOMEM (-12) and the exact line,
and the same cell at the old 4 x (4096 + 1024) KiB arenas is drawn. The
test text is in "The fix" below; the follow-up adds it for good.

### 3. Not the texture state

`the_vertex_stage_binding_is_the_textures_own_chain`: the vertex-stage
`GlesTextureBind` of a 2D cell is unit 0, `GLES_TEXTYPE_2D`, 128x128, stride
128, `mip_filter` 2, `mip_levels` 8, the cell's wraps, `incomplete` 0. Each
level's first texel, read at the offset the TPU walks to (levels back to
back, 4 bytes a texel), holds that level's data. The PowerVR builds the
vertex stage's descriptors with the same `build_desc_block` /
`build_tex_desc` as the fragment stage's (`src/pvr/backend.rs:5605-5626`
and `:5023-5029`), so there is no separate vertex-stage packing to differ.
`cc78859`'s non-power-of-two layout does not apply to a 128x128 texture.

### 4. Not the compiler

`the_vertex_program_samples_at_u_lod_and_a_texcoord_on_the_usc_model`: the
vertex program of the draw (the online compiler's output, as linked) runs on
the host USC model (`tests/usc_exec`). For `u_lod` 1.5, -2.0 and 0.25 the
`smp` takes `u_lod` (dword 20, after the 20-dword descriptor) as its
explicit level, reads `a_texCoord` from its vertex input registers, and
`v_color` is the model's texel there. Also in a `report-es2` build.

### Commands

```
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_pvr_vertex_texture
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_pvr_vertex_texture
```

Both: `test result: ok. 4 passed; 0 failed; 0 ignored`. The file has no
`#[ignore]` test: every test in it pins the GL core's side, which is right
and which the fix does not change. The failing test belongs to the
library's rig (`src/pvr/backend_tests.rs`, `cfg(test)` of the library),
because only there does the real recorder run on the host. Its failure at
the tip is shown in evidence 2.

## The fix (follow-up item, `src/pvr/backend.rs`)

Give a draw whose staged copies do not fit an empty GENERAL arena its own
memory for them, for the life of its kick, as the GC7000 back end does
(`src/vivante/backend.rs:2264-2286` and `ensure_extra` at `:2377-2400`).
The arena then bounds how many draws share one kick, not how big one draw
may be. Changes:

1. `src/pvr/backend.rs:1142-1157` `struct FrameSet`: add `extra:
   OsalGpuMem` and `extra_used: u32` (a GENERAL-heap block, empty by
   default).
2. A new `fn staged_need(d: &GlesDraw) -> u64` next to `draw_part`: the sum,
   each rounded up to 16, of `d.index_bytes` (when `d.index_cpu` is set) and
   of `cpu_bytes` of every attribute with `cpu` set. This is what
   `record_draw` copies (`:4602-4610`, `:4651-4673`).
3. `src/pvr/backend.rs:4220` `draw_part`, before `record_draw_txn`: when
   `staged_need(d) + GEN_PER_DRAW_MAX as u64 > p.b.gen.mem.size as u64`:
   - if the frame holds application draws, `flush(p)` and `frame_begin(p,
     d)` (the draw starts a frame of its own, as `:4227-4231` does);
   - make the current set's `extra` hold `staged_need(d)` bytes: keep it if
     it is big enough, otherwise `osal::gpu_free` it and `osal::gpu_alloc`
     a new one in the GENERAL heap (the set's fence was waited for at
     `:3556`, so the GPU no longer reads the old one);
   - bound it by `ARENA_GEN_MAX` (`:276`, 4096 KiB, the arena every draw had
     before `dcaafa41`). A larger draw, or a failed allocation, is refused
     as today, by name ("... does not fit an empty frame: the draw's own
     staging (N bytes) ...").
4. `src/pvr/backend.rs:4602-4610` and `:4651-4673`: put the staged copies
   with a small helper `stage_put(b, src, len, out_gpu)` that writes into
   `extra` when the frame has one and into `b.gen` otherwise. The other
   per-draw words (descriptors, PPP, uniforms) stay in `b.gen`.
5. `src/pvr/backend.rs:3549-3568` (`frame_begin_with`, the set's reuse):
   reset `extra_used` to 0. `src/pvr/backend.rs:4519-4530`
   `record_draw_txn`: save and restore `extra_used` with the other cursors,
   so a failed draw leaves no trace.
6. `src/pvr/backend.rs:2501-2514` (`pvr_backend_shutdown`): free each set's
   `extra`. `:6543-6590` (`pvr_backend_pool_owners`): count it under the
   frame sets.
7. A cache flush of `extra` before the kick, as for the arenas.

Rejected:

- Raise `ARENA_GEN_BYTES` / `YAKOGL_PVR_ARENA_GEN_KB` to 2048 KiB: it costs
  4 x 1 MiB of the 64 MiB carveout for good. That carveout is already short
  (refract's depth buffer, `pvr-render-pool.md` "Next"). And the next
  larger draw fails again.
- Set `CONFIG_YAKOGL_PVR_ARENA_GEN_KB=4096` in the dEQP images only: the
  conformance image would then differ from the product, and applications
  with the same draw shape would still fail.

### Tests the fix flips or must update

Add to `src/pvr/backend_tests.rs` (the text that failed at the tip above):

```rust
/// One cell of dEQP-GLES2 texture.vertex.2d as the GL core hands it over
/// (tests/es2_pvr_vertex_texture.rs): 127 x 127 quads, four vertices a quad,
/// two staged vec2 arrays of 516,128 bytes and 193,548 bytes of staged u16
/// indices, 1,225,804 bytes in all (tools/cts/log/investigations/es2-pvr-vertex-texture.md).
fn deqp_vertex_2d_cell(rig: &Rig, pos: &[f32], tc: &[f32], idx: &[u16]) -> GlesDraw {
    let mut d = rig.draw(3);
    d.index_cpu = idx.as_ptr() as *const c_void;
    d.index_bytes = (idx.len() * 2) as u32;
    d.index_count = idx.len() as u32;
    for (i, a) in [pos, tc].into_iter().enumerate() {
        d.attribs[i] = GlesAttrib {
            vi_reg: 4 * i as u8,
            components: 2,
            stride: 8,
            cpu: a.as_ptr() as *const c_void,
            cpu_bytes: (a.len() * 4) as u32,
            ..GlesAttrib::ZERO
        };
    }
    d.num_attribs = 2;
    d
}

/// The 45 AM62P texture.vertex.2d failures: such a cell must be drawn at
/// the default arenas.
#[test]
fn a_deqp_vertex_texture_2d_cell_is_drawn_at_the_default_arenas() {
    let rig = Rig::new();
    let verts = 127 * 127 * 4;
    let (pos, tc) = (vec![0.0f32; verts * 2], vec![0.5f32; verts * 2]);
    let idx: Vec<u16> = (0..127 * 127 * 6).map(|i| (i % verts) as u16).collect();
    let d = deqp_vertex_2d_cell(&rig, &pos, &tc, &idx);
    assert_eq!(d.index_bytes as usize + 2 * verts * 8, 1_225_804);
    let _ = rig.logs();
    let rc = draw(&d);
    let logs = rig.logs();
    assert_eq!(rc, 0, "{logs:?}");
}
```

It flips from FAILED (-12) to ok. The follow-up should also check that the
copies are in the extra block (the streams' addresses lie outside
`gen.mem`), that a second such draw in the same frame set reuses the block,
and that `pvr_backend_shutdown` frees it (the rig's live-allocation balance,
as `two_small_frame_sets_are_what_the_backend_builds_and_rotates` checks).

Existing rig tests whose assertions encode "refused" and change with the
4096 KiB bound in step 3:

- `src/pvr/backend_tests.rs:1018-1043`
  `the_default_arenas_are_1024_and_768_kib_and_bound_a_draws_client_indices`:
  1.5 MB of client indices are then drawn, not -ENOMEM. Keep the arena
  sizes assertion; change the second half to "drawn, in a frame of its own".
- `src/pvr/backend_tests.rs:996-1016`
  `a_smaller_general_arena_refuses_a_draw_it_cannot_hold_and_names_it`:
  20,000 bytes at a 16 KiB arena are then drawn. To keep testing the
  refusal line, use a draw above 4096 KiB.
- `src/pvr/backend_tests.rs:1243-1269`
  `a_draw_larger_than_an_empty_frame_is_out_of_memory_and_leaves_the_frame_alone`:
  6,000,000 bytes stay above the bound, so it stays refused; only the
  logged line's wording may change.
- `tests/es2_pvr_vertex_texture.rs`: no change (it pins the GL core's draw,
  which keeps its size).

### Board check (after the fix)

AM62P, **image 9** of `tools/cts/am62p-full-tree.txt` (the row
`texture.mipmap.*, texture.specification.*, texture.vertex.*,
texture.wrap.*`; `TREE_ROWS=8`, 0-based), built at the fix's commit and run
with `tools/board.sh am62p tree` as in `am62p-es2-r1c.md`:

- Pass: `deqp: done: 452 passed, 0 failed` (r1c: 407 passed, 45 failed),
  every `texture.vertex.2d.*` case `Pass`, `texture.vertex.cube.*` still 45
  pass.
- The console has no `does not fit an empty frame` line.
- Images 8 and 10-17 are not affected by the change for any draw that
  fitted before; one gl_test run (default list, 48 of 48) checks the
  common draw path.

Optional confirmation of the diagnosis before the fix, at no code change:
the same image 9 at the tip built with `CONFIG_YAKOGL_PVR_ARENA_GEN_KB=4096`
should pass all 45. That is a diagnosis A/B only, not the fix. Without a
board, the kept console of r1c's `img2-r1c` (image 9) should show exactly
one line `pvr_backend: a draw (96774 vertices/indices) does not fit an empty
frame: the general arena (1048576 bytes) is too small; not drawn` (the line
is logged once per boot). That console was not available on this worker.

## Not determined here

- The board: no run in this item. The refusal on silicon is inferred from
  the recorder's code and its host rig, which runs the same Rust recorder.
- Whether other dEQP-GLES2 cases draw more than 1 MiB of client data. The
  other AM62P round-1 failures are the 9 `shaders.invariance` cases
  ("Detected variance", `am62p-plan1.md`) and the 2 ms4 `multisample`
  cases, which do not look like refused draws. The 2026-10-04 A/B found
  none in `vertex_arrays`, `draw` and `buffer`. The fix covers any such
  draw up to 4096 KiB anyway.
