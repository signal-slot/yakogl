//! V3D 7.1 limits and tile-size choice (pure, no `crate::` imports so a test
//! can include it with `#[path]`). Every figure follows the vendored Mesa
//! 26.2.1 (`third_party/mesa-26.2.1/src/broadcom/...`), the `ver >= 71` branch.
//! Mesa's `v3d_screen.c` is not vendored; the texture figures are derived
//! from `v3d_limits.h`, which that file reads.

/// Render targets: `V3D_MAX_RENDER_TARGETS(ver)` = `ver < 71 ? 4 : 8`
/// (`common/v3d_limits.h:44`), stored by `common/v3d_device_info.c:82`.
pub const MAX_RENDER_TARGETS: u32 = 8;

/// Largest target a side: `V3D_MAX_FRAMEBUFFER_SIZE(ver)` = `ver < 71 ?
/// 4096u : 7680u` (`common/v3d_limits.h:63`), stored by
/// `common/v3d_device_info.c:83`.
pub const MAX_TARGET: u32 = 7680;

/// Largest image a side: `V3D_MAX_IMAGE_DIMENSION` (`common/v3d_limits.h:62`),
/// not version dependent; the HW could do 16384 but Mesa caps the mip chain
/// at `V3D_MAX_MIP_LEVELS` = 13 (`v3d_limits.h:69`), i.e. `1 << 12`.
pub const MAX_IMAGE_DIMENSION: u32 = 4096;

/// `V3D_MAX_MIP_LEVELS` (`common/v3d_limits.h:69`).
pub const MAX_MIP_LEVELS: u32 = 13;

/// GL_MAX_TEXTURE_SIZE: `V3D_MAX_IMAGE_DIMENSION` (`v3d_limits.h:62`).
pub const GL_MAX_TEXTURE_SIZE: u32 = MAX_IMAGE_DIMENSION;

/// GL_MAX_RENDERBUFFER_SIZE: a renderbuffer is an image, so the same
/// `V3D_MAX_IMAGE_DIMENSION` (`v3d_limits.h:62`), not the 7680 target bound.
pub const GL_MAX_RENDERBUFFER_SIZE: u32 = MAX_IMAGE_DIMENSION;

/// GL_MAX_VIEWPORT_DIMS: kept at the image bound (`v3d_limits.h:62`) so a
/// viewport never exceeds what a texture or renderbuffer can back; the
/// 7680 px target bound (`MAX_TARGET`) is only the Frame limit. Decision, not
/// a Mesa figure (v3d_screen.c is not vendored).
pub const GL_MAX_VIEWPORT_DIMS: u32 = MAX_IMAGE_DIMENSION;

/// `V3D_MAX_DRAW_BUFFERS` (`common/v3d_limits.h:43`): GL_MAX_DRAW_BUFFERS.
pub const MAX_DRAW_BUFFERS: u32 = 8;

/// `V3D_MAX_SAMPLES` (`common/v3d_limits.h:41`).
pub const MAX_SAMPLES: u32 = 4;

/// `V3D71_TLB_COLOR_SIZE` (`common/v3d_util.c:101`).
pub const TLB_COLOR_SIZE: u32 = 16 * 1024;
/// `V3D71_TLB_DETPH_SIZE` (`common/v3d_util.c:102`, sic).
pub const TLB_DEPTH_SIZE: u32 = 16 * 1024;
/// `V3D71_TLB_AUX_DETPH_SIZE` (`common/v3d_util.c:103`).
pub const TLB_AUX_DEPTH_SIZE: u32 = 8 * 1024;

/// `tile_sizes[]` of `v3d_choose_tile_size` (`common/v3d_util.c:133-141`),
/// width, height pairs from the largest.
pub const TILE_SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];

/// `tile_size_valid` (`common/v3d_util.c:106-119`).
pub const fn tile_size_valid(pixel_count: u32, color_bpp: u32, depth_bpp: u32) -> bool {
    // v3d_util.c:111-114: depth in the aux buffer, colour in colour + depth.
    if pixel_count * depth_bpp <= TLB_AUX_DEPTH_SIZE && pixel_count * color_bpp <= TLB_COLOR_SIZE + TLB_DEPTH_SIZE {
        return true;
    }
    // v3d_util.c:117-118: the main buffers.
    pixel_count * depth_bpp <= TLB_DEPTH_SIZE && pixel_count * color_bpp <= TLB_COLOR_SIZE
}

/// Tile size for V3D 7.1: `v3d_choose_tile_size` `ver >= 71` branch
/// (`common/v3d_util.c:144-180`). `total_color_bpp` is the sum of the bytes
/// per pixel of every colour attachment (the 7.x argument, `:126`); the
/// render-target count and the 4.x max internal bpp are not inputs on 7.1.
/// MSAA multiplies colour and depth by 4 (`:169-170`); the first valid size
/// wins (`:171-177`); double buffer steps one size down (`:179-180`), and the
/// result must stay inside the table (`:202`, `assert`) -- `None` otherwise.
pub const fn choose_tile_size(total_color_bpp: u32, msaa: bool, double_buffer: bool) -> Option<(u32, u32)> {
    let m = if msaa { 4 } else { 1 };
    let color_bpp = total_color_bpp * m; // :169
    let depth_bpp = 4 * m; // :170
    let mut idx = 0;
    // do { ... idx++ } while (idx < 7): idx may reach 7 when nothing fits.
    while idx < TILE_SIZES.len() {
        let (w, h) = TILE_SIZES[idx];
        if tile_size_valid(w * h, color_bpp, depth_bpp) {
            break;
        }
        idx += 1;
    }
    if double_buffer {
        idx += 1; // :180
    }
    if idx < TILE_SIZES.len() {
        Some(TILE_SIZES[idx])
    } else {
        None
    }
}

/// The most tiles one job may bin at 7.1's `MAX_TARGET`, sized like the 4.2
/// `MAX_JOB_TILES` (`src/v3d/backend.rs`: the target square at four 32-bpp
/// render targets, no MSAA, single buffer): that is 16 bytes of colour, the
/// 32x32 tile of `choose_tile_size(16, false, false)`.
pub const MAX_JOB_TILES: u32 = (MAX_TARGET / 32) * (MAX_TARGET / 32);

/// Tiles of a `w` x `h` target at `tile` (rounded up, as the Frame's tile
/// counts are).
pub const fn tiles(w: u32, h: u32, tile: (u32, u32)) -> (u32, u32) {
    (w.div_ceil(tile.0), h.div_ceil(tile.1))
}
