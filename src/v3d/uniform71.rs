// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 table's uniform stream fill (docs/notes/v3d71-gl-table.md).
//!
//! Pure over plain words, no `crate::` import. A program's uniform list is
//! `(contents, data)` pairs in the order the QPU code consumes them (the
//! 7.1 online compiler's `uniforms`, `tools/v3d/gltest-shaders71/*.json`).
//! The fill is the 4.2 contract (`driver/src/v3d/v3d_frame.c`,
//! `write_uniforms`; `driver/src/v3d/v3d_job71.c` is its C twin) with one
//! difference: the clipper's XY granularity is 64 on 7.1, 256 on 4.2.
//!
//! | contents | word |
//! |---|---|
//! | 0 `CONSTANT` | `data` |
//! | 1 `UNIFORM` | `gl_words[data]` (the stage's GL uniform block) |
//! | 2 `VIEWPORT_X_SCALE` | `scale[0] * 64` as f32 bits |
//! | 3 `VIEWPORT_Y_SCALE` | `scale[1] * 64` |
//! | 4 `VIEWPORT_Z_OFFSET` | `translate[2]` |
//! | 5 `VIEWPORT_Z_SCALE` | `scale[2]` |
//!
//! Every other contents is a named [`Refusal`], never a 0 word. The clear
//! colour of the gl_test clear program is a `UNIFORM` list: the caller puts
//! the colour's four f32 bit patterns in `gl_words`.

/// Mesa's `enum quniform_contents` (src/v3d/ffi.rs, select.rs).
pub const CONSTANT: u32 = 0;
pub const UNIFORM: u32 = 1;
pub const VIEWPORT_X_SCALE: u32 = 2;
pub const VIEWPORT_Y_SCALE: u32 = 3;
pub const VIEWPORT_Z_OFFSET: u32 = 4;
pub const VIEWPORT_Z_SCALE: u32 = 5;
pub const TMU_CONFIG_P0: u32 = 40;
pub const TMU_CONFIG_P1: u32 = 41;
pub const TEXTURE_WIDTH: u32 = 44;
pub const TEXTURE_HEIGHT: u32 = 45;
pub const TEXTURE_DEPTH: u32 = 46;
pub const TEXTURE_ARRAY_SIZE: u32 = 47;
pub const FB_WPOS_Y_SCALE: u32 = 0x10000;
pub const FB_WPOS_Y_OFFSET: u32 = 0x10001;
pub const FB_PNTC_Y_SCALE: u32 = 0x10002;
pub const FB_PNTC_Y_OFFSET: u32 = 0x10003;

/// `devinfo->clipper_xy_granularity` at 7.1 (1/64 pixel).
pub const CLIPPER_XY_GRANULARITY: f32 = 64.0;

/// One uniform stream word to produce (`struct v3d_uniform`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    pub contents: u32,
    pub data: u32,
}

/// gallium's `pipe_viewport_state`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub scale: [f32; 3],
    pub translate: [f32; 3],
}

impl Viewport {
    /// A `w` x `h` pixel viewport with depth range `near..far`: scale `w/2`,
    /// `h/2`, `(far-near)/2`; Z translate `(far+near)/2` (the X and Y
    /// translates are not read by any uniform).
    pub fn new(w: u32, h: u32, near: f32, far: f32) -> Viewport {
        Viewport {
            scale: [w as f32 / 2.0, h as f32 / 2.0, (far - near) / 2.0],
            translate: [w as f32 / 2.0, h as f32 / 2.0, (far + near) / 2.0],
        }
    }
}

/// Why a stream is not filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `UNIFORM` word `index` of a block of `count` words (or no block).
    UniformIndex { index: u32, count: usize },
    /// Texture size words (`textureSize`): the 7.1 texture state is P10.
    TextureSize { contents: u32 },
    /// TMU configuration words 0 and 1: the 7.1 texture state is P10.
    TmuConfig { contents: u32 },
    /// `gl_FragCoord.y` / `gl_PointCoord.y` window transform words.
    FragCoordY { contents: u32 },
    /// Any other contents.
    Unknown { contents: u32 },
    /// The output slice holds fewer words than the list.
    OutputTooSmall { need: usize, have: usize },
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Refusal::UniformIndex { index, count } => {
                write!(f, "uniform word {index} is outside the stage's {count}-word GL uniform block")
            }
            Refusal::TextureSize { contents } => {
                write!(f, "texture size uniform (contents {contents}): the 7.1 texture state is not written yet")
            }
            Refusal::TmuConfig { contents } => {
                write!(f, "TMU config uniform (contents {contents}): the 7.1 texture state is not written yet")
            }
            Refusal::FragCoordY { contents } => {
                write!(f, "gl_FragCoord/gl_PointCoord y transform (contents {contents:#x}) is not given on 7.1 yet")
            }
            Refusal::Unknown { contents } => write!(f, "uniform contents {contents:#x} is not supported on 7.1"),
            Refusal::OutputTooSmall { need, have } => write!(f, "uniform stream needs {need} words, room for {have}"),
        }
    }
}

/// One word of the stream.
pub fn word(w: Word, vp: &Viewport, gl_words: &[u32]) -> Result<u32, Refusal> {
    Ok(match w.contents {
        CONSTANT => w.data,
        UNIFORM => match gl_words.get(w.data as usize) {
            Some(&v) => v,
            None => return Err(Refusal::UniformIndex { index: w.data, count: gl_words.len() }),
        },
        VIEWPORT_X_SCALE => (vp.scale[0] * CLIPPER_XY_GRANULARITY).to_bits(),
        VIEWPORT_Y_SCALE => (vp.scale[1] * CLIPPER_XY_GRANULARITY).to_bits(),
        VIEWPORT_Z_OFFSET => vp.translate[2].to_bits(),
        VIEWPORT_Z_SCALE => vp.scale[2].to_bits(),
        TMU_CONFIG_P0 | TMU_CONFIG_P1 => return Err(Refusal::TmuConfig { contents: w.contents }),
        TEXTURE_WIDTH | TEXTURE_HEIGHT | TEXTURE_DEPTH | TEXTURE_ARRAY_SIZE => {
            return Err(Refusal::TextureSize { contents: w.contents });
        }
        FB_WPOS_Y_SCALE | FB_WPOS_Y_OFFSET | FB_PNTC_Y_SCALE | FB_PNTC_Y_OFFSET => {
            return Err(Refusal::FragCoordY { contents: w.contents });
        }
        c => return Err(Refusal::Unknown { contents: c }),
    })
}

/// Fill `out[..list.len()]` and return the word count. Nothing past the
/// first refusal is meaningful; the caller drops the stream.
pub fn fill(list: &[Word], vp: &Viewport, gl_words: &[u32], out: &mut [u32]) -> Result<usize, Refusal> {
    if out.len() < list.len() {
        return Err(Refusal::OutputTooSmall { need: list.len(), have: out.len() });
    }
    for (o, w) in out.iter_mut().zip(list) {
        *o = word(*w, vp, gl_words)?;
    }
    Ok(list.len())
}
