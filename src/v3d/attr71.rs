// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 vertex attribute records (roadmap P9; the Pi 5's V3D 7.1.10).
//!
//! Pure functions over plain integers and bools; no `crate::` import, so a
//! test includes this file with `#[path]` (tests/v3d71_attr.rs). Nothing
//! hooks it into `src/v3d/mod.rs` yet: the 7.1 job builder does that.
//!
//! Each draw's `GL_SHADER_STATE_RECORD_DRAW_INDEX` (record71) is followed by
//! one `GL_SHADER_STATE_ATTRIBUTE_RECORD` per attribute array: 16 bytes
//! (v3d_packet.xml "GL Shader State Attribute Record", one layout for 4.2
//! and 7.1), [`FIELDS`]. [`plan`] turns a plain description of each GL
//! attribute plus the programs' reads into those records; [`pack`] writes a
//! record's bytes.
//!
//! # A disabled attribute the program reads (no default-attribute BO)
//!
//! 4.2 points the shader record at a job-wide BO of 16 `(0,0,0,1)` slots and
//! gives an unserved attribute a record whose address is in that BO. 7.1 has
//! no such field and Mesa never reserves the BO
//! (`v3d_device_needs_default_attribute_values`: `ver <= 42`;
//! docs/notes/v3d71-gl-table.md section 2 item 3; the 7.1 builders
//! driver/src/v3d/v3d_job71.c and v3d_frame71_tri.c:290-293 reserve none).
//! The vendored tree has no `v3dx_state.c`, but its design is known from the
//! gallium interface: a gallium driver is never told "attribute disabled";
//! the state tracker binds the generic value as a vertex buffer of stride 0
//! holding the 16 bytes, and the driver emits an ordinary record over it.
//! The choice here follows that: a disabled attribute that a program reads
//! gets an ordinary record, **stride 0, vec size 4, the address of a 16-byte
//! slot of the draw's own** ([`Attr::constant_addr`], which the caller
//! allocates in the draw's data and fills with [`constant_bytes`]: the
//! attribute's generic value, `(0,0,0,1)` until the application sets one).
//! There is no job-wide BO and no fixed slot per index. Every vertex then
//! reads the same four words, which is what a constant attribute is. A
//! caller with no slot to give (`constant_addr` None) gets
//! [`Refusal71::ConstantAttribute`], never an address of zero.
//!
//! A disabled attribute the programs do not read still gets its record (the
//! record index is the attribute index) with both read counts 0 and the
//! caller's `dummy_addr`, "a valid address of data whose value will be
//! unused" (GFXH-930), as v3d_job71.c points its dummy record.
//!
//! The GFXH-930 / GFXH-1602 rules for what the coordinate shader must load
//! and the dummy record of a program that reads no attribute are v3d_job71.c's
//! (lines 690-730), reproduced in [`plan`].
//!
//! # What 7.1 cannot take (named refusals, [`Refusal71`])
//!
//! A GL type with no 7.1 attribute type (`GL_DOUBLE`, unknown enums), a
//! packed 2_10_10_10 type with other than 4 components, an integer attribute
//! of a float type, more attributes than [`MAX_ATTRS`], a stride, divisor or
//! read count above its field, a buffer address plus offset past 32 bits, and
//! a vertex or coordinate shader with separate VPM segments (the shader
//! record's input sizes are then not 0; see [`vpm_input_size`]).

/// GL attribute types (GLenum values).
pub const GL_BYTE: u32 = 0x1400;
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_SHORT: u32 = 0x1402;
pub const GL_UNSIGNED_SHORT: u32 = 0x1403;
pub const GL_INT: u32 = 0x1404;
pub const GL_UNSIGNED_INT: u32 = 0x1405;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_HALF_FLOAT: u32 = 0x140B;
pub const GL_DOUBLE: u32 = 0x140A;
pub const GL_FIXED: u32 = 0x140C;
pub const GL_HALF_FLOAT_OES: u32 = 0x8D61;
pub const GL_UNSIGNED_INT_2_10_10_10_REV: u32 = 0x8368;
pub const GL_INT_2_10_10_10_REV: u32 = 0x8D9F;

/// The record's `Type` field values (v3d_packet.xml).
pub const TYPE_HALF_FLOAT: u8 = 1;
pub const TYPE_FLOAT: u8 = 2;
pub const TYPE_FIXED: u8 = 3;
pub const TYPE_BYTE: u8 = 4;
pub const TYPE_SHORT: u8 = 5;
pub const TYPE_INT: u8 = 6;
pub const TYPE_INT2_10_10_10: u8 = 7;

