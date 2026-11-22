// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 per-draw state packets (roadmap P9; the Pi 5's V3D 7.1.10).
//!
//! A pure module over plain integers, floats and bools; no `crate::` import,
//! so a test includes this file with `#[path]` (tests/v3d71_draw.rs). Nothing
//! hooks it into `src/v3d/mod.rs` yet: the 7.1 table (backend71) does that and
//! calls [`translate`].
//!
//! From a plain description of the GL draw state ([`DrawState`]) it gives the
//! field values of each state packet of the binner list as structs, and each
//! struct packs to the bytes the control list holds (`pack`):
//!
//! | struct | packet (`X:` = cle/v3d_packet.xml) | opcode | bytes |
//! |---|---|---|---|
//! | [`CfgBits`] | `Cfg Bits`, min_ver 71 (`X:849`) | 96 | 4 |
//! | [`BlendEnables`] | `Blend Enables` (`X:773`) | 83 | 2 |
//! | [`BlendCfg`] | `Blend Cfg`, min_ver 71 (`X:787`) | 84 | 5 |
//! | [`ColorWriteMasks`] | `Color Write Masks` (`X:804`) | 87 | 5 |
//! | [`StencilCfg`] | `Stencil Cfg` (`X:761`) | 80 | 6 |
//! | [`ClipWindow`] | `clip_window` (`X:900`) | 107 | 9 |
//! | [`ViewportOffset`] | `Viewport Offset` (`X:907`) | 108 | 9 |
//! | [`ClipperZMinMax`] | `Clipper Z min/max clipping planes` (`X:914`) | 109 | 9 |
//! | [`ClipperXy`] | `Clipper XY Scaling`, min_ver 71 (`X:924`) | 110 | 9 |
//! | [`ClipperZ`] | `Clipper Z Scale and Offset` (`X:929`) | 111 | 9 |
//! | [`DepthOffset`] | `Depth Offset` (`X:894`) | 106 | 9 |
//!
//! What 7.1 changed, against the 4.2 packets of the same name:
//! * `Cfg Bits` lost `Early Z enable` and `Early Z updates enable` (bits 16
//!   and 17) and gained `Z Clipping mode` (22, 2 bits), `Z Clamp Mode` (10),
//!   `Depth Bounds Test Enable` (5); `Line Rasterization` is 1 bit (4). See
//!   [`EarlyZ71`] for the early-Z rules that replace the lost bits
//!   (docs/notes/v3d71-record-u11.md section 4);
//! * `Blend Cfg`'s `Render Target Mask` is 8 bits (24..31): a blend
//!   configuration covers up to 8 targets;
//! * `Color Write Masks` is 32 bits as at 4.2, four "disable" bits per
//!   target: eight targets fill it;
//! * `Clipper XY Scaling` is in 1/64 pixel (4.2: 1/256).
//!
//! Left out, as other items' or constant on the six shapes: `Blend Constant
//! Color` (86), `Sample State` (91), `Point size` (104), `Line width` (105),
//! the flat-shade, non-perspective and centroid flag packets (97-100,
//! 88-89), `Transform Feedback Specs`, `Occlusion Query Counter` (92): their
//! layouts are the 4.2 ones and the C builder (driver/src/v3d/v3d_job71.c
//! emit_state) writes them. `Depth Bounds Test Limits` (93) and `Clipper Z
//! Scale and Offset no guardband` (112) are 7.1-only and GL ES has no state
//! for them (the second is a named refusal, [`Refusal71::DepthScaleTooSmall`]).
//!
//! The order a draw emits them in (Mesa's `v3dX(emit_state)`, as the C
//! builder and tools/v3d/pi5/*.clif show): CLIP, CFG_BITS, DEPTH_OFFSET (only
//! with polygon offset), [point size, line width], CLIPPER_XY, CLIPPER_Z,
//! CLIPZ, VIEWPORT_OFFSET, BLEND_ENABLES, BLEND_CFG (enabled targets only),
//! COLOR_WMASKS, [blend constant colour], STENCIL_CFG (front, then back when
//! two-sided; only with the stencil test), [flag packets, sample state].
//!
//! A clear that Mesa (and so this table) cannot do in the tile buffer (a
//! scissor box, a partial colour mask, a stencil write mask) is a draw of a
//! quad whose geometry is the box: [`DrawState::blitter_clear`] is that
//! draw's state. Its viewport is the whole target and its scissor is off, so
//! the clip window is the whole target in every dump of tools/v3d/pi5/ (the
//! box is in the vertices); it is never bounded by the GL viewport (gl_test
//! case 66).
//!
//! Unsupported state is a named [`Refusal71`] ("V3D 7.1: ...").

/// One field of a packet: the name, bit position (after the 8-bit opcode)
/// and width of v3d_packet.xml, and the XML `type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub start: u32,
    pub size: u32,
    pub ty: &'static str,
}

const fn fl(name: &'static str, start: u32, size: u32, ty: &'static str) -> Field {
    Field { name, start, size, ty }
}

/// A packet's layout at 7.1.
#[derive(Clone, Copy, Debug)]
pub struct PacketSpec {
    /// `name=` in v3d_packet.xml
    pub xml_name: &'static str,
    /// the name the CLIF dump prints
    pub clif_name: &'static str,
    /// `V3D71_<c_name>_length` in the generated packer
    pub c_name: &'static str,
    pub code: u8,
    pub fields: &'static [Field],
}

impl PacketSpec {
    /// The packet's length in bytes, the opcode included.
    pub const fn length(&self) -> usize {
        let mut end = 0;
        let mut i = 0;
        while i < self.fields.len() {
            let e = (self.fields[i].start + self.fields[i].size) as usize;
            if e > end {
                end = e;
            }
            i += 1;
        }
        1 + end.div_ceil(8)
    }

    /// The field called `name`; panics for a name the packet lacks (a bug of
    /// this file, which the test of every packet's layout finds).
    pub fn field(&self, name: &str) -> &'static Field {
        let mut i = 0;
        while i < self.fields.len() {
            if self.fields[i].name.len() == name.len() && eq(self.fields[i].name, name) {
                return &self.fields[i];
            }
            i += 1;
        }
        panic!("draw71: packet has no such field")
    }
}

const fn eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

pub static CFG_BITS_SPEC: PacketSpec = PacketSpec {
    xml_name: "Cfg Bits",
    clif_name: "CFG_BITS",
    c_name: "CFG_BITS",
    code: 96,
    fields: &[
        fl("Z Clipping mode", 22, 2, "Z Clip Mode"),
        fl("Direct3D Provoking Vertex", 21, 1, "bool"),
        fl("Direct3D 'Point-fill' mode", 20, 1, "bool"),
        fl("Blend enable", 19, 1, "bool"),
        fl("Stencil enable", 18, 1, "bool"),
        fl("Z updates enable", 15, 1, "bool"),
        fl("Depth-Test Function", 12, 3, "Compare Function"),
        fl("Direct3D Wireframe triangles mode", 11, 1, "bool"),
        fl("Z Clamp Mode", 10, 1, "bool"),
        fl("Rasterizer Oversample Mode", 6, 2, "uint"),
        fl("Depth Bounds Test Enable", 5, 1, "bool"),
        fl("Line Rasterization", 4, 1, "uint"),
        fl("Enable Depth Offset", 3, 1, "bool"),
        fl("Clockwise Primitives", 2, 1, "bool"),
        fl("Enable Reverse Facing Primitive", 1, 1, "bool"),
        fl("Enable Forward Facing Primitive", 0, 1, "bool"),
    ],
};

