// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Shader back-end interface (`include/gles_shaderc.h`): GLSL ES -> PowerVR
//! USC + the wiring metadata the GPU backend needs.
//!
//! Two back ends: [`online`], our own compiler, and [`offline`], which looks a
//! shader up by the SHA-256 of its GLSL in the Mesa reference table. The
//! structs are `#[repr(C)]`: they are the shader ABI shared with C consumers
//! and the generated tables.
//!
//! The Mesa reference ([`mesa_reference`]) is what Mesa's compilers made of
//! the source list, kept as host-test material in `tests/reference/`:
//! `blobs.rs`, the PowerVR one (`yakogl-tools glsl2usc`), and
//! `vivante_blobs.rs`, the GC7000 one (`tools/vivante/gen_shader_table.py`).
//! Exactly one GPU's is compiled, and only into the host tests or a build that
//! asks for it as its table (`mesa-reference-table`, the measurement images):
//! no other product build links a Mesa-built row.

use core::ffi::{c_char, c_void};

#[cfg(all(test, mesa_reference))]
mod es100_twins;
pub mod link;

/* The Mesa reference table, under `cfg(mesa_reference)`, which build.rs sets
 * for `mesa-reference-table` and for `host-test` without `no-shader-table`.
 * It is the largest single thing the library could carry (6.9 MB of PowerVR
 * instruction words, 258,000 bytes of GC7000) and nothing links it out on
 * its own, because `g_shaderc`'s initialiser below names the offline back
 * end: so it is not compiled at all where it is not asked for. */
/// The Mesa reference table of this build's GPU (`tests/reference/`): the
/// PowerVR one from pco_shc, or the GC7000 one from etna_shc.
#[cfg(all(mesa_reference, not(feature = "gpu-vivante")))]
#[path = "../../tests/reference/blobs.rs"]
pub mod mesa_reference;
/// The Mesa reference table of this build's GPU (`tests/reference/`): the
/// PowerVR one from pco_shc, or the GC7000 one from etna_shc.
#[cfg(all(mesa_reference, feature = "gpu-vivante"))]
#[path = "../../tests/reference/vivante_blobs.rs"]
pub mod mesa_reference;
#[cfg(mesa_reference)]
pub mod offline;
/// The in-process compiler, when this build has one (`shaderc-online`).
#[cfg(feature = "shaderc-online")]
pub mod online;
pub mod packing;
/// The precompiled table (`precompiled-shaders`): the tier in front of the
/// compiler, or, in an image without one, the whole back end.
pub mod precompiled;
pub mod record_key;
pub mod sha256;

/* enum shc_stage */
pub const SHC_STAGE_VERTEX: u32 = 0;
pub const SHC_STAGE_FRAGMENT: u32 = 1;

/* enum shc_blend_factor: Mesa's pipe_blendfactor codes, so the generator and
 * the runtime agree. */
pub const SHC_BF_ONE: u32 = 1;
pub const SHC_BF_SRC_COLOR: u32 = 2;
pub const SHC_BF_SRC_ALPHA: u32 = 3;
pub const SHC_BF_DST_ALPHA: u32 = 4;
pub const SHC_BF_DST_COLOR: u32 = 5;
pub const SHC_BF_SRC_ALPHA_SATURATE: u32 = 6;
pub const SHC_BF_CONST_COLOR: u32 = 7;
pub const SHC_BF_CONST_ALPHA: u32 = 8;
pub const SHC_BF_INVERT: u32 = 0x10;
pub const SHC_BF_ZERO: u32 = SHC_BF_INVERT | SHC_BF_ONE;
pub const SHC_BF_INV_SRC_COLOR: u32 = SHC_BF_INVERT | SHC_BF_SRC_COLOR;
pub const SHC_BF_INV_SRC_ALPHA: u32 = SHC_BF_INVERT | SHC_BF_SRC_ALPHA;
pub const SHC_BF_INV_DST_ALPHA: u32 = SHC_BF_INVERT | SHC_BF_DST_ALPHA;
pub const SHC_BF_INV_DST_COLOR: u32 = SHC_BF_INVERT | SHC_BF_DST_COLOR;
pub const SHC_BF_INV_CONST_COLOR: u32 = SHC_BF_INVERT | SHC_BF_CONST_COLOR;
pub const SHC_BF_INV_CONST_ALPHA: u32 = SHC_BF_INVERT | SHC_BF_CONST_ALPHA;

/* enum shc_blend_func */
pub const SHC_BLEND_ADD: u32 = 0;
pub const SHC_BLEND_SUBTRACT: u32 = 1;
pub const SHC_BLEND_REVERSE_SUBTRACT: u32 = 2;
pub const SHC_BLEND_MIN: u32 = 3;
pub const SHC_BLEND_MAX: u32 = 4;

/// Blend-variant key: [0:4] src_rgb, [5:9] dst_rgb, [10:14] src_a, [15:19]
/// dst_a, [20:22] eq_rgb, [23:25] eq_a, [26:29] disabled colour channels.
pub const fn shc_blend_key(src_rgb: u32, dst_rgb: u32, src_a: u32, dst_a: u32, eq_rgb: u32, eq_a: u32) -> u32 {
    src_rgb | (dst_rgb << 5) | (src_a << 10) | (dst_a << 15) | (eq_rgb << 20) | (eq_a << 23)
}

/// GC7000 only. Blending and colour masks are pixel-engine state on that part,
/// so the blend key is always 0 there and the whole space is free; this
/// borrows the value 1 to mean "the same fragment shader, compiled for the
/// opposite front-face winding". See [`ShcShader::reads_front_facing`].
pub const SHC_VIV_KEY_FRONT_FLIPPED: u32 = 1;

/// The glColorMask part of the key (bit 0 = R ... bit 3 = A, set = disabled).
pub const fn shc_blend_key_mask_off(disabled_rgba: u32) -> u32 {
    (disabled_rgba & 0xf) << 26
}

/// Bits 30..31: the render target class the colour output is packed for
/// (glsl2usc '@rt' variants, pco_shc PCO_SHC_OUT): 0 RGBA8 unorm (the base
/// shader), 1 RGBA16F (four halves in two dwords), 2 R16F, 3 R32F.
pub const SHC_BLEND_KEY_RT_SHIFT: u32 = 30;
pub const fn shc_blend_key_rt(cls: u32) -> u32 {
    (cls & 3) << SHC_BLEND_KEY_RT_SHIFT
}
pub const SHC_RT_RGBA8: u32 = 0;
pub const SHC_RT_F16X4: u32 = 1;
pub const SHC_RT_F16: u32 = 2;
pub const SHC_RT_F32: u32 = 3;

/// ONE/ZERO ADD on both channels with every channel written: no variant.
pub const SHC_BLEND_KEY_REPLACE: u32 =
    shc_blend_key(SHC_BF_ONE, SHC_BF_ZERO, SHC_BF_ONE, SHC_BF_ZERO, SHC_BLEND_ADD, SHC_BLEND_ADD);

/// The PowerVR's native cube (`gl::native_cube`), default ON since step C
/// (2026-09-25; the AM62P's 333 dEQP cube cases at 9548e106: 323/10 with
/// it, 276/57 without, no case failing that the stacked image passes;
/// am62p-board): the online compiler samples a samplerCube as the TPU's own
/// cube (three coordinates, `smp` 3d) and says so in the record
/// (`ShcShader::native_cube`), and the draw binds a twiddled TEXTYPE_CUBE
/// copy for such a program. The table's cube programs are native too
/// (glsl2usc no longer rewrites samplerCube). Read when a shader COMPILES;
/// a program compiled with it off keeps the stacked cube.
#[no_mangle]
pub static mut pvr_native_cube: i32 = PVR_NATIVE_CUBE_DEFAULT;

/// [`pvr_native_cube`]'s value at boot.
pub const PVR_NATIVE_CUBE_DEFAULT: i32 = 1;

/// Set [`pvr_native_cube`].
#[no_mangle]
pub extern "C" fn pvr_set_native_cube(on: core::ffi::c_int) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_native_cube).write(i32::from(on != 0)) }
}

/// Is [`pvr_native_cube`] on?
pub fn native_cube_enabled() -> bool {
    // SAFETY: a plain read of an int only the setter writes.
    (unsafe { core::ptr::addr_of!(pvr_native_cube).read() }) != 0
}

