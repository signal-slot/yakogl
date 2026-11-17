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

use super::sel::{Bank, Mods, OpType, SelDst, SelGroup, SelOp, SelSrc, VReg, MAX_LD_BURST};
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

/// Whether, and from where, a stage reads its uniform-block members with
/// `ld` (docs/notes/pvr-ubo-memory-load.md §2, §3, §8.4).
///
/// **Off by default, and nothing sets it yet.** Off, every uniform dword is
/// a shared-register read as before and no [`SelOp::Ld`] or
/// [`SelOp::Add64`] is ever made, so the compiler's output is unchanged.
/// The chain turns it on (plan #378 item `ubo-chain`) once the record
/// carries `ubo_addr_off1` and the back end writes the pair.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemberLoads {
    /// Read block members from memory.
    pub on: bool,
    /// The absolute shared register of the address pair's low dword,
    /// `ubo_addr_off1 - 1` (the high dword is the next one; no alignment,
    /// §3): `base + uniform_sh_dwords` in the §8.4 layout.
    pub pair: u16,
}

/// The byte offset a member read adds to the block address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberOffset {
    /// A constant read: the member's flat dword (its row's `dword_off` plus
    /// the lane). The byte offset is `4 * dword`.
    Dword(u32),
    /// A run-time read: a 32-bit byte offset the selector computed (the
    /// clamped index times the byte stride, plus `4 * base_dw`; §2).
    Bytes(SelSrc),
}

impl MemberLoads {
    /// The groups of one member read, §2's sequence: `add64_32` of the
    /// shared pair and the byte offset into the pair `addr`, `addr + 1`;
    /// `ld` of `burst` dwords into the run `dst` .. `dst + burst - 1`; and
    /// the `wdf` after which the run holds the data. `addr` and `dst` must
    /// be fresh values, `addr + 1` and the run's later registers too.
    ///
    /// `None` when the option is off, or for a burst outside
    /// 1..=[`MAX_LD_BURST`] (a read wider than a vec4 is two reads).
    pub fn load(self, offset: MemberOffset, burst: u8, addr: VReg, dst: VReg) -> Option<[SelGroup; 3]> {
        if !self.on || !(1..=MAX_LD_BURST).contains(&burst) {
            return None;
        }
        let sh = |reg: u16| Some(SelSrc::Fixed { bank: Bank::Shared, reg, mods: Mods::default() });
        let off = match offset {
            MemberOffset::Dword(d) => SelSrc::Imm(d.checked_mul(4)?),
            MemberOffset::Bytes(s) => s,
        };
        Some([
            SelGroup {
                op: SelOp::Add64,
                ty: OpType::U32,
                dst: Some(SelDst::Virtual(addr)),
                srcs: [sh(self.pair), sh(self.pair.checked_add(1)?), Some(off)],
            },
            SelGroup {
                op: SelOp::Ld { burst },
                ty: OpType::U32,
                dst: Some(SelDst::Virtual(dst)),
                srcs: [Some(SelSrc::Virtual { v: addr, mods: Mods::default() }), None, None],
            },
            SelGroup { op: SelOp::WaitData, ty: OpType::U32, dst: None, srcs: [None; 3] },
        ])
    }
}

#[cfg(test)]
mod load_tests {
    //! The member-read groups through the passes the chain runs them
    //! through -- `prepare`, `index_high_shared`, the allocator, the
    //! `ld`/`wdf` order check, the builder and the decode-back check --
    //! with the emitted words pinned. Built directly: no selector routes a
    //! read here yet. Feature set `host-test,gpu-powervr,shaderc-online`.
    use super::*;
    use crate::shaderc::online::powervr::alloc::{allocate, Boundary, Limits};
    use crate::shaderc::online::powervr::assemble::build_group;
    use crate::shaderc::online::powervr::encode::encode_group;
    use crate::shaderc::online::powervr::prepare::{index_high_shared, prepare_counted};
    use crate::shaderc::online::powervr::select::Selected;
    use crate::shaderc::online::powervr::verify::{check_group, check_ld_waits};
    use std::string::String;
    use std::vec::Vec;

