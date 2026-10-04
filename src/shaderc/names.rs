// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The name pool: how a reflection entry names itself, on both GPUs.
//!
//! Every reflection name -- vertex attribute, varying, uniform, GC7000 io --
//! lives in one NUL-terminated byte pool per shader table, and the entry holds
//! a [`ShcNameRef`], the byte offset of its first character. This replaces the
//! fixed `[c_char; 32]` / `[c_char; 64]` fields those entries carried.
//!
//! Two reasons, in order of how much they cost:
//!
//! 1. **A fixed field cuts names, and a cut name is silently wrong.** The
//!    stages are PAIRED by varying name, `glGetUniformLocation` matches on the
//!    name, and dEQP compares what `glGetActiveAttrib` returns byte for byte.
//!    Neither writer of those fields could even report the cut: both
//!    `util::set_name` and `util::cname` reserve the last byte for the NUL, so
//!    a 40-character name and a 31-character one were stored identically.
//!    `shaderc::link` refuses a link at the limit today (it can do no better
//!    from the table alone); a pool has no limit to be at.
//! 2. **The fields are most of the table.** Counted by
//!    `the_shipped_table_is_mostly_repeated_names` below: the PowerVR table's
//!    35,852 name fields are 2,144,288 bytes and hold 2,462 distinct names --
//!    143,408 bytes of offsets plus a 57,572-byte pool, 10.7x smaller and
//!    1.94 MB off the image. The GC7000 table goes 195,328 -> 37,262 bytes,
//!    5.2x. Qt Quick 3D is why the fields repeat: every permutation of a
//!    shader re-states `ubDirectionalLights.directionalLights[0].
//!    dimensionsInverted` in full, and there are 2,042 permutations.
//!
//! Offset 0 of every pool is a NUL, so a `ShcNameRef` of 0 reads as the empty
//! string and means "unnamed" -- the same thing an all-NUL field meant, with
//! no sentinel value to get wrong.
//!
//! The offline table has one pool for all of its blobs (that is where the
//! deduplication pays); the remote back end builds a small pool per shader
//! with [`NamePoolBuilder`] as the compile reply is decoded. `ShcShader`
//! gains `names: *const u8` and `names_len: u32` pointing at whichever, and
//! the four entry types trade their name field for a `ShcNameRef`.
//!
//! This module is the representation and nothing else yet: flipping the
//! entries over to it rewrites all 2042 `ShcShader` literals of the generated
//! table, so it lands with the generator change and #39's rebuild, in one
//! commit, on both GPUs at once.
//!
//! # The format, for the three ends that have to agree
//!
//! **The table** (`glsl2usc.py`, `glsl2vivante.py`). One pool per generated
//! table, emitted once:
//!
//! ```text
//! pub static SHC_NAME_POOL: &[u8] = b"\0a_pos\0v_uv\0uTex\0...";
//! ```
//!
//! A leading NUL, then every distinct name NUL-terminated, in a deterministic
//! order (by first use or sorted -- pick one, so a rebuild diffs cleanly).
//! Exact deduplication only: a name that is a suffix of another still gets its
//! own entry. Each entry's `name: cname("v_uv")` becomes `name: <offset>`, and
//! each `ShcShader` literal gains `names: SHC_NAME_POOL.as_ptr(),
//! names_len: SHC_NAME_POOL.len() as u32`. The generator is the one place that
//! still holds the original GLSL string, so it is the one place that can
//! refuse a name it had to shorten -- and with a pool there is nothing to
//! shorten it to, so this only ever fires on something pathological. Make the
//! message name the shader and the name.
//!
//! **The wire** (`tools/shaderc_server.py`, `remote.rs`). Unchanged: the
//! compile reply already carries `"name": "v_uv"` as a JSON string with no
//! length limit, and the server never sees a fixed field. Only the client's
//! decode changes -- `name_field(&mut d.name, ...)` becomes an `intern` into a
//! [`NamePoolBuilder`] over a `names: Vec<u8>` on the `Compiled` record, which
//! is leaked `&'static` like `usc` and `uniforms` already are, so the pool
//! outlives every shader that points into it. A `None` from `intern` fails the
//! compile with a message; it cannot truncate.
//!
//! **The structs** (`gles_shaderc.h`, `abi_layout.rs` and its C check, both
//! GPUs): `ShcVarying.name`, `ShcAttrib.name`, `ShcUniform.name` and
//! `ShcVivIo.name` become `ShcNameRef`; `ShcShader` gains `names: *const u8`
//! and `names_len: u32`.
//!
//! The width of `ShcNameRef` is behind this alias on purpose: it is one line
//! to change, and every end should be written against the name rather than the
//! width. It is `u32` today because the PowerVR pool measures 57,572 bytes --
//! 88% of what a `u16` addresses, on a table #39 is about to grow. A `u16`
//! would save a further 71,704 bytes, 3.6% of the 1.94 MB, and cap the pool at
//! 64 KiB; an overflow there wraps an offset onto the wrong name, which is the
//! same silent mis-wiring this task exists to end. If the cap is taken anyway,
//! the generator must refuse to emit a pool it cannot address: a build break,
//! never a runtime surprise.