/// The PowerVR's `texture2DProj`, default OFF: the online compiler divides
/// in the shader (frcp(q) * s, frcp(q) * t) and samples the quotient without
/// `smp`'s `proj`, instead of pco's form (q in the data block, the TPU
/// divides). The TPU then takes the level from the quotient's own quad
/// derivatives, which is what dEQP's reference computes. With `proj` the
/// AM62P drew mipmap.2d.projected.* (all 12) with a thin band of pixels on
/// every level boundary in the other level (93 of 65,536 in
/// nearest_nearest_clamp, a60104c2): the boundary isolines are shifted,
/// the picture otherwise right. Whether the in-shader divide moves them
/// back is the board's measurement. Read when a shader COMPILES.
#[no_mangle]
pub static mut pvr_proj_divide: i32 = 0;

/// Set [`pvr_proj_divide`].
#[no_mangle]
pub extern "C" fn pvr_set_proj_divide(on: core::ffi::c_int) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_proj_divide).write(i32::from(on != 0)) }
}

/// Is [`pvr_proj_divide`] on?
pub fn proj_divide_enabled() -> bool {
    // SAFETY: a plain read of an int only the setter writes.
    (unsafe { core::ptr::addr_of!(pvr_proj_divide).read() }) != 0
}

/// The level of an implicit-level sample of a smooth varying, on the
/// PowerVR online compiler: 0 = the TPU's own (`smp lodm = normal`),
/// 1 = exact per-pixel gradients for a 2D sample, 2 = for a cube too.
///
/// The TPU's implicit level takes one set of derivatives per 2x2 quad. For a
/// perspective-interpolated coordinate the derivatives change from pixel to
/// pixel, and the pixel diagonal to the quad's anchor got a level dEQP's
/// verifier rejects (texture.mipmap.2d.projected.*, one fixed quad slot;
/// RESULTS.md 1m). With this on, the shader computes each coordinate's
/// derivatives from the plane equations the iterator evaluates -- u = P/Q
/// with P, Q linear in x and y gives du/dx = (A_P - u A_Q) / Q -- and samples
/// with `lodm = gradients`. Read when a shader COMPILES.
#[no_mangle]
pub static mut pvr_proj_lod: i32 = PVR_PROJ_LOD_DEFAULT;

/// [`pvr_proj_lod`]'s default.
pub const PVR_PROJ_LOD_DEFAULT: i32 = 2;

/// Set [`pvr_proj_lod`].
#[no_mangle]
pub extern "C" fn pvr_set_proj_lod(mode: core::ffi::c_int) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(pvr_proj_lod).write(mode) }
}

/// [`pvr_proj_lod`]'s value.
pub fn proj_lod_mode() -> i32 {
    // SAFETY: a plain read of an int only the setter writes.
    unsafe { core::ptr::addr_of!(pvr_proj_lod).read() }
}

/// Set by the online back end around a fragment compile for a program whose
/// vertex shader writes a constant `gl_Position.w`
/// (`ir::Module::constant_position_w`, decided at link): every varying is
/// then affine in screen space, the TPU's quad derivatives are exact, and
/// [`pvr_proj_lod`]'s gradients are not emitted -- the program is the one
/// `pvr_proj_lod = 0` compiles. Not a user switch.
static FS_W_CONSTANT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Set [`FS_W_CONSTANT`] for the compiles that follow; returns the old value
/// for the caller to put back.
pub fn set_fs_w_constant(on: bool) -> bool {
    FS_W_CONSTANT.swap(on, core::sync::atomic::Ordering::Relaxed)
}

/// Is [`FS_W_CONSTANT`] set?
pub fn fs_w_constant() -> bool {
    FS_W_CONSTANT.load(core::sync::atomic::Ordering::Relaxed)
}

/* ---- how an `if` with arithmetic arms is lowered (PowerVR online) -------- */

/// How the PowerVR online compiler lowers an `if` whose two arms are
/// straight-line arithmetic (`powervr::select`, `predicable`). Both forms
/// are pco's (Mesa `src/imagination/pco/pco_cf.c`): the arms run
/// predicated, every arm group issued under `if(!p0)` / `if(p0)`
/// (`lower_if_pred_exec`), or under the execution mask with a branch past
/// each arm no instance runs (`lower_if_cond_exec`). The values are
/// `PVR_IF_LOWERING_*` in `pvr_backend.h` (`PVR_BACKEND_OPT_IF_LOWERING`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IfLowering {
    /// The compiler chooses by the arms' sizes: it branches only when
    /// skipping either arm saves more than the branch costs
    /// (`powervr::select::branch_pays`, gl_test 194/195's constants), and
    /// predicates otherwise, as pco does.
    Default,
    /// Always predicate.
    Predicate,
    /// Always branch (pco's `PCO_DEBUG=no_pred_cf`).
    Branch,
}

impl IfLowering {
    /// From the option's value; anything but 1 and 2 is the default.
    pub const fn from_u32(v: u32) -> IfLowering {
        match v {
            1 => IfLowering::Predicate,
            2 => IfLowering::Branch,
            _ => IfLowering::Default,
        }
    }

    /// The option's value.
    pub const fn as_u32(self) -> u32 {
        match self {
            IfLowering::Default => 0,
            IfLowering::Predicate => 1,
            IfLowering::Branch => 2,
        }
    }
}

/// The user's switch (`PVR_BACKEND_OPT_IF_LOWERING`): read when a shader is
/// compiled from its source, and kept by everything compiled again from it
/// (a blend variant, a link's substitution, a sampler-state variant), so
/// the same source can be compiled both ways on one context.
static IF_LOWERING: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Set the user's [`IfLowering`] (a value from `PVR_IF_LOWERING_*`).
pub fn set_if_lowering(v: u32) {
    IF_LOWERING.store(IfLowering::from_u32(v).as_u32(), core::sync::atomic::Ordering::Relaxed);
}

/// The user's [`IfLowering`].
pub fn if_lowering() -> IfLowering {
    IfLowering::from_u32(IF_LOWERING.load(core::sync::atomic::Ordering::Relaxed))
}

/// The [`IfLowering`] the online compile in progress was asked for; set by
/// the online back end around a compile, like [`FS_W_CONSTANT`]. Not a user
/// switch.
static COMPILE_IF_LOWERING: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Set the compile's [`IfLowering`]; returns the old one for the caller to
/// put back.
pub fn set_compile_if_lowering(l: IfLowering) -> IfLowering {
    IfLowering::from_u32(COMPILE_IF_LOWERING.swap(l.as_u32(), core::sync::atomic::Ordering::Relaxed))
}

/// The compile's [`IfLowering`].
pub fn compile_if_lowering() -> IfLowering {
    IfLowering::from_u32(COMPILE_IF_LOWERING.load(core::sync::atomic::Ordering::Relaxed))
}

/* ---- sampler-state variants (ShcBackend::lod_sensitivity) ------------------ */

/// The draw-time facts about the samplers a shader reads that its code
/// depends on, one bit per texture binding of the stage (bit b = binding b,
/// the `ShcShader::sampler_bindings` numbering). As a key, a set bit asks for
/// the variant behaviour at that binding; as a sensitivity, it says the
/// binding's state can change the code. The C ABI carries it as one
/// `uint64_t` ([`LodKey::bits`]): `mip_free` in bits 0..15 and `ret32` in
/// bits 16..31 (bindings 0..15 each: the GL core keys bindings below
/// `CAP_SAMPLERS`, 16, and no other), `mag_switch` in bits 32..62,
/// `sample_kill` in bit 63.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LodKey {
    /// PowerVR, fragment: the binding's sampler has no mipmap filter (or one
    /// level), so the exact per-pixel gradients of [`pvr_proj_lod`] cannot
    /// change a texel and the look-up samples with the TPU's own level --
    /// the program `pvr_proj_lod = 0` compiles.
    pub mip_free: u32,
    /// GL ES 2.0 3.7.8 / ES 3.0 3.8.11: TEXTURE_MAG_FILTER LINEAR with
    /// TEXTURE_MIN_FILTER NEAREST_MIPMAP_NEAREST or NEAREST_MIPMAP_LINEAR puts
    /// the magnification/minification switch at c = 0.5, where the PowerVR
    /// TPU and the GC7000 TE switch at 0. An explicit-level look-up of the
    /// binding has its level remapped, `lod <= 0.5 ? -1.0 : lod`
    /// (`online::lodkey`), so the hardware magnifies where GL does. Only in a
    /// build that reports ES 2.0 (`crate::REPORT_ES2`): dEQP judges an
    /// ES 3-capable context against its seamless cube reference, whose
    /// switch is at 0 (tcuTexture.cpp sampleLevelArrayCubeSeamless), and
    /// there the hardware's own switch passes.
    pub mag_switch: u32,
    /// PowerVR, fragment: not a sampler fact but the draw's sample state,
    /// carried in the same key because it is the same kind of variant (the
    /// stage compiled again, everything else as it was). Set: the fragment
    /// shader is compiled with pco's per-sample kill epilogue even when it
    /// has no `discard` of its own, so the per-draw control word's sample
    /// mask and alpha-to-coverage bit take effect (GL ES 2.0 4.1.3,
    /// glSampleCoverage / SAMPLE_ALPHA_TO_COVERAGE on a multisampled
    /// target; Mesa's pvr compiles that epilogue into every fragment
    /// shader, `pco_nir_pfo` `lower_isp_fb`). Bit 63 of [`Self::bits`], so
    /// `mag_switch` keys bindings 0..30 only (the GL core keys fewer,
    /// `CAP_SAMPLERS`).
    pub sample_kill: bool,
    /// V3D: the binding's texture is a format the TMU returns at 32 bits a
    /// channel (R32F, a depth texture as the GL core keeps it; Mesa's return
    /// size 32, v3dx_format_table.c), so its look-ups set TMU Config
    /// Parameter 1's "Output Type 32-bit" (v3d_packet.xml) and read one word
    /// per channel instead of two f16 channels a word (`online::v3d::select`).
    /// Bindings 0..15.
    pub ret32: u32,
}

