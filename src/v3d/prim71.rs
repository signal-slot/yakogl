// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 draw packet values (roadmap P9; the Pi 5's V3D 7.1.10).
//!
//! Pure functions over plain integers; no `crate::` import, so a test
//! includes this file with `#[path]` (tests/v3d71_prim.rs). Nothing hooks it
//! into `src/v3d/mod.rs` yet: the 7.1 job builder does that.
//!
//! Layouts are Mesa 26.2.1's `v3d_packet.xml` (none of these packets carries
//! a `min_ver`/`max_ver`: one layout serves 4.2 and 7.1). A packet is its
//! opcode byte, then the body; a field's `start` is a bit offset into the
//! body, little endian ([`FIELDS`]):
//!
//! | packet | code | bytes |
//! |---|---|---|
//! | Indexed Prim List | 32 | 1+9 |
//! | Indexed Instanced Prim List | 34 | 1+13 |
//! | Vertex Array Prims | 36 | 1+9 |
//! | Vertex Array Instanced Prims | 38 | 1+13 |
//! | Base Vertex Base Instance | 43 | 1+8 |
//! | Index Buffer Setup | 44 | 1+8 |
//!
//! `mode` is the XML `Primitive` enum, which equals the GL mode number for
//! the seven modes the table carries (POINTS 0 ... TRIANGLE_FAN 6); the
//! `_TF` modes (16..22) are transform feedback's and refused. Index type is
//! 0 / 1 / 2 for 8 / 16 / 32 bit. Primitive restart is the packet's
//! `Enable Primitive Restarts` bit: V3D's restart value is the all-ones
//! index of the type, which is exactly ES3's fixed restart index.
//!
//! [`pack_draw`] returns the whole sequence for one GL draw: the base packet
//! (only when the base vertex or base instance is not 0), the index buffer
//! setup (indexed draws), then the prim packet. A draw of one instance uses
//! the plain (non-instanced) packet. The vendored tree has no `v3dx_draw.c`
//! (only the cle XML), so the order and the "emit the base packet when
//! non-zero" choice follow the gallium contract and the Pi 4 builder
//! (driver/src/v3d/v3d_job.c); a board run is the check.
//!
//! # What this module does not carry
//!
//! The 7.1.10 (2712 D0) draw-index variant is a field of the GL shader-state
//! record (`GL_SHADER_STATE_RECORD_DRAW_INDEX`, docs/notes/v3d71-record-u11.md),
//! not of any draw packet; record71 owns it and nothing here repeats it.
//! Indirect draws (codes 33, 37), single-instance draws (39) and transform
//! feedback modes are refused by name ([`Refusal71`]).

use std::fmt;

/// GL mode numbers.
pub const GL_POINTS: u32 = 0;
pub const GL_LINES: u32 = 1;
pub const GL_LINE_LOOP: u32 = 2;
pub const GL_LINE_STRIP: u32 = 3;
pub const GL_TRIANGLES: u32 = 4;
pub const GL_TRIANGLE_STRIP: u32 = 5;
pub const GL_TRIANGLE_FAN: u32 = 6;
/// GL index types.
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_UNSIGNED_SHORT: u32 = 0x1403;
pub const GL_UNSIGNED_INT: u32 = 0x1405;

pub const OP_INDEXED_PRIM_LIST: u8 = 32;
pub const OP_INDEXED_INSTANCED_PRIM_LIST: u8 = 34;
pub const OP_VERTEX_ARRAY_PRIMS: u8 = 36;
pub const OP_VERTEX_ARRAY_INSTANCED_PRIMS: u8 = 38;
pub const OP_BASE_VERTEX_BASE_INSTANCE: u8 = 43;
pub const OP_INDEX_BUFFER_SETUP: u8 = 44;

/// One xml field: `(packet name, field name, start bit, size bits)`.
pub type Field = (&'static str, &'static str, u32, u32);

