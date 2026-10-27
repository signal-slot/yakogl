// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A bump arena over GPU-visible memory the caller owns: the place a frame's
//! descriptors, shader programs and vertex data are written.
//!
//! The caller gives the memory as a CPU slice and the GPU address its first
//! byte has; a block's GPU address is that base plus the block's offset.
//! Alignment is reckoned on the GPU address, which is what the hardware
//! sees. A block that does not fit is refused ([`ArenaError::Full`]), never
//! wrapped or clamped, and every block starts zero-filled, so a descriptor
//! field nobody writes is 0 whatever the memory held before.
//!
//! The arena keeps the most it held ([`Arena::high_water`]); its owner logs
//! that when it rises past every earlier frame's (`backend.rs`): a limit set
//! while the frames were small is reached later without anyone deciding to
//! reach it (CLAUDE.md).
//!
//! A [`Pool`] chains arenas over several memories, so a frame's data grows
//! past its first chunk without moving a block.
//!
//! [`Arena::reset`] gives the whole arena back. It is safe only when the GPU
//! no longer reads anything in it: today no frame is ever submitted
//! (`backend.rs`, "no submit path yet"); the submit path must reset only
//! after the frame's fence.

use core::fmt;

/// Why an arena refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    /// The block does not fit in what is left.
    Full { label: &'static str, need: usize, align: u64, free: usize },
    /// An alignment that is not a power of two.
    BadAlign { label: &'static str, align: u64 },
    /// The memory's last byte would be past the top of the 64-bit space.
    BadBase { label: &'static str, base: u64, len: usize },
    /// A write past the end of the block it names (a builder defect).
    OutOfBlock { label: &'static str, at: usize, len: usize, block_len: usize },
    /// A block this arena did not hand out (or hands out no longer, after a
    /// reset): another arena's, or one forged. Its address or its extent is
    /// not an allocated range of this arena.
    ForeignBlock { label: &'static str, offset: usize, len: usize, gpu: u64 },
}

impl fmt::Display for ArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ArenaError::Full { label, need, align, free } => {
                write!(f, "mali arena '{label}' is full: {need} bytes aligned to {align} do not fit in the {free} left")
            }
            ArenaError::BadAlign { label, align } => {
                write!(f, "mali arena '{label}': alignment {align} is not a power of two")
            }
            ArenaError::BadBase { label, base, len } => {
                write!(f, "mali arena '{label}': {len} bytes at {base:#x} pass the top of the address space")
            }
            ArenaError::ForeignBlock { label, offset, len, gpu } => write!(
                f,
                "mali arena '{label}': a block of {len} bytes at offset {offset} (GPU {gpu:#x}) is not one this arena holds"
            ),
            ArenaError::OutOfBlock { label, at, len, block_len } => write!(
                f,
                "mali arena '{label}': a write of {len} bytes at {at} passes the end of its {block_len}-byte block"
            ),
        }
    }
}

/// A block of an arena: where it is in the memory and on the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub offset: usize,
    pub gpu: u64,
    pub len: usize,
}

/// A bump arena (see the module documentation).
pub struct Arena<'m> {
    label: &'static str,
    base: u64,
    mem: &'m mut [u8],
    top: usize,
    high_water: usize,
}

impl<'m> Arena<'m> {
    /// An arena over `mem`, whose first byte the GPU sees at `base`.
    pub fn new(label: &'static str, base: u64, mem: &'m mut [u8]) -> Result<Self, ArenaError> {
        let len = mem.len();
        if (len as u64).checked_add(base).is_none() {
            return Err(ArenaError::BadBase { label, base, len });
        }
        Ok(Arena { label, base, mem, top: 0, high_water: 0 })
    }

    /// A zero-filled block of `len` bytes whose GPU address is a multiple of
    /// `align`.
    pub fn alloc(&mut self, len: usize, align: u64) -> Result<Block, ArenaError> {
        let label = self.label;
        if !align.is_power_of_two() {
            return Err(ArenaError::BadAlign { label, align });
        }
        let free = self.mem.len() - self.top;
        let full = ArenaError::Full { label, need: len, align, free };
        let at = self.base + self.top as u64;
        let pad = (at.wrapping_neg() & (align - 1)) as usize;
        let start = self.top.checked_add(pad).ok_or(full)?;
        let end = start.checked_add(len).ok_or(full)?;
        if end > self.mem.len() {
            return Err(full);
        }
        self.mem[start..end].fill(0);
        self.top = end;
        self.high_water = self.high_water.max(end);
        Ok(Block { offset: start, gpu: self.base + start as u64, len })
    }

