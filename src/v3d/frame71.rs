// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 frame-level packets (roadmap P9; the Pi 5's V3D 7.1.10).
//!
//! Pure functions and plain structs over integers and bools; no `crate::`
//! import, so a test includes this file with `#[path]`
//! (tests/v3d71_frame.rs). Nothing hooks it into `src/v3d/mod.rs` yet: the
//! 7.1 job builder (backend71) does that.
//!
//! A 7.1 job has frame packets around the per-draw state:
//!
//! * binner list: `Tile Binning Mode Cfg` (7.1 form: width, height, log2 tile
//!   size and the two allocation block sizes; the RT count, the maximum bpp
//!   and the MSAA bit are gone, the tile size is given outright);
//! * render list: `Tile Rendering Mode Cfg (Common)` and `(ZS Clear
//!   Values)`, `Tile List Initial Block Size`, `Multicore Rendering Tile List
//!   Set Base`, `Multicore Rendering Supertile Cfg`, the two dummy tiles of
//!   the initial clear (GFXH-1742), the generic tile list's start and end,
//!   the supertile coordinates and `End of rendering`;
//! * the generic per-tile list: `Load Tile Buffer General` per loaded
//!   buffer, `End of Loads`, `Store Tile Buffer General` per stored buffer
//!   (buffer NONE when nothing is stored), then **`Clear Render Targets`**
//!   (packet 25, no fields): on 7.1 the store has no clear bit in use (the
//!   field stays in the XML; [`Op71::Store`] never sets it) and the clear of
//!   the tile buffer for the next tile is this packet.
//!
//! What is not here: the per-target config and clear colour (`Render Target
//! Part1/2/3`, item rt71), the tile-size choice (limits71) and the per-draw
//! state. The per-buffer formats, memory formats and strides are plain
//! inputs ([`Surface71`]); so is the tile size.
//!
//! The behaviour is Mesa 26.2.1's `v3dx_rcl.c` as driver/src/v3d/v3d_job71.c
//! and v3d_frame.c transcribe it; every packet's opcode, field positions and
//! widths are a [`Spec`] that tests/v3d71_frame.rs compares with the 7.1
//! definitions in `cle/v3d_packet.xml`, and every frame of the reference
//! dumps in tools/v3d/pi5/*.clif is rebuilt from its description.
//!
//! A frame the 7.1 table does not support is refused by a [`Refusal71`]
//! variant ("V3D 7.1: ..."), never by a default answer.

/// The render-target limit (`Number of Render Targets` is 4 bits; the Store
/// packet names RT 0-7 at 7.1).
pub const MAX_RTS: u32 = 8;
/// The framebuffer limit in pixels.
pub const MAX_SIZE: u32 = 7680;
/// Mesa's `V3D_MAX_SUPERTILES` (broadcom/common/v3d_limits.h).
pub const MAX_SUPERTILES: u32 = 256;
/// `Tile Binning Mode Cfg`'s block sizes as Mesa sets them: 64 bytes, and 128
/// bytes for the first block. `Tile List Initial Block Size` repeats 128.
pub const ALLOC_BLOCK: u8 = 0;
pub const ALLOC_INITIAL_BLOCK: u8 = 1;

/// `Output Image Format` D24S8 (Mesa's depth/stencil surface format).
pub const FORMAT_D24S8: u8 = 43;
/// `Output Image Format` s8: the format of a separate stencil surface.
pub const FORMAT_S8: u8 = 44;

/// `Buffer to Store` / `Buffer to Load` values (v3d_packet.xml).
pub const BUFFER_RT0: u8 = 0;
pub const BUFFER_NONE: u8 = 8;
pub const BUFFER_Z: u8 = 9;
pub const BUFFER_STENCIL: u8 = 10;
pub const BUFFER_ZSTENCIL: u8 = 11;

/// One field of a packet as the XML gives it: its start bit in the packet
/// body (after the one-byte header) and its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub start: u32,
    pub size: u32,
}

/// A packet's opcode, body length and fields: the XML's 7.1 definition as
/// this module uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    /// the XML `name` attribute
    pub name: &'static str,
    pub code: u8,
    /// body bytes after the header byte
    pub body: usize,
    pub fields: &'static [Field],
}

const fn f(name: &'static str, start: u32, size: u32) -> Field {
    Field { name, start, size }
}

pub const BINNING_CFG: Spec = Spec {
    name: "Tile Binning Mode Cfg",
    code: 120,
    body: 8,
    fields: &[
        f("Height (in pixels)", 48, 16),
        f("Width (in pixels)", 32, 16),
        f("Log2 Tile Height", 11, 3),
        f("Log2 Tile Width", 8, 3),
        f("tile allocation block size", 4, 2),
        f("tile allocation initial block size", 2, 2),
    ],
};

