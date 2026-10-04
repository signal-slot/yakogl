// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The oracle harness: the in-process compiler against the offline table.
//!
//! The offline table is the only thing that has ever drawn a correct picture
//! on either board, so it is what the new compiler is scored against. For
//! every source in `SHC_SOURCE_TABLE` this runs the source through the new
//! front end and compares the reflection it produces with the reflection the
//! table's row holds, **field by field**, and prints a table of differences.
//!
//! ```sh
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
//!     --test online_oracle -- --nocapture
//! ```
//!
//! # Why a difference table and not a pass/fail
//!
//! A pass/fail would be one bit over hundreds of sources, and the first run
//! will not pass. What is wanted while the front end is being written is
//! *which* field disagrees on *how many* shaders, so that the next hour's work
//! is obvious; and later, that the set of differences is empty. The counts
//! print either way, so a run that fixed nothing and a run that fixed half the
//! rows cannot look alike.
//!
//! # Why it starts by scoring the oracle itself
//!
//! Before believing any difference count, the thing being counted has to be
//! able to appear in what was measured. A harness that could not find a
//! source's row would report "no differences" for it, and that silent zero
//! reads exactly like a clean result. So the first section counts the oracle
//! side — sources, rows found, rows missing — and the comparison only runs
//! over sources that have a row. Anything else is reported as unmatched, never
//! as agreement.
//!
//! # Its two readers
//!
//! It is es100-frontend's M2 scoreboard as much as gles2-core's M3
//! deliverable: it needs no code generator, no encoder and no board, so it can
//! run the day the front end first produces reflection. It reads
//! `shaderc::table` rather than either GPU's table by name, so the same
//! harness scores a PowerVR build and a GC7000 build.
// The corpus lives in the offline table, so this measures nothing in a build
// that carries none (`no-shader-table`, the image configuration).
#![cfg(not(feature = "no-shader-table"))]
#![cfg(feature = "shaderc-online")]

use std::collections::BTreeMap;
use std::ffi::c_char;
use std::fmt::Write as _;

use yakogl::shaderc::sha256::sha256;
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcBlob, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use yakogl::util::{cstr_bytes, name_bytes};

/// One GC7000 register's GLSL name, as bytes. `ShcVivIo::name()` returns
/// `c_char`s, which are signed on this host.
#[cfg(feature = "gpu-vivante")]
fn io_name(io: &yakogl::shaderc::ShcVivIo) -> &[u8] {
    let n = io.name();
    // SAFETY: c_char and u8 have the same size and alignment; this reads the
    // same bytes with no signedness applied.
    unsafe { core::slice::from_raw_parts(n.as_ptr() as *const u8, n.len()) }
}

/// The GLSL text of a source row, without the trailing NUL the table stores.
fn source_text(text: &str) -> &str {
    text.strip_suffix('\0').unwrap_or(text)
}

/// The table row a source was compiled into: the base shader (blend key 0).
///
/// The key is the SHA-256 of the source text exactly as an application passes
/// it, which is how `offline.rs` looks a shader up at run time — so this finds
/// the row by the same rule the product does, rather than by name.
fn row_for(text: &str) -> Option<&'static ShcBlob> {
    let digest = sha256(source_text(text).as_bytes());
    SHC_BLOB_TABLE.iter().find(|b| b.blend_key == 0 && b.sha256 == digest)
}

/// Why a source has no row, when it has none.
///
/// The distinction matters: one of these is a property of the generated table
/// that the harness has to work around, and the other is the harness scoring a
/// subset of the shaders without saying so.
enum NoRow {
    /// A variant entry the table generator wrote: its name is `<base> @<what>`
    /// and its text is the *rewritten* GLSL of the variant, while the blob was
    /// built from, and keyed by, the base source. Measured on the GC7000
    /// table: `facing.frag @front_cw` holds text hashing to e7e95724..., and
    /// both of its blob rows carry 8e9a6255... (the base text) with blend keys
    /// 0 and 1. So no row can be found for it, and none should be: the
    /// variant's reflection is scored through its base.
    VariantOfABaseThatHasARow,
    /// No row, and no explanation. The harness cannot score this source, and
    /// must not pretend the silence is agreement.
    Unexplained,
}

fn why_no_row(name: &str) -> NoRow {
    let Some((base, _variant)) = name.split_once(" @") else {
        return NoRow::Unexplained;
    };
    match SHC_SOURCE_TABLE.iter().find(|s| s.name == base) {
        Some(b) if row_for(b.text).is_some() => NoRow::VariantOfABaseThatHasARow,
        _ => NoRow::Unexplained,
    }
}