    const LIMITS: Limits = Limits { temps: 64, shared: 2048, vtx_in: 32, coeff: 32 };

    /// One built group: its listing (operands named, offsets left out) and
    /// its bytes in hex.
    type Built = (String, String);

    /// `groups` through the passes, each built group checked against its
    /// meaning on the way. `live_in` is written before the first group (a
    /// run-time offset), `live_out` read after the last (the loaded lanes).
    fn build(groups: &[SelGroup], live_in: &[VReg], live_out: &[VReg]) -> Vec<Built> {
        let mut buf = std::vec![0u8; 1 << 20];
        let a = Arena::new(&mut buf);
        let sel = Selected {
            groups: a.alloc_copy(groups).unwrap(),
            vregs: 64,
            outputs: &[],
            inputs: &[],
            discard: None,
            alpha_to_one: None,
        };
        let (sel, _) = prepare_counted(&sel, &a).expect("prepares");
        let sel = index_high_shared(&sel, &a).expect("indexes");
        check_ld_waits(sel.groups).expect("waits, before allocation");
        let al = allocate(sel.groups, LIMITS, [], Boundary { live_in, live_out }, &a).expect("allocates");
        check_ld_waits(al.groups).expect("waits, after allocation");
        let mut out = Vec::new();
        for g in al.groups {
            let b = build_group(g).unwrap_or_else(|e| panic!("{g:?}: {e:?}"));
            let mut bytes = std::vec![0u8; b.total];
            assert_eq!(encode_group(&b, &mut bytes, 0), Ok(b.total));
            if g.op != SelOp::WaitData {
                assert_eq!(check_group(g, "mbyp", &bytes), Ok(b.total), "{g:?}");
            }
            let mut text = String::new();
            super::super::listing::group_text(&mut text, 0, &b).unwrap();
            let text = text.trim().split_once(' ').map_or(String::new(), |(_, t)| t.trim().into());
            out.push((text, bytes.iter().map(|x| std::format!("{x:02x}")).collect::<Vec<_>>().join(" ")));
        }
        out
    }

    const ON: MemberLoads = MemberLoads { on: true, pair: 8 };

    fn seq(m: MemberLoads, off: MemberOffset, burst: u8) -> (Vec<SelGroup>, Vec<VReg>) {
        let g = m.load(off, burst, VReg(10), VReg(20)).expect("on");
        (g.to_vec(), (0..u32::from(burst)).map(|k| VReg(20 + k)).collect())
    }

    fn show(b: &[Built]) -> String {
        b.iter().map(|(t, h)| std::format!("{t}\n    {h}\n")).collect()
    }

    #[test]
    fn off_makes_no_groups() {
        assert_eq!(MemberLoads::default().load(MemberOffset::Dword(0), 1, VReg(0), VReg(2)), None);
        assert!(!MemberLoads::default().on);
        assert_eq!(ON.load(MemberOffset::Dword(0), 5, VReg(0), VReg(2)), None);
        assert_eq!(ON.load(MemberOffset::Dword(0), 0, VReg(0), VReg(2)), None);
        assert_eq!(ON.load(MemberOffset::Dword(u32::MAX), 1, VReg(0), VReg(2)), None, "past 2^32 bytes");
    }

    /// The words of the groups, and each group's listing holding `facts`.
    fn pinned(got: &[Built], want: &[(&[&str], &str)]) {
        assert_eq!(got.len(), want.len(), "{}", show(got));
        for ((text, hex), (facts, words)) in got.iter().zip(want) {
            assert_eq!(hex, words, "{}", show(got));
            for f in *facts {
                assert!(text.contains(f), "`{f}` not in\n{text}");
            }
        }
    }

