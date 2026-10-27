// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The state groups a draw writes, as the open command stream last wrote
//! them: a draw writes a group only when its words differ from those (fix F2
//! of docs/notes/gl-draw-fixed-cost.md; Mesa's etnaviv keeps `ETNA_DIRTY_*`
//! bits for the same purpose, the V3D back end its `PACK` rows).
//!
//! Each group is written into a scratch buffer first, by the same writer as
//! before (`backend.rs` `emit_draw`), and compared with the words the stream
//! holds last for it. Equal: nothing goes into the stream, and the registers
//! still hold those values. Different: the words are copied into the stream
//! and kept. So the stream a draw ends up with sets every register to the
//! value the stream without the skip would set it to, whatever the GL state:
//! no input of a group has to be listed for this to hold.
//!
//! Writing a group's words costs more CPU than storing them on a host with
//! cacheable command buffers, so the four groups that cost most to write
//! (raster, pixel engine, shader wiring, program: about 0.35 us of a host
//! draw) also carry a key: the inputs their writer reads, listed by the
//! caller ([`group_keyed`]). A key equal to the one the kept words were
//! written from skips the writer as well. A key that misses an input would
//! skip words that differ, so with [`set_verify`] on -- the default in a
//! `host-test` build, so every host test of the GC7000 runs it -- a key hit
//! writes the words anyway and panics if they are not the kept ones.
//!
//! What it rests on, and where each is kept:
//!
//! * The words kept are the ones the GPU's registers hold, so they are
//!   forgotten ([`invalidate`]) wherever that may stop being true: a new
//!   stream (`frame_open`, which writes the context reset of
//!   `emit_context_init`, and `frame_submit`, after whose resolve the stream
//!   ends), a draw taken back out of the stream (`Mark::restore`), the
//!   mipmap chain's kicks (`mipmap.rs`), the depth resolve, `gc_init` and
//!   `gc_shutdown`, and switching the skip off or on.
//! * No two groups, and nothing else written between two draws of one
//!   stream (the flushes, the stalls, the occlusion counter, the texture
//!   descriptors and samplers), write the same register. The groups'
//!   registers are listed in `backend.rs` above `emit_draw`.
//! * A group holds no register whose write does something beyond setting
//!   its value: the cache flushes, the stalls and the two instruction-cache
//!   prefetches stay outside every group and are written at every draw, as
//!   before. The shader code and the constants are one group, so the order
//!   the constant file's steering needs (`shader.rs` `emit_prefetch`) is
//!   kept whenever either is written.
//! * A key names a shader by its address and its code's GPU address: the
//!   code's memory is held while the open frame may use it (the OSAL frees
//!   it for the next kick at the earliest, `frame_open_to_osal`), so no
//!   other shader takes that address inside one stream.
//!
//! [`set_enabled`]`(false)` writes every group at every draw again, as the
//! back end did before; the host tests compare the two.

use crate::util::Global;

use super::cmd::GcCmd;

/// The groups, in the order `emit_draw` writes them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    /// The vertex elements (`emit_vertex_elements`).
    Vertex,
    /// The index stream's address, type and restart index.
    Index,
    /// The vertex streams' addresses, strides and divisors (`emit_streams`).
    Streams,
    /// Viewport, scissor and rasteriser (`emit_raster_state`).
    Raster,
    /// The rasteriser's varying packing and early depth (`emit_ra_state`).
    Ra,
    /// Multisampling (`emit_msaa_state`).
    Msaa,
    /// Depth, stencil, blending and the colour surfaces (`emit_pe_state`).
    Pe,
    /// The two stages' wiring (`shader::emit_shader_state`).
    Shader,
    /// The instruction ranges and the constant file
    /// (`shader::emit_shader_code`, `shader::emit_uniforms`).
    Program,
}

pub const GROUPS: usize = 9;

/// Room kept for each group's words, in group order. A group whose words
/// are longer than its room is written whole and not kept (the next draw
/// writes it again): the sizes cover what the writers can produce, with
/// margin, so that is not expected to happen.
const ROOM: [usize; GROUPS] = [
    64,   /* 3 x (1 + 16 + 1) */
    8,    /* 3 x 2 */
    96,   /* 3 x 16 x 2 */
    64,   /* 22 registers x 2 */
    8,    /* 2 x 2 */
    48,   /* 18 x 2 */
    128,  /* 38 x 2 with three more targets */
    160,  /* about 80 */
    1408, /* 12 x 2 + 2 x 2 + the constant file (1280) in 256-dword pieces */
];