pub const RENDERING_COMMON: Spec = Spec {
    name: "Tile Rendering Mode Cfg (Common)",
    code: 121,
    body: 8,
    fields: &[
        f("Pad", 58, 6),
        f("Log2 Tile Height", 55, 3),
        f("Log2 Tile Width", 52, 3),
        f("Early Depth/Stencil Clear", 51, 1),
        f("Internal Depth Type", 47, 4),
        f("Early-Z disable", 46, 1),
        f("Early-Z Test and Update Direction", 45, 1),
        f("Depth-buffer disable", 44, 1),
        f("Double-buffer in non-ms mode", 43, 1),
        f("Multisample Mode (4x)", 42, 1),
        f("Image Height (pixels)", 24, 16),
        f("Image Width (pixels)", 8, 16),
        f("Number of Render Targets", 4, 4),
        f("sub-id", 0, 3),
    ],
};

pub const ZS_CLEAR_VALUES: Spec = Spec {
    name: "Tile Rendering Mode Cfg (ZS Clear Values)",
    code: 121,
    body: 8,
    fields: &[f("unused", 48, 16), f("Z Clear Value", 16, 32), f("Stencil Clear Value", 8, 8), f("sub-id", 0, 4)],
};

pub const INITIAL_BLOCK_SIZE: Spec = Spec {
    name: "Tile List Initial Block Size",
    code: 126,
    body: 1,
    fields: &[f("Use auto-chained tile lists", 2, 1), f("Size of first block in chained tile lists", 0, 2)],
};

pub const TILE_LIST_SET_BASE: Spec = Spec {
    name: "Multicore Rendering Tile List Set Base",
    code: 123,
    body: 4,
    fields: &[f("address", 6, 26), f("Tile List Set Number", 0, 4)],
};

pub const SUPERTILE_CFG: Spec = Spec {
    name: "Multicore Rendering Supertile Cfg",
    code: 122,
    body: 8,
    fields: &[
        f("Number of Bin Tile Lists", 61, 3),
        f("Supertile Raster Order", 60, 1),
        f("Multicore Enable", 56, 1),
        f("Total Frame Height in Tiles", 44, 12),
        f("Total Frame Width in Tiles", 32, 12),
        f("Total Frame Height in Supertiles", 24, 8),
        f("Total Frame Width in Supertiles", 16, 8),
        f("Supertile Height in Tiles", 8, 8),
        f("Supertile Width in Tiles", 0, 8),
    ],
};

pub const TILE_COORDS: Spec = Spec {
    name: "Tile Coordinates",
    code: 124,
    body: 3,
    fields: &[f("tile row number", 12, 12), f("tile column number", 0, 12)],
};

pub const SUPERTILE_COORDS: Spec = Spec {
    name: "Supertile Coordinates",
    code: 23,
    body: 2,
    fields: &[f("row number in supertiles", 8, 8), f("column number in supertiles", 0, 8)],
};

pub const GENERIC_TILE_LIST: Spec = Spec {
    name: "Start Address of Generic Tile List",
    code: 20,
    body: 8,
    fields: &[f("start", 0, 32), f("end", 32, 32)],
};

pub const BRANCH_IMPLICIT_TILE: Spec =
    Spec { name: "Branch to Implicit Tile List", code: 21, body: 1, fields: &[f("tile list set number", 0, 8)] };

pub const NUMBER_OF_LAYERS: Spec =
    Spec { name: "Number of Layers", code: 119, body: 1, fields: &[f("Number of Layers", 0, 8)] };

pub const STORE: Spec = Spec {
    name: "Store Tile Buffer General",
    code: 29,
    body: 12,
    fields: &[
        f("Address", 64, 32),
        f("Height", 48, 16),
        f("Height in UB or Stride", 28, 20),
        f("R/B swap", 20, 1),
        f("Channel Reverse", 19, 1),
        f("Clear buffer being stored", 18, 1),
        f("Output Image Format", 12, 6),
        f("Decimate mode", 10, 2),
        f("Dither Mode", 8, 2),
        f("Flip Y", 7, 1),
        f("Memory Format", 4, 3),
        f("Buffer to Store", 0, 4),
    ],
};

pub const LOAD: Spec = Spec {
    name: "Load Tile Buffer General",
    code: 30,
    body: 12,
    fields: &[
        f("Address", 64, 32),
        f("Height", 48, 16),
        f("Height in UB or Stride", 28, 20),
        f("R/B swap", 20, 1),
        f("Channel Reverse", 19, 1),
        f("Force alpha 1", 18, 1),
        f("Input Image Format", 12, 6),
        f("Decimate mode", 10, 2),
        f("Flip Y", 7, 1),
        f("Memory Format", 4, 3),
        f("Buffer to Load", 0, 4),
    ],
};

/// Every packet with fields.
pub const SPECS: [Spec; 13] = [
    BINNING_CFG,
    RENDERING_COMMON,
    ZS_CLEAR_VALUES,
    INITIAL_BLOCK_SIZE,
    TILE_LIST_SET_BASE,
    SUPERTILE_CFG,
    TILE_COORDS,
    SUPERTILE_COORDS,
    GENERIC_TILE_LIST,
    BRANCH_IMPLICIT_TILE,
    NUMBER_OF_LAYERS,
    STORE,
    LOAD,
];