pub static BLEND_ENABLES_SPEC: PacketSpec = PacketSpec {
    xml_name: "Blend Enables",
    clif_name: "BLEND_ENABLES",
    c_name: "BLEND_ENABLES",
    code: 83,
    fields: &[fl("Mask", 0, 8, "uint")],
};

pub static BLEND_CFG_SPEC: PacketSpec = PacketSpec {
    xml_name: "Blend Cfg",
    clif_name: "BLEND_CFG",
    c_name: "BLEND_CFG",
    code: 84,
    fields: &[
        fl("Render Target Mask", 24, 8, "uint"),
        fl("Color blend dst factor", 20, 4, "Blend Factor"),
        fl("Color blend src factor", 16, 4, "Blend Factor"),
        fl("Color blend mode", 12, 4, "Blend Mode"),
        fl("Alpha blend dst factor", 8, 4, "Blend Factor"),
        fl("Alpha blend src factor", 4, 4, "Blend Factor"),
        fl("Alpha blend mode", 0, 4, "Blend Mode"),
    ],
};

pub static COLOR_WRITE_MASKS_SPEC: PacketSpec = PacketSpec {
    xml_name: "Color Write Masks",
    clif_name: "COLOR_WMASKS",
    c_name: "COLOR_WRITE_MASKS",
    code: 87,
    fields: &[fl("Mask", 0, 32, "uint")],
};

pub static STENCIL_CFG_SPEC: PacketSpec = PacketSpec {
    xml_name: "Stencil Cfg",
    clif_name: "STENCIL_CFG",
    c_name: "STENCIL_CFG",
    code: 80,
    fields: &[
        fl("Stencil Write Mask", 32, 8, "uint"),
        fl("Back Config", 29, 1, "bool"),
        fl("Front Config", 28, 1, "bool"),
        fl("Stencil Pass Op", 25, 3, "Stencil Op"),
        fl("Depth Test Fail Op", 22, 3, "Stencil Op"),
        fl("Stencil Test Fail Op", 19, 3, "Stencil Op"),
        fl("Stencil Test Function", 16, 3, "Compare Function"),
        fl("Stencil Test Mask", 8, 8, "uint"),
        fl("Stencil Ref Value", 0, 8, "uint"),
    ],
};

pub static CLIP_WINDOW_SPEC: PacketSpec = PacketSpec {
    xml_name: "clip_window",
    clif_name: "CLIP",
    c_name: "CLIP_WINDOW",
    code: 107,
    fields: &[
        fl("Clip Window Height in pixels", 48, 16, "uint"),
        fl("Clip Window Width in pixels", 32, 16, "uint"),
        fl("Clip Window Bottom Pixel Coordinate", 16, 16, "uint"),
        fl("Clip Window Left Pixel Coordinate", 0, 16, "uint"),
    ],
};

pub static VIEWPORT_OFFSET_SPEC: PacketSpec = PacketSpec {
    xml_name: "Viewport Offset",
    clif_name: "VIEWPORT_OFFSET",
    c_name: "VIEWPORT_OFFSET",
    code: 108,
    fields: &[
        fl("Coarse Y", 54, 10, "int"),
        fl("Fine Y", 32, 22, "u14.8"),
        fl("Coarse X", 22, 10, "int"),
        fl("Fine X", 0, 22, "u14.8"),
    ],
};

pub static CLIPPER_Z_MIN_MAX_SPEC: PacketSpec = PacketSpec {
    xml_name: "Clipper Z min/max clipping planes",
    clif_name: "CLIPZ",
    c_name: "CLIPPER_Z_MIN_MAX_CLIPPING_PLANES",
    code: 109,
    fields: &[fl("Maximum Zw", 32, 32, "float"), fl("Minimum Zw", 0, 32, "float")],
};

pub static CLIPPER_XY_SPEC: PacketSpec = PacketSpec {
    xml_name: "Clipper XY Scaling",
    clif_name: "CLIPPER_XY",
    c_name: "CLIPPER_XY_SCALING",
    code: 110,
    fields: &[
        fl("Viewport Half-Height in 1/64th of pixel", 32, 32, "float"),
        fl("Viewport Half-Width in 1/64th of pixel", 0, 32, "float"),
    ],
};

pub static CLIPPER_Z_SPEC: PacketSpec = PacketSpec {
    xml_name: "Clipper Z Scale and Offset",
    clif_name: "CLIPPER_Z",
    c_name: "CLIPPER_Z_SCALE_AND_OFFSET",
    code: 111,
    fields: &[fl("Viewport Z Offset (Zc to Zs)", 32, 32, "float"), fl("Viewport Z Scale (Zc to Zs)", 0, 32, "float")],
};

pub static DEPTH_OFFSET_SPEC: PacketSpec = PacketSpec {
    xml_name: "Depth Offset",
    clif_name: "DEPTH_OFFSET",
    c_name: "DEPTH_OFFSET",
    code: 106,
    fields: &[
        fl("Limit", 32, 32, "float"),
        fl("Depth Offset Units", 16, 16, "f187"),
        fl("Depth Offset Factor", 0, 16, "f187"),
    ],
};

/// Every packet of this module, for the test of the layouts.
pub static ALL_SPECS: [&PacketSpec; 11] = [
    &CFG_BITS_SPEC,
    &BLEND_ENABLES_SPEC,
    &BLEND_CFG_SPEC,
    &COLOR_WRITE_MASKS_SPEC,
    &STENCIL_CFG_SPEC,
    &CLIP_WINDOW_SPEC,
    &VIEWPORT_OFFSET_SPEC,
    &CLIPPER_Z_MIN_MAX_SPEC,
    &CLIPPER_XY_SPEC,
    &CLIPPER_Z_SPEC,
    &DEPTH_OFFSET_SPEC,
];

/// A field's value as the packet holds it: the integer, or the float (the
/// whole 32 bits of an f32 field).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    U(u64),
    I(i64),
    F(f32),
}

impl Val {
    /// The value as the CLIF dump prints it (`%u`, `%d`, `%f`).
    pub fn clif_text(&self) -> String {
        match *self {
            Val::U(v) => v.to_string(),
            Val::I(v) => v.to_string(),
            Val::F(v) => format!("{v:.6}"),
        }
    }
}

fn put(buf: &mut [u8], f: &Field, v: Val) {
    let bits: u64 = match v {
        Val::U(v) => v,
        Val::I(v) => (v as u64) & ((1u64 << f.size) - 1),
        Val::F(v) => {
            assert_eq!(f.size, 32, "draw71: a float field is 32 bits");
            u64::from(v.to_bits())
        }
    };
    assert!(f.size == 64 || bits < (1u64 << f.size), "draw71: value does not fit its field");
    for b in 0..f.size {
        if (bits >> b) & 1 != 0 {
            let at = 8 + f.start + b;
            buf[(at / 8) as usize] |= 1 << (at % 8);
        }
    }
}