/// Room for each group's key; 0: the group has none. Program's holds both
/// uniform blocks (at most the constant file, 1280 dwords) and 16 texture
/// bindings a stage.
const KEY_ROOM: [usize; GROUPS] = [0, 0, 0, 40, 0, 0, 112, 48, 1600];

const fn offsets(room: [usize; GROUPS]) -> [usize; GROUPS] {
    let mut off = [0; GROUPS];
    let mut i = 1;
    while i < GROUPS {
        off[i] = off[i - 1] + room[i - 1];
        i += 1;
    }
    off
}

const OFF: [usize; GROUPS] = offsets(ROOM);
const KEPT_WORDS: usize = OFF[GROUPS - 1] + ROOM[GROUPS - 1];
const KEY_OFF: [usize; GROUPS] = offsets(KEY_ROOM);
const KEY_WORDS: usize = KEY_OFF[GROUPS - 1] + KEY_ROOM[GROUPS - 1];
/// The scratch a group is written into: larger than any group (the constant
/// file of a linked program is at most 1280 dwords, `shader.rs`
/// `constant_files_fit`).
const SCRATCH_WORDS: usize = 2048;
/// `len` of a group the stream holds nothing known of; `key_len` of a group
/// with no key kept.
const UNKNOWN: u16 = u16::MAX;

struct StateCache {
    /// Off: every group written at every draw.
    off: bool,
    /// A key hit writes the words anyway and checks them ([`set_verify`]).
    verify: bool,
    /// Each group's words as the stream holds them last, at `OFF[g]`.
    kept: [u32; KEPT_WORDS],
    /// Their counts; [`UNKNOWN`] for a group not to be compared.
    len: [u16; GROUPS],
    /// The key the kept words were written from, at `KEY_OFF[g]`.
    keys: [u32; KEY_WORDS],
    key_len: [u16; GROUPS],
    scratch: [u32; SCRATCH_WORDS],
    /// The key being built.
    key_scratch: [u32; KEY_ROOM[GROUPS - 1]],
    /// Groups written into the stream, and groups skipped as equal (by
    /// their words or by their key).
    written: u32,
    skipped: u32,
}

static CACHE: Global<StateCache> = Global::new(StateCache {
    off: false,
    verify: cfg!(feature = "host-test"),
    kept: [0; KEPT_WORDS],
    len: [UNKNOWN; GROUPS],
    keys: [0; KEY_WORDS],
    key_len: [UNKNOWN; GROUPS],
    scratch: [0; SCRATCH_WORDS],
    key_scratch: [0; KEY_ROOM[GROUPS - 1]],
    written: 0,
    skipped: 0,
});

fn cache() -> &'static mut StateCache {
    // SAFETY: single-threaded by contract; each function below takes it once.
    unsafe { CACHE.get() }
}

/// Forget every group: the next draw writes each of them.
pub fn invalidate() {
    let c = cache();
    c.len = [UNKNOWN; GROUPS];
    c.key_len = [UNKNOWN; GROUPS];
}

/// Skip equal groups (`true`, the default) or write every group at every
/// draw. Either way the groups are forgotten: a stream written with the skip
/// off holds words the cache never saw.
pub fn set_enabled(on: bool) {
    cache().off = !on;
    invalidate();
}

/// Check every key hit by writing the group anyway (on by default in a
/// `host-test` build, off in every other); a hit whose words differ from the
/// kept ones panics, naming the group: its key misses an input. A timing
/// run turns it off.
pub fn set_verify(on: bool) {
    cache().verify = on;
}

/// (groups written into a stream, groups skipped as equal to the stream's
/// last) since the last [`reset_counts`].
pub fn counts() -> (u32, u32) {
    let c = cache();
    (c.written, c.skipped)
}

pub fn reset_counts() {
    let c = cache();
    c.written = 0;
    c.skipped = 0;
}

/// Write group `g` into `w`, unless its words equal the ones the stream
/// holds last for it. `emit` writes the group into the stream it is given,
/// and is called once (twice only for a group larger than the scratch,
/// which no writer produces: then it writes straight into `w`); its result
/// is returned.
pub fn group<R>(w: &mut GcCmd, g: Group, emit: impl FnMut(&mut GcCmd) -> R) -> R {
    let c = cache();
    if c.off {
        let mut emit = emit;
        return emit(w);
    }
    words(c, w, g, emit)
}