/// The packets with no fields: their XML name and one-byte opcode.
pub const SIMPLE: [(&str, u8); 9] = [
    ("Flush", 4),
    ("Start Tile Binning", 6),
    ("End of rendering", 13),
    ("Return from sub-list", 18),
    ("Flush VCD cache", 19),
    ("Clear Render Targets", 25),
    ("End of Loads", 26),
    ("End of Tile Marker", 27),
    ("Tile Coordinates Implicit", 125),
];
pub const OP_END_RENDER: u8 = 13;
pub const OP_RETURN: u8 = 18;
pub const OP_CLEAR_VCD_CACHE: u8 = 19;
pub const OP_CLEAR_RT: u8 = 25;
pub const OP_END_LOADS: u8 = 26;
pub const OP_END_TILE: u8 = 27;
/// `Tile Coordinates Implicit` (code 125, no fields).
pub const OP_IMPLICIT_TILE_COORDS: u8 = 125;

/// Why a frame is refused. [`Refusal71::message`] is the "V3D 7.1: ..."
/// text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal71 {
    /// width or height is 0
    EmptyFrame,
    /// a side over [`MAX_SIZE`]
    FrameTooLarge,
    /// more than [`MAX_RTS`] render targets
    TooManyRenderTargets,
    /// a tile side that is not 8, 16, 32 or 64
    TileSize,
    /// double-buffer is a non-MSAA mode
    MsaaDoubleBuffer,
    /// depth kind and the depth / stencil attachments disagree
    DepthKind,
    /// a load of a render target above 3: the XML names none
    LoadRenderTarget,
    /// a field value over its width (the name is the field's)
    FieldRange(&'static str),
    /// an address that must be 64-byte aligned
    Unaligned,
    /// a Load packet asked for a dither mode, or a Store for force-alpha
    WrongPacketField,
}

impl Refusal71 {
    pub fn message(self) -> &'static str {
        match self {
            Refusal71::EmptyFrame => "V3D 7.1: a frame of 0 pixels",
            Refusal71::FrameTooLarge => "V3D 7.1: a frame side over 7680 pixels",
            Refusal71::TooManyRenderTargets => "V3D 7.1: more than 8 render targets",
            Refusal71::TileSize => "V3D 7.1: a tile side other than 8, 16, 32 or 64 pixels",
            Refusal71::MsaaDoubleBuffer => "V3D 7.1: double-buffer mode with 4x MSAA",
            Refusal71::DepthKind => "V3D 7.1: the depth type and the depth/stencil attachments disagree",
            Refusal71::LoadRenderTarget => "V3D 7.1: Load Tile Buffer General names no render target above 3",
            Refusal71::FieldRange(_) => "V3D 7.1: a packet field value over its width",
            Refusal71::Unaligned => "V3D 7.1: an address that is not 64-byte aligned",
            Refusal71::WrongPacketField => "V3D 7.1: a field the packet does not have",
        }
    }
}

/// Write `v` into `b` (header at byte 0) at body bit `start`, `size` wide.
fn put(b: &mut [u8], spec: &Spec, name: &'static str, v: u64) -> Result<(), Refusal71> {
    let fld = spec.fields.iter().find(|x| x.name == name).expect("field of the spec");
    if fld.size < 64 && v >> fld.size != 0 {
        return Err(Refusal71::FieldRange(name));
    }
    for i in 0..fld.size {
        if v >> i & 1 != 0 {
            let bit = fld.start + i;
            b[1 + (bit / 8) as usize] |= 1 << (bit % 8);
        }
    }
    Ok(())
}

fn new_packet<const N: usize>(spec: &Spec) -> [u8; N] {
    assert_eq!(N, spec.body + 1);
    let mut b = [0u8; N];
    b[0] = spec.code;
    b
}

fn log2_tile(edge: u32) -> Result<u8, Refusal71> {
    match edge {
        8 => Ok(0),
        16 => Ok(1),
        32 => Ok(2),
        64 => Ok(3),
        _ => Err(Refusal71::TileSize),
    }
}

/// `Tile Binning Mode Cfg`, 7.1 form. Width and height are the logical
/// values; the packet stores them minus one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BinningCfg71 {
    pub width: u32,
    pub height: u32,
    pub log2_tile_width: u8,
    pub log2_tile_height: u8,
    pub alloc_block_size: u8,
    pub alloc_initial_block_size: u8,
}

impl BinningCfg71 {
    pub fn pack(&self) -> Result<[u8; 9], Refusal71> {
        let s = &BINNING_CFG;
        let mut b = new_packet::<9>(s);
        put(&mut b, s, "Height (in pixels)", (self.height as u64).wrapping_sub(1))?;
        put(&mut b, s, "Width (in pixels)", (self.width as u64).wrapping_sub(1))?;
        put(&mut b, s, "Log2 Tile Height", self.log2_tile_height as u64)?;
        put(&mut b, s, "Log2 Tile Width", self.log2_tile_width as u64)?;
        put(&mut b, s, "tile allocation block size", self.alloc_block_size as u64)?;
        put(&mut b, s, "tile allocation initial block size", self.alloc_initial_block_size as u64)?;
        Ok(b)
    }
}

