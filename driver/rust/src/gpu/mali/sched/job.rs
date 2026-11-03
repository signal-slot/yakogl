// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! The command stream instruction words panthor's `queue_run_job` wraps a
//! job in (Linux v6.18, drivers/gpu/drm/panthor/panthor_sched.c:3133-3231,
//! used under the MIT option of its `GPL-2.0 or MIT` licence): the
//! parameters of `get_job_cs_params` (:3005-3030), the sequence of
//! `prepare_job_instrs` (:3039-3120) without the profiling words, padded to
//! a 64-byte line (:94, :3116-3119), and the ring copy of
//! `copy_instrs_to_ringbuf` (:2969-2994). [`super`]'s doc lists the words.
//!
//! The module holds pure functions only: the encoders (one per instruction,
//! each a literal copy of panthor's expression), [`job_block`], the ring
//! arithmetic and the 16-byte sync object's byte layout. Nothing here
//! touches memory the caller does not pass in. The profiling variants of
//! panthor's sequence (`JOB_INSTR_CYCLES`, `JOB_INSTR_TIMESTAMP`, :3050-3060,
//! :3070-3081) are not transcribed: with profiling off panthor skips them
//! (:3107-3111).
//!
//! panthor never checks the ring's free space in `queue_run_job`: the DRM
//! scheduler holds a job back until its credits fit, the limit being the
//! ring's size in instructions (:3322, [`JOB_CREDITS`] per job,
//! `calc_job_credits` :3122-3131). We have no such scheduler, so
//! [`check_ring_fit`] refuses instead.

use super::{SchedRefusal, SyncObj64, RING_MAX_SIZE, RING_MIN_SIZE, SYNCOBJ_SIZE};

/// `NUM_INSTRS_PER_CACHE_LINE` (panthor_sched.c:94): 64 / sizeof(u64).
pub const NUM_INSTRS_PER_CACHE_LINE: usize = 64 / size_of::<u64>();

/// The instructions of the non-profiled sequence, the entries of
/// `instr_seq` with `JOB_INSTR_ALWAYS` (:3045, 3047, 3062, 3064, 3066, 3068,
/// 3083, 3085, 3087, 3089, 3092).
pub const JOB_INSTRS: usize = 11;

/// The block's words after `ALIGN(count, NUM_INSTRS_PER_CACHE_LINE)`
/// (:3116-3119): 16, 128 bytes.
pub const JOB_WORDS: usize = JOB_INSTRS.next_multiple_of(NUM_INSTRS_PER_CACHE_LINE);

/// The block's size in bytes, what `insert` advances by (:3181).
pub const JOB_BYTES: u64 = (JOB_WORDS * size_of::<u64>()) as u64;

/// The credits `calc_job_credits` charges a job (:3122-3131): its words.
pub const JOB_CREDITS: u32 = JOB_WORDS as u32;

/// `params->val_reg` = `addr_reg + 2` (:3015).
pub const fn val_reg(addr_reg: u64) -> u64 {
    addr_reg + 2
}

/// `params->addr_reg` (:3013-3014): `cs_reg_count - unpreserved_cs_reg_count`.
pub const fn addr_reg(cs_reg_count: u32, unpreserved_cs_reg_count: u32) -> u64 {
    (cs_reg_count - unpreserved_cs_reg_count) as u64
}

/// `params->waitall_mask` = `GENMASK(sb_slot_count - 1, 0)` (:3023).
pub const fn waitall_mask(sb_slot_count: u32) -> u32 {
    if sb_slot_count >= 32 {
        !0
    } else {
        (1u32 << sb_slot_count) - 1
    }
}

/// `MOV32 rX+2, imm` (:3045, :3064): `(2ull << 56) | (val_reg << 48) | imm`.
pub const fn mov32(reg: u64, imm: u32) -> u64 {
    (2u64 << 56) | (reg << 48) | imm as u64
}

/// `MOV48 rX:rX+1, imm` (:3062, :3083, :3085): `(1ull << 56) | (reg << 48) | imm`.
/// The 48-bit immediate is not masked, as in panthor; [`check_params`]
/// refuses an address above it.
pub const fn mov48(reg: u64, imm: u64) -> u64 {
    (1u64 << 56) | (reg << 48) | imm
}

