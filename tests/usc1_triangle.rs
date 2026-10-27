// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! USC-1, the allocator's half: the triangle example's pair through selection and
//! `powervr::alloc::allocate`, so the builder has real input the day legalise
//! lands.
//!
//! `passthrough.vert` and `interp_color.frag` are what the example compiles
//! (`examples/triangle/check/src/lib.rs`, the shader texts), and both are rows in
//! the shipped PowerVR table, so the same source has a reference blob to be
//! diffed against once the builder can emit.
//!
//! Asserts almost nothing on purpose: this is a report for two other people,
//! and a number pinned here would stop either of them improving it.

#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]

use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
use yakogl::shaderc::online::powervr::alloc::{allocate, boundary_of, Allocated, Limits};
use yakogl::shaderc::online::powervr::sel::{Bank, SelDst, SelGroup, SelSrc, VReg};
use yakogl::shaderc::online::powervr::select;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

thread_local! {
    /// Counted inside the arena closure, read after it: `with_scratch` hands
    /// back only one value and this is a second.
    static DEAD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `temps` is the measured 252 -- the PDS `DOUTU` field holds `0x3f` blocks
/// of four. The other three are still stand-ins until pvrgpu-platform
/// measures them, which is why they are named here rather than defaulted
/// inside the allocator: a guessed constant in the library would be the
/// GC7000's 67-becomes-3 defect again.
///
/// It read `temps: 64` until the corpus peak was measured at 32. Half the
/// real file is not a safe stand-in for the whole of it: a survey that
/// refuses a program the part would accept reports a gap that is its own.
const PLACEHOLDER: Limits = Limits { temps: 252, shared: 256, vtx_in: 64, coeff: 64 };

/// The ES 1.00 twin on disk, because the shipped PowerVR table holds only
/// the GLSL 450 originals and an ES 2.0 front end rightly refuses those.
///
/// **This is a caveat for the diff, not a detail.** The reference blob in the
/// table was built from the 450 source; the twin is a different text with the
/// same meaning and, by construction, the same reflection. So "diffed against
/// the offline table's blob for the byte-identical source" cannot be done for
/// this pair until the twins are table rows (task #39). What can be done is a
/// diff against the blob for the SEMANTICALLY identical source, and any
/// difference has to be read with that in mind.
fn source(name: &str) -> Option<String> {
    std::fs::read_to_string(format!("tools/shaders/gl_test_es100/{name}")).ok()
}

fn report(name: &str, stage: Stage) {
    let Some(src) = source(name) else {
        println!("{name}: not in the shipped table");
        return;
    };
    /* The whole report is formatted INSIDE the closure. `Allocated` borrows
     * the arena, and the arena dies with `with_scratch`, so nothing carrying
     * a register map can be returned out of here -- which is the arena doing
     * its job rather than an inconvenience. */
    let out = store::with_scratch(|a| -> Result<String, String> {
        let text = a.alloc_str(&src).map_err(|e| format!("arena: {e:?}"))?;
        let lowered = match lower::compile_to_ir(a, stage, text) {
            Ok(l) => l,
            Err(e) => return Err(format!("front end: {e:?}")),
        };
        let sel = match select::select(&lowered.module, a) {
            Ok(s) => s,
            Err(e) => return Err(format!("selection: {e:?}")),
        };
        let groups: &[SelGroup] = sel.groups;
        let bbuf =
            a.alloc_filled(4 * (sel.inputs.len() + sel.outputs.len()), VReg(0)).map_err(|e| format!("arena: {e:?}"))?;
        let boundary = boundary_of(&sel, bbuf).map_err(|e| format!("boundary: {e:?}"))?;
        /* Nothing is pinned yet: pvrgpu-platform owes the ABI's set per
         * stage, and the allocator reserves only what it is told.
         *
         * An empty group list is a result, not a crash: a shader whose whole
         * body is writes to io registers has no ALU work to allocate. The
         * GC7000 corpus has fifteen rows of exactly that shape, and the
         * allocator answers for one rather than being skipped around. */
        let n = groups.len();
        match allocate(groups, PLACEHOLDER, [], boundary, a) {
            Err(e) => Ok(format!("{name}: {n} groups, allocator refused: {e:?}")),
            Ok(al) if al.groups.is_empty() => {
                Ok(format!("{name}: 0 groups -> {:?}\n      no groups: the program is entirely io writes", Summary(al)))
            }
            Ok(al) => Ok(format!("{name}: {n} groups -> {:?}\n      {}", Summary(al), summarise(al.groups))),
        }
    });
    match out {
        None => panic!("{name}: the scratch arena was already borrowed -- another test is running concurrently"),
        Some(Err(e)) => println!("{name}: REFUSED before the allocator -- {e}"),
        Some(Ok(s)) => println!("{s}"),
    }
}

struct Summary<'a>(Allocated<'a>);
impl core::fmt::Debug for Summary<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let a = self.0;
        write!(f, "temps {} | shared {} | vtx_in {} | coeff {}", a.temps, a.shared, a.vtx_in, a.coeff)
    }
}