/// Every reflection field of one shader, as strings, so that any two can be
/// compared and printed without a case per field.
///
/// Stringly-typed on purpose: the harness's job is to say *which field*
/// disagrees, and a `String` compares and prints uniformly. The values come
/// from the `ShcShader` the table holds and, once the front end exists, from
/// the one it produces.
fn reflection_fields(sh: &ShcShader) -> BTreeMap<String, String> {
    let mut f = BTreeMap::new();
    /* Read by the driver: `src/vivante/shader.rs` programs the temporary
     * register count from `temps` (:52 vertex, :127 fragment) and the
     * instruction count from `usc_dw / 4` (:157). The harness was blind to
     * both -- a compiler that produced perfect reflection around a shader
     * claiming the wrong register count would have scored clean here and
     * failed on the board. */
    f.insert("temps".into(), sh.temps.to_string());
    f.insert("usc_dw".into(), sh.usc_dw.to_string());
    f.insert("num_attribs".into(), sh.num_attribs.to_string());
    f.insert("num_varyings".into(), sh.num_varyings.to_string());
    f.insert("num_uniforms".into(), sh.num_uniforms.to_string());
    f.insert("num_outputs".into(), sh.num_outputs.to_string());
    f.insert("uses_discard".into(), sh.uses_discard.to_string());
    f.insert("uniform_dwords".into(), sh.uniform_dwords.to_string());

    for i in 0..sh.num_attribs as usize {
        let a = &sh.attribs[i];
        let name = String::from_utf8_lossy(name_bytes(&a.name)).into_owned();
        f.insert(format!("attrib[{i}].name"), name);
        f.insert(format!("attrib[{i}].location"), a.location.to_string());
        f.insert(format!("attrib[{i}].components"), a.components.to_string());
        f.insert(format!("attrib[{i}].gl_type"), a.gl_type.to_string());
    }
    for i in 0..sh.num_varyings as usize {
        let v = &sh.varyings[i];
        let name = String::from_utf8_lossy(name_bytes(&v.name)).into_owned();
        f.insert(format!("varying[{i}].name"), name);
        f.insert(format!("varying[{i}].location"), v.location.to_string());
        f.insert(format!("varying[{i}].components"), v.components.to_string());
        f.insert(format!("varying[{i}].interp"), v.interp.to_string());
    }
    /* On the GC7000 the inter-stage wiring is not in `varyings` at all --
     * every blob of the shipped table has num_varyings = 0 (measured: 307 of
     * 307) -- it is in `ShcViv`, as `in_`/`out` register lists. A comparison
     * that stopped at the fields above would have reported "no differences"
     * for everything that GPU actually uses, and the whole report would have
     * been clean and meaningless. */
    #[cfg(feature = "gpu-vivante")]
    {
        let v = &sh.viv;
        f.insert("viv.num_in".into(), v.num_in.to_string());
        f.insert("viv.num_out".into(), v.num_out.to_string());
        f.insert("viv.vs_pos_out_reg".into(), v.vs_pos_out_reg.to_string());
        f.insert("viv.vs_pointsize_out_reg".into(), v.vs_pointsize_out_reg.to_string());
        f.insert("viv.vs_id_in_reg".into(), v.vs_id_in_reg.to_string());
        f.insert("viv.ps_color_out_reg".into(), v.ps_color_out_reg.to_string());
        f.insert("viv.ps_depth_out_reg".into(), v.ps_depth_out_reg.to_string());
        f.insert("viv.vs_load_balancing".into(), format!("{:#x}", v.vs_load_balancing));
        f.insert("viv.input_count_unk8".into(), v.input_count_unk8.to_string());
        f.insert("viv.const_count".into(), v.const_count.to_string());
        for i in 0..v.num_in as usize {
            let io = &v.in_[i];
            f.insert(format!("viv.in[{i}].name"), String::from_utf8_lossy(io_name(io)).into_owned());
            f.insert(format!("viv.in[{i}].reg"), io.reg.to_string());
            f.insert(format!("viv.in[{i}].slot"), io.slot.to_string());
            f.insert(format!("viv.in[{i}].components"), io.components.to_string());
            f.insert(format!("viv.in[{i}].interp"), io.interp.to_string());
        }
        for i in 0..v.num_out as usize {
            let io = &v.out[i];
            f.insert(format!("viv.out[{i}].name"), String::from_utf8_lossy(io_name(io)).into_owned());
            f.insert(format!("viv.out[{i}].reg"), io.reg.to_string());
            f.insert(format!("viv.out[{i}].slot"), io.slot.to_string());
            f.insert(format!("viv.out[{i}].components"), io.components.to_string());
        }
        for (i, (&what, &data)) in v.const_contents().iter().zip(v.const_data()).enumerate() {
            f.insert(format!("viv.const[{i}]"), format!("{what}:{data:#x}"));
        }
    }

    for (i, u) in sh.uniform_table().iter().enumerate() {
        let name = String::from_utf8_lossy(name_bytes(&u.name)).into_owned();
        f.insert(format!("uniform[{i}].name"), name);
        f.insert(format!("uniform[{i}].dword_off"), u.dword_off.to_string());
        f.insert(format!("uniform[{i}].dwords"), u.dwords.to_string());
        f.insert(format!("uniform[{i}].gl_type"), u.gl_type.to_string());
        f.insert(format!("uniform[{i}].array_len"), u.array_len.to_string());
        f.insert(format!("uniform[{i}].binding"), u.binding.to_string());
        f.insert(format!("uniform[{i}].inner"), u.inner.to_string());
    }
    f
}