/// `FLUSH_CACHE2.clean_inv_all.no_wait.signal(0) rX+2` (:3047-3048):
/// `(36ull << 56) | (0ull << 48) | (val_reg << 40) | (0 << 16) | 0x233`; the
/// zero fields are left out (clippy::identity_op), the word is the same.
pub const fn flush_cache2(val_reg: u64) -> u64 {
    (36u64 << 56) | (val_reg << 40) | 0x233
}

/// `WAIT(mask)` (:3066 with mask 1, :3087 with the scoreboard mask):
/// `(3ull << 56) | (mask << 16)`.
pub const fn wait(mask: u32) -> u64 {
    (3u64 << 56) | ((mask << 16) as u64)
}

/// `CALL rX:rX+1, rX+2` (:3068-3069):
/// `(32ull << 56) | (addr_reg << 40) | (val_reg << 32)`.
pub const fn call(addr_reg: u64, val_reg: u64) -> u64 {
    (32u64 << 56) | (addr_reg << 40) | (val_reg << 32)
}

/// `SYNC_ADD64.system_scope.propagate_err.nowait rX:rX+1, rX+2`
/// (:3089-3090): `(51ull << 56) | (0ull << 48) | (addr_reg << 40) |
/// (val_reg << 32) | (0 << 16) | 1`, the zero fields left out as above.
pub const fn sync_add64(addr_reg: u64, val_reg: u64) -> u64 {
    (51u64 << 56) | (addr_reg << 40) | (val_reg << 32) | 1
}

/// `ERROR_BARRIER` (:3092): `47ull << 56`.
pub const fn error_barrier() -> u64 {
    47u64 << 56
}

/// What `get_job_cs_params` gathers (:3005-3030), without the profiling
/// fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobParams {
    /// `addr_reg` (:3013-3014); `val_reg` is this plus 2.
    pub addr_reg: u64,
    /// `cs_start` (:3025): the user stream's GPU address.
    pub cs_start: u64,
    /// `cs_size` (:3026): the user stream's size in bytes.
    pub cs_size: u32,
    /// `sync_addr` (:3019-3020): the sync object's GPU address.
    pub sync_addr: u64,
    /// `last_flush` (:3027): the flush id for `FLUSH_CACHE2`'s register.
    pub last_flush: u32,
    /// The `waitall_mask` (:3023), see [`waitall_mask`].
    pub waitall_mask: u32,
}

/// Why [`check_params`] refuses; none is panthor's, which takes the
/// values from validated ioctl input and firmware reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamsError {
    /// An address does not fit `MOV48`'s 48 bits, so panthor's `|` would
    /// overwrite the register field (the value: its low 32 bits).
    AddressAbove48Bits(u32),
    /// `addr_reg` is odd, or it, `addr_reg + 1` or `val_reg` is not a
    /// register the 8-bit register fields hold (the value: `addr_reg`).
    RegisterRange(u32),
    /// The wait mask is zero or does not fit in the 16-bit field at
    /// bit 16 (the value: the mask).
    WaitMask(u32),
    /// A zero-size stream: panthor runs no instructions for it (:3150-3153).
    EmptyStream,
}

/// The checks the encoders' `|` assume.
pub const fn check_params(p: &JobParams) -> Result<(), ParamsError> {
    if p.cs_size == 0 {
        return Err(ParamsError::EmptyStream);
    }
    if p.cs_start >> 48 != 0 {
        return Err(ParamsError::AddressAbove48Bits(p.cs_start as u32));
    }
    if p.sync_addr >> 48 != 0 {
        return Err(ParamsError::AddressAbove48Bits(p.sync_addr as u32));
    }
    if p.addr_reg & 1 != 0 || p.addr_reg + 2 > 0xff {
        return Err(ParamsError::RegisterRange(p.addr_reg as u32));
    }
    if p.waitall_mask == 0 || p.waitall_mask > 0xffff {
        return Err(ParamsError::WaitMask(p.waitall_mask));
    }
    Ok(())
}