/// One attribute record is 16 bytes.
pub const RECORD_BYTES: usize = 16;
/// Attribute arrays per draw: GL_MAX_VERTEX_ATTRIBS of the table and the C
/// builder's V3D_MAX_ATTRS. (The packet's "number of attribute arrays" is 5
/// bits, 31; the program's per-record read table has 16 entries.)
pub const MAX_ATTRS: usize = 16;
/// Largest `Instance Divisor` (16 bits; v3d_limits.h
/// V3D_MAX_VERTEX_ATTRIB_DIVISOR).
pub const MAX_DIVISOR: u32 = 0xffff;
/// `Maximum Index` Mesa writes for every record.
pub const MAX_INDEX: u32 = 0x00ff_ffff;

/// Record fields as (name, start bit, size in bits), v3d_packet.xml order
/// (`start="6b"` is bit 48).
pub const FIELDS: [(&str, u32, u32); 11] = [
    ("Address", 0, 32),
    ("Vec size", 32, 2),
    ("Type", 34, 3),
    ("Signed int type", 37, 1),
    ("Normalized int type", 38, 1),
    ("Read as int/uint", 39, 1),
    ("Number of values read by Coordinate shader", 40, 4),
    ("Number of values read by Vertex shader", 44, 4),
    ("Instance Divisor", 48, 16),
    ("Stride", 64, 32),
    ("Maximum Index", 96, 32),
];

/// Why 7.1 does not take an input. `message` is the named refusal text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal71 {
    /// a program reads a disabled attribute and no constant slot was given
    ConstantAttribute,
    /// a GL type with no 7.1 attribute type
    Type(u32),
    /// component count not 1..=4
    Components(u32),
    /// 2_10_10_10 with other than 4 components
    PackedComponents,
    /// glVertexAttribIPointer with a float, half-float or fixed type, or
    /// integer together with normalized
    IntegerType,
    /// more attributes than [`MAX_ATTRS`]
    TooManyAttributes(usize),
    /// stride above the 32-bit field
    Stride(u64),
    /// instance divisor above the 16-bit field
    Divisor(u32),
    /// read count above the 4-bit field
    Reads(u8),
    /// a program reads an attribute index with no record
    ReadWithoutRecord(usize),
    /// buffer address plus offset past 32 bits
    Address,
    /// separate VPM segments (non-zero input size) are not built
    SeparateSegments,
}

impl Refusal71 {
    pub fn message(&self) -> &'static str {
        match self {
            Refusal71::ConstantAttribute => {
                "V3D 7.1: a constant vertex attribute the program reads has no 16-byte slot to read from (no default-attribute BO)"
            }
            Refusal71::Type(_) => "V3D 7.1: this vertex attribute type has no 7.1 attribute type",
            Refusal71::Components(_) => "V3D 7.1: a vertex attribute has 1 to 4 components",
            Refusal71::PackedComponents => {
                "V3D 7.1: a 2_10_10_10 vertex attribute has 4 components"
            }
            Refusal71::IntegerType => {
                "V3D 7.1: an integer vertex attribute needs an integer type and is not normalized"
            }
            Refusal71::TooManyAttributes(_) => "V3D 7.1: more than 16 vertex attribute arrays",
            Refusal71::Stride(_) => "V3D 7.1: a vertex attribute stride above 32 bits",
            Refusal71::Divisor(_) => "V3D 7.1: a vertex attribute divisor above 65535",
            Refusal71::Reads(_) => "V3D 7.1: a program reads more than 15 values of an attribute",
            Refusal71::ReadWithoutRecord(_) => {
                "V3D 7.1: a program reads an attribute that has no record"
            }
            Refusal71::Address => "V3D 7.1: a vertex buffer address plus offset past 32 bits",
            Refusal71::SeparateSegments => {
                "V3D 7.1: a shader with separate input and output VPM segments is not built"
            }
        }
    }
}

/// A plain description of one GL vertex attribute (index = array index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub enabled: bool,
    /// GL size, 1..=4
    pub components: u32,
    /// GL_FLOAT etc.
    pub gl_type: u32,
    pub normalized: bool,
    /// glVertexAttribIPointer
    pub integer: bool,
    /// bytes between vertices, already resolved (GL stride 0 = tight is the
    /// caller's computation); 0 reads one value for every vertex
    pub stride: u64,
    pub offset: u64,
    /// GPU address of the buffer
    pub buffer_addr: u32,
    pub divisor: u32,
    /// the generic value's four words as bits, read when disabled
    pub generic: [u32; 4],
    /// where the draw keeps the 16 bytes of [`constant_bytes`], when the
    /// attribute is disabled and read; 16-byte aligned
    pub constant_addr: Option<u32>,
}