fn pack_pairs<const N: usize>(spec: &PacketSpec, pairs: &[(&'static Field, Val)]) -> [u8; N] {
    assert_eq!(spec.length(), N);
    let mut out = [0u8; N];
    out[0] = spec.code;
    for (f, v) in pairs {
        put(&mut out, f, *v);
    }
    out
}

/// What every packet struct gives: its layout, its values by field and its
/// bytes.
pub trait Packet {
    const SPEC: &'static PacketSpec;
    /// Every field of the layout once, with its value.
    fn pairs(&self) -> Vec<(&'static Field, Val)>;
}

fn b(v: bool) -> Val {
    Val::U(u64::from(v))
}

fn u(v: u32) -> Val {
    Val::U(u64::from(v))
}

// ---- enums (v3d_packet.xml:3-60, :322) -----------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Compare {
    Never = 0,
    Less = 1,
    Equal = 2,
    LEqual = 3,
    Greater = 4,
    NotEqual = 5,
    GEqual = 6,
    Always = 7,
}

pub const COMPARES: [(Compare, &str); 8] = [
    (Compare::Never, "NEVER"),
    (Compare::Less, "LESS"),
    (Compare::Equal, "EQUAL"),
    (Compare::LEqual, "LEQUAL"),
    (Compare::Greater, "GREATER"),
    (Compare::NotEqual, "NOTEQUAL"),
    (Compare::GEqual, "GEQUAL"),
    (Compare::Always, "ALWAYS"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BlendFactor {
    Zero = 0,
    One = 1,
    SrcColor = 2,
    InvSrcColor = 3,
    DstColor = 4,
    InvDstColor = 5,
    SrcAlpha = 6,
    InvSrcAlpha = 7,
    DstAlpha = 8,
    InvDstAlpha = 9,
    ConstColor = 10,
    InvConstColor = 11,
    ConstAlpha = 12,
    InvConstAlpha = 13,
    SrcAlphaSaturate = 14,
}

pub const BLEND_FACTORS: [(BlendFactor, &str); 15] = [
    (BlendFactor::Zero, "ZERO"),
    (BlendFactor::One, "ONE"),
    (BlendFactor::SrcColor, "SRC_COLOR"),
    (BlendFactor::InvSrcColor, "INV_SRC_COLOR"),
    (BlendFactor::DstColor, "DST_COLOR"),
    (BlendFactor::InvDstColor, "INV_DST_COLOR"),
    (BlendFactor::SrcAlpha, "SRC_ALPHA"),
    (BlendFactor::InvSrcAlpha, "INV_SRC_ALPHA"),
    (BlendFactor::DstAlpha, "DST_ALPHA"),
    (BlendFactor::InvDstAlpha, "INV_DST_ALPHA"),
    (BlendFactor::ConstColor, "CONST_COLOR"),
    (BlendFactor::InvConstColor, "INV_CONST_COLOR"),
    (BlendFactor::ConstAlpha, "CONST_ALPHA"),
    (BlendFactor::InvConstAlpha, "INV_CONST_ALPHA"),
    (BlendFactor::SrcAlphaSaturate, "SRC_ALPHA_SATURATE"),
];

/// The blend modes GL ES 3.0 has (the XML also names MUL, SCREEN, DARKEN and
/// LIGHTEN, which no GL ES equation selects).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BlendMode {
    Add = 0,
    Sub = 1,
    RSub = 2,
    Min = 3,
    Max = 4,
}

pub const BLEND_MODES: [(BlendMode, &str); 5] = [
    (BlendMode::Add, "ADD"),
    (BlendMode::Sub, "SUB"),
    (BlendMode::RSub, "RSUB"),
    (BlendMode::Min, "MIN"),
    (BlendMode::Max, "MAX"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StencilOp {
    Zero = 0,
    Keep = 1,
    Replace = 2,
    Incr = 3,
    Decr = 4,
    Invert = 5,
    IncWrap = 6,
    DecWrap = 7,
}

pub const STENCIL_OPS: [(StencilOp, &str); 8] = [
    (StencilOp::Zero, "ZERO"),
    (StencilOp::Keep, "KEEP"),
    (StencilOp::Replace, "REPLACE"),
    (StencilOp::Incr, "INCR"),
    (StencilOp::Decr, "DECR"),
    (StencilOp::Invert, "INVERT"),
    (StencilOp::IncWrap, "INCWRAP"),
    (StencilOp::DecWrap, "DECWRAP"),
];

/// `Z Clip Mode`: the GL ES core uses `MinOneToOne` (clip space Z in -1..1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ZClipMode {
    None = 0,
    MinOneToOne = 1,
    ZeroToOne = 2,
}

pub const Z_CLIP_MODES: [(ZClipMode, &str); 3] =
    [(ZClipMode::None, "NONE"), (ZClipMode::MinOneToOne, "MIN_ONE_TO_ONE"), (ZClipMode::ZeroToOne, "ZERO_TO_ONE")];

// ---- GL enumerants -> the enums, with named refusals ----------------------

/// Why a state is not drawn: each is a refusal named "V3D 7.1: ...".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal71 {
    TargetSize,
    TooManyTargets,
    Multisample,
    UnknownCompare,
    UnknownBlendFactor,
    UnknownBlendEquation,
    UnknownStencilOp,
    BlendReadsMissingAlpha,
    ViewportNotFinite,
    DepthScaleTooSmall,
    ClipWindowRange,
    ViewportOffsetRange,
    OffsetNotFinite,
}

impl Refusal71 {
    pub const ALL: [Refusal71; 13] = [
        Refusal71::TargetSize,
        Refusal71::TooManyTargets,
        Refusal71::Multisample,
        Refusal71::UnknownCompare,
        Refusal71::UnknownBlendFactor,
        Refusal71::UnknownBlendEquation,
        Refusal71::UnknownStencilOp,
        Refusal71::BlendReadsMissingAlpha,
        Refusal71::ViewportNotFinite,
        Refusal71::DepthScaleTooSmall,
        Refusal71::ClipWindowRange,
        Refusal71::ViewportOffsetRange,
        Refusal71::OffsetNotFinite,
    ];

    pub fn message(self) -> &'static str {
        match self {
            Refusal71::TargetSize => "V3D 7.1: a render target side outside 1 to 7680 pixels",
            Refusal71::TooManyTargets => "V3D 7.1: more than 8 colour render targets",
            Refusal71::Multisample => "V3D 7.1: multisampled drawing (the 7.1 table has no 4x oversample state yet)",
            Refusal71::UnknownCompare => "V3D 7.1: a compare function that is not a GL one",
            Refusal71::UnknownBlendFactor => "V3D 7.1: a blend factor that is not a GL ES 3.0 one",
            Refusal71::UnknownBlendEquation => "V3D 7.1: a blend equation that is not a GL ES 3.0 one",
            Refusal71::UnknownStencilOp => "V3D 7.1: a stencil operation that is not a GL one",
            Refusal71::BlendReadsMissingAlpha => {
                "V3D 7.1: a blend factor reading destination alpha on a target without an alpha channel"
            }
            Refusal71::ViewportNotFinite => "V3D 7.1: a viewport that is not a finite box with a size of 0 or more",
            Refusal71::DepthScaleTooSmall => {
                "V3D 7.1: a depth range whose Z scale is under 0.01 (it needs the no-guardband clipper packet)"
            }
            Refusal71::ClipWindowRange => "V3D 7.1: a clip window outside 0 to 65535 pixels",
            Refusal71::ViewportOffsetRange => "V3D 7.1: a viewport offset outside the packet's range",
            Refusal71::OffsetNotFinite => "V3D 7.1: a polygon offset that is not a finite number",
        }
    }
}

/// glDepthFunc / glStencilFunc enumerant (GL_NEVER 0x200 .. GL_ALWAYS 0x207).
pub fn compare_from_gl(gl: u32) -> Result<Compare, Refusal71> {
    match gl.wrapping_sub(0x200) {
        0 => Ok(Compare::Never),
        1 => Ok(Compare::Less),
        2 => Ok(Compare::Equal),
        3 => Ok(Compare::LEqual),
        4 => Ok(Compare::Greater),
        5 => Ok(Compare::NotEqual),
        6 => Ok(Compare::GEqual),
        7 => Ok(Compare::Always),
        _ => Err(Refusal71::UnknownCompare),
    }
}

pub fn blend_factor_from_gl(gl: u32) -> Result<BlendFactor, Refusal71> {
    Ok(match gl {
        0 => BlendFactor::Zero,
        1 => BlendFactor::One,
        0x0300 => BlendFactor::SrcColor,
        0x0301 => BlendFactor::InvSrcColor,
        0x0302 => BlendFactor::SrcAlpha,
        0x0303 => BlendFactor::InvSrcAlpha,
        0x0304 => BlendFactor::DstAlpha,
        0x0305 => BlendFactor::InvDstAlpha,
        0x0306 => BlendFactor::DstColor,
        0x0307 => BlendFactor::InvDstColor,
        0x0308 => BlendFactor::SrcAlphaSaturate,
        0x8001 => BlendFactor::ConstColor,
        0x8002 => BlendFactor::InvConstColor,
        0x8003 => BlendFactor::ConstAlpha,
        0x8004 => BlendFactor::InvConstAlpha,
        _ => return Err(Refusal71::UnknownBlendFactor),
    })
}

pub fn blend_mode_from_gl(gl: u32) -> Result<BlendMode, Refusal71> {
    Ok(match gl {
        0x8006 => BlendMode::Add,
        0x800A => BlendMode::Sub,
        0x800B => BlendMode::RSub,
        0x8007 => BlendMode::Min,
        0x8008 => BlendMode::Max,
        _ => return Err(Refusal71::UnknownBlendEquation),
    })
}

pub fn stencil_op_from_gl(gl: u32) -> Result<StencilOp, Refusal71> {
    Ok(match gl {
        0 => StencilOp::Zero,
        0x1E00 => StencilOp::Keep,
        0x1E01 => StencilOp::Replace,
        0x1E02 => StencilOp::Incr,
        0x1E03 => StencilOp::Decr,
        0x150A => StencilOp::Invert,
        0x8507 => StencilOp::IncWrap,
        0x8508 => StencilOp::DecWrap,
        _ => return Err(Refusal71::UnknownStencilOp),
    })
}

// ---- the packets ----------------------------------------------------------

/// `Cfg Bits` at 7.1. The Direct3D and wireframe bits, `Z Clamp Mode`, the
/// depth-bounds test and the oversample mode are fields the GL ES core never
/// sets (they stay 0, as in every dump of tools/v3d/pi5/).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CfgBits {
    pub z_clipping_mode: ZClipMode,
    pub direct3d_provoking_vertex: bool,
    pub direct3d_point_fill_mode: bool,
    pub blend_enable: bool,
    pub stencil_enable: bool,
    pub z_updates_enable: bool,
    pub depth_test_function: Compare,
    pub direct3d_wireframe_triangles_mode: bool,
    pub z_clamp_mode: bool,
    pub rasterizer_oversample_mode: u8,
    pub depth_bounds_test_enable: bool,
    /// 0 diamond exit (GL's), 1 perpendicular end caps
    pub line_rasterization: u8,
    pub enable_depth_offset: bool,
    pub clockwise_primitives: bool,
    pub enable_reverse_facing_primitive: bool,
    pub enable_forward_facing_primitive: bool,
}

impl Packet for CfgBits {
    const SPEC: &'static PacketSpec = &CFG_BITS_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Z Clipping mode"), u(self.z_clipping_mode as u32)),
            (s.field("Direct3D Provoking Vertex"), b(self.direct3d_provoking_vertex)),
            (s.field("Direct3D 'Point-fill' mode"), b(self.direct3d_point_fill_mode)),
            (s.field("Blend enable"), b(self.blend_enable)),
            (s.field("Stencil enable"), b(self.stencil_enable)),
            (s.field("Z updates enable"), b(self.z_updates_enable)),
            (s.field("Depth-Test Function"), u(self.depth_test_function as u32)),
            (s.field("Direct3D Wireframe triangles mode"), b(self.direct3d_wireframe_triangles_mode)),
            (s.field("Z Clamp Mode"), b(self.z_clamp_mode)),
            (s.field("Rasterizer Oversample Mode"), u(u32::from(self.rasterizer_oversample_mode))),
            (s.field("Depth Bounds Test Enable"), b(self.depth_bounds_test_enable)),
            (s.field("Line Rasterization"), u(u32::from(self.line_rasterization))),
            (s.field("Enable Depth Offset"), b(self.enable_depth_offset)),
            (s.field("Clockwise Primitives"), b(self.clockwise_primitives)),
            (s.field("Enable Reverse Facing Primitive"), b(self.enable_reverse_facing_primitive)),
            (s.field("Enable Forward Facing Primitive"), b(self.enable_forward_facing_primitive)),
        ]
    }
}

