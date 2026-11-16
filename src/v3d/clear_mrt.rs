// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The clear quad's fragment program for several colour attachments: a
//! scissored or masked colour clear of up to four render targets of mixed
//! float and integer classes, in one quad.
//!
//! The one-target programs, `backend.rs` `CLEAR_FS` and `CLEAR_FS_INT`, write
//! RT 0 only (the back end once refused such a clear of several attachments
//! by name). [`clear_mrt_source`] writes the GLSL ES 3.00 fragment shader
//! that clears a given set of them; it links with `backend.rs` `CLEAR_VS_INT`
//! (the position passed through, in the same language), duplicated here as
//! [`CLEAR_VS`]. Building and drawing it is the back end's
//! (`backend.rs` `mrt_clear_program`, one program per class pattern, and
//! `draw_clear_quad`; docs/notes/v3d-mrt-clear.md section 0); this module is
//! the source and what the caller fills.
//!
//! Per cleared target `n`, in RT order:
//!
//! - a float target: `uniform vec4 u_color<n>` and
//!   `layout(location = n) out vec4`, which the compiler writes as two f16
//!   pairs (`select.rs` `tlb_config_f16`: the TLB's default config for RT 0
//!   with no TLBU write, an explicit one naming the RT otherwise);
//! - an integer target, signed or not: `uniform uvec4 u_color<n>` and
//!   `layout(location = n) out uvec4`, the four raw words of
//!   glClearBuffer{i,ui}v as `CLEAR_FS_INT` writes them (`select.rs`
//!   `tlb_config_32b`, then four 32-bit words that the tile buffer narrows
//!   to the target's internal type). One output type serves both
//!   signednesses: the 32-bit TLB type carries none (Mesa 26.2.1 writes
//!   every `f32_color_rb` target with TLB_TYPE_F32_COLOR,
//!   nir_to_vir.c:1963-1977; docs/notes/v3d-int-clear.md).
//!
//! A target that is not cleared has no output and no uniform. Mesa's
//! `vir_emit_tlb_color_write` (nir_to_vir.c, called once per RT from
//! `emit_frag_end`) emits nothing for an RT the shader has no colour output
//! for, and the TLB then keeps that RT's tile-buffer contents: the colour
//! the tile load brought in, or an earlier draw's. So an attachment that
//! the clear leaves alone (a draw buffer of GL_NONE, or one whose
//! glClearBuffer is not this one) keeps its colour under the quad. The
//! colour mask is not the program's: the job's COLOR_WRITE_MASKS packet
//! (v3d_packet.xml code 87, four disable bits per RT; Mesa v3dx_emit.c
//! translates GL's mask per RT) applies it to every RT the quad writes, and
//! ES 3.0 has one colour mask for all draw buffers. nir_to_vir.c and
//! v3dx_emit.c are not in third_party/; the line numbers are those the
//! compiler's `select.rs` and docs/notes/v3d-int-clear.md cite.
//!
//! The uniforms are declared in RT order, each a vec4 of the fragment
//! block (std430, `reflect.rs`), so the k-th cleared target's colour is the
//! block's dwords `4k..4k + 3` ([`ClearUniform::dword_off`]). A float
//! target's dwords are its clear colour's f32 bits; an integer target's the
//! raw clear words (`GlesDraw::clear_rgba` holds them as f32 bit patterns).
//!
//! A float output always writes f16 pairs: the V3D program is not keyed by
//! the target's internal type. A 32F target (R32F) that must keep a full
//! 32-bit clear value can be named [`ClearClass::Uint`] with its float's
//! bits as the words, which the 32-bit TLB write stores as they are.

/// The clear quad's vertex shader, `backend.rs` `CLEAR_VS_INT` byte for
/// byte (a test in tests/v3d_clear_mrt.rs holds them equal).
pub const CLEAR_VS: &[u8] = b"#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n\0";

/// The most colour attachments one clear quad writes (the V3D 4.2's four
/// render targets, `V3D_MAX_DRAW_BUFFERS`).
pub const MAX_RTS: usize = 4;

/// What one render target is to the clear.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClearClass {
    /// A float or normalized fixed-point target: a `vec4` output.
    Float,
    /// An unsigned integer target: a `uvec4` output of the raw words.
    Uint,
    /// A signed integer target: the same `uvec4` output as [`Self::Uint`].
    Int,
    /// Not cleared: no output, the tile buffer keeps its colour.
    NotCleared,
}