/// `Tile Rendering Mode Cfg (Common)`. `render_targets` is the logical
/// count (1-8; the packet stores it minus one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderingCommon71 {
    pub render_targets: u32,
    pub width: u32,
    pub height: u32,
    pub msaa: bool,
    pub double_buffer: bool,
    pub depth_buffer_disable: bool,
    pub early_z_gt: bool,
    pub early_z_disable: bool,
    pub internal_depth_type: u8,
    pub early_ds_clear: bool,
    pub log2_tile_width: u8,
    pub log2_tile_height: u8,
}

impl RenderingCommon71 {
    pub fn pack(&self) -> Result<[u8; 9], Refusal71> {
        let s = &RENDERING_COMMON;
        let mut b = new_packet::<9>(s);
        put(&mut b, s, "Log2 Tile Height", self.log2_tile_height as u64)?;
        put(&mut b, s, "Log2 Tile Width", self.log2_tile_width as u64)?;
        put(&mut b, s, "Early Depth/Stencil Clear", self.early_ds_clear as u64)?;
        put(&mut b, s, "Internal Depth Type", self.internal_depth_type as u64)?;
        put(&mut b, s, "Early-Z disable", self.early_z_disable as u64)?;
        put(&mut b, s, "Early-Z Test and Update Direction", self.early_z_gt as u64)?;
        put(&mut b, s, "Depth-buffer disable", self.depth_buffer_disable as u64)?;
        put(&mut b, s, "Double-buffer in non-ms mode", self.double_buffer as u64)?;
        put(&mut b, s, "Multisample Mode (4x)", self.msaa as u64)?;
        put(&mut b, s, "Image Height (pixels)", self.height as u64)?;
        put(&mut b, s, "Image Width (pixels)", self.width as u64)?;
        put(&mut b, s, "Number of Render Targets", (self.render_targets as u64).wrapping_sub(1))?;
        Ok(b)
    }
}

/// `Tile Rendering Mode Cfg (ZS Clear Values)`: Z as the float's bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZsClear71 {
    pub z_bits: u32,
    pub stencil: u8,
}

impl ZsClear71 {
    pub fn pack(&self) -> Result<[u8; 9], Refusal71> {
        let s = &ZS_CLEAR_VALUES;
        let mut b = new_packet::<9>(s);
        put(&mut b, s, "Z Clear Value", self.z_bits as u64)?;
        put(&mut b, s, "Stencil Clear Value", self.stencil as u64)?;
        put(&mut b, s, "sub-id", 1)?;
        Ok(b)
    }
}

/// `Tile List Initial Block Size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitialBlock71 {
    pub auto_chained: bool,
    pub first_block_size: u8,
}

impl InitialBlock71 {
    pub fn pack(&self) -> Result<[u8; 2], Refusal71> {
        let s = &INITIAL_BLOCK_SIZE;
        let mut b = new_packet::<2>(s);
        put(&mut b, s, "Use auto-chained tile lists", self.auto_chained as u64)?;
        put(&mut b, s, "Size of first block in chained tile lists", self.first_block_size as u64)?;
        Ok(b)
    }
}

/// `Multicore Rendering Supertile Cfg` (one core, one bin tile list). The
/// width and height in tiles are the logical values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupertileCfg71 {
    pub supertile_width: u32,
    pub supertile_height: u32,
    pub frame_width_supertiles: u32,
    pub frame_height_supertiles: u32,
    pub frame_width_tiles: u32,
    pub frame_height_tiles: u32,
    pub bin_tile_lists: u32,
}

impl SupertileCfg71 {
    pub fn pack(&self) -> Result<[u8; 9], Refusal71> {
        let s = &SUPERTILE_CFG;
        let mut b = new_packet::<9>(s);
        put(&mut b, s, "Number of Bin Tile Lists", (self.bin_tile_lists as u64).wrapping_sub(1))?;
        put(&mut b, s, "Total Frame Height in Tiles", self.frame_height_tiles as u64)?;
        put(&mut b, s, "Total Frame Width in Tiles", self.frame_width_tiles as u64)?;
        put(&mut b, s, "Total Frame Height in Supertiles", self.frame_height_supertiles as u64)?;
        put(&mut b, s, "Total Frame Width in Supertiles", self.frame_width_supertiles as u64)?;
        put(&mut b, s, "Supertile Height in Tiles", (self.supertile_height as u64).wrapping_sub(1))?;
        put(&mut b, s, "Supertile Width in Tiles", (self.supertile_width as u64).wrapping_sub(1))?;
        Ok(b)
    }
}