impl CfgBits {
    pub fn pack(&self) -> [u8; 4] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Blend Enables`: bit n is render target n (8 targets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlendEnables {
    pub mask: u8,
}

impl Packet for BlendEnables {
    const SPEC: &'static PacketSpec = &BLEND_ENABLES_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        vec![(Self::SPEC.field("Mask"), u(u32::from(self.mask)))]
    }
}

impl BlendEnables {
    pub fn pack(&self) -> [u8; 2] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Blend Cfg` at 7.1: the equation for every target of `render_target_mask`
/// (8 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlendCfg {
    pub render_target_mask: u8,
    pub color_blend_dst_factor: BlendFactor,
    pub color_blend_src_factor: BlendFactor,
    pub color_blend_mode: BlendMode,
    pub alpha_blend_dst_factor: BlendFactor,
    pub alpha_blend_src_factor: BlendFactor,
    pub alpha_blend_mode: BlendMode,
}

impl Packet for BlendCfg {
    const SPEC: &'static PacketSpec = &BLEND_CFG_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Render Target Mask"), u(u32::from(self.render_target_mask))),
            (s.field("Color blend dst factor"), u(self.color_blend_dst_factor as u32)),
            (s.field("Color blend src factor"), u(self.color_blend_src_factor as u32)),
            (s.field("Color blend mode"), u(self.color_blend_mode as u32)),
            (s.field("Alpha blend dst factor"), u(self.alpha_blend_dst_factor as u32)),
            (s.field("Alpha blend src factor"), u(self.alpha_blend_src_factor as u32)),
            (s.field("Alpha blend mode"), u(self.alpha_blend_mode as u32)),
        ]
    }
}