impl ClearClass {
    /// Does the program write this target with 32-bit TLB words?
    pub const fn is_integer(self) -> bool {
        matches!(self, ClearClass::Uint | ClearClass::Int)
    }
}

/// The uniform the caller fills for one cleared target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClearUniform {
    /// The render target (the output's location).
    pub rt: u8,
    /// The uniform's name in the source, `u_color<rt>`.
    pub name: &'static str,
    /// Its first dword in the fragment block; it takes four.
    pub dword_off: u16,
    /// A `uvec4` of raw words, not a `vec4`.
    pub integer: bool,
}

/// Why no program was written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClearMrtError {
    /// More than [`MAX_RTS`] targets.
    TooManyTargets,
    /// No target is cleared: the quad would draw nothing.
    NothingCleared,
}

/// The longest source [`clear_mrt_source`] writes: the header and four
/// integer targets come to well under it.
const SRC_MAX: usize = 512;

const NAMES: [&str; MAX_RTS] = ["u_color0", "u_color1", "u_color2", "u_color3"];

/// The clear program for one set of targets: its fragment source and the
/// uniforms it reads.
#[derive(Clone, Copy)]
pub struct ClearMrt {
    src: [u8; SRC_MAX],
    len: usize,
    uniforms: [ClearUniform; MAX_RTS],
    cleared: usize,
}

impl ClearMrt {
    /// The fragment shader, NUL-terminated (the shader back end's
    /// `compile` takes a C string).
    pub fn source(&self) -> &[u8] {
        &self.src[..self.len]
    }

    /// The uniforms, one per cleared target, in RT order (and block order).
    pub fn uniforms(&self) -> &[ClearUniform] {
        &self.uniforms[..self.cleared]
    }

    /// The fragment block's size in dwords: four per cleared target.
    pub fn block_dwords(&self) -> u16 {
        4 * self.cleared as u16
    }

    /// Bit n: the program writes render target n (the compiled program's
    /// `rt_mask` must be the same).
    pub fn rt_mask(&self) -> u8 {
        self.uniforms().iter().fold(0, |m, u| m | 1 << u.rt)
    }
}

/// A fixed buffer the source is written into.
struct Writer {
    buf: [u8; SRC_MAX],
    len: usize,
}

impl Writer {
    fn put(&mut self, s: &[u8]) {
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
    }
}

/// The GLSL ES 3.00 fragment source of a clear quad for `rts` (RT 0 first),
/// and the uniforms the caller fills. For example `[Float, NotCleared,
/// Int]` gives
///
/// ```text
/// #version 300 es
/// precision highp float;
/// precision highp int;
/// uniform vec4 u_color0;
/// uniform uvec4 u_color2;
/// layout(location = 0) out vec4 o0;
/// layout(location = 2) out uvec4 o2;
/// void main() { o0 = u_color0; o2 = u_color2; }
/// ```
pub fn clear_mrt_source(rts: &[ClearClass]) -> Result<ClearMrt, ClearMrtError> {
    if rts.len() > MAX_RTS {
        return Err(ClearMrtError::TooManyTargets);
    }
    let mut uniforms = [ClearUniform { rt: 0, name: "", dword_off: 0, integer: false }; MAX_RTS];
    let mut cleared = 0;
    for (rt, &c) in rts.iter().enumerate() {
        if c != ClearClass::NotCleared {
            uniforms[cleared] =
                ClearUniform { rt: rt as u8, name: NAMES[rt], dword_off: 4 * cleared as u16, integer: c.is_integer() };
            cleared += 1;
        }
    }
    if cleared == 0 {
        return Err(ClearMrtError::NothingCleared);
    }
    let us = &uniforms[..cleared];
    let ty = |u: &ClearUniform| if u.integer { &b"uvec4 "[..] } else { &b"vec4 "[..] };
    let digit = |u: &ClearUniform| [b'0' + u.rt];
    let mut w = Writer { buf: [0; SRC_MAX], len: 0 };
    w.put(b"#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for u in us {
        w.put(b"uniform ");
        w.put(ty(u));
        w.put(u.name.as_bytes());
        w.put(b";\n");
    }
    for u in us {
        w.put(b"layout(location = ");
        w.put(&digit(u));
        w.put(b") out ");
        w.put(ty(u));
        w.put(b"o");
        w.put(&digit(u));
        w.put(b";\n");
    }
    w.put(b"void main() {");
    for u in us {
        w.put(b" o");
        w.put(&digit(u));
        w.put(b" = ");
        w.put(u.name.as_bytes());
        w.put(b";");
    }
    w.put(b" }\n\0");
    Ok(ClearMrt { src: w.buf, len: w.len, uniforms, cleared })
}

