// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How many temporary registers the online compiler declares, against the
//! table's.
//!
//! `temps` is the register count the driver programs, and it is not free: the
//! GC7000 runs fewer threads when a shader declares more, so a compiler that
//! never reuses a register pays for it in occupancy on every draw. This
//! measures the gap and predicts what closing it is worth. It does not close
//! it -- register allocation is `vivante/alloc.rs`, which is not this file's.
//!
//! Two of the three numbers are printed rather than asserted, because "how
//! many registers" is a quality, not a contract. The third **is** asserted:
//! `abi.md` says a shader that uses more temps than it declares corrupts
//! another task's registers, so writing above `temps` is a correctness bug and
//! is pinned here.

#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]

use core::ffi::c_char;
use std::collections::BTreeMap;
use yakogl::shaderc::online::vivante::decode::for_each_instr;
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// `SRCn_RGROUP` for a temporary register, from `vivante/build.rs`.
const RGROUP_TEMP: u32 = 0;

/// Compile one source, resetting the keep arena first so the run measures the
/// compiler rather than the harness's accumulated corpus.
///
/// The reset invalidates every `ShcShader` handed out before it, so it is
/// safe only while nothing else in this binary holds a result.
///
/// **That precondition was written here and then broken.** This file had two
/// `#[test]` functions, which `cargo` runs in parallel threads: each one's
/// `forget_compiled()` pulled the ground out from under the other, and the
/// corpus line read 10, then 108, then 10 sources on three runs of identical
/// code. It passed every time. The two are one test now, in the order they
/// have to run, which is the same fix `usc1_triangle.rs` needed for the same
/// reason -- and the same reason a survey that cannot fail is worse than no
/// survey: I nearly reported a 105-to-10 regression from this noise.
fn compile(stage: u32, src: &str) -> Option<ShcShader> {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let mut s = String::from(src);
    if !s.ends_with('\0') {
        s.push('\0');
    }
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end has a compile entry point");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    (rc == 0).then_some(out)
}

/// Why the compiler refused this source, in its own words.
///
/// `compile` throws the log away, which turns every refusal into the word
/// "refused" -- and a refusal without its reason is exactly the shape this
/// project keeps having to chase back down. The ladder above `s16` refuses
/// today and nobody could say what it refuses on without recompiling by hand.
fn last_refusal(src: &str, stage: u32) -> String {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let mut s = String::from(src);
    if !s.ends_with('\0') {
        s.push('\0');
    }
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end has a compile entry point");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        return String::from("(it compiled this time)");
    }
    let bytes: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Where a register is first written and last read, by register number.
type LiveRanges = BTreeMap<u32, (usize, usize)>;

/// First definition and last use of every temporary register, whether the
/// program branches, and how many instructions it has.
///
/// The ranges are indices into the straight-line instruction list, so they are
/// only a sound liveness approximation when the program does not branch --
/// which is why `branches` is reported beside them rather than assumed away.
fn ranges(code: &[u32]) -> Option<(LiveRanges, bool, usize)> {
    let mut r = LiveRanges::new();
    let mut branches = false;
    let mut n = 0;
    for_each_instr(code, |i, ins| {
        n = i + 1;
        if ins.op().fields.iter().any(|f| f.name == "TARGET") {
            branches = true;
        }
        let mut touch = |reg: u32, i: usize| {
            let e = r.entry(reg).or_insert((i, i));
            e.0 = e.0.min(i);
            e.1 = e.1.max(i);
        };
        /* `DST` is a packed 14-bit field -- addressing mode, register, write
         * mask -- not a register number. Reading it whole reports register
         * 15377 and a defect that is not there. */
        if ins.get("DST_USE") == Some(1) {
            if let Some(d) = ins.get("DST") {
                touch((d >> 3) & 0x7f, i);
            }
        }
        for s in 0..3 {
            let (u, g, rg) = (format!("SRC{s}_USE"), format!("SRC{s}_RGROUP"), format!("SRC{s}_REG"));
            if ins.get(&u) == Some(1) && ins.get(&g) == Some(RGROUP_TEMP) {
                if let Some(x) = ins.get(&rg) {
                    touch(x, i);
                }
            }
        }
        Ok(())
    })
    .ok()?;
    Some((r, branches, n))
}