impl Attr {
    /// A disabled attribute with GL's initial generic value `(0,0,0,1)`.
    pub const fn disabled() -> Attr {
        Attr {
            enabled: false,
            components: 4,
            gl_type: GL_FLOAT,
            normalized: false,
            integer: false,
            stride: 0,
            offset: 0,
            buffer_addr: 0,
            divisor: 0,
            generic: [0, 0, 0, 0x3f80_0000],
            constant_addr: None,
        }
    }
}

/// One record's field values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record71 {
    pub address: u32,
    /// the 2-bit field: components & 3 (0 means 4)
    pub vec_size: u8,
    pub ty: u8,
    pub signed: bool,
    pub normalized: bool,
    pub read_as_int: bool,
    pub cs_values: u8,
    pub vs_values: u8,
    pub divisor: u16,
    pub stride: u32,
    pub max_index: u32,
}

/// What a draw needs: the records, in attribute order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan71 {
    pub records: [Record71; MAX_ATTRS],
    /// records in use = the GL_SHADER packet's "number of attribute arrays"
    pub count: usize,
}

/// The 16 bytes a constant attribute's slot holds (little endian words).
pub fn constant_bytes(a: &Attr) -> [u8; 16] {
    let mut b = [0u8; 16];
    for (i, w) in a.generic.iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    b
}

/// (type, signed) of a GL type, with the packed flag.
fn gl_type_row(t: u32) -> Result<(u8, bool, bool), Refusal71> {
    Ok(match t {
        GL_BYTE => (TYPE_BYTE, true, false),
        GL_UNSIGNED_BYTE => (TYPE_BYTE, false, false),
        GL_SHORT => (TYPE_SHORT, true, false),
        GL_UNSIGNED_SHORT => (TYPE_SHORT, false, false),
        GL_INT => (TYPE_INT, true, false),
        GL_UNSIGNED_INT => (TYPE_INT, false, false),
        GL_FLOAT => (TYPE_FLOAT, false, false),
        GL_HALF_FLOAT | GL_HALF_FLOAT_OES => (TYPE_HALF_FLOAT, false, false),
        GL_FIXED => (TYPE_FIXED, false, false),
        GL_INT_2_10_10_10_REV => (TYPE_INT2_10_10_10, true, true),
        GL_UNSIGNED_INT_2_10_10_10_REV => (TYPE_INT2_10_10_10, false, true),
        _ => return Err(Refusal71::Type(t)),
    })
}

/// The record of one attribute without its read counts.
fn describe(a: &Attr, read: bool, dummy_addr: u32) -> Result<Record71, Refusal71> {
    let mut r = Record71 {
        address: dummy_addr,
        vec_size: 0,
        ty: TYPE_FLOAT,
        signed: false,
        normalized: false,
        read_as_int: false,
        cs_values: 0,
        vs_values: 0,
        divisor: 0,
        stride: 0,
        max_index: MAX_INDEX,
    };
    if !a.enabled {
        if read {
            r.address = a.constant_addr.ok_or(Refusal71::ConstantAttribute)?;
            if a.integer {
                r.ty = TYPE_INT;
                r.read_as_int = true;
            }
        }
        return Ok(r);
    }
    if a.components == 0 || a.components > 4 {
        return Err(Refusal71::Components(a.components));
    }
    let (ty, signed, packed) = gl_type_row(a.gl_type)?;
    if packed && a.components != 4 {
        return Err(Refusal71::PackedComponents);
    }
    let int_type = matches!(ty, TYPE_BYTE | TYPE_SHORT | TYPE_INT) && !packed;
    if a.integer && (!int_type || a.normalized) {
        return Err(Refusal71::IntegerType);
    }
    if a.stride > u32::MAX as u64 {
        return Err(Refusal71::Stride(a.stride));
    }
    if a.divisor > MAX_DIVISOR {
        return Err(Refusal71::Divisor(a.divisor));
    }
    let addr = a.buffer_addr as u64 + a.offset;
    if addr > u32::MAX as u64 {
        return Err(Refusal71::Address);
    }
    r.address = addr as u32;
    r.vec_size = (a.components & 3) as u8;
    r.ty = ty;
    r.signed = signed;
    /* GL ignores `normalized` for float, half and fixed */
    r.normalized = a.normalized && (int_type || packed);
    r.read_as_int = a.integer;
    r.divisor = a.divisor as u16;
    r.stride = a.stride as u32;
    Ok(r)
}