/// [`LodKey::sample_kill`]'s bit in [`LodKey::bits`].
pub const LOD_KEY_SAMPLE_KILL: u64 = 1 << 63;

impl LodKey {
    pub const NONE: LodKey = LodKey { mip_free: 0, mag_switch: 0, sample_kill: false, ret32: 0 };

    /// The C ABI's `uint64_t`.
    pub const fn bits(self) -> u64 {
        (self.mip_free & 0xffff) as u64
            | ((self.ret32 & 0xffff) as u64) << 16
            | ((self.mag_switch & 0x7fff_ffff) as u64) << 32
            | if self.sample_kill { LOD_KEY_SAMPLE_KILL } else { 0 }
    }

    /// [`Self::bits`] read back.
    pub const fn from_bits(b: u64) -> LodKey {
        LodKey {
            mip_free: b as u32 & 0xffff,
            ret32: (b >> 16) as u32 & 0xffff,
            mag_switch: (b >> 32) as u32 & 0x7fff_ffff,
            sample_kill: b & LOD_KEY_SAMPLE_KILL != 0,
        }
    }

    pub const fn is_none(self) -> bool {
        self.mip_free == 0 && self.mag_switch == 0 && !self.sample_kill && self.ret32 == 0
    }
}

/// Switch (a) of the sampler-state variants, PowerVR online compiler: a
/// draw whose fragment look-up with exact gradients reads a sampler with no
/// mipmap filter takes the fragment shader compiled without them
/// ([`LodKey::mip_free`]). 1 on (default), 0 off. Read when a program is
/// first drawn after its link (the program's sensitivity is cached then).
#[no_mangle]
pub static mut shc_lod_mip_variant: i32 = 1;

/// Switch (b): a draw whose explicit-level look-up reads a sampler with
/// mag LINEAR and min NEAREST_MIPMAP_* takes the stage compiled with the
/// c = 0.5 remap ([`LodKey::mag_switch`]), PowerVR and GC7000 online
/// compilers. 1 on (default; host-verified only), 0 off. Read as switch (a).
#[no_mangle]
pub static mut shc_lod_switch_variant: i32 = 1;

/// Set [`shc_lod_mip_variant`] and [`shc_lod_switch_variant`] (a negative
/// value keeps a switch as it is).
#[no_mangle]
pub extern "C" fn shc_set_lod_variants(mip: core::ffi::c_int, switch: core::ffi::c_int) {
    // SAFETY: single-threaded by contract; plain stores of ints.
    unsafe {
        if mip >= 0 {
            core::ptr::addr_of_mut!(shc_lod_mip_variant).write(i32::from(mip != 0));
        }
        if switch >= 0 {
            core::ptr::addr_of_mut!(shc_lod_switch_variant).write(i32::from(switch != 0));
        }
    }
}

/// Are switches (a) and (b) on?
pub fn lod_variants_enabled() -> (bool, bool) {
    // SAFETY: plain reads of ints only the setter writes.
    unsafe {
        (core::ptr::addr_of!(shc_lod_mip_variant).read() != 0, core::ptr::addr_of!(shc_lod_switch_variant).read() != 0)
    }
}

/// The [`LodKey`] the online compile in progress was asked for; set by the
/// online back end around a compile, like [`FS_W_CONSTANT`]. Not a user
/// switch.
static COMPILE_MIP_FREE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static COMPILE_MAG_SWITCH: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static COMPILE_SAMPLE_KILL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static COMPILE_RET32: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Set the compile's [`LodKey`]; returns the old one for the caller to put
/// back.
pub fn set_compile_lod_key(k: LodKey) -> LodKey {
    use core::sync::atomic::Ordering::Relaxed;
    LodKey {
        mip_free: COMPILE_MIP_FREE.swap(k.mip_free, Relaxed),
        mag_switch: COMPILE_MAG_SWITCH.swap(k.mag_switch, Relaxed),
        sample_kill: COMPILE_SAMPLE_KILL.swap(k.sample_kill, Relaxed),
        ret32: COMPILE_RET32.swap(k.ret32, Relaxed),
    }
}

/// The compile's [`LodKey`].
pub fn compile_lod_key() -> LodKey {
    use core::sync::atomic::Ordering::Relaxed;
    LodKey {
        mip_free: COMPILE_MIP_FREE.load(Relaxed),
        mag_switch: COMPILE_MAG_SWITCH.load(Relaxed),
        sample_kill: COMPILE_SAMPLE_KILL.load(Relaxed),
        ret32: COMPILE_RET32.load(Relaxed),
    }
}

pub const SHC_NO_PSIZE: u8 = 0xff;
pub const SHC_NO_BLEND_CONSTS: u8 = 0xff;
/// The glBlendColor rgba dwords glsl2usc appends to a CONSTANT_\* variant's
/// uniform block (the program keeps room for them past the block the
/// application writes; see [`crate::gl::context::BLEND_CONSTS_DWORDS`]).
pub const SHC_BLEND_CONSTS_DWORDS: usize = 4;
pub const SHC_UNIFORM_NAME_MAX: usize = 64;
pub const SHC_UNIFORM_NO_BLOCK: u16 = 0xffff;
pub const SHC_NO_META: u16 = 0xffff;
/// The full-mode (discard) FS meta word: colour writes on all 8 RTs, every
/// sample-mask bit, no alpha-to-coverage / alpha-to-one.
pub const SHC_FS_META_DEFAULT: u32 = 0x01ff_fffe;

/// `struct shc_varying`: one interpolated fragment input / vertex output.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcVarying {
    pub location: u8,
    pub components: u8,
    /// 0 smooth (perspective), 1 noperspective, 2 flat.
    pub interp: u8,
    pub vtxout_off: u8,
    pub name: [c_char; 32],
}

impl ShcVarying {
    pub const ZERO: ShcVarying = ShcVarying { location: 0, components: 0, interp: 0, vtxout_off: 0, name: [0; 32] };
}

/// `struct shc_attrib`: one vertex attribute.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcAttrib {
    pub location: u8,
    pub components: u8,
    pub vi_reg: u8,
    pub format: u8,
    pub name: [c_char; 32],
    pub gl_type: u16,
}

impl ShcAttrib {
    pub const ZERO: ShcAttrib =
        ShcAttrib { location: 0, components: 0, vi_reg: 0, format: 0, name: [0; 32], gl_type: 0 };
}

/// Vertex attributes a shader may declare for the link-time layout
/// ([`ShcAttribMap`]): more than the 16 locations, since aliasing lets a
/// shader declare more attributes than there are locations.
pub const SHC_MAX_DECLARED_ATTRIBS: usize = 32;

/// `struct shc_attrib_binding`: one declared vertex attribute and the GL
/// location the link decided for it (src/shaderc/SHADERC_NOTES.md,
/// "Attribute registers decided at link").
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcAttribBinding {
    /// The attribute's name, as in the source (the same field as
    /// [`ShcAttrib::name`]).
    pub name: [c_char; 32],
    /// The generic attribute index of its first column: glBindAttribLocation's
    /// index when bound, else the lowest free run the link found.
    /// [`SHC_ATTRIB_NO_LOCATION`] in a compile's `declared` list (not decided yet).
    pub location: u8,
    /// Locations it takes: 1, or 2/3/4 for a matN (one per column).
    pub columns: u8,
    /// The GL type as declared (`GL_FLOAT_VEC4`, `GL_FLOAT_MAT3`, ...): what
    /// glGetActiveAttrib reports for it, also when it shares its location
    /// with another name and the record's row carries the other one.
    pub gl_type: u16,
    /// 1: the shader statically reads it, so it is an active attribute
    /// (ES 2.0 2.10.4); 0: declared only. The compile that sets
    /// `layout_at_link` fills it; a relink does not change it.
    pub active: u8,
}