/// A compiled shader, holding the lock that keeps it readable.
///
/// `forget_compiled()` resets the keep arena, and **every `ShcShader` handed
/// out before it points into that arena**. Serialising the compiles alone is
/// not enough: the caller reads the result *after* the compile returns, and
/// another test thread resetting the arena in that window leaves the words
/// pointing at whatever it wrote next. That is not hypothetical -- it decoded
/// as `NoParse` in `generated_code_against_the_table_structurally`, which read
/// as "our encoder emitted an instruction no hardware can read" and was in
/// fact this. Tying the guard to the value makes the window impossible to
/// leave open: the arena cannot be reset while any result of a compile is
/// alive, because resetting it needs this same lock.
struct Compiled {
    sh: Box<ShcShader>,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl core::ops::Deref for Compiled {
    type Target = ShcShader;
    fn deref(&self) -> &ShcShader {
        &self.sh
    }
}

/// What the new compiler made of one source, or why there is nothing to
/// compare.
enum Attempt {
    Compiled(Compiled),
    /// The compiler is not finished: the text is its own log.
    Unavailable(String),
}

/// Run one source through the new compiler, by the path an application takes.
///
/// This calls `SHC_ONLINE.compile` rather than the front end directly, so the
/// harness scores what `glCompileShader` would actually produce — the cache,
/// the arenas and the error mapping included. Today it always reports
/// `Unavailable`, and the log says which milestone is missing.
/// Serialises every compile in this binary.
///
/// The compiler owns **one** scratch arena, and `store::with_scratch` refuses a
/// second borrow of it -- deliberately, because a compile running inside
/// another compile would hand out memory the outer one still holds. `cargo
/// test` runs this file's tests on parallel threads, so without this lock two
/// tests iterating the table meet in that refusal.
///
/// It cost 61 of 307 sources to find. They were reported as "a compile was
/// started inside a compile", which reads like a compiler defect and is not:
/// those sources were never measured at all, and the run still said
/// `test result: ok`. A harness that silently drops a fifth of its corpus
/// reports a cleaner number than the truth, which is the one failure mode an
/// oracle must not have.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn compile_through_the_new_compiler(stage: u32, text: &str) -> Attempt {
    /* Poison is not interesting here: a test that panicked while holding this
     * has already failed the run, and the arena is reset at the top of every
     * compile regardless. */
    let serial = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    /* One shader at a time. The keep arena is never reset in a real build --
     * the GL core holds pointers into it -- so compiling a 324-source corpus
     * in one process accumulates all of them and measures something no
     * application does. What a product's arena has to hold is the largest
     * single shader and however many it keeps alive at once, and the
     * corpus-total is neither.
     *
     * This is also what the arena running out taught: sized from a
     * corpus-total measured when two sources compiled, the constant was
     * reached the moment coverage grew, and 43 sources were refused for
     * memory while reading as compiler gaps. */
    yakogl::shaderc::online::backend::forget_compiled();
    let mut src = Vec::from(source_text(text).as_bytes());
    src.push(0);
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end has a compile entry point");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size
    // announced -- the contract `struct shc_backend` states.
    let rc = unsafe { compile(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        return Attempt::Compiled(Compiled { sh: Box::new(out), _serial: serial });
    }
    // SAFETY: the back end NUL-terminated the log.
    let why = unsafe { cstr_bytes(log.as_ptr()) };
    Attempt::Unavailable(String::from_utf8_lossy(why).into_owned())
}

/// How every source fared, in the order the report prints them.
#[derive(Default)]
struct Tally {
    /// Sources in the table.
    sources: usize,
    /// Sources whose table row was found by digest.
    matched: usize,
    /// Variant entries whose base source carries the row (see [`NoRow`]).
    variants: Vec<&'static str>,
    /// Sources with no row and no explanation: the harness cannot score them.
    unmatched: Vec<&'static str>,
    /// Sources the new compiler could not compile yet.
    not_compiled: usize,
    /// Sources compiled and identical in every reflection field.
    identical: usize,
    /// Per dialect: how many sources, and how many of them compiled.
    by_dialect: BTreeMap<&'static str, (usize, usize)>,
    /// Field name -> how many sources disagree on it.
    differing: BTreeMap<String, usize>,
    /// The first few disagreements in full, so a reader has something concrete.
    examples: Vec<String>,
}

const EXAMPLES: usize = 12;

/// Which language a source is written in.
///
/// This compiler is an **ES 2.0** compiler, so GLSL ES 1.00 is the only
/// dialect it is meant to accept. The other two it must REFUSE, and refusing
/// them is success. A single score over the whole table mixes the two and
/// measures the wrong thing: it can be raised by accepting a shader that
/// should have been rejected, and it makes the compiler look worse than it is
/// by counting sources nobody will ever ask it to compile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dialect {
    /// `#version 100`, or no directive at all -- which IS ES 1.00, by the
    /// spec's own default, not an unknown.
    Es100,
    /// `#version 300 es`: ES 3.00, outside an ES 2.0 compiler by construction.
    Es300,
    /// `#version 450`: desktop GLSL. No ES 2.0 application can write one.
    Glsl450,
}