/// `prepare_job_instrs` without the profiling entries (:3039-3120): the
/// eleven instructions in panthor's order, then zeroes up to a cache line.
/// Pure; call [`check_params`] first.
pub const fn job_block(p: &JobParams) -> [u64; JOB_WORDS] {
    let a = p.addr_reg;
    let v = val_reg(a);
    let mut b = [0u64; JOB_WORDS];
    b[0] = mov32(v, p.last_flush); // :3045
    b[1] = flush_cache2(v); // :3047
    b[2] = mov48(a, p.cs_start); // :3062
    b[3] = mov32(v, p.cs_size); // :3064
    b[4] = wait(1); // :3066
    b[5] = call(a, v); // :3068
    b[6] = mov48(a, p.sync_addr); // :3083
    b[7] = mov48(v, 1); // :3085
    b[8] = wait(p.waitall_mask); // :3087
    b[9] = sync_add64(a, v); // :3089
    b[10] = error_barrier(); // :3092
    b // :3116-3119: words 11..16 stay zero
}

/// Bytes the stream has not consumed: `insert - extract`. Both are the
/// running 64-bit counters panthor keeps in the user input and output pages
/// (:3175, :3193-3194), not wrapped offsets.
pub const fn ring_used(insert: u64, extract: u64) -> u64 {
    insert.wrapping_sub(extract)
}

/// Bytes still free in a ring of `size` bytes.
pub const fn ring_free(size: u32, insert: u64, extract: u64) -> u64 {
    (size as u64).saturating_sub(ring_used(insert, extract))
}

/// `job->ringbuf.start & (ringbuf_size - 1)` (:2975).
pub const fn ring_offset(insert: u64, size: u32) -> u32 {
    (insert & (size as u64 - 1)) as u32
}

/// The two copies of `copy_instrs_to_ringbuf` (:2986-2993) as
/// `(start, first, second)`: the first `first` bytes go to ring offset
/// `start`, the remaining `second` bytes to offset 0.
/// `first` = `written = min(ringbuf_size - start, size)`.
pub const fn ring_split(insert: u64, size: u32, len: u32) -> (u32, u32, u32) {
    let start = ring_offset(insert, size);
    let first = if size - start < len { size - start } else { len };
    (start, first, len - first)
}

/// Whether a block of `len` bytes may be written at `insert` without
/// overwriting what the stream has not consumed. Refuses with
/// [`SchedRefusal::RingOverflow`] and the value that decided it: the free
/// bytes (low 32 bits). A ring size panthor would not create is
/// [`SchedRefusal::RingSize`].
pub const fn check_ring_fit(size: u32, insert: u64, extract: u64, len: u32) -> Result<(), (SchedRefusal, u32)> {
    if size < RING_MIN_SIZE || size > RING_MAX_SIZE || !size.is_power_of_two() {
        return Err((SchedRefusal::RingSize, size));
    }
    // An extract ahead of insert, or more used than the ring holds, is a
    // stream that ran off or a corrupt page: no room either.
    if extract > insert || ring_used(insert, extract) > size as u64 || (len as u64) > ring_free(size, insert, extract) {
        return Err((SchedRefusal::RingOverflow, ring_free(size, insert, extract) as u32));
    }
    Ok(())
}

/// `copy_instrs_to_ringbuf` (:2969-2994) on a ring held as bytes
/// (little-endian words, as the GPU reads them): the whole block at
/// `insert & (size - 1)`, wrapping at the end. Returns the new `insert`,
/// `job->ringbuf.end` (:3181).
pub fn copy_to_ring(
    ring: &mut [u8],
    insert: u64,
    extract: u64,
    block: &[u64; JOB_WORDS],
) -> Result<u64, (SchedRefusal, u32)> {
    let size = ring.len() as u32;
    check_ring_fit(size, insert, extract, JOB_BYTES as u32)?;
    let (start, first, _) = ring_split(insert, size, JOB_BYTES as u32);
    let mut bytes = [0u8; JOB_BYTES as usize];
    for (i, w) in block.iter().enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    let (a, b) = bytes.split_at(first as usize);
    ring[start as usize..(start + first) as usize].copy_from_slice(a);
    ring[..b.len()].copy_from_slice(b);
    Ok(insert + JOB_BYTES)
}

/// A `struct panthor_syncobj_64b` (panthor_sched.c:338-351) from its 16
/// bytes: `seqno` u64 at 0, `status` u32 at 8, `pad` u32 at 12, little-endian.
pub fn read_syncobj(b: &[u8; SYNCOBJ_SIZE as usize]) -> SyncObj64 {
    SyncObj64 {
        seqno: u64::from_le_bytes(b[0..8].try_into().unwrap()),
        status: u32::from_le_bytes(b[8..12].try_into().unwrap()),
        pad: u32::from_le_bytes(b[12..16].try_into().unwrap()),
    }
}