    /* Each read starts with the high dword of the pair sh8:sh9 moved to a
     * temporary (`add64_32`'s s1 has no shared form,
     * `legalise::mem_src_banks`): to t0, or to t1 when t0 is taken. */
    const MOVE_SH9_T0: (&[&str], &str) =
        (&["mbyp", "sb0=shared s0=9", "dbN=temp dN=0"], "35 82 00 87 89 08 00 00 00 40");
    const MOVE_SH9_T1: (&[&str], &str) =
        (&["mbyp", "sb0=shared s0=9", "dbN=temp dN=1"], "35 82 00 87 89 08 00 00 00 41");
    /* `wdf` on DRC 0 (`fragment::wait_data`). */
    const WDF: (&[&str], &str) = (&["ctrlop=wdf", "miscctl=0"], "02 80 6a ff");
    /* `add64_32.s t2, t3 = sh8, t0, t1`: the high dword in t0, the byte
     * offset in t1, the sum into the pair t2:t3. */
    const ADD_SH8_T0_T1: (&[&str], &str) = (
        &["add6432", "s=1", "sb0=shared s0=8", "sb1=temp s1=0", "sb2=temp s2=1", "d0=2", "d1=3"],
        "37 8e 00 e8 88 60 80 41 08 00 c0 82 43 ff",
    );
    /* `add64_32.s t2, t3 = sh8, t1, t0`: the operands the other way round. */
    const ADD_SH8_T1_T0: (&[&str], &str) = (
        &["add6432", "s=1", "sb0=shared s0=8", "sb1=temp s1=1", "sb2=temp s2=0", "d0=2", "d1=3"],
        "37 8e 00 e8 88 61 80 40 08 00 c0 82 43 ff",
    );
    /* `ld t0, drc0, 1, t2:t3`: the backend word f1 04 00 of the note's §1
     * table (DRC 0, burst 1, s0, normal). */
    const LD1_T2_T0: (&[&str], &str) = (
        &["ld_immbl", "burstlen=1", "cachemode_ld=normal", "drc=0", "sb0=temp s0=2", "sb3=temp s3=0"],
        "65 a0 00 f1 04 00 42 40 00 ff",
    );

    /// A 16384-byte block's first dword (byte 0): `add64_32` of sh8, sh9
    /// and byte 0 -- special register 0, no move -- into t1:t2, then `ld`
    /// burst 1 from the pair into t0 (free again once the sum read it) and
    /// `wdf`.
    #[test]
    fn the_first_dword_of_a_16384_byte_block() {
        let (g, out) = seq(ON, MemberOffset::Dword(0), 1);
        pinned(
            &build(&g, &[], &out),
            &[
                MOVE_SH9_T0,
                (
                    &["add6432", "s=1", "sb0=shared s0=8", "sb1=temp s1=0", "sb2=special s2=0", "d0=1", "d1=2"],
                    "37 8e 00 e8 88 60 80 00 08 00 c0 81 42 ff",
                ),
                (&["ld_immbl", "burstlen=1", "sb0=temp s0=1", "sb3=temp s3=0"], "65 a0 00 f1 04 00 41 40 00 ff"),
                WDF,
            ],
        );
    }

    /// Its last dword (byte 16380): the offset has no constant register, so
    /// a `movi32` puts it in t1 for the `add64_32`.
    #[test]
    fn the_last_dword_of_a_16384_byte_block() {
        let (g, out) = seq(ON, MemberOffset::Dword(4095), 1);
        pinned(
            &build(&g, &[], &out),
            &[
                MOVE_SH9_T0,
                (&["bbyp0bm_imm32", "imm32=16380", "dN=1"], "86 92 40 13 fc 3f 00 00 00 00 41 ff"),
                ADD_SH8_T0_T1,
                LD1_T2_T0,
                WDF,
            ],
        );
    }