impl BlendCfg {
    pub fn pack(&self) -> [u8; 5] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Color Write Masks`: four bits per target, target n at bits 4n..4n+3, a
/// set bit DISABLES the channel (bit 0 R, 1 G, 2 B, 3 A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorWriteMasks {
    pub mask: u32,
}

impl Packet for ColorWriteMasks {
    const SPEC: &'static PacketSpec = &COLOR_WRITE_MASKS_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        vec![(Self::SPEC.field("Mask"), u(self.mask))]
    }
}

impl ColorWriteMasks {
    pub fn pack(&self) -> [u8; 5] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Stencil Cfg`: one face configuration (`front_config`, `back_config`
/// say which faces it configures).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StencilCfg {
    pub stencil_write_mask: u8,
    pub back_config: bool,
    pub front_config: bool,
    pub stencil_pass_op: StencilOp,
    pub depth_test_fail_op: StencilOp,
    pub stencil_test_fail_op: StencilOp,
    pub stencil_test_function: Compare,
    pub stencil_test_mask: u8,
    pub stencil_ref_value: u8,
}

impl Packet for StencilCfg {
    const SPEC: &'static PacketSpec = &STENCIL_CFG_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Stencil Write Mask"), u(u32::from(self.stencil_write_mask))),
            (s.field("Back Config"), b(self.back_config)),
            (s.field("Front Config"), b(self.front_config)),
            (s.field("Stencil Pass Op"), u(self.stencil_pass_op as u32)),
            (s.field("Depth Test Fail Op"), u(self.depth_test_fail_op as u32)),
            (s.field("Stencil Test Fail Op"), u(self.stencil_test_fail_op as u32)),
            (s.field("Stencil Test Function"), u(self.stencil_test_function as u32)),
            (s.field("Stencil Test Mask"), u(u32::from(self.stencil_test_mask))),
            (s.field("Stencil Ref Value"), u(u32::from(self.stencil_ref_value))),
        ]
    }
}

impl StencilCfg {
    pub fn pack(&self) -> [u8; 6] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `clip_window` (the clipper's pixel rectangle; Y from the bottom row of
/// the target's tile memory, as the viewport and scissor of the caller are).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipWindow {
    pub left: u16,
    pub bottom: u16,
    pub width: u16,
    pub height: u16,
}

impl Packet for ClipWindow {
    const SPEC: &'static PacketSpec = &CLIP_WINDOW_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Clip Window Height in pixels"), u(u32::from(self.height))),
            (s.field("Clip Window Width in pixels"), u(u32::from(self.width))),
            (s.field("Clip Window Bottom Pixel Coordinate"), u(u32::from(self.bottom))),
            (s.field("Clip Window Left Pixel Coordinate"), u(u32::from(self.left))),
        ]
    }
}

impl ClipWindow {
    pub fn pack(&self) -> [u8; 9] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Viewport Offset`: the viewport's translation as coarse (64-pixel
/// blocks, signed 10 bits) and fine (unsigned 14.8 fixed point, the raw
/// 22-bit value) parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportOffset {
    pub coarse_y: i16,
    pub fine_y: u32,
    pub coarse_x: i16,
    pub fine_x: u32,
}

impl Packet for ViewportOffset {
    const SPEC: &'static PacketSpec = &VIEWPORT_OFFSET_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Coarse Y"), Val::I(i64::from(self.coarse_y))),
            (s.field("Fine Y"), u(self.fine_y)),
            (s.field("Coarse X"), Val::I(i64::from(self.coarse_x))),
            (s.field("Fine X"), u(self.fine_x)),
        ]
    }
}

impl ViewportOffset {
    pub fn pack(&self) -> [u8; 9] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Clipper Z min/max clipping planes`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipperZMinMax {
    pub maximum_zw: f32,
    pub minimum_zw: f32,
}

impl Packet for ClipperZMinMax {
    const SPEC: &'static PacketSpec = &CLIPPER_Z_MIN_MAX_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![(s.field("Maximum Zw"), Val::F(self.maximum_zw)), (s.field("Minimum Zw"), Val::F(self.minimum_zw))]
    }
}