/// Every field of the six packets, as v3d_packet.xml writes them.
pub const FIELDS: &[Field] = &[
    ("Indexed Prim List", "Index Offset", 40, 32),
    ("Indexed Prim List", "Enable Primitive Restarts", 39, 1),
    ("Indexed Prim List", "Length", 8, 31),
    ("Indexed Prim List", "Index type", 6, 2),
    ("Indexed Prim List", "mode", 0, 6),
    ("Indexed Instanced Prim List", "Index Offset", 72, 32),
    ("Indexed Instanced Prim List", "Number of Instances", 40, 32),
    ("Indexed Instanced Prim List", "Enable Primitive Restarts", 39, 1),
    ("Indexed Instanced Prim List", "Instance Length", 8, 31),
    ("Indexed Instanced Prim List", "Index type", 6, 2),
    ("Indexed Instanced Prim List", "mode", 0, 6),
    ("Vertex Array Prims", "Index of First Vertex", 40, 32),
    ("Vertex Array Prims", "Length", 8, 32),
    ("Vertex Array Prims", "mode", 0, 8),
    ("Vertex Array Instanced Prims", "Index of First Vertex", 72, 32),
    ("Vertex Array Instanced Prims", "Number of Instances", 40, 32),
    ("Vertex Array Instanced Prims", "Instance Length", 8, 32),
    ("Vertex Array Instanced Prims", "mode", 0, 8),
    ("Base Vertex Base Instance", "Base Instance", 32, 32),
    ("Base Vertex Base Instance", "Base Vertex", 0, 32),
    ("Index Buffer Setup", "Address", 0, 32),
    ("Index Buffer Setup", "Size", 32, 32),
];

/// The indexed part of a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indices {
    /// `GL_UNSIGNED_BYTE` / `SHORT` / `INT`.
    pub gl_type: u32,
    /// Bus address of the index buffer's first byte.
    pub addr: u32,
    /// Size of the index buffer in bytes (from `addr`).
    pub size: u64,
    /// Byte offset of the first index (GL's `indices` pointer).
    pub offset: u64,
    /// ES3 fixed primitive restart index enabled.
    pub restart: bool,
}

/// One GL draw call. Wide integer types so an overflow is seen, not wrapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draw {
    /// GL mode number (`GL_POINTS` ... `GL_TRIANGLE_FAN`).
    pub mode: u32,
    /// `Some` for glDrawElements.
    pub indices: Option<Indices>,
    /// glDrawArrays `first` (ignored for indexed draws).
    pub first: u64,
    /// Vertex or index count.
    pub count: u64,
    /// Instance count (1 for a plain draw).
    pub instances: u64,
    /// glDrawElementsBaseVertex's base vertex (signed).
    pub base_vertex: i64,
    pub base_instance: u64,
}

impl Draw {
    /// glDrawArrays(mode, first, count).
    pub fn arrays(mode: u32, first: u64, count: u64) -> Draw {
        Draw { mode, indices: None, first, count, instances: 1, base_vertex: 0, base_instance: 0 }
    }
}

/// Why the 7.1 table will not carry a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal71 {
    /// A mode outside POINTS..TRIANGLE_FAN (patches, adjacency, quads, ...).
    Mode(u32),
    /// An index type outside UNSIGNED_BYTE/SHORT/INT.
    IndexType(u32),
    /// Count 0: GL draws nothing, the caller skips the draw.
    ZeroCount,
    /// Instance count 0: likewise.
    ZeroInstances,
    /// A named field does not hold the value (the field's bits are too few).
    Overflow { field: &'static str, value: u64, bits: u32 },
    /// A base vertex outside the signed 32-bit range.
    BaseVertex(i64),
    /// The index range `offset + count * size` runs past the buffer.
    IndexRange { end: u64, size: u64 },
    /// An index offset not a multiple of the index size (GL forbids it).
    IndexAlign { offset: u64, index_bytes: u64 },
}