/// What the builder will actually receive, in the form it receives it.
fn summarise(groups: &[SelGroup]) -> String {
    let mut virt_left = 0usize;
    let mut temp_dst = 0usize;
    let mut by_bank = [0usize; 5];
    for g in groups {
        if matches!(g.dst, Some(SelDst::Virtual(_))) {
            virt_left += 1;
        }
        if matches!(g.dst, Some(SelDst::Fixed { bank: Bank::Temp, .. })) {
            temp_dst += 1;
        }
        for s in g.srcs.iter().flatten() {
            if let SelSrc::Fixed { bank, .. } = s {
                by_bank[*bank as usize] += 1;
            }
            if matches!(s, SelSrc::Virtual { .. }) {
                virt_left += 1;
            }
        }
    }
    format!(
        "virtual operands left {virt_left} (must be 0), temp destinations {temp_dst}, \
         fixed sources by bank {by_bank:?}"
    )
}

/// Which of the table's own sources reach the allocator at all, and what
/// comes out. This is the input the builder will have on day one, so it is
/// worth knowing before the builder exists.
fn what_reaches_the_allocator_from_the_shipped_table() {
    let (mut lowered, mut selected, mut allocated, mut empty) = (0usize, 0usize, 0usize, 0usize);
    let mut refusals: std::collections::BTreeMap<String, usize> = Default::default();
    let mut biggest: Vec<(usize, u16, String)> = Vec::new();
    let (mut collisions, mut clobbered) = (0usize, Vec::<String>::new());
    let mut total_groups = 0usize;
    let mut worst: Vec<(usize, usize, String)> = Vec::new();
    for row in SHC_SOURCE_TABLE.iter() {
        let stage = if row.name.ends_with(".frag") { Stage::Fragment } else { Stage::Vertex };
        let r = store::with_scratch(|a| -> Result<(usize, u16, usize, usize), String> {
            /* The table stores each source NUL-terminated for the C entry
             * point; `compile_to_ir` takes a Rust &str and the NUL is an
             * illegal character to it. Feeding it straight in refused all
             * 240 ES 1.00 rows as "illegal character", which read as a front
             * end that rejects everything until I printed the message
             * instead of bucketing by prefix. */
            let text = a.alloc_str(row.text.trim_end_matches('\0')).map_err(|e| format!("arena: {e:?}"))?;
            let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
            let sel = select::select(&l.module, a).map_err(|e| format!("selection: {e:?}"))?;
            let groups: &[SelGroup] = sel.groups;
            if groups.is_empty() {
                return Ok((0, 0, 0, 0));
            }
            let bbuf = a
                .alloc_filled(4 * (sel.inputs.len() + sel.outputs.len()), VReg(0))
                .map_err(|e| format!("arena: {e:?}"))?;
            let boundary = boundary_of(&sel, bbuf).map_err(|e| format!("boundary: {e:?}"))?;
            let a2 = allocate(groups, PLACEHOLDER, [], boundary, a).map_err(|e| format!("allocator: {e:?}"))?;
            let map = a2.assignment;

            /* The invariant the boundary exists for, checked on every source
             * rather than on the one that exposed it: two DIFFERENT values
             * the builder's output writes read must not have been given the
             * same register. Before the boundary was an input to `allocate`,
             * texfill.vert put three components of the vertex
             * position in `Temp2`. */
            /* Dead groups, now that the boundary says what "read" means: a
             * group whose destination no later group reads and no output
             * write names computes a value nobody can observe. Counted here
             * rather than guessed, because the allocator's temp figure and
             * the instruction count both pay for these. */
            let mut dead = 0usize;
            for (i, g) in groups.iter().enumerate() {
                let Some(SelDst::Virtual(d)) = g.dst else { continue };
                let read_later = groups
                    .iter()
                    .skip(i + 1)
                    .any(|h| h.srcs.iter().flatten().any(|s| matches!(s, SelSrc::Virtual { v, .. } if *v == d)));
                let redefined_later =
                    groups.iter().skip(i + 1).any(|h| matches!(h.dst, Some(SelDst::Virtual(v)) if v == d));
                let at_edge = boundary.live_out.contains(&d);
                if !read_later && (!at_edge || redefined_later) {
                    dead += 1;
                }
            }
            DEAD.with(|c| c.set(c.get() + dead));
            debug_assert!(dead <= groups.len());

            let lo = boundary.live_out;
            let mut collisions = 0usize;
            for (i, va) in lo.iter().enumerate() {
                for vb in lo.iter().skip(i + 1) {
                    if va != vb && map[va.0 as usize] == map[vb.0 as usize] {
                        collisions += 1;
                    }
                }
            }
            Ok((groups.len(), a2.temps, collisions, dead))
        });
        match r {
            None => {}
            Some(Err(e)) => {
                /* Front-end and allocator refusals keep their whole message.
                 * "allocator: 3" is a bucket that hides which file ran out
                 * and by how much -- the same shape as the refusal bucket
                 * that once read every PowerVR source as a front-end reject
                 * because the table stores them NUL-terminated. A bucket
                 * that hides the reason is not a measurement. */
                let kind = if e.starts_with("front end") || e.starts_with("allocator") {
                    e.clone()
                } else {
                    e.split(':').next().unwrap_or("?").to_string()
                };
                *refusals.entry(kind).or_insert(0) += 1;
                if e.starts_with("front end") {
                    continue;
                }
                lowered += 1;
            }
            Some(Ok((n, t, c, d))) => {
                lowered += 1;
                selected += 1;
                allocated += 1;
                collisions += c;
                if c > 0 {
                    clobbered.push(row.name.to_string());
                }
                total_groups += n;
                if d > 0 {
                    worst.push((d, n, row.name.to_string()));
                }
                if n == 0 {
                    empty += 1
                } else {
                    biggest.push((n, t, row.name.to_string()))
                }
            }
        }
    }
    biggest.sort_by_key(|(n, _, _)| core::cmp::Reverse(*n));
    let mut versions: std::collections::BTreeMap<String, usize> = Default::default();
    for row in SHC_SOURCE_TABLE.iter() {
        let v =
            row.text.lines().find(|l| l.trim_start().starts_with("#version")).unwrap_or("(none)").trim().to_string();
        *versions.entry(v).or_insert(0) += 1;
    }
    println!("shipped table: {} sources, by #version: {versions:?}", SHC_SOURCE_TABLE.len());
    println!(
        "  reached selection {lowered}, selected {selected}, allocated {allocated} (of those, {empty} had no groups)"
    );
    println!("  refusals by stage:");
    for (k, v) in &refusals {
        println!("     {k}: {v}");
    }
    println!("  the largest that allocate today (groups, temps):");
    for (n, t, name) in biggest.iter().take(10) {
        println!("     {name:<44} {n:5} groups -> temps {t}");
    }
    println!("  output values sharing a register: {collisions} (over {selected} selected sources)");
    println!("  groups whose result nothing reads: {} of {}", DEAD.with(|c| c.get()), total_groups);
    worst.sort_by_key(|(d, _, _)| core::cmp::Reverse(*d));
    for (d, n, name) in worst.iter().take(5) {
        println!("     {name:<44} {d:4} dead of {n}");
    }

    /* The largest temp count anywhere, which the list above does not show:
     * it is ordered by groups, and the widest program is not the longest.
     * Against `Limits.temps = 252` this is the headroom figure. */
    let peak = biggest.iter().max_by_key(|(_, t, _)| *t);
    if let Some((_, t, name)) = peak {
        println!("  most temporaries: {t} ({name}), declared as {} of 252", t.div_ceil(4) * 4);
    }

    /* Asserted, unlike everything else here, because it is not a quality: an
     * output write reading a register another output's value was placed in is
     * a wrong picture, and one this file measured on the corpus before any
     * encoder existed to show it on a panel. */
    assert_eq!(collisions, 0, "distinct output values share a register in: {clobbered:?}");
    assert!(selected > 0, "nothing selected; the invariant above would hold vacuously");
}