fn words<R>(c: &mut StateCache, w: &mut GcCmd, g: Group, mut emit: impl FnMut(&mut GcCmd) -> R) -> R {
    let i = g as usize;
    let mut s = GcCmd { words: c.scratch.as_mut_ptr(), used: 0, cap: SCRATCH_WORDS as u32, overflow: false };
    let r = emit(&mut s);
    if s.overflow {
        c.len[i] = UNKNOWN;
        c.written = c.written.wrapping_add(1);
        return emit(w);
    }
    let n = s.used as usize;
    let new = &c.scratch[..n];
    let kept = &mut c.kept[OFF[i]..OFF[i] + ROOM[i]];
    if usize::from(c.len[i]) == n && kept[..n] == *new {
        c.skipped = c.skipped.wrapping_add(1);
        return r;
    }
    w.append(new);
    c.written = c.written.wrapping_add(1);
    if w.overflow || n > ROOM[i] {
        /* not in the stream (the draw is taken back: `Mark::restore`), or
         * too long to keep */
        c.len[i] = UNKNOWN;
    } else {
        kept[..n].copy_from_slice(new);
        c.len[i] = n as u16;
    }
    r
}

/// A group's key being listed: the inputs its writer reads, as words.
pub struct Key<'a> {
    buf: &'a mut [u32],
    n: usize,
    /// More words than the room: no key (the group is written).
    over: bool,
}

impl Key<'_> {
    pub fn u32(&mut self, v: u32) {
        match self.buf.get_mut(self.n) {
            Some(slot) => {
                *slot = v;
                self.n += 1;
            }
            None => self.over = true,
        }
    }

    pub fn u64(&mut self, v: u64) {
        self.u32(v as u32);
        self.u32((v >> 32) as u32);
    }

    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    pub fn ptr<T>(&mut self, p: *const T) {
        self.u64(p as usize as u64);
    }

    pub fn words(&mut self, s: &[u32]) {
        self.u32(s.len() as u32);
        match self.buf.get_mut(self.n..self.n + s.len()) {
            Some(dst) => {
                dst.copy_from_slice(s);
                self.n += s.len();
            }
            None => self.over = true,
        }
    }
}

/// What [`group_keyed`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Keyed {
    /// The key equals the kept words' one: nothing was written, `emit` was
    /// not called (but with [`set_verify`] on).
    Hit,
    /// `emit` ran, with this result: the words went through [`group`].
    Ran(bool),
}

/// [`group`] for a writer whose inputs `key` lists: when they equal the
/// ones the kept words were written from, neither the words nor the writer
/// run. `key` answers false for a draw it cannot list (the group is then
/// written and keeps no key), and `emit` false for a refused draw (its key
/// is not kept).
pub fn group_keyed(
    w: &mut GcCmd,
    g: Group,
    key: impl FnOnce(&mut Key) -> bool,
    emit: impl FnMut(&mut GcCmd) -> bool,
) -> Keyed {
    let c = cache();
    if c.off {
        let mut emit = emit;
        return Keyed::Ran(emit(w));
    }
    let i = g as usize;
    let (ko, room) = (KEY_OFF[i], KEY_ROOM[i]);
    let mut k = Key { buf: &mut c.key_scratch[..room], n: 0, over: false };
    let keyed = key(&mut k) && !k.over;
    let n = k.n;
    if keyed && c.len[i] != UNKNOWN && usize::from(c.key_len[i]) == n && c.keys[ko..ko + n] == c.key_scratch[..n] {
        if c.verify {
            verify(c, g, emit);
        }
        c.skipped = c.skipped.wrapping_add(1);
        return Keyed::Hit;
    }
    let ok = words(c, w, g, emit);
    if keyed && ok && c.len[i] != UNKNOWN {
        c.keys[ko..ko + n].copy_from_slice(&c.key_scratch[..n]);
        c.key_len[i] = n as u16;
    } else {
        c.key_len[i] = UNKNOWN;
    }
    Keyed::Ran(ok)
}

/// A key hit, checked: the writer's words must be the kept ones.
fn verify(c: &mut StateCache, g: Group, mut emit: impl FnMut(&mut GcCmd) -> bool) {
    let i = g as usize;
    let mut s = GcCmd { words: c.scratch.as_mut_ptr(), used: 0, cap: SCRATCH_WORDS as u32, overflow: false };
    let ok = emit(&mut s);
    let n = s.used as usize;
    let same = ok && !s.overflow && usize::from(c.len[i]) == n && c.kept[OFF[i]..OFF[i] + n] == c.scratch[..n];
    if !same {
        panic!("vivante state dedup: group {g:?}'s key is equal and its words are not: the key misses an input");
    }
}