impl fmt::Display for Refusal71 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Refusal71::Mode(m) => {
                write!(f, "7.1 draw: primitive mode 0x{m:x} is not one of the seven the table carries")
            }
            Refusal71::IndexType(t) => {
                write!(f, "7.1 draw: index type 0x{t:x} is not UNSIGNED_BYTE, SHORT or INT")
            }
            Refusal71::ZeroCount => write!(f, "7.1 draw: count 0 draws nothing (skip it)"),
            Refusal71::ZeroInstances => write!(f, "7.1 draw: instance count 0 draws nothing (skip it)"),
            Refusal71::Overflow { field, value, bits } => {
                write!(f, "7.1 draw: {field} {value} does not fit its {bits}-bit field")
            }
            Refusal71::BaseVertex(v) => {
                write!(f, "7.1 draw: base vertex {v} is outside the signed 32-bit range")
            }
            Refusal71::IndexRange { end, size } => {
                write!(f, "7.1 draw: indices end at byte {end}, past the {size}-byte index buffer")
            }
            Refusal71::IndexAlign { offset, index_bytes } => {
                write!(f, "7.1 draw: index offset {offset} is not a multiple of the {index_bytes}-byte index")
            }
        }
    }
}

impl std::error::Error for Refusal71 {}

/// The `Primitive` enum value of a GL mode.
pub fn prim_mode(gl_mode: u32) -> Result<u8, Refusal71> {
    match gl_mode {
        GL_POINTS..=GL_TRIANGLE_FAN => Ok(gl_mode as u8),
        m => Err(Refusal71::Mode(m)),
    }
}

/// The packet's `Index type` value and the index size in bytes.
pub fn index_type(gl_type: u32) -> Result<(u8, u64), Refusal71> {
    match gl_type {
        GL_UNSIGNED_BYTE => Ok((0, 1)),
        GL_UNSIGNED_SHORT => Ok((1, 2)),
        GL_UNSIGNED_INT => Ok((2, 4)),
        t => Err(Refusal71::IndexType(t)),
    }
}

/// Set `size` bits of `v` at bit `start` of `buf` (little endian).
fn put(buf: &mut [u8], start: u32, size: u32, v: u64) {
    for i in 0..size {
        if (v >> i) & 1 != 0 {
            let b = start + i;
            buf[(b / 8) as usize] |= 1 << (b % 8);
        }
    }
}

fn fit(field: &'static str, value: u64, bits: u32) -> Result<u64, Refusal71> {
    if value >> bits != 0 {
        Err(Refusal71::Overflow { field, value, bits })
    } else {
        Ok(value)
    }
}

/// Vertex Array Prims (36), or Vertex Array Instanced Prims (38) when
/// `instances` is not 1.
pub fn pack_array(mode: u32, first: u64, count: u64, instances: u64) -> Result<Vec<u8>, Refusal71> {
    let mode = prim_mode(mode)?;
    if count == 0 {
        return Err(Refusal71::ZeroCount);
    }
    if instances == 0 {
        return Err(Refusal71::ZeroInstances);
    }
    let first = fit("Index of First Vertex", first, 32)?;
    let count = fit("Length", count, 32)?;
    if instances == 1 {
        let mut p = vec![0u8; 10];
        p[0] = OP_VERTEX_ARRAY_PRIMS;
        put(&mut p[1..], 0, 8, mode as u64);
        put(&mut p[1..], 8, 32, count);
        put(&mut p[1..], 40, 32, first);
        Ok(p)
    } else {
        let n = fit("Number of Instances", instances, 32)?;
        let mut p = vec![0u8; 14];
        p[0] = OP_VERTEX_ARRAY_INSTANCED_PRIMS;
        put(&mut p[1..], 0, 8, mode as u64);
        put(&mut p[1..], 8, 32, count);
        put(&mut p[1..], 40, 32, n);
        put(&mut p[1..], 72, 32, first);
        Ok(p)
    }
}

