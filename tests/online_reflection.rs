// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! M2's acceptance test: the online front end's reflection must equal the
//! offline table's rows, field by field, for every GLSL ES 1.00 source in the
//! table.
//!
//! Those rows are not an internal detail. `glGetActiveUniform`,
//! `glGetActiveAttrib` and `glGetAttribLocation` answer straight out of them,
//! and their *order* is the index an application enumerates by, so a
//! reflection that differs from the table is a behaviour change for every
//! shader in the set. The table was built by a host pipeline
//! (`tools/vivante/gen_shader_table.py` for the GC7000, `yakogl-tools
//! glsl2usc` for the PowerVR); this checks that a front end which never sees
//! that pipeline lands on the same numbers.
//!
//! The table compiled into a build is whichever GPU it is for, so this test
//! measures that one. Sources that are not ES 1.00 are counted and skipped:
//! the table also holds GLSL ES 3.00 and desktop 4.50 shaders, which are a
//! named follow-on, and a test that silently passed them would be measuring
//! nothing.
//!
//! # What "equal" can mean, and what it cannot
//!
//! Three of the four row kinds in the table are post-dead-code. The block
//! uniforms are not: `gen_shader_table.py`'s `block_uniforms` reads the
//! declarations, so every declared one has a row and the comparison is
//! equality, in order, field for field. But a **sampler**, an **attribute**
//! and a **varying** reach the table only if the code generator kept them --
//! they are register wiring, and an unused one gets no register. A front end
//! cannot know what a later pass will eliminate.
//!
//! So the claim this test makes is the strongest one a front end can make:
//! *every row the table has, the front end has, with identical fields and in
//! the same order.* Rows the front end has and the table does not are
//! declared-but-eliminated, and they are counted and printed rather than
//! quietly accepted -- the count is what would notice if elimination ever
//! started removing something it should not. It is also exactly the contract
//! M3 needs: the back end filters this list by what it kept, and a name it
//! keeps must be one the front end numbered.
// The corpus lives in the offline table, so this measures nothing in a build
// that carries none (`no-shader-table`, the image configuration).
#![cfg(not(feature = "no-shader-table"))]
#![cfg(feature = "shaderc-online")]

use yakogl::shaderc::online::{arena::Arena, parser, pp, reflect};
use yakogl::shaderc::sha256::sha256;
use yakogl::shaderc::{mesa_reference as table, ShcBlob};
use yakogl::util::name_bytes;

/// Scratch for one compile. The largest source in the table is a few
/// kilobytes; this is generous on purpose, so that "out of scratch" can never
/// be mistaken for a reflection mismatch.
const SCRATCH: usize = 4 << 20;

/// Type names GLSL ES 1.00 does not have, which arrived in ES 3.00.
///
/// Five sources in the tables declare `#version 100` and then use one --
/// `sampler2DArray` in every case so far. The offline pipeline never
/// noticed: `translate_es100_gl` rewrites the text without checking types and
/// hands it to glslang as GLSL 4.60, where the type exists. A front end that
/// applies ES 1.00's rules refuses them, which is correct, so they are
/// counted under their own heading with the offending token printed. It is a
/// rule rather than a list of shader names, so the evidence is the token the
/// front end refused and a new one is reported rather than predicted.
const LATER_VERSION_TYPES: [&str; 9] = [
    "sampler2DArray",
    "sampler2DArrayShadow",
    "sampler3D",
    "sampler2DShadow",
    "samplerCubeShadow",
    "uint",
    "uvec2",
    "uvec3",
    "uvec4",
];

