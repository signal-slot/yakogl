// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools usc-blob`: compile one internal GLSL shader with pco_shc and
//! (re)write its USC dwords in `tests/reference/pvr_internal.rs` (Mesa's
//! programs for the jobs the backend does itself, the reference the
//! backend's own -- our compiler's, src/pvr/internal.rs -- are run against).
//!
//! The block of SYMBOL (the "GENERATED from" comment lines, SYMBOL_TEMPS,
//! SYMBOL_DWORDS and the SYMBOL static; see the header of
//! tests/reference/pvr_internal.rs)
//! is replaced in place and the rest of the file is left alone. With
//! `--after OTHER` a missing block is added after OTHER's block.

use crate::glsl::exit;
use crate::glsl2usc::{parse_pco_meta, run_pco, split_rt, stage_of, Shader};
use crate::py;
use crate::{Error, R};

const USAGE: &str =
    "usage: yakogl-tools usc-blob <in.frag|in.vert> <tests/reference/pvr_internal.rs> <SYMBOL> [VARIANT]
                              [--after SYMBOL] [--title TEXT]
  VARIANT is a glsl2usc variant spec ('replace@f16x4': the colour output packed
  for an RGBA16F target). --after adds the block after SYMBOL's when the file
  has none yet; --title is the new block's section heading.
  e.g. yakogl-tools usc-blob tools/shaders/internal/fbload.frag tests/reference/pvr_internal.rs PVR_USC_FBLOAD";

/// The generated block text (the format documented in tests/reference/pvr_internal.rs).
pub fn render(sym: &str, rel_src: &str, sh: &Shader, usc: &[u32]) -> String {
    let descs: Vec<String> = sh.descs.iter().map(|d| d.repr()).collect();
    let mut lines = vec![
        format!(" * GENERATED from {rel_src}."),
        format!(" * temps={} shareds(descriptors)=[{}] uses_w={} */", sh.temps, descs.join(", "), sh.uses_w),
        format!("pub const {sym}_TEMPS: u32 = {};", sh.temps),
        format!("pub const {sym}_DWORDS: usize = {};", usc.len()),
        "#[rustfmt::skip]".to_string(),
        format!("pub static {sym}: [u32; {sym}_DWORDS] = ["),
    ];
    let mut line = String::from("    ");
    for dw in usc {
        let tok = format!("0x{dw:08x},");
        if line.len() + tok.len() + 1 > 96 {
            lines.push(line.trim_end().to_string());
            line = "    ".into();
        }
        line.push_str(&tok);
        line.push(' ');
    }
    if !py::strip(&line).is_empty() {
        lines.push(line.trim_end().to_string());
    }
    lines.push("];".into());
    lines.join("\n")
}

fn block_re(sym: &str) -> regex::Regex {
    py::re(&format!(
        concat!(
            r"(?s) \* GENERATED from [^\n]*\n \* temps=[^\n]*\*/\n",
            r"pub const {s}_TEMPS: u32 = \d+;\n",
            r"pub const {s}_DWORDS: usize = \d+;\n",
            r"#\[rustfmt::skip\]\n",
            r"pub static {s}: \[u32; {s}_DWORDS\] = \[\n.*?\n\];"
        ),
        s = py::esc(sym)
    ))
}

/// Replace SYMBOL's block in `text`, or add it after `after`'s block.
pub fn update(text: &str, sym: &str, block: &str, after: Option<(&str, &str)>) -> R<String> {
    let re = block_re(sym);
    let n = re.find_iter(text).count();
    if n == 1 {
        return Ok(py::sub_lit(&re, text, block));
    }
    match (n, after) {
        (0, Some((other, title))) => {
            let ore = block_re(other);
            let found: Vec<_> = ore.find_iter(text).collect();
            if found.len() != 1 {
                exit!("usc-blob: no (single) generated block for {other} to add {sym} after");
            }
            let at = found[0].end();
            let mut head = format!("/* ---- {title} ");
            while head.chars().count() < 79 {
                head.push('-');
            }
            Ok(format!("{}\n\n{}\n{}{}", &text[..at], head, block, &text[at..]))
        }
        _ => exit!("usc-blob: no (single) generated block for {sym}"),
    }
}