impl Dialect {
    fn of(text: &str) -> Dialect {
        /* The first non-blank, non-comment line, which is where a #version
         * directive is required to be. Reading the whole text would match the
         * word inside a comment further down. */
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with("//") {
                continue;
            }
            let Some(rest) = l.strip_prefix("#version") else { break };
            let rest = rest.trim();
            if rest.starts_with("300") && rest.contains("es") {
                return Dialect::Es300;
            }
            if rest.starts_with("100") {
                return Dialect::Es100;
            }
            if rest.starts_with("450") {
                return Dialect::Glsl450;
            }
            break;
        }
        /* No directive: ES 1.00 by the spec's default. */
        Dialect::Es100
    }

    fn name(self) -> &'static str {
        match self {
            Dialect::Es100 => "ES 1.00",
            Dialect::Es300 => "ES 3.00",
            Dialect::Glsl450 => "GLSL 450",
        }
    }
}

/// Fields the register allocator decides, where a difference from the table is
/// expected rather than suspicious.
///
/// Kept deliberately short. Every name here is one the harness stops holding
/// the compiler to, so a name added carelessly buys silence on a real defect;
/// each needs a measurement showing the shipped blobs themselves disagree.
fn allocation_result(field: &str) -> bool {
    field == "viv.ps_color_out_reg"
}

/// Fields that differ because of a decision, not a defect.
///
/// The constant file's ORDER: ours runs forward from block dword 0, the
/// offline compiler's runs backward (`CD_4` is `12,13,14,15 / 8,9,10,11 /
/// 4,5,6,7 / 0,1,2,3`, so its `u0` holds the last column). Both are
/// self-consistent, because the IR names a uniform by its block dword and the
/// code generator decides the register; what must never differ is which column
/// meets which vector component, and that is the IR's to say.
///
/// **Ruled 2026-09-22: it stays as it is.** Matching the reference would buy a
/// cleaner diff and nothing else, and the change touches every uniform read.
/// Labelled here so nobody re-opens it from the diff, which is where it looks
/// like dozens of separate defects.
fn ordering_decided(field: &str) -> bool {
    field.starts_with("viv.const[")
}

#[test]
fn reflection_against_the_offline_table() {
    let mut t = Tally::default();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();

    for s in SHC_SOURCE_TABLE.iter() {
        t.sources += 1;
        /* Classified for every source, whether or not it has a row: a
         * source's dialect is a property of its text, and counting it only
         * where the comparison happens would make the denominator move for
         * reasons that have nothing to do with the language. */
        let dialect = Dialect::of(source_text(s.text)).name();
        t.by_dialect.entry(dialect).or_default().0 += 1;
        let Some(row) = row_for(s.text) else {
            /* No row for this text. The comparison cannot run, and calling it
             * "no differences" would be a zero that nothing could have made
             * non-zero. */
            match why_no_row(s.name) {
                NoRow::VariantOfABaseThatHasARow => t.variants.push(s.name),
                NoRow::Unexplained => t.unmatched.push(s.name),
            }
            continue;
        };
        t.matched += 1;

        let want = reflection_fields(&row.shader);
        let got = match compile_through_the_new_compiler(row.stage, s.text) {
            Attempt::Compiled(sh) => {
                t.by_dialect.entry(dialect).or_default().1 += 1;
                reflection_fields(&sh)
            }
            Attempt::Unavailable(why) => {
                t.not_compiled += 1;
                /* Only ES 1.00 refusals go in the work list. An ES 3.00 or
                 * GLSL 450 source refused at its version line is the compiler
                 * doing its job, and mixing those in makes the list look long
                 * and gives it nothing anyone can act on. */
                if dialect == Dialect::Es100.name() {
                    *reasons.entry(why).or_default() += 1;
                }
                continue;
            }
        };

        let mut differences = Vec::new();
        for (field, expected) in &want {
            match got.get(field) {
                Some(actual) if actual == expected => {}
                Some(actual) => differences.push(format!("{field}: table {expected:?}, ours {actual:?}")),
                None => differences.push(format!("{field}: table {expected:?}, ours absent")),
            }
        }
        for (field, actual) in &got {
            if !want.contains_key(field) {
                differences.push(format!("{field}: table absent, ours {actual:?}"));
            }
        }

        if differences.is_empty() {
            t.identical += 1;
            continue;
        }
        for d in &differences {
            let field = d.split(':').next().unwrap_or(d).to_string();
            *t.differing.entry(field).or_default() += 1;
        }
        if t.examples.len() < EXAMPLES {
            t.examples.push(format!("{} [{}]\n      {}", s.name, stage_name(row.stage), differences.join("\n      ")));
        }
    }

    println!("{}", report(&t, &reasons));

    /* The oracle side is the only thing asserted today: a harness that cannot
     * find the rows it scores against would print zeros that look like
     * success. The reflection comparison itself is a report and not a verdict
     * until the front end exists -- see the module note. */
    assert!(t.sources > 0, "the source table is empty: there is nothing to score against");
    assert_eq!(
        t.unmatched.len(),
        0,
        "every source in SHC_SOURCE_TABLE must have a blob row, or be a variant of one that has \
         (see NoRow) -- otherwise the harness is scoring a subset without saying so; missing: {:?}",
        &t.unmatched[..t.unmatched.len().min(8)]
    );
}

fn stage_name(stage: u32) -> &'static str {
    match stage {
        SHC_STAGE_VERTEX => "vertex",
        SHC_STAGE_FRAGMENT => "fragment",
        _ => "?",
    }
}