impl ClipperZMinMax {
    pub fn pack(&self) -> [u8; 9] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Clipper XY Scaling` at 7.1: the half sizes in 1/64 pixel (a negative
/// half height is the flip of a window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipperXy {
    pub half_height_1_64: f32,
    pub half_width_1_64: f32,
}

impl Packet for ClipperXy {
    const SPEC: &'static PacketSpec = &CLIPPER_XY_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Viewport Half-Height in 1/64th of pixel"), Val::F(self.half_height_1_64)),
            (s.field("Viewport Half-Width in 1/64th of pixel"), Val::F(self.half_width_1_64)),
        ]
    }
}

impl ClipperXy {
    pub fn pack(&self) -> [u8; 9] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Clipper Z Scale and Offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipperZ {
    pub z_offset: f32,
    pub z_scale: f32,
}

impl Packet for ClipperZ {
    const SPEC: &'static PacketSpec = &CLIPPER_Z_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Viewport Z Offset (Zc to Zs)"), Val::F(self.z_offset)),
            (s.field("Viewport Z Scale (Zc to Zs)"), Val::F(self.z_scale)),
        ]
    }
}

impl ClipperZ {
    pub fn pack(&self) -> [u8; 9] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// `Depth Offset`: factor and units are `f187` (the top 16 bits of an f32:
/// [`f187`]), the limit a float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOffset {
    pub limit: f32,
    pub units_f187: u16,
    pub factor_f187: u16,
}

impl Packet for DepthOffset {
    const SPEC: &'static PacketSpec = &DEPTH_OFFSET_SPEC;
    fn pairs(&self) -> Vec<(&'static Field, Val)> {
        let s = Self::SPEC;
        vec![
            (s.field("Limit"), Val::F(self.limit)),
            (s.field("Depth Offset Units"), u(u32::from(self.units_f187))),
            (s.field("Depth Offset Factor"), u(u32::from(self.factor_f187))),
        ]
    }
}

impl DepthOffset {
    pub fn pack(&self) -> [u8; 9] {
        pack_pairs(Self::SPEC, &self.pairs())
    }
}

/// An f32 as an `f187` field holds it: its top 16 bits (truncated, as Mesa's
/// packer does).
pub fn f187(v: f32) -> u16 {
    (v.to_bits() >> 16) as u16
}

// ---- the description of a draw --------------------------------------------

pub const MAX_TARGETS: usize = 8;
pub const MAX_SIZE: u32 = 7680;

/// One face's stencil state (GL: glStencilFunc, glStencilOp, glStencilMask).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StencilFace {
    pub func: Compare,
    pub sfail: StencilOp,
    pub zfail: StencilOp,
    pub zpass: StencilOp,
    pub reference: u32,
    pub value_mask: u32,
    pub write_mask: u32,
}

impl StencilFace {
    /// GL's initial state: ALWAYS, KEEP everywhere, ref 0, masks all ones.
    pub const GL_DEFAULT: StencilFace = StencilFace {
        func: Compare::Always,
        sfail: StencilOp::Keep,
        zfail: StencilOp::Keep,
        zpass: StencilOp::Keep,
        reference: 0,
        value_mask: 0xffff_ffff,
        write_mask: 0xffff_ffff,
    };
}

/// One blend equation (glBlendFuncSeparate, glBlendEquationSeparate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlendEq {
    pub rgb_mode: BlendMode,
    pub rgb_src: BlendFactor,
    pub rgb_dst: BlendFactor,
    pub alpha_mode: BlendMode,
    pub alpha_src: BlendFactor,
    pub alpha_dst: BlendFactor,
}

impl BlendEq {
    /// GL's initial state: FUNC_ADD, ONE, ZERO.
    pub const GL_DEFAULT: BlendEq = BlendEq {
        rgb_mode: BlendMode::Add,
        rgb_src: BlendFactor::One,
        rgb_dst: BlendFactor::Zero,
        alpha_mode: BlendMode::Add,
        alpha_src: BlendFactor::One,
        alpha_dst: BlendFactor::Zero,
    };
}

/// One colour target's draw state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetState {
    pub blend_enable: bool,
    pub blend: BlendEq,
    /// glColorMask: bit 0 R, 1 G, 2 B, 3 A; a set bit writes the channel
    pub color_mask: u8,
    /// whether the target's format has an alpha channel
    pub has_alpha: bool,
}

impl TargetState {
    pub const GL_DEFAULT: TargetState =
        TargetState { blend_enable: false, blend: BlendEq::GL_DEFAULT, color_mask: 0xf, has_alpha: true };
}

/// What the compiled fragment program does to depth and to the pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FragProgram {
    /// writes gl_FragDepth itself
    pub writes_z: bool,
    /// can discard a pixel
    pub discards: bool,
}

/// The cull face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
    None,
    Front,
    Back,
    FrontAndBack,
}

/// The GL state of one draw, in plain values. The target is the job's
/// framebuffer (`flip_y`: a window whose first row in memory is the top of
/// the picture; a framebuffer object is not flipped).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawState {
    pub target_w: u32,
    pub target_h: u32,
    pub flip_y: bool,
    pub samples: u32,
    /// colour targets in the framebuffer, 0 to 8
    pub nr_targets: u32,
    /// false: `targets[0]` is every target's state (gallium without
    /// independent blend); true: each target has its own
    pub independent_blend: bool,
    pub targets: [TargetState; MAX_TARGETS],
    pub depth_test: bool,
    pub depth_func: Compare,
    pub depth_write: bool,
    pub stencil_test: bool,
    /// the back face has its own state (GL's two-sided stencil)
    pub stencil_two_sided: bool,
    pub stencil_front: StencilFace,
    pub stencil_back: StencilFace,
    pub polygon_offset: bool,
    pub offset_factor: f32,
    pub offset_units: f32,
    pub offset_clamp: f32,
    /// glFrontFace(GL_CW)
    pub front_cw: bool,
    pub cull: Cull,
    /// the draw's primitives are triangles (culling applies to polygons
    /// only)
    pub polygon: bool,
    /// glScissor x, y, w, h when the scissor test is on
    pub scissor: Option<[i32; 4]>,
    /// glViewport x, y, w, h
    pub viewport: [f32; 4],
    /// glDepthRangef near, far
    pub depth_range: [f32; 2],
    pub frag: FragProgram,
}

impl DrawState {
    /// GL's initial state on a `w` x `h` target of `nr_targets` colour
    /// buffers (depth func LESS, as GL starts; the depth test is off).
    pub fn gl_default(w: u32, h: u32, flip_y: bool, nr_targets: u32) -> DrawState {
        DrawState {
            target_w: w,
            target_h: h,
            flip_y,
            samples: 1,
            nr_targets,
            independent_blend: false,
            targets: [TargetState::GL_DEFAULT; MAX_TARGETS],
            depth_test: false,
            depth_func: Compare::Less,
            depth_write: true,
            stencil_test: false,
            stencil_two_sided: false,
            stencil_front: StencilFace::GL_DEFAULT,
            stencil_back: StencilFace::GL_DEFAULT,
            polygon_offset: false,
            offset_factor: 0.0,
            offset_units: 0.0,
            offset_clamp: 0.0,
            front_cw: false,
            cull: Cull::None,
            polygon: true,
            scissor: None,
            viewport: [0.0, 0.0, w as f32, h as f32],
            depth_range: [0.0, 1.0],
            frag: FragProgram::default(),
        }
    }

    /// The state of the quad a clear is drawn as when the tile buffer cannot
    /// do it (a scissor box, a partial colour mask, a stencil write mask):
    /// the whole target as viewport (a clear is not bounded by glViewport),
    /// no scissor (the quad's vertices are the box), no depth test, no
    /// blending, the colour mask and, when `stencil` is set, a stencil test
    /// that always passes and writes `value` through `write_mask` (the
    /// blitter's stencil clear).
    pub fn blitter_clear(
        w: u32,
        h: u32,
        flip_y: bool,
        nr_targets: u32,
        color_mask: u8,
        stencil: Option<(u32, u32)>,
    ) -> DrawState {
        let mut d = DrawState::gl_default(w, h, flip_y, nr_targets);
        for t in &mut d.targets {
            t.color_mask = color_mask;
        }
        if let Some((value, write_mask)) = stencil {
            d.stencil_test = true;
            d.stencil_front = StencilFace {
                func: Compare::Always,
                sfail: StencilOp::Replace,
                zfail: StencilOp::Replace,
                zpass: StencilOp::Replace,
                reference: value,
                value_mask: 0xff,
                write_mask,
            };
        }
        d
    }
}

/// The early-Z rules of docs/notes/v3d71-record-u11.md section 4. 7.1's
/// `Cfg Bits` has no early-Z field, so nothing about early-Z is in a state
/// packet; what a table must still do is here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EarlyZ71 {
    /// always false: `Cfg Bits` has no `Early Z enable`/`Early Z updates
    /// enable` at 7.1 (the 4.2 bits 16, 17)
    pub cfg_bits_has_early_z_fields: bool,
    /// the render list's tile mode keeps `Early-Z disable` set, as the 4.2
    /// builder does (early-Z off: legal, only slower)
    pub tile_mode_early_z_disable: bool,
    /// the fragment program writes its own Z or discards: the pixel's Z is
    /// not known before the program runs (the record's `Turn off early-z
    /// test` and `Fragment shader does Z writes` carry it, record71)
    pub program_needs_late_z: bool,
    /// Mesa's `v3d_update_job_ez`: a program writing its own Z disables
    /// early-Z for the whole job. Moot while `tile_mode_early_z_disable`;
    /// a table that enables early-Z must read it.
    pub program_disables_job_early_z: bool,
}