/// Where a reflection entry's name starts in its shader's pool. 0 = unnamed.
pub type ShcNameRef = u32;

/// A shader table's names, as `ShcShader::names` points at them.
#[derive(Clone, Copy)]
pub struct NamePool<'a>(&'a [u8]);

impl<'a> NamePool<'a> {
    /// An empty pool: every lookup in it is the empty name.
    pub const EMPTY: NamePool<'static> = NamePool(&[0]);

    pub fn new(bytes: &'a [u8]) -> Self {
        NamePool(bytes)
    }

    /// The name at `off`, without its NUL.
    ///
    /// An offset past the end of the pool reads as the empty name rather than
    /// panicking or reading past it: a reflection table is data, it can come
    /// from a remote compile, and a GL entry point must not fault on it. An
    /// unterminated pool stops at its end for the same reason.
    pub fn get(&self, off: ShcNameRef) -> &'a [u8] {
        let Some(tail) = self.0.get(off as usize..) else {
            return &[];
        };
        let n = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
        &tail[..n]
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.0
    }
}

/// Fills a caller-owned buffer with a pool, interning as it goes.
///
/// The remote back end decodes a compile reply into one of these. It refuses
/// to grow past the buffer instead of truncating: a name that does not fit
/// fails the compile with a message, where the fixed field it replaces would
/// have cut the name and linked something wrong.
pub struct NamePoolBuilder<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> NamePoolBuilder<'a> {
    /// `buf` must hold at least the leading NUL; a smaller one interns
    /// nothing but the empty name.
    pub fn new(buf: &'a mut [u8]) -> Self {
        if let Some(first) = buf.first_mut() {
            *first = 0;
        }
        let len = buf.len().min(1);
        NamePoolBuilder { buf, len }
    }

    /// The offset of `name` in the pool, adding it if it is not there yet.
    /// `None` when the buffer is full -- the caller fails the compile.
    ///
    /// Deduplication is a linear scan of what is already interned. A shader
    /// has at most a few hundred names, and the alternative is a hash map in
    /// a `no_std` crate with no allocator.
    pub fn intern(&mut self, name: &[u8]) -> Option<ShcNameRef> {
        if name.is_empty() || name.contains(&0) {
            return Some(0);
        }
        let done = &self.buf[..self.len];
        let mut off = 0usize;
        while off < done.len() {
            let n = done[off..].iter().position(|&b| b == 0)?;
            if &done[off..off + n] == name {
                return Some(off as ShcNameRef);
            }
            off += n + 1;
        }
        let at = self.len;
        let end = at.checked_add(name.len())?.checked_add(1)?;
        if end > self.buf.len() {
            return None;
        }
        self.buf[at..at + name.len()].copy_from_slice(name);
        self.buf[at + name.len()] = 0;
        self.len = end;
        Some(at as ShcNameRef)
    }

    /// The bytes interned so far, to hand to [`NamePool::new`].
    pub fn finish(self) -> &'a [u8] {
        &self.buf[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the pool is worth on the table we actually ship, counted rather
    /// than asserted: the numbers in this module's header are printed by
    /// `cargo test -- --nocapture names::tests::the_shipped_table`. The
    /// assertion is only that pooling wins by a lot, so the test still means
    /// something when the table is rebuilt (#39).
    /// Reads the offline table, so it measures nothing without one.
    #[cfg(not(feature = "no-shader-table"))]
    #[test]
    fn the_shipped_table_is_mostly_repeated_names() {
        use crate::shaderc::table::SHC_BLOB_TABLE;
        use std::collections::HashSet;

        let (mut fixed, mut refs) = (0usize, 0usize);
        let mut distinct: HashSet<std::vec::Vec<u8>> = HashSet::new();
        let mut note = |n: &[u8], cap: usize, fixed: &mut usize, refs: &mut usize| {
            *fixed += cap;
            *refs += core::mem::size_of::<ShcNameRef>();
            if !n.is_empty() {
                distinct.insert(n.to_vec());
            }
        };
        for b in SHC_BLOB_TABLE.iter() {
            let sh = &b.shader;
            for v in &sh.varyings[..(sh.num_varyings as usize).min(sh.varyings.len())] {
                note(crate::util::name_bytes(&v.name), v.name.len(), &mut fixed, &mut refs);
            }
            for a in &sh.attribs[..(sh.num_attribs as usize).min(sh.attribs.len())] {
                note(crate::util::name_bytes(&a.name), a.name.len(), &mut fixed, &mut refs);
            }
            for u in sh.uniform_table() {
                note(crate::util::name_bytes(&u.name), u.name.len(), &mut fixed, &mut refs);
            }
        }
        let pool: usize = 1 + distinct.iter().map(|n| n.len() + 1).sum::<usize>();
        let pooled = refs + pool;
        std::println!(
            "names: {} rows, {} distinct; fixed fields {} B -> refs {} B + pool {} B = {} B ({:.1}x)",
            refs / core::mem::size_of::<ShcNameRef>(),
            distinct.len(),
            fixed,
            refs,
            pool,
            pooled,
            fixed as f64 / pooled.max(1) as f64
        );
        assert!(pooled * 2 < fixed, "pooling has to be worth the churn: {pooled} vs {fixed}");
    }

    #[test]
    fn an_empty_pool_names_nothing_and_does_not_fault() {
        let p = NamePool::EMPTY;
        assert_eq!(p.get(0), b"");
        assert_eq!(p.get(1), b"", "one past the end");
        assert_eq!(p.get(u32::MAX), b"", "a wild offset reads empty, not a panic");
    }

    #[test]
    fn a_pool_round_trips_names_of_any_length() {
        /* The point of the pool: this name is 59 characters, four short of
         * the 64-byte field it replaces, and Qt Quick 3D writes longer. */
        let long = b"ubDirectionalLights.directionalLights[0].dimensionsInverted";
        let mut buf = [0u8; 256];
        let mut b = NamePoolBuilder::new(&mut buf);
        let a = b.intern(b"a_pos").unwrap();
        let l = b.intern(long).unwrap();
        let bytes = b.finish();
        let p = NamePool::new(bytes);
        assert_eq!(p.get(a), b"a_pos");
        assert_eq!(p.get(l), long);
        assert_eq!(p.get(0), b"", "offset 0 is the leading NUL");
    }

    #[test]
    fn interning_the_same_name_twice_stores_it_once() {
        /* Where the 16.7x comes from: every permutation of a Qt shader
         * re-states the same uniform names. */
        let mut buf = [0u8; 64];
        let mut b = NamePoolBuilder::new(&mut buf);
        let first = b.intern(b"v_texcoord").unwrap();
        let again = b.intern(b"v_texcoord").unwrap();
        assert_eq!(first, again);
        assert_eq!(b.finish().len(), 1 + 10 + 1, "the leading NUL and one copy");
    }

    #[test]
    fn a_name_that_is_a_suffix_of_another_is_still_its_own_entry() {
        /* Tail merging would be a further win and a further chance to be
         * wrong; exact matches only, and the test says so on purpose. */
        let mut buf = [0u8; 64];
        let mut b = NamePoolBuilder::new(&mut buf);
        let long = b.intern(b"v_texcoord").unwrap();
        let short = b.intern(b"texcoord").unwrap();
        assert_ne!(long, short);
        let p = NamePool::new(b.finish());
        assert_eq!(p.get(long), b"v_texcoord");
        assert_eq!(p.get(short), b"texcoord");
    }

    #[test]
    fn a_full_buffer_refuses_rather_than_truncating() {
        /* The behaviour the fixed field could not have: the caller is told,
         * and fails the compile, instead of storing half a name. */
        let mut buf = [0u8; 8];
        let mut b = NamePoolBuilder::new(&mut buf);
        assert_eq!(b.intern(b"abcdef"), Some(1));
        assert_eq!(b.intern(b"ghijkl"), None, "8 bytes hold one of these, not two");
        let p = NamePool::new(b.finish());
        assert_eq!(p.get(1), b"abcdef", "and what did fit is intact");
    }

    #[test]
    fn a_buffer_too_small_for_the_leading_nul_interns_only_the_empty_name() {
        let mut buf = [0u8; 0];
        let mut b = NamePoolBuilder::new(&mut buf);
        assert_eq!(b.intern(b""), Some(0));
        assert_eq!(b.intern(b"a"), None);
        assert!(b.finish().is_empty());
        assert_eq!(NamePool::new(&[]).get(0), b"");
    }

    #[test]
    fn an_empty_name_is_offset_zero_and_costs_nothing() {
        let mut buf = [0u8; 16];
        let mut b = NamePoolBuilder::new(&mut buf);
        assert_eq!(b.intern(b""), Some(0));
        assert_eq!(b.finish().len(), 1);
    }

    #[test]
    fn an_unterminated_pool_stops_at_its_end() {
        /* A pool that arrived over the wire need not be well formed. */
        let p = NamePool::new(b"\0abc");
        assert_eq!(p.get(1), b"abc");
    }
}
