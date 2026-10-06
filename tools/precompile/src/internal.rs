// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `--internal`: the programs a back end binds itself, made by our compiler.
//!
//! The list's `internal` lines for this build's GPU, in list order: each
//! source compiled alone through the product back end (`SHC_ONLINE`) as the
//! table's rows are, and for a fragment shader the variant its SPEC names
//! asked of the plain record; each `attach:N` the clear-attach program our
//! encoder builds (`powervr::assemble::build_clear_attach`). The rows are
//! printed as the precompiled table prints them ([`table::row_text`]), into
//! one file the back end compiles in -- src/pvr/internal.rs or
//! src/vivante/internal.rs -- which is committed, because every image needs
//! these programs whether it carries a compiler or a table or neither, and
//! tools/check.sh makes it again and compares.
//!
//! No record key and no commit in the file: nothing looks these rows up, and
//! a mark that moved with every commit would make the committed file stale
//! on the next one. What makes it is named instead: the list's lines and the
//! compiler's version.

use crate::drive;
use crate::list::{Gpu, InternalFrom, List};
use crate::shc_print::{emit_u32_array, rs_comment};
use crate::table::{row_text, Origin, Row, Source, Step};
use yakogl::shaderc::SHC_STAGE_FRAGMENT;

/// The GPU this build compiles for, as the list's `internal` lines name it.
const LIST_GPU: Gpu = if cfg!(feature = "gpu-vivante") { Gpu::Gc7000 } else { Gpu::Powervr };
/// The prefix of the file's names: `PVR_INTERNAL_...`, `GC_INTERNAL_...`.
const PREFIX: &str = if cfg!(feature = "gpu-vivante") { "GC" } else { "PVR" };

/// One program of the file.
enum Made {
    /* boxed: a row is large beside a word list */
    Row { name: String, spec: Option<String>, row: Box<Row> },
    Words { name: String, dwords: u32, words: Vec<u32> },
}