/// The early-Z answers for a fragment program.
pub fn early_z(frag: FragProgram) -> EarlyZ71 {
    EarlyZ71 {
        cfg_bits_has_early_z_fields: false,
        tile_mode_early_z_disable: true,
        program_needs_late_z: frag.writes_z || frag.discards,
        program_disables_job_early_z: frag.writes_z,
    }
}

/// Every state packet of a draw. `blend_cfg[..blend_cfg_count]` and
/// `stencil[..stencil_count]` are the packets emitted; `depth_offset` is
/// `None` without polygon offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatePackets71 {
    pub clip: ClipWindow,
    pub cfg: CfgBits,
    pub depth_offset: Option<DepthOffset>,
    pub clipper_xy: ClipperXy,
    pub clipper_z: ClipperZ,
    pub clipper_z_min_max: ClipperZMinMax,
    pub viewport_offset: ViewportOffset,
    pub blend_enables: BlendEnables,
    pub blend_cfg: [Option<BlendCfg>; MAX_TARGETS],
    pub blend_cfg_count: usize,
    pub color_write_masks: ColorWriteMasks,
    pub stencil: [Option<StencilCfg>; 2],
    pub stencil_count: usize,
    pub early_z: EarlyZ71,
}

fn finite(v: f32) -> bool {
    v.is_finite()
}

/// The viewport transform of `glViewport`/`glDepthRangef`
/// (`_mesa_get_viewport_xform`, GL_LOWER_LEFT, GL_NEGATIVE_ONE_TO_ONE) with
/// the window's flip (`st_update_viewport`): scale and translate for X, Y,
/// Z. The same arithmetic as `viewport()` in src/v3d/state.rs and
/// `v3d_viewport_gl()` in driver/src/v3d/v3d_frame.c.
pub fn viewport_xform(d: &DrawState) -> ([f32; 3], [f32; 3]) {
    let [x, y, w, h] = d.viewport;
    let n = f64::from(d.depth_range[0].clamp(0.0, 1.0));
    let f = f64::from(d.depth_range[1].clamp(0.0, 1.0));
    let (half_w, half_h) = (0.5 * w, 0.5 * h);
    let mut scale = [half_w, half_h, (0.5 * (f - n)) as f32];
    let mut translate = [half_w + x, half_h + y, (0.5 * (n + f)) as f32];
    if d.flip_y {
        scale[1] = -scale[1];
        translate[1] = d.target_h as f32 - translate[1];
    }
    (scale, translate)
}

/// `st_update_scissor`: the box clipped to the framebuffer (an empty box is
/// all zero) and flipped for a window; `[minx, miny, maxx, maxy]`, max
/// exclusive. `None` with the scissor test off.
pub fn scissor_box(d: &DrawState) -> Option<[u32; 4]> {
    let [x, y, w, h] = d.scissor?.map(i64::from);
    let (fw, fh) = (i64::from(d.target_w), i64::from(d.target_h));
    let (mut minx, mut miny, mut maxx, mut maxy) = (0i64, 0i64, fw, fh);
    let xmax = (x + w).max(0);
    let ymax = (y + h).max(0);
    if x > minx {
        minx = x;
    }
    if y > miny {
        miny = y;
    }
    if xmax < maxx {
        maxx = xmax;
    }
    if ymax < maxy {
        maxy = ymax;
    }
    if minx >= maxx || miny >= maxy {
        (minx, miny, maxx, maxy) = (0, 0, 0, 0);
    }
    if d.flip_y {
        (miny, maxy) = (fh - maxy, fh - miny);
    }
    Some([minx as u32, miny as u32, maxx as u32, maxy as u32])
}

fn to_pixel(v: f32) -> Result<u32, Refusal71> {
    if (0.0..65536.0).contains(&v) {
        Ok(v as u32)
    } else {
        Err(Refusal71::ClipWindowRange)
    }
}

/// A float as an unsigned 14.8 fixed-point field value (round to nearest).
fn to_u14_8(v: f32) -> Result<u32, Refusal71> {
    if v.is_nan() || v < 0.0 {
        return Err(Refusal71::ViewportOffsetRange);
    }
    let x = f64::from(v) * 256.0 + 0.5;
    if x >= f64::from(1u32 << 22) {
        return Err(Refusal71::ViewportOffsetRange);
    }
    Ok(x as u32)
}

/// The Viewport Offset parts of one translation: fine in 0..64 pixels once
/// a negative value is moved into the coarse (64-pixel) block count.
fn offset_parts(t: f32) -> Result<(i16, u32), Refusal71> {
    let (mut fine, mut coarse) = (t, 0i32);
    if fine < 0.0 {
        let blocks = ((fine.abs() + 63.0) / 64.0) as i32;
        fine += 64.0 * blocks as f32;
        coarse -= blocks;
    }
    if !(-512..=511).contains(&coarse) {
        return Err(Refusal71::ViewportOffsetRange);
    }
    Ok((coarse as i16, to_u14_8(fine)?))
}

fn face_cfg(f: &StencilFace, front: bool, back: bool) -> StencilCfg {
    StencilCfg {
        stencil_write_mask: (f.write_mask & 0xff) as u8,
        back_config: back,
        front_config: front,
        stencil_pass_op: f.zpass,
        depth_test_fail_op: f.zfail,
        stencil_test_fail_op: f.sfail,
        stencil_test_function: f.func,
        stencil_test_mask: (f.value_mask & 0xff) as u8,
        /* glStencilFunc clamps the reference to the buffer's range */
        stencil_ref_value: f.reference.min(0xff) as u8,
    }
}

fn reads_dst_alpha(f: BlendFactor) -> bool {
    matches!(f, BlendFactor::DstAlpha | BlendFactor::InvDstAlpha | BlendFactor::SrcAlphaSaturate)
}

