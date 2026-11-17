// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Which of a stage's flat uniform dwords are default-block rows and which
//! are uniform-block member rows (docs/notes/pvr-ubo-memory-load.md §2, §5,
//! §8.2, §8.4).
//!
//! The GL core fills one flat array per stage (`program.rs`
//! `fill_uniform_blocks` / `copy_block_member`): a default uniform at its
//! row's `dword_off`, a block member at its own row's `dword_off`, each row
//! `dwords` long. The selector reads default dwords from shared registers and
//! member dwords with `ld` from the block's address plus `4 * dword_off`, so
//! it asks this table which one a dword is, as the GC7000UL selector does by
//! block range (`select::select_blocks`). Each dword is mapped separately.
//!
//! The table is a sorted run of member ranges, one per block member, in the
//! compile's arena; a dword in none is a default one.
//!
//! Unit-test feature set: `host-test,gpu-powervr,shaderc-online` (not `report-es2`)
//! (`cargo test --lib --features host-test,gpu-powervr,shaderc-online ubo_rows`).

use crate::shaderc::online::arena::{Arena, ArenaError};
use crate::shaderc::online::reflect::Reflection;

/// What one flat dword is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// A default-block uniform: a shared register.
    Default,
    /// A block member's row: `ld` from the block's address.
    Member {
        /// Index into `Reflection::blocks` (one per element of an array of blocks).
        block: u16,
        /// Index into `Reflection::block_members`.
        member: u16,
        /// The member's `Reflection::uniforms` row.
        row: u16,
    },
}

/// A read no single `ld` or register read serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Lanes in default dwords and in a block's.
    DefaultAndBlock,
    /// Lanes in two members' rows of one block.
    LeavesRow,
    /// Lanes in two blocks.
    CrossesBlocks,
}

/// One member's dwords `start..end` of the flat array.
#[derive(Clone, Copy)]
struct Range {
    start: u16,
    end: u16,
    block: u16,
    member: u16,
    row: u16,
}

pub struct UboRows<'a> {
    ranges: &'a [Range],
}

impl<'a> UboRows<'a> {
    /// The table of `r`'s block members.
    pub fn build(a: &Arena<'a>, r: &Reflection<'_>) -> Result<UboRows<'a>, ArenaError> {
        let ranges = a.alloc_filled(r.block_members.len(), Range { start: 0, end: 0, block: 0, member: 0, row: 0 })?;
        let mut n = 0;
        for (i, bm) in r.block_members.iter().enumerate() {
            let Some(u) = r.uniforms.get(usize::from(bm.row)) else { continue };
            let i = i as u16;
            let block = r.blocks.iter().position(|b| b.first_member <= i && i - b.first_member < b.num_members);
            let at = Range {
                start: u.dword_off,
                end: u.dword_off.saturating_add(u.dwords),
                block: block.unwrap_or(0) as u16,
                member: i,
                row: bm.row,
            };
            /* insertion by start: block members are mostly in order already */
            let mut k = n;
            while k > 0 && ranges[k - 1].start > at.start {
                ranges[k] = ranges[k - 1];
                k -= 1;
            }
            ranges[k] = at;
            n += 1;
        }
        Ok(UboRows { ranges: &ranges[..n] })
    }

    /// What flat dword `d` is.
    pub fn row(&self, d: u32) -> Row {
        let i = self.ranges.partition_point(|x| u32::from(x.start) <= d);
        match i.checked_sub(1).map(|i| self.ranges[i]) {
            Some(x) if d < u32::from(x.end) => Row::Member { block: x.block, member: x.member, row: x.row },
            _ => Row::Default,
        }
    }

    /// A read of `lanes` dwords from `d`, each mapped on its own: `Default`
    /// if all are, `Member` if all are in one member's row, else a refusal.
    pub fn read(&self, d: u32, lanes: u32) -> Result<Row, Refusal> {
        let first = self.row(d);
        for l in 1..lanes {
            match (first, self.row(d + l)) {
                (Row::Default, Row::Default) => {}
                (Row::Member { member: a, .. }, Row::Member { member: b, .. }) if a == b => {}
                (Row::Member { block: a, .. }, Row::Member { block: b, .. }) if a == b => {
                    return Err(Refusal::LeavesRow)
                }
                (Row::Member { .. }, Row::Member { .. }) => return Err(Refusal::CrossesBlocks),
                _ => return Err(Refusal::DefaultAndBlock),
            }
        }
        Ok(first)
    }
}