/// Both reports, in ONE test, because the compiler's scratch arena is
/// process-wide and `with_scratch` refuses a second concurrent borrow.
///
/// As two tests they raced: `cargo` runs them in parallel threads, one held
/// the arena and the other got `None`, and the report printed "no scratch"
/// instead of numbers. **It still passed**, because printing nothing is not
/// an assertion failure -- measured at 2 silent reports in 5 of 6 runs, and
/// check.sh never saw it. That is worse than the `LAST_*` flake it rhymes
/// with: that one at least failed loudly some of the time.
///
/// So they are one test, and `report` now panics rather than printing "no
/// scratch", so silence can never pass again.
#[test]
fn usc1_the_allocator_half() {
    what_reaches_the_allocator_from_the_shipped_table();
    the_triangle_pair_through_the_allocator();
    usc2_the_first_arithmetic_diff();
    usc2_the_builder_against_the_shipped_blob();
}

/// USC-2's vehicle, in detail: `texfill.vert` (Qt Quick's texture-fill program
/// under other names) is the chain's
/// first source with real arithmetic and a blob in the shipped table, so this
/// is what the builder diffs against. Printed group by group because the
/// diff will be read group by group.
fn usc2_the_first_arithmetic_diff() {
    const NAME: &str = "texfill.vert";
    let Some(row) = SHC_SOURCE_TABLE.iter().find(|r| r.name == NAME) else {
        println!("{NAME}: not in the table");
        return;
    };
    let out = store::with_scratch(|a| -> Result<String, String> {
        let text = a.alloc_str(row.text.trim_end_matches('\0')).map_err(|e| format!("arena: {e:?}"))?;
        let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| format!("front end: {}", e.msg))?;
        let sel = select::select(&l.module, a).map_err(|e| format!("selection: {e:?}"))?;
        let groups: &[SelGroup] = sel.groups;
        let bbuf =
            a.alloc_filled(4 * (sel.inputs.len() + sel.outputs.len()), VReg(0)).map_err(|e| format!("arena: {e:?}"))?;
        let boundary = boundary_of(&sel, bbuf).map_err(|e| format!("boundary: {e:?}"))?;
        let alloc = allocate(groups, PLACEHOLDER, [], boundary, a).map_err(|e| format!("allocator: {e:?}"))?;
        let map = alloc.assignment;
        let mut s = format!("{NAME}: {} groups, {:?}\n", groups.len(), Summary(alloc));
        for (i, g) in alloc.groups.iter().enumerate() {
            s.push_str(&format!("  {i:3} {:?}\n", One(g)));
        }
        /* The output writes the builder has to emit after the last group,
         * with the registers they read -- the half of the program that is
         * not groups and that the allocator could not see until `Boundary`. */
        s.push_str("  -- output writes (not groups; the builder emits these) --\n");
        for (slot, comps) in sel.outputs {
            s.push_str(&format!("  {slot:?} <-"));
            for c in comps.iter() {
                match c {
                    Some(SelSrc::Virtual { v, .. }) => s.push_str(&format!(" Temp{}", map[v.0 as usize])),
                    Some(other) => s.push_str(&format!(" {other:?}")),
                    None => s.push_str(" _"),
                }
            }
            s.push('\n');
        }
        Ok(s)
    });
    match out {
        None => panic!("{NAME}: the scratch arena was already borrowed"),
        Some(Err(e)) => println!("{NAME}: {e}"),
        Some(Ok(s)) => print!("{s}"),
    }
}