/// `ShcAttribBinding::location` of an attribute no location was decided for.
pub const SHC_ATTRIB_NO_LOCATION: u8 = 0xff;

impl ShcAttribBinding {
    pub const ZERO: ShcAttribBinding =
        ShcAttribBinding { name: [0; 32], location: 0, columns: 0, gl_type: 0, active: 0 };
}

/// `struct shc_attrib_map`: every declared vertex attribute of a program's
/// vertex shader with the location the link decided -- the input layout a
/// back end compiles the vertex shader with at link (`compile_vs_linked`).
/// Two entries whose location ranges overlap alias. No register numbers:
/// each back end places the attributes it reads itself.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcAttribMap {
    pub count: u8,
    pub b: [ShcAttribBinding; SHC_MAX_DECLARED_ATTRIBS],
}

impl ShcAttribMap {
    pub const ZERO: ShcAttribMap = ShcAttribMap { count: 0, b: [ShcAttribBinding::ZERO; SHC_MAX_DECLARED_ATTRIBS] };
}

/// Section 7.5's built-in uniform `gl_DepthRange` as the rows a compiled
/// shader that reads it carries (the online compiler's reflection puts them
/// after the application's uniforms), in the order the section declares the
/// members. The GL core fills them from glDepthRangef at draw time.
pub const SHC_DEPTH_RANGE_ROWS: [&str; 3] = ["gl_DepthRange.near", "gl_DepthRange.far", "gl_DepthRange.diff"];

/// One active uniform block of a compiled shader (ES 3.00 section 4.3.7):
/// its GL name (`Block`, or `Block[k]` for an element of an array of
/// blocks), its std140 size in bytes, and its members, a run of
/// [`ShcBlocks::members`].
///
/// Rust-only, deliberately: only the online compiler produces blocks (the
/// offline tables never do), so the table lives in the
/// online back end's own record behind `priv_` and the GL core reads it
/// through [`uniform_blocks`]. `struct shc_shader`, its C header and the
/// generated tables are unchanged (team-lead, 2026-09-24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShcBlock {
    pub name: &'static str,
    pub data_size: u32,
    pub first_member: u16,
    pub num_members: u16,
}

/// One member of a uniform block as GL names it, where it lives in the
/// bound buffer (std140, bytes), and the row of
/// [`ShcShader::uniform_table`] it fills ([`SHC_BLOCK_NO_ROW`] when the
/// shader's table has none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShcBlockMember {
    pub name: &'static str,
    pub row: u16,
    pub offset: u32,
    pub array_stride: u32,
    pub matrix_stride: u32,
    pub row_major: bool,
    pub gl_type: u16,
    pub array_size: u16,
}

pub const SHC_BLOCK_NO_ROW: u16 = u16::MAX;

/// A shader's uniform blocks: empty for every shader but an online-compiled
/// one that declares some.
#[derive(Clone, Copy, Debug)]
pub struct ShcBlocks {
    pub blocks: &'static [ShcBlock],
    pub members: &'static [ShcBlockMember],
    /// The uniform table rows (`ShcShader::uniform_table` indices) whose
    /// declaration is an array, whatever its length: `float f[1]` is an
    /// array that `array_len` (1) cannot tell from `float f`. Known only for
    /// an online-compiled shader; empty for a table shader, where
    /// the GL core falls back to `array_len > 1`.
    pub arrays: &'static [u16],
}

impl ShcBlocks {
    pub const EMPTY: ShcBlocks = ShcBlocks { blocks: &[], members: &[], arrays: &[] };

    /// Row `k` of the uniform table is declared as an array.
    pub fn is_array_row(&self, k: usize) -> bool {
        self.arrays.iter().any(|&r| usize::from(r) == k)
    }

    /// The members of block `b`.
    pub fn members_of(&self, b: &ShcBlock) -> &'static [ShcBlockMember] {
        let (s, n) = (usize::from(b.first_member), usize::from(b.num_members));
        self.members.get(s..s + n).unwrap_or(&[])
    }
}

/// The uniform blocks of `sh`: empty -- never an error -- for a shader of
/// the offline tables, and in a build without the online
/// compiler.
pub fn uniform_blocks(sh: &ShcShader) -> ShcBlocks {
    #[cfg(feature = "shaderc-online")]
    {
        online::backend::blocks_of(sh)
    }
    /* without the compiler, a precompiled row's (the compiler made it) */
    #[cfg(not(feature = "shaderc-online"))]
    {
        precompiled::view(sh).map_or(ShcBlocks::EMPTY, |r| r.blocks)
    }
}

/// Whether the in-process (online) compiler made `sh`. Its uniform rows are
/// one per array, element by element (`s[1].b[0].b`, the member's own
/// array), never the offline table's per-member flattening of an array of
/// structs (`A.B[0].C` standing for every element), and the GL core must
/// not read them as that (`program::build_locations`).
pub fn is_online(sh: &ShcShader) -> bool {
    #[cfg(feature = "shaderc-online")]
    {
        online::backend::is_ours(sh)
    }
    /* without the compiler, a precompiled row: the compiler made it, at
     * build time, with the same uniform rows */
    #[cfg(not(feature = "shaderc-online"))]
    {
        precompiled::view(sh).is_some()
    }
}

/// `struct shc_uniform`: one uniform addressable by name.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcUniform {
    pub name: [c_char; SHC_UNIFORM_NAME_MAX],
    /// Offset in the stage's uniform block, or `SHC_UNIFORM_NO_BLOCK`.
    pub dword_off: u16,
    pub dwords: u16,
    pub gl_type: u16,
    pub array_len: u8,
    /// Samplers: descriptor binding (texture unit).
    pub binding: u8,
    /// Elements per outer index of a flattened array-of-structs member (0 = 1).
    pub inner: u8,
}

impl ShcUniform {
    pub const ZERO: ShcUniform = ShcUniform {
        name: [0; SHC_UNIFORM_NAME_MAX],
        dword_off: 0,
        dwords: 0,
        gl_type: 0,
        array_len: 0,
        binding: 0,
        inner: 0,
    };
}

/* ---- the Vivante GC7000 (i.MX8MP) shader ABI -------------------------------
 *
 * On a `gpu-vivante` build the same [`ShcShader`] carries the reflection
 * (attributes, varyings, uniforms), `usc`/`usc_dw` carry the GC7000
 * instruction words instead of PowerVR USC (the GL core uploads the binary
 * the same way for both GPUs, so the field keeps its name; the backend calls
 * it the shader code), and `viv` carries everything else the backend has to
 * program: the register numbers the shader reads and writes, and the constant
 * register file. src/vivante/VIVANTE_NOTES.md documents it in full.
 */

/* enum shc_viv_const: where one dword of the constant register file comes
 * from. The codes are Mesa's enum etna_uniform_contents, which the offline
 * tool reports. */
pub type ShcVivConst = u8;
pub const SHC_VIV_CONST_UNUSED: ShcVivConst = 0;
/// `data` is the value itself.
pub const SHC_VIV_CONST_LITERAL: ShcVivConst = 1;
/// `data` is a dword index into the uniform block.
pub const SHC_VIV_CONST_UNIFORM: ShcVivConst = 2;
/// `data` is a sampler binding: 1.0 / width.
pub const SHC_VIV_CONST_TEXRECT_SCALE_X: ShcVivConst = 3;
/// `data` is a sampler binding: 1.0 / height.
pub const SHC_VIV_CONST_TEXRECT_SCALE_Y: ShcVivConst = 4;
/// `data` is a sampler binding (textureSize).
pub const SHC_VIV_CONST_TEXTURE_WIDTH: ShcVivConst = 5;
pub const SHC_VIV_CONST_TEXTURE_HEIGHT: ShcVivConst = 6;
pub const SHC_VIV_CONST_TEXTURE_DEPTH: ShcVivConst = 7;
/// `data` is a sampler binding.
pub const SHC_VIV_CONST_SAMPLER_LOD_MIN: ShcVivConst = 8;
pub const SHC_VIV_CONST_SAMPLER_LOD_MAX: ShcVivConst = 9;
pub const SHC_VIV_CONST_SAMPLER_LOD_BIAS: ShcVivConst = 10;
/// Uniform blocks: not produced by this path.
pub const SHC_VIV_CONST_UBO_ADDR: ShcVivConst = 11;
/// Not Mesa's (which lowers it with a state variable): `gl_FragCoord.y`'s
/// scale, -1 on a target the backend draws mirrored (`flip_y`) and 1
/// otherwise; `data` unused. The program computes y' = y * scale + offset.
pub const SHC_VIV_CONST_FRAGCOORD_Y_SCALE: ShcVivConst = 12;
/// ... and the offset: the target's height when mirrored, 0 otherwise.
pub const SHC_VIV_CONST_FRAGCOORD_Y_OFFSET: ShcVivConst = 13;
/// Ours: `gl_PointCoord.t`'s scale, 1 on a target the backend draws mirrored
/// and -1 otherwise. The point sprite's t runs from the top of the target as
/// the rasteriser sees it, which is GL's top only when the target is
/// mirrored. The program computes t' = t * scale + offset.
pub const SHC_VIV_CONST_PNTC_Y_SCALE: ShcVivConst = 14;
/// ... and the offset: 0 when mirrored, 1 otherwise.
pub const SHC_VIV_CONST_PNTC_Y_OFFSET: ShcVivConst = 15;