fn report(t: &Tally, reasons: &BTreeMap<String, usize>) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "\n=== online shaderc: reflection against the offline table ===\n");
    let _ = writeln!(o, "  the oracle");
    let _ = writeln!(o, "    sources in the table       {}", t.sources);
    let _ = writeln!(o, "    with a blob row            {}", t.matched);
    let _ = writeln!(o, "    variant of a scored base   {}", t.variants.len());
    let _ = writeln!(o, "    with no blob row           {}", t.unmatched.len());
    let compared = t.matched - t.not_compiled;

    /* The governing number first, and by dialect, because this is an ES 2.0
     * compiler: GLSL ES 1.00 is its corpus and the other two it must refuse.
     * A single figure over the whole table can be raised by accepting a
     * shader that should have been rejected. */
    let _ = writeln!(o, "\n  the compiler, by dialect");
    let es100 = t.by_dialect.get("ES 1.00").copied().unwrap_or((0, 0));
    let _ = writeln!(o, "    ES 1.00   {:4} of {:4}   <- the score", es100.1, es100.0);
    for d in ["ES 3.00", "GLSL 450"] {
        if let Some((n, ok)) = t.by_dialect.get(d) {
            let _ = writeln!(
                o,
                "    {d:<9} {n:4} refused by design{}",
                if *ok == 0 { "" } else { "   *** ACCEPTED SOME: that is a defect, not progress ***" }
            );
        }
    }

    let _ = writeln!(o, "\n  the compiler, over every source the harness tried");
    let _ = writeln!(o, "    not compiled yet           {}", t.not_compiled);
    let _ = writeln!(o, "    compared                   {compared}");
    let _ = writeln!(o, "    identical in every field   {}", t.identical);
    /* The arena's high-water mark over every source attempted. This is the
     * number `store::SCRATCH_BYTES` is set from -- measured over the corpus
     * rather than guessed, which is what the constant's doc comment promised
     * and what "scratch buffer exhausted" rows make urgent. */
    let _ = writeln!(
        o,
        "\n  compiler memory\n    scratch peak               {} bytes of {}\n    keep used                  {} bytes of {}",
        yakogl::shaderc::online::store::scratch_peak(),
        yakogl::shaderc::online::store::SCRATCH_BYTES,
        yakogl::shaderc::online::store::keep_used(),
        yakogl::shaderc::online::store::KEEP_BYTES,
    );
    /* The allocator's bound, measured for the same reason as the arenas: a
     * bound nobody re-reads becomes wrong the first time the front end
     * covers more. */
    let _ = writeln!(
        o,
        "    virtual registers          {} of {}",
        yakogl::shaderc::online::vivante::alloc::vreg_peak(),
        yakogl::shaderc::online::vivante::alloc::MAX_VREGS,
    );
    let _ = writeln!(o, "    differing                  {}", compared.saturating_sub(t.identical));

    if !reasons.is_empty() {
        let _ = writeln!(o, "\n  why an ES 1.00 source was not compiled -- the work list");
        for (why, n) in reasons {
            let _ = writeln!(o, "    {n:5}  {why}");
        }
    }

    if !t.differing.is_empty() {
        let _ = writeln!(o, "\n  field                                sources differing");
        let mut rows: Vec<_> = t.differing.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (field, n) in rows {
            let note = if allocation_result(field) {
                "(allocation result)"
            } else if ordering_decided(field) {
                "(ordering, decided 2026-09-22)"
            } else {
                ""
            };
            let _ = writeln!(o, "    {field:<36} {n:5}  {note}");
        }
        if t.differing.keys().any(|f| allocation_result(f) || ordering_decided(f)) {
            for line in [
                "",
                "  An (allocation result) field is one the register allocator chooses, so ours",
                "  differing from the table's is not by itself a defect. ps_color_out_reg takes",
                "  six different values across the 138 fragment blobs (0 in 3, 1 in 90, 2 in 39,",
                "  3 in 4, 7 and 8 in one each) and overlaps a dead input register in 37 of them,",
                "  so there is no one right answer to match. Still reported: a field that stopped",
                "  being printed is one nobody would notice going wrong.",
                "",
                "  On a VERTEX shader it differs for a second reason: the table writes 0 in all",
                "  179 vertex blobs and we write NO_REG. The field does not apply to that stage,",
                "  and src/vivante/shader.rs reads it only from the fragment shader, mapping",
                "  NO_REG to 0 -- so both work, and NO_REG is the one that cannot be misread as",
                "  \"the colour is in register 0\".",
            ] {
                let _ = writeln!(o, "{line}");
            }
        }
    }

    if !t.examples.is_empty() {
        let _ = writeln!(o, "\n  first {} disagreements in full", t.examples.len());
        for e in &t.examples {
            let _ = writeln!(o, "    {e}");
        }
    }

    if !t.variants.is_empty() {
        let _ = writeln!(o, "\n  variant entries, scored through their base source");
        for n in &t.variants {
            let _ = writeln!(o, "    {n}");
        }
    }
    if !t.unmatched.is_empty() {
        let _ = writeln!(o, "\n  sources with no blob row (nothing to compare against)");
        for n in t.unmatched.iter().take(16) {
            let _ = writeln!(o, "    {n}");
        }
    }
    o
}