/// One resolved group, in the form the builder receives it.
struct One<'a>(&'a SelGroup);
impl core::fmt::Debug for One<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let g = self.0;
        write!(f, "{:?}.{:?} ", g.op, g.ty)?;
        match g.dst {
            Some(SelDst::Fixed { bank, reg }) => write!(f, "-> {bank:?}{reg}")?,
            Some(SelDst::Virtual(v)) => write!(f, "-> UNRESOLVED v{}", v.0)?,
            None => write!(f, "-> (none)")?,
        }
        for s in g.srcs.iter().flatten() {
            match s {
                SelSrc::Fixed { bank, reg, .. } => write!(f, "  {bank:?}{reg}")?,
                SelSrc::Virtual { v, .. } => write!(f, "  UNRESOLVED v{}", v.0)?,
                SelSrc::Imm(x) => write!(f, "  #{x:#010x}")?,
            }
        }
        Ok(())
    }
}

fn the_triangle_pair_through_the_allocator() {
    println!("USC-1, allocator half. Limits are PLACEHOLDERS: {PLACEHOLDER:?}");
    let rows: Vec<&str> = SHC_SOURCE_TABLE
        .iter()
        .map(|r| r.name)
        .filter(|n| n.contains("passthrough") || n.contains("interp_color"))
        .collect();
    println!("the same names as PowerVR table rows (GLSL 450, the reference blobs): {rows:?}");
    /* The ES 1.00 twins, not the GLSL 450 originals: the originals are
     * correctly rejected by an ES 2.0 front end, which is a right answer and
     * not a result. The same trap as the es100_twins work -- a name in the
     * table is not automatically a source this compiler accepts. */
    report("passthrough.vert", Stage::Vertex);
    report("interp_color.frag", Stage::Fragment);
}