/// An `int8_t` -1 from the compiler, as a register.
pub const SHC_VIV_NO_REG: u8 = 0xff;
/// The name of the GC7000 fragment input row `gl_PointCoord` arrives in:
/// the rasteriser makes it, so no vertex output pairs with it (`link.rs`
/// leaves it unpaired, the driver programs its two components as the point
/// coordinate). A `gl_` name is never a user varying.
pub const SHC_VIV_POINT_COORD: &str = "gl_PointCoord";
pub const SHC_VIV_MAX_IO: usize = 16;

/// The GC7000's vertex attribute locations: its front end's vertex elements
/// (`vivante::state::GC_MAX_VERTEX_ELEMENTS`), each writing one whole vec4
/// input register; a matrix takes one a column. A vertex shader declaring
/// more waits for the link's layout (`layout_at_link`).
pub const SHC_VIV_MAX_ATTRIB_LOCATIONS: u32 = 16;

/// One shader input or output register (`struct shc_viv_io`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcVivIo {
    /// GC7000 register number.
    pub reg: u8,
    /// The compiler's `gl_varying_slot` / `gl_vert_attrib`.
    pub slot: u8,
    /// 1..4.
    pub components: u8,
    /// Fragment inputs: 0 smooth, 1 noperspective, 2 flat.
    pub interp: u8,
    /// GLSL name: how `link()` pairs VS outputs with FS inputs.
    pub name: [c_char; 32],
}

impl ShcVivIo {
    pub const ZERO: ShcVivIo = ShcVivIo { reg: 0, slot: 0, components: 0, interp: 0, name: [0; 32] };

    /// The register's GLSL name, up to its NUL.
    pub fn name(&self) -> &[c_char] {
        match self.name.iter().position(|&b| b == 0) {
            Some(n) => &self.name[..n],
            None => &self.name,
        }
    }
}

/// The GC7000-specific half of a compiled shader (`struct shc_viv`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcViv {
    /// Inputs: VS attributes, FS varyings (the compiler places FS varyings in
    /// registers 1..N, so N is also the varying count).
    pub in_: [ShcVivIo; SHC_VIV_MAX_IO],
    pub num_in: u8,
    /// Outputs: VS varyings and position, FS colour.
    pub out: [ShcVivIo; SHC_VIV_MAX_IO],
    pub num_out: u8,
    /// Fragment, filled by `link()`: the VS output register that feeds
    /// `in_[i]`, or [`SHC_VIV_NO_REG`] when the vertex shader writes no such
    /// output.
    pub vs_reg_of_in: [u8; SHC_VIV_MAX_IO],

    /* special registers ([`SHC_VIV_NO_REG`] when the shader has none) */
    pub vs_pos_out_reg: u8,
    pub vs_pointsize_out_reg: u8,
    pub vs_id_in_reg: u8,
    pub ps_color_out_reg: u8,
    pub ps_depth_out_reg: u8,

    /// The VS_LOAD_BALANCING word the compiler chose.
    pub vs_load_balancing: u32,
    /// The `{VS,PS}_INPUT_COUNT` field UNK8.
    pub input_count_unk8: u16,

    /* the constant register file: one entry per dword, in register order */
    /// [`ShcVivConst`] per dword.
    pub const_contents: *const u8,
    pub const_data: *const u32,
    pub const_count: u16,
}

impl ShcViv {
    pub const ZERO: ShcViv = ShcViv {
        in_: [ShcVivIo::ZERO; SHC_VIV_MAX_IO],
        num_in: 0,
        out: [ShcVivIo::ZERO; SHC_VIV_MAX_IO],
        num_out: 0,
        vs_reg_of_in: [SHC_VIV_NO_REG; SHC_VIV_MAX_IO],
        vs_pos_out_reg: SHC_VIV_NO_REG,
        vs_pointsize_out_reg: SHC_VIV_NO_REG,
        vs_id_in_reg: SHC_VIV_NO_REG,
        ps_color_out_reg: SHC_VIV_NO_REG,
        ps_depth_out_reg: SHC_VIV_NO_REG,
        vs_load_balancing: 0,
        input_count_unk8: 0,
        const_contents: core::ptr::null(),
        const_data: core::ptr::null(),
        const_count: 0,
    };

    /// Where each dword of the constant register file comes from.
    pub fn const_contents(&self) -> &[ShcVivConst] {
        if self.const_contents.is_null() {
            return &[];
        }
        // SAFETY: the table's arrays hold `const_count` entries each.
        unsafe { core::slice::from_raw_parts(self.const_contents, self.const_count as usize) }
    }

    /// The datum of each dword of the constant register file (a literal, a
    /// uniform index or a sampler binding, per `const_contents`).
    pub fn const_data(&self) -> &[u32] {
        if self.const_data.is_null() {
            return &[];
        }
        // SAFETY: as above.
        unsafe { core::slice::from_raw_parts(self.const_data, self.const_count as usize) }
    }
}

/* ---- the Broadcom V3D 4.2 (Raspberry Pi 4) shader ABI --------------------
 *
 * The V3D compiles a PROGRAM, not a stage: the vertex shader writes its
 * varyings in the order the fragment shader reads them, and the binner runs a
 * third program, the coordinate shader. So on a `gpu-v3d` build a compiled
 * stage carries only its reflection (attributes, varyings, uniforms) and no
 * code, and `link()` replaces the program's fragment shader with the linked
 * program: `usc`/`usc_dw` then hold the three QPU programs back to back (the
 * GL core uploads them as it uploads any fragment shader) and `v3d` says where
 * each one starts and what the driver's shader state record needs for it.
 * docs/online-shaderc/v3d-backend.md, "M6", is the design.
 */

/// An attribute location the linked program does not read.
pub const SHC_V3D_NO_RECORD: u8 = 0xff;

/// The V3D's vertex attribute locations: one attribute record each
/// (`ShcV3d::record_of_location`); a matrix takes one a column. A vertex
/// shader declaring more waits for the link's layout (`layout_at_link`).
pub const SHC_V3D_MAX_ATTRIB_LOCATIONS: u32 = 16;
const _: () = assert!(SHC_V3D_MAX_ATTRIB_LOCATIONS as usize == ShcV3d::ZERO.record_of_location.len());

/// One word of a QPU program's uniform stream (`struct shc_v3d_uniform`):
/// Mesa's `enum quniform_contents` and its datum. For
/// `QUNIFORM_UNIFORM` (1) the datum is a dword index into the stage's GL
/// uniform block.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShcV3dUniform {
    pub contents: u32,
    pub data: u32,
}

/// The fragment shader of a linked program (`struct shc_v3d_fs`): the
/// fields of the driver's `struct v3d_fs_prog` that are not addresses.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcV3dFs {
    /// Byte offset of the program in `usc`, a multiple of 8.
    pub code_off: u32,
    pub code_bytes: u32,
    pub uniforms: *const ShcV3dUniform,
    pub num_uniforms: u32,
    pub num_inputs: u32,
    pub flat_shade_flags: u32,
    pub noperspective_flags: u32,
    pub centroid_flags: u32,
    pub threads: u8,
    pub single_seg: u8,
    pub writes_z: u8,
    pub disable_ez: u8,
    pub uses_center_w: u8,
    pub lock_scoreboard_on_first_thrsw: u8,
    pub uses_implicit_point_line_varyings: u8,
    pub uses_pid: u8,
    /// Bit n: the program writes colour to render target n (0..3); 0 for
    /// a program that writes no colour (a depth pre-pass).
    pub rt_mask: u8,
}