/// The harness's own instrument, checked before any count from it is believed.
///
/// `reflection_fields` is what every difference is measured with, so a field it
/// silently dropped would read as agreement on every shader in the table --
/// the whole report would be clean and mean nothing.
#[test]
fn the_comparison_notices_a_change_in_every_kind_of_field_it_claims_to_cover() {
    let row = SHC_BLOB_TABLE
        .iter()
        .find(|b| b.blend_key == 0 && b.shader.num_attribs > 0)
        .expect("the table has a shader with vertex attributes");

    let base = reflection_fields(&row.shader);
    assert!(base.len() > 6, "a shader with attributes has more than the scalar fields: {}", base.len());

    let mut changed = row.shader;
    changed.num_attribs = changed.num_attribs.wrapping_add(1);
    assert_ne!(reflection_fields(&changed), base, "a changed attribute count has to show up");

    let mut changed = row.shader;
    changed.attribs[0].components = changed.attribs[0].components.wrapping_add(1);
    assert_ne!(reflection_fields(&changed), base, "a changed attribute width has to show up");

    let mut changed = row.shader;
    changed.uses_discard ^= 1;
    assert_ne!(reflection_fields(&changed), base, "a changed discard flag has to show up");

    let mut changed = row.shader;
    changed.uniform_dwords = changed.uniform_dwords.wrapping_add(1);
    assert_ne!(reflection_fields(&changed), base, "a changed uniform block size has to show up");

    /* The inter-stage wiring, which lives in a different place on each GPU.
     * This is the field group the first GC7000 run of this harness caught me
     * not covering at all. */
    #[cfg(not(feature = "gpu-vivante"))]
    {
        let row = SHC_BLOB_TABLE
            .iter()
            .find(|b| b.blend_key == 0 && b.shader.num_varyings > 0)
            .expect("the PowerVR table has a shader with varyings");
        let base = reflection_fields(&row.shader);
        let mut changed = row.shader;
        changed.varyings[0].location = changed.varyings[0].location.wrapping_add(1);
        assert_ne!(reflection_fields(&changed), base, "a changed varying location has to show up");
    }
    #[cfg(feature = "gpu-vivante")]
    {
        /* Measured: every blob of the shipped GC7000 table has
         * num_varyings = 0, so `varyings` carries nothing there and the
         * wiring is `ShcViv::in_` / `out`. */
        assert!(
            SHC_BLOB_TABLE.iter().all(|b| b.shader.num_varyings == 0),
            "if the GC7000 table ever fills `varyings`, this harness has to compare it too"
        );
        let row = SHC_BLOB_TABLE
            .iter()
            .find(|b| b.blend_key == 0 && b.shader.viv.num_in > 0)
            .expect("the GC7000 table has a shader with input registers");
        let base = reflection_fields(&row.shader);
        let mut changed = row.shader;
        changed.viv.in_[0].reg = changed.viv.in_[0].reg.wrapping_add(1);
        assert_ne!(reflection_fields(&changed), base, "a changed GC7000 input register has to show up");

        let mut changed = row.shader;
        changed.viv.num_in = changed.viv.num_in.wrapping_add(1);
        assert_ne!(reflection_fields(&changed), base, "a changed GC7000 input count has to show up");

        let mut changed = row.shader;
        changed.viv.vs_load_balancing ^= 1;
        assert_ne!(reflection_fields(&changed), base, "a changed VS_LOAD_BALANCING word has to show up");
    }
}

/// The variant entries the GC7000 table generator writes hold the variant's
/// *rewritten* GLSL, while the blob rows are keyed by the base source's
/// digest. Measured on the shipped table: `facing.frag @front_cw` holds text
/// hashing to e7e95724..., and both of its blob rows carry 8e9a6255... So the
/// harness cannot find a row for the variant's own text, and `why_no_row` has
/// to recognise that rather than count it as a gap.
///
/// It matters beyond this harness: `SHC_SOURCE_TABLE` is documented as "what
/// an application must pass to glShaderSource byte for byte on an offline-only
/// target", and that text would miss the table. Reported to the table's owner;
/// pinned here so the day it changes, this says so.
#[test]
fn a_variant_entry_is_recognised_and_not_counted_as_a_gap() {
    let variants: Vec<&str> = SHC_SOURCE_TABLE.iter().filter(|s| row_for(s.text).is_none()).map(|s| s.name).collect();
    for name in &variants {
        assert!(
            matches!(why_no_row(name), NoRow::VariantOfABaseThatHasARow),
            "{name} has no row and no explanation: the harness would be scoring a subset"
        );
        let (base, _) = name.split_once(" @").expect("recognised as a variant, so it has the suffix");
        assert!(SHC_SOURCE_TABLE.iter().any(|s| s.name == base), "{name}: its base {base} is in the table");
    }
    if cfg!(not(feature = "gpu-vivante")) {
        assert!(variants.is_empty(), "the PowerVR table has no variant source entries: {variants:?}");
    }
}

/// A source is matched to its row by the digest the product uses. Matching by
/// name instead would score a different row than the one an application gets.
#[test]
fn a_source_is_matched_to_its_row_by_digest_and_not_by_name() {
    let s = SHC_SOURCE_TABLE.first().expect("the source table is not empty");
    let row = row_for(s.text).expect("the first source has a row");
    assert_eq!(row.sha256, sha256(source_text(s.text).as_bytes()));
    assert!(row_for("void main(){ /* nothing in the table looks like this */ }").is_none());
}