/// The clear-colour words of a float render target in the render list
/// (plan #360 v3d-clear-words): what Mesa 26.2.1's `v3d_tlb_clear`
/// (v3dx_draw.c:1706-1769) puts in `job->clear_color[rt]` for a full-tile
/// clear, and `v3dx_rcl.c:833-860` emits as `TILE_RENDERING_MODE_CFG`'s
/// clear-colour parts: PART1 the low 32 bits and the next 24, PART2 from
/// 64 bpp, PART3 at 128 bpp. `internal_type` and `internal_bpp` are
/// `ffi::V3D_INTERNAL_TYPE_*` and `ffi::V3D_INTERNAL_BPP_*` (the target's
/// `state::rt_format` row); `rgba` is the glClear / glClearBufferfv colour,
/// not clamped (the clear colour bypasses the TLB's clamp, Mesa's comment
/// at v3dx_draw.c:1702-1712).
///
/// Mesa copies `internal_size = 4 << internal_bpp` bytes
/// (v3dx_draw.c:1704; docs/notes/v3d-msaa-es3.md), so one word reaches a
/// 32-bpp target, two a 64-bpp one, four a 128-bpp one, and the words past
/// them stay 0 (the job is zeroed).
///
/// - `V3D_INTERNAL_TYPE_16F`: `util_pack_color(color,
///   PIPE_FORMAT_R16G16B16A16_FLOAT)` (v3dx_draw.c:1739-1743), each channel
///   a half (`_mesa_float_to_half`, [`super::state::f32_to_f16`]: nearest
///   even, 65504 the largest, overflow to infinity, the sign kept), two a
///   word, r in the low half. Every 16F target is 64 bpp in Mesa: RGBA16F
///   (v3dx_format_table.c:100, :254-260), R16F and RG16F (:90, :95,
///   :263-269, 64 bpp so the TLB keeps alpha), and R11F_G11F_B10F:
///   PIPE_FORMAT_R11G11B10_FLOAT has the output format R11F_G11F_B10F
///   (:146), whose tile buffer is 16F at 64 bpp (:253, :259-260) like
///   RGBA16F. Its clear is four halves, alpha included; the narrowing to
///   11/11/10 bits is the TLB store's, not the clear's.
/// - `V3D_INTERNAL_TYPE_32F`: the colour's f32 bits as they are (the 32-bit
///   types' `memcpy(job->clear_color[rt], color->ui, internal_size)` in
///   v3dx_draw.c:1706-1769): R32F (v3dx_format_table.c:91, :327-329,
///   32 bpp) word 0, RG32F (:96, :323-325, 64 bpp) words 0 and 1, RGBA32F
///   (:101, :319-321, 128 bpp) all four.
///
/// Any other internal type is not a float target: `[0; 4]`. No float
/// format's swizzle puts blue first (SWIZ_X001, SWIZ_XY01, SWIZ_XYZ1 and
/// SWIZ_XYZW, v3dx_format_table.c:90-101, :146), so Mesa's red/blue swap
/// of the clear (`job->swap_color_rb`) never applies here.
///
/// Today's back end packs the clear in `state::clear_words` (by surface
/// class, called from `backend.rs`'s clear path): for 16F (R16F, RGBA16F)
/// it gives these words; for R32F word 0 is the same but words 1..3 carry
/// the other channels' bits, which Mesa leaves 0 (and which PART1's next
/// 24 bits then carry for a 32-bpp target). It has no RG32F, RGBA32F or
/// R11F_G11F_B10F row.
pub fn clear_color_words(internal_type: u32, internal_bpp: u32, rgba: [f32; 4]) -> [u32; 4] {
    use super::ffi::{V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_TYPE_32F};
    let mut out = match internal_type {
        V3D_INTERNAL_TYPE_16F => {
            let h = rgba.map(|x| u32::from(super::state::f32_to_f16(x)));
            [h[0] | h[1] << 16, h[2] | h[3] << 16, 0, 0]
        }
        V3D_INTERNAL_TYPE_32F => rgba.map(f32::to_bits),
        _ => return [0; 4],
    };
    /* internal_size = 4 << bpp bytes: 1, 2 or 4 words */
    let words = 1usize << internal_bpp.min(2);
    for w in out.iter_mut().skip(words) {
        *w = 0;
    }
    out
}
