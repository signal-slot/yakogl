// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What runs out when the online path refuses 74 of the table's sources.
//!
//! cts-runner's twin-draw host half reports `Abi(OutOfMemory { left: 30 })`
//! for 74 of the 324 table sources, the whole `distancefieldtext` family
//! among them, all of which the offline table has blobs for -- so they are
//! known-compilable on this part. The instruction was to NAME THE RESOURCE
//! with numbers before anyone writes a fix.
//!
//! **It is not an ABI space and not a GPU limit.** `twin_draw_list.rs` labels
//! it "the code generator ran out of ABI space"; `abi.rs` says otherwise --
//! the error is built as `OutOfMemory { left: store::keep().left() }`. The
//! resource is [`store::KEEP_BYTES`], the compiler's own **keep arena**: 512
//! KiB of `.bss` holding the code, reflection and names of every shader
//! compiled in the process, which **accumulates and is never reset**, because
//! the `ShcShader` the GL core holds points into it.
//!
//! That makes the refusal cumulative rather than per-shader, and the two have
//! opposite fixes: per-shader means our layout wastes space in big shaders,
//! cumulative means the budget is too small for a harness that compiles
//! everything and frees nothing -- in which case the family that shows up in
//! the refusal list is an artefact of where those rows sit in the table. The
//! test below decides between them rather than assuming.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use core::ffi::c_char;
use std::collections::BTreeSet;
use yakogl::shaderc::mesa_reference::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::online::store;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// Compile one source and say what happened, resetting nothing: the arena's
/// accumulation is the subject.
fn compile(name: &str, text: &str) -> Result<ShcShader, String> {
    let stage = if name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let mut s = String::from(text.trim_end_matches('\0'));
    s.push('\0');
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, a log of the size announced.
    let rc = unsafe { f(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        return Ok(out);
    }
    Err(log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect::<String>())
}

/// Bytes this compile added to the keep arena, read from the arena itself
/// rather than reconstructed from the shader's fields.
///
/// Measuring the delta rather than adding up `code + uniforms + consts` is
/// deliberate: the arena also holds the reflection names, and a sum of the
/// parts I happen to know about would be a floor presented as a total. The
/// arena knows.
fn keep_delta(before: usize) -> usize {
    keep_now().saturating_sub(before)
}

/// The keep memory CURRENTLY owned, not its high-water.
///
/// `store::keep_used()` returns `peak.max(used)`, which is the right number
/// for a budget report and the wrong one for a delta: after
/// `forget_compiled()` resets the arena the peak stays where it was, so every
/// delta reads zero. My first version of this file used it and printed
/// "largest of them 0 bytes", which is not a small number, it is no
/// measurement at all.
///
/// And not `store::keep().used()` either, which is what this read until
/// 2026-09-23: since keep became a granule map (11e0f31) a compile's bytes
/// are committed to granules when it finishes and the open arena is fresh
/// again, so that read 0 for every compile -- "0 kept, 48800 of it code
/// (4880000 %)" -- and nothing failed. `keep_in_use` counts the granules.
fn keep_now() -> usize {
    store::keep_in_use()
}

#[test]
fn the_resource_that_runs_out_is_the_keep_arena_and_whether_it_is_cumulative() {
    /* Pass one: the table in its own order, which is what the twin-draw
     * harness does, so the refusal set is reproduced rather than described. */
    let mut refused_oom: Vec<&str> = Vec::new();
    let mut refused_other = 0usize;
    let mut compiled = 0usize;
    let mut costs: Vec<(usize, &str)> = Vec::new();
    let mut code_bytes: Vec<(usize, usize, &str)> = Vec::new();
    for e in SHC_SOURCE_TABLE.iter() {
        let before = keep_now();
        match compile(e.name, e.text) {
            Ok(sh) => {
                compiled += 1;
                let d = keep_delta(before);
                costs.push((d, e.name));
                code_bytes.push((d, sh.usc_words().len() * 4, e.name));
            }
            Err(msg) if msg.contains("OutOfMemory") => refused_oom.push(e.name),
            Err(_) => refused_other += 1,
        }
    }
    let used = store::keep_used();

    println!("\n=== the resource: the compiler's keep arena, not an ABI space ===\n");
    println!("  store::KEEP_BYTES            {:>8}", store::KEEP_BYTES);
    println!("  high-water after the corpus  {used:>8}");
    println!("  sources in the table         {:>8}", SHC_SOURCE_TABLE.len());
    println!("  compiled                     {compiled:>8}");
    println!("  refused OutOfMemory          {:>8}", refused_oom.len());
    println!("  refused for anything else    {refused_other:>8}");

    /* Where the limit came from, quoted from `store.rs`: 300,768 bytes
     * measured for the whole 324-source corpus on 2026-09-22, and 512 KiB
     * called "about 70 % headroom". If the high-water sits at the ceiling now
     * then that margin is gone, and by how much matters more than the fact. */
    /* A compile keeps at least its code, so a zero here is the instrument
     * reading the wrong thing, and must fail rather than print. */
    assert!(
        costs.iter().all(|(c, _)| *c > 0),
        "a compiled program kept 0 bytes: keep_now() is not measuring the keep memory"
    );
    costs.sort_by_key(|(c, _)| core::cmp::Reverse(*c));
    println!("\n  per-shader keep cost, largest five that COMPILED:");
    for (c, n) in costs.iter().take(5) {
        println!("    {c:>7}  {n}");
    }
    if let Some((c, n)) = costs.last() {
        println!("    {c:>7}  {n}   <- smallest");
    }

    /* And how much of that is the CODE, which is the part the arena exists
     * for. What is left is reflection, names and the constant file. If the
     * remainder dominates, the budget is not being spent on shaders. */
    code_bytes.sort_by_key(|(d, _, _)| core::cmp::Reverse(*d));
    println!("\n  of the largest, how much is code and how much is everything else:");
    println!("    {:>7} {:>7} {:>7}  source", "keep", "code", "rest");
    for (d, c, n) in code_bytes.iter().take(5) {
        println!("    {d:>7} {c:>7} {:>7}  {n}", d.saturating_sub(*c));
    }
    let total_keep: usize = code_bytes.iter().map(|(d, _, _)| d).sum();
    let total_code: usize = code_bytes.iter().map(|(_, c, _)| c).sum();
    println!(
        "    over all {} compiled: {total_keep} kept, {total_code} of it code ({} %)",
        code_bytes.len(),
        total_code * 100 / total_keep.max(1)
    );

    if refused_oom.is_empty() {
        println!("\n  nothing was refused for memory in this run.");
        return;
    }
    println!("\n  refused for memory, first five in table order:");
    for n in refused_oom.iter().take(5) {
        println!("    {n}");
    }

    /* THE DECIDING EXPERIMENT. Per-shader means a refused source compiled
     * FIRST in a fresh arena still fails. Cumulative means it compiles, and
     * the family in the refusal list is an artefact of table position.
     *
     * The arena is reset before EACH source. It used to be reset once and the
     * refused sources compiled one after another, so the second pass
     * accumulated like the first: frontend3's ES 3.00 sources made five of
     * 112 "still refused alone" that each compile alone in under 12.3 KB. */
    let refused_set: BTreeSet<&str> = refused_oom.iter().copied().collect();
    yakogl::shaderc::online::backend::forget_compiled();
    let (mut compiled_alone, mut still_refused) = (0usize, 0usize);
    let mut alone_costs: Vec<(usize, &str)> = Vec::new();
    for e in SHC_SOURCE_TABLE.iter().filter(|e| refused_set.contains(e.name)) {
        yakogl::shaderc::online::backend::forget_compiled();
        let before = keep_now();
        match compile(e.name, e.text) {
            Ok(_) => {
                compiled_alone += 1;
                alone_costs.push((keep_delta(before), e.name));
            }
            Err(msg) if msg.contains("OutOfMemory") => still_refused += 1,
            Err(_) => {}
        }
    }
    println!("\n  the same {} sources with the arena reset first:", refused_set.len());
    println!("    compiled        {compiled_alone}");
    println!("    still refused   {still_refused}");
    /* The finding, asserted rather than printed: every source refused in the
     * full walk compiles on a fresh arena. If one ever does not, the refusal
     * has stopped being positional and something really is too big -- which
     * is a different report and should fail rather than read as the same one. */
    assert_eq!(
        still_refused, 0,
        "a source refused in the walk is still refused alone: the budget is no longer only cumulative"
    );
    alone_costs.sort_by_key(|(c, _)| core::cmp::Reverse(*c));
    if let Some((c, n)) = alone_costs.first() {
        println!("    largest of them {c} bytes ({n})");
    }
    if let Some((c, n)) = alone_costs.last() {
        println!("    smallest        {c} bytes ({n})");
    }

    /* And what the reference spends on the same shaders, so "our layout is
     * wasteful" is a comparison rather than an impression. */
    println!("\n  the reference's code size for the family named in the report:");
    for e in SHC_SOURCE_TABLE.iter().filter(|e| e.name.contains("distancefield")) {
        let key = yakogl::shaderc::sha256::sha256(e.text.trim_end_matches('\0').as_bytes());
        if let Some(b) = SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) {
            println!("    {:>7}  {}", b.shader.usc_words().len() * 4, e.name);
        }
    }

    /* The minimal pair the lead named: `lineargradient.vert` refuses in the
     * full walk and `conicalgradient.vert` is accepted. On a fresh arena,
     * compiled together and nothing else, they must BOTH compile if the
     * refusal is positional. */
    yakogl::shaderc::online::backend::forget_compiled();
    println!(
        "\n  one cache record is {} bytes, of which ShcShader is {} ({} of it ShcViv)",
        yakogl::shaderc::online::backend::compiled_record_bytes(),
        core::mem::size_of::<ShcShader>(),
        core::mem::size_of::<yakogl::shaderc::ShcViv>(),
    );
    println!("\n  the minimal pair, alone in a fresh arena:");
    for want in ["conicalgradient.vert", "lineargradient.vert"] {
        match SHC_SOURCE_TABLE.iter().find(|e| e.name == want) {
            None => println!("    {want}: not a table row"),
            Some(e) => {
                let before = keep_now();
                match compile(e.name, e.text) {
                    Ok(sh) => {
                        /* Every writer, in the order `abi.rs` and
                         * `backend.rs` call the arena. The source text and
                         * the cache record are the two nobody had counted. */
                        let source = e.text.trim_end_matches('\0').len() + 1;
                        let record = yakogl::shaderc::online::backend::compiled_record_bytes();
                        let code = sh.usc_words().len() * 4;
                        let uni = sh.num_uniforms as usize * core::mem::size_of::<yakogl::shaderc::ShcUniform>();
                        let consts = sh.viv.const_count as usize * 5;
                        let named = source + record + code + uni + consts;
                        let total = keep_now() - before;
                        println!(
                            "    {want}: {total} kept = source {source} + record {record} + code {code} \
                             + uniforms {uni} + consts {consts}  (named {named}, {} alignment)",
                            total as i64 - named as i64
                        );
                    }
                    Err(m) => println!("    {want}: refused -- {}", m.trim()),
                }
            }
        }
    }

    /* Vacuity: a run that compiled nothing would print a tidy table of zeros
     * and prove nothing about the arena. */
    assert!(compiled > 0, "nothing compiled; this would be a measurement of the harness");
}