/// The vertex or coordinate shader of a linked program
/// (`struct shc_v3d_vs`): the fields of the driver's `struct v3d_vs_prog`
/// that are not addresses.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcV3dVs {
    pub code_off: u32,
    pub code_bytes: u32,
    pub uniforms: *const ShcV3dUniform,
    pub num_uniforms: u32,
    pub vpm_input_size: u32,
    pub vpm_output_size: u32,
    pub vcm_cache_size: u32,
    pub threads: u8,
    pub single_seg: u8,
    pub separate_segments: u8,
    pub uses_vid: u8,
    pub uses_iid: u8,
    pub uses_biid: u8,
    pub writes_psiz: u8,
    /// Per attribute record: the components this program reads.
    pub vattr_sizes: [u8; 16],
}

/// The V3D half of a compiled shader (`struct shc_v3d`). All zero on a
/// compiled stage; filled on the fragment shader a link produced.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcV3d {
    /// 1 on a linked program, 0 on a compiled stage.
    pub linked: u8,
    /// The primitive class the program was compiled for (`SHC_V3D_PRIM_*`).
    pub prim: u8,
    /// Attribute records the program fetches (0..16).
    pub num_records: u8,
    /// Per GL attribute location: the attribute record it is fetched
    /// through, or [`SHC_V3D_NO_RECORD`].
    pub record_of_location: [u8; 16],
    pub fs: ShcV3dFs,
    pub vs: ShcV3dVs,
    pub cs: ShcV3dVs,
}

/// `ShcV3d::prim`: triangles (the program links for these; lines use it too).
pub const SHC_V3D_PRIM_TRIANGLES: u8 = 0;

impl ShcV3dFs {
    pub const ZERO: ShcV3dFs = ShcV3dFs {
        code_off: 0,
        code_bytes: 0,
        uniforms: core::ptr::null(),
        num_uniforms: 0,
        num_inputs: 0,
        flat_shade_flags: 0,
        noperspective_flags: 0,
        centroid_flags: 0,
        threads: 0,
        single_seg: 0,
        writes_z: 0,
        disable_ez: 0,
        uses_center_w: 0,
        lock_scoreboard_on_first_thrsw: 0,
        uses_implicit_point_line_varyings: 0,
        uses_pid: 0,
        rt_mask: 0,
    };

    /// The uniform stream.
    pub fn uniform_stream(&self) -> &[ShcV3dUniform] {
        if self.uniforms.is_null() {
            return &[];
        }
        // SAFETY: the linked program's arrays hold `num_uniforms` entries.
        unsafe { core::slice::from_raw_parts(self.uniforms, self.num_uniforms as usize) }
    }
}

impl ShcV3dVs {
    pub const ZERO: ShcV3dVs = ShcV3dVs {
        code_off: 0,
        code_bytes: 0,
        uniforms: core::ptr::null(),
        num_uniforms: 0,
        vpm_input_size: 0,
        vpm_output_size: 0,
        vcm_cache_size: 0,
        threads: 0,
        single_seg: 0,
        separate_segments: 0,
        uses_vid: 0,
        uses_iid: 0,
        uses_biid: 0,
        writes_psiz: 0,
        vattr_sizes: [0; 16],
    };

    /// The uniform stream.
    pub fn uniform_stream(&self) -> &[ShcV3dUniform] {
        if self.uniforms.is_null() {
            return &[];
        }
        // SAFETY: as for ShcV3dFs::uniform_stream.
        unsafe { core::slice::from_raw_parts(self.uniforms, self.num_uniforms as usize) }
    }
}

impl ShcV3d {
    pub const ZERO: ShcV3d = ShcV3d {
        linked: 0,
        prim: SHC_V3D_PRIM_TRIANGLES,
        num_records: 0,
        record_of_location: [SHC_V3D_NO_RECORD; 16],
        fs: ShcV3dFs::ZERO,
        vs: ShcV3dVs::ZERO,
        cs: ShcV3dVs::ZERO,
    };
}

/// `struct shc_shader`: the compiled-shader ABI the backend wires up.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShcShader {
    pub usc: *const u32,
    pub usc_dw: u32,
    pub temps: u16,
    /// Fragment: the inputs; vertex: the outputs (name -> vtxout_off).
    pub varyings: [ShcVarying; 16],
    pub num_varyings: u8,
    pub uses_w: u8,
    pub uses_discard: u8,
    /// Fragment, GC7000 only: the shader reads `gl_FrontFacing`, whose value
    /// the hardware hands over as a raw 0.0/1.0 with no sense of its own.
    /// There the sense is compiled in, so such a shader has a second binary
    /// under the same sha256 with blend key [`SHC_VIV_KEY_FRONT_FLIPPED`], and
    /// the GL core asks for it when the effective winding is reversed. The
    /// PowerVR needs none of this: `fs_front_face_off1` below names a shared
    /// register the driver writes the sense into at draw time.
    pub reads_front_facing: u8,
    pub meta_off: u16,
    /// `gl_FrontFacing`: the shared register the driver writes the front-face
    /// op into (0 keep the ISP's facing, 1 swap it, 2 always front), plus one
    /// (0 = the shader does not read `gl_FrontFacing`).
    pub fs_front_face_off1: u16,
    pub num_outputs: u8,
    /// Fragment: the outputs that did not fit the pixel's four output
    /// registers (three RGBA16F attachments want six) are stored to tile
    /// buffers instead, in output order after the ones that fit. The driver
    /// writes each buffer's address into two shared registers starting at
    /// `tile_buffer_sh`, and the end-of-tile program loads the data back
    /// before emitting that attachment. 0 = every output is a register.
    pub num_tile_buffers: u8,
    pub tile_buffer_sh: u16,
    pub attribs: [ShcAttrib; 16],
    pub num_attribs: u8,
    pub vs_output_size: u8,
    pub vs_psize_off: u8,
    /// OpenGL ES 3.0 gl_VertexID / gl_InstanceID: the vtxin register the
    /// compiler reads them from, plus one (0 = the shader reads none).
    pub vs_vertex_id_reg1: u8,
    pub vs_instance_id_reg1: u8,
    pub uniform_dwords: u16,
    /// The prefix of that block the compiled shader actually reads (a
    /// contiguous `[0, N)` DMA). The application writes the whole block, but
    /// only this much has to reach the shared registers. 0 = the whole block.
    pub uniform_sh_dwords: u16,
    pub uniforms: *const ShcUniform,
    pub num_uniforms: u8,
    pub blend_key: u32,
    pub uses_fbfetch: u8,
    pub blend_consts_off: u8,
    /// Fragment, PowerVR: the shader reads `gl_PointCoord`, which pco
    /// iterates in the eight coefficients after W's four (from 0 without W),
    /// before every varying; the driver feeds them with a point-sprite
    /// iterator. See `include/gles_shaderc.h`.
    pub uses_pntc: u8,
    /// 1: the fragment shader reads `gl_FragCoord.z`, iterated from a Z
    /// coefficient set at cf0..3 before W and every other input, which the
    /// driver's coefficient program must set up first. See
    /// `include/gles_shaderc.h`.
    pub uses_z: u8,
    /// Fragment, PowerVR, online compiler: one-based shared register of the
    /// (scale, offset) pair the program applies to `gl_FragCoord.y`, which
    /// the driver writes per draw from `GlesDraw::flip_y`; 0 = none. See
    /// `include/gles_shaderc.h`.
    pub fragcoord_ytf_off1: u16,
    /// PowerVR, online compiler: the program samples a samplerCube as the
    /// TPU's own cube, so the draw binds the twiddled copy
    /// (`gl::native_cube`). See `include/gles_shaderc.h`.
    pub native_cube: u8,
    /// Vertex: the back end could not lay the inputs out without the
    /// program's bindings (more attributes, or more input registers, than
    /// it can place unaided) and compiled the shader for its reflection
    /// only. The shader reports COMPILE_STATUS true; glLinkProgram compiles
    /// it again with the layout (`ShcBackend::compile_vs_linked`), and a
    /// failure there is a link error.
    pub layout_at_link: u8,
    /// With `layout_at_link`: every attribute the source declares (location
    /// [`SHC_ATTRIB_NO_LOCATION`]); `attribs` holds at most 16. Null
    /// otherwise.
    pub declared: *const ShcAttribBinding,
    pub num_declared: u8,
    /// The GC7000 register wiring and constant file ([`ShcViv`]);
    /// `varyings`/`vs_output_size` and the blend fields stay unused there.
    #[cfg(feature = "gpu-vivante")]
    pub viv: ShcViv,
    /// The V3D program a link produced ([`ShcV3d`]).
    #[cfg(feature = "gpu-v3d")]
    pub v3d: ShcV3d,
    pub priv_: *const c_void,
}