/// The clear-attach program of `dwords` output dwords, as words.
#[cfg(not(feature = "gpu-vivante"))]
fn attach(dwords: u32) -> Result<Vec<u32>, String> {
    let mut buf = [0u8; 256];
    let n = yakogl::shaderc::online::powervr::assemble::build_clear_attach(dwords as u16, &mut buf)
        .map_err(|e| format!("attach:{dwords}: {e:?}"))?;
    Ok(buf[..n].chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

#[cfg(feature = "gpu-vivante")]
fn attach(dwords: u32) -> Result<Vec<u32>, String> {
    Err(format!("attach:{dwords}: the PowerVR's, not this GPU's"))
}

/// The row of the record behind `sh`, copied while it is live.
fn row_of(sh: &yakogl::shaderc::ShcShader, source: usize, origin: (Origin, Step)) -> Result<Row, String> {
    let mut row = None;
    yakogl::shaderc::online::backend::each_record(|r| {
        if core::ptr::eq(r.shader.priv_, sh.priv_) {
            row = Some(Row::of(r, source, origin.clone()));
        }
    });
    let mut row = row.ok_or("a shader the compiler handed out has no record in its cache")?;
    row.sensitivity = drive::sensitivity(sh);
    Ok(row)
}

/// The file for this GPU's `internal` lines of `list`.
pub fn generate(list: &List, list_name: &str) -> Result<String, String> {
    let mut sources: Vec<Source> = Vec::new();
    let mut made: Vec<Made> = Vec::new();
    for i in list.internal.iter().filter(|i| i.gpu == LIST_GPU) {
        match &i.from {
            InternalFrom::Attach(n) => {
                made.push(Made::Words { name: i.name.clone(), dwords: *n, words: attach(*n)? });
            }
            InternalFrom::Source { path, blend_key, spec } => {
                let text = std::fs::read_to_string(list.abs(path)).map_err(|e| format!("{path}: {e}"))?;
                let source = match sources.iter().position(|s| &s.name == path) {
                    Some(k) => k,
                    None => {
                        sources.push(Source { name: path.clone(), text: text.clone() });
                        sources.len() - 1
                    }
                };
                let stage = drive::stage_of(path);
                let mut h = drive::Holders::default();
                let r = (|| {
                    let sh = drive::compile(&mut h, stage, &text).map_err(|m| format!("{path}: refused: {m}"))?;
                    let origin = Origin::Alone { src: path.clone() };
                    if *blend_key == 0 {
                        return row_of(&sh, source, (origin, Step::Compile));
                    }
                    if stage != SHC_STAGE_FRAGMENT {
                        return Err(format!("{path}: a variant of a vertex shader"));
                    }
                    let v = drive::variant_unlinked(&mut h, &sh, *blend_key)
                        .map_err(|m| format!("{path} {}: refused: {m}", spec.as_deref().unwrap_or("")))?;
                    row_of(&v, source, (origin, Step::Variant(*blend_key)))
                })();
                h.release_all()?;
                made.push(Made::Row { name: i.name.clone(), spec: spec.clone(), row: Box::new(r?) });
            }
        }
    }
    /* two lines asking for one program would print its statics twice */
    for (k, m) in made.iter().enumerate() {
        if let Made::Row { name, row, .. } = m {
            let twin = made[..k].iter().find_map(|o| match o {
                Made::Row { name: other, row: r, .. } if r.ident() == row.ident() => Some(other),
                _ => None,
            });
            if let Some(other) = twin {
                return Err(format!("internal {other} and {name} are one program ({}): list it once", row.ident()));
            }
        }
    }
    if made.is_empty() {
        return Err(format!(
            "{list_name}: no `internal {}` lines",
            if LIST_GPU == Gpu::Gc7000 { "gc7000" } else { "powervr" }
        ));
    }
    emit(&made, &sources, list_name)
}

fn emit(made: &[Made], sources: &[Source], list_name: &str) -> Result<String, String> {
    let gpu = if LIST_GPU == Gpu::Gc7000 { "gc7000" } else { "powervr" };
    let rows: Vec<&Made> = made.iter().filter(|m| matches!(m, Made::Row { .. })).collect();
    let mut out: Vec<String> = vec![
        "// SPDX-License-Identifier: Apache-2.0".into(),
        "//".into(),
        format!("// GENERATED by `yakogl-precompile --internal` (tools/precompile) from the `internal {gpu}`"),
        format!("// lines of {list_name}. DO NOT EDIT: tools/check.sh makes it again and compares."),
        "//".into(),
        "// The programs this back end binds itself, every one made by YakoGL's own shader compiler".into(),
        "// (src/shaderc/online) on the host through the product back end's entry points, a source".into(),
        "// compiled alone and a fragment shader's variant asked of its plain record; or, for".into(),
        "// `attach:N`, built by its USC encoder. Mesa's programs for the same jobs are host-test".into(),
        "// reference material (tests/reference/), which tests/internal_programs.rs runs against these.".into(),
        "//".into(),
        format!("// toolchain: yakogl={} gpu={gpu}", env!("CARGO_PKG_VERSION")),
        "//".into(),
        "// name, source, variant, row:".into(),
    ];
    for m in made {
        match m {
            Made::Row { name, spec, row } => out.push(rs_comment(&format!(
                "//   {name}: {} {} {}",
                sources[row.source].name,
                spec.as_deref().unwrap_or("plain"),
                row.ident()
            ))),
            Made::Words { name, dwords, words } => {
                out.push(format!("//   {name}: attach:{dwords} ({} words)", words.len()))
            }
        }
    }
    out.extend(
        [
            "",
            "#![allow(clippy::all)]",
            "#![cfg_attr(rustfmt, rustfmt_skip)]",
            "",
            "#[allow(unused_imports)]",
            "use crate::shaderc::{",
            "    pad_attribs, pad_varyings, ShcAttrib, ShcAttribBinding, ShcAttribMap, ShcBlob, ShcShader, ShcUniform,",
            "    ShcVarying, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,",
            "};",
        ]
        .map(String::from),
    );
    #[cfg(feature = "gpu-vivante")]
    out.extend(
        ["#[allow(unused_imports)]", "use crate::shaderc::{pad_io, ShcViv, ShcVivIo, SHC_VIV_MAX_IO};"]
            .map(String::from),
    );
    out.extend(["#[allow(unused_imports)]", "use crate::util::cname;", ""].map(String::from));
    let mut entries = Vec::new();
    for m in &rows {
        if let Made::Row { row, .. } = m {
            let (st, e) = row_text(row, sources)?;
            out.extend(st);
            entries.push(e);
        }
    }
    out.push(String::new());
    out.push(format!(
        "/// The programs, in the list's order; [`{PREFIX}_INTERNAL_{}`] and its",
        upper(first_row(made))
    ));
    out.push("/// neighbours below index it by name.".into());
    out.push(format!("pub static {PREFIX}_INTERNAL_TABLE: [ShcBlob; {}] = [", entries.len()));
    for e in entries {
        out.extend(e);
    }
    out.push("];".into());
    out.push(String::new());
    let mut k = 0usize;
    for m in made {
        match m {
            Made::Row { name, spec, row } => {
                out.push(format!(
                    "/// `{name}`: {}{}.",
                    sources[row.source].name,
                    spec.as_ref().map_or(String::new(), |s| format!(", variant {s} (blend key 0x{:x})", row.blend_key))
                ));
                out.push(format!("pub const {PREFIX}_INTERNAL_{}: usize = {k};", upper(name)));
                k += 1;
            }
            Made::Words { name, dwords, words } => {
                out.push(format!("/// `{name}`: the clear-attach program of {dwords} dword(s): shared register k"));
                out.push("/// moved to pixel output k, the last move ending the program.".into());
                let mut a = Vec::new();
                emit_u32_array(&mut a, &format!("{PREFIX}_INTERNAL_{}", upper(name)), words);
                if let Some(first) = a.first_mut() {
                    *first = format!("pub {first}");
                }
                out.extend(a);
            }
        }
    }
    out.push(String::new());
    Ok(out.join("\n"))
}

fn first_row(made: &[Made]) -> &str {
    made.iter()
        .find_map(|m| match m {
            Made::Row { name, .. } => Some(name.as_str()),
            Made::Words { .. } => None,
        })
        .unwrap_or("")
}

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}