/// The sync object as bytes, the inverse of [`read_syncobj`].
pub fn write_syncobj(o: &SyncObj64) -> [u8; SYNCOBJ_SIZE as usize] {
    let mut b = [0u8; SYNCOBJ_SIZE as usize];
    b[0..8].copy_from_slice(&o.seqno.to_le_bytes());
    b[8..12].copy_from_slice(&o.status.to_le_bytes());
    b[12..16].copy_from_slice(&o.pad.to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    const X: u64 = 92; // 96 - 4 (the G610)
    const V: u64 = 94;

    fn params() -> JobParams {
        JobParams {
            addr_reg: X,
            cs_start: 0x12_3456_7000,
            cs_size: 0x180,
            sync_addr: 0x10_0000,
            last_flush: 0x1234,
            waitall_mask: waitall_mask(8),
        }
    }

    #[test]
    fn encoders_match_panthors_expressions() {
        // Each computed by hand: opcode in 63:56, then the shifted fields.
        assert_eq!(mov32(V, 0x1234), 0x025e_0000_0000_1234); // 2<<56 | 0x5e<<48 | imm
        assert_eq!(mov48(X, 0x12_3456_7000), 0x015c_0012_3456_7000); // 1<<56 | 0x5c<<48 | imm
        assert_eq!(flush_cache2(V), 0x2400_5e00_0000_0233); // 36<<56 | 0x5e<<40 | 0x233
        assert_eq!(wait(1), 0x0300_0000_0001_0000);
        assert_eq!(wait(0xff), 0x0300_0000_00ff_0000);
        assert_eq!(call(X, V), 0x2000_5c5e_0000_0000); // 32<<56 | 0x5c<<40 | 0x5e<<32
        assert_eq!(sync_add64(X, V), 0x3300_5c5e_0000_0001); // 51<<56 | ... | 1
        assert_eq!(error_barrier(), 0x2f00_0000_0000_0000);
    }

    #[test]
    fn registers_and_mask() {
        assert_eq!(addr_reg(96, 4), 92);
        assert_eq!(val_reg(92), 94);
        assert_eq!(waitall_mask(8), 0xff);
        assert_eq!(waitall_mask(1), 1);
    }

    #[test]
    fn block_is_panthors_example() {
        let b = job_block(&params());
        assert_eq!(
            b,
            [
                0x025e_0000_0000_1234, // MOV32 r94, last_flush
                0x2400_5e00_0000_0233, // FLUSH_CACHE2 r94
                0x015c_0012_3456_7000, // MOV48 r92:93, cs.start
                0x025e_0000_0000_0180, // MOV32 r94, cs.size
                0x0300_0000_0001_0000, // WAIT(0)
                0x2000_5c5e_0000_0000, // CALL r92:93, r94
                0x015c_0000_0010_0000, // MOV48 r92:93, sync
                0x015e_0000_0000_0001, // MOV48 r94, 1
                0x0300_0000_00ff_0000, // WAIT(all)
                0x3300_5c5e_0000_0001, // SYNC_ADD64
                0x2f00_0000_0000_0000, // ERROR_BARRIER
                0,
                0,
                0,
                0,
                0,
            ]
        );
    }

    #[test]
    fn block_size_and_alignment() {
        assert_eq!(JOB_INSTRS, 11);
        assert_eq!(JOB_WORDS, 16);
        assert_eq!(JOB_BYTES, 128);
        assert_eq!(JOB_BYTES % 64, 0);
        assert_eq!(size_of::<[u64; JOB_WORDS]>() as u64, JOB_BYTES);
        assert!(job_block(&params())[JOB_INSTRS..].iter().all(|&w| w == 0));
    }

    #[test]
    fn params_refusals() {
        let p = params();
        assert_eq!(check_params(&p), Ok(()));
        assert_eq!(check_params(&JobParams { cs_size: 0, ..p }), Err(ParamsError::EmptyStream));
        assert_eq!(check_params(&JobParams { cs_start: 1 << 48, ..p }), Err(ParamsError::AddressAbove48Bits(0)));
        assert_eq!(check_params(&JobParams { sync_addr: (1 << 48) | 8, ..p }), Err(ParamsError::AddressAbove48Bits(8)));
        assert_eq!(check_params(&JobParams { addr_reg: 254, ..p }), Err(ParamsError::RegisterRange(254)));
        assert_eq!(check_params(&JobParams { addr_reg: 91, ..p }), Err(ParamsError::RegisterRange(91)));
        assert_eq!(check_params(&JobParams { waitall_mask: 0, ..p }), Err(ParamsError::WaitMask(0)));
        assert_eq!(check_params(&JobParams { waitall_mask: 1 << 16, ..p }), Err(ParamsError::WaitMask(1 << 16)));
    }

    #[test]
    fn ring_space() {
        assert_eq!(ring_used(256, 128), 128);
        assert_eq!(ring_free(4096, 256, 128), 4096 - 128);
        assert_eq!(ring_free(4096, 0, 0), 4096);
        assert_eq!(ring_free(4096, 4096, 0), 0);
        assert_eq!(ring_offset(4096 + 128, 4096), 128);
        // free exactly fits
        assert_eq!(check_ring_fit(4096, 4096 - 128, 0, 128), Ok(()));
        // one byte short
        assert_eq!(check_ring_fit(4096, 4096 - 127, 0, 128), Err((SchedRefusal::RingOverflow, 127)));
        // full ring
        assert_eq!(check_ring_fit(4096, 4096, 0, 128), Err((SchedRefusal::RingOverflow, 0)));
        // consumed part is free again, counters past the ring size
        assert_eq!(check_ring_fit(4096, 8192 + 128, 8192, 128), Ok(()));
        // extract ahead of insert is refused, not wrapped
        assert_eq!(check_ring_fit(4096, 0, 128, 128), Err((SchedRefusal::RingOverflow, 0)));
        // a ring panthor would not create
        assert_eq!(check_ring_fit(2048, 0, 0, 128), Err((SchedRefusal::RingSize, 2048)));
        assert_eq!(check_ring_fit(6144, 0, 0, 128), Err((SchedRefusal::RingSize, 6144)));
    }

    #[test]
    fn ring_wrap() {
        assert_eq!(ring_split(0, 4096, 128), (0, 128, 0));
        assert_eq!(ring_split(4096 - 128, 4096, 128), (3968, 128, 0));
        assert_eq!(ring_split(4096 - 64, 4096, 128), (4032, 64, 64));
        assert_eq!(ring_split(3 * 4096 - 8, 4096, 128), (4088, 8, 120));
    }

    #[test]
    fn copy_wraps_and_advances() {
        let block = job_block(&params());
        let mut ring = [0xaau8; 4096];
        // Start 64 bytes before the end: the second half lands at 0.
        let insert = 4096 * 2 + 4096 - 64;
        let end = copy_to_ring(&mut ring, insert, insert - 64, &block).unwrap();
        assert_eq!(end, insert + 128);
        let word = |off: usize| u64::from_le_bytes(ring[off..off + 8].try_into().unwrap());
        assert_eq!(word(4096 - 64), block[0]);
        assert_eq!(word(4096 - 8), block[7]);
        assert_eq!(word(0), block[8]);
        assert_eq!(word(56), block[15]);
        assert_eq!(ring[64], 0xaa);
        assert_eq!(ring[4096 - 65], 0xaa);
    }

    #[test]
    fn copy_overflow_leaves_ring_untouched() {
        let block = job_block(&params());
        let mut ring = [0x55u8; 4096];
        assert_eq!(copy_to_ring(&mut ring, 4096 - 64, 0, &block), Err((SchedRefusal::RingOverflow, 64)));
        assert!(ring.iter().all(|&b| b == 0x55));
        assert_eq!(copy_to_ring(&mut [0u8; 1000], 0, 0, &block), Err((SchedRefusal::RingSize, 1000)));
    }

    #[test]
    fn syncobj_layout() {
        let bytes: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let o = read_syncobj(&bytes);
        assert_eq!(o.seqno, 0x0807_0605_0403_0201);
        assert_eq!(o.status, 0x0c0b_0a09);
        assert_eq!(o.pad, 0x100f_0e0d);
        assert_eq!(write_syncobj(&o), bytes);
        assert_eq!(core::mem::offset_of!(SyncObj64, status), 8);
        assert_eq!(core::mem::offset_of!(SyncObj64, pad), 12);
        // The job's SYNC_ADD64 adds 1 to the seqno word.
        let mut z = [0u8; 16];
        z[..8].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(read_syncobj(&z).seqno, 1);
    }
}