/// The records of a draw. `cs_reads[i]` / `vs_reads[i]` are the programs'
/// `vattr_sizes[i]` (values of attribute i they read); `cs_builtins` is the
/// coordinate shader's use of the vertex or instance id; `dummy_addr` is a
/// valid address of unused data. A program that reads no attribute gets the
/// one GFXH-930 dummy record (vec size 1, float, reads 1 and 1, stride 0).
pub fn plan(
    attrs: &[Attr],
    cs_reads: &[u8; MAX_ATTRS],
    vs_reads: &[u8; MAX_ATTRS],
    cs_builtins: bool,
    dummy_addr: u32,
) -> Result<Plan71, Refusal71> {
    if attrs.len() > MAX_ATTRS {
        return Err(Refusal71::TooManyAttributes(attrs.len()));
    }
    for i in attrs.len()..MAX_ATTRS {
        if cs_reads[i] != 0 || vs_reads[i] != 0 {
            return Err(Refusal71::ReadWithoutRecord(i));
        }
    }
    let blank = Record71 {
        address: 0,
        vec_size: 0,
        ty: 0,
        signed: false,
        normalized: false,
        read_as_int: false,
        cs_values: 0,
        vs_values: 0,
        divisor: 0,
        stride: 0,
        max_index: 0,
    };
    let mut out = Plan71 { records: [blank; MAX_ATTRS], count: attrs.len() };
    let mut cs_loaded_any = false;
    for (i, a) in attrs.iter().enumerate() {
        for r in [cs_reads[i], vs_reads[i]] {
            if r > 15 {
                return Err(Refusal71::Reads(r));
            }
        }
        let read = cs_reads[i] != 0 || vs_reads[i] != 0;
        let mut r = describe(a, read, dummy_addr)?;
        r.cs_values = cs_reads[i];
        r.vs_values = vs_reads[i];
        if r.cs_values != 0 {
            cs_loaded_any = true;
        }
        /* GFXH-930 / GFXH-1602: the coordinate shader loads something */
        if ((i == 0 && cs_builtins) || i == attrs.len() - 1) && !cs_loaded_any {
            r.cs_values = 1;
            cs_loaded_any = true;
        }
        out.records[i] = r;
    }
    if attrs.is_empty() {
        out.records[0] =
            Record71 { address: dummy_addr, vec_size: 1, ty: TYPE_FLOAT, cs_values: 1, vs_values: 1, ..blank };
        out.count = 1;
    }
    Ok(out)
}

/// Pack a record into its 16 bytes (little endian; bit positions [`FIELDS`]).
pub fn pack(r: &Record71) -> [u8; RECORD_BYTES] {
    let mut b = [0u8; RECORD_BYTES];
    b[0..4].copy_from_slice(&r.address.to_le_bytes());
    b[4] = (r.vec_size & 3)
        | (r.ty & 7) << 2
        | (r.signed as u8) << 5
        | (r.normalized as u8) << 6
        | (r.read_as_int as u8) << 7;
    b[5] = (r.cs_values & 15) | (r.vs_values & 15) << 4;
    b[6..8].copy_from_slice(&r.divisor.to_le_bytes());
    b[8..12].copy_from_slice(&r.stride.to_le_bytes());
    b[12..16].copy_from_slice(&r.max_index.to_le_bytes());
    b
}

/// The shader record's "Coordinate / Vertex shader input VPM segment size":
/// 0 for the shared-segment shaders this table builds (Mesa's 7.1 records
/// show 0 for both, tools/v3d/pi5/triangle.clif). With separate segments the
/// size would count the input rows, which no program here needs: refused.
pub fn vpm_input_size(separate_segments: bool) -> Result<u32, Refusal71> {
    if separate_segments {
        Err(Refusal71::SeparateSegments)
    } else {
        Ok(0)
    }
}

/// The input rows the programs read, `sum(vattr_sizes)` plus the vertex and
/// instance id rows (the shaderc chain71 `vpm_fields` count), for sizing the
/// output segment.
pub fn input_rows(reads: &[u8; MAX_ATTRS], uses_vid: bool, uses_iid: bool) -> u32 {
    let sum: u32 = reads.iter().map(|&x| x as u32).sum();
    let driver_row = (uses_vid || uses_iid) && reads[0] == 0;
    sum + uses_vid as u32 + uses_iid as u32 + driver_row as u32
}
