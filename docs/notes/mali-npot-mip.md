# Mali: the mip chain of a non-power-of-two image (D-NPOTMIP)

glmark2's refract scene builds its colour texture at twice the canvas
(1600x1200 at the default 800x600: both sides NPOT) with
`GL_LINEAR_MIPMAP_LINEAR` and `glGenerateMipmap`. Until this change the GL
core laid no chain out for the Mali, so the texture kept one level and was
sampled without a mip filter; panfrost gives it the full chain of 11 levels
(floor rule: 1600, 800, 400, 200, 100, 50, 25, 12, 6, 3, 1 wide).

## What code relies on

- `npot_mip_chains(layers)` in `src/gl/texture.rs` is true for a single image
  (`layers <= 1`) on every back end, and for stacked layers only where the back
  end walks them (`native_layered_textures`). The PowerVR keeps its own
  power-of-two-extent layout (`pot_chain_layout`, `lvl_align == 0`); the V3D and
  GC7000 were already true. Only the Mali (`mip_level_align` 64, not layered)
  changes.
- `chain_layout_npot` lays such a chain out as a power-of-two chain does for a
  back end with `lvl_align != 0`: level l is `max(w >> l, 1)` x `max(h >> l, 1)`,
  every level on level 0's pitch and on a 64-byte boundary.
- The Mali samples that storage in place, one plane per level, with
  `level_at` (`src/mali/backend.rs`): it never needed a power-of-two check for
  a single image. `texture_of` therefore refuses an NPOT chain only for a cube
  map or a 2D array (`NOT_BUILT_TEXTURE_NPOT_CHAIN`, "cube map or 2D array"):
  their stacked faces have no per-level arithmetic here, and the GL core keeps
  one level for them.
- The A and B halves must land together: the core's chain alone would make the
  Mali refuse refract's texture.

## Test

`tests/mali_npot_mip.rs` describes the draw's binding with
`yakogl::mali::backend::host_texture_of` (host-test only; `mali_draw` returns
`-ENODEV` on the host before it builds a texture). `tests/mali_npot_mip_gl.rs`
pins what reaches the back end.

On the board refract still needs its shader compile refusals cleared before a
run shows the chain.