    /// The last vec4 (dwords 4092..4095, byte 16368) in one 4-dword burst:
    /// backend word f1 10 00 (§1's `ld drc0, 4, s0`). The run is t4..t7, not
    /// t0..t3: the address pair t2:t3 stays live to the `wdf`.
    #[test]
    fn a_four_dword_burst_at_the_end_of_a_16384_byte_block() {
        let (g, out) = seq(ON, MemberOffset::Dword(4092), 4);
        pinned(
            &build(&g, &[], &out),
            &[
                MOVE_SH9_T0,
                (&["bbyp0bm_imm32", "imm32=16368", "dN=1"], "86 92 40 13 f0 3f 00 00 00 00 41 ff"),
                ADD_SH8_T0_T1,
                (&["ld_immbl", "burstlen=4", "sb0=temp s0=2", "sb3=temp s3=4"], "65 a0 00 f1 10 00 42 44 00 ff"),
                WDF,
            ],
        );
    }

    /// A run-time byte offset (here a value written before the program, t0)
    /// goes into the `add64_32` as it is.
    #[test]
    fn a_run_time_offset() {
        let off = MemberOffset::Bytes(SelSrc::Virtual { v: VReg(5), mods: Mods::default() });
        let (g, out) = seq(ON, off, 1);
        pinned(&build(&g, &[VReg(5)], &out), &[MOVE_SH9_T1, ADD_SH8_T1_T0, LD1_T2_T0, WDF]);
    }

    /// The pair past sh255 (a 1536-dword default prefix puts it there):
    /// both halves are read through `idx0`, each group setting it first
    /// (`prepare::index_high_shared`, as pco's legalizer does): the move of
    /// the high dword at idx0 = 301, the `add64_32`'s low dword at idx0 =
    /// 300. A check of our rewrite, not of the hardware (§3).
    #[test]
    fn a_pair_past_sh255_is_read_through_idx0() {
        let (g, out) = seq(MemberLoads { on: true, pair: 300 }, MemberOffset::Dword(4095), 1);
        pinned(
            &build(&g, &[], &out),
            &[
                (&["bbyp0bm_imm32", "imm32=301", "dbN=idx0 dN=5"], "86 92 40 13 2d 01 00 00 00 00 85 0c"),
                (&["mbyp", "sb0=idx0 s0=3", "dbN=temp dN=0"], "35 82 00 87 83 0c 00 00 00 40"),
                (&["bbyp0bm_imm32", "imm32=16380", "dN=1"], "86 92 40 13 fc 3f 00 00 00 00 41 ff"),
                (&["bbyp0bm_imm32", "imm32=300", "dbN=idx0 dN=5"], "86 92 40 13 2c 01 00 00 00 00 85 0c"),
                (
                    &["add6432", "sb0=idx0 s0=3", "sb1=temp s1=0", "sb2=temp s2=1", "d0=2", "d1=3"],
                    "37 8e 00 e8 83 60 90 41 0c 00 00 c0 82 43",
                ),
                LD1_T2_T0,
                WDF,
            ],
        );
    }

    /// Two reads in a row: the second pair and run are placed apart from the
    /// first's lanes, which are still live, and each `ld` waits for its own
    /// `wdf`.
    #[test]
    fn two_reads_keep_their_runs_apart() {
        let mut g = ON.load(MemberOffset::Dword(0), 4, VReg(10), VReg(20)).unwrap().to_vec();
        g.extend(ON.load(MemberOffset::Dword(4092), 4, VReg(12), VReg(24)).unwrap());
        let out: Vec<VReg> = (20..28).map(VReg).collect();
        let b = build(&g, &[], &out);
        let runs: Vec<&str> = b.iter().filter(|(t, _)| t.contains("ld_immbl")).map(|(t, _)| t.as_str()).collect();
        assert_eq!(runs.len(), 2);
        let s3 = |t: &str| t.split("sb3=temp s3=").nth(1).and_then(|x| x.split_whitespace().next()).unwrap().to_owned();
        let (a, c): (u32, u32) = (s3(runs[0]).parse().unwrap(), s3(runs[1]).parse().unwrap());
        assert!(a + 4 <= c || c + 4 <= a, "the runs t{a}.. and t{c}.. overlap\n{}", show(&b));
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