/// The packets of one draw, or the refusal naming what the 7.1 table does
/// not draw.
pub fn translate(d: &DrawState) -> Result<StatePackets71, Refusal71> {
    if !(1..=MAX_SIZE).contains(&d.target_w) || !(1..=MAX_SIZE).contains(&d.target_h) {
        return Err(Refusal71::TargetSize);
    }
    if d.nr_targets as usize > MAX_TARGETS {
        return Err(Refusal71::TooManyTargets);
    }
    if d.samples != 1 {
        return Err(Refusal71::Multisample);
    }
    if !(d.viewport.iter().all(|v| finite(*v)) && d.viewport[2] >= 0.0 && d.viewport[3] >= 0.0)
        || !d.depth_range.iter().all(|v| finite(*v))
    {
        return Err(Refusal71::ViewportNotFinite);
    }
    if d.polygon_offset && !(finite(d.offset_factor) && finite(d.offset_units) && finite(d.offset_clamp)) {
        return Err(Refusal71::OffsetNotFinite);
    }

    /* the clip window: the viewport's box, and the scissor or the target */
    let (scale, translate) = viewport_xform(d);
    let (vp_minx, vp_maxx) = (-scale[0].abs() + translate[0], scale[0].abs() + translate[0]);
    let (vp_miny, vp_maxy) = (-scale[1].abs() + translate[1], scale[1].abs() + translate[1]);
    let [lo_x, lo_y, hi_x, hi_y] = match scissor_box(d) {
        Some(s) => s.map(|v| v as f32),
        None => [0.0, 0.0, d.target_w as f32, d.target_h as f32],
    };
    let minx = to_pixel(vp_minx.max(lo_x))?;
    let miny = to_pixel(vp_miny.max(lo_y))?;
    let maxx = to_pixel(vp_maxx.min(hi_x))?;
    let maxy = to_pixel(vp_maxy.min(hi_y))?;
    let (cw, ch) = if maxx > minx && maxy > miny { (maxx - minx, maxy - miny) } else { (0, 0) };
    let clip = ClipWindow { left: minx as u16, bottom: miny as u16, width: cw as u16, height: ch as u16 };

    /* the depth and the facing: no early-Z bits at 7.1, Z clipping -1..1 */
    let (depth_func, z_updates) = if d.depth_test { (d.depth_func, d.depth_write) } else { (Compare::Always, false) };
    let cull_front = matches!(d.cull, Cull::Front | Cull::FrontAndBack);
    let cull_back = matches!(d.cull, Cull::Back | Cull::FrontAndBack);
    /* gallium's front_ccw: glFrontFace == GL_CCW, inverted for a framebuffer
     * object (st_atom_rasterizer.c:77-92); the packet's bit "seems
     * backwards" and takes it as it is (Mesa's comment) */
    let front_ccw = d.front_cw ^ d.flip_y;
    let blend0 = d.targets[0].blend_enable;
    let cfg = CfgBits {
        z_clipping_mode: ZClipMode::MinOneToOne,
        direct3d_provoking_vertex: false,
        direct3d_point_fill_mode: false,
        blend_enable: blend0,
        stencil_enable: d.stencil_test,
        z_updates_enable: z_updates,
        depth_test_function: depth_func,
        direct3d_wireframe_triangles_mode: false,
        z_clamp_mode: false,
        rasterizer_oversample_mode: 0,
        depth_bounds_test_enable: false,
        line_rasterization: 0,
        enable_depth_offset: d.polygon_offset,
        clockwise_primitives: front_ccw,
        /* culling is for polygons: points and lines are always drawn */
        enable_reverse_facing_primitive: !(d.polygon && cull_back),
        enable_forward_facing_primitive: !(d.polygon && cull_front),
    };
    let cfg = if d.independent_blend {
        let any = (0..d.nr_targets as usize).any(|i| d.targets[i].blend_enable);
        CfgBits { blend_enable: any, ..cfg }
    } else {
        cfg
    };

    let depth_offset = d.polygon_offset.then(|| DepthOffset {
        limit: d.offset_clamp,
        units_f187: f187(d.offset_units),
        factor_f187: f187(d.offset_factor),
    });

    /* the viewport: XY in 1/64 pixel at 7.1; a |Z scale| under 0.01 is the
     * no-guardband packet, refused */
    if scale[2].is_nan() || scale[2].abs() < 0.01 {
        return Err(Refusal71::DepthScaleTooSmall);
    }
    let clipper_xy = ClipperXy { half_height_1_64: scale[1] * 64.0, half_width_1_64: scale[0] * 64.0 };
    let clipper_z = ClipperZ { z_offset: translate[2], z_scale: scale[2] };
    let (za, zb) = (translate[2] - scale[2], translate[2] + scale[2]);
    let clipper_z_min_max = ClipperZMinMax { maximum_zw: za.max(zb), minimum_zw: za.min(zb) };
    let (coarse_x, fine_x) = offset_parts(translate[0])?;
    let (coarse_y, fine_y) = offset_parts(translate[1])?;
    let viewport_offset = ViewportOffset { coarse_y, fine_y, coarse_x, fine_x };

    /* blend: the targets' equations (target 0's for all 8 without
     * independent blend, as one packet for all of them) */
    let eff = |i: usize| if d.independent_blend { d.targets[i] } else { d.targets[0] };
    for i in 0..d.nr_targets as usize {
        let t = eff(i);
        if t.blend_enable {
            let e = &t.blend;
            let reads = [e.rgb_src, e.rgb_dst, e.alpha_src, e.alpha_dst].into_iter().any(reads_dst_alpha);
            if reads && !d.targets[i].has_alpha {
                return Err(Refusal71::BlendReadsMissingAlpha);
            }
        }
    }
    let to_cfg = |mask: u8, e: &BlendEq| {
        /* GL ignores the factors of MIN and MAX: gallium gives ONE, ONE */
        let (rs, rd) = if matches!(e.rgb_mode, BlendMode::Min | BlendMode::Max) {
            (BlendFactor::One, BlendFactor::One)
        } else {
            (e.rgb_src, e.rgb_dst)
        };
        let (as_, ad) = if matches!(e.alpha_mode, BlendMode::Min | BlendMode::Max) {
            (BlendFactor::One, BlendFactor::One)
        } else {
            (e.alpha_src, e.alpha_dst)
        };
        BlendCfg {
            render_target_mask: mask,
            color_blend_dst_factor: rd,
            color_blend_src_factor: rs,
            color_blend_mode: e.rgb_mode,
            alpha_blend_dst_factor: ad,
            alpha_blend_src_factor: as_,
            alpha_blend_mode: e.alpha_mode,
        }
    };
    let mut blend_cfg = [None; MAX_TARGETS];
    let mut blend_cfg_count = 0;
    let mut enables = 0u8;
    if d.independent_blend {
        for i in 0..d.nr_targets as usize {
            if d.targets[i].blend_enable {
                enables |= 1 << i;
                blend_cfg[blend_cfg_count] = Some(to_cfg(1 << i, &d.targets[i].blend));
                blend_cfg_count += 1;
            }
        }
    } else if d.targets[0].blend_enable {
        enables = 0xff;
        blend_cfg[0] = Some(to_cfg(0xff, &d.targets[0].blend));
        blend_cfg_count = 1;
    }
    let mut wm = 0u32;
    for i in 0..MAX_TARGETS {
        wm |= u32::from(!eff(i).color_mask & 0xf) << (4 * i);
    }

    /* stencil: the front face configures the back too unless the back has
     * its own */
    let mut stencil = [None; 2];
    let mut stencil_count = 0;
    if d.stencil_test {
        stencil[0] = Some(face_cfg(&d.stencil_front, true, !d.stencil_two_sided));
        stencil_count = 1;
        if d.stencil_two_sided {
            stencil[1] = Some(face_cfg(&d.stencil_back, false, true));
            stencil_count = 2;
        }
    }

    Ok(StatePackets71 {
        clip,
        cfg,
        depth_offset,
        clipper_xy,
        clipper_z,
        clipper_z_min_max,
        viewport_offset,
        blend_enables: BlendEnables { mask: enables },
        blend_cfg,
        blend_cfg_count,
        color_write_masks: ColorWriteMasks { mask: wm },
        stencil,
        stencil_count,
        early_z: early_z(d.frag),
    })
}