    /// A block holding `bytes`.
    pub fn upload(&mut self, bytes: &[u8], align: u64) -> Result<Block, ArenaError> {
        let b = self.alloc(bytes.len(), align)?;
        self.write(b, 0, bytes)?;
        Ok(b)
    }

    /// Refuse a block that is not an allocated range of this arena: its GPU
    /// address is this arena's base plus its offset, and the range lies
    /// below the top.
    fn own(&self, block: Block) -> Result<(), ArenaError> {
        let foreign =
            ArenaError::ForeignBlock { label: self.label, offset: block.offset, len: block.len, gpu: block.gpu };
        let end = block.offset.checked_add(block.len).ok_or(foreign)?;
        if block.gpu != self.base.wrapping_add(block.offset as u64) || end > self.top {
            return Err(foreign);
        }
        Ok(())
    }

    /// Write `bytes` at `at` within `block`.
    pub fn write(&mut self, block: Block, at: usize, bytes: &[u8]) -> Result<(), ArenaError> {
        self.own(block)?;
        let label = self.label;
        let oob = ArenaError::OutOfBlock { label, at, len: bytes.len(), block_len: block.len };
        let end = at.checked_add(bytes.len()).ok_or(oob)?;
        if end > block.len {
            return Err(oob);
        }
        let start = block.offset + at;
        self.mem[start..start + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// The bytes of a block, as written.
    pub fn bytes(&self, block: Block) -> Result<&[u8], ArenaError> {
        self.own(block)?;
        Ok(&self.mem[block.offset..block.offset + block.len])
    }

    /// Everything allocated so far, from the first byte.
    pub fn used(&self) -> &[u8] {
        &self.mem[..self.top]
    }

    /// The GPU address of the first byte.
    pub fn base(&self) -> u64 {
        self.base
    }

    /// The most bytes this arena has held.
    pub fn high_water(&self) -> usize {
        self.high_water
    }

    /// The arena's name and size, for its owner's high-water line.
    pub fn label(&self) -> &'static str {
        self.label
    }

    pub fn capacity(&self) -> usize {
        self.mem.len()
    }

    /// Give every block back (see the module documentation for when).
    pub fn reset(&mut self) {
        self.top = 0;
    }

    /// The first free byte: what [`Arena::rewind`] and [`Arena::resume`]
    /// take.
    pub fn top(&self) -> usize {
        self.top
    }

    /// Give back every block allocated after `top` (an earlier
    /// [`Arena::top`]): a draw that did not fit is undone. A later top is
    /// ignored; nothing is ever given back twice.
    pub fn rewind(&mut self, top: usize) {
        self.top = self.top.min(top);
    }

    /// The arena over `mem` again, with the blocks below `top` kept: a frame
    /// left open between two calls goes on where it stopped. A `top` past
    /// the memory is the caller's defect and refused.
    pub fn resume(label: &'static str, base: u64, mem: &'m mut [u8], top: usize) -> Result<Self, ArenaError> {
        let mut a = Arena::new(label, base, mem)?;
        if top > a.mem.len() {
            return Err(ArenaError::Full { label, need: top, align: 1, free: a.mem.len() });
        }
        (a.top, a.high_water) = (top, top);
        Ok(a)
    }
}

/// The most chunks a [`Pool`] chains: its first and the ones grown after it.
pub const POOL_CHUNKS: usize = 9;

/// Where a [`Pool`] stands: the chunk it bumps and every chunk's top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolTop {
    pub chunk: usize,
    pub tops: [usize; POOL_CHUNKS],
}

impl PoolTop {
    /// An empty pool's.
    pub const ZERO: PoolTop = PoolTop { chunk: 0, tops: [0; POOL_CHUNKS] };

    /// The bytes held, in all chunks.
    pub fn used(&self) -> usize {
        self.tops.iter().sum()
    }
}

/// A chain of arenas, as Mesa's batch pool is a chain of BOs (Mesa 26.2.1
/// `panfrost_pool_alloc_aligned`, src/gallium/drivers/panfrost/
/// pan_mempool.c:162-164: a block that does not fit the current BO goes to
/// a new one of `ALIGN_POT(MAX2(slab_size, sz), 4096)` bytes; the stream
/// uploader that holds user vertex arrays grows the same way, u_upload_mgr.c
/// :181). A block that does not
/// fit the chunk being bumped goes to the first later chunk it fits in, and
/// the chunks it passes are left empty; one that fits no chunk is refused
/// ([`ArenaError::Full`], its `need` and `align` the block's), and the owner
/// may add a chunk and try again. Blocks never move, so every GPU address
/// handed out stays what it was: a frame that fits the first chunk is the
/// frame a single [`Arena`] over it builds, byte for byte.
pub struct Pool<'m> {
    label: &'static str,
    chunks: [Option<Arena<'m>>; POOL_CHUNKS],
    cur: usize,
}