/// Every source either compiles or comes back with a reason. Never silence.
///
/// This replaces the test that pinned "the compiler is unfinished, so every
/// source reports not-implemented". That stopped being true the hour the chain
/// closed, and the test failing is how the change announced itself, which is
/// what it was for.
///
/// What has to keep holding is weaker and more durable: a source the compiler
/// cannot handle says *which* thing it cannot handle. The first run after the
/// front end was wired reported 302 sources with an EMPTY reason, because the
/// rejection carried no diagnostic — a table of 302 blanks that measured
/// nothing. This is the test that would have caught it.
#[test]
fn every_source_either_compiles_or_says_why() {
    let mut blank = 0usize;
    let mut compiled = 0usize;
    for s in SHC_SOURCE_TABLE.iter() {
        let Some(row) = row_for(s.text) else { continue };
        match compile_through_the_new_compiler(row.stage, s.text) {
            Attempt::Compiled(_) => compiled += 1,
            Attempt::Unavailable(why) if why.trim().is_empty() => blank += 1,
            Attempt::Unavailable(_) => {}
        }
    }
    assert_eq!(blank, 0, "a refusal with no reason measures nothing; {blank} sources gave one");
    println!("{compiled} of {} sources compile through the online path", SHC_SOURCE_TABLE.len());
}

/// Which sources compile today, by name.
///
/// Not a pinning test -- the list is expected to grow and this prints rather
/// than asserts. It exists because the report's "2 of 307" says how many and
/// never which, and "which" is what tells the next person whether the shader
/// they care about is in.
#[test]
fn name_the_sources_that_compile() {
    let mut ok = Vec::new();
    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        if let Attempt::Compiled(_) = compile_through_the_new_compiler(stage, e.text) {
            ok.push(e.name);
        }
    }
    println!("compiling sources ({}): {ok:#?}", ok.len());
}

/// Which ES 1.00 sources do NOT compile, grouped by the reason, with names.
///
/// The counterpart of the list above, and the one the split needs: the report
/// says "32 control flow, 2 OutOfRegisters" and never WHICH, so a reason with
/// two sources behind it cannot be looked at without running the corpus by
/// hand. A reason with a small count is usually the interesting one --
/// `OutOfRegisters(257)` on exactly two vertex shaders is not an allocator
/// limit, it is a shape, and finding out which two is the whole investigation.
///
/// Prints rather than asserts: `every_source_either_compiles_or_says_why`
/// already pins that a refusal has a reason, and this is for reading.
#[test]
fn name_the_sources_that_do_not_compile_by_reason() {
    use std::collections::BTreeMap;
    let mut by_reason: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for e in SHC_SOURCE_TABLE.iter() {
        let Some(row) = row_for(e.text) else { continue };
        if let Attempt::Unavailable(why) = compile_through_the_new_compiler(row.stage, e.text) {
            by_reason.entry(why.trim().to_string()).or_default().push(e.name);
        }
    }
    println!();
    for (why, names) in &by_reason {
        println!("  {:>4}  {why}", names.len());
        /* Every name for a small group, a sample for a large one: the point
         * is to be able to open the source, and twelve names is enough to
         * see a pattern. */
        for n in names.iter().take(12) {
            println!("          {n}");
        }
        if names.len() > 12 {
            println!("          ... and {} more", names.len() - 12);
        }
    }
}

/// The first decoded field on which two programs of the same shape disagree.
///
/// A whole-word diff says "these bytes differ", which sends a reader to a hex
/// dump. Naming the field turns it into a question someone can answer: it is
/// how PMODE was found.
#[cfg(not(feature = "gpu-vivante"))]
fn first_field_difference(_ours: &[u32], _theirs: &[u32]) -> Option<(&'static str, u32, u32)> {
    /* The USC's per-group comparison is tests/usc_limits.rs's skeleton gate. */
    None
}

/// The operations a program holds, by name, decoded with the build's GPU's
/// own decoder: the GC7000's instructions, or the USC's groups (each group's
/// operations joined, as `usc_limits` names them).
#[cfg(feature = "gpu-vivante")]
fn op_counts(code: &[u32]) -> Result<BTreeMap<&'static str, usize>, String> {
    use yakogl::shaderc::online::vivante::decode::for_each_instr;
    let mut m = BTreeMap::new();
    for_each_instr(code, |_, i| {
        *m.entry(i.op().name).or_insert(0) += 1;
        Ok(())
    })
    .map_err(|err| format!("{err:?}"))?;
    Ok(m)
}

#[cfg(not(feature = "gpu-vivante"))]
fn op_counts(code: &[u32]) -> Result<BTreeMap<&'static str, usize>, String> {
    use yakogl::shaderc::online::powervr::decode::decode_group;
    use yakogl::shaderc::online::powervr::isa::ops_for;
    let bytes: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut m = BTreeMap::new();
    let mut off = 0;
    while off < bytes.len() {
        let g = decode_group(&bytes, off).map_err(|e| format!("at {off:#x}: {e:?}"))?;
        if g.total == 0 {
            return Err(format!("a zero-length group at {off:#x}"));
        }
        for (_, ph, r) in g.instrs() {
            *m.entry(ops_for(r, ph).next().map(|o| o.name).unwrap_or("<unknown>")).or_insert(0) += 1;
        }
        let end = g.hdr.get("end") == Some(1);
        off += g.total;
        /* A table blob may hold further programs after the first ends. */
        if end {
            break;
        }
    }
    Ok(m)
}