/// USC-2's diff, group by group: the library's chain against the blob pco
/// built from the same source.
///
/// Per group, not totals. Each of our groups is printed with its operands
/// and its skeleton (everything but registers and literals,
/// `verify::skeleton`), and marked with the index of the pco group IN THIS
/// SAME BLOB that has the identical skeleton, or `--` when none does. Then
/// pco's groups the same way. Totals hide exactly what this is for: the
/// first version of this report said "29 decode back" while every
/// destination was a special register.
fn usc2_the_builder_against_the_shipped_blob() {
    use yakogl::shaderc::mesa_reference::SHC_BLOB_TABLE;
    use yakogl::shaderc::online::powervr::chain::build_vertex;
    use yakogl::shaderc::online::powervr::decode::{decode_group, Group};
    use yakogl::shaderc::online::powervr::isa::ops_for;
    use yakogl::shaderc::online::powervr::isa_tables::ENUMS;
    use yakogl::shaderc::online::powervr::operands::{dsts, srcs};
    use yakogl::shaderc::online::powervr::verify::skeleton;

    const NAME: &str = "texfill.vert";
    let Some(row) = SHC_SOURCE_TABLE.iter().find(|r| r.name == NAME) else {
        panic!("{NAME}: not in the source table");
    };
    let regbank = ENUMS.iter().find(|e| e.name == "regbank").unwrap();
    let line = |g: &Group| -> (String, String) {
        let mut ops = Vec::new();
        for (_, ph, r) in g.instrs() {
            let n = ops_for(r, ph).next().map(|o| o.name).unwrap_or("?");
            let mut o = n.to_string();
            for f in ["imm32", "imm_addr"] {
                if let Some(v) = r.get(f) {
                    o.push_str(&format!(" {f}={v:#x}"));
                }
            }
            for f in ["s0neg", "s0abs", "s0flr", "s1neg", "s1abs", "s2neg"] {
                if r.get(f) == Some(1) {
                    o.push_str(&format!(" {f}"));
                }
            }
            ops.push(o);
        }
        let name = |o: Option<yakogl::shaderc::online::powervr::operands::Operand>| {
            o.map(|o| format!("{}{}", regbank.name_of(o.bank).unwrap_or("?"), o.index)).unwrap_or_default()
        };
        let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
        let sr = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
        let ss: Vec<String> = sr[..3].iter().filter(|o| o.is_some()).map(|o| name(*o)).collect();
        let rpt = g.hdr.get("rpt").unwrap_or(1);
        let mut k = String::new();
        skeleton(&mut k, g).unwrap();
        (format!("{:<34} {:>8} <- {:<28} rpt{rpt} {}B", ops.join("+"), name(d[0]), ss.join(", "), g.total), k)
    };
    let walk = |bytes: &[u8]| -> Vec<Group> {
        let mut v = Vec::new();
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            off += g.total;
            v.push(g);
        }
        v
    };

    let out = store::with_scratch(|a| -> Result<String, String> {
        let text = a.alloc_str(row.text.trim_end_matches('\0')).map_err(|e| format!("arena: {e:?}"))?;
        let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| format!("front end: {}", e.msg))?;
        let mut image = vec![0u8; 8192];
        let b = build_vertex(&l.module, PLACEHOLDER, a, &mut image).map_err(|e| format!("chain: {e:?}"))?;
        if let Ok(p) = std::env::var("USC2_DUMP") {
            std::fs::write(&p, &image[..b.len]).map_err(|e| format!("{p}: {e}"))?;
        }
        let ours = walk(&image[..b.len]);

        let key = yakogl::shaderc::sha256::sha256(row.text.trim_end_matches('\0').as_bytes());
        let blob = SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0).ok_or("no pco blob")?;
        // SAFETY: `usc` points at a static of the generated table.
        let dw = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
        let pco_bytes: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
        let theirs = walk(&pco_bytes);
        let their_lines: Vec<(String, String)> = theirs.iter().map(line).collect();

        let mut s = format!(
            "USC-2, {NAME}: ours {} groups / {} bytes (temps {}), pco {} groups / {} bytes\n",
            ours.len(),
            b.len,
            b.temps,
            theirs.len(),
            pco_bytes.len()
        );
        s.push_str("  ours (=pN: pco group N in this blob has the identical skeleton):\n");
        let mut matched = 0usize;
        for (i, g) in ours.iter().enumerate() {
            let (t, k) = line(g);
            let m = their_lines.iter().position(|(_, pk)| *pk == k);
            if m.is_some() {
                matched += 1;
            }
            let tag = m.map(|j| format!("=p{j}")).unwrap_or_else(|| "--".into());
            s.push_str(&format!("   {i:3} {tag:>5}  {t}\n"));
        }
        s.push_str("  pco:\n");
        for (j, (t, _)) in their_lines.iter().enumerate() {
            s.push_str(&format!("   p{j:<3}        {t}\n"));
        }
        s.push_str(&format!(
            "  {matched} of {} of our groups share a skeleton with a group of pco's program\n",
            ours.len()
        ));

        /* The output dwords each side writes, from the decoded writes. */
        let written = |gs: &[Group]| -> Vec<u32> {
            let mut w = Vec::new();
            for g in gs {
                for (_, ph, r) in g.instrs() {
                    if ops_for(r, ph).next().is_some_and(|o| o.name == "uvsw.write") {
                        let base = r.get("imm_addr").unwrap();
                        w.extend(base..base + g.hdr.get("rpt").unwrap_or(1));
                    }
                }
            }
            w.sort();
            w
        };
        let (wo, wp) = (written(&ours), written(&theirs));
        s.push_str(&format!("  output dwords written: ours {wo:?}, pco {wp:?}\n"));
        assert_eq!(wo, wp, "the two programs write different output dwords");
        assert!(
            matched > 0,
            "no skeleton in common with pco's program for the same source: the instrument or the builder is broken"
        );
        Ok(s)
    });
    match out {
        None => panic!("{NAME}: the scratch arena was already borrowed"),
        Some(Err(e)) => panic!("{NAME}: {e}"),
        Some(Ok(s)) => print!("{s}"),
    }
}