impl<'m> Pool<'m> {
    /// A pool of one chunk, `mem` at `base`.
    pub fn new(label: &'static str, base: u64, mem: &'m mut [u8]) -> Result<Self, ArenaError> {
        let mut chunks = [const { None }; POOL_CHUNKS];
        chunks[0] = Some(Arena::new(label, base, mem)?);
        Ok(Pool { label, chunks, cur: 0 })
    }

    /// The pool over `chunks` (each its GPU address and memory, None for a
    /// chunk not there; the first must be), standing at `at`. A top past
    /// its chunk, a missing chunk with a top, or a bumped chunk that is not
    /// there is the caller's defect and refused.
    pub fn resume(
        label: &'static str,
        chunks: [Option<(u64, &'m mut [u8])>; POOL_CHUNKS],
        at: PoolTop,
    ) -> Result<Self, ArenaError> {
        let mut out = [const { None }; POOL_CHUNKS];
        for (k, (c, o)) in chunks.into_iter().zip(out.iter_mut()).enumerate() {
            match c {
                Some((base, mem)) => *o = Some(Arena::resume(label, base, mem, at.tops[k])?),
                None if at.tops[k] == 0 && k != 0 && k != at.chunk => {}
                None => return Err(ArenaError::Full { label, need: at.tops[k], align: 1, free: 0 }),
            }
        }
        Ok(Pool { label, chunks: out, cur: at.chunk })
    }

    /// A zero-filled block of `len` bytes whose GPU address is a multiple of
    /// `align` (see the type's documentation for where).
    pub fn alloc(&mut self, len: usize, align: u64) -> Result<Block, ArenaError> {
        let label = self.label;
        let mut free = 0;
        for k in self.cur..POOL_CHUNKS {
            let Some(c) = self.chunks[k].as_mut() else { continue };
            match c.alloc(len, align) {
                Ok(b) => {
                    self.cur = k;
                    return Ok(b);
                }
                Err(ArenaError::Full { free: f, .. }) if k == self.cur => free = f,
                Err(ArenaError::Full { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Err(ArenaError::Full { label, need: len, align, free })
    }

    /// A block holding `bytes`.
    pub fn upload(&mut self, bytes: &[u8], align: u64) -> Result<Block, ArenaError> {
        let b = self.alloc(bytes.len(), align)?;
        self.write(b, 0, bytes)?;
        Ok(b)
    }

    /// The chunk whose memory holds `block`'s GPU address.
    fn chunk_of(&mut self, block: Block) -> Result<&mut Arena<'m>, ArenaError> {
        let foreign =
            ArenaError::ForeignBlock { label: self.label, offset: block.offset, len: block.len, gpu: block.gpu };
        self.chunks.iter_mut().flatten().find(|c| block.gpu.wrapping_sub(c.base()) < c.capacity() as u64).ok_or(foreign)
    }

    /// Write `bytes` at `at` within `block`.
    pub fn write(&mut self, block: Block, at: usize, bytes: &[u8]) -> Result<(), ArenaError> {
        self.chunk_of(block)?.write(block, at, bytes)
    }

    /// The bytes of a block, as written.
    pub fn bytes(&mut self, block: Block) -> Result<&[u8], ArenaError> {
        self.chunk_of(block)?.bytes(block)
    }

    /// Where the pool stands: what [`Pool::rewind`] and [`Pool::resume`]
    /// take.
    pub fn top(&self) -> PoolTop {
        let mut tops = [0; POOL_CHUNKS];
        for (t, c) in tops.iter_mut().zip(&self.chunks) {
            *t = c.as_ref().map_or(0, Arena::top);
        }
        PoolTop { chunk: self.cur, tops }
    }

    /// Give back every block allocated after `at` (an earlier
    /// [`Pool::top`]); as [`Arena::rewind`], nothing is given back twice.
    pub fn rewind(&mut self, at: PoolTop) {
        for (c, &t) in self.chunks.iter_mut().zip(&at.tops) {
            if let Some(c) = c {
                c.rewind(t);
            }
        }
        self.cur = self.cur.min(at.chunk);
    }

    /// The chunks there are.
    pub fn chunks(&self) -> usize {
        self.chunks.iter().flatten().count()
    }

    /// The bytes of every chunk.
    pub fn capacity(&self) -> usize {
        self.chunks.iter().flatten().map(Arena::capacity).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_align_on_the_gpu_address_and_start_zeroed() {
        let mut mem = [0xa5u8; 256];
        /* a base 16 past a 64-byte boundary: offset 48 is GPU-aligned */
        let mut a = Arena::new("t", 0x1010, &mut mem).expect("an arena");
        let x = a.alloc(8, 8).expect("fits");
        assert_eq!((x.offset, x.gpu), (0, 0x1010));
        let y = a.alloc(4, 64).expect("fits");
        assert_eq!((y.offset, y.gpu), (48, 0x1040));
        assert_eq!(a.bytes(y).expect("own"), [0, 0, 0, 0]);
        assert_eq!(a.high_water(), 52);
        a.write(y, 1, &[7, 8, 9]).expect("inside");
        assert_eq!(a.bytes(y).expect("own"), [0, 7, 8, 9]);
    }

    #[test]
    fn each_refusal_has_its_own_message() {
        let mut mem = [0u8; 64];
        let mut a = Arena::new("t", 0x1000, &mut mem).expect("an arena");
        let b = a.alloc(60, 4).expect("fits");
        let full = a.alloc(8, 4).unwrap_err();
        assert_eq!(full, ArenaError::Full { label: "t", need: 8, align: 4, free: 4 });
        let align = a.alloc(1, 3).unwrap_err();
        let oob = a.write(b, 58, &[0; 4]).unwrap_err();
        let mut tiny = [0u8; 2];
        let base = Arena::new("t", u64::MAX, &mut tiny).err().expect("refused");
        let msgs = [full, align, oob, base].map(|e| e.to_string());
        for (i, m) in msgs.iter().enumerate() {
            for n in &msgs[i + 1..] {
                assert_ne!(m, n);
            }
        }
        assert!(msgs[0].contains("is full") && msgs[2].contains("passes the end"));
        /* the refused blocks took nothing; a reset gives everything back */
        assert_eq!(a.high_water(), 60);
        a.reset();
        assert_eq!(a.alloc(64, 1).map(|b| b.offset), Ok(0));
    }

    /// A block of another arena, a forged one and one from before a reset
    /// are refused with their own line, not an index panic.
    #[test]
    fn a_block_this_arena_did_not_hand_out_is_refused() {
        let (mut mem_a, mut mem_b) = ([0u8; 64], [0u8; 64]);
        let mut a = Arena::new("a", 0x1000, &mut mem_a).expect("an arena");
        let mut b = Arena::new("b", 0x2000, &mut mem_b).expect("an arena");
        let from_b = b.alloc(16, 1).expect("fits");
        let own = a.alloc(16, 1).expect("fits");
        /* the same offset and length, another arena's address */
        let foreign = ArenaError::ForeignBlock { label: "a", offset: from_b.offset, len: 16, gpu: from_b.gpu };
        assert_eq!(a.write(from_b, 0, &[1]), Err(foreign));
        assert_eq!(a.bytes(from_b), Err(foreign));
        /* the right address, past the top (a forged extent) */
        let forged = Block { len: 200, ..own };
        assert!(matches!(a.write(forged, 0, &[1]), Err(ArenaError::ForeignBlock { len: 200, .. })));
        assert!(matches!(a.bytes(forged), Err(ArenaError::ForeignBlock { .. })));
        /* an extent that overflows */
        let wrap = Block { offset: usize::MAX, gpu: 0x1000u64.wrapping_add(usize::MAX as u64), len: 2 };
        assert!(matches!(a.bytes(wrap), Err(ArenaError::ForeignBlock { .. })));
        /* a block from before a reset is no longer held */
        a.reset();
        assert!(matches!(a.bytes(own), Err(ArenaError::ForeignBlock { .. })));
        /* its own block still works, and the message is its own */
        let again = a.alloc(16, 1).expect("fits");
        assert_eq!(a.bytes(again).map(<[u8]>::len), Ok(16));
        assert!(foreign.to_string().contains("is not one this arena holds"));
    }

    /// A pool of one chunk hands out the blocks an arena over the same
    /// memory does: a frame that fits is the frame it was.
    #[test]
    fn a_pool_of_one_chunk_is_its_arena() {
        let (mut ma, mut mp) = ([0x5au8; 256], [0x5au8; 256]);
        let mut a = Arena::new("d", 0x1010, &mut ma).expect("an arena");
        let mut p = Pool::new("d", 0x1010, &mut mp).expect("a pool");
        for (len, align) in [(8, 8), (4, 64), (20, 16), (1, 1), (100, 16)] {
            let (x, y) = (a.alloc(len, align), p.alloc(len, align));
            assert_eq!(x, y);
            if let Ok(b) = y {
                p.write(b, 0, &[len as u8]).expect("inside");
                a.write(b, 0, &[len as u8]).expect("inside");
            }
        }
        assert_eq!(a.alloc(200, 1), p.alloc(200, 1), "the same refusal");
        assert_eq!(p.top().tops[0], a.top());
        assert_eq!(p.top().used(), a.top());
        assert_eq!(ma, mp);
    }

    /// A block that does not fit the chunk being bumped goes to the first
    /// later chunk it fits in, passing one too small; one that fits no
    /// chunk is refused with its own size and alignment; blocks never move
    /// and are read and written in their own chunk; a rewind undoes the
    /// blocks of every chunk after the top it names, and a resume goes on
    /// where the pool stood.
    #[test]
    fn a_pool_goes_on_in_its_next_chunk_that_holds_the_block() {
        let (mut m0, mut m1, mut m2) = ([0u8; 64], [0u8; 16], [0u8; 128]);
        let (b0, b1, b2) = (0x1000u64, 0x2000u64, 0x3000u64);
        fn none<'m>() -> [Option<(u64, &'m mut [u8])>; POOL_CHUNKS] {
            [const { None }; POOL_CHUNKS]
        }
        fn mk<'m>(m: [(u64, &'m mut [u8]); 3], at: PoolTop) -> Pool<'m> {
            let mut c = none();
            for (k, m) in [0, 1, 3].into_iter().zip(m) {
                c[k] = Some(m);
            }
            Pool::resume("d", c, at).expect("a pool")
        }
        let mut p = mk([(b0, &mut m0), (b1, &mut m1), (b2, &mut m2)], PoolTop::ZERO);
        assert_eq!((p.chunks(), p.capacity()), (3, 64 + 16 + 128));
        let x = p.alloc(48, 16).expect("fits chunk 0");
        assert_eq!(x.gpu, b0);
        let mark = p.top();
        /* 32 bytes: not in chunk 0's 16 left, nor in chunk 1's 16; chunk 3 */
        let y = p.alloc(32, 16).expect("fits chunk 3");
        assert_eq!((y.gpu, y.offset), (b2, 0));
        /* chunk 1 was passed: the next small block stays in chunk 3 */
        let z = p.upload(&[1, 2, 3], 4).expect("fits chunk 3");
        assert_eq!(z.gpu, b2 + 32);
        assert_eq!(p.bytes(z), Ok(&[1u8, 2, 3][..]));
        p.write(x, 47, &[9]).expect("its own chunk");
        assert_eq!(p.top(), PoolTop { chunk: 3, tops: [48, 0, 0, 35, 0, 0, 0, 0, 0] });
        let full = p.alloc(129, 1).unwrap_err();
        assert_eq!(full, ArenaError::Full { label: "d", need: 129, align: 1, free: 93 });
        /* a block of no chunk, and one forged past a chunk's top */
        let stray = Block { offset: 0, gpu: 0x9000, len: 1 };
        assert!(matches!(p.write(stray, 0, &[1]), Err(ArenaError::ForeignBlock { gpu: 0x9000, .. })));
        assert!(matches!(p.bytes(Block { len: 100, ..y }), Err(ArenaError::ForeignBlock { .. })));
        /* the pool as it stood, resumed */
        let at = p.top();
        assert_eq!(m0[47], 9);
        let mut p = mk([(b0, &mut m0), (b1, &mut m1), (b2, &mut m2)], at);
        assert_eq!(p.top(), at);
        assert_eq!(p.alloc(4, 4).map(|b| b.gpu), Ok(b2 + 36));
        /* undo everything after the mark: chunk 3 is empty again, and chunk
         * 0 bumped */
        p.rewind(mark);
        assert_eq!(p.top(), mark);
        assert_eq!(p.alloc(16, 16).map(|b| b.gpu), Ok(b0 + 48));
        /* a later top is ignored */
        p.rewind(at);
        assert_eq!(p.top().chunk, 0);
        /* a pool without its first chunk, or whose bumped chunk is not there */
        assert!(Pool::resume("d", none(), PoolTop::ZERO).is_err());
        let mut c = none();
        c[0] = Some((b0, &mut m0[..]));
        assert!(Pool::resume("d", c, PoolTop { chunk: 2, ..PoolTop::ZERO }).is_err());
    }
}