#[cfg(all(test, not(feature = "report-es2")))] /* the shaders are ES 3.0 */
mod tests {
    use super::*;
    use crate::shaderc::online::store;

    /// `src`'s fragment shader lowered as the PowerVR lowers it, and its table.
    fn with_rows(src: &str, f: impl FnOnce(&Reflection<'_>, &UboRows<'_>)) {
        use crate::shaderc::online::{lower, sema::Stage};
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir_ordered(a, Stage::Fragment, text, super::super::MAT_TIMES_VEC_ORDER)
                .map_err(|e| e.msg)
                .unwrap();
            let t = UboRows::build(a, &l.reflection).unwrap();
            f(&l.reflection, &t);
        })
        .unwrap();
    }

    /// Whether a row is called `name`, or `<instance>.name` for a block member.
    fn named(row: &str, name: &str) -> bool {
        row == name || row.rsplit_once('.').is_some_and(|(_, m)| m == name)
    }

    /// The flat dword of the row named `name`.
    fn off(r: &Reflection<'_>, name: &str) -> u32 {
        let u = r.uniforms.iter().find(|u| named(u.name, name));
        u32::from(u.unwrap_or_else(|| panic!("no row {name}")).dword_off)
    }

    fn member(r: &Reflection<'_>, name: &str) -> u16 {
        r.block_members.iter().position(|m| named(m.row_name, name)).unwrap_or_else(|| panic!("no member {name}"))
            as u16
    }

    const SMALL: &str = "#version 300 es