/// `Multicore Rendering Tile List Set Base`: the 64-byte aligned tile
/// allocation address, set 0.
pub fn pack_tile_list_base(address: u32, set: u8) -> Result<[u8; 5], Refusal71> {
    if address & 63 != 0 {
        return Err(Refusal71::Unaligned);
    }
    let s = &TILE_LIST_SET_BASE;
    let mut b = new_packet::<5>(s);
    put(&mut b, s, "address", (address >> 6) as u64)?;
    put(&mut b, s, "Tile List Set Number", set as u64)?;
    Ok(b)
}

/// `Number of Layers` (logical count, stored minus one).
pub fn pack_number_of_layers(layers: u32) -> Result<[u8; 2], Refusal71> {
    let s = &NUMBER_OF_LAYERS;
    let mut b = new_packet::<2>(s);
    put(&mut b, s, "Number of Layers", (layers as u64).wrapping_sub(1))?;
    Ok(b)
}

/// What a Load or a Store packet carries about one buffer. The format,
/// memory format, stride and swap come from the render-target item (rt71) or
/// the depth buffer's own answer; here they are plain inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Surface71 {
    /// `Output Image Format` / `Input Image Format` (6 bits)
    pub format: u8,
    /// `Memory Format` (0 raster, 4 UIF no XOR, ...; 3 bits)
    pub memory_format: u8,
    /// UIF height in UIF blocks, or the raster byte stride (20 bits)
    pub height_in_ub_or_stride: u32,
    pub address: u32,
    pub r_b_swap: bool,
    pub channel_reverse: bool,
    pub flip_y: bool,
    /// the Y-flip height (16 bits)
    pub height: u32,
    /// `Decimate Mode`: 0 sample 0, 1 4x, 3 all samples
    pub decimate: u8,
    /// Store only: `Dither Mode`
    pub dither: u8,
    /// Load only: `Force alpha 1`
    pub force_alpha_1: bool,
}

/// One Load or Store packet: the buffer and its surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transfer71 {
    pub buffer: u8,
    pub surface: Surface71,
}

fn check_transfer(t: &Transfer71) -> Result<(), Refusal71> {
    let s = &t.surface;
    if s.decimate == 2 {
        return Err(Refusal71::FieldRange("Decimate mode"));
    }
    if s.memory_format > 5 {
        return Err(Refusal71::FieldRange("Memory Format"));
    }
    Ok(())
}

impl Transfer71 {
    /// `Store Tile Buffer General`. "Clear buffer being stored" is never
    /// set: 7.1 clears with `Clear Render Targets`.
    pub fn pack_store(&self) -> Result<[u8; 13], Refusal71> {
        check_transfer(self)?;
        if self.surface.force_alpha_1 {
            return Err(Refusal71::WrongPacketField);
        }
        let s = &STORE;
        let u = &self.surface;
        let mut b = new_packet::<13>(s);
        put(&mut b, s, "Address", u.address as u64)?;
        put(&mut b, s, "Height", u.height as u64)?;
        put(&mut b, s, "Height in UB or Stride", u.height_in_ub_or_stride as u64)?;
        put(&mut b, s, "R/B swap", u.r_b_swap as u64)?;
        put(&mut b, s, "Channel Reverse", u.channel_reverse as u64)?;
        put(&mut b, s, "Output Image Format", u.format as u64)?;
        put(&mut b, s, "Decimate mode", u.decimate as u64)?;
        put(&mut b, s, "Dither Mode", u.dither as u64)?;
        put(&mut b, s, "Flip Y", u.flip_y as u64)?;
        put(&mut b, s, "Memory Format", u.memory_format as u64)?;
        put(&mut b, s, "Buffer to Store", self.buffer as u64)?;
        Ok(b)
    }

    /// `Load Tile Buffer General`.
    pub fn pack_load(&self) -> Result<[u8; 13], Refusal71> {
        check_transfer(self)?;
        if self.surface.dither != 0 {
            return Err(Refusal71::WrongPacketField);
        }
        let s = &LOAD;
        let u = &self.surface;
        let mut b = new_packet::<13>(s);
        put(&mut b, s, "Address", u.address as u64)?;
        put(&mut b, s, "Height", u.height as u64)?;
        put(&mut b, s, "Height in UB or Stride", u.height_in_ub_or_stride as u64)?;
        put(&mut b, s, "R/B swap", u.r_b_swap as u64)?;
        put(&mut b, s, "Channel Reverse", u.channel_reverse as u64)?;
        put(&mut b, s, "Force alpha 1", u.force_alpha_1 as u64)?;
        put(&mut b, s, "Input Image Format", u.format as u64)?;
        put(&mut b, s, "Decimate mode", u.decimate as u64)?;
        put(&mut b, s, "Flip Y", u.flip_y as u64)?;
        put(&mut b, s, "Memory Format", u.memory_format as u64)?;
        put(&mut b, s, "Buffer to Load", self.buffer as u64)?;
        Ok(b)
    }
}