fn temps_against_the_table() {
    let mut delta: BTreeMap<i64, usize> = BTreeMap::new();
    let mut rows = Vec::new();
    let mut overs: Vec<(&str, i64, i64, i64, usize)> = Vec::new();
    let mut under_declared: Vec<(&str, i64, i64)> = Vec::new();
    let (mut with_cf, mut hits, mut above, mut below, mut gapped) = (0, 0, 0, 0, 0);
    let (mut sum_ours, mut sum_tab, mut sum_pred) = (0i64, 0i64, 0i64);

    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let Some(ours) = compile(stage, e.text) else { continue };
        let txt = e.text.strip_suffix('\0').unwrap_or(e.text);
        let digest = yakogl::shaderc::sha256::sha256(txt.as_bytes());
        let Some(row) = SHC_BLOB_TABLE.iter().find(|b| b.blend_key == 0 && b.sha256 == digest) else { continue };

        let (ot, tt) = (i64::from(ours.temps), i64::from(row.shader.temps));
        *delta.entry(ot - tt).or_insert(0) += 1;
        let Some((r, cf, n)) = ranges(ours.usc_words()) else { continue };

        let highest = r.keys().copied().max().map_or(0, |m| i64::from(m) + 1);
        if highest > ot {
            under_declared.push((e.name, ot, highest));
        }
        if ot == tt {
            continue;
        }
        gapped += 1;
        if cf {
            with_cf += 1;
            continue;
        }

        /* What a register-reusing allocator would have needed: the most
         * registers simultaneously live, but never fewer than the io floor,
         * because a register pinned to an input or an output cannot be
         * reused while the stage is running. */
        let peak = (0..n).map(|i| r.values().filter(|(a, b)| *a <= i && i <= *b).count()).max().unwrap_or(0);
        let v = &ours.viv;
        let floor = v.in_[..v.num_in as usize]
            .iter()
            .chain(v.out[..v.num_out as usize].iter())
            .map(|io| i64::from(io.reg) + 1)
            .max()
            .unwrap_or(1);
        let pred = floor.max(peak as i64);

        match pred.cmp(&tt) {
            core::cmp::Ordering::Equal => hits += 1,
            core::cmp::Ordering::Greater => {
                above += 1;
                overs.push((e.name, ot, tt, pred, row.shader.usc_words().len() / 4));
            }
            core::cmp::Ordering::Less => below += 1,
        }
        sum_ours += ot;
        sum_tab += tt;
        sum_pred += pred;
        rows.push((e.name, ot, tt, pred, r.len(), n));
    }

    println!("\n=== temps: the online compiler against the table ===\n");
    println!("  delta (ours - table): {delta:?}");
    println!("  sources with a gap: {gapped}, of which branch: {with_cf}");
    println!("  totals over the gapped sources: ours {sum_ours}, table {sum_tab}, reuse would need {sum_pred}");
    println!("  reuse prediction against the table: == {hits}   > {above}   < {below}");
    println!("\n  where reuse would still exceed the table:");
    for (n, o, t, p, ti) in &overs {
        let tag = if *ti <= 1 { "   <- table row is an EMPTY program" } else { "" };
        println!("    {n:<44} ours {o:>3} table {t:>3} reuse {p:>3} table_instrs {ti:>3}{tag}");
    }
    println!("\n  the widest gaps:");
    let mut worst = rows.clone();
    worst.sort_by_key(|(_, o, t, _, _, _)| t - o);
    for (n, o, t, p, regs, ins) in worst.iter().take(10) {
        println!("    {n:<44} ours {o:>3} table {t:>3} reuse {p:>3}  regs {regs:>3} instrs {ins:>3}");
    }

    /* A vacuity guard, because the assertion below passes on an empty list.
     * `shaderc::table` aliases the PowerVR blobs in a non-vivante build, and a
     * corpus that produced nothing would make every check here succeed while
     * measuring nothing at all. */
    assert!(!rows.is_empty() || delta.len() == 1, "no source compiled: this measured nothing");

    /* The one real contract in this file. A shader that writes a register it
     * did not declare corrupts another task's registers (`abi.md`), so this
     * is a correctness bug rather than a quality one, and it is asserted. */
    assert!(
        under_declared.is_empty(),
        "these shaders write a register above the temps they declare, which corrupts another task: {under_declared:?}"
    );
}