precision highp float;
uniform vec4 d;
uniform float e;
layout(std140) uniform B { vec4 a; float b; mat2 m; } blk;
out vec4 o;
void main() { o = d * e + blk.a * blk.b + vec4(blk.m[1], 0.0, 0.0); }
";

    #[test]
    fn default_and_member_dwords_are_told_apart() {
        with_rows(SMALL, |r, t| {
            let d = off(r, "d");
            assert_eq!(t.row(d), Row::Default);
            assert_eq!(t.row(d + 3), Row::Default);
            assert_eq!(t.row(off(r, "e")), Row::Default);
            assert_eq!(t.read(d, 4), Ok(Row::Default));
            let a = off(r, "a");
            match t.row(a) {
                Row::Member { block, member: m, row } => {
                    assert_eq!((block, m), (0, member(r, "a")));
                    assert_eq!(u32::from(r.uniforms[usize::from(row)].dword_off), a);
                }
                x => panic!("{x:?}"),
            }
            assert!(matches!(t.read(a, 4), Ok(Row::Member { .. })));
            assert!(matches!(t.row(off(r, "b")), Row::Member { block: 0, .. }));
            assert!(matches!(t.row(off(r, "m")), Row::Member { block: 0, .. }));
            /* past every row is default */
            assert_eq!(t.row(60000), Row::Default);
        });
    }

    #[test]
    fn a_vec4_straddling_two_members_is_refused() {
        with_rows(SMALL, |r, t| {
            let (a, b) = (off(r, "a"), off(r, "b"));
            assert_eq!(b, a + 4, "b follows a's four dwords");
            assert_eq!(t.read(b - 2, 4), Err(Refusal::LeavesRow));
            assert_eq!(t.read(a, 4), Ok(t.row(a)));
        });
    }

    #[test]
    fn two_blocks_and_default_into_block_are_refused() {
        let src = "#version 300 es
precision highp float;
uniform vec4 d;
layout(std140) uniform A { vec4 a; } ba;
layout(std140) uniform B { vec4 b; } bb;
out vec4 o;
void main() { o = d + ba.a + bb.b; }
";
        with_rows(src, |r, t| {
            let (d, a, b) = (off(r, "d"), off(r, "a"), off(r, "b"));
            assert!(matches!(t.row(a), Row::Member { block: 0, .. }));
            assert!(matches!(t.row(b), Row::Member { block: 1, .. }));
            assert_eq!(b, a + 4);
            assert_eq!(t.read(a + 2, 4), Err(Refusal::CrossesBlocks));
            if d + 4 == a {
                assert_eq!(t.read(d + 2, 4), Err(Refusal::DefaultAndBlock));
            }
        });
    }

    /// 16384 bytes as 4 * `vec4[255]` + `vec4[4]` (1024 vec4s; a single
    /// `vec4[1024]` is refused by the u8 array length, reflect.rs `fit_u8`).
    /// The online front end takes the 4096-dword stage array: the table is
    /// built from the lowered module's own reflection.
    #[test]
    fn a_16384_byte_block_maps_its_first_and_last_dword() {
        let src = "#version 300 es
precision highp float;
layout(std140) uniform Big { vec4 a0[255]; vec4 a1[255]; vec4 a2[255]; vec4 a3[255]; vec4 tail[4]; } b;
uniform int i;
out vec4 o;
void main() { o = b.a0[0] + b.tail[3] + b.a2[i]; }
";
        with_rows(src, |r, t| {
            assert_eq!(r.blocks.len(), 1);
            assert_eq!(r.blocks[0].data_size, 16384);
            let first = off(r, "a0");
            let last = off(r, "tail") + 4 * 3 + 3;
            assert_eq!(last - first, 4095);
            assert!(matches!(t.row(first), Row::Member { block: 0, .. }));
            match t.row(last) {
                Row::Member { block: 0, member: m, .. } => assert_eq!(m, member(r, "tail")),
                x => panic!("{x:?}"),
            }
            assert_eq!(t.row(last + 1), Row::Default);
            assert!(matches!(t.read(last - 3, 4), Ok(Row::Member { .. })));
            /* a run-time index into a2 stays inside a2's row */
            let a2 = off(r, "a2");
            assert!(matches!(t.read(a2 + 4 * 254, 4), Ok(Row::Member { .. })));
            assert_eq!(t.read(a2 + 4 * 254 + 2, 4), Err(Refusal::LeavesRow));
            assert_eq!(t.row(off(r, "i")), Row::Default);
        });
    }

    #[test]
    fn matrix_rows_follow_the_flat_array_packing() {
        let src = "#version 300 es
precision highp float;
layout(std140, row_major) uniform R { mat4 r; mat2x3 s; } b;
layout(std140) uniform C { mat4 c; } k;
out vec4 o;
void main() { o = b.r[1] + vec4(b.s[0], 0.0) + k.c[3]; }
";
        with_rows(src, |r, t| {
            /* a mat4 is four 4-dword columns (program.rs copy_block_member) */
            let (mr, mc) = (off(r, "r"), off(r, "c"));
            for c in 0..4 {
                assert!(matches!(t.read(mr + 4 * c, 4), Ok(Row::Member { block: 0, .. })));
                assert!(matches!(t.read(mc + 4 * c, 4), Ok(Row::Member { block: 1, .. })));
            }
            let rm = &r.block_members[usize::from(member(r, "r"))];
            assert!(rm.row_major);
            assert_eq!(r.uniforms[usize::from(rm.row)].dwords, 16);
            /* mat2x3: two columns of three rows, a column strided to 4 dwords */
            let s = off(r, "s");
            let sm = &r.block_members[usize::from(member(r, "s"))];
            assert!(sm.row_major);
            assert_eq!(r.uniforms[usize::from(sm.row)].dwords, 8);
            assert!(matches!(t.read(s, 8), Ok(Row::Member { .. })));
            assert_eq!(t.read(mr + 12, 8), Err(Refusal::LeavesRow));
        });
    }
}