/// One packet of a render list or tile list that the frame plan produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op71 {
    TileCoords {
        column: u32,
        row: u32,
    },
    ImplicitTileCoords,
    EndLoads,
    Load(Transfer71),
    Store(Transfer71),
    /// `Clear Render Targets`: one byte, no fields
    ClearRt,
    EndTile,
    ClearVcdCache,
    GenericTileList {
        start: u32,
        end: u32,
    },
    SupertileCoords {
        column: u32,
        row: u32,
    },
    BranchImplicitTile {
        set: u8,
    },
    EndRender,
    ReturnFromSubList,
}

impl Op71 {
    pub fn pack(&self) -> Result<Vec<u8>, Refusal71> {
        Ok(match *self {
            Op71::TileCoords { column, row } => {
                let s = &TILE_COORDS;
                let mut b = new_packet::<4>(s);
                put(&mut b, s, "tile column number", column as u64)?;
                put(&mut b, s, "tile row number", row as u64)?;
                b.to_vec()
            }
            Op71::ImplicitTileCoords => vec![OP_IMPLICIT_TILE_COORDS],
            Op71::EndLoads => vec![OP_END_LOADS],
            Op71::Load(t) => t.pack_load()?.to_vec(),
            Op71::Store(t) => t.pack_store()?.to_vec(),
            Op71::ClearRt => vec![OP_CLEAR_RT],
            Op71::EndTile => vec![OP_END_TILE],
            Op71::ClearVcdCache => vec![OP_CLEAR_VCD_CACHE],
            Op71::GenericTileList { start, end } => {
                let s = &GENERIC_TILE_LIST;
                let mut b = new_packet::<9>(s);
                put(&mut b, s, "start", start as u64)?;
                put(&mut b, s, "end", end as u64)?;
                b.to_vec()
            }
            Op71::SupertileCoords { column, row } => {
                let s = &SUPERTILE_COORDS;
                let mut b = new_packet::<3>(s);
                put(&mut b, s, "column number in supertiles", column as u64)?;
                put(&mut b, s, "row number in supertiles", row as u64)?;
                b.to_vec()
            }
            Op71::BranchImplicitTile { set } => {
                let s = &BRANCH_IMPLICIT_TILE;
                let mut b = new_packet::<2>(s);
                put(&mut b, s, "tile list set number", set as u64)?;
                b.to_vec()
            }
            Op71::EndRender => vec![OP_END_RENDER],
            Op71::ReturnFromSubList => vec![OP_RETURN],
        })
    }
}

/// What the frame's depth/stencil buffer is, for `Internal Depth Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthKind71 {
    /// no depth or stencil buffer: depth_32f, the field's zero
    None,
    D32F,
    D24,
    D16,
    /// a stencil-only buffer, which Mesa keeps as D24S8: depth_24
    StencilOnly,
}

impl DepthKind71 {
    pub fn internal_type(self) -> u8 {
        match self {
            DepthKind71::None | DepthKind71::D32F => 0,
            DepthKind71::D24 | DepthKind71::StencilOnly => 1,
            DepthKind71::D16 => 2,
        }
    }
}

/// What happens to a buffer when the frame ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndOp71 {
    Store,
    /// not stored: the contents are dropped (no packet)
    Invalidate,
}

/// One attached buffer: its surface, whether the tile list loads it, and
/// what happens to it at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attach71 {
    pub surface: Surface71,
    pub load: bool,
    pub end: EndOp71,
}

/// The depth/stencil surface of a frame, with what the tile list does to its
/// depth and its stencil part. Mesa picks the buffer number from the parts
/// in use (`zs_buffer_from_pipe_bits`): both is Z+Stencil, depth alone Z,
/// stencil alone Stencil, whatever the surface's format. A part of a
/// separate stencil surface ([`Frame71::stencil`]) is not in this struct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZsAttach71 {
    pub surface: Surface71,
    pub load_z: bool,
    pub load_s: bool,
    pub store_z: bool,
    pub store_s: bool,
}

/// The plain frame description.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame71 {
    pub width: u32,
    pub height: u32,
    /// the tile size in pixels (8, 16, 32 or 64), from limits71
    pub tile_width: u32,
    pub tile_height: u32,
    /// the number of render targets of the job; a slot of `rt` that is
    /// `None` is configured unused (no load, no store)
    pub render_targets: u32,
    pub msaa: bool,
    pub double_buffer: bool,
    pub rt: [Option<Attach71>; MAX_RTS as usize],
    pub depth_kind: DepthKind71,
    /// the depth (and stencil) surface
    pub zs: Option<ZsAttach71>,
    /// a separate stencil surface (not with a stencil part in `zs`)
    pub stencil: Option<Attach71>,
    pub depth_buffer_disable: bool,
    pub early_z_disable: bool,
    pub early_z_gt: bool,
    /// `Early Depth/Stencil Clear`
    pub early_ds_clear: bool,
    /// every tile starts from the clear values (Mesa's `job->clear_tlb`):
    /// `Clear Render Targets` ends the tile list
    pub clear_tile_buffers: bool,
    pub z_clear_bits: u32,
    pub stencil_clear: u8,
}