impl ShcShader {
    pub const ZERO: ShcShader = ShcShader {
        usc: core::ptr::null(),
        usc_dw: 0,
        temps: 0,
        varyings: [ShcVarying::ZERO; 16],
        num_varyings: 0,
        uses_w: 0,
        uses_discard: 0,
        reads_front_facing: 0,
        meta_off: 0,
        fs_front_face_off1: 0,
        num_outputs: 0,
        num_tile_buffers: 0,
        tile_buffer_sh: 0,
        attribs: [ShcAttrib::ZERO; 16],
        num_attribs: 0,
        vs_output_size: 0,
        vs_psize_off: 0,
        vs_vertex_id_reg1: 0,
        vs_instance_id_reg1: 0,
        uniform_dwords: 0,
        uniform_sh_dwords: 0,
        uniforms: core::ptr::null(),
        num_uniforms: 0,
        blend_key: 0,
        uses_fbfetch: 0,
        blend_consts_off: 0,
        uses_pntc: 0,
        uses_z: 0,
        fragcoord_ytf_off1: 0,
        native_cube: 0,
        layout_at_link: 0,
        declared: core::ptr::null(),
        num_declared: 0,
        #[cfg(feature = "gpu-vivante")]
        viv: ShcViv::ZERO,
        #[cfg(feature = "gpu-v3d")]
        v3d: ShcV3d::ZERO,
        priv_: core::ptr::null(),
    };

    /// The USC instruction dwords.
    pub fn usc_words(&self) -> &[u32] {
        if self.usc.is_null() {
            &[]
        } else {
            // SAFETY: usc/usc_dw describe a static table (or backend blob).
            unsafe { core::slice::from_raw_parts(self.usc, self.usc_dw as usize) }
        }
    }

    /// The name-reflection table (block members + samplers).
    pub fn uniform_table(&self) -> &[ShcUniform] {
        if self.uniforms.is_null() {
            &[]
        } else {
            // SAFETY: uniforms/num_uniforms describe a static table.
            unsafe { core::slice::from_raw_parts(self.uniforms, self.num_uniforms as usize) }
        }
    }

    /// The texture bindings the stage samples, each with its sampler's GLSL
    /// type: one per sampler and one per ELEMENT of an array of samplers
    /// (element k of a row reads binding `binding + k`, the online compiler's
    /// `reflect::sampler_row`). The rows are the active samplers: the online
    /// compiler keeps only the statically used ones (`reflect::uniform_rows`).
    /// The draw wires one descriptor per item and the link counts them against
    /// the GL limits, so the two agree by construction.
    pub fn sampler_bindings(&self) -> impl Iterator<Item = (u8, u16)> + '_ {
        self.uniform_table()
            .iter()
            .filter(|su| su.dword_off == SHC_UNIFORM_NO_BLOCK)
            .flat_map(|su| (0..su.array_len.max(1)).map(move |k| (su.binding.saturating_add(k), su.gl_type)))
    }
}

/// `struct shc_backend`.
#[repr(C)]
pub struct ShcBackend {
    /// Compile one stage; 0 and `*out` filled, or < 0 and a log.
    pub compile: Option<
        unsafe extern "C" fn(
            stage: u32,
            glsl: *const c_char,
            out: *mut ShcShader,
            log: *mut c_char,
            log_sz: usize,
        ) -> i32,
    >,
    /// Match the FS inputs to the VS outputs; patches the FS copy's vtxout_off.
    pub link:
        Option<unsafe extern "C" fn(vs: *const ShcShader, fs: *mut ShcShader, log: *mut c_char, log_sz: usize) -> i32>,
    /// The variant of a compiled FS for one blend key; 0 and `*out` filled,
    /// [`VARIANT_NO_ROOM`] when the compiler had no room for it now (it may
    /// succeed once something is freed), any other value < 0 when the
    /// variant cannot be had (-ENOENT: the offline table has none). On
    /// failure the log, if not null, says why.
    pub blend_variant: Option<
        unsafe extern "C" fn(
            fs: *const ShcShader,
            blend_key: u32,
            out: *mut ShcShader,
            log: *mut c_char,
            log_sz: usize,
        ) -> i32,
    >,
    /// Compile a vertex shader with the program's attribute layout
    /// ([`ShcAttribMap`]), at glLinkProgram; 0 and `*out` filled (its
    /// `attribs` rows at the map's locations), or < 0 and a log. Called when
    /// the compile set `layout_at_link` or the bindings alias. `None`: the
    /// back end has no such path (the offline table cannot recompile), and such a program links as before or,
    /// with `layout_at_link`, fails to link.
    pub compile_vs_linked: Option<
        unsafe extern "C" fn(
            glsl: *const c_char,
            map: *const ShcAttribMap,
            out: *mut ShcShader,
            log: *mut c_char,
            log_sz: usize,
        ) -> i32,
    >,
    /// Which texture bindings' filter state the shader's code depends on, as
    /// a [`LodKey`] (`uint64_t`, [`LodKey::bits`]); 0 for a shader that
    /// depends on none or is not this back end's. Asked once per program,
    /// the first time it is drawn after its link. `None`: no variants (the
    /// offline table).
    pub lod_sensitivity: Option<unsafe extern "C" fn(sh: *const ShcShader) -> u64>,
    /// The shader compiled again for `key` (a [`LodKey`], within its
    /// sensitivity), everything else as the input was compiled (the attribute
    /// map, the constant W); 0 and `*out` filled, which the caller releases
    /// like a blend variant, or < 0. A fragment result is linked by the
    /// caller (`link`) as a blend variant is; its blend variants inherit the
    /// key. Key 0 copies the input. Failures as for `blend_variant`.
    pub sampler_variant: Option<
        unsafe extern "C" fn(
            sh: *const ShcShader,
            key: u64,
            out: *mut ShcShader,
            log: *mut c_char,
            log_sz: usize,
        ) -> i32,
    >,
}

/// What `blend_variant` and `sampler_variant` return when the compiler had no
/// room for the variant now: the caller may ask again once something has been
/// freed. Every other failure is final for that key.
pub const VARIANT_NO_ROOM: i32 = -crate::util::ENOMEM;

/// The shader back end of this build (`extern const struct shc_backend *g_shaderc;`).
/* The initialiser decides what the linker must keep. Naming the offline back
 * end here is what pulls the whole table into an image, so only a build that
 * asked for the Mesa reference as its table starts there; every other starts
 * at the in-process compiler. */
#[cfg(mesa_reference)]
#[no_mangle]
pub static mut g_shaderc: *const ShcBackend = &offline::SHC_OFFLINE;
/// See above: with no table there is nothing to fall back to, so the
/// in-process compiler is the back end from the first call.
#[cfg(all(not(mesa_reference), feature = "shaderc-online"))]
#[no_mangle]
pub static mut g_shaderc: *const ShcBackend = &online::SHC_ONLINE;
/// An image with neither: the precompiled shaders are the whole back end.
#[cfg(all(not(mesa_reference), not(feature = "shaderc-online"), feature = "precompiled-shaders"))]
#[no_mangle]
pub static mut g_shaderc: *const ShcBackend = &precompiled::SHC_PRECOMPILED;

/// The active shader back end.
///
/// With `shaderc-online` compiled in, the first call installs the in-process
/// compiler ([`online::select_once`]). Without it this is the table.
#[inline]
pub fn backend() -> Option<&'static ShcBackend> {
    #[cfg(feature = "shaderc-online")]
    online::select_once();
    // SAFETY: g_shaderc is null or points at a static table.
    unsafe { g_shaderc.as_ref() }
}

/// The GL core keeps one more copy of a compiled shader: the shader object's,
/// a program's snapshot at link, or a blend variant's.
///
/// Every copy the GL core keeps is paired with one [`release_shader`]. The
/// in-process compiler counts them: its output lives in a bounded region
/// that must be given back. It recognises its own shaders by where `priv`
/// points, so this is safe to call on any shader whichever back end is
/// current: the table's live in the image and own nothing.
#[inline]
pub fn retain_shader(sh: &ShcShader) {
    #[cfg(feature = "shaderc-online")]
    online::backend::retain(sh);
    let _ = sh;
}

/// The GL core lets go of one copy of a compiled shader (see
/// [`retain_shader`]). A zeroed `ShcShader` is nothing to release.
#[inline]
pub fn release_shader(sh: &ShcShader) {
    #[cfg(feature = "shaderc-online")]
    online::backend::release(sh);
    let _ = sh;
}

/* ---- the offline table's row types (blobs.rs is generated) ---------------- */

/// One precompiled shader: the SHA-256 of its GLSL source, the stage, and the
/// fully populated [`ShcShader`] (USC dwords + wiring metadata).
pub struct ShcBlob {
    pub sha256: [u8; 32],
    /// Source basename, for logs. NUL-terminated, so that
    /// [`shc_shader_source_name`] can hand C a `const char *` into the table
    /// (use [`ShcBlob::source_name`] on this side).
    pub name: &'static str,
    pub stage: u32,
    /// FS: the `shc_blend_key` this variant was built for (0 = none).
    pub blend_key: u32,
    pub shader: ShcShader,
}