/// Indexed Prim List (32), or Indexed Instanced Prim List (34) when
/// `instances` is not 1. `offset` is the byte offset of the first index.
pub fn pack_indexed(
    mode: u32,
    gl_type: u32,
    offset: u64,
    count: u64,
    instances: u64,
    restart: bool,
) -> Result<Vec<u8>, Refusal71> {
    let mode = prim_mode(mode)?;
    let (ty, _) = index_type(gl_type)?;
    if count == 0 {
        return Err(Refusal71::ZeroCount);
    }
    if instances == 0 {
        return Err(Refusal71::ZeroInstances);
    }
    let offset = fit("Index Offset", offset, 32)?;
    let count = fit(if instances == 1 { "Length" } else { "Instance Length" }, count, 31)?;
    if instances == 1 {
        let mut p = vec![0u8; 10];
        p[0] = OP_INDEXED_PRIM_LIST;
        put(&mut p[1..], 0, 6, mode as u64);
        put(&mut p[1..], 6, 2, ty as u64);
        put(&mut p[1..], 8, 31, count);
        put(&mut p[1..], 39, 1, restart as u64);
        put(&mut p[1..], 40, 32, offset);
        Ok(p)
    } else {
        let n = fit("Number of Instances", instances, 32)?;
        let mut p = vec![0u8; 14];
        p[0] = OP_INDEXED_INSTANCED_PRIM_LIST;
        put(&mut p[1..], 0, 6, mode as u64);
        put(&mut p[1..], 6, 2, ty as u64);
        put(&mut p[1..], 8, 31, count);
        put(&mut p[1..], 39, 1, restart as u64);
        put(&mut p[1..], 40, 32, n);
        put(&mut p[1..], 72, 32, offset);
        Ok(p)
    }
}

/// Base Vertex Base Instance (43): 9 bytes.
pub fn pack_base(base_vertex: i64, base_instance: u64) -> Result<Vec<u8>, Refusal71> {
    let bv = i32::try_from(base_vertex).map_err(|_| Refusal71::BaseVertex(base_vertex))?;
    let bi = fit("Base Instance", base_instance, 32)?;
    let mut p = vec![0u8; 9];
    p[0] = OP_BASE_VERTEX_BASE_INSTANCE;
    put(&mut p[1..], 0, 32, bv as u32 as u64);
    put(&mut p[1..], 32, 32, bi);
    Ok(p)
}

/// Index Buffer Setup (44): 9 bytes.
pub fn pack_index_buffer(addr: u32, size: u64) -> Result<Vec<u8>, Refusal71> {
    let size = fit("Size", size, 32)?;
    let mut p = vec![0u8; 9];
    p[0] = OP_INDEX_BUFFER_SETUP;
    put(&mut p[1..], 0, 32, addr as u64);
    put(&mut p[1..], 32, 32, size);
    Ok(p)
}

/// The packets of one GL draw, in the order the control list takes them.
/// A refusal returns before anything is produced.
pub fn pack_draw(d: &Draw) -> Result<Vec<u8>, Refusal71> {
    prim_mode(d.mode)?;
    let mut out = Vec::new();
    if d.base_vertex != 0 || d.base_instance != 0 {
        out.extend(pack_base(d.base_vertex, d.base_instance)?);
    }
    match d.indices {
        None => out.extend(pack_array(d.mode, d.first, d.count, d.instances)?),
        Some(ix) => {
            let (_, bytes) = index_type(ix.gl_type)?;
            if ix.offset % bytes != 0 {
                return Err(Refusal71::IndexAlign { offset: ix.offset, index_bytes: bytes });
            }
            let end = d.count.checked_mul(bytes).and_then(|n| n.checked_add(ix.offset));
            match end {
                Some(e) if e <= ix.size => {}
                e => return Err(Refusal71::IndexRange { end: e.unwrap_or(u64::MAX), size: ix.size }),
            }
            out.extend(pack_index_buffer(ix.addr, ix.size)?);
            out.extend(pack_indexed(d.mode, ix.gl_type, ix.offset, d.count, d.instances, ix.restart)?);
        }
    }
    Ok(out)
}