#[cfg(feature = "gpu-vivante")]
fn first_field_difference(ours: &[u32], theirs: &[u32]) -> Option<(&'static str, u32, u32)> {
    use yakogl::shaderc::online::vivante::decode::for_each_instr;
    let fields = |code: &[u32]| {
        let mut v: Vec<(&'static str, u32)> = Vec::new();
        let _ = for_each_instr(code, |_, i| {
            for (k, f) in i.op().fields.iter().enumerate() {
                v.push((f.name, i.vals[k]));
            }
            Ok(())
        });
        v
    };
    let (a, b) = (fields(ours), fields(theirs));
    if a.len() != b.len() {
        return None;
    }
    a.iter().zip(&b).find(|((n, x), (m, y))| n == m && x != y).map(|((n, x), (_, y))| (*n, *x, *y))
}

/// The generated instructions against the table's, structurally.
///
/// The reflection comparison above says nothing about the code: a compiler
/// that emitted the right `ShcShader` around an empty instruction stream would
/// score perfectly on it. This decodes both sides with M1's decoder and
/// compares what the instructions *are* -- how many, and which operations.
///
/// **Byte equality is not the goal and is not checked.** Register allocation
/// and constant placement are ours to choose; two correct compilers disagree
/// on them routinely. What must not differ quietly is the shape: a missing
/// instruction, or a `mul` where the table has a `dp4`, is a different program.
///
/// Prints rather than asserts, for the same reason the reflection table does:
/// with two sources compiling, a pass/fail is one bit, and "which operation,
/// how many" is the next hour's work.
#[test]
fn generated_code_against_the_table_structurally() {
    let mut rows = Vec::new();
    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let Attempt::Compiled(ours) = compile_through_the_new_compiler(stage, e.text) else { continue };
        let Some(row) = row_for(e.text) else { continue };

        let counts = op_counts;
        let theirs_code = row.shader.usc_words();
        let ours_code = ours.usc_words();
        /* Byte equality is not required -- register allocation and constant
         * placement are ours -- but when it happens it is the strongest
         * evidence available that the code generator agrees with the compiler
         * that shipped, so it is worth printing. When it ALMOST happens, the
         * differing field is named: that is what turned "the draw produces no
         * fragments" into a one-field answer. */
        let identical = theirs_code == ours_code;
        let field_diff = if identical { None } else { first_field_difference(ours_code, theirs_code) };
        rows.push((
            e.name,
            counts(ours_code),
            counts(theirs_code),
            ours_code.len() / 4,
            theirs_code.len() / 4,
            identical,
            field_diff,
        ));
    }

    let mut o = String::new();
    let _ = writeln!(o, "\n=== online shaderc: generated code against the table ===\n");
    if rows.is_empty() {
        let _ = writeln!(o, "  no source both compiles and has a blob row yet");
    }
    for (name, ours, theirs, n_ours, n_theirs, identical, field_diff) in &rows {
        /* A blob of four zero dwords decodes as one `nop`. 15 of the table's
         * rows are exactly that -- full reflection, no instructions -- so a
         * difference against one of them says nothing about our compiler.
         * Reported to shaderc-remote, whose table it is; labelled here so
         * nobody reads a "difference" off a row that has no program in it. */
        let placeholder = theirs.as_ref().is_ok_and(|t| t.len() == 1 && t.get("nop") == Some(&1));
        if placeholder {
            let _ = writeln!(o, "  {name}: {n_ours} instructions, TABLE ROW IS A PLACEHOLDER (all-zero code)");
            continue;
        }
        let _ = writeln!(
            o,
            "  {name}: {n_ours} instructions, table {n_theirs}{}",
            if *identical { "  IDENTICAL, word for word" } else { "" }
        );
        if let Some((field, a, b)) = field_diff {
            let _ = writeln!(o, "      same instructions, one field differs: {field}: ours {a}, table {b}");
        }
        match (ours, theirs) {
            (Ok(a), Ok(b)) => {
                if a == b {
                    let _ = writeln!(o, "      same operations: {a:?}");
                } else {
                    let _ = writeln!(o, "      ours  {a:?}");
                    let _ = writeln!(o, "      table {b:?}");
                }
            }
            (a, b) => {
                /* A decode failure on OUR side is a real defect: the encoder
                 * produced something its own decoder cannot read. On the
                 * table's side it is news about the corpus. Either way, say
                 * which side. */
                if let Err(e) = a {
                    let _ = writeln!(o, "      ours DID NOT DECODE: {e}");
                }
                if let Err(e) = b {
                    let _ = writeln!(o, "      table did not decode: {e}");
                }
            }
        }
    }
    println!("{o}");

    for (name, ours, ..) in &rows {
        assert!(ours.is_ok(), "{name}: our own encoder produced code our own decoder cannot read");
    }
}