impl ShcBlob {
    /// The source basename without the trailing NUL.
    pub fn source_name(&self) -> &'static str {
        match self.name.as_bytes().iter().position(|&b| b == 0) {
            Some(n) => &self.name[..n],
            None => self.name,
        }
    }
}

// SAFETY: the raw pointers in `shader` point at immutable statics.
unsafe impl Sync for ShcBlob {}

/// The offline table's row behind a compiled shader: `Some` only when its
/// `priv` points at a row of this build's table (what the offline back end
/// sets). Another back end's `priv` is its own: the in-process compiler's
/// points at its record in the keep region, which read as a table row gave
/// a name pointer of whatever bytes were there (a crash in the backend's
/// debug log, the first draw of an online image with `gl_debug=1`).
pub fn table_row_of(sh: &ShcShader) -> Option<&'static ShcBlob> {
    #[cfg(mesa_reference)]
    {
        let rows: &'static [ShcBlob] = &mesa_reference::SHC_BLOB_TABLE[..];
        let size = core::mem::size_of::<ShcBlob>();
        let off = (sh.priv_ as usize).checked_sub(rows.as_ptr() as usize)?;
        if off % size == 0 {
            return rows.get(off / size);
        }
    }
    let _ = sh;
    None
}

/// The source basename behind a compiled shader ("" when it is not a row of
/// the offline table: see [`table_row_of`]).
pub fn shader_source_name(sh: &ShcShader) -> &'static str {
    table_row_of(sh).map_or("", |b| b.source_name())
}

/// `shc_shader_source_name()`: the same for C, as a `const char *` into the
/// table (the empty string when the shader did not come from it).
///
/// # Safety
/// `sh` is null or a compiled shader this library handed out.
#[no_mangle]
pub unsafe extern "C" fn shc_shader_source_name(sh: *const ShcShader) -> *const c_char {
    // SAFETY: the caller's contract.
    let name = match unsafe { sh.as_ref() } {
        Some(sh) => shader_source_name_c(sh),
        None => "\0",
    };
    name.as_ptr() as *const c_char
}

/// The NUL-terminated table name behind `sh` ("\0" when there is none).
fn shader_source_name_c(sh: &ShcShader) -> &'static str {
    match table_row_of(sh) {
        Some(b) if b.name.as_bytes().last() == Some(&0) => b.name,
        _ => "\0",
    }
}

/// One link a precompiled table recorded (`SHC_PRECOMPILED_LINKS`,
/// yakogl-precompile): the program's vertex shader and fragment shader by
/// record key, and the record key of the fragment shader the link left in the
/// program -- the same as `fs` when it substituted nothing, another row when
/// it compiled the fragment shader again (a varying the vertex shader never
/// writes read as 0, or for a constant `gl_Position.w`). The link verdict
/// without the front end: `shaderc::precompiled`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShcPrecompiledLink {
    pub vs: [u8; 32],
    pub fs: [u8; 32],
    /// The fragment shader's blend key (a variant is linked as its base).
    pub fs_blend_key: u32,
    pub linked_fs: [u8; 32],
}

/// The GLSL text a blob was built from, by source basename: what an
/// application must pass to glShaderSource byte for byte on an offline-only
/// target (`gles_offline_shader_source`). `text` ends in a NUL.
pub struct ShcSource {
    pub name: &'static str,
    pub text: &'static str,
}

/* ---- the precompiled table's row types (yakogl-precompile writes them) ---- */

/// What every record the compiler made from one source agrees on: its
/// reflection. A precompiled table holds one per source, not one per row.
///
/// A [`ShcShader`] is one of these plus a [`ShcPrecompiledRow`], and
/// [`precompiled::materialize`](precompiled::materialize) is the only thing
/// that joins them, so the tier and the proof read the same fields. What
/// proves the split loses nothing: `yakogl-precompile` refuses a source whose
/// rows disagree on a field stored here (its own compile of the source says
/// it once), and the library's test compares every row and every variant the
/// table recorded against what the compiler makes of it with the table out of
/// the way ([`precompiled::materialize`](precompiled::materialize) included).
#[derive(Clone, Copy)]
pub struct ShcPrecompiledReflection {
    /// The lists below hold the source's own entries, unpadded; a
    /// [`ShcShader`] gets the fixed array of [`ShcShader::varyings`] and
    /// [`ShcShader::attribs`], and its counts are these lengths.
    pub varyings: &'static [ShcVarying],
    pub attribs: &'static [ShcAttrib],
    pub uniforms: &'static [ShcUniform],
    pub declared: &'static [ShcAttribBinding],
    pub uses_w: u8,
    pub uses_discard: u8,
    pub reads_front_facing: u8,
    pub meta_off: u16,
    pub fs_front_face_off1: u16,
    pub num_tile_buffers: u8,
    pub tile_buffer_sh: u16,
    pub vs_output_size: u8,
    pub vs_psize_off: u8,
    pub vs_vertex_id_reg1: u8,
    pub vs_instance_id_reg1: u8,
    pub uniform_sh_dwords: u16,
    pub uses_pntc: u8,
    pub uses_z: u8,
    pub fragcoord_ytf_off1: u16,
    pub native_cube: u8,
    pub layout_at_link: u8,
    /// The source's uniform blocks: what its `uniform` declarations are, which
    /// no variant of it changes.
    pub blocks: ShcBlocks,
}

/// One row of a precompiled table: the variant. Its code and the wiring that
/// differs between a source's variants; everything else the source's
/// [`ShcPrecompiledReflection`] already says.
#[derive(Clone, Copy)]
pub struct ShcPrecompiledRow {
    /// Its source: an index into `SHC_PRECOMPILED_SOURCES` (what the table's
    /// `SHC_PRECOMPILED_SOURCE` column used to be).
    pub source: u16,
    /// What the compiler made of that source, shared by all of its rows.
    pub reflection: &'static ShcPrecompiledReflection,
    pub stage: u32,
    /// The identity [`precompiled::find`](precompiled::find) matches: FS, the
    /// `shc_blend_key` this variant was built for (0 = none).
    pub blend_key: u32,
    /// What the compiler wrote in the shader it made ([`ShcShader::blend_key`]),
    /// which [`precompiled::materialize`](precompiled::materialize) hands out.
    /// Not always the number above: the PowerVR records the variant's key
    /// there, the GC7000 leaves it 0 and puts the flip in its wiring, so a row
    /// carries both.
    pub shader_blend_key: u32,
    pub usc: &'static [u32],
    pub temps: u16,
    pub num_outputs: u8,
    pub uniform_dwords: u16,
    pub uses_fbfetch: u8,
    pub blend_consts_off: u8,
    /// The GC7000 register wiring and constant file. Per row, not per source:
    /// `ShcViv::vs_reg_of_in` is filled by a link, so it belongs to the
    /// program the source was linked into.
    #[cfg(feature = "gpu-vivante")]
    pub viv: ShcViv,
}

// SAFETY: the raw pointers point at immutable statics.
unsafe impl Sync for ShcPrecompiledRow {}

/// Pad a generated varying list to the fixed array of [`ShcShader`].
pub const fn pad_varyings<const N: usize>(v: [ShcVarying; N]) -> [ShcVarying; 16] {
    let mut out = [ShcVarying::ZERO; 16];
    let mut i = 0;
    while i < N {
        out[i] = v[i];
        i += 1;
    }
    out
}

/// Pad a generated register list to the fixed array of [`ShcViv`].
///
/// Beside `pad_attribs`/`pad_varyings` rather than in the generated table,
/// because `src/vivante/internal.rs` uses it and the clear shaders must
/// exist in a build that carries no table at all.
#[cfg(feature = "gpu-vivante")]
pub const fn pad_io<const N: usize>(io: [ShcVivIo; N]) -> [ShcVivIo; SHC_VIV_MAX_IO] {
    let mut out = [ShcVivIo::ZERO; SHC_VIV_MAX_IO];
    let mut i = 0;
    while i < N {
        out[i] = io[i];
        i += 1;
    }
    out
}

/// Pad a generated attribute list to the fixed array of [`ShcShader`].
pub const fn pad_attribs<const N: usize>(a: [ShcAttrib; N]) -> [ShcAttrib; 16] {
    let mut out = [ShcAttrib::ZERO; 16];
    let mut i = 0;
    while i < N {
        out[i] = a[i];
        i += 1;
    }
    out
}