/// Does this blob's own attribute list name one attribute twice?
///
/// Such a table row cannot be agreed with, because it disagrees with itself:
/// `glGetActiveAttrib` would name that attribute at two indices and never
/// name the one it displaced. `qt3d/qtappshaders_1.vert` is the case in the
/// GC7000 table -- it declares `attr_pos, attr_norm, attr_uv0, attr_textan,
/// attr_binormal` and its third row reads
/// `location: 0, components: 0, name: "attr_pos"`, with no row for
/// `attr_uv0` at all.
///
/// The mechanism is `gen_shader_table.py`'s attribute loop: when the
/// compiler's slot does not map to a declared location it falls back with
/// `loc = e["slot"]` and carries on, so an input it cannot place becomes a
/// row pointing at whatever location that number happens to be. It has no
/// outcome for "this attribute was eliminated" -- no representation for
/// absence, so a value indistinguishable from a real one is substituted.
///
/// Detected by rule rather than listed by name, so a new one is reported
/// instead of needing to be predicted, and a fixed one stops being counted
/// on its own.
fn table_names_an_attribute_twice(b: &ShcBlob) -> bool {
    let names: Vec<String> = b.shader.attribs[..b.shader.num_attribs as usize].iter().map(|x| cname(&x.name)).collect();
    let mut sorted = names.clone();
    sorted.sort();
    let before = sorted.len();
    sorted.dedup();
    sorted.len() != before
}

/// Is this GLSL ES 1.00? A source with no `#version` is, by section 3.3.
fn is_es100(text: &str) -> bool {
    match text.find("#version") {
        None => true,
        Some(i) => text[i..].trim_start_matches("#version").trim_start().starts_with("100"),
    }
}

fn cname(field: &[core::ffi::c_char]) -> String {
    String::from_utf8_lossy(name_bytes(field)).into_owned()
}