pub fn main(argv: &[String]) -> R<()> {
    let mut pos = Vec::new();
    let mut after = None;
    let mut title = None;
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--after" => after = Some(it.next().ok_or_else(|| Error::Usage(USAGE.into()))?.to_uppercase()),
            "--title" => title = Some(it.next().ok_or_else(|| Error::Usage(USAGE.into()))?.clone()),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => pos.push(a.clone()),
        }
    }
    if pos.len() != 3 && pos.len() != 4 {
        return Err(Error::Usage(USAGE.into()));
    }
    let (src, out) = (&pos[0], &pos[1]);
    let sym = pos[2].to_uppercase();
    let variant = pos.get(3).cloned().unwrap_or_default();
    let stage = stage_of(src)?;
    split_rt(&variant)?;
    let (usc, meta) = run_pco(src, stage, &variant)?;
    let sh = parse_pco_meta(&meta, stage)?;
    let rel_src = py::relpath(src, &crate::repo_root());
    let text = py::read_text(out)?;
    let block = render(&sym, &rel_src, &sh, &usc);
    let title = title.unwrap_or_else(|| {
        let v = if variant.is_empty() { String::new() } else { format!(" {variant}") };
        format!("{}{}", py::basename(src), v)
    });
    let new = update(&text, &sym, &block, after.as_deref().map(|a| (a, title.as_str())))?;
    py::write_text(out, &new)?;
    eprintln!("usc-blob: updated {} in {} ({} dwords, temps {})", sym, out, usc.len(), sh.temps);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glsl2usc::parse_pco_meta;

    const FILE: &str = "//! header
/* ---- a.frag: first ------------------------------------------------------------
 * GENERATED from tools/shaders/internal/a.frag.
 * temps=1 shareds(descriptors)=[] uses_w=0 */
pub const A_TEMPS: u32 = 1;
pub const A_DWORDS: usize = 2;
#[rustfmt::skip]
pub static A: [u32; A_DWORDS] = [
    0x00000001, 0x00000002,
];

/* ---- hand written */
pub const B: u32 = 3;
";

    #[test]
    fn replace_in_place_and_add() {
        let sh = parse_pco_meta("temps=5\ndesc[set=0 binding=0]: sh.start=0 count=20 stride=20\n", "fs").unwrap();
        let usc: Vec<u32> = (0..9).collect();
        let block = render("A", "tools/shaders/internal/a.frag", &sh, &usc);
        let t = update(FILE, "A", &block, None).unwrap();
        assert!(t.contains(
            " * temps=5 shareds(descriptors)=[{'set': 0, 'binding': 0, 'sh_start': 0, 'count': 20, 'stride': 20}] uses_w=0 */\n\
             pub const A_TEMPS: u32 = 5;\npub const A_DWORDS: usize = 9;"
        ));
        // 7 tokens of 11 characters fit the 96-column line
        assert!(t.contains(
            "    0x00000000, 0x00000001, 0x00000002, 0x00000003, 0x00000004, 0x00000005, 0x00000006,\n    0x00000007, 0x00000008,\n];"
        ));
        assert!(
            t.starts_with("//! header\n/* ---- a.frag")
                && t.ends_with("/* ---- hand written */\npub const B: u32 = 3;\n")
        );
        // a missing symbol is an error unless --after names where it goes
        assert!(update(FILE, "C", &render("C", "x", &sh, &usc), None).is_err());
        let t2 = update(FILE, "C", &render("C", "x", &sh, &usc), Some(("A", "c.frag @f16: new"))).unwrap();
        let head = "\n];\n\n/* ---- c.frag @f16: new ";
        let at = t2.find(head).unwrap();
        let line = t2[at + 5..].lines().next().unwrap();
        assert_eq!(line.len(), 79);
        assert!(t2.contains("pub static C: [u32; C_DWORDS] = ["));
        // and then it is replaced in place like any other
        let t3 = update(&t2, "C", &render("C", "y", &sh, &usc[..1]), None).unwrap();
        assert!(t3.contains(" * GENERATED from y.") && !t3.contains(" * GENERATED from x."));
    }
}