impl Frame71 {
    /// A colour-only frame of `rt0`'s kind with nothing loaded: the plain
    /// starting point for a description.
    pub fn new(width: u32, height: u32, tile_width: u32, tile_height: u32) -> Frame71 {
        Frame71 {
            width,
            height,
            tile_width,
            tile_height,
            render_targets: 1,
            msaa: false,
            double_buffer: false,
            rt: [None; MAX_RTS as usize],
            depth_kind: DepthKind71::None,
            zs: None,
            stencil: None,
            depth_buffer_disable: false,
            early_z_disable: true,
            early_z_gt: false,
            early_ds_clear: false,
            clear_tile_buffers: true,
            z_clear_bits: 0,
            stencil_clear: 0,
        }
    }
}

/// The packets of a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan71 {
    /// binner list
    pub binning: BinningCfg71,
    /// render list, in the order of Mesa's `emit_rcl`
    pub common: RenderingCommon71,
    pub zs_clear: ZsClear71,
    pub initial_block: InitialBlock71,
    pub supertile_cfg: SupertileCfg71,
    /// the two dummy tiles of the initial clear, each from `TileCoords` to
    /// `EndTile`
    pub initial_tiles: Vec<Op71>,
    /// the generic list's loads (after `ImplicitTileCoords`, before
    /// `EndLoads`)
    pub list_loads: Vec<Op71>,
    /// the generic list's stores, the `ClearRt` when the tile buffers are
    /// cleared, and `EndTile` (after the branch to the primitives)
    pub list_stores: Vec<Op71>,
    /// `SupertileCoords`, row by row
    pub supertiles: Vec<Op71>,
}

fn range(v: u32, bits: u32, name: &'static str) -> Result<(), Refusal71> {
    if v >> bits != 0 {
        Err(Refusal71::FieldRange(name))
    } else {
        Ok(())
    }
}

/// Mesa's `zs_buffer_from_pipe_bits`: `None` when no part is used.
fn zs_buffer(z: bool, s: bool) -> Option<u8> {
    match (z, s) {
        (true, true) => Some(BUFFER_ZSTENCIL),
        (true, false) => Some(BUFFER_Z),
        (false, true) => Some(BUFFER_STENCIL),
        (false, false) => None,
    }
}

/// Mesa's `emit_render_layer`: grow the supertile (width first once equal)
/// until the frame has under 256 supertiles on a side and at most 256 in
/// all. Returns (supertile w, h, frame w, h in supertiles).
pub fn choose_supertile(tiles_x: u32, tiles_y: u32) -> (u32, u32, u32, u32) {
    let (mut sw, mut sh) = (1, 1);
    loop {
        let fw = tiles_x.div_ceil(sw);
        let fh = tiles_y.div_ceil(sh);
        if fw < MAX_SUPERTILES && fh < MAX_SUPERTILES && fw * fh <= MAX_SUPERTILES {
            return (sw, sh, fw, fh);
        }
        if sw < sh {
            sw += 1;
        } else {
            sh += 1;
        }
    }
}