/// The corpus's total temps, and every wedge source with the allocator's own
/// account of where its live values come from.
///
/// The lead's measure for the if-conversion work: the corpus total before and
/// after, the four wedge sources against the reference's table, and the
/// `wedge_probe` sources unchanged (they are the board's ladder, so a change
/// in them would invalidate a run rather than improve anything).
///
/// The per-source columns are `vivante::alloc`'s instruments, which answer
/// "where do the live values come from" from the allocator's side:
///
/// * `live` -- the most values live at one instruction. This IS the count the
///   lead asked about: 64 on `hsl_to_rgb_vertex` before any of this.
/// * `sunk` -- what liveness would be if every value were produced just
///   before its first read, so `live - sunk` is what better placement could
///   still buy.
/// * `cpy` / `coal` -- virtual-to-virtual copies, and how many of them
///   coalescing could delete.
///
/// Printed, never asserted. Every number here is a quality.
fn the_corpus_total_and_every_wedge_source() {
    use yakogl::shaderc::online::vivante::alloc;

    let mut total = 0u32;
    let mut compiled = 0usize;
    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        if let Some(sh) = compile(stage, e.text) {
            total += u32::from(sh.temps);
            compiled += 1;
            /* Per-source rows, for an A/B that has to name WHICH sources a
             * pass moved rather than only the total. Off unless asked for:
             * 137 rows is noise in an ordinary run. */
            if std::env::var_os("YAKOGL_TEMPS_ROWS").is_some() {
                println!("ROW {} {} {}", e.name, sh.temps, sh.usc_words().len() / 4);
            }
        }
    }
    println!("\n=== the corpus ===");
    println!("  {compiled} sources compile, {total} temps in total");
    /* A canary for the race described on `compile`, not a score. The corpus
     * has 142 sources this back end can be asked about and it serves about a
     * hundred; a run that reports ten has not measured the compiler. */
    assert!(compiled > 50, "only {compiled} sources compiled: the keep arena was pulled out from under this run");

    println!("\n=== the wedge sources, and where their live values come from ===");
    println!(
        "  {:<30} {:>5} {:>6} {:>5} {:>5} {:>4} {:>5} {:>4}",
        "source", "temps", "instrs", "live", "sunk", "cpy", "coal", "wide"
    );
    let show = |dir: &str, file: &str| {
        let path = format!("{dir}/{file}");
        let Ok(src) = std::fs::read_to_string(&path) else {
            println!("  {file:<30} (missing: {path})");
            return;
        };
        let stage = if file.ends_with(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        match compile(stage, &src) {
            Some(sh) => println!(
                "  {:<30} {:>5} {:>6} {:>5} {:>5} {:>4} {:>5} {:>4}",
                file,
                sh.temps,
                sh.usc_words().len() / 4,
                alloc::last_live_components(),
                alloc::last_sunk_overlap(),
                alloc::last_copies(),
                alloc::last_coalescable(),
                alloc::last_wide_but_scalar(),
            ),
            None => println!("  {file:<30} refused: {}", last_refusal(&src, stage)),
        }
    };
    for f in
        ["hsl_to_rgb_vertex.vert", "rgb_to_hsl_vertex.vert", "hsl_to_rgb_fragment.frag", "rgb_to_hsl_fragment.frag"]
    {
        show("tools/cts/testdata/wedge-shaders", f);
    }
    println!("\n=== the wedge_probe ladder: these must NOT move ===");
    for f in ["a_many_temps.vert", "b_many_instructions.vert"] {
        show("tools/cts/wedge_probe", f);
    }
    for f in ["s08.vert", "s12.vert", "s16.vert", "s20.vert", "s24.vert", "s28.vert", "s32.vert", "s36.vert"] {
        show("tools/cts/wedge_probe/steps", f);
    }
}

/// Both surveys, in ONE test.
///
/// Not a style choice: see `compile` above. Two tests in this binary race
/// over the compiler's keep arena and each other's results, and the corpus
/// count then swings by a factor of ten between runs with nothing changed.
#[test]
fn the_vivante_temps_surveys() {
    temps_against_the_table();
    the_corpus_total_and_every_wedge_source();
}