/// The part of a table name after the last '/'; for reporting only.
fn basename(name: &str) -> &str {
    match name.rfind('/') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// One uniform row as the fields that must match.
type Row = (String, u16, u16, u16, u8, u8, u8);

/// Every element of `want` appears in `got`, in the same order and equal in
/// every field.
///
/// This is the shape of the claim a front end can make about a row kind the
/// code generator prunes: nothing the table kept may be missing or different,
/// and what the front end has beyond it was eliminated later.
fn contains_in_order<T: PartialEq + core::fmt::Debug>(got: &[T], want: &[T]) -> Result<(), String> {
    let mut it = got.iter();
    for w in want.iter() {
        loop {
            match it.next() {
                Some(g) if g == w => break,
                Some(_) => continue,
                None => {
                    return Err(format!(
                        "the table's {w:?} is missing or out of order\n      got  {got:?}\n      want {want:?}"
                    ))
                }
            }
        }
    }
    Ok(())
}

/// Mesa's `gl_varying_slot::VARYING_SLOT_VAR0`, which is what the GC7000
/// table stores in `ShcVivIo::slot` for the first user varying. The test
/// checks the assumption rather than trusting it: a slot below this would
/// mean the base is not 32, and it fails saying so instead of quietly
/// computing a negative location.
#[cfg(feature = "gpu-vivante")]
const VARYING_SLOT_VAR0: u8 = 32;

/// The varyings the table records for a shader, as (location, name).
///
/// The two tables keep them in different places, and the difference is the
/// GPU's: the PowerVR backend reads `ShcVarying` rows, while on the GC7000
/// the varyings are register wiring and live in `ShcViv` (`src/shaderc/mod.rs`
/// says `varyings`/`vs_output_size` stay unused there). Reading the empty
/// `ShcVarying` array on a GC7000 build and calling that agreement would be
/// measuring nothing.
#[cfg(not(feature = "gpu-vivante"))]
fn table_varyings(b: &ShcBlob) -> Result<Vec<(u8, String)>, String> {
    Ok(b.shader.varyings[..b.shader.num_varyings as usize].iter().map(|x| (x.location, cname(&x.name))).collect())
}

#[cfg(feature = "gpu-vivante")]
fn table_varyings(b: &ShcBlob) -> Result<Vec<(u8, String)>, String> {
    let viv = &b.shader.viv;
    let (io, n) =
        if b.stage == yakogl::shaderc::SHC_STAGE_FRAGMENT { (&viv.in_, viv.num_in) } else { (&viv.out, viv.num_out) };
    let mut out = Vec::new();
    for x in io[..n as usize].iter() {
        /* A vertex shader's outputs include gl_Position and gl_PointSize,
         * which are below VAR0 and are not user varyings. */
        if x.slot < VARYING_SLOT_VAR0 {
            if b.stage == yakogl::shaderc::SHC_STAGE_FRAGMENT {
                return Err(format!("fragment input '{}' has slot {} below VAR0", cname(&x.name), x.slot));
            }
            continue;
        }
        out.push((x.slot - VARYING_SLOT_VAR0, cname(&x.name)));
    }
    Ok(out)
}

#[test]
fn the_reflection_equals_the_offline_tables_rows() {
    let mut scratch = vec![0u8; SCRATCH];
    let (mut checked, mut skipped_version, mut no_blob) = (0usize, 0usize, 0usize);
    /* Samplers the front end reports and the table does not: dead code the
     * code generator removed. Counted so the number is visible rather than
     * absorbed. */
    let mut dropped = 0usize;
    /* Attributes and varyings in the same position. */
    let mut dropped_io = 0usize;
    let mut table_defects = 0usize;
    let mut needs_extension = 0usize;
    let mut later_types = 0usize;
    let mut type_names: Vec<String> = Vec::new();
    let mut defect_names: Vec<String> = Vec::new();
    let mut ext_names: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for src in table::SHC_SOURCE_TABLE.iter() {
        let text = src.text.trim_end_matches('\0');
        if !is_es100(text) {
            skipped_version += 1;
            continue;
        }
        /* Keyed by the SHA-256 of the source text, which is what the table
         * itself is keyed by -- the same lookup `offline.rs` does at run
         * time. Matching on the name instead needs a heuristic (a blob name
         * may carry the directory it was found in) and the PowerVR table has
         * names that collide once the directory is dropped.
         *
         * The base entry, not a blend or render-target variant: they share a
         * source and their reflection is the same. */
        let key = sha256(text.as_bytes());
        let Some(blob) = table::SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) else {
            no_blob += 1;
            continue;
        };

        if table_names_an_attribute_twice(blob) {
            table_defects += 1;
            defect_names.push(basename(blob.source_name()).to_string());
            continue;
        }

        let a = Arena::new(&mut scratch);
        let got = match pp::preprocess(&a, text)
            .and_then(|pre| parser::parse(&a, pre.tokens))
            .and_then(|ast| reflect::reflect(&a, &ast))
        {
            Ok(r) => r,
            Err(e) if LATER_VERSION_TYPES.contains(&e.tok) => {
                /* Declares ES 1.00 and uses a type from a later version:
                 * refusing is correct, and the finding is about the table. */
                later_types += 1;
                type_names.push(format!("{} ({})", src.name, e.tok));
                continue;
            }
            Err(e) if e.msg == "extension is not supported" => {
                /* A correct refusal, and a finding about the table rather
                 * than the front end: `src/gl/state.rs` advertises no
                 * `GL_OES_standard_derivatives`, and its comment says every
                 * name there "is something this library really does". A
                 * shader that `require`s an extension the implementation does
                 * not advertise must fail to compile, so the table holds
                 * shaders this runtime cannot legitimately accept. */
                needs_extension += 1;
                ext_names.push(format!("{} ({})", src.name, e.tok));
                continue;
            }
            Err(e) => {
                failures.push(format!("{}: refused at line {} {:?}: {}", src.name, e.line, e.tok, e.msg));
                continue;
            }
        };
        checked += 1;

        /* The table drops a sampler the code generator's dead-code pass
         * removed, which a front end cannot know. Compare after the same
         * filter -- but never let a sampler the shader actually mentions go
         * missing, which would be a real disagreement rather than dead code. */
        let want: Vec<Row> = blob
            .shader
            .uniform_table()
            .iter()
            .map(|u| (cname(&u.name), u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding, u.inner))
            .collect();
        let got_u: Vec<Row> = got
            .uniforms
            .iter()
            .map(|u| (u.name.to_string(), u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding, u.inner))
            .collect();
        /* The block half has no dead-code pass behind it, so it is equality. */
        let want_block: Vec<&Row> = want.iter().filter(|w| w.1 != reflect::NO_BLOCK).collect();
        let got_block: Vec<&Row> = got_u.iter().filter(|w| w.1 != reflect::NO_BLOCK).collect();
        if got_block != want_block {
            failures.push(format!("{}: block uniforms\n      got  {got_block:?}\n      want {want_block:?}", src.name));
        }
        /* The sampler half is what survived compilation. */
        let want_s: Vec<&Row> = want.iter().filter(|w| w.1 == reflect::NO_BLOCK).collect();
        let got_s: Vec<&Row> = got_u.iter().filter(|w| w.1 == reflect::NO_BLOCK).collect();
        if let Err(why) = contains_in_order(&got_s, &want_s) {
            failures.push(format!("{}: samplers: {why}", src.name));
        }
        dropped += got_u
            .iter()
            .filter(|u| u.1 == reflect::NO_BLOCK)
            .count()
            .saturating_sub(want.iter().filter(|w| w.1 == reflect::NO_BLOCK).count());

        let want_a: Vec<(u8, String, u16)> = blob.shader.attribs[..blob.shader.num_attribs as usize]
            .iter()
            .map(|x| (x.location, cname(&x.name), x.gl_type))
            .collect();
        let got_a: Vec<(u8, String, u16)> =
            got.attribs.iter().map(|x| (x.location, x.name.to_string(), x.gl_type)).collect();
        if let Err(why) = contains_in_order(&got_a, &want_a) {
            failures.push(format!("{}: attributes: {why}", src.name));
        }
        dropped_io += got_a.len().saturating_sub(want_a.len());

        /* Only the location and the name are the front end's; the
         * interpolation and the register or vertex-output offset beside them
         * are the code generator's. */
        let mut want_v = match table_varyings(blob) {
            Ok(v) => v,
            Err(why) => {
                failures.push(format!("{}: {why}", src.name));
                continue;
            }
        };
        let mut got_v: Vec<(u8, String)> = got.varyings.iter().map(|x| (x.location, x.name.to_string())).collect();
        got_v.sort();
        /* The GC7000 table can list one varying more than once: `ShcVivIo` is
         * register wiring, and the generator emits a row per register the
         * compiler wired, not per declaration. Comparing the distinct
         * (location, name) pairs is the question the front end can answer. */
        want_v.sort();
        want_v.dedup();
        if let Err(why) = contains_in_order(&got_v, &want_v) {
            failures.push(format!("{}: varyings: {why}", src.name));
        }
        dropped_io += got_v.len().saturating_sub(want_v.len());
    }

    println!(
        "online reflection: {checked} ES 1.00 sources checked, {skipped_version} not ES 1.00, \
         {no_blob} with no base blob, {dropped} samplers and {dropped_io} attributes or \
         varyings eliminated by the code generator, {table_defects} with self-inconsistent table \
         rows, {needs_extension} requiring an unadvertised extension, {later_types} using a type \
         from a later version, {} mismatched",
        failures.len()
    );
    for n in defect_names.iter() {
        println!("  table names an attribute twice: {n}");
    }
    for n in ext_names.iter() {
        println!("  requires an extension this implementation does not advertise: {n}");
    }
    for n in type_names.iter() {
        println!("  declares ES 1.00 and uses a later version's type: {n}");
    }
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "{} of {checked} ES 1.00 sources disagree with the offline table", failures.len());
    assert!(checked > 0, "the table held no ES 1.00 source, so this test measured nothing");
}