/// Plan a frame, or name why 7.1 does not support it.
pub fn plan(fr: &Frame71) -> Result<Plan71, Refusal71> {
    if fr.width == 0 || fr.height == 0 {
        return Err(Refusal71::EmptyFrame);
    }
    if fr.width > MAX_SIZE || fr.height > MAX_SIZE {
        return Err(Refusal71::FrameTooLarge);
    }
    if fr.render_targets > MAX_RTS {
        return Err(Refusal71::TooManyRenderTargets);
    }
    let lw = log2_tile(fr.tile_width)?;
    let lh = log2_tile(fr.tile_height)?;
    if fr.msaa && fr.double_buffer {
        return Err(Refusal71::MsaaDoubleBuffer);
    }
    let has_depth = fr.zs.is_some() || fr.stencil.is_some();
    let kind_ok = match fr.depth_kind {
        DepthKind71::None => !has_depth,
        DepthKind71::StencilOnly => fr.stencil.is_none() && fr.zs.is_some_and(|z| !z.load_z && !z.store_z),
        _ => fr.zs.is_some(),
    };
    // a separate stencil surface and a stencil part of the depth surface
    // are two ways to the same buffer
    if fr.stencil.is_some() && fr.zs.is_some_and(|z| z.load_s || z.store_s) {
        return Err(Refusal71::DepthKind);
    }
    if !kind_ok {
        return Err(Refusal71::DepthKind);
    }
    for (n, a) in fr.rt.iter().enumerate() {
        if let Some(a) = a {
            if n as u32 >= fr.render_targets {
                return Err(Refusal71::TooManyRenderTargets);
            }
            if a.load && n > 3 {
                return Err(Refusal71::LoadRenderTarget);
            }
        }
    }

    let tiles_x = fr.width.div_ceil(fr.tile_width);
    let tiles_y = fr.height.div_ceil(fr.tile_height);
    let (sw, sh, fw, fh) = choose_supertile(tiles_x, tiles_y);
    range(tiles_x, 12, "Total Frame Width in Tiles")?;
    range(tiles_y, 12, "Total Frame Height in Tiles")?;

    let binning = BinningCfg71 {
        width: fr.width,
        height: fr.height,
        log2_tile_width: lw,
        log2_tile_height: lh,
        alloc_block_size: ALLOC_BLOCK,
        alloc_initial_block_size: ALLOC_INITIAL_BLOCK,
    };
    let common = RenderingCommon71 {
        render_targets: fr.render_targets.max(1),
        width: fr.width,
        height: fr.height,
        msaa: fr.msaa,
        double_buffer: fr.double_buffer,
        depth_buffer_disable: fr.depth_buffer_disable,
        early_z_gt: fr.early_z_gt,
        early_z_disable: fr.early_z_disable,
        internal_depth_type: fr.depth_kind.internal_type(),
        early_ds_clear: fr.early_ds_clear,
        log2_tile_width: lw,
        log2_tile_height: lh,
    };
    let zs_clear = ZsClear71 { z_bits: fr.z_clear_bits, stencil: fr.stencil_clear };
    let initial_block = InitialBlock71 { auto_chained: true, first_block_size: ALLOC_INITIAL_BLOCK };
    let supertile_cfg = SupertileCfg71 {
        supertile_width: sw,
        supertile_height: sh,
        frame_width_supertiles: fw,
        frame_height_supertiles: fh,
        frame_width_tiles: tiles_x,
        frame_height_tiles: tiles_y,
        bin_tile_lists: 1,
    };

    // the initial clear and the GFXH-1742 workaround: two dummy tiles that
    // store buffer NONE, the clear in the first (and in both with
    // double-buffer)
    let none = Op71::Store(Transfer71 { buffer: BUFFER_NONE, surface: Surface71::default() });
    let mut initial_tiles = Vec::new();
    for i in 0..2 {
        initial_tiles.push(Op71::TileCoords { column: 0, row: 0 });
        initial_tiles.push(Op71::EndLoads);
        initial_tiles.push(none);
        if i == 0 || fr.double_buffer {
            initial_tiles.push(Op71::ClearRt);
        }
        initial_tiles.push(Op71::EndTile);
    }

    // v3d_rcl_emit_loads: the RTs in order, a separate stencil, then Z or Z+S
    let mut list_loads = Vec::new();
    for (n, a) in fr.rt.iter().enumerate() {
        if let Some(a) = a.filter(|a| a.load) {
            list_loads.push(Op71::Load(Transfer71 { buffer: BUFFER_RT0 + n as u8, surface: a.surface }));
        }
    }
    if let Some(a) = fr.stencil.filter(|a| a.load) {
        list_loads.push(Op71::Load(Transfer71 { buffer: BUFFER_STENCIL, surface: a.surface }));
    }
    if let Some(z) = fr.zs {
        if let Some(buffer) = zs_buffer(z.load_z, z.load_s) {
            list_loads.push(Op71::Load(Transfer71 { buffer, surface: z.surface }));
        }
    }

    // v3d_rcl_emit_stores: the RTs in order, Z or Z+S, then the separate
    // stencil; nothing to store stores buffer NONE
    let mut list_stores = Vec::new();
    let stored = |a: &Option<Attach71>| a.filter(|a| a.end == EndOp71::Store);
    for (n, a) in fr.rt.iter().enumerate() {
        if let Some(a) = stored(a) {
            list_stores.push(Op71::Store(Transfer71 { buffer: BUFFER_RT0 + n as u8, surface: a.surface }));
        }
    }
    if let Some(z) = fr.zs {
        if let Some(buffer) = zs_buffer(z.store_z, z.store_s) {
            list_stores.push(Op71::Store(Transfer71 { buffer, surface: z.surface }));
        }
    }
    if let Some(a) = stored(&fr.stencil) {
        list_stores.push(Op71::Store(Transfer71 { buffer: BUFFER_STENCIL, surface: a.surface }));
    }
    if list_stores.is_empty() {
        list_stores.push(none);
    }
    if fr.clear_tile_buffers {
        list_stores.push(Op71::ClearRt);
    }
    list_stores.push(Op71::EndTile);

    let mut supertiles = Vec::new();
    for row in 0..fh {
        for column in 0..fw {
            supertiles.push(Op71::SupertileCoords { column, row });
        }
    }

    let p = Plan71 {
        binning,
        common,
        zs_clear,
        initial_block,
        supertile_cfg,
        initial_tiles,
        list_loads,
        list_stores,
        supertiles,
    };
    // every packet must pack: a value over its field is a refusal now, not
    // at emit time
    p.binning.pack()?;
    p.common.pack()?;
    p.zs_clear.pack()?;
    p.initial_block.pack()?;
    p.supertile_cfg.pack()?;
    for op in p.list_loads.iter().chain(&p.list_stores) {
        op.pack()?;
    }
    Ok(p)
}