/// The PowerVR target's hidden `float pvr_layers_<sampler>` block member:
/// for every used sampler2DArray and sampler3D, at the sampler's place in
/// the block, as glsl2usc puts it for pco. es300/array.frag equals the
/// table row for row (pco's block: pvr_layers_layers @0, layer @1); the Qt
/// Quick 3D sources that use qt_shadowmap_texture, and those that declare
/// it unused, agree with the table on where the member is and whether it is
/// there.
///
/// Revert check: drop the `pvr_layers_` block in `reflect::reflect` and
/// every one of these differs from the table at the hidden member.
#[cfg(not(feature = "gpu-vivante"))]
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn the_powervr_block_carries_pvr_layers_where_pco_does() {
    let mut scratch = vec![0u8; SCRATCH];
    let (mut with_member, mut without) = (Vec::new(), Vec::new());
    let mut failures = Vec::new();
    for src in table::SHC_SOURCE_TABLE.iter() {
        let text = src.text.trim_end_matches('\0');
        let wanted = (src.name.ends_with("array.frag") && text.contains("uniform sampler2DArray layers;"))
            || (text.contains("sampler2DArray qt_shadowmap_texture") && text.starts_with("#version 300 es"));
        if !wanted {
            continue;
        }
        let key = sha256(text.as_bytes());
        let Some(blob) = table::SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) else {
            continue;
        };
        let stage = if src.name.ends_with(".vert") {
            yakogl::shaderc::online::sema::Stage::Vertex
        } else {
            yakogl::shaderc::online::sema::Stage::Fragment
        };
        let a = Arena::new(&mut scratch);
        let got = match reflect::interface(&a, stage, text) {
            Ok(i) => i.reflection,
            Err(e) => {
                failures.push(format!("{}: refused: {}", src.name, e.msg));
                continue;
            }
        };
        /* Names and offsets: where the hidden member sits and what it moves.
         * (The rows' `dwords` for an ES 3.00 struct-array member differ
         * from pco's record in a way that has nothing to do with this
         * member, and are not what this test is about.) */
        let want: Vec<(String, u16)> = blob
            .shader
            .uniform_table()
            .iter()
            .filter(|u| u.dword_off != reflect::NO_BLOCK)
            .map(|u| (cname(&u.name), u.dword_off))
            .collect();
        let got: Vec<(String, u16)> = got
            .uniforms
            .iter()
            .filter(|u| u.dword_off != reflect::NO_BLOCK)
            .map(|u| (u.name.to_string(), u.dword_off))
            .collect();
        /* es300/array.frag has plain uniforms only, and must equal the
         * table row for row. The Qt Quick 3D sources lay a uniform array of
         * structs out element by element here and member by member in
         * pco's record (see RESULTS.md), which moves every later offset by
         * the same amount on both sides of the comparison; what is compared
         * there is the ORDER of the top-level declarations, the hidden
         * member included -- which is where it has to sit. A source that
         * declares the sampler and never reads it has no member in the
         * table, and must have none here either. */
        let top = |rows: &[(String, u16)]| -> Vec<String> {
            let mut v: Vec<String> = Vec::new();
            for (n, _) in rows {
                let t = n.split(['.', '[']).next().unwrap_or("").to_string();
                if v.last() != Some(&t) {
                    v.push(t);
                }
            }
            v
        };
        let equal = if src.name.ends_with("array.frag") { got == want } else { top(&got) == top(&want) };
        if !equal {
            failures.push(format!("{}:\n      got  {got:?}\n      want {want:?}", src.name));
        }
        if want.iter().any(|w| w.0.starts_with("pvr_layers_")) {
            with_member.push(src.name);
        } else {
            without.push(src.name);
        }
    }
    println!("pvr_layers_ member, equal to the table: {with_member:?}");
    println!("no member (sampler unused), equal to the table: {} sources", without.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(with_member.iter().any(|n| n.ends_with("array.frag")), "es300/array.frag was compared");
    assert!(with_member.len() >= 2, "a Qt Quick 3D source with qt_shadowmap_texture was compared too");
}
