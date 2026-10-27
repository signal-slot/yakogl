// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The back end against its references.
//!
//! * The byte reference: Mesa 26.2.1's GL driver on drm-shim
//!   (tools/mali/drm_shim_ref.sh), its programs extracted by
//!   `pan_ref_diff.py extract --draw` into tools/mali/testdata/*.expect.json:
//!   tri and tri_varying (the board's tri.c and tri_varying.c), and the
//!   triangle sample (tools/shaders/gl_test_es100/passthrough.vert and
//!   interp_color.frag, triangle_sample.expect.json, made the same way).
//!   Every region's words are compared one by one, a mismatch printed with
//!   both sides' disassembly; the facts (work registers, preload mask, the
//!   secondary offset, the FAU words) are compared too.
//! * The board: Mesa 25.0.7 (tools/mali/testdata/board-25.0.7/), which
//!   differs from 26.2.1 by the four causes README-pan_shc.md names. Each
//!   region's difference is pinned by its differing word count and digest,
//!   computed as pan_ref_diff.py computes them, so that a gap that closes
//!   and a new difference both fail.
//!
//! A score here says only that nothing these programs cover differs.

use super::fau::FauWord;
use super::isa;
use super::{MaliError, MaliShader, Section, VaryingSlot};
use crate::shaderc::online::codegen::{self, CompileError};
use crate::shaderc::online::store;
use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use std::fmt::Write as _;

/* ---- the references, as committed ---------------------------------- */

const TRI_VS: &str = include_str!("../../../../tools/mali/testdata/tri.vert");
const TRI_FS: &str = include_str!("../../../../tools/mali/testdata/tri.frag");
const TV_VS: &str = include_str!("../../../../tools/mali/testdata/tri_varying.vert");
const TV_FS: &str = include_str!("../../../../tools/mali/testdata/tri_varying.frag");
const SAMPLE_VS: &str = include_str!("../../../../tools/shaders/gl_test_es100/passthrough.vert");
const SAMPLE_FS: &str = include_str!("../../../../tools/shaders/gl_test_es100/interp_color.frag");
const TRI_26: &str = include_str!("../../../../tools/mali/testdata/tri.expect.json");
const TV_26: &str = include_str!("../../../../tools/mali/testdata/tri_varying.expect.json");
const SAMPLE_26: &str = include_str!("../../../../tools/mali/testdata/triangle_sample.expect.json");
const SAMPLE_96X64: &str = include_str!("../../../../tools/mali/testdata/triangle_sample_96x64.expect.json");
const TRI_25: &str = include_str!("../../../../tools/mali/testdata/board-25.0.7/tri.expect.json");
const TV_25: &str = include_str!("../../../../tools/mali/testdata/board-25.0.7/tri_varying.expect.json");

/// One region of an expect.json (pan_ref_diff.py's `extract` output).
#[derive(Debug, Default)]
pub(super) struct Region {
    pub words: Vec<u64>,
    pub work_reg_count: u8,
    pub preload: u64,
    /// None when the trace printed no FAU table for the region.
    pub fau: Option<Vec<u32>>,
}

/// The text after `"key":` in `s`.
fn after<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("\"{key}\":");
    s.find(&k).map(|i| s[i + k.len()..].trim_start())
}

/// The quoted hex strings of the array that starts `s`.
fn hex_array(s: &str) -> Option<Vec<u64>> {
    if s.starts_with("null") {
        return None;
    }
    let end = s.find(']')?;
    Some(s[1..end].split(',').filter_map(|x| u64::from_str_radix(x.trim().trim_matches('"'), 16).ok()).collect())
}

/// A region by name, or None when the expect.json has none.
pub(super) fn region(json: &str, name: &str) -> Option<Region> {
    let r = after(json, name)?;
    let end = r.find("\n  }").unwrap_or(r.len());
    let r = &r[..end];
    let num = |k: &str| after(r, k).map(|t| t.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or(""));
    Some(Region {
        words: hex_array(after(r, "words")?)?,
        work_reg_count: num("work_reg_count")?.parse().ok()?,
        preload: u64::from_str_radix(after(r, "preload")?.trim_matches(|c| c == '"').get(2..18)?, 16).ok()?,
        fau: hex_array(after(r, "fau")?).map(|v| v.into_iter().map(|w| w as u32).collect()),
    })
}

/// The top-level `secondary_offset`.
pub(super) fn secondary_offset(json: &str) -> Option<u32> {
    after(json, "secondary_offset")?.split(',').next()?.trim().parse().ok()
}

/* ---- our side ------------------------------------------------------- */

/// A compiled stage: its code as 64-bit words and its record.
pub(super) struct Stage {
    pub code: Vec<u64>,
    pub info: Info,
    /// The uniform rows of the record (name, first dword, dwords).
    pub uniforms: Vec<(String, u16, u16)>,
}

/// A copy of a [`MaliShader`] that owns its arrays: the record's own live
/// in a keep transaction that ends with the compile (it keeps nothing), so
/// the next compile reuses that memory.
#[derive(Clone, Debug)]
pub(super) struct Info {
    pub stage: u32,
    pub preload: u64,
    pub work_reg_count: u8,
    pub secondary: Option<super::Secondary>,
    pub no_psiz_offset: Option<u32>,
    pub fau: Vec<FauWord>,
    pub attributes: Vec<(u8, u8)>,
    pub varyings: Vec<VaryingSlot>,
    pub colour_f16: bool,
    pub compacted: bool,
    pub moved: Vec<super::motion::Moved>,
    pub sampler: Option<u8>,
    pub helper_threads: bool,
    pub can_discard: bool,
    pub ubo: Vec<super::UboRow>,
}

impl Info {
    fn of(r: &MaliShader<'_>) -> Info {
        Info {
            stage: r.stage,
            preload: r.preload,
            work_reg_count: r.work_reg_count,
            secondary: r.secondary,
            no_psiz_offset: r.no_psiz_offset,
            fau: r.fau.to_vec(),
            attributes: r.attributes.to_vec(),
            varyings: r.varyings.to_vec(),
            colour_f16: r.colour_f16,
            compacted: r.compacted,
            moved: r.moved.to_vec(),
            sampler: r.sampler,
            helper_threads: r.helper_threads,
            can_discard: r.can_discard,
            ubo: r.ubo.to_vec(),
        }
    }

    /// The record again, borrowing this copy's arrays.
    pub fn shader(&self) -> MaliShader<'_> {
        MaliShader {
            magic: super::MALI_RECORD_MAGIC,
            stage: self.stage,
            preload: self.preload,
            work_reg_count: self.work_reg_count,
            secondary: self.secondary,
            no_psiz_offset: self.no_psiz_offset,
            fau: &self.fau,
            attributes: &self.attributes,
            varyings: &self.varyings,
            colour_f16: self.colour_f16,
            compacted: self.compacted,
            moved: &self.moved,
            sampler: self.sampler,
            helper_threads: self.helper_threads,
            can_discard: self.can_discard,
            ubo: &self.ubo,
        }
    }
}

pub(super) fn compile(stage: u32, src: &str) -> Result<Stage, CompileError> {
    let _k = store::test_keep();
    store::with_scratch(|s| {
        codegen::compile_linked_blocks(stage, src.as_bytes(), 0, &[], s, store::keep()).map(|(sh, _)| stage_of(&sh))
    })
    .expect("the scratch arena is free")
}

/// A compiled record as the tests read it (its arrays copied out of keep).
fn stage_of(sh: &crate::shaderc::ShcShader) -> Stage {
    let dw = sh.usc_words();
    let code = dw.chunks(2).map(|c| c[0] as u64 | (c[1] as u64) << 32).collect();
    let uniforms = if sh.uniforms.is_null() {
        Vec::new()
    } else {
        // SAFETY: the record's rows live in keep, as long as `sh`.
        let rows = unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) };
        rows.iter()
            .map(|u| {
                let bytes: Vec<u8> = u.name.iter().map(|&b| b as u8).take_while(|&b| b != 0).collect();
                (String::from_utf8_lossy(&bytes).into_owned(), u.dword_off, u.dwords)
            })
            .collect()
    };
    Stage { code, info: Info::of(MaliShader::of(sh).expect("a Mali record")), uniforms }
}

/// A program's words without the zero padding (a program never ends in an
/// all-zero word, va_pack.c:1225-1239).
fn trim(w: &[u64]) -> Vec<u64> {
    let mut v = w.to_vec();
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

/// The regions of a vertex stage: position, then varying (empty when none).
fn vs_regions(s: &Stage) -> (Vec<u64>, Vec<u64>) {
    match s.info.secondary {
        Some(sec) => {
            let at = sec.offset as usize / 8;
            (trim(&s.code[..at]), trim(&s.code[at..]))
        }
        None => (trim(&s.code), Vec::new()),
    }
}

fn text(w: u64) -> String {
    let mut s = String::new();
    match isa::describe_word(w, &mut s) {
        Ok(()) => s,
        Err(e) => format!("{e:?}"),
    }
}

/// Byte-equal, word by word; a mismatch lists every differing word with
/// both disassemblies.
fn assert_words(name: &str, ours: &[u64], theirs: &[u64]) {
    if ours == theirs {
        return;
    }
    let mut m = format!("{name}: {} words ours, {} reference\n", ours.len(), theirs.len());
    for i in 0..ours.len().max(theirs.len()) {
        let (o, t) = (ours.get(i), theirs.get(i));
        let mark = if o == t { ' ' } else { '*' };
        let _ = writeln!(
            m,
            "{mark}[{i:2}] ours {:<18} {}\n      ref  {:<18} {}",
            o.map_or("(none)".into(), |w| format!("{w:016x}")),
            o.map_or(String::new(), |w| text(*w)),
            t.map_or("(none)".into(), |w| format!("{w:016x}")),
            t.map_or(String::new(), |w| text(*w)),
        );
    }
    panic!("{m}");
}

/// The viewport sysvals of a shim draw (`panfrost_upload_viewport_*_sysval`,
/// pan_cmdstream.c:989-1010), as fp32 bits.
#[derive(Clone, Copy)]
pub(super) struct Viewport {
    scale: [u32; 3],
    offset: [u32; 3],
}

/// gl_ref.py's default draw: a 64x64 framebuffer, the whole of it as the
/// viewport and the depth range 0,1, so the scale and offset are (32, 32,
/// 0.5) both (the trace's words read 0x42000000 and 0x3f000000). This
/// viewport cannot tell scale from offset, or x from y: [`VIEWPORT_96X64`]
/// can.
const VIEWPORT_64X64: Viewport =
    Viewport { scale: [0x4200_0000, 0x4200_0000, 0x3f00_0000], offset: [0x4200_0000, 0x4200_0000, 0x3f00_0000] };

/// triangle_sample_96x64.expect.json's draw (`--size 96x64 --viewport
/// 8,4,80,56 --depth-range 0.25,0.75`): scale (40, 28, 0.25), offset (48,
/// 32, 0.5), six distinct words.
const VIEWPORT_96X64: Viewport =
    Viewport { scale: [0x4220_0000, 0x41e0_0000, 0x3e80_0000], offset: [0x4240_0000, 0x4200_0000, 0x3f00_0000] };

/// The value of each FAU word in a shim draw with viewport `vp`, the blend
/// constants 0, padded with a zero word to whole 64-bit slots as the trace
/// prints them.
fn fau_values(f: &[FauWord], vp: Viewport, uv: &dyn Fn(u16) -> u32) -> Vec<u32> {
    use super::bir::Sysval;
    let mut v: Vec<u32> = f
        .iter()
        .map(|w| match *w {
            FauWord::Sysval(Sysval::ViewportScale, c) => vp.scale[c as usize],
            FauWord::Sysval(Sysval::ViewportOffset, c) => vp.offset[c as usize],
            FauWord::BlendConstant(_) => 0,
            FauWord::Constant(c) => c,
            FauWord::Uniform { dword } => uv(dword),
            FauWord::UniformHalves { lo, hi } => {
                let h = |d: Option<u16>| d.map_or(0, |d| crate::mali::frame::f32_to_f16(uv(d)) as u32);
                h(Some(lo)) | h(hi) << 16
            }
            /* gl_ref.py draws into a pbuffer, which Mesa does not flip, and
             * its viewport is the whole target (the offset's y its middle) */
            FauWord::FbWposYTransform(c) => {
                let height = (2.0 * f32::from_bits(vp.offset[1])) as u32;
                crate::mali::frame::fb_wpos_y_transform(false, height, usize::from(c)).to_bits()
            }
        })
        .collect();
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn assert_facts(name: &str, r: &Region, wrc: u8, preload: u64, fau: &[FauWord], vp: Viewport, uv: &dyn Fn(u16) -> u32) {
    assert_eq!(wrc, r.work_reg_count, "{name}: work_reg_count");
    assert_eq!(preload, r.preload, "{name}: preload");
    assert_eq!(Some(fau_values(fau, vp, uv)), r.fau, "{name}: FAU words");
}

/// The uniforms a program declares, in gl_ref.py's `uniforms()` order: the
/// vertex shader's, then the fragment shader's not already declared.
fn uniform_names(vs: &str, fs: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for text in [vs, fs] {
        let mut t = String::new();
        let mut rest = text;
        while let Some(a) = rest.find("/*") {
            t.push_str(&rest[..a]);
            rest = rest[a..].find("*/").map_or("", |b| &rest[a + b + 2..]);
        }
        t.push_str(rest);
        for stmt in t.split([';', '{', '}']) {
            let w: Vec<&str> = stmt.split_whitespace().collect();
            if w.first() == Some(&"uniform") {
                let name = w.last().expect("a name").to_string();
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out
}

/// The value gl_ref.py's `--uniform-values` gave the uniform dword `d` of
/// a stage (component c of uniform k: 16 (k + 1) + c), as fp32 bits.
fn uniform_value(names: &[String], rows: &[(String, u16, u16)], d: u16) -> u32 {
    let (name, first, _) = rows.iter().find(|r| d >= r.1 && d < r.1 + r.2).expect("a uniform row holds the dword");
    let k = names.iter().position(|n| n == name).expect("the uniform is declared");
    (16.0 * (k as f32 + 1.0) + (d - first) as f32).to_bits()
}

/// One program pair against an expect.json (26.2.1) of gl_ref.py's default
/// draw.
fn check_program(vs: &str, fs: &str, json: &str) -> (Stage, Stage) {
    check_program_in(vs, fs, json, VIEWPORT_64X64)
}

fn check_program_in(vs: &str, fs: &str, json: &str, vp: Viewport) -> (Stage, Stage) {
    let v = check_vs_in(vs, fs, json, vp);
    let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
    check_fs_stage(&f, vs, fs, json, vp);
    (v, f)
}

/// The fragment program `f` (of `fs`, in a program with `vs`) against an
/// expect.json: its words, facts and padding.
fn check_fs_stage(f: &Stage, vs: &str, fs: &str, json: &str, vp: Viewport) {
    let names = uniform_names(vs, fs);
    let uv_f = |d: u16| uniform_value(&names, &f.uniforms, d);
    let rf = region(json, "fs.fragment").expect("fs.fragment");
    assert_words("fs.fragment", &trim(&f.code), &rf.words);
    assert_facts("fs.fragment", &rf, f.info.work_reg_count, f.info.preload, &f.info.fau, vp, &uv_f);
    /* the padding: up to the next 128 bytes past 16 more */
    assert_eq!(f.code.len(), (rf.words.len() * 8 + 16).div_ceil(128) * 16);
}

/// The vertex stage of a program pair against an expect.json: both IDVS
/// programs, their facts and the secondary offset (`fs` names the
/// uniforms gl_ref.py set).
fn check_vs_in(vs: &str, fs: &str, json: &str, vp: Viewport) -> Stage {
    let v = compile(SHC_STAGE_VERTEX, vs).expect("the vertex shader compiles");
    check_vs_stage(&v, vs, fs, json, vp);
    v
}

/// [`check_vs_in`] of the compiled vertex stage `v`.
fn check_vs_stage(v: &Stage, vs: &str, fs: &str, json: &str, vp: Viewport) {
    let (pos, var) = vs_regions(v);
    let names = uniform_names(vs, fs);
    let uv_v = |d: u16| uniform_value(&names, &v.uniforms, d);
    let rp = region(json, "vs.position").expect("vs.position");
    assert_words("vs.position", &pos, &rp.words);
    assert_facts("vs.position", &rp, v.info.work_reg_count, v.info.preload, &v.info.fau, vp, &uv_v);
    match region(json, "vs.varying") {
        Some(rv) => {
            assert_words("vs.varying", &var, &rv.words);
            let sec = v.info.secondary.expect("a varying program");
            assert_facts("vs.varying", &rv, sec.work_reg_count, sec.preload, &v.info.fau, vp, &uv_v);
            assert_eq!(Some(sec.offset), secondary_offset(json), "secondary_offset");
        }
        None => {
            assert!(var.is_empty() && v.info.secondary.is_none(), "no varying program in the reference");
            assert_eq!(secondary_offset(json), None);
        }
    }
    assert_eq!(
        v.code.len(),
        (pos.len() * 8 + 16).div_ceil(128) * 16
            + if var.is_empty() { 0 } else { (var.len() * 8 + 16).div_ceil(128) * 16 }
    );
}

/// A program pair through the Mali's link step: the vertex shader's
/// constant lanes (`vs_constants`), the fragment shader compiled with them
/// (`compile_fs_for_vs`), the vertex shader built for its reads
/// (`compile_vs_for_reads`), as Mesa's link leaves the pair
/// (nir_opt_varyings).
fn compile_linked(vs: &str, fs: &str) -> (Stage, Stage) {
    let _k = store::test_keep();
    let consts = store::with_scratch(|s| super::vs_constants(vs.as_bytes(), s))
        .expect("the scratch arena is free")
        .expect("the vertex shader's constant lanes");
    let f = store::with_scratch(|s| {
        super::compile_fs_for_vs(fs.as_bytes(), consts, s, store::keep()).map(|(sh, _)| stage_of(&sh))
    })
    .expect("the scratch arena is free")
    .expect("the fragment shader compiles for the vertex shader");
    let pr = super::fragment_reads(&f.info.shader()).expect("the record's moved expressions encode");
    let v = store::with_scratch(|s| {
        super::compile_vs_for_reads(vs.as_bytes(), pr.reads(), pr.moved(), s, store::keep())
            .map(|(sh, _)| stage_of(&sh))
    })
    .expect("the scratch arena is free")
    .expect("the vertex shader compiles for the program");
    (v, f)
}

/// [`compile_linked`] checked against an expect.json, every region, and
/// each fragment read at its vertex output's offset (the link's rule).
fn check_linked(vs: &str, fs: &str, json: &str) -> (Stage, Stage) {
    let (v, f) = compile_linked(vs, fs);
    check_vs_stage(&v, vs, fs, json, VIEWPORT_64X64);
    check_fs_stage(&f, vs, fs, json, VIEWPORT_64X64);
    for s in &f.info.varyings {
        assert!(
            v.info.varyings.iter().any(|x| x.varying == s.varying && x.offset == s.offset && x.mask == s.mask),
            "{s:?} in {:?}",
            v.info.varyings
        );
    }
    (v, f)
}

/* ---- the byte reference --------------------------------------------- */

#[test]
fn tri_is_byte_equal_to_mesa_26_2_1() {
    let (v, f) = check_program(TRI_VS, TRI_FS, TRI_26);
    use super::bir::Sysval::{ViewportOffset as O, ViewportScale as S};
    /* pan_shc-out/tri.json names the words */
    assert_eq!(
        v.info.fau,
        [
            FauWord::Sysval(S, 0),
            FauWord::Sysval(O, 0),
            FauWord::Sysval(S, 1),
            FauWord::Sysval(O, 1),
            FauWord::Sysval(O, 2),
            FauWord::Constant(4),
            FauWord::Constant(8),
            FauWord::Constant(12),
        ]
    );
    assert_eq!(
        f.info.fau,
        [
            FauWord::BlendConstant(0),
            FauWord::BlendConstant(1),
            FauWord::BlendConstant(2),
            FauWord::BlendConstant(3),
            FauWord::Constant(0x3c00),
        ]
    );
    assert_eq!(v.info.attributes, [(0, 0)]);
    assert_eq!(
        v.info.varyings,
        [VaryingSlot { varying: None, comps: 4, section: Section::Position, offset: 0, mask: 0xf }]
    );
    assert!(f.info.varyings.is_empty() && f.info.colour_f16);
}

#[test]
fn tri_varying_is_byte_equal_to_mesa_26_2_1() {
    let (v, f) = check_program(TV_VS, TV_FS, TV_26);
    /* pan_shc-out/tri_varying.json: col at location 1 is attribute record
     * 1; the layout's generic slot VAR0 has 3 components at offset 0 */
    assert_eq!(v.info.attributes, [(0, 0), (1, 1)]);
    assert_eq!(
        v.info.varyings,
        [
            VaryingSlot { varying: None, comps: 4, section: Section::Position, offset: 0, mask: 0xf },
            VaryingSlot { varying: Some(0), comps: 3, section: Section::Generic, offset: 0, mask: 0x7 },
        ]
    );
    assert_eq!(
        f.info.varyings,
        [VaryingSlot { varying: Some(0), comps: 3, section: Section::Generic, offset: 0, mask: 0x7 }]
    );
    assert_eq!(f.info.fau.last(), Some(&FauWord::Constant(0x3c00_3c00)));
}

/// gl_test case 209 varyorder (#1386): one vertex shader, its fragment
/// shader declaring the varyings in the vertex order and in the reverse
/// one. Mesa's link gives the fragment shader the vertex shader's layout,
/// so both pairs are one program, byte for byte (the shim drew each twice,
/// 0dce654a8:tools/mali/board-ref/2026-10-23-varying-order/shim/: the four extracts
/// equal but for the trace's name). The reverse one is ours through the
/// back end's own compile and link (link.rs `VaryingOrder`); compiled alone
/// it is in its own order and is not Mesa's.
#[test]
fn a_pair_in_another_order_is_byte_equal_to_mesa_26_2_1() {
    use crate::shaderc::online::backend::{release, SHC_ONLINE};
    use crate::shaderc::ShcShader;
    const VS: &str = include_str!("../../../../tools/mali/testdata/varyorder.vert");
    const FWD: &str = include_str!("../../../../tools/mali/testdata/varyorder.frag");
    const REV: &str = include_str!("../../../../tools/mali/testdata/varyorder_rev.frag");
    const JSON: &str = include_str!("../../../../tools/mali/testdata/varyorder_rev.expect.json");
    /* the vertex shader's order: the stage alone is Mesa's */
    check_program(VS, FWD, JSON);
    let _serial = store::test_lock();
    let compile = |stage: u32, src: &str| {
        let text = format!("{src}\0");
        let mut sh = ShcShader::ZERO;
        // SAFETY: a NUL-terminated source, a real output, no log.
        let rc =
            unsafe { (SHC_ONLINE.compile.unwrap())(stage, text.as_ptr().cast(), &mut sh, core::ptr::null_mut(), 0) };
        assert_eq!(rc, 0, "compiles");
        sh
    };
    let (vs, plain) = (compile(SHC_STAGE_VERTEX, VS), compile(SHC_STAGE_FRAGMENT, REV));
    let mut fs = plain;
    crate::shaderc::online::backend::retain(&plain);
    // SAFETY: two shaders of this back end, no log.
    assert_eq!(unsafe { (SHC_ONLINE.link.unwrap())(&vs, &mut fs, core::ptr::null_mut(), 0) }, 0);
    let (f, alone) = (stage_of(&fs), stage_of(&plain));
    let rf = region(JSON, "fs.fragment").expect("fs.fragment");
    assert_words("fs.fragment", &trim(&f.code), &rf.words);
    assert_facts("fs.fragment", &rf, f.info.work_reg_count, f.info.preload, &f.info.fau, VIEWPORT_64X64, &|_| 0);
    assert_eq!(f.code.len(), (rf.words.len() * 8 + 16).div_ceil(128) * 16);
    assert_ne!(trim(&alone.code), rf.words, "compiled alone, its own order");
    let v = stage_of(&vs);
    let generic = |s: &Stage, loc: u8| {
        s.info.varyings.iter().find(|x| x.section == Section::Generic && x.varying == Some(loc)).map(|x| x.offset)
    };
    /* v_b, v_uv, v_a: the fragment locations 0, 1, 2 at the vertex locations 2, 1, 0's offsets */
    for (fl, vl) in [(0, 2), (1, 1), (2, 0)] {
        assert_eq!(generic(&f, fl), generic(&v, vl), "fragment location {fl}");
    }
    /* the board's Mesa 25.0.7 drew both orders with one program
     * (board-25.0.7/varyorder.expect.json, its first draw; the second
     * draw's three regions are the same words,
     * 0dce654a8:tools/mali/board-ref/2026-10-23-varying-order/README.md): ours
     * differs from it by causes the_board_s_25_0_7_programs_differ_by_the_
     * four_named_causes names, pinned by count and digest. The VS stores
     * (one STORE per output in 25.0.7, per word with SHADDX in 26.2.1:
     * 22 of 24/14 and 41 of 41/11); in the FS the colour's conversion
     * (V2F32_TO_V2F16 against FADD.f32 `.h0`, four words) and ATEST's place
     * (after the two MKVEC in 25.0.7, reading r1.h1), 7 of 17/17. The three
     * loads, whose offsets are what the order decides, are equal. */
    const BOARD: &str = include_str!("../../../../tools/mali/testdata/board-25.0.7/varyorder.expect.json");
    let (pos, var) = vs_regions(&v);
    let pin = [
        ("vs.position", pos, 22usize, "fbe547972810"),
        ("vs.varying", var, 41, "54d6d9047f31"),
        ("fs.fragment", trim(&f.code), 7, "a687d07593bb"),
    ];
    for (reg, ours, n, d) in pin {
        let board = region(BOARD, reg).expect("the board region");
        assert_eq!(digest(&ours, &board.words), (n, d.to_string()), "{reg}");
    }
    let board_fs = region(BOARD, "fs.fragment").expect("the board region");
    assert_eq!((f.info.work_reg_count, f.info.preload), (board_fs.work_reg_count, board_fs.preload));
    assert_eq!(trim(&f.code)[..3], board_fs.words[..3], "the three loads");
    for sh in [&fs, &plain, &vs] {
        release(sh);
    }
}

/// The triangle sample's vertex FAU, by name (the value check of a 64x64
/// draw cannot tell scale from offset or x from y; the 96x64 draw below
/// can): scale.x, offset.x, scale.y, offset.y paired by their FMAs, then
/// scale.z/offset.z, the clamp of 1/w and the store offsets.
fn assert_sample_vs_fau(fau: &[FauWord]) {
    use super::bir::Sysval::{ViewportOffset as O, ViewportScale as S};
    assert_eq!(
        fau,
        [
            FauWord::Sysval(S, 0),
            FauWord::Sysval(O, 0),
            FauWord::Sysval(S, 1),
            FauWord::Sysval(O, 1),
            FauWord::Sysval(S, 2),
            FauWord::Sysval(O, 2),
            FauWord::Constant(0xc700_0000),
            FauWord::Constant(0x4700_0000),
            FauWord::Constant(4),
            FauWord::Constant(8),
            FauWord::Constant(12),
        ]
    );
}

#[test]
fn the_triangle_sample_is_byte_equal_to_mesa_26_2_1() {
    let (v, _) = check_program(SAMPLE_VS, SAMPLE_FS, SAMPLE_26);
    assert_sample_vs_fau(&v.info.fau);
}

/// The same program drawn with a 96x64 framebuffer, viewport 8,4,80,56 and
/// depth range 0.25,0.75 (`drm_shim_ref.sh run ... --size 96x64 --viewport
/// 8,4,80,56 --depth-range 0.25,0.75`): the bytes are the 64x64 draw's,
/// and every sysval word has its own value, so a word that names the wrong
/// sysval or component fails the value check.
#[test]
fn the_triangle_sample_s_fau_words_are_named_by_a_draw_that_tells_them_apart() {
    let vp = VIEWPORT_96X64;
    let words = [vp.scale[0], vp.offset[0], vp.scale[1], vp.offset[1], vp.scale[2], vp.offset[2]];
    for (i, a) in words.iter().enumerate() {
        assert!(!words[i + 1..].contains(a), "the draw's sysval words are distinct");
    }
    let (v, _) = check_program_in(SAMPLE_VS, SAMPLE_FS, SAMPLE_96X64, vp);
    assert_sample_vs_fau(&v.info.fau);
    let rp = region(SAMPLE_96X64, "vs.position").expect("vs.position");
    assert_eq!(rp.fau.as_deref().map(|f| &f[..6]), Some(&words[..]), "the trace's six sysval words");
}

/// The population of the selector: every combination its code admits
/// (select.rs `plan_vs`: the position Xy01 or Whole, times no varying or a
/// varying of 1 to 4 components copying another attribute or, at the
/// position's own size, the position's attribute; the fragment side: the pairs
/// (Cc, Cc) with no varying, (Vc, Cc) for a float, (Vv, Cc) for a vec2,
/// (Vv, Vc) for a vec3, (Vv, Vv) for a vec4, since every lane of the
/// varying is read in place; the alpha is a constant unless the varying is
/// a vec4; a trailing `return` in either stage, and an unread varying in
/// the fragment shader). Each pair below was drawn once on the shim
/// (`drm_shim_ref.sh run`, Mesa 26.2.1, then `pan_ref_diff.py extract
/// --draw`); with tri, tri_varying and the triangle sample above it covers
/// every combination. A combination the code admits and no case here
/// draws is the class that reaches a board: add its case before taking it.
const POPULATION: [(&str, &str, &str, &str); 10] = [
    (
        "sel_return",
        include_str!("../../../../tools/mali/testdata/sel_return.vert"),
        include_str!("../../../../tools/mali/testdata/sel_return.frag"),
        include_str!("../../../../tools/mali/testdata/sel_return.expect.json"),
    ),
    (
        "sel_xy01_vec2_same",
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec2_same.vert"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec2_same.frag"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec2_same.expect.json"),
    ),
    (
        "sel_xy01_vec2_other",
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec2_other.vert"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec2_other.frag"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec2_other.expect.json"),
    ),
    (
        "sel_xy01_float",
        include_str!("../../../../tools/mali/testdata/sel_xy01_float.vert"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_float.frag"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_float.expect.json"),
    ),
    (
        "sel_xy01_vec4",
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec4.vert"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec4.frag"),
        include_str!("../../../../tools/mali/testdata/sel_xy01_vec4.expect.json"),
    ),
    (
        "sel_whole_none",
        include_str!("../../../../tools/mali/testdata/sel_whole_none.vert"),
        include_str!("../../../../tools/mali/testdata/sel_whole_none.frag"),
        include_str!("../../../../tools/mali/testdata/sel_whole_none.expect.json"),
    ),
    (
        "sel_whole_float",
        include_str!("../../../../tools/mali/testdata/sel_whole_float.vert"),
        include_str!("../../../../tools/mali/testdata/sel_whole_float.frag"),
        include_str!("../../../../tools/mali/testdata/sel_whole_float.expect.json"),
    ),
    (
        "sel_whole_vec2",
        include_str!("../../../../tools/mali/testdata/sel_whole_vec2.vert"),
        include_str!("../../../../tools/mali/testdata/sel_whole_vec2.frag"),
        include_str!("../../../../tools/mali/testdata/sel_whole_vec2.expect.json"),
    ),
    (
        "sel_whole_vec3_loc1",
        include_str!("../../../../tools/mali/testdata/sel_whole_vec3_loc1.vert"),
        include_str!("../../../../tools/mali/testdata/sel_whole_vec3_loc1.frag"),
        include_str!("../../../../tools/mali/testdata/sel_whole_vec3_loc1.expect.json"),
    ),
    (
        "sel_whole_vec4_same",
        include_str!("../../../../tools/mali/testdata/sel_whole_vec4_same.vert"),
        include_str!("../../../../tools/mali/testdata/sel_whole_vec4_same.frag"),
        include_str!("../../../../tools/mali/testdata/sel_whole_vec4_same.expect.json"),
    ),
];

#[test]
fn the_selector_s_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json) in POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program(vs, fs, json)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_default();
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), POPULATION.len(), failed.join("\n"));
}

/* ---- the board (Mesa 25.0.7) ---------------------------------------- */

/// pan_ref_diff.py's `word_digest` over the differing positions.
fn digest(ours: &[u64], theirs: &[u64]) -> (usize, String) {
    let mut s = String::new();
    let mut n = 0;
    for i in 0..ours.len().max(theirs.len()) {
        let (o, t) = (ours.get(i), theirs.get(i));
        if o == t {
            continue;
        }
        n += 1;
        let f = |w: Option<&u64>| w.map_or("(none)".to_string(), |w| format!("{w:016x}"));
        let _ = writeln!(s, "{i} {} {}", f(o), f(t));
    }
    let d = crate::shaderc::sha256::sha256(s.as_bytes());
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    (n, hex[..12].to_string())
}

/// Against the board's 25.0.7 programs, exactly the four named
/// differences (README-pan_shc.md, "Results"; the counts and digests are
/// the ones it prints for pan_shc, whose programs these are):
///
/// * the VS stores: one STORE.i128/.i96 in 25.0.7, per-word STORE.i32 with
///   SHADDX addresses in 26.2.1 (vs.position 16 of 19/7, vs.varying 11 of
///   11/3), which also moves the varying program to offset 256;
/// * the constants in the FAU (tri's FS `MOV u2.w0` for 0x3c00 against
///   `IADD_IMM`; the store offsets 4, 8, 12 as FAU words);
/// * ATEST's alpha (tri's FS: the fp32 constant 1.0 against the packed
///   half), with the previous cause 3 of 7/7;
/// * the mediump varying load (tri_varying's FS: LD_VAR f32 and a separate
///   conversion against LD_VAR f16), 8 of 8/6.
#[test]
fn the_board_s_25_0_7_programs_differ_by_the_four_named_causes() {
    let pin = [
        ("tri vs.position", TRI_VS, TRI_25, "vs.position", 16, "41ef03dff988"),
        ("tri fs.fragment", TRI_FS, TRI_25, "fs.fragment", 3, "8eb3c005a976"),
        ("tri_varying vs.position", TV_VS, TV_25, "vs.position", 16, "41ef03dff988"),
        ("tri_varying vs.varying", TV_VS, TV_25, "vs.varying", 11, "ec221f7f002c"),
        ("tri_varying fs.fragment", TV_FS, TV_25, "fs.fragment", 8, "a2e324f5d090"),
    ];
    for (name, src, json, reg, n, d) in pin {
        let stage = if reg.starts_with("vs") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let s = compile(stage, src).expect("compiles");
        let ours = match reg {
            "vs.position" => vs_regions(&s).0,
            "vs.varying" => vs_regions(&s).1,
            _ => trim(&s.code),
        };
        let board = region(json, reg).expect("the board region").words;
        assert_eq!(digest(&ours, &board), (n, d.to_string()), "{name}");
        /* the facts the four causes leave alone */
        let r = region(json, reg).unwrap();
        assert_eq!((s.info.work_reg_count, s.info.preload), (r.work_reg_count, r.preload), "{name}");
    }
    let v = compile(SHC_STAGE_VERTEX, TV_VS).unwrap();
    assert_eq!((v.info.secondary.unwrap().offset, secondary_offset(TV_25)), (256, Some(128)));
}

/* ---- the population ------------------------------------------------- */

/// The ISA ops the back end emits, derived from the corpus's programs:
/// a new op (or one gone) changes the count and fails here until the pin
/// is re-derived.
#[test]
fn the_ops_the_generator_uses_are_pinned() {
    let mut ops = std::collections::BTreeSet::new();
    let mut words = 0;
    for (st, src) in [
        (SHC_STAGE_VERTEX, TRI_VS),
        (SHC_STAGE_FRAGMENT, TRI_FS),
        (SHC_STAGE_VERTEX, TV_VS),
        (SHC_STAGE_FRAGMENT, TV_FS),
        (SHC_STAGE_VERTEX, SAMPLE_VS),
        (SHC_STAGE_FRAGMENT, SAMPLE_FS),
    ] {
        let s = compile(st, src).unwrap();
        for &w in s.code.iter().filter(|w| **w != 0) {
            ops.insert(isa::decode(w).expect("our word decodes").op.info().mnemonic);
            words += 1;
        }
    }
    assert_eq!(words, 19 + 7 + 19 + 11 + 8 + 24 + 15 + 7, "the corpus's instructions");
    assert_eq!(ops.len(), 18, "{ops:?}");
}

/// The ISA ops the uniform population adds to the corpus's, derived from
/// its programs: FADD_IMM and FMA.v2f16 (FADD.f32 the corpus has), pinned
/// by name, so a new op fails here until a reference has it.
#[test]
fn the_ops_the_uniform_population_adds_are_pinned() {
    let mut ops = std::collections::BTreeSet::new();
    for (_, vs, fs, _) in UNIFORM_POPULATION {
        for (st, src) in [(SHC_STAGE_VERTEX, vs), (SHC_STAGE_FRAGMENT, fs)] {
            for &w in compile(st, src).unwrap().code.iter().filter(|w| **w != 0) {
                ops.insert(isa::decode(w).expect("our word decodes").op.info().mnemonic);
            }
        }
    }
    let mut corpus = std::collections::BTreeSet::new();
    for (st, src) in [(SHC_STAGE_VERTEX, TRI_VS), (SHC_STAGE_FRAGMENT, TRI_FS), (SHC_STAGE_VERTEX, TV_VS)]
        .into_iter()
        .chain([(SHC_STAGE_FRAGMENT, TV_FS), (SHC_STAGE_VERTEX, SAMPLE_VS), (SHC_STAGE_FRAGMENT, SAMPLE_FS)])
    {
        for &w in compile(st, src).unwrap().code.iter().filter(|w| **w != 0) {
            corpus.insert(isa::decode(w).expect("our word decodes").op.info().mnemonic);
        }
    }
    let new: Vec<_> = ops.difference(&corpus).collect();
    assert_eq!(new, [&"FADD_IMM.f32", &"FMA.v2f16"], "{ops:?}");
}

/* ---- the gaps ------------------------------------------------------- */

fn gap(stage: u32, src: &str) -> MaliError {
    match compile(stage, src) {
        Err(CompileError::BackEnd(codegen::BackEndError::Mali(e))) => e,
        Err(e) => panic!("{src}: not a Mali gap: {e:?}"),
        Ok(_) => panic!("{src}: compiled"),
    }
}

/// The Mali back end's verdict on every shader source under tools/shaders
/// (`.vert` and `.frag`, recursively), one line each, sorted by path:
/// `<path>: ok`, `<path>: <the gap>`, or the front end's refusal.
fn corpus_verdict_lines() -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).expect("a readable directory").flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "vert" || x == "frag") {
                out.push(p);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("tools/shaders"), &mut files);
    files.sort();
    let mut lines = Vec::new();
    for f in files {
        let Ok(src) = std::fs::read_to_string(&f) else { continue };
        let stage = if f.extension().is_some_and(|x| x == "vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let verdict = match compile(stage, &src) {
            Ok(_) => "ok".to_string(),
            Err(CompileError::BackEnd(codegen::BackEndError::Mali(e))) => e.message().to_string(),
            Err(e) => format!("front end: {e:?}"),
        };
        lines.push(format!("{}: {verdict}", f.strip_prefix(root).unwrap_or(&f).display()));
    }
    lines
}

/// The corpus's verdicts are the ones tools/mali/corpus-verdicts.txt pins:
/// a change that lowers a shape, or refuses one it took, moves lines there
/// in the same commit (`tools/mali/corpus-verdicts.sh --write`). A source
/// added to or removed from tools/shaders is not compared until the file is
/// written again. With
/// YAKOGL_CORPUS_PRINT set it prints the verdicts (`corpus: <line>`) and
/// compares nothing: what the script reads.
#[test]
fn the_corpus_verdicts_are_pinned() {
    let lines = corpus_verdict_lines();
    if std::env::var_os("YAKOGL_CORPUS_PRINT").is_some() {
        for l in &lines {
            println!("corpus: {l}");
        }
        return;
    }
    let pinned: Vec<&str> =
        include_str!("../../../../tools/mali/corpus-verdicts.txt").lines().filter(|l| !l.starts_with('#')).collect();
    let now: Vec<&str> = lines.iter().map(String::as_str).collect();
    /* only the sources on both sides: a source added to tools/shaders or
     * removed from it is not a verdict that moved */
    let path = |l: &str| l.split(": ").next().unwrap_or("").to_string();
    let (pinned_paths, now_paths): (Vec<_>, Vec<_>) =
        (pinned.iter().map(|l| path(l)).collect(), now.iter().map(|l| path(l)).collect());
    let gone: Vec<_> = pinned.iter().filter(|l| now_paths.contains(&path(l)) && !now.contains(l)).collect();
    let new: Vec<_> = now.iter().filter(|l| pinned_paths.contains(&path(l)) && !pinned.contains(l)).collect();
    assert!(
        gone.is_empty() && new.is_empty(),
        "the corpus verdicts moved (tools/mali/corpus-verdicts.sh --write):\n- {}\n+ {}",
        gone.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n- "),
        new.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n+ ")
    );
}

#[test]
fn every_gap_has_its_own_message_of_the_not_lowered_class() {
    for (i, a) in MaliError::GAPS.iter().enumerate() {
        assert!(a.message().ends_with("is not lowered yet"), "{a:?}");
        assert_eq!(format!("{a:?}"), a.message());
        for b in &MaliError::GAPS[i + 1..] {
            assert_ne!(a.message(), b.message());
        }
    }
}

/// Why a gap cannot be reached from GLSL ES 1.00 source today, or None
/// for a gap a test below reaches. The match is exhaustive: a new gap does
/// not compile until it is put on one side.
fn unreachable_from_es100(e: MaliError) -> Option<&'static str> {
    use MaliError as E;
    match e {
        E::IndexedLocal => Some(
            "a dynamic index is an int value, refused before the access (IntValue for a constant or a \
             loop counter, Uniform, Arithmetic); a constant index the front end resolves to a plain local",
        ),
        E::FragOutput => Some(
            "ES 1.00 has no fragment output but gl_FragColor and gl_FragData[0] (both FragColor(0)); the \
             front end refuses gl_FragData[n > 0] (the test pins its verdict), and a vertex shader writes none",
        ),
        E::Interpolation => Some("ES 1.00 has no interpolation qualifier: every varying is smooth"),
        E::FauFull => Some(
            "the sysvals are at most 6 words (VS) and 4 (FS) of 128, pushed first; uniforms that do not fit are \
             UboFallback",
        ),
        E::Spill | E::TooManyConstants | E::InstrTableFull | E::ValueTableFull | E::BlockTableFull => None,
        E::IfForm => Some(
            "the front end makes every if condition a comparison's value or a bool value it refuses first \
             (IntValue for a constant, Arithmetic for a boolean operator, UniformType for a bool uniform)",
        ),
        E::LinkedVertexNotLowered
        | E::BlendVariantNotLowered
        | E::ControlFlow
        | E::Arithmetic
        | E::IntValue
        | E::UniformType
        | E::UniformArray
        | E::UniformIndexed
        | E::UniformPrecision
        | E::FsUniform
        | E::MixedPrecision
        | E::UboFallback
        | E::ArithConstant
        | E::Rebalance
        | E::ConstantFold
        | E::FusedMultiplyAdd
        | E::Invariant
        | E::FauPage
        | E::TexVertex
        | E::TexVertexConstant
        | E::TexKind
        | E::TexCount
        | E::TexCoord
        | E::TexCoordShared
        | E::TexLanes
        | E::TexHighp
        | E::DiscardForm
        | E::LoopForm
        | E::SpecialInput
        | E::NoPosition
        | E::PositionLane
        | E::VaryingLocation
        | E::VaryingForm
        | E::VaryingNotFromAttribute
        | E::VaryingDuplicate
        | E::FragmentTooLarge
        | E::FsOperandForm
        | E::NoColour
        | E::ColourForm
        | E::ColourPair
        | E::ColourZeroHalf
        | E::Fp16Inexact
        | E::TooManyLocals
        | E::TooManyValues
        | E::CompareForm
        | E::AlgebraicForm
        | E::MovedTooLarge
        | E::FragCoordMediump => None,
        E::OutOfScratch | E::Defect(_) => Some("not a gap"),
    }
}

/// The ES 1.00 sources that reach each reachable gap, one or more per gap.
fn gap_cases() -> Vec<(u32, String, MaliError)> {
    const FS: u32 = SHC_STAGE_FRAGMENT;
    const VS: u32 = SHC_STAGE_VERTEX;
    let locals: String =
        (0..33).map(|i| format!("vec4 a{i} = {}; ", if i == 0 { "p".into() } else { format!("a{}", i - 1) })).collect();
    let varyings: String = (0..17).map(|i| format!("varying float v{i}; ")).collect();
    let mut v: Vec<(u32, String, MaliError)> = [
        (VS, "attribute vec4 p; void main(){ gl_Position = abs(p); }", MaliError::Arithmetic),
        /* a constant times a product with a constant: GLSL IR folds the
         * two (reassociate_constant) */
        (VS, "attribute vec2 p; attribute float q; void main(){ gl_Position = vec4(3.0 * (p * 0.75), 0.0, 1.0); }", MaliError::ArithConstant),
        /* fneg(fneg(a)) is fcanonicalize(a) */
        (VS, "attribute vec2 p; void main(){ gl_Position = vec4(-(p * -1.0), 0.0, 1.0); }", MaliError::ArithConstant),
        (VS, "uniform mat2 m; attribute vec2 p; void main(){ gl_Position = vec4(m * p, 0.0, 1.0); }", MaliError::UniformType),
        (VS, "uniform vec3 u[2]; void main(){ gl_Position = vec4(u[1], 1.0); }", MaliError::UniformArray),
        (VS, "uniform mediump vec4 u; void main(){ gl_Position = u; }", MaliError::UniformPrecision),
        (VS, "attribute vec2 p; void main(){ gl_Position = vec4(p, 0.0, 0.0); }", MaliError::ConstantFold),
        /* reflect(i, 0): the dot and its scale fold, the fma stays */
        (VS, "attribute vec4 p; void main(){ gl_Position = reflect(p, vec4(0.0)); }", MaliError::FusedMultiplyAdd),
        (VS, "invariant gl_Position; attribute vec4 p; void main(){ gl_Position = p; }", MaliError::Invariant),
        (VS, "attribute vec2 p; void main(){ gl_Position.xy = p; }", MaliError::PositionLane),
        (FS, "precision mediump float; uniform vec4 u; void main(){ gl_FragColor = u.yxzw; }", MaliError::FsUniform),
        (FS, "precision mediump float; uniform highp vec4 u; void main(){ gl_FragColor = u; }", MaliError::FsUniform),
        (FS, "precision mediump float; uniform mat4 u; void main(){ gl_FragColor = u[1]; }", MaliError::FsUniform),
        /* two uniform words in one product: a uniform first in a product
         * is a place no reference draws */
        (FS, "precision mediump float; uniform vec4 u; uniform vec4 w; void main(){ gl_FragColor = u * w; }", MaliError::FsOperandForm),
        /* a run-time index into an array that does not start the block
         * (its byte offset would add the base), and a second such read */
        (FS, "precision mediump float; uniform int i; uniform vec4 u[2]; void main(){ gl_FragColor = u[i]; }", MaliError::UniformIndexed),
        (FS, "precision mediump float; uniform vec4 u[2]; uniform int i; uniform int j; void main(){ gl_FragColor = u[i] + u[j]; }", MaliError::UniformIndexed),
        /* a mediump varying of a highp shader is an fp16 load; a highp
         * varying's sum in a mediump shader is fp32; a mediump local of a
         * highp shader is fp16 */
        (FS, "precision highp float; varying mediump vec4 v; void main(){ gl_FragColor = v; }", MaliError::MixedPrecision),
        (FS, "precision mediump float; varying highp vec4 a; varying highp vec4 b; void main(){ gl_FragColor = a + b; }", MaliError::MixedPrecision),
        (FS, "precision highp float; varying vec4 v; void main(){ mediump vec4 t = v; gl_FragColor = t; }", MaliError::MixedPrecision),
        (VS, "attribute vec4 p; void main(){ if (p.x > 0.0) gl_Position = p; }", MaliError::Arithmetic),
        (VS, "attribute vec4 p; void main(){ while (true) { gl_Position = p; break; } }", MaliError::ControlFlow),
        (VS, "attribute vec4 p; void main(){ for (int i = 0; i < 2; i++) { gl_Position = p; } }", MaliError::IntValue),
        (VS, "attribute vec4 p; void main(){ }", MaliError::NoPosition),
        (VS, "attribute vec4 p; varying vec4 v; void main(){ v = p; }", MaliError::NoPosition),
        (VS, "attribute vec4 p; void main(){ gl_Position = p; gl_PointSize = 2.3; }", MaliError::Fp16Inexact),
        (VS, "attribute vec4 p; varying vec4 v; void main(){ gl_Position = p; v.xy = p.xy; }", MaliError::VaryingForm),
        (VS, "attribute vec4 p; attribute vec2 c; varying vec4 v; void main(){ gl_Position = p; v = vec4(c, 0.0, 1.0); }", MaliError::VaryingNotFromAttribute),
        (VS, "attribute vec4 p; varying vec2 v; void main(){ gl_Position = p; v = p.xx; }", MaliError::VaryingDuplicate),
        (FS, "precision mediump float; varying float f; varying vec4 v; void main(){ gl_FragColor = f + v; }", MaliError::FsOperandForm),
        (FS, "precision mediump float; void main(){ int i = 1; gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }", MaliError::IntValue),
        (VS, "uniform sampler2D s; uniform sampler2D t; attribute vec4 p; varying vec4 v; void main(){ gl_Position = p; v = texture2DLod(s, p.xy, 0.0) * texture2DLod(t, p.xy, 0.0); }", MaliError::TexVertex),
        /* a cube look-up at a direction of constants alone (case 101's
         * tools/shaders/es100/cubelod.vert: `vec3(1.0, 0.0, 0.0)`; one
         * constant lane is an operand, case114_v) */
        (VS, "uniform samplerCube s; attribute vec4 p; varying vec4 v; void main(){ gl_Position = p; v = textureCubeLod(s, vec3(0.0, 0.0, 1.0), 0.0) * p; }", MaliError::TexVertexConstant),
        (FS, "precision mediump float; uniform sampler2D s; varying vec2 v; void main(){ gl_FragColor = texture2D(s, v, 1.0); }", MaliError::TexKind),
        (FS, "precision mediump float; uniform sampler2D s; varying vec3 v; void main(){ gl_FragColor = texture2DProj(s, v); }", MaliError::TexKind),
        (FS, "precision mediump float; uniform samplerCube s; varying vec3 v; void main(){ gl_FragColor = textureCube(s, v); }", MaliError::TexKind),
        (FS, "precision mediump float; uniform sampler2D s; uniform sampler2D t; varying vec2 v; void main(){ gl_FragColor = texture2D(s, v) * texture2D(t, v); }", MaliError::TexCount),
        /* a look-up in a loop after one made before it, whatever the loop's
         * form (femtovg's filter_image: a run-time break) */
        (FS, "precision highp float; uniform sampler2D s; uniform float u; varying vec2 v; void main(){ vec4 c = texture2D(s, v); for (float i = 0.0; i < 4.0; i += 1.0) { if (i >= u) break; c += texture2D(s, v * i); } gl_FragColor = c; }", MaliError::TexCount),
        (FS, "precision mediump float; uniform sampler2D s; varying vec2 v; void main(){ gl_FragColor = texture2D(s, v.yx); }", MaliError::TexCoord),
        /* a computed coordinate (a constant one is lowered: tex_coord_const) */
        (FS, "precision mediump float; uniform sampler2D s; varying vec2 v; void main(){ gl_FragColor = texture2D(s, v * 2.0); }", MaliError::TexCoord),
        (FS, "precision mediump float; uniform sampler2D s; varying vec4 v; void main(){ gl_FragColor = texture2D(s, v.xy); }", MaliError::TexCoord),
        (FS, "precision mediump float; uniform sampler2D s; varying vec2 v; void main(){ gl_FragColor = vec4(texture2D(s, v).xy, v); }", MaliError::TexCoordShared),
        (FS, "precision mediump float; uniform sampler2D s; varying vec2 v; void main(){ gl_FragColor = texture2D(s, v).yxzw; }", MaliError::TexLanes),
        (FS, "precision mediump float; uniform highp sampler2D s; varying vec2 v; void main(){ gl_FragColor = texture2D(s, v); }", MaliError::TexHighp),
        /* the discard forms no reference draws: unconditional (here a
         * mediump shader's, whose `if (a < b) discard;` is refused at its
         * compare first); in a block with another statement; behind an
         * integer compare; a side of nothing but a discard under another
         * `if` (the GL front end folds the two conditions into one `iand`) */
        (FS, "precision mediump float; void main(){ discard; }", MaliError::DiscardForm),
        (FS, "precision highp float; varying vec4 v; void main(){ vec4 c = v; if (v.x < 0.5) { c = v.yyyy; discard; } gl_FragColor = c; }", MaliError::DiscardForm),
        (FS, "precision highp float; varying vec4 v; void main(){ if (int(v.x) < 2) discard; gl_FragColor = v; }", MaliError::DiscardForm),
        (FS, "precision highp float; varying vec4 v; uniform float u; void main(){ bool b = v.y < u; if (v.x < 0.5) { if (b) discard; } gl_FragColor = v; }", MaliError::DiscardForm),
        /* a loop whose break hides behind a run-time condition (femtovg's
         * filter_image: Mesa's complex_unroll), and one of more than 32
         * passes */
        (FS, "precision highp float; varying vec4 v; uniform float u; void main(){ vec4 c = v; for (float i = 0.0; i < 8.0; i += 1.0) { if (i >= u) break; if (i >= v.x) break; c = c + v; } gl_FragColor = c; }", MaliError::LoopForm),
        (FS, "precision highp float; varying vec4 v; void main(){ vec4 c = v; for (int i = 0; i < 40; i++) c = c * v; gl_FragColor = c; }", MaliError::LoopForm),
        /* gl_FragCoord's z and w; any read of it in a mediump shader and
         * arithmetic on the built-in itself in a highp one (both fp16 in
         * Mesa: gl_FragCoord is mediump in ES 1.00) */
        (FS, "precision highp float; void main(){ gl_FragColor = vec4(gl_FragCoord.zw, 0.0, 1.0); }", MaliError::SpecialInput),
        (FS, "precision mediump float; void main(){ gl_FragColor = gl_FragCoord; }", MaliError::FragCoordMediump),
        (FS, "precision highp float; void main(){ gl_FragColor = vec4(floor(gl_FragCoord.xy), 0.0, 1.0); }", MaliError::FragCoordMediump),
        (FS, "precision mediump float; void main(){ }", MaliError::NoColour),
        (FS, "precision mediump float; void main(){ gl_FragColor = vec4(0.1, 0.0, 0.0, 1.0); }", MaliError::Fp16Inexact),
        (FS, "precision mediump float; varying vec4 v; void main(){ gl_FragColor = v.xzyw; }", MaliError::ColourForm),
        (FS, "precision mediump float; varying vec4 v; void main(){ gl_FragColor = v.yxzw; }", MaliError::FsOperandForm),
        (FS, "precision mediump float; varying vec2 v; void main(){ gl_FragColor = vec4(1.0, v, 1.0); }", MaliError::ColourPair),
        (FS, "precision mediump float; varying float v; void main(){ gl_FragColor = vec4(v, 0.0, 0.0, 1.0); }", MaliError::ColourZeroHalf),
        /* the S4 rows no reference draws: `!=` on floats and on integers;
         * the clamp forms Mesa's late rules make; a negated float(bool) in
         * a product */
        (FS, "precision highp float; varying vec4 v; void main(){ float r = float(v.x != v.y); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }", MaliError::CompareForm),
        (FS, "precision highp float; varying vec4 v; void main(){ float r = float(int(v.x) != int(v.y)); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }", MaliError::CompareForm),
        (FS, "precision highp float; varying vec4 v; void main(){ float r = clamp(v.x, -1.0, 1.0); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }", MaliError::AlgebraicForm),
        (FS, "precision highp float; varying vec4 v; void main(){ float r = -float(v.x > 0.5) * v.y; gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }", MaliError::AlgebraicForm),
        /* a chain of more than 8 operations that Mesa's link would move */
        (FS, "precision highp float; varying vec4 v; void main(){ float r = abs(((((v.x * 2.0 + 1.0) * 2.0 + 1.0) * 2.0 + 1.0) * 2.0 + 1.0) * 2.0 + 1.0); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }", MaliError::MovedTooLarge),
    ]
    .into_iter()
    .map(|(st, src, e)| (st, src.to_string(), e))
    .collect();
    v.push((VS, format!("attribute vec4 p; void main(){{ {locals} gl_Position = a32; }}"), MaliError::TooManyLocals));
    v.push((
        VS,
        format!("attribute vec4 p; void main(){{ vec4 a = p; {} gl_Position = a; }}", "a = a; ".repeat(200)),
        MaliError::TooManyValues,
    ));
    v.push((
        VS,
        format!("attribute vec4 p; attribute float q; {varyings} void main(){{ v16 = q; gl_Position = p; }}"),
        MaliError::VaryingLocation,
    ));
    /* more ifs than the graph's blocks hold (three each, bir.rs MAX_BLOCKS) */
    let ifs: String = (0..11).map(|i| format!("if (v.x > {i}.0) r = r * v.y; ")).collect();
    v.push((
        FS,
        format!("precision highp float; varying vec4 v; void main(){{ float r = v.z; {ifs} gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }}"),
        MaliError::BlockTableFull,
    ));
    /* the references of the FAU's limits (tools/mali/testdata) */
    v.push((VS, SEL_U_FAU65.to_string(), MaliError::FauPage));
    v.push((VS, SEL_U_FAU130.to_string(), MaliError::UboFallback));
    v.push((VS, SEL_U_REBALANCE.to_string(), MaliError::Rebalance));
    v.extend(stress_cases());
    v
}

/// Every reachable gap is reached by its own ES 1.00 source, and the rest
/// say why they cannot be (`unreachable_from_es100`). The counts are
/// pinned: a new gap, or one that becomes reachable, fails here until it
/// has a case.
#[test]
fn every_reachable_gap_is_reached_by_an_es100_source() {
    let mut reached = std::collections::BTreeSet::new();
    for (st, src, want) in gap_cases() {
        let got = gap(st, &src);
        assert_eq!(got, want, "{src}");
        reached.insert(want.message());
    }
    /* two gaps are reached through the entry points, not a source alone */
    reached.insert(a_blend_variant_is_refused_gap().message());
    reached.insert(a_linked_vertex_shader_is_refused_gap().message());
    let reachable: Vec<_> = MaliError::GAPS.iter().filter(|e| unreachable_from_es100(**e).is_none()).collect();
    let unreachable = MaliError::GAPS.len() - reachable.len();
    for e in &reachable {
        assert!(reached.contains(e.message()), "no case reaches {e:?}");
    }
    assert_eq!(reached.len(), reachable.len(), "a case reaches a gap classified unreachable");
    assert_eq!((MaliError::GAPS.len(), reachable.len(), unreachable), (58, 53, 5), "gaps: all, reachable, unreachable");
}

/// The front end's verdict on gl_FragData[1] (the reason FragOutput is
/// unreachable from ES 1.00 source).
#[test]
fn gl_frag_data_past_0_is_the_front_end_s_verdict() {
    let e = compile(SHC_STAGE_FRAGMENT, "precision mediump float; void main(){ gl_FragData[1] = vec4(1.0); }");
    match e {
        Err(CompileError::Rejected { msg, .. }) => {
            assert_eq!(msg, "OpenGL ES 2.0 has one draw buffer, so only gl_FragData[0] exists")
        }
        Err(e) => panic!("not the front end's verdict: {e:?}"),
        Ok(_) => panic!("gl_FragData[1] compiled"),
    }
}

/// The ColourZeroHalf refusal is Mesa's U16_TO_U32, measured: the shim's
/// program for the same pair (sel_vc_zero) has it where a MKVEC.v2i16
/// stands for any other constant, and its vertex shader is byte-equal.
#[test]
fn a_varying_lane_paired_with_zero_is_mesa_s_u16_to_u32() {
    const VS_SRC: &str = include_str!("../../../../tools/mali/testdata/sel_vc_zero.vert");
    const FS_SRC: &str = include_str!("../../../../tools/mali/testdata/sel_vc_zero.frag");
    const JSON: &str = include_str!("../../../../tools/mali/testdata/sel_vc_zero.expect.json");
    let fs = region(JSON, "fs.fragment").expect("fs.fragment");
    let ops: Vec<_> = fs.words.iter().map(|w| isa::decode(*w).expect("decodes").op.info().mnemonic).collect();
    assert!(ops.contains(&"U16_TO_U32") && !ops.contains(&"MKVEC.v2i16"), "{ops:?}");
    assert_eq!(gap(SHC_STAGE_FRAGMENT, FS_SRC), MaliError::ColourZeroHalf);
    let v = compile(SHC_STAGE_VERTEX, VS_SRC).expect("the vertex shader compiles");
    let (pos, var) = vs_regions(&v);
    assert_words("sel_vc_zero vs.position", &pos, &region(JSON, "vs.position").unwrap().words);
    assert_words("sel_vc_zero vs.varying", &var, &region(JSON, "vs.varying").unwrap().words);
}

/// The referenced programs' size against the back end's tables, measured
/// over the populations (the corpus, the selector's, the uniforms'): the
/// longest program and the most FAU words of a stage. sel_u_fau64 fills
/// FAU page 0; the tables' own limits are reached by the stress cases of
/// `stress_cases`.
#[test]
fn the_slice_s_programs_are_far_from_the_back_end_s_tables() {
    let mut srcs: Vec<(u32, &str)> = vec![
        (SHC_STAGE_VERTEX, TRI_VS),
        (SHC_STAGE_FRAGMENT, TRI_FS),
        (SHC_STAGE_VERTEX, TV_VS),
        (SHC_STAGE_FRAGMENT, TV_FS),
        (SHC_STAGE_VERTEX, SAMPLE_VS),
        (SHC_STAGE_FRAGMENT, SAMPLE_FS),
    ];
    for (_, vs, fs, _) in POPULATION.iter().chain(&UNIFORM_POPULATION) {
        srcs.push((SHC_STAGE_VERTEX, vs));
        srcs.push((SHC_STAGE_FRAGMENT, fs));
    }
    srcs.extend(VARYING_POPULATION.iter().map(|c| (SHC_STAGE_VERTEX, c.1)));
    srcs.extend(FRAGMENT_VARYING_POPULATION.iter().map(|c| (SHC_STAGE_FRAGMENT, c.2)));
    let (mut longest, mut fau) = ([0usize; 2], [0usize; 2]);
    for (st, src) in srcs {
        let s = compile(st, src).expect("compiles");
        let k = (st == SHC_STAGE_FRAGMENT) as usize;
        let (a, b) = if k == 0 { vs_regions(&s) } else { (trim(&s.code), Vec::new()) };
        longest[k] = longest[k].max(a.len()).max(b.len());
        fau[k] = fau[k].max(s.info.fau.len());
    }
    assert_eq!((longest, fau), ([44, 17], [64, 7]), "(longest VS, FS program), (most VS, FS FAU words)");
    assert!(longest[0] < super::bir::MAX_INSTRS && fau[0] == super::fau::FAU_PAGE0_WORDS);
}

/// An invalid shader gets the front end's verdict, not a gap.
#[test]
fn an_invalid_shader_gets_the_front_end_s_verdict() {
    let e = compile(SHC_STAGE_VERTEX, "void main() { gl_Position = undeclared; }\n");
    assert!(matches!(e, Err(CompileError::Rejected { .. })), "{:?}", e.err());
}

/// A blend variant is refused: the Mali blends in its blend descriptor.
fn a_blend_variant_is_refused_gap() -> MaliError {
    let _k = store::test_keep();
    let r = store::with_scratch(|s| {
        codegen::compile_linked_blocks(SHC_STAGE_FRAGMENT, TRI_FS.as_bytes(), 1, &[], s, store::keep()).map(|_| ())
    })
    .unwrap();
    match r {
        Err(CompileError::BackEnd(codegen::BackEndError::Mali(e))) => e,
        r => panic!("not a Mali gap: {r:?}"),
    }
}

#[test]
fn a_blend_variant_is_refused() {
    assert_eq!(a_blend_variant_is_refused_gap(), MaliError::BlendVariantNotLowered);
}

/// A vertex shader at the link's attribute locations is refused after the
/// front end's verdict.
fn a_linked_vertex_shader_is_refused_gap() -> MaliError {
    let _k = store::test_keep();
    let map = crate::shaderc::ShcAttribMap::ZERO;
    let r = store::with_scratch(|s| {
        codegen::compile_vs_mapped_blocks(TRI_VS.as_bytes(), &map, s, store::keep()).map(|_| ())
    })
    .unwrap();
    match r {
        Err(CompileError::BackEnd(codegen::BackEndError::Mali(e))) => e,
        r => panic!("not a Mali gap: {r:?}"),
    }
}

#[test]
fn a_linked_vertex_shader_is_refused() {
    assert_eq!(a_linked_vertex_shader_is_refused_gap(), MaliError::LinkedVertexNotLowered);
}

/* ---- the frame builder's side (src/mali/frame.rs) ------------------- */

/// What the frame builder's `ShaderMeta` takes from tri's records, through
/// src/mali/program.rs: the register allocation (32 per thread), the
/// preloaded r48-r63 and every push word in FAU order. The vertex shader's
/// sysval words are exactly the ones frame_tests.rs pushes for the board's
/// 25.0.7 program (`VS_PUSH`: scale.x, offset.x, scale.y, offset.y,
/// offset.z; sysval 0 = scale, 1 = offset), then the three promoted store
/// offsets 4, 8, 12; the fragment shader's are the four blend constants
/// (sysval 0 of its block, Mesa's first fragment slot) and the promoted
/// 1.0h. A varying program exists only with a varying (tri_varying's at
/// byte 256, generic size 12 bytes for its vec3).
#[test]
fn the_record_meets_the_frame_builder_s_meta() {
    use crate::mali::frame::{PushWord, SecondaryProgram, Sysval};
    use crate::mali::genxml::v10::{ShaderRegisterAllocation, ShaderStage};
    use crate::mali::program::StageMeta;
    let v = compile(SHC_STAGE_VERTEX, TRI_VS).unwrap();
    let f = compile(SHC_STAGE_FRAGMENT, TRI_FS).unwrap();
    let (vm, fm) = (StageMeta::of(&v.info.shader()).unwrap(), StageMeta::of(&f.info.shader()).unwrap());
    let (vm, fm) = (vm.meta(), fm.meta());
    assert_eq!((vm.stage, fm.stage), (ShaderStage::VERTEX, ShaderStage::FRAGMENT));
    assert_eq!(vm.register_allocation, ShaderRegisterAllocation::_32_PER_THREAD);
    assert_eq!(fm.register_allocation, ShaderRegisterAllocation::_32_PER_THREAD);
    assert_eq!((vm.preload, fm.preload), (0x3800, 0x3000));
    let s = |sysval, component| PushWord::Sysval { sysval, component };
    assert_eq!(vm.sysvals, [Sysval::ViewportScale, Sysval::ViewportOffset]);
    assert_eq!(
        vm.push,
        [
            s(0, 0),
            s(1, 0),
            s(0, 1),
            s(1, 1),
            s(1, 2),
            PushWord::Constant(4),
            PushWord::Constant(8),
            PushWord::Constant(12)
        ]
    );
    assert_eq!(fm.sysvals, [Sysval::BlendConstants]);
    assert_eq!(fm.push, [s(0, 0), s(0, 1), s(0, 2), s(0, 3), PushWord::Constant(0x3c00)]);
    assert_eq!((v.info.shader().fau_slots(), f.info.shader().fau_slots()), (4, 3));
    assert_eq!((vm.varyings, vm.varying_bytes, vm.secondary, fm.varyings), (0, 0, None, 0));
    assert_eq!(fm.colour_outputs, 1);

    let v = compile(SHC_STAGE_VERTEX, TV_VS).unwrap();
    let f = compile(SHC_STAGE_FRAGMENT, TV_FS).unwrap();
    let (vm, fm) = (StageMeta::of(&v.info.shader()).unwrap(), StageMeta::of(&f.info.shader()).unwrap());
    let (vm, fm) = (vm.meta(), fm.meta());
    assert_eq!(
        vm.secondary,
        Some(SecondaryProgram {
            offset: 256,
            preload: 0x3800,
            register_allocation: ShaderRegisterAllocation::_32_PER_THREAD
        })
    );
    assert_eq!((vm.varyings, vm.varying_bytes, fm.varyings), (1, 12, 1));
}

/// A shader through the cache (`SHC_ONLINE.compile`, the GL core's path):
/// the cache's record, its only holder.
fn compile_cached(stage: u32, src: &str) -> crate::shaderc::ShcShader {
    let text = std::ffi::CString::new(src).unwrap();
    let mut out = crate::shaderc::ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 256];
    let compile = crate::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output and a log of that size.
    let rc = unsafe { compile(stage, text.as_ptr(), &mut out, log.as_mut_ptr(), log.len()) };
    // SAFETY: the writer NUL-terminated the log.
    let msg = unsafe { core::ffi::CStr::from_ptr(log.as_ptr()) }.to_string_lossy().into_owned();
    assert_eq!(rc, 0, "{msg}");
    out
}

/// `backend_priv` (online/backend.rs) by tier: a live cache record leads to
/// the Mali record the code generator made; a code generator's record that
/// never went through the cache is found by its tagged `priv_`; a freed
/// record leads to nothing, whatever now lies where its `priv_` points.
/// The last is the defect this pins: the in-keep fallback returned the dead
/// record's pointer and `MaliShader::of` trusted the magic it found there,
/// so a stale handle could resolve to a newer compile's record.
#[test]
fn a_freed_record_gives_no_mali_record_and_a_stale_handle_never_a_newer_one() {
    use crate::shaderc::online::backend::{backend_priv, release};
    let _serial = store::test_lock();
    /* a text no other test compiles, so this is the record's only holder */
    let a_src = format!("{TRI_VS}\n// backend_priv: the record that is freed\n");
    let a = compile_cached(SHC_STAGE_VERTEX, &a_src);
    assert!(store::in_keep(a.priv_ as *const u8) && a.priv_.align_offset(8) == 0, "a cache record");
    let live = MaliShader::of(&a).expect("a live cache record has a Mali record").stage;
    assert_eq!(live, SHC_STAGE_VERTEX);
    assert!(!backend_priv(&a).is_null());
    release(&a);
    assert!(!crate::shaderc::is_online(&a), "the only holder let go: the record is freed");
    assert!(backend_priv(&a).is_null(), "a freed record");
    assert!(MaliShader::of(&a).is_none());

    /* a newer compile's Mali record where the dead record was: the bytes a
     * stale handle would have resolved to (written into the freed extent,
     * which nothing holds while the lock is taken) */
    let fake = MaliShader {
        magic: super::MALI_RECORD_MAGIC,
        stage: SHC_STAGE_FRAGMENT,
        preload: 0,
        work_reg_count: 0,
        secondary: None,
        no_psiz_offset: None,
        fau: &[],
        attributes: &[],
        varyings: &[],
        colour_f16: true,
        compacted: false,
        moved: &[],
        sampler: None,
        helper_threads: false,
        can_discard: false,
        ubo: &[],
    };
    // SAFETY: an aligned address in the keep region's freed extent (checked
    // above), large enough for a cache record, which is larger than this.
    unsafe { (a.priv_ as *mut MaliShader<'static>).write(fake) };
    assert!(backend_priv(&a).is_null(), "a stale handle with a Mali record behind it");
    assert!(MaliShader::of(&a).is_none());

    /* and a real newer compile: it resolves, the stale handle does not */
    let b_src = format!("{TV_VS}\n// backend_priv: the newer record\n");
    let b = compile_cached(SHC_STAGE_VERTEX, &b_src);
    assert_eq!(MaliShader::of(&b).expect("the newer record").attributes, [(0, 0), (1, 1)]);
    assert!(MaliShader::of(&a).is_none());
    assert_ne!(backend_priv(&a), backend_priv(&b));
    release(&b);

    /* a code generator's own record (never through the cache): tagged */
    let _k = store::test_keep();
    let ok = store::with_scratch(|s| {
        codegen::compile_linked_blocks(SHC_STAGE_VERTEX, TRI_VS.as_bytes(), 0, &[], s, store::keep()).map(|(sh, _)| {
            assert_eq!(sh.priv_.addr() & 1, 1, "the code generator's record is tagged");
            MaliShader::of(&sh).map(|r| r.stage)
        })
    })
    .expect("the scratch arena is free")
    .expect("tri.vert compiles");
    assert_eq!(ok, Some(SHC_STAGE_VERTEX));
}

/* ---- issue #1176: a record outlives the compile that made it ---------- */

/// The record a test reads, as owned values (the same calls on any version
/// of the helper's `Stage`).
fn record_values(s: &Stage) -> (Vec<FauWord>, Vec<(u8, u8)>, Vec<VaryingSlot>) {
    (s.info.fau.to_vec(), s.info.attributes.to_vec(), s.info.varyings.to_vec())
}

/// Issue #1176, deterministic: a stage's record still holds its own words
/// after the next compiles. The sequence is atomic (`store::test_lock`), so
/// nothing else compiles in between. Before PR #1174's fix the record's
/// slices pointed into keep memory that the compile's transaction did not
/// keep, and the next compile wrote there: the words read afterwards were
/// that compile's bytes.
#[test]
fn a_stage_s_record_survives_the_next_compiles() {
    let _serial = store::test_lock();
    let a = compile(SHC_STAGE_VERTEX, SAMPLE_VS).expect("the sample's vertex shader compiles");
    let want = record_values(&a);
    assert_sample_vs_fau(&want.0);
    for (stage, src) in [
        (SHC_STAGE_VERTEX, TRI_VS),
        (SHC_STAGE_FRAGMENT, TV_FS),
        (SHC_STAGE_VERTEX, TV_VS),
        (SHC_STAGE_FRAGMENT, SAMPLE_FS),
    ] {
        compile(stage, src).expect("compiles");
    }
    assert_eq!(record_values(&a), want, "the record after four more compiles");
}

/// Issue #1176 with the compile that reuses the memory on another thread,
/// as under the gate's parallel test threads (where it was seen, in
/// the_triangle_sample_s_fau_words_are_named_by_a_draw_that_tells_them_apart):
/// this thread compiles a stage the way the byte-reference tests do (no
/// lock held between the compile and the read), a second thread compiles
/// another shader, then this thread reads the record. Lock-step through two
/// channels, so every round has that order and the run is bounded. Every
/// read must be the record's own words.
#[test]
fn a_stage_s_record_is_its_own_while_another_thread_compiles() {
    use std::sync::mpsc::channel;
    let want = record_values(&compile(SHC_STAGE_VERTEX, SAMPLE_VS).expect("compiles"));
    let (go, go_rx) = channel::<(u32, &'static str)>();
    let (done_tx, done) = channel::<()>();
    let other = std::thread::spawn(move || {
        let mut n = 0u32;
        for (stage, src) in go_rx {
            compile(stage, src).expect("compiles");
            n += 1;
            done_tx.send(()).expect("the reader waits");
        }
        n
    });
    let srcs = [(SHC_STAGE_VERTEX, TRI_VS), (SHC_STAGE_FRAGMENT, TV_FS), (SHC_STAGE_VERTEX, TV_VS)];
    let mut wrong = 0;
    for round in 0..20 {
        let s = compile(SHC_STAGE_VERTEX, SAMPLE_VS).expect("compiles");
        go.send(srcs[round % srcs.len()]).expect("the other thread runs");
        done.recv().expect("the other thread compiled");
        if record_values(&s) != want {
            wrong += 1;
        }
    }
    drop(go);
    let compiles = other.join().expect("the other thread");
    assert_eq!(compiles, 20, "the other thread compiled once a round");
    assert_eq!(wrong, 0, "rounds of 20 whose read was not the record's own words");
}

/* ---- uniforms (L3): the shim references ------------------------------ */

macro_rules! td {
    ($f:literal) => {
        include_str!(concat!("../../../../tools/mali/testdata/", $f))
    };
}

/* ---- gl_PointSize: pan_shc's references ------------------------------ */

/// One stage object of a pan_shc JSON (glsl2pan.py's stdout, its first line
/// the build line): the text from `"<stage>": {` to the next stage's.
fn pan_shc_stage<'a>(json: &'a str, stage: &str) -> &'a str {
    let at = json.find(&format!("\"{stage}\": {{")).expect("the stage");
    let rest = &json[at + 1..];
    &rest[..rest.find("\n  \"").unwrap_or(rest.len())]
}

fn pan_shc_num(s: &str, key: &str) -> u64 {
    let t = after(s, key).unwrap_or_else(|| panic!("{key}"));
    let t = t.trim_matches('"');
    let end = t.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(t.len());
    let t = &t[..end];
    match t.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).expect("hex"),
        None => t.parse().unwrap_or_else(|_| panic!("{key}: {t}")),
    }
}

/// The `fau.words` of a pan_shc stage, each as "sysval NAME c", "constant
/// 0x..." or "uniform d".
fn pan_shc_fau(s: &str) -> Vec<String> {
    let w = after(s, "words").expect("the FAU words");
    let w = &w[..w.find(']').expect("the end")];
    w.split('}')
        .filter(|x| x.contains("\"word\""))
        .map(|x| {
            let str_of = |k: &str| after(x, k).map(|t| t.split('"').nth(1).unwrap_or("").to_string());
            if let Some(c) = str_of("constant") {
                format!("constant {c}")
            } else if let Some(n) = str_of("sysval") {
                format!("sysval {n} {}", pan_shc_num(x, "component"))
            } else {
                format!("uniform {}", pan_shc_num(x, "uniform_dword"))
            }
        })
        .collect()
}

fn our_fau(f: &[FauWord]) -> Vec<String> {
    use super::bir::Sysval;
    f.iter()
        .map(|w| match *w {
            FauWord::Sysval(Sysval::ViewportScale, c) => format!("sysval VIEWPORT_SCALE {c}"),
            FauWord::Sysval(Sysval::ViewportOffset, c) => format!("sysval VIEWPORT_OFFSET {c}"),
            FauWord::BlendConstant(c) => format!("sysval BLEND_CONSTANTS {c}"),
            FauWord::Constant(c) => format!("constant 0x{c:08x}"),
            FauWord::Uniform { dword } => format!("uniform {dword}"),
            other => format!("{other:?}"),
        })
        .collect()
}

/// A vertex shader against pan_shc's record of it (Mesa 26.2.1, `--prim
/// points`; tools/mali/testdata/cases.txt finds pan_shc equal to the shim
/// driver for ctl_psiz.vert): the whole code (the position program, its
/// no_psiz copy and the varying program, each padded), the offsets, the
/// register counts, the preloads and the FAU words.
fn check_vs_pan_shc(vs: &str, json: &str) {
    check_vs_record_pan_shc(&compile(SHC_STAGE_VERTEX, vs).expect("the vertex shader compiles"), json);
}

/// [`check_vs_pan_shc`] of a compiled stage, and the generic varyings'
/// bytes (`generic_size_B`).
fn check_vs_record_pan_shc(v: &Stage, json: &str) {
    let r = pan_shc_stage(json, "vs");
    let generic = v.info.varyings.iter().filter(|s| s.section == Section::Generic);
    let bytes = generic.map(|s| s.offset as u64 + 4 * s.comps as u64).max().unwrap_or(0);
    assert_eq!(bytes, pan_shc_num(r, "generic_size_B"), "generic_size_B");
    let code: Vec<u64> = after(r, "code")
        .and_then(|t| t.find(']').map(|e| &t[1..e]))
        .expect("the code")
        .split(',')
        .map(|x| u64::from_str_radix(x.trim().trim_matches('"').trim_start_matches("0x"), 16).expect("hex"))
        .collect();
    assert_words("vs (all regions)", &v.code, &code);
    /* pan_shc prints 0 for a shader that does not write gl_PointSize */
    let psiz = r.contains("\"writes_point_size\": true");
    assert_eq!(v.info.no_psiz_offset.is_some(), psiz, "writes_point_size");
    assert_eq!(v.info.no_psiz_offset.map_or(0, u64::from), pan_shc_num(r, "no_psiz_offset"), "no_psiz_offset");
    assert_eq!(v.info.work_reg_count as u64, pan_shc_num(r, "work_reg_count"), "work_reg_count");
    assert_eq!(v.info.preload, pan_shc_num(r, "preload"), "preload");
    match v.info.secondary {
        Some(sec) => {
            assert!(r.contains("\"secondary_enable\": true"), "secondary_enable");
            assert_eq!(sec.offset as u64, pan_shc_num(r, "secondary_offset"), "secondary_offset");
            assert_eq!(sec.work_reg_count as u64, pan_shc_num(r, "secondary_work_reg_count"));
            assert_eq!(sec.preload, pan_shc_num(r, "secondary_preload"), "secondary_preload");
        }
        None => assert!(r.contains("\"secondary_enable\": false"), "secondary_enable"),
    }
    assert_eq!(our_fau(&v.info.fau), pan_shc_fau(r), "FAU words");
}

/// Both stages of a program against pan_shc's record of it.
fn check_program_pan_shc(vs: &str, fs: &str, json: &str) {
    check_vs_pan_shc(vs, json);
    check_fs_pan_shc(fs, json);
}

/// A fragment shader against pan_shc's record of it: the code, the
/// register count, the preload and the FAU words.
fn check_fs_pan_shc(fs: &str, json: &str) {
    let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
    let r = pan_shc_stage(json, "fs");
    let code: Vec<u64> = after(r, "code")
        .and_then(|t| t.find(']').map(|e| &t[1..e]))
        .expect("the code")
        .split(',')
        .map(|x| u64::from_str_radix(x.trim().trim_matches('"').trim_start_matches("0x"), 16).expect("hex"))
        .collect();
    assert_words("fs.fragment", &f.code, &code);
    assert_eq!(f.info.work_reg_count as u64, pan_shc_num(r, "work_reg_count"), "work_reg_count");
    assert_eq!(f.info.preload, pan_shc_num(r, "preload"), "preload");
    assert_eq!(our_fau(&f.info.fau), pan_shc_fau(r), "FAU words");
}

const PSIZ_POINTS: &str = td!("pan_shc-out/psiz_points.json");
const PSIZ_CONST: &str = td!("pan_shc-out/psiz_const.json");
const PSIZ_VARYING: &str = td!("pan_shc-out/psiz_varying.json");
const POINTS_VS: &str = include_str!("../../../../tools/shaders/gl_test_es100/points.vert");

/// A vertex shader built for the program of `fs`: the fragment shader's
/// record's reads.
fn compile_vs_for(vs: &str, fs: &str) -> Stage {
    let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
    let pr = super::fragment_reads(&f.info.shader()).expect("the record's moved expressions encode");
    let _k = store::test_keep();
    store::with_scratch(|s| {
        super::compile_vs_for_reads(vs.as_bytes(), pr.reads(), pr.moved(), s, store::keep())
            .map(|(sh, _)| stage_of(&sh))
    })
    .expect("the scratch arena is free")
    .expect("the vertex shader compiles for the program")
}

/// The program's vertex shader of each compaction case against pan_shc's:
/// only the components read stored, at their compacted offsets (the
/// varying program's words, `generic_size_B` through the record).
#[test]
fn the_compacted_vertex_programs_are_byte_equal_to_mesa_26_2_1() {
    let cases = [
        (td!("cmp_vs4.vert"), td!("cmp_b.frag"), td!("pan_shc-out/cmp_b.json")),
        (td!("cmp_vs4.vert"), td!("cmp_xy.frag"), td!("pan_shc-out/cmp_xy.json")),
        (td!("cmp_vs4.vert"), td!("cmp_xz.frag"), td!("pan_shc-out/cmp_xz.json")),
        (td!("cmp_vs4.vert"), td!("cmp_w.frag"), td!("pan_shc-out/cmp_w.json")),
        (td!("cmp_vs4.vert"), td!("cmp_zw.frag"), td!("pan_shc-out/cmp_zw.json")),
        (td!("cmp_vs4.vert"), td!("cmp_xyz.frag"), td!("pan_shc-out/cmp_xyz.json")),
        (td!("cmp_vs2x4.vert"), td!("cmp_axby.frag"), td!("pan_shc-out/cmp_axby.json")),
        (td!("cmp_vs2x4.vert"), td!("cmp_axybzw.frag"), td!("pan_shc-out/cmp_axybzw.json")),
        (POINTS_VS, POINTCOORDVARY_FS, td!("pan_shc-out/pointcoordvary.json")),
    ];
    for (vs, fs, json) in cases {
        check_vs_record_pan_shc(&compile_vs_for(vs, fs), json);
    }
}

/// A vertex shader's multiply by a constant, as Mesa's algebraic rules
/// leave it (pan_shc's references, tools/mali/testdata/cmul_*): the
/// constant second, `* -1.0` an fneg folded into its user or stored as
/// `FADD x.neg, -0.0`, `* 2.0` an `fadd(a, a)` after the fusion, other
/// constants an FMA with a table or pushed constant; gl_test's ioswap.vert
/// and immneg.vert themselves.
#[test]
fn the_constant_products_are_byte_equal_to_mesa_26_2_1() {
    for (vs, json) in [
        (td!("cmul_m075.vert"), td!("pan_shc-out/cmul_m075.json")),
        (td!("cmul_m2.vert"), td!("pan_shc-out/cmul_m2.json")),
        (td!("cmul_mneg.vert"), td!("pan_shc-out/cmul_mneg.json")),
        (td!("cmul_m05.vert"), td!("pan_shc-out/cmul_m05.json")),
        (td!("cmul_m2s1.vert"), td!("pan_shc-out/cmul_m2s1.json")),
        (td!("cmul_m3w.vert"), td!("pan_shc-out/cmul_m3w.json")),
        (td!("cmul_m3mix.vert"), td!("pan_shc-out/cmul_m3mix.json")),
        (td!("cmul_w2.vert"), td!("pan_shc-out/cmul_w2.json")),
        (td!("cmul_vneg.vert"), td!("pan_shc-out/cmul_vneg.json")),
        (td!("cmul_v075.vert"), td!("pan_shc-out/cmul_v075.json")),
        (include_str!("../../../../tools/shaders/es100/ioswap.vert"), td!("pan_shc-out/cmul_ioswap.json")),
        (include_str!("../../../../tools/shaders/es100/immneg.vert"), td!("pan_shc-out/cmul_immneg.json")),
        (td!("cmul_z05.vert"), td!("pan_shc-out/cmul_z05.json")),
        (td!("cmul_negprod.vert"), td!("pan_shc-out/cmul_negprod.json")),
        /* `x * 0.5 + 0.5` into a varying (cmul_tsf.frag reads it whole) */
        (include_str!("../../../../tools/shaders/gltest/tsf_vs.vert"), td!("pan_shc-out/cmul_tsf.json")),
    ] {
        let name = json.lines().nth(1).unwrap_or("");
        if let Err(e) = std::panic::catch_unwind(|| check_vs_pan_shc(vs, json)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_default();
            panic!("{}: {m}", vs.lines().next().unwrap_or(name));
        }
    }
}

/// A division, `a * rcp(b)` from the front end (pan_shc's references):
/// one by a uniform, and femtovg's position (`2.0 * x / u - 1.0` and `1.0 -
/// 2.0 * y / u`: the negated product's fusion, `ffma(fneg(a), b, c)`).
/// femtovg's main.vert compiles (its two vec2 varyings share one slot,
/// [`the_packed_varyings_are_byte_equal_to_mesa_26_2_1`]).
#[test]
fn the_divisions_are_byte_equal_to_mesa_26_2_1() {
    check_vs_pan_shc(td!("cdiv_d1.vert"), td!("pan_shc-out/cdiv_d1.json"));
    check_vs_pan_shc(td!("cdiv_d2.vert"), td!("pan_shc-out/cdiv_d2.json"));
    compile(SHC_STAGE_VERTEX, include_str!("../../../../tools/shaders/femtovg/main.vert")).expect("compiles");
}

/// Two vec2 varyings in one slot: the second at byte 8, the fragment
/// shader's one four-lane load (pan_shc's cpack_p2; the shim's
/// sel_v_packed is [`two_varyings_in_one_slot_are_mesa_s_packed_varyings`]).
#[test]
fn the_packed_varyings_are_byte_equal_to_mesa_26_2_1() {
    check_program_pan_shc(td!("cpack_p2.vert"), td!("cpack_p2.frag"), td!("pan_shc-out/cpack_p2.json"));
}

/// femtovg's program, as pan_shc builds it, moves `ftcoord.x * 2.0 - 1.0`
/// out of fill_color.frag into the vertex shader (nir_opt_varyings'
/// inter-stage code motion): the varying program holds `FMA x, 2.0, -1.0`.
/// So the program's vertex shader depends on the fragment shader's code,
/// not only on its reads (motion.rs; the shim's words are
/// [`the_s4_population_is_byte_equal_to_mesa_26_2_1`]'s fill_color pair).
#[test]
fn femtovg_s_program_moves_fragment_code_into_the_vertex_shader() {
    let json = td!("pan_shc-out/cpack_femtovg.json");
    let r = pan_shc_stage(json, "vs");
    let code: Vec<u64> = after(r, "code")
        .and_then(|t| t.find(']').map(|e| &t[1..e]))
        .expect("the code")
        .split(',')
        .map(|x| u64::from_str_radix(x.trim().trim_matches('"').trim_start_matches("0x"), 16).expect("hex"))
        .collect();
    let secondary = pan_shc_num(r, "secondary_offset") as usize / 8;
    assert!(code[secondary..]
        .iter()
        .any(|w| text(*w).starts_with("FMA.f32") && text(*w).contains("0x40000000, 0x3F800000.neg")));
}

/// `0.75 * a_pos.xy`: Mesa's GLSL path keeps the constant first (`FMA
/// 0.75, x`, the board's Mesa 25.0.7, 0dce654a8:tools/mali/board-ref/
/// 2026-10-19-const-order), and so do we; pan_shc's SPIR-V path puts the
/// scalar after the vector (glslang's `OpVectorTimesScalar`). The program
/// equals pan_shc's but for those FMA words, whose first two sources are
/// swapped.
#[test]
fn a_constant_first_product_keeps_the_glsl_order() {
    let json = td!("pan_shc-out/cmul_c075x.json");
    let v = compile(SHC_STAGE_VERTEX, td!("cmul_c075x.vert")).expect("compiles");
    let r = pan_shc_stage(json, "vs");
    let code: Vec<u64> = after(r, "code")
        .and_then(|t| t.find(']').map(|e| &t[1..e]))
        .expect("the code")
        .split(',')
        .map(|x| u64::from_str_radix(x.trim().trim_matches('"').trim_start_matches("0x"), 16).expect("hex"))
        .collect();
    assert_eq!(v.code.len(), code.len());
    let mut swapped = 0;
    for (k, (o, t)) in v.code.iter().zip(&code).enumerate() {
        if o == t {
            continue;
        }
        /* `FMA.f32 d, a, b, c` against `FMA.f32 d, b, a, c` (a last-use
         * `^` mark follows the register, not the operand's place) */
        let (ot, tt) = (text(*o), text(*t));
        let ops = |s: &str| s.split(", ").map(|x| x.trim_end_matches('^').to_string()).collect::<Vec<_>>();
        let (oo, to) = (ops(&ot), ops(&tt));
        assert!(ot.starts_with("FMA.f32") && oo.len() == 4 && to.len() == 4, "[{k}] ours {ot}, theirs {tt}");
        assert_eq!((&oo[0], &oo[1], &oo[2], &oo[3]), (&to[0], &to[2], &to[1], &to[3]), "[{k}] ours {ot}, theirs {tt}");
        swapped += 1;
    }
    assert_eq!(swapped, 2, "the two FMA words of the product, and nothing else");
}

/// gl_PointSize from a uniform, a constant and an attribute (with a
/// varying): the fp16 point size store and the no_psiz copy.
#[test]
fn the_point_size_programs_are_byte_equal_to_mesa_26_2_1() {
    check_vs_pan_shc(td!("ctl_psiz.vert"), PSIZ_POINTS);
    check_vs_pan_shc(td!("ctl_psiz_const.vert"), PSIZ_CONST);
    check_vs_pan_shc(POINTS_VS, PSIZ_VARYING);
}

const POINTCOORD_FS: &str = include_str!("../../../../tools/shaders/es100/pointcoord.frag");
const POINTCOORDVARY_FS: &str = include_str!("../../../../tools/shaders/es100/pointcoordvary.frag");

/// gl_PointCoord (gl_test case 83's shader): `LD_VAR_SPECIAL.point` and
/// its conversion.
#[test]
fn the_point_coord_program_is_byte_equal_to_mesa_26_2_1() {
    check_fs_pan_shc(POINTCOORD_FS, td!("pan_shc-out/pointcoord.json"));
}

/// The fragment shaders that read part of their varyings, after Mesa's
/// link keeps only the components read (`compact_varyings`): case 105's
/// (`v_color.b` beside gl_PointCoord) and the compaction controls (one vec4
/// read as .b, .xy, .xz, .w, .zw, .xyz; two vec4s read as `v_a.x, v_b.y`
/// and `v_a.xy, v_b.zw`, which share one slot), each with its vertex
/// shader (tools/mali/testdata/cmp_*).
#[test]
fn the_compacted_fragment_programs_are_byte_equal_to_mesa_26_2_1() {
    check_fs_pan_shc(POINTCOORDVARY_FS, td!("pan_shc-out/pointcoordvary.json"));
    for (fs, json) in [
        (td!("cmp_b.frag"), td!("pan_shc-out/cmp_b.json")),
        (td!("cmp_xy.frag"), td!("pan_shc-out/cmp_xy.json")),
        (td!("cmp_xz.frag"), td!("pan_shc-out/cmp_xz.json")),
        (td!("cmp_w.frag"), td!("pan_shc-out/cmp_w.json")),
        (td!("cmp_zw.frag"), td!("pan_shc-out/cmp_zw.json")),
        (td!("cmp_xyz.frag"), td!("pan_shc-out/cmp_xyz.json")),
        (td!("cmp_axby.frag"), td!("pan_shc-out/cmp_axby.json")),
        (td!("cmp_axybzw.frag"), td!("pan_shc-out/cmp_axybzw.json")),
    ] {
        check_fs_pan_shc(fs, json);
    }
}

const SEL_U_FAU65: &str = td!("sel_u_fau65.vert");
const SEL_U_FAU130: &str = td!("sel_u_fau130.vert");
const SEL_U_REBALANCE: &str = td!("sel_u_rebalance.vert");
const PASSTHROUGH_VS: &str = SAMPLE_VS;
const INTERP_FS: &str = SAMPLE_FS;

/// The uniform population: every case drawn on the shim with
/// `drm_shim_ref.sh run ... --uniform-values` (Mesa 26.2.1 on drm-shim,
/// the work3 build of 2026-10-19), its programs extracted by
/// `pan_ref_diff.py extract --draw` (name, VS, FS, expect.json).
const UNIFORM_POPULATION: [(&str, &str, &str, &str); 25] = [
    ("sel_u_ctl", td!("ctl_uniforms.vert"), TRI_FS, td!("sel_u_ctl.expect.json")),
    ("sel_u_mat4", td!("ctl_mat4.vert"), TRI_FS, td!("sel_u_mat4.expect.json")),
    ("sel_u_vecmat", td!("sel_u_vecmat.vert"), TRI_FS, td!("sel_u_vecmat.expect.json")),
    ("sel_u_offset", td!("sel_u_offset.vert"), TRI_FS, td!("sel_u_offset.expect.json")),
    ("sel_u_color", TRI_VS, td!("sel_u_color.frag"), td!("sel_u_color.expect.json")),
    ("sel_u_mix", PASSTHROUGH_VS, td!("sel_u_mix.frag"), td!("sel_u_mix.expect.json")),
    ("sel_u_cse", td!("sel_u_cse.vert"), TRI_FS, td!("sel_u_cse.expect.json")),
    ("sel_u_float_vec4", td!("sel_u_float_vec4.vert"), TRI_FS, td!("sel_u_float_vec4.expect.json")),
    ("sel_u_vec3_float", td!("sel_u_vec3_float.vert"), TRI_FS, td!("sel_u_vec3_float.expect.json")),
    ("sel_u_float_mat4", td!("sel_u_float_mat4.vert"), TRI_FS, td!("sel_u_float_mat4.expect.json")),
    ("sel_u_zw", td!("sel_u_zw.vert"), TRI_FS, td!("sel_u_zw.expect.json")),
    ("sel_u_yz", td!("sel_u_yz.vert"), TRI_FS, td!("sel_u_yz.expect.json")),
    ("sel_u_hole", td!("sel_u_hole.vert"), TRI_FS, td!("sel_u_hole.expect.json")),
    ("sel_u_vec2", td!("sel_u_vec2.vert"), TRI_FS, td!("sel_u_vec2.expect.json")),
    ("sel_u_two_attr", td!("sel_u_two_attr.vert"), TRI_FS, td!("sel_u_two_attr.expect.json")),
    ("sel_u_attr_fma", td!("sel_u_attr_fma.vert"), TRI_FS, td!("sel_u_attr_fma.expect.json")),
    ("sel_u_whole", td!("sel_u_whole.vert"), TRI_FS, td!("sel_u_whole.expect.json")),
    ("sel_u_w", td!("sel_u_w.vert"), TRI_FS, td!("sel_u_w.expect.json")),
    ("sel_u_unused", td!("sel_u_unused.vert"), TRI_FS, td!("sel_u_unused.expect.json")),
    ("sel_u_repair_alpha", TRI_VS, td!("sel_u_repair_alpha.frag"), td!("sel_u_repair_alpha.expect.json")),
    ("sel_u_mvp_varying", td!("sel_u_mvp_varying.vert"), INTERP_FS, td!("sel_u_mvp_varying.expect.json")),
    ("sel_u_fma_const", td!("sel_u_fma_const.vert"), TRI_FS, td!("sel_u_fma_const.expect.json")),
    ("sel_u_cse_alu", td!("sel_u_cse_alu.vert"), TRI_FS, td!("sel_u_cse_alu.expect.json")),
    ("sel_u_fold", td!("sel_u_fold.vert"), TRI_FS, td!("sel_u_fold.expect.json")),
    ("sel_u_fau64", td!("sel_u_fau64.vert"), TRI_FS, td!("sel_u_fau64.expect.json")),
];

#[test]
fn the_uniform_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    let mut regions = 0;
    for (name, vs, fs, json) in UNIFORM_POPULATION {
        regions += 2 + region(json, "vs.varying").is_some() as usize;
        if let Err(e) = std::panic::catch_unwind(|| check_program(vs, fs, json)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), UNIFORM_POPULATION.len(), failed.join("\n"));
    /* the population: 25 programs, 52 regions (sel_u_mix and
     * sel_u_mvp_varying have a varying program) */
    assert_eq!((UNIFORM_POPULATION.len(), regions), (25, 52), "(programs, regions) compared");
}

/// The productions the vertex program builder's code can make
/// ([`super::nir::Prod`]), each referenced by a case of the populations: a
/// new production fails here until a shim case draws it.
#[test]
fn the_uniform_population_is_referenced() {
    let mut seen = 0u32;
    for src in vs_reference_sources() {
        seen |= prods(src);
    }
    /* the compaction cases, built for their programs (pan_shc's) */
    for (vs, fs) in [(td!("cmp_vs4.vert"), td!("cmp_b.frag")), (td!("cmp_vs2x4.vert"), td!("cmp_axby.frag"))] {
        let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
        let pr = super::fragment_reads(&f.info.shader()).expect("the record's moved expressions encode");
        seen |= module_facts_for(SHC_STAGE_VERTEX, vs, Some(pr.reads())).0;
    }
    let missing: Vec<_> = super::nir::Prod::ALL.iter().filter(|p| seen & (1 << **p as u32) == 0).collect();
    assert!(missing.is_empty(), "no reference draws {missing:?}");
}

/// The selection's rows (emit.rs `ROWS`, `bi_emit_alu`'s cases) are exactly
/// the ones the references draw, the vertex stage's and the fragment
/// graph's: a row no reference has fails here.
#[test]
fn the_selection_rows_are_the_references() {
    let mut seen: Vec<super::emit::Row> = Vec::new();
    for src in vs_reference_sources() {
        let _k = store::test_keep();
        store::with_scratch(|s| {
            let l = codegen::lower_source(SHC_STAGE_VERTEX, src.as_bytes(), &[], s).expect("the front end takes it");
            let vs = super::nir::build(&l.module, s, None, &[]).expect("the vertex program");
            for &id in &vs.order[..vs.len] {
                let nd = vs.nodes[id as usize];
                if let super::nir::Kind::Alu(op) = nd.kind {
                    let row = super::emit::Row { op, bits: nd.bits, width: nd.width };
                    if !seen.contains(&row) {
                        seen.push(row);
                    }
                }
            }
        })
        .expect("the scratch arena is free");
    }
    /* the fragment populations, through the graph (nir_fs.rs) */
    let mut fs_rows = 0u64;
    for src in fs_reference_sources() {
        let _k = store::test_keep();
        fs_rows |= store::with_scratch(|s| {
            let l = codegen::lower_source(SHC_STAGE_FRAGMENT, src.as_bytes(), &[], s).expect("the front end takes it");
            let text = s.alloc_str(src).expect("room");
            let st = crate::shaderc::online::sema::Stage::Fragment;
            let decls = crate::shaderc::online::reflect::interface(s, st, text).expect("the interface");
            let fs = super::FsPrecision::of(&l.module, text, s).expect("the precision");
            super::compile_module_on(&l.module, decls.uniforms, fs, None, &[], None, s).expect("compiles").rows
        })
        .expect("the scratch arena is free");
    }
    for (i, r) in super::emit::ROWS.iter().enumerate() {
        if fs_rows & (1 << i) != 0 && !seen.contains(&r.0) {
            seen.push(r.0);
        }
    }
    let rows: Vec<_> = super::emit::ROWS.iter().map(|r| r.0).collect();
    let unknown: Vec<_> = seen.iter().filter(|r| !rows.contains(r)).collect();
    let undrawn: Vec<_> = rows.iter().filter(|r| !seen.contains(r)).collect();
    assert!(
        unknown.is_empty() && undrawn.is_empty(),
        "drawn but not a row: {unknown:?}; a row nothing draws: {undrawn:?}"
    );
}

/// The fragment shaders of the references: the samples and the fragment
/// populations.
fn fs_reference_sources() -> Vec<&'static str> {
    let mut srcs: Vec<&str> = vec![TRI_FS, TV_FS, SAMPLE_FS];
    srcs.extend(POPULATION.iter().map(|c| c.2));
    srcs.extend(UNIFORM_POPULATION.iter().map(|c| c.2));
    srcs.extend(FRAGMENT_VARYING_POPULATION.iter().map(|c| c.2));
    srcs.extend(FS_CONST_POPULATION.iter().map(|c| c.2));
    srcs.extend(FS_NEG_POPULATION.iter().map(|c| c.2));
    srcs.extend(FS_ABS_POPULATION.iter().map(|c| c.2));
    srcs.extend(HIGHP_POPULATION.iter().map(|c| c.2));
    srcs.extend(S4_POPULATION.iter().map(|c| c.2));
    srcs.extend(S5_POPULATION.iter().map(|c| c.2));
    /* the constant coordinate and the run-time index (#1385 (b)) */
    srcs.extend([td!("tex_coord_const.frag"), td!("vtl_fs.frag"), td!("uniidx_d.frag"), td!("uniidx_i.frag")]);
    /* a mediump computed coordinate (plan #153 texcoord) */
    srcs.push(td!("case121.frag"));
    srcs.extend(UNIARRAY_POPULATION.iter().map(|c| c.2));
    srcs.extend(ALU_POPULATION.iter().map(|c| c.2));
    srcs
}

/// The vertex shaders of the references: the populations, the samples, the
/// point size and the constant product cases.
fn vs_reference_sources() -> Vec<&'static str> {
    let mut srcs: Vec<&str> = UNIFORM_POPULATION.iter().map(|c| c.1).collect();
    srcs.extend(POPULATION.iter().map(|c| c.1));
    srcs.extend(VARYING_POPULATION.iter().map(|c| c.1));
    srcs.extend([TRI_VS, TV_VS, SAMPLE_VS]);
    /* the point size cases (pan_shc's references) */
    srcs.extend([td!("ctl_psiz.vert"), td!("ctl_psiz_const.vert"), POINTS_VS]);
    /* the constant products (pan_shc's references) */
    srcs.extend([
        td!("cmul_m075.vert"),
        td!("cmul_mneg.vert"),
        td!("cmul_m2.vert"),
        td!("cdiv_d2.vert"),
        td!("cmul_negprod.vert"),
    ]);
    /* the vertex look-ups (the shim's, #1385), the cube's among them */
    srcs.extend([td!("tex_vs_lod.vert"), td!("tex_vs_zero.vert"), td!("cube_vs.vert")]);
    srcs
}

/* ---- vertex-stage look-ups (#1385 (a)) ------------------------------ */

/// Case 197's vertex shader (gl_test `vtxgrid`), `texture2DLod` at a
/// uniform level: the varying program holds the look-up (only `v_color`
/// reads it), EXPLICIT with the 8.8 level computed first, the shim's
/// program to the byte (tools/mali/testdata/tex_vs_lod.expect.json,
/// `drm_shim_ref.sh run ... --uniform-values`: u_lod is 32).
#[test]
fn a_vertex_look_up_at_a_uniform_level_is_byte_equal_to_mesa_26_2_1() {
    let vs = td!("tex_vs_lod.vert");
    let (v, _) = check_program(vs, td!("tex_vs_color.frag"), td!("tex_vs_lod.expect.json"));
    assert_eq!(v.info.sampler, Some(0), "the look-up's sampler binding");
    /* the level's FAU words after the position's: u_lod, 1/16, 4096 and
     * the handle (the trace's words 8-11) */
    assert_eq!(
        v.info.fau[8..],
        [
            FauWord::Uniform { dword: 0 },
            FauWord::Constant(0x3d80_0000),
            FauWord::Constant(0x4580_0000),
            FauWord::Constant(super::nir_fs::TEX_HANDLE)
        ]
    );
}

/// The same look-up at the constant level 0.0 (`vtl_vs_grid.vert`):
/// ZERO_LOD, no level word.
#[test]
fn a_vertex_look_up_at_level_zero_is_byte_equal_to_mesa_26_2_1() {
    let (v, _) = check_program(td!("tex_vs_zero.vert"), td!("tex_vs_color.frag"), td!("tex_vs_zero.expect.json"));
    assert_eq!(v.info.sampler, Some(0));
}

/* ---- the cube-map look-up (#1385 (c), case 113) ---------------------- */

/// gl_test case 113's vertex shader (tools/shaders/es100/cubemap.vert as
/// tools/mali/testdata/cube_vs.vert): `textureCubeLod(u_s, u_dir, u_lod)`
/// at a uniform direction and level. The varying program holds Mesa's cube
/// descriptor sequence (`build_cube_desc`: CUBEFACE1, CUBEFACE2_V9,
/// CUBE_SSEL, CUBE_TSEL, FRCP, `FMA 0.5, r`, two `FMA.clamp_0_1` with the
/// saturates folded in, MUX.i32.bit with the mask 0x1fffffff), the 8.8
/// level, `TEX_SINGLE.cube.explicit` and, since the look-up reads no
/// attribute, the convergent varying's `FMA.f32 x, 0x0, x` per lane that
/// Mesa's link stores when it promotes the varying to flat: both IDVS
/// programs the shim's to the byte (tools/mali/testdata/cube_vs.expect.json,
/// `drm_shim_ref.sh run ... --uniform-values`: u_dir is (32, 33, 34),
/// u_lod 48).
///
/// The fragment shader is case 113's pass-through (cubelod.frag). Mesa's
/// link reads the promoted varying flat (`LD_VAR_BUF_IMM.f32 ...
/// .src_flat32 ..., 0x0` where ours interpolates, `.src_f32 ..., r61`;
/// nir_opt_varyings.c:4964-4985), a link-side promotion this back end does
/// not make yet: that one word is pinned here and every other word and
/// fact of the fragment program is equal. The picture is the same (the
/// interpolation of one value is that value).
#[test]
fn a_vertex_cube_look_up_is_byte_equal_to_mesa_26_2_1() {
    const VS: &str = td!("cube_vs.vert");
    const FS: &str = td!("cube_vs_color.frag");
    const JSON: &str = td!("cube_vs.expect.json");
    let v = check_vs_in(VS, FS, JSON, VIEWPORT_64X64);
    assert_eq!(v.info.sampler, Some(0), "the look-up's sampler binding");
    /* the FAU words after the position's eight and the store offsets: the
     * direction and the level (words 11-14), the mask, 1/16, 4096, the
     * handle (words 15-18; the trace's) */
    assert_eq!(
        v.info.fau[11..],
        [
            FauWord::Uniform { dword: 0 },
            FauWord::Uniform { dword: 1 },
            FauWord::Uniform { dword: 2 },
            FauWord::Uniform { dword: 3 },
            FauWord::Constant(0x1fff_ffff),
            FauWord::Constant(0x3d80_0000),
            FauWord::Constant(0x4580_0000),
            FauWord::Constant(super::nir_fs::TEX_HANDLE)
        ]
    );
    let (_, var) = vs_regions(&v);
    let cube = var.iter().map(|w| text(*w)).filter(|t| t.starts_with("TEX_SINGLE")).count();
    assert_eq!(cube, 1);
    assert!(var.iter().any(|w| text(*w).starts_with("TEX_SINGLE") && text(*w).contains(".cube.explicit")));
    assert_eq!(var.iter().filter(|w| text(**w).starts_with("FMA.f32 r") && text(**w).contains(", 0x0, r")).count(), 4);
    /* the fragment program: the flat load alone */
    let f = compile(SHC_STAGE_FRAGMENT, FS).expect("the fragment shader compiles");
    let rf = region(JSON, "fs.fragment").expect("fs.fragment");
    let ours = trim(&f.code);
    assert_eq!(ours.len(), rf.words.len(), "the fragment program's length");
    let differ: Vec<usize> = (0..ours.len()).filter(|&k| ours[k] != rf.words[k]).collect();
    assert_eq!(differ, [0], "only the varying load differs:\n{}", {
        let mut s = String::new();
        for k in &differ {
            s += &format!("  {k}: ours {}  mesa {}\n", text(ours[*k]), text(rf.words[*k]));
        }
        s
    });
    assert!(text(ours[0]).contains(".src_f32.") && text(ours[0]).contains(", r61^,"), "{}", text(ours[0]));
    assert!(
        text(rf.words[0]).contains(".src_flat32.") && text(rf.words[0]).contains(", 0x0,"),
        "{}",
        text(rf.words[0])
    );
    assert_facts("fs.fragment", &rf, f.info.work_reg_count, f.info.preload, &f.info.fau, VIEWPORT_64X64, &|_| 0);
}

/// The cube look-up's gaps beside the reference: a direction with a
/// constant lane (case 101's cubelod.vert) is refused by name; a direction
/// from an attribute is divergent, so its varying is stored as it is (no
/// `FMA x, 0, x`; the shape of the reference without the level and the
/// promotion); a uniform alone stays refused (Mesa rewrites it in an order
/// this back end has no reference for, `varyings_mesa_s_link_moves_or_merges_are_refused`).
#[test]
fn a_cube_direction_from_an_attribute_is_divergent() {
    let src = "uniform samplerCube s; attribute vec4 p; varying vec4 v; void main(){ gl_Position = p; v = textureCubeLod(s, p.xyz, 0.0); }";
    let v = compile(SHC_STAGE_VERTEX, src).expect("compiles");
    let (_, var) = vs_regions(&v);
    let texts: Vec<String> = var.iter().map(|w| text(*w)).collect();
    assert!(texts.iter().any(|t| t.starts_with("TEX_SINGLE") && t.contains(".cube.zero")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("FMA.f32 r") && t.contains(", 0x0, r")), "{texts:?}");
    for op in ["CUBEFACE1", "CUBEFACE2_V9", "CUBE_SSEL", "CUBE_TSEL", "MUX.i32.bit"] {
        assert_eq!(texts.iter().filter(|t| t.starts_with(op)).count(), 1, "{op}");
    }
    /* tools/shaders/es100/cubelod.vert's look-up: `vec3(1.0, 0.0, 0.0)` */
    let constant = "attribute vec4 a_pos; uniform samplerCube u_s; uniform float u_lod; varying vec4 v_col; void main(){ gl_Position = a_pos; v_col = textureCubeLod(u_s, vec3(1.0, 0.0, 0.0), u_lod); }";
    assert_eq!(gap(SHC_STAGE_VERTEX, constant), MaliError::TexVertexConstant);
}

/// gl_test case 114's vertex stage (`cubelod`, prog[0]: tools/shaders/
/// es300/cubelodv.vert as tools/mali/testdata/case114_v.vert):
/// `textureLod(u_s, vec3(1.0, a_pos.y * 0.95, a_pos.x * 0.95), u_lod)`. The
/// constant lane is an operand of the cube ops, an immediate of the ISA's
/// table (`CUBEFACE1 r2, 0x3F800000, r1, r0`). Its fragment shader stores
/// the varying whole as a highp ES 3.00 colour, at fp32 (`ATEST ... r3`,
/// `BLEND.v4.f32`, no conversion). All three programs the shim's to the
/// byte (case114_v.expect.json, `--uniform-values`).
#[test]
fn a_vertex_cube_look_up_at_a_constant_lane_is_byte_equal_to_mesa_26_2_1() {
    let (v, f) = check_program(td!("case114_v.vert"), td!("case114_v.frag"), td!("case114_v.expect.json"));
    assert!(!f.info.colour_f16, "a highp ES 3.00 colour is stored at fp32");
    assert_eq!(v.info.sampler, Some(0));
    let (_, var) = vs_regions(&v);
    let texts: Vec<String> = var.iter().map(|w| text(*w)).collect();
    for op in ["CUBEFACE1", "CUBEFACE2_V9", "CUBE_SSEL"] {
        assert!(texts.iter().any(|t| t.starts_with(op) && t.contains("0x3F800000")), "{op}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t.starts_with("TEX_SINGLE") && t.contains(".cube.explicit")), "{texts:?}");
}

/// gl_test case 41 (`es3cube`, tools/shaders/es300/cube.{vert,frag} as
/// tools/mali/testdata/case41.*): `textureLod(cube, axis + vec3(v_xy, 0.0)
/// * 0.8, 0.0)` in a mediump fragment shader. The direction is computed in
/// fp16 (`FMA.v2f16` of the pair x, y with 0.8 rounded to fp16 as
/// lower_precision does, 0x3a66; z the uniform's half), each lane converted
/// (`F16_TO_F32`) for the fp32 cube ops, `build_cube_desc`'s sequence, then
/// `TEX_SINGLE.16.cube.zero` at the two staging words; the texel's collect
/// stands before ATEST. Both programs the shim's to the byte
/// (case41.expect.json, `--uniform-values`).
#[test]
fn a_fragment_cube_look_up_is_byte_equal_to_mesa_26_2_1() {
    let (_, f) = check_program(td!("case41.vert"), td!("case41.frag"), td!("case41.expect.json"));
    assert_eq!(f.info.sampler, Some(0));
    assert!(!f.info.helper_threads, "a look-up at level 0 needs no helper invocation");
    assert!(f.info.fau.contains(&FauWord::Constant(0x3a66_3a66)), "0.8 rounded to fp16: {:?}", f.info.fau);
    let texts: Vec<String> = trim(&f.code).iter().map(|w| text(*w)).collect();
    assert_eq!(texts.iter().filter(|t| t.starts_with("F16_TO_F32")).count(), 3, "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("TEX_SINGLE") && t.contains(".16.cube.zero")), "{texts:?}");
    /* the gaps beside it: an implicit level (`textureCube`'s computed LOD)
     * and a mediump explicit level have no reference */
    let fs = |lod: &str| {
        format!(
            "#version 300 es\nprecision mediump float; uniform samplerCube c; uniform float l; in vec2 v; out vec4 o; void main(){{ o = {lod}; }}"
        )
    };
    assert_eq!(gap(SHC_STAGE_FRAGMENT, &fs("texture(c, vec3(v, 1.0))")), MaliError::TexKind);
    assert_eq!(gap(SHC_STAGE_FRAGMENT, &fs("textureLod(c, vec3(v, 1.0), l)")), MaliError::TexKind);
}

/// gl_test case 114's fragment stage (`cubelod`, prog[1]: tools/shaders/
/// es300/cubelodf.{vert,frag} as tools/mali/testdata/case114_f.*):
/// `textureLod(u_s, v_dir, u_lod)` in a highp shader with a highp sampler,
/// `v_dir = vec3(1.0, a_pos.y * 0.95, a_pos.x * 0.95)`. Mesa's link
/// propagates the constant lane into the fragment shader (nir_opt_varyings)
/// and stores the two others, which the link step does too (mod.rs
/// `vs_constants`, `compile_fs_for_vs`, then the vertex shader for the
/// reads): `build_cube_desc` at fp32 with the constant an operand
/// (`CUBEFACE1.discard r2, 0x3F800000, r0, r1`), the 8.8 level,
/// `TEX_SINGLE.32.cube.explicit` at three staging words (the sampler's
/// highp keeps the texel 32-bit), and the colour stored at fp32 (`ATEST
/// ... r3`, `BLEND.v4.f32`). All three programs the shim's to the byte
/// (case114_f.expect.json, `--uniform-values`). The vertex shader alone
/// stays refused: it stores the constant, which no program of Mesa's does.
#[test]
fn a_highp_fragment_cube_look_up_is_byte_equal_to_mesa_26_2_1() {
    let (vs, fs) = (td!("case114_f.vert"), td!("case114_f.frag"));
    let (v, f) = check_linked(vs, fs, td!("case114_f.expect.json"));
    assert!(!f.info.colour_f16, "a highp ES 3.00 colour is stored at fp32");
    assert!(!f.info.helper_threads, "a look-up at a given level needs no helper invocation");
    assert_eq!(f.info.sampler, Some(0));
    /* v_dir's two computed lanes, compacted at offset 0 */
    let generic = |s: &Stage| s.info.varyings.iter().filter(|x| x.section == Section::Generic).count();
    assert_eq!((generic(&v), generic(&f)), (1, 1));
    assert!(f.info.varyings.iter().any(|x| x.offset == 0 && x.mask == 0b110), "{:?}", f.info.varyings);
    let texts: Vec<String> = trim(&f.code).iter().map(|w| text(*w)).collect();
    assert!(texts.iter().any(|t| t.starts_with("TEX_SINGLE") && t.contains(".32.cube.explicit")), "{texts:?}");
    /* the shader alone: the vertex shader stores the constant (refused),
     * the fragment shader reads the three lanes */
    assert_eq!(gap(SHC_STAGE_VERTEX, vs), MaliError::VaryingNotFromAttribute);
    let alone = compile(SHC_STAGE_FRAGMENT, fs).expect("case114_f.frag compiles alone");
    assert!(alone.info.varyings.iter().any(|x| x.mask == 0b111), "{:?}", alone.info.varyings);
}

/// The link step on a mediump pair (sel_v_const: `v_c = vec4(a_t, 0.0,
/// 1.0)`, `gl_FragColor = v_c`): the two constant lanes are the colour's
/// constant word (`MOV.i32 r1, 0x3C000000`, ATEST at `0x3F800000`), the
/// two lanes left one `.v2` load; the vertex shader stores two words.
/// Every program the shim's to the byte (sel_v_const.expect.json).
#[test]
fn a_constant_varying_lane_is_propagated_into_the_fragment_shader() {
    let (v, f) = check_linked(td!("sel_v_const.vert"), td!("sel_v_const.frag"), td!("sel_v_const.expect.json"));
    assert!(f.info.colour_f16, "gl_FragColor is mediump");
    assert!(v.info.varyings.iter().any(|x| x.section == Section::Generic && x.mask == 0b11), "{:?}", v.info.varyings);
    /* a vertex shader with no constant lane: the link step changes nothing */
    let (vs, fs) = (td!("tri_varying.vert"), td!("tri_varying.frag"));
    let (lv, lf) = compile_linked(vs, fs);
    let (av, af) = (compile(SHC_STAGE_VERTEX, vs).expect("vs"), compile(SHC_STAGE_FRAGMENT, fs).expect("fs"));
    assert_eq!((lv.code, lf.code), (av.code, af.code));
}

/* ---- the constant coordinate and the run-time index (#1385 (b)) ------ */

/// A fragment look-up at the constant coordinate `vec2(0.5)` alone
/// (tools/mali/testdata/tex_coord_const.expect.json): the two constants
/// are FAU words moved into the staging registers (`MOV.i32 r0, u2.w0`,
/// `MOV.i32 r1, u2.w0`), the look-up as a varying coordinate's.
#[test]
fn a_look_up_at_a_constant_coordinate_is_byte_equal_to_mesa_26_2_1() {
    let (_, f) = check_program(td!("tri.vert"), td!("tex_coord_const.frag"), td!("tex_coord_const.expect.json"));
    assert_eq!(f.info.sampler, Some(0));
    assert!(f.info.helper_threads, "an implicit-LOD look-up");
    assert!(f.info.ubo.is_empty(), "no block is loaded");
    assert_eq!(&f.info.fau[4..], [FauWord::Constant(0x3f00_0000), FauWord::Constant(super::nir_fs::TEX_HANDLE)]);
}

/// Case 197's fragment shader (gl_test `vtxgrid`, tools/shaders/gltest/
/// vtl_fs.frag) with its vertex shader: `mix(v_color, texture2D(u_ftex,
/// vec2(0.5, 0.5)), u_pick)`, the varying loaded first, the look-up at the
/// constants, then `FADD.v2f16 t, v.neg` and `FMA.v2f16 u_pick.h00, d, v`
/// (tools/mali/testdata/vtl_fs.expect.json, `--uniform-values`: u_lod 32,
/// u_pick 64). `u_pick` is the fp16 block's one pushed word.
#[test]
fn case_197_s_fragment_shader_is_byte_equal_to_mesa_26_2_1() {
    let (v, f) = check_program(td!("tex_vs_lod.vert"), td!("vtl_fs.frag"), td!("vtl_fs.expect.json"));
    assert_eq!((v.info.sampler, f.info.sampler), (Some(0), Some(0)));
    assert!(f.info.ubo.is_empty(), "every uniform read is pushed");
    assert_eq!(
        &f.info.fau[4..],
        [
            FauWord::UniformHalves { lo: 0, hi: None },
            FauWord::Constant(0x3f00_0000),
            FauWord::Constant(super::nir_fs::TEX_HANDLE)
        ]
    );
}

/// gl_test case 108 D's fragment shader (tools/shaders/es100/uniidx_d.frag):
/// a mediump `vec4 fcols[4]` read at the int uniform `fidx`, the colour.
/// Mesa's `LD_PKA.i64` of UBO 0 at `fidx << 3` (the pushed int word
/// shifted by the table's byte `0x3020100.b3`, moved to a register as the
/// instruction's third FAU word), its two words the colour
/// (tools/mali/testdata/uniidx_d.expect.json; the passthrough vertex
/// shader's varying is unread, so the program has no varying stage). The
/// record carries the block the frame uploads: the four elements at words
/// 0-7 as fp16 pairs, `fidx` at word 8, 36 bytes.
#[test]
fn a_vec4_array_read_at_a_run_time_index_is_byte_equal_to_mesa_26_2_1() {
    check_program_for("uniidx_d", td!("uniidx_i.vert"), td!("uniidx_d.frag"), td!("uniidx_d.expect.json"), false);
    let f = compile(SHC_STAGE_FRAGMENT, td!("uniidx_d.frag")).expect("compiles");
    assert_eq!(&f.info.fau[4..], [FauWord::Uniform { dword: 16 }]);
    assert_eq!(f.info.ubo, ubo_rows_of_fcols_and_fidx());
    assert!(f.info.sampler.is_none() && !f.info.helper_threads);
}

/// gl_test case 121 `uniidxh` (tools/shaders/es100/uniidx_h.frag, as
/// tools/mali/testdata/case121.frag): `0.5 * (c + texture2D(ftex,
/// vec2(c.w * 0.5)))` with `c = fcols[fidx]`, a look-up at a computed
/// coordinate in a mediump shader (plan #153 texcoord). Mesa's `LD_PKA.i64` of the element,
/// then the coordinate as one fp16 lane (`FMA.v2f16 r2, r1.h11,
/// 0x38000000.h11, 0x0.neg`), converted once (`F16_TO_F32 r2, r2^.h0`) and
/// moved into the second staging word (`MOV.i32 r3, r2`), the look-up
/// `.16` (tools/mali/testdata/case121.expect.json,
/// docs/notes/mali-texture-refusals.md section (d)).
#[test]
fn a_mediump_computed_texture_coordinate_is_byte_equal_to_mesa_26_2_1() {
    check_program_for("case121", td!("case121.vert"), td!("case121.frag"), td!("case121.expect.json"), false);
    let f = compile(SHC_STAGE_FRAGMENT, td!("case121.frag")).expect("compiles");
    assert_eq!(f.info.ubo, ubo_rows_of_fcols_and_fidx());
    assert!(f.info.sampler == Some(0) && f.info.helper_threads);
}

/// The float uniform array population (#1385, plan #153 uniarray): gl_test
/// case 108 A (tools/shaders/es100/uniidx_a.frag, dEQP-GLES2
/// float_dynamic_read: four reads of a mediump `float u_arr[4]` at four int
/// uniforms, summed). Mesa's NIR loads the ints in one vec4 `load_ubo`,
/// pushed as `u2.w0 .. u3.w1`; each read is `ishl idx, 2` and `iadd 0x10`
/// (`LSHIFT_OR.i32 .., 0x3020100.b22`, `IADD_IMM.i32 #0x10`), then a
/// 16-bit `load_ubo` (`LD_PKA.i16`) moved down to its first reader, the sum
/// one lane (`FADD.v2f16 a.h00, b.h00`), the red and green word `U16_TO_U32`
/// (docs/notes/mali-uniform-arrays.md). Programs B and C (uniarray_b,
/// uniarray_c: one read beside 0.0; C's other arithmetic folds away, so
/// Mesa's program is B's): the offset, ATEST, then the load sunk to the
/// colour's word. uniarray_vsconst: a vertex shader's vec4 array at
/// constant indices (cases 149 and 150's shape), each element one vec4
/// load pushed to the FAU as a vec4 uniform's is. Each byte-equal to the
/// shim in both stages.
const UNIARRAY_POPULATION: [(&str, &str, &str, &str, bool); 4] = [
    (
        "uniarray_vsconst",
        td!("uniarray_vsconst.vert"),
        td!("uniarray_vs.frag"),
        td!("uniarray_vsconst.expect.json"),
        false,
    ),
    (
        "case108_uniidx",
        td!("case108_uniidx.vert"),
        td!("case108_uniidx.frag"),
        td!("case108_uniidx.expect.json"),
        false,
    ),
    ("uniarray_b", td!("case108_uniidx.vert"), td!("uniarray_b.frag"), td!("uniarray_b.expect.json"), false),
    ("uniarray_c", td!("case108_uniidx.vert"), td!("uniarray_c.frag"), td!("uniarray_c.expect.json"), false),
];

#[test]
fn the_uniform_array_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json, values) in UNIARRAY_POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program_for(name, vs, fs, json, values)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), UNIARRAY_POPULATION.len(), failed.join("\n"));
}

/// Case 108 B's read moved to the vertex stage (uniarray_vs: `v_color =
/// vec4(u_arr[ui_two], 0.0, 0.0, 1.0)`, plan #153 uniarray-int): Mesa's
/// link propagates the three constant lanes into the fragment shader, so
/// the program's vertex shader stores `v_color.x` alone, a convergent lane
/// promoted to flat (`FMA.f32 r0, r0^, 0x0, r0^`). The read is the 32-bit
/// `load_ubo(0, 4 + (ui_two << 2))`: `LSHIFT_OR.i32 r0, u5.w1,
/// 0x3020100.b22, 0x0`, `IADD_IMM.i32 #0x4`, `LD_PKA.i32` of UBO 0 (`ui_two`
/// word 0 of the block, the array words 1-4), `ui_two` the pushed word
/// after the store offsets. Both IDVS programs the shim's to the byte
/// (uniarray_vs.expect.json; no `--uniform-values`: gl_ref.py sets no int
/// uniform, so every pushed uniform word is 0). The vertex shader is built
/// for the fragment shader's read as the link leaves it (`v_color.x`):
/// the linked fragment shader itself is refused at its colour pair of
/// that lane and 0.0 (`U16_TO_U32`, [`MaliError::ColourZeroHalf`]; Mesa
/// also reads the lane flat, `LD_VAR_BUF_IMM.f32 ... .src_flat32 ...,
/// 0x0`, the promotion `a_vertex_cube_look_up_is_byte_equal_to_mesa_26_2_1`
/// pins), neither of which is the array's.
#[test]
fn a_vertex_float_array_read_at_an_int_uniform_is_byte_equal_to_mesa_26_2_1() {
    let (vs, fs, json) = (td!("uniarray_vs.vert"), td!("uniarray_vs.frag"), td!("uniarray_vs.expect.json"));
    let alone = compile(SHC_STAGE_FRAGMENT, fs).expect("uniarray_vs.frag compiles alone");
    let loc = alone.info.varyings.iter().find_map(|x| x.varying).expect("v_color");
    let _k = store::test_keep();
    let v = store::with_scratch(|s| {
        super::compile_vs_for_reads(vs.as_bytes(), &[(loc, 0b0001)], &[], s, store::keep()).map(|(sh, _)| stage_of(&sh))
    })
    .expect("the scratch arena is free")
    .expect("the vertex shader compiles for the program's read");
    let (pos, var) = vs_regions(&v);
    let vp = VIEWPORT_64X64;
    let rp = region(json, "vs.position").expect("vs.position");
    assert_words("vs.position", &pos, &rp.words);
    assert_facts("vs.position", &rp, v.info.work_reg_count, v.info.preload, &v.info.fau, vp, &|_| 0);
    let rv = region(json, "vs.varying").expect("vs.varying");
    assert_words("vs.varying", &var, &rv.words);
    let sec = v.info.secondary.expect("a varying program");
    assert_facts("vs.varying", &rv, sec.work_reg_count, sec.preload, &v.info.fau, vp, &|_| 0);
    assert_eq!(Some(sec.offset), secondary_offset(json), "secondary_offset");
    /* the block the frame uploads: `ui_two`, then the four elements */
    let dword = |name: &str| v.uniforms.iter().find(|r| r.0 == name).map(|r| r.1).expect("a uniform row");
    let (idx, arr) = (dword("ui_two"), dword("u_arr"));
    assert_eq!(
        v.info.ubo,
        [
            super::UboRow { dword: idx, word: 0, comps: 1, halves: false },
            super::UboRow { dword: arr, word: 1, comps: 4, halves: false }
        ]
    );
    assert_eq!(v.info.fau[11], FauWord::Uniform { dword: idx });
    /* the linked fragment shader's own gap */
    let consts = store::with_scratch(|s| super::vs_constants(vs.as_bytes(), s))
        .expect("the scratch arena is free")
        .expect("the vertex shader's constant lanes");
    let f = store::with_scratch(|s| super::compile_fs_for_vs(fs.as_bytes(), consts, s, store::keep()).map(|_| ()))
        .expect("the scratch arena is free");
    assert!(matches!(f, Err(CompileError::BackEnd(codegen::BackEndError::Mali(MaliError::ColourZeroHalf)))), "{f:?}");
}

/// Case 108 A at highp (uniarray_highp, plan #153 uniarray-int): Mesa keeps
/// the block fp32, so each read is the 32-bit `load_ubo(0, 0x10 + (idx <<
/// 2))`: the same `LSHIFT_OR.i32 .., 0x3020100.b22` and `IADD_IMM.i32
/// #0x10` as the mediump program, then `LD_PKA.i32` moved down to its first
/// reader, the sum `FADD.f32`, the red lane converted at the store (`FADD.f32
/// r0.h0, r0^, 0x0.neg`, `U16_TO_U32`). Both stages the shim's to the byte
/// (uniarray_highp.expect.json). The record: the block the frame uploads is
/// the four ints at words 0-3, then `u_arr`'s four fp32 elements at words
/// 4-7 (one row, a word an element); the FAU the blend constants, then the
/// four ints.
#[test]
fn a_highp_float_array_read_at_int_uniforms_is_byte_equal_to_mesa_26_2_1() {
    let (vs, fs) = (td!("case108_uniidx.vert"), td!("uniarray_highp.frag"));
    check_program_for("uniarray_highp", vs, fs, td!("uniarray_highp.expect.json"), false);
    let f = compile(SHC_STAGE_FRAGMENT, fs).expect("compiles");
    let mut rows: Vec<super::UboRow> =
        (0..4).map(|e| super::UboRow { dword: e, word: e, comps: 1, halves: false }).collect();
    rows.push(super::UboRow { dword: 4, word: 4, comps: 4, halves: false });
    assert_eq!(f.info.ubo, rows);
    assert_eq!(&f.info.fau[4..], (0..4).map(|d| FauWord::Uniform { dword: d }).collect::<Vec<_>>());
}

/// An int uniform is read as an array index only: one read as a value
/// stays refused at the uniform, in either fp32 stage, as before.
#[test]
fn an_int_uniform_outside_an_array_index_stays_refused() {
    let vs = "attribute vec4 p; uniform int i; uniform float a[4]; varying vec4 v;
              void main(){ gl_Position = p; v = vec4(a[i] + float(i)); }";
    assert_eq!(gap(SHC_STAGE_VERTEX, vs), MaliError::UniformType);
    let fs = "precision highp float; uniform highp int i; uniform highp float a[4];
              void main(){ gl_FragColor = vec4(a[i] + float(i)); }";
    assert_eq!(gap(SHC_STAGE_FRAGMENT, fs), MaliError::UniformType);
}

/// Case 108 A's record: the block the frame uploads is the four ints at
/// words 0-3, one word each, then `u_arr`'s elements at words 4-7, one fp16
/// half in each word's low half (Mesa's `vec4[2] uniform_0`, 32 bytes); the
/// FAU the blend constants, then the four ints.
#[test]
fn case_108_a_s_block_holds_the_ints_then_the_array_a_word_an_element() {
    let f = compile(SHC_STAGE_FRAGMENT, td!("case108_uniidx.frag")).expect("compiles");
    let mut rows: Vec<super::UboRow> =
        (0..4).map(|e| super::UboRow { dword: e, word: e, comps: 1, halves: false }).collect();
    rows.extend((0..4).map(|e| super::UboRow { dword: 4 + e, word: 4 + e, comps: 1, halves: true }));
    assert_eq!(f.info.ubo, rows);
    assert_eq!(&f.info.fau[4..], (0..4).map(|d| FauWord::Uniform { dword: d }).collect::<Vec<_>>());
}

/// The block of `vec4 fcols[4]` then `int fidx` (uniidx_d, uniidx_i): each
/// element two words of fp16 pairs, the int one word, 36 bytes
/// (mesa-uniidxi.pandecode:261's `Size: 36`).
fn ubo_rows_of_fcols_and_fidx() -> Vec<super::UboRow> {
    let mut rows: Vec<super::UboRow> =
        (0..4).map(|e| super::UboRow { dword: 4 * e, word: 2 * e, comps: 4, halves: true }).collect();
    rows.push(super::UboRow { dword: 16, word: 8, comps: 1, halves: false });
    rows
}

/// Case 122's fragment shader (gl_test `uniidxi`, tools/shaders/es100/
/// uniidx_i.frag): the look-up at `vec2(0.5)`, then `fcols[fidx + int(t.w *
/// 0.0)]`, whose index folds to `fidx` (no F32_TO_S32, no IADD in Mesa's
/// program: `t.w * 0.0` is 0.0), then `0.5 * (fcols[..] + t)` as two
/// `FADD.v2f16` and two `FMA.v2f16` by the table's 0.5 half
/// (tools/mali/testdata/uniidx_i.expect.json). The FAU: the blend
/// constants, `fidx`, then the promoted 0.5 and the handle.
#[test]
fn case_122_s_fragment_shader_is_byte_equal_to_mesa_26_2_1() {
    check_program_for("uniidx_i", td!("uniidx_i.vert"), td!("uniidx_i.frag"), td!("uniidx_i.expect.json"), false);
    let f = compile(SHC_STAGE_FRAGMENT, td!("uniidx_i.frag")).expect("compiles");
    assert_eq!(
        &f.info.fau[4..],
        [FauWord::Uniform { dword: 16 }, FauWord::Constant(0x3f00_0000), FauWord::Constant(super::nir_fs::TEX_HANDLE)]
    );
    assert_eq!(f.info.ubo, ubo_rows_of_fcols_and_fidx());
    assert_eq!(f.info.sampler, Some(0));
}

fn prods(src: &str) -> u32 {
    module_facts(SHC_STAGE_VERTEX, src).0
}

/// A stage through `compile_module`: its productions and its use of each
/// table ([`super::Table`]).
fn module_facts(stage: u32, src: &str) -> (u32, [usize; super::Table::ALL.len()]) {
    module_facts_for(stage, src, None)
}

/// [`module_facts`] through `compile_module_for`, a vertex shader built for
/// a fragment shader's `reads`.
fn module_facts_for(stage: u32, src: &str, reads: Option<&[(u8, u8)]>) -> (u32, [usize; super::Table::ALL.len()]) {
    let _k = store::test_keep();
    store::with_scratch(|s| {
        let l = codegen::lower_source(stage, src.as_bytes(), &[], s).expect("the front end takes it");
        let st = if stage == SHC_STAGE_VERTEX {
            crate::shaderc::online::sema::Stage::Vertex
        } else {
            crate::shaderc::online::sema::Stage::Fragment
        };
        let text = s.alloc_str(src).expect("room");
        let decls = crate::shaderc::online::reflect::interface(s, st, text).expect("the interface");
        let fs = if stage == SHC_STAGE_FRAGMENT {
            super::FsPrecision::of(&l.module, text, s).expect("the precision")
        } else {
            super::FsPrecision::default()
        };
        let sc = super::compile_module_on(&l.module, decls.uniforms, fs, reads, &[], None, s).expect("compiles");
        (sc.prods, sc.marks)
    })
    .expect("the scratch arena is free")
}

/// Every fixed table's use is measured and published: the most each
/// referenced program used (pinned, so a change that moves it is seen), and
/// the process's high-water mark ([`super::high_water`], the number the log
/// line prints when it rises) at least that much and never past the table.
#[test]
fn the_tables_high_water_marks_are_measured() {
    use super::Table;
    let mut srcs: Vec<(u32, &str)> = vec![
        (SHC_STAGE_VERTEX, TRI_VS),
        (SHC_STAGE_FRAGMENT, TRI_FS),
        (SHC_STAGE_VERTEX, TV_VS),
        (SHC_STAGE_FRAGMENT, TV_FS),
        (SHC_STAGE_VERTEX, SAMPLE_VS),
        (SHC_STAGE_FRAGMENT, SAMPLE_FS),
    ];
    for (_, vs, fs, _) in POPULATION.iter().chain(&UNIFORM_POPULATION) {
        srcs.push((SHC_STAGE_VERTEX, vs));
        srcs.push((SHC_STAGE_FRAGMENT, fs));
    }
    srcs.extend(VARYING_POPULATION.iter().map(|c| (SHC_STAGE_VERTEX, c.1)));
    srcs.extend(FRAGMENT_VARYING_POPULATION.iter().map(|c| (SHC_STAGE_FRAGMENT, c.2)));
    let mut most = [0usize; Table::ALL.len()];
    for (st, src) in srcs {
        let (_, marks) = module_facts(st, src);
        for (m, u) in most.iter_mut().zip(marks) {
            *m = (*m).max(u);
        }
    }
    let got: Vec<(&str, usize, usize)> =
        Table::ALL.iter().map(|t| (t.label(), most[*t as usize], t.capacity())).collect();
    assert_eq!(
        got,
        [
            ("IR values", 34, 256),
            ("locals", 1, 32),
            ("NIR instructions", 52, 384),
            ("BIR instructions", 116, 192),
            ("BIR values", 56, 192),
            ("FAU words", 64, 128),
            ("immediates", 9, 64),
            ("uniform rows", 14, 32),
            ("fragment pair nodes", 10, 64),
        ],
        "(table, the referenced programs' most, capacity)"
    );
    for t in Table::ALL {
        let hw = super::high_water(t);
        assert!(hw >= most[t as usize] && hw <= t.capacity(), "{t:?}: high-water {hw}");
    }
}

/// FAU page 1: one word past sel_u_fau64's, and the reference reads it
/// through page 1 (`va_select_fau_page`, not transcribed: refused).
#[test]
fn a_push_word_past_page_0_is_mesa_s_page_1() {
    let r = region(td!("sel_u_fau65.expect.json"), "vs.position").expect("the region");
    let pages: Vec<u64> = r.words.iter().map(|w| (w >> 57) & 3).filter(|p| *p != 0).collect();
    assert!(!pages.is_empty(), "the reference reads FAU page 1 or above");
    assert_eq!(gap(SHC_STAGE_VERTEX, SEL_U_FAU65), MaliError::FauPage);
    /* one word less is byte-equal (the population above) and fills page 0 */
    let v = compile(SHC_STAGE_VERTEX, td!("sel_u_fau64.vert")).expect("compiles");
    assert_eq!(v.info.fau.len(), 64, "sel_u_fau64 fills FAU page 0");
}

/// Past the FAU's 128 words Mesa loads the rest from the UBO (`LD_PKA` in
/// the reference): refused.
#[test]
fn a_uniform_past_128_words_is_mesa_s_ubo_load() {
    let r = region(td!("sel_u_fau130.expect.json"), "vs.position").expect("the region");
    let ops: Vec<_> = r.words.iter().map(|w| isa::decode(*w).expect("decodes").op.info().mnemonic).collect();
    assert!(ops.iter().any(|m| m.starts_with("LD_PKA")), "{ops:?}");
    assert_eq!(gap(SHC_STAGE_VERTEX, SEL_U_FAU130), MaliError::UboFallback);
}

/// A chain of additions is reassociated by GLSL's do_rebalance_tree when
/// it is not a whole right-hand side: the reference adds two uniform words
/// in one FADD, which the chain's order never does. Refused.
#[test]
fn a_chain_of_additions_is_mesa_s_rebalanced_tree() {
    let r = region(td!("sel_u_rebalance.expect.json"), "vs.position").expect("the region");
    let two_fau = r.words.iter().any(|w| {
        let i = isa::decode(*w).expect("decodes");
        i.op.info().mnemonic == "FADD.f32" && (w & 0xc0) == 0x80 && ((w >> 8) & 0xc0) == 0x80
    });
    assert!(two_fau, "the reference adds two uniform words");
    assert_eq!(gap(SHC_STAGE_VERTEX, SEL_U_REBALANCE), MaliError::Rebalance);
}

/// The gaps that need a large program, each reached by one.
fn stress_cases() -> Vec<(u32, String, MaliError)> {
    const VS: u32 = SHC_STAGE_VERTEX;
    let attrs = |n: usize| (0..n).map(|i| format!("attribute vec4 a{i}; ")).collect::<String>();
    /* nine matrix products: 144 FMAs */
    let instrs = format!(
        "attribute vec4 a; uniform mat4 m; void main(){{ gl_Position = {}a{}; }}",
        "m * (".repeat(9),
        ")".repeat(9)
    );
    /* 24 attribute loads (a SPLIT each) in one alternating chain */
    let mut e = "(a0 * a1 + a2)".to_string();
    for k in (3..23).step_by(2) {
        e = format!("({e} * a{k} + a{})", k + 1);
    }
    let values = format!("{} void main(){{ gl_Position = {e}; }}", attrs(24));
    /* 64 distinct fused addends, four a statement */
    let mut body = String::new();
    let mut prev = "a".to_string();
    for s in 0..16 {
        let cs: Vec<String> = (0..4).map(|i| format!("{}.3", 4 * s + i + 3)).collect();
        body += &format!("vec4 t{s} = {prev} * u + vec4({}); ", cs.join(", "));
        prev = format!("t{s}");
    }
    let consts = format!("attribute vec4 a; uniform vec4 u; void main(){{ {body} gl_Position = {prev}; }}");
    /* five vectors dotted with four others: all nine live at once */
    let dots: Vec<String> = (0..5).flat_map(|x| (5..9).map(move |y| format!("dot(a{x}, a{y})"))).collect();
    let lanes: Vec<String> =
        dots.chunks(5).map(|q| format!("((({} * {} + {}) * {} + {}))", q[0], q[1], q[2], q[3], q[4])).collect();
    let spill = format!("{} void main(){{ gl_Position = vec4({}); }}", attrs(9), lanes.join(", "));
    /* a fused multiply-add chain: three vectorised nodes a step and pair */
    let chain = format!(
        "precision mediump float; varying vec4 a; varying vec4 b; void main(){{ vec4 t = a; {} gl_FragColor = t; }}",
        "t = t * b + a; ".repeat(12)
    );
    vec![
        (SHC_STAGE_FRAGMENT, chain, MaliError::FragmentTooLarge),
        (VS, instrs, MaliError::InstrTableFull),
        (VS, values, MaliError::ValueTableFull),
        (VS, consts, MaliError::TooManyConstants),
        (VS, spill, MaliError::Spill),
    ]
}

/* ---- varyings (L3): the shim references ------------------------------ */

/// The varying population, step one: vertex shaders writing several
/// varyings, each the copy of a whole attribute (drawn with `drm_shim_ref.sh
/// run`, Mesa 26.2.1 on drm-shim, the work3 build; extracted by
/// `pan_ref_diff.py extract --draw`). Per case: name, VS, FS, expect.json,
/// the generic layout as (components, byte offset) in the layout's order,
/// and the trace's "Varying allocation" (the IDVS VARY_SIZE, bytes). Each
/// one's fragment shader reads every varying, so Mesa's link keeps them
/// all; the fragment programs are not compared here (the fragment side
/// reads one varying today).
#[allow(clippy::type_complexity)]
const VARYING_POPULATION: [(&str, &str, &str, &str, &[(u8, u16)], u32); 13] = [
    (
        "sel_v_color_uv",
        td!("sel_v_color_uv.vert"),
        td!("sel_v_color_uv.frag"),
        td!("sel_v_color_uv.expect.json"),
        &[(4, 0), (2, 16)],
        24,
    ),
    (
        "sel_v_color_uv_declared_reversed",
        td!("sel_v_color_uv_declared_reversed.vert"),
        td!("sel_v_color_uv_declared_reversed.frag"),
        td!("sel_v_color_uv_declared_reversed.expect.json"),
        &[(4, 0), (2, 16)],
        24,
    ),
    (
        "sel_v_color_uv_written_reversed",
        td!("sel_v_color_uv_written_reversed.vert"),
        td!("sel_v_color_uv_written_reversed.frag"),
        td!("sel_v_color_uv_written_reversed.expect.json"),
        &[(4, 0), (2, 16)],
        24,
    ),
    (
        "sel_v_v4v4",
        td!("sel_v_v4v4.vert"),
        td!("sel_v_v4v4.frag"),
        td!("sel_v_v4v4.expect.json"),
        &[(4, 0), (4, 16)],
        32,
    ),
    (
        "sel_v_v4v4_written_reversed",
        td!("sel_v_v4v4_written_reversed.vert"),
        td!("sel_v_v4v4_written_reversed.frag"),
        td!("sel_v_v4v4_written_reversed.expect.json"),
        &[(4, 0), (4, 16)],
        32,
    ),
    (
        "sel_v_v4_float",
        td!("sel_v_v4_float.vert"),
        td!("sel_v_v4_float.frag"),
        td!("sel_v_v4_float.expect.json"),
        &[(4, 0), (1, 16)],
        20,
    ),
    (
        "sel_v_v4_vec3",
        td!("sel_v_v4_vec3.vert"),
        td!("sel_v_v4_vec3.frag"),
        td!("sel_v_v4_vec3.expect.json"),
        &[(4, 0), (3, 16)],
        28,
    ),
    (
        "sel_v_v4v4v2",
        td!("sel_v_v4v4v2.vert"),
        td!("sel_v_v4v4v2.frag"),
        td!("sel_v_v4v4v2.expect.json"),
        &[(4, 0), (4, 16), (2, 32)],
        40,
    ),
    /* step (c): varyings computed from an attribute */
    (
        "sel_v_attr_times_uniform",
        td!("sel_v_attr_times_uniform.vert"),
        td!("sel_v_attr_times_uniform.frag"),
        td!("sel_v_attr_times_uniform.expect.json"),
        &[(4, 0)],
        16,
    ),
    (
        "sel_v_attr_plus_const",
        td!("sel_v_attr_plus_const.vert"),
        td!("sel_v_attr_plus_const.frag"),
        td!("sel_v_attr_plus_const.expect.json"),
        &[(2, 0)],
        8,
    ),
    (
        "sel_v_swizzle",
        td!("sel_v_swizzle.vert"),
        td!("sel_v_swizzle.frag"),
        td!("sel_v_swizzle.expect.json"),
        &[(2, 0)],
        8,
    ),
    (
        "sel_v_shared_with_position",
        td!("sel_v_shared_with_position.vert"),
        td!("sel_v_shared_with_position.frag"),
        td!("sel_v_shared_with_position.expect.json"),
        &[(4, 0)],
        16,
    ),
    (
        "sel_v_attr_fma",
        td!("sel_v_attr_fma.vert"),
        td!("sel_v_attr_fma.frag"),
        td!("sel_v_attr_fma.expect.json"),
        &[(4, 0)],
        16,
    ),
];

#[test]
fn the_varying_population_is_byte_equal_to_mesa_26_2_1() {
    use crate::mali::program::StageMeta;
    let mut failed = Vec::new();
    let mut regions = 0;
    for (name, vs, fs, json, layout, bytes) in VARYING_POPULATION {
        regions += 1 + region(json, "vs.varying").is_some() as usize;
        let r = std::panic::catch_unwind(|| {
            let v = check_vs_in(vs, fs, json, VIEWPORT_64X64);
            let generic: Vec<(u8, u16)> =
                v.info.varyings.iter().filter(|s| s.section == Section::Generic).map(|s| (s.comps, s.offset)).collect();
            assert_eq!(generic, layout, "the generic layout");
            let meta = StageMeta::of(&v.info.shader()).expect("the frame builder takes the record");
            assert_eq!(meta.meta().varying_bytes, bytes, "VARY_SIZE");
        });
        if let Err(e) = r {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), VARYING_POPULATION.len(), failed.join("\n"));
    assert_eq!((VARYING_POPULATION.len(), regions), (13, 26), "(programs, vertex regions) compared");
}

/// Two vec2 varyings: GLSL's link packs them into one slot, the second at
/// byte 8, and the fragment shader reads both with one four-lane LD_VAR at
/// byte 0. Both stages are byte-equal to the shim's (Mesa 26.2.1's GL
/// driver, the GLSL path).
#[test]
fn two_varyings_in_one_slot_are_mesa_s_packed_varyings() {
    let r = region(td!("sel_v_packed.expect.json"), "fs.fragment").expect("the region");
    let ld: Vec<String> = r.words.iter().map(|w| text(*w)).filter(|t| t.starts_with("LD_VAR")).collect();
    assert_eq!(ld.len(), 1, "{ld:?}");
    assert!(ld[0].contains(".v4") && ld[0].contains("index:0x0"), "{ld:?}");
    check_program(td!("sel_v_packed.vert"), td!("sel_v_packed.frag"), td!("sel_v_packed.expect.json"));
}

/// A fragment shader's product with a constant: one constant pair a
/// product (`v * 0.5`, `0.5 * v` keeping the order), `* 2.0` an fadd after
/// the fusion, a pair per lane pair, a constant addend fused. Each drawn
/// with `drm_shim_ref.sh run` (the work3 build of 2026-10-19), with gl_test's
/// passthrough.vert, both stages compared.
const FS_CONST_POPULATION: [(&str, &str, &str, &str); 5] = [
    ("sel_f_const_h", PASSTHROUGH_VS, td!("sel_f_const_h.frag"), td!("sel_f_const_h.expect.json")),
    ("sel_f_const_hc", PASSTHROUGH_VS, td!("sel_f_const_hc.frag"), td!("sel_f_const_hc.expect.json")),
    ("sel_f_const_t", PASSTHROUGH_VS, td!("sel_f_const_t.frag"), td!("sel_f_const_t.expect.json")),
    ("sel_f_const_q", PASSTHROUGH_VS, td!("sel_f_const_q.frag"), td!("sel_f_const_q.expect.json")),
    ("sel_f_const_hf", PASSTHROUGH_VS, td!("sel_f_const_hf.frag"), td!("sel_f_const_hf.expect.json")),
];

#[test]
fn the_fragment_constant_products_are_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json) in FS_CONST_POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program(vs, fs, json)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), FS_CONST_POPULATION.len(), failed.join("\n"));
}

/// A fragment shader's fneg: stored (`-v`, `-(v * 0.5)`, a pair of `*
/// -1.0`), an fadd's operand (`1.0 - v`, `v.xy - v.zw`, `1.0 - (a + b)`), a
/// negated constant addend fused (`v * 0.5 - 0.25`) and a negated product
/// fused (`0.5 - v * 0.5`). Each drawn with `drm_shim_ref.sh run` (the work3
/// build of 2026-10-19), with gl_test's passthrough.vert, both stages
/// compared.
const FS_NEG_POPULATION: [(&str, &str, &str, &str); 8] = [
    ("sel_f_neg_n", PASSTHROUGH_VS, td!("sel_f_neg_n.frag"), td!("sel_f_neg_n.expect.json")),
    ("sel_f_neg_nm", PASSTHROUGH_VS, td!("sel_f_neg_nm.frag"), td!("sel_f_neg_nm.expect.json")),
    ("sel_f_neg_m1", PASSTHROUGH_VS, td!("sel_f_neg_m1.frag"), td!("sel_f_neg_m1.expect.json")),
    ("sel_f_neg_om", PASSTHROUGH_VS, td!("sel_f_neg_om.frag"), td!("sel_f_neg_om.expect.json")),
    ("sel_f_neg_sub", PASSTHROUGH_VS, td!("sel_f_neg_sub.frag"), td!("sel_f_neg_sub.expect.json")),
    ("sel_f_neg_ns", PASSTHROUGH_VS, td!("sel_f_neg_ns.frag"), td!("sel_f_neg_ns.expect.json")),
    ("sel_f_neg_hm", PASSTHROUGH_VS, td!("sel_f_neg_hm.frag"), td!("sel_f_neg_hm.expect.json")),
    ("sel_f_neg_mh", PASSTHROUGH_VS, td!("sel_f_neg_mh.frag"), td!("sel_f_neg_mh.expect.json")),
];

#[test]
fn the_fragment_negations_are_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json) in FS_NEG_POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program(vs, fs, json)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), FS_NEG_POPULATION.len(), failed.join("\n"));
}

/// A fragment shader's fabs: stored (`abs(v)`, `abs(-v)` its fabs,
/// `-abs(v)` one FADD of both modifiers, `abs(v - 0.5)` after an
/// FADD_IMM), an FADD's operand (`abs(abs(v) - 0.5)`, `1.0 - abs(v * 2.0 -
/// 1.0)` the negated fabs), and an FMA's, which has no abs modifier, so the
/// fabs is its own FADD (`abs(v) * 0.5`). Each drawn with `drm_shim_ref.sh
/// run` (the work3 build of 2026-10-19), with gl_test's passthrough.vert,
/// both stages compared.
const FS_ABS_POPULATION: [(&str, &str, &str, &str); 7] = [
    ("sel_f_abs_a", PASSTHROUGH_VS, td!("sel_f_abs_a.frag"), td!("sel_f_abs_a.expect.json")),
    ("sel_f_abs_an", PASSTHROUGH_VS, td!("sel_f_abs_an.frag"), td!("sel_f_abs_an.expect.json")),
    ("sel_f_abs_na", PASSTHROUGH_VS, td!("sel_f_abs_na.frag"), td!("sel_f_abs_na.expect.json")),
    ("sel_f_abs_as", PASSTHROUGH_VS, td!("sel_f_abs_as.frag"), td!("sel_f_abs_as.expect.json")),
    ("sel_f_abs_aa", PASSTHROUGH_VS, td!("sel_f_abs_aa.frag"), td!("sel_f_abs_aa.expect.json")),
    ("sel_f_abs_oa", PASSTHROUGH_VS, td!("sel_f_abs_oa.frag"), td!("sel_f_abs_oa.expect.json")),
    ("sel_f_abs_am", PASSTHROUGH_VS, td!("sel_f_abs_am.frag"), td!("sel_f_abs_am.expect.json")),
];

#[test]
fn the_fragment_absolute_values_are_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json) in FS_ABS_POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program(vs, fs, json)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), FS_ABS_POPULATION.len(), failed.join("\n"));
}

/// The fragment varying population: fragment shaders reading several
/// varyings, each by its byte offset in Mesa's load order, and the colour's
/// fp16 arithmetic over them (nir_fs.rs). Each was drawn with
/// `drm_shim_ref.sh run` and is compared whole, both stages; together they
/// draw every production the vectoriser admits (nir_fs.rs `ADMITTED`,
/// `the_fragment_productions_are_the_references`).
const FRAGMENT_VARYING_POPULATION: [(&str, &str, &str, &str); 20] = [
    ("sel_v_color_uv", td!("sel_v_color_uv.vert"), td!("sel_v_color_uv.frag"), td!("sel_v_color_uv.expect.json")),
    (
        "sel_v_uv_times_color",
        td!("sel_v_uv_times_color.vert"),
        td!("sel_v_uv_times_color.frag"),
        td!("sel_v_uv_times_color.expect.json"),
    ),
    (
        "sel_v_v4_times_v4",
        td!("sel_v_v4_times_v4.vert"),
        td!("sel_v_v4_times_v4.frag"),
        td!("sel_v_v4_times_v4.expect.json"),
    ),
    /* step (d): sums, fused products, a lane alone, swizzles, a uniform */
    ("sel_v_v4v4", td!("sel_v_v4v4.vert"), td!("sel_v_v4v4.frag"), td!("sel_v_v4v4.expect.json")),
    ("sel_v_v4_float", td!("sel_v_v4_float.vert"), td!("sel_v_v4_float.frag"), td!("sel_v_v4_float.expect.json")),
    ("sel_v_v4_vec3", td!("sel_v_v4_vec3.vert"), td!("sel_v_v4_vec3.frag"), td!("sel_v_v4_vec3.expect.json")),
    ("sel_v_v4v4v2", td!("sel_v_v4v4v2.vert"), td!("sel_v_v4v4v2.frag"), td!("sel_v_v4v4v2.expect.json")),
    (
        "sel_v_half_reused",
        td!("sel_v_half_reused.vert"),
        td!("sel_v_half_reused.frag"),
        td!("sel_v_half_reused.expect.json"),
    ),
    (
        "sel_v_swizzle_zwxy",
        td!("sel_v_swizzle_zwxy.vert"),
        td!("sel_v_swizzle_zwxy.frag"),
        td!("sel_v_swizzle_zwxy.expect.json"),
    ),
    (
        "sel_v_swizzle_yxwz",
        td!("sel_v_swizzle_yxwz.vert"),
        td!("sel_v_swizzle_yxwz.frag"),
        td!("sel_v_swizzle_yxwz.expect.json"),
    ),
    (
        "sel_v_varying_uniform_fma",
        td!("sel_v_varying_uniform_fma.vert"),
        td!("sel_v_varying_uniform_fma.frag"),
        td!("sel_v_varying_uniform_fma.expect.json"),
    ),
    (
        "sel_v_varying_plus_uniform",
        td!("sel_v_varying_plus_uniform.vert"),
        td!("sel_v_varying_plus_uniform.frag"),
        td!("sel_v_varying_plus_uniform.expect.json"),
    ),
    /* every operand form in every place the vectoriser admits */
    (
        "sel_v_swizzled_times",
        td!("sel_v_swizzled_times.vert"),
        td!("sel_v_swizzled_times.frag"),
        td!("sel_v_swizzled_times.expect.json"),
    ),
    (
        "sel_v_plus_float",
        td!("sel_v_plus_float.vert"),
        td!("sel_v_plus_float.frag"),
        td!("sel_v_plus_float.expect.json"),
    ),
    (
        "sel_v_fma_plus_vec3",
        td!("sel_v_fma_plus_vec3.vert"),
        td!("sel_v_fma_plus_vec3.frag"),
        td!("sel_v_fma_plus_vec3.expect.json"),
    ),
    (
        "sel_v_fma_plus_uniform",
        td!("sel_v_fma_plus_uniform.vert"),
        td!("sel_v_fma_plus_uniform.frag"),
        td!("sel_v_fma_plus_uniform.expect.json"),
    ),
    ("sel_v_sum_times", td!("sel_v_sum_times.vert"), td!("sel_v_sum_times.frag"), td!("sel_v_sum_times.expect.json")),
    ("sel_v_float_fma", td!("sel_v_float_fma.vert"), td!("sel_v_float_fma.frag"), td!("sel_v_float_fma.expect.json")),
    (
        "sel_v_product_of_three",
        td!("sel_v_product_of_three.vert"),
        td!("sel_v_product_of_three.frag"),
        td!("sel_v_product_of_three.expect.json"),
    ),
    (
        "sel_v_swizzled_fma",
        td!("sel_v_swizzled_fma.vert"),
        td!("sel_v_swizzled_fma.frag"),
        td!("sel_v_swizzled_fma.expect.json"),
    ),
];

#[test]
fn the_fragment_varying_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    let mut regions = 0;
    for (name, vs, fs, json) in FRAGMENT_VARYING_POPULATION {
        regions += 2 + region(json, "vs.varying").is_some() as usize;
        let r = std::panic::catch_unwind(|| {
            let (v, f) = check_program(vs, fs, json);
            /* the link's rule: each varying the fragment shader reads at the
             * vertex shader's offset */
            for s in &f.info.varyings {
                assert!(v.info.varyings.iter().any(|x| x.varying == s.varying && x.offset == s.offset), "{s:?}");
            }
        });
        if let Err(e) = r {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {} differ:\n{}",
        failed.len(),
        FRAGMENT_VARYING_POPULATION.len(),
        failed.join("\n")
    );
    assert_eq!((FRAGMENT_VARYING_POPULATION.len(), regions), (20, 60), "(programs, regions) compared");
}

/// The varyings Mesa's link moves, rewrites or merges (nir_opt_varyings),
/// so that the layout is not the vertex stage's alone: the references show
/// it, the back end refuses each by its own gap.
#[test]
fn varyings_mesa_s_link_moves_or_merges_are_refused() {
    let stores = |json: &str| {
        let r = region(json, "vs.varying").expect("a varying program");
        r.words.iter().filter(|w| text(**w).starts_with("STORE")).count()
    };
    /* vec4(a_t, 0.0, 1.0): the two constant lanes are not stored */
    assert_eq!(stores(td!("sel_v_const.expect.json")), 2);
    assert_eq!(gap(SHC_STAGE_VERTEX, td!("sel_v_const.vert")), MaliError::VaryingNotFromAttribute);
    /* a uniform alone: an exact ffma(u, 0, u) per lane */
    let r = region(td!("sel_v_uniform.expect.json"), "vs.varying").expect("a varying program");
    let fmas = r.words.iter().map(|w| text(*w)).filter(|t| t.starts_with("FMA.f32") && t.contains(", 0x0, u")).count();
    assert_eq!(fmas, 4);
    assert_eq!(gap(SHC_STAGE_VERTEX, td!("sel_v_uniform.vert")), MaliError::VaryingNotFromAttribute);
    /* two varyings of equal value: one slot, one four-lane load */
    assert_eq!(stores(td!("sel_v_duplicate.expect.json")), 4);
    let fs = region(td!("sel_v_duplicate.expect.json"), "fs.fragment").expect("fs.fragment");
    assert_eq!(fs.words.iter().filter(|w| text(**w).starts_with("LD_VAR")).count(), 1);
    assert_eq!(gap(SHC_STAGE_VERTEX, td!("sel_v_duplicate.vert")), MaliError::VaryingDuplicate);
}

/// The ISA ops the varying populations add to the corpus's and the
/// uniform population's, derived from their programs: FADD.v2f16 only,
/// pinned by name, so a new op fails here until a reference has it.
#[test]
fn the_ops_the_varying_populations_add_are_pinned() {
    let ops_of = |srcs: &[(u32, &str)]| {
        let mut ops = std::collections::BTreeSet::new();
        for (st, src) in srcs {
            for &w in compile(*st, src).unwrap().code.iter().filter(|w| **w != 0) {
                ops.insert(isa::decode(w).expect("our word decodes").op.info().mnemonic);
            }
        }
        ops
    };
    let mut before: Vec<(u32, &str)> = vec![
        (SHC_STAGE_VERTEX, TRI_VS),
        (SHC_STAGE_FRAGMENT, TRI_FS),
        (SHC_STAGE_VERTEX, TV_VS),
        (SHC_STAGE_FRAGMENT, TV_FS),
        (SHC_STAGE_VERTEX, SAMPLE_VS),
        (SHC_STAGE_FRAGMENT, SAMPLE_FS),
    ];
    for (_, vs, fs, _) in UNIFORM_POPULATION {
        before.push((SHC_STAGE_VERTEX, vs));
        before.push((SHC_STAGE_FRAGMENT, fs));
    }
    let mut varyings: Vec<(u32, &str)> = VARYING_POPULATION.iter().map(|c| (SHC_STAGE_VERTEX, c.1)).collect();
    varyings.extend(FRAGMENT_VARYING_POPULATION.iter().map(|c| (SHC_STAGE_FRAGMENT, c.2)));
    let (before, after) = (ops_of(&before), ops_of(&varyings));
    let new: Vec<_> = after.difference(&before).collect();
    assert_eq!(new, [&"FADD.v2f16"], "{after:?}");
}

/// The vectoriser's productions (nir_fs.rs `prod`: an operand form in a
/// place) are exactly the ones the references draw: the fragment shaders
/// of every population, compiled, make `ADMITTED` and nothing else, so
/// a production the code admits is never one no reference has.
#[test]
fn the_fragment_productions_are_the_references() {
    let mut fs: Vec<&str> = vec![TRI_FS, TV_FS, SAMPLE_FS];
    fs.extend(POPULATION.iter().map(|c| c.2));
    fs.extend(UNIFORM_POPULATION.iter().map(|c| c.2));
    fs.extend(FRAGMENT_VARYING_POPULATION.iter().map(|c| c.2));
    fs.extend(FS_CONST_POPULATION.iter().map(|c| c.2));
    fs.extend(FS_NEG_POPULATION.iter().map(|c| c.2));
    fs.extend(FS_ABS_POPULATION.iter().map(|c| c.2));
    fs.extend(TEXTURE_POPULATION.iter().map(|c| c.2));
    /* the constant coordinate and the run-time index (#1385 (b)) */
    fs.extend([td!("tex_coord_const.frag"), td!("vtl_fs.frag"), td!("uniidx_d.frag"), td!("uniidx_i.frag")]);
    fs.extend(UNIARRAY_POPULATION.iter().map(|c| c.2));
    let mut seen = 0u128;
    for src in fs {
        let _k = store::test_keep();
        seen |= store::with_scratch(|s| {
            let l = codegen::lower_source(SHC_STAGE_FRAGMENT, src.as_bytes(), &[], s).expect("the front end takes it");
            let text = s.alloc_str(src).expect("room");
            let prec = super::FsPrecision::of(&l.module, text, s).expect("the precision");
            assert!(!prec.highp, "the vectoriser's populations are mediump");
            let mut f = super::nir_fs::walk(&l.module, s, prec).expect("the walk");
            let st = crate::shaderc::online::sema::Stage::Fragment;
            let decls = crate::shaderc::online::reflect::interface(s, st, text).expect("the interface");
            let highp = |name: &str| {
                decls
                    .uniforms
                    .iter()
                    .any(|u| u.name == name && u.precision == crate::shaderc::online::ast::Precision::High)
            };
            super::nir_fs::layout(&mut f, &l.module, None).expect("the layout");
            let mut p = super::prog(s).expect("room");
            let ubo = super::select::UboLayout::fs16(&l.module, &f.nir, &highp).expect("the block");
            super::nir_fs::select(&mut p, &mut f, &ubo).expect("selected");
            f.prods
        })
        .expect("the scratch arena is free");
    }
    assert_eq!(seen, super::nir_fs::ADMITTED, "drawn {seen:#x}, admitted {:#x}", super::nir_fs::ADMITTED);
}

/* ---- textures (L3, 2026-10-19) ------------------------------------------ */

/// The texture population: each program pair drawn once on the shim
/// (`drm_shim_ref.sh run ... --uniform-values`, Mesa 26.2.1; gl_ref.py
/// binds each sampler its own 2x2 RGBA8 texture) and compared whole, both
/// stages.
const TEXTURE_POPULATION: [(&str, &str, &str, &str, u8); 2] = [
    ("sel_t_tex", td!("sel_t_tex.vert"), td!("sel_t_tex.frag"), td!("sel_t_tex.expect.json"), 0),
    ("sel_t_second", td!("sel_t_tex.vert"), td!("sel_t_second.frag"), td!("sel_t_second.expect.json"), 1),
];

#[test]
fn the_texture_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    let mut regions = 0;
    for (name, vs, fs, json, binding) in TEXTURE_POPULATION {
        regions += 2 + region(json, "vs.varying").is_some() as usize;
        let r = std::panic::catch_unwind(|| {
            let (_, f) = check_program(vs, fs, json);
            /* the frame builder binds this sampler's texture at index 0 */
            assert_eq!(f.info.sampler, Some(binding), "the sampler read");
            /* an implicit LOD: the quads' helpers (the shim's NIR prints
             * needs_coarse_quad_helper_invocations: true) */
            assert!(f.info.helper_threads, "the look-up needs helper threads");
        });
        if let Err(e) = r {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), TEXTURE_POPULATION.len(), failed.join("\n"));
    assert_eq!((TEXTURE_POPULATION.len(), regions), (2, 6), "(programs, regions) compared");
}

/// The ISA ops the texture population adds to every other population's:
/// TEX_SINGLE only, pinned by name.
#[test]
fn the_ops_the_texture_population_adds_are_pinned() {
    let ops_of = |srcs: &[&str]| {
        let mut ops = std::collections::BTreeSet::new();
        for src in srcs {
            for &w in compile(SHC_STAGE_FRAGMENT, src).unwrap().code.iter().filter(|w| **w != 0) {
                ops.insert(isa::decode(w).expect("our word decodes").op.info().mnemonic);
            }
        }
        ops
    };
    let mut before: Vec<&str> = vec![TRI_FS, TV_FS, SAMPLE_FS];
    before.extend(UNIFORM_POPULATION.iter().map(|c| c.2));
    before.extend(FRAGMENT_VARYING_POPULATION.iter().map(|c| c.2));
    let tex: Vec<&str> = TEXTURE_POPULATION.iter().map(|c| c.2).collect();
    let (before, after) = (ops_of(&before), ops_of(&tex));
    let new: Vec<_> = after.difference(&before).collect();
    assert_eq!(new, [&"TEX_SINGLE"], "{after:?}");
}

/* ---- the colour preload (2026-10-19) ------------------------------------ */

/// The colour preload program (preload.rs) against Mesa's: the program of
/// the pre-frame draw Mesa 26.2.1 made on the shim for gl_ref.py
/// `--second-frame` (tri.vert, tri.frag; `pan_ref_diff.py extract <trace>
/// fs.fragment=7fffffffe100`, the preload shader's VA in that trace), and
/// the board's Mesa 25.0.7 the same words (0dce654a8:tools/mali/board-ref/
/// 2026-10-19-preload/mesa/mesa-strip.pandecode, Binary 0x7fffffffe100).
/// The facts the frame builder takes: r60 and r61 preloaded, the split
/// file's 32 work registers.
#[test]
fn the_colour_preload_is_mesa_s() {
    /* the region's words (its facts are not printed for a program no draw
     * names as its fragment shader: the pre-frame draw's) */
    let json = td!("preload.expect.json");
    let mesa = after(json, "words").and_then(hex_array).expect("the preload's words");
    let (words, preload, wrc) = store::with_scratch(|s| {
        let mut out = vec![0u64; 64];
        let p = super::preload::colour(&mut out, s).expect("the preload compiles");
        (p.code.to_vec(), p.preload, p.work_reg_count)
    })
    .expect("the scratch arena is free");
    assert_words("preload", &trim(&words), &mesa);
    assert_eq!((preload >> 48, wrc), (0x3000, 32), "preload r60 r61, 32 work registers");
    /* the board's 25.0.7 words, as its dump prints them */
    let board: [u64; 7] = [
        0x6854_8004_1200_007d,
        0x0110_c218_0010_00c0,
        0x0891_c300_0000_00c0,
        0x4928_40b4_53c0_0042,
        0x787f_4008_b23c_00f0,
        0x0110_f000_0000_00c0,
        0x002f_c110_0000_f1c0,
    ];
    assert_eq!(trim(&words), board, "the board's Mesa 25.0.7 preload");
}

/// The depth/stencil preload program (`super::preload::zs`) against Mesa
/// 26.2.1's on drm-shim (preload_zs.expect.json: `gl_ref.py --second-frame
/// --depth-stencil`), and against the board's Mesa 25.0.7
/// (0dce654a8:tools/mali/board-ref/2026-10-19-preload-zs/mesa), whose ATEST reads
/// the alpha 0 (word 8) where 26.2.1's reads 1.0.
#[test]
fn the_zs_preload_is_mesa_s() {
    let json = td!("preload_zs.expect.json");
    let mesa = after(json, "words").and_then(hex_array).expect("the Z/S preload's words");
    let (words, preload, wrc) = store::with_scratch(|s| {
        let mut out = vec![0u64; 64];
        let p = super::preload::zs(&mut out, s, true, true).expect("the Z/S preload compiles");
        (p.code.to_vec(), p.preload, p.work_reg_count)
    })
    .expect("the scratch arena is free");
    assert_words("preload_zs", &trim(&words), &mesa);
    assert_eq!((preload >> 48, wrc), (0x3000, 32), "preload r60 r61, 32 work registers");
    let mut board = mesa.clone();
    board[8] = 0x407d_bc02_00ea_c07c;
    assert_eq!(&trim(&words)[..8], &board[..8], "the board's Mesa 25.0.7 words before ATEST");
    assert_eq!(trim(&words)[9], board[9], "and its ZS_EMIT");
}

/// The depth-only and the stencil-only variants of the Z/S preload against
/// Mesa 26.2.1's on drm-shim (`gl_ref.py --second-frame --depth-stencil
/// --second-clear stencil`, and `depth`): the frame clears the other plane.
#[test]
fn the_one_plane_zs_preloads_are_mesa_s() {
    for (name, json, z, s) in [
        ("depth only", td!("preload_zs_stencil.expect.json"), true, false),
        ("stencil only", td!("preload_zs_depth.expect.json"), false, true),
    ] {
        let mesa = after(json, "words").and_then(hex_array).expect("the Z/S preload's words");
        let (words, preload) = store::with_scratch(|sc| {
            let mut out = vec![0u64; 64];
            let p = super::preload::zs(&mut out, sc, z, s).expect("the Z/S preload compiles");
            (p.code.to_vec(), p.preload)
        })
        .expect("the scratch arena is free");
        assert_words(name, &trim(&words), &mesa);
        assert_eq!(preload >> 48, 0x3000, "{name}: preload r60 r61");
    }
}

/* ---- blend shaders (#1386, 2026-10-23) ------------------------------------ */

/// The blend shaders (`super::blend::shader`) of every equation a constant
/// colour makes a blend shader of (glBlendFunc with `CONSTANT_COLOR` or its
/// inverse against ONE, ZERO or the other), into an RGBA8 target (memory
/// format RGBA8_TB, RGBA: `0xED000`), against Mesa 26.2.1's on drm-shim:
/// `gl_ref.py --blend-func S,D --blend-color 0.25,0.5,0.75,1.0` with
/// gl_test's colour_uv pair, the shader the trace disassembles after the
/// draw's blend descriptor, extracted by `pan_ref_diff.py extract <trace>
/// blend=<va>` (0dce654a8:tools/mali/board-ref/2026-10-23-blendconst/README.md).
/// `blend_c_ic` is gl_test case 8's fifth frame.
#[test]
fn the_blend_shaders_are_mesa_s() {
    use super::blend::{Equation, Factor::*};
    const RGBA8_TB_RGBA: u32 = 0xed000;
    for (name, json, src, dst) in [
        ("blend_c_ic", td!("blend_c_ic.expect.json"), Const, InvConst),
        ("blend_ic_c", td!("blend_ic_c.expect.json"), InvConst, Const),
        ("blend_c_zero", td!("blend_c_zero.expect.json"), Const, Zero),
        ("blend_c_one", td!("blend_c_one.expect.json"), Const, One),
        ("blend_one_c", td!("blend_one_c.expect.json"), One, Const),
        ("blend_zero_c", td!("blend_zero_c.expect.json"), Zero, Const),
        ("blend_ic_zero", td!("blend_ic_zero.expect.json"), InvConst, Zero),
        ("blend_ic_one", td!("blend_ic_one.expect.json"), InvConst, One),
        ("blend_one_ic", td!("blend_one_ic.expect.json"), One, InvConst),
        ("blend_zero_ic", td!("blend_zero_ic.expect.json"), Zero, InvConst),
    ] {
        let mesa = after(json, "words").and_then(hex_array).expect("the blend shader's words");
        let (words, wrc) = store::with_scratch(|sc| {
            let mut out = vec![0u64; 64];
            let b = super::blend::shader(&mut out, sc, Equation::both(src, dst), RGBA8_TB_RGBA)
                .expect("the blend shader compiles");
            (b.code.to_vec(), b.work_reg_count)
        })
        .expect("the scratch arena is free");
        assert_words(name, &trim(&words), &mesa);
        assert_eq!(wrc, 32, "{name}: the split file's count");
    }
    /* both factors the same constant term: NIR factors the sum, no
     * reference is drawn; refused, never approximated */
    let same = Equation::both(Const, Const);
    assert!(!super::blend::builds(same));
}

/// The blend shaders of gl_test cases 211 and 190 (docs/notes/
/// mali-blend-shaders.md sections 2 and 5), against Mesa 26.2.1's on
/// drm-shim (tools/mali/testdata/blend_case*.words, one word a line; the
/// `.txt` next to each is pandecode's disassembly): 211's `CONSTANT_COLOR,
/// ONE_MINUS_CONSTANT_COLOR` at 4x (the tile read at the sample id of
/// r61), 190's column 3 (GL_MIN colour, SRC_ALPHA, ONE_MINUS_SRC_ALPHA
/// alpha) and column 4 (ONE_MINUS_DST_ALPHA, ONE_MINUS_SRC_ALPHA colour,
/// ONE, ZERO alpha) at 1x, into RGBA8.
#[test]
fn case_211_and_190_s_blend_shaders_are_mesa_s() {
    use super::blend::{builds, Channel, Equation, Factor::*};
    const RGBA8_TB_RGBA: u32 = 0xed000;
    let words = |text: &str| -> Vec<u64> {
        text.lines().map(|l| u64::from_str_radix(l.trim(), 16).expect("a hex word")).collect()
    };
    let case211 = Equation { samples: 4, ..Equation::both(Const, InvConst) };
    let col3 = Equation { rgb: Channel::MIN, alpha: Channel::add(SrcAlpha, InvSrcAlpha), samples: 1 };
    let col4 = Equation { rgb: Channel::add(InvDstAlpha, InvSrcAlpha), alpha: Channel::add(One, Zero), samples: 1 };
    for (name, text, eq) in [
        ("blend_case211", td!("blend_case211.words"), case211),
        ("blend_case190_col3", td!("blend_case190_col3.words"), col3),
        ("blend_case190_col4", td!("blend_case190_col4.words"), col4),
    ] {
        let mesa = words(text);
        let (ours, wrc) = store::with_scratch(|sc| {
            let mut out = vec![0u64; 64];
            let b = super::blend::shader(&mut out, sc, eq, RGBA8_TB_RGBA).expect("the blend shader compiles");
            (b.code.to_vec(), b.work_reg_count)
        })
        .expect("the scratch arena is free");
        assert_words(name, &trim(&ours), &mesa);
        assert_eq!(wrc, 32, "{name}: the split file's count");
    }
    /* the key: what no reference draws is refused, never approximated */
    let refused = [
        Equation { samples: 4, ..col3 },
        Equation { samples: 4, ..col4 },
        Equation { samples: 4, ..Equation::both(Const, Zero) },
        Equation { samples: 2, ..case211 },
        Equation { rgb: Channel { func: super::blend::Func::Min, ..Channel::add(One, One) }, ..col4 },
        Equation { alpha: Channel::MIN, ..col3 },
        Equation {
            rgb: Channel::add(InvDstAlpha, InvSrcAlpha),
            alpha: Channel::add(SrcAlpha, InvSrcAlpha),
            samples: 1,
        },
        Equation::both(SrcAlpha, InvSrcAlpha),
    ];
    for eq in refused {
        assert!(!builds(eq), "{eq:?}");
        let r = store::with_scratch(|sc| super::blend::shader(&mut [0u64; 64], sc, eq, RGBA8_TB_RGBA).err())
            .expect("the scratch arena is free");
        assert_eq!(r, Some(MaliError::BlendVariantNotLowered), "{eq:?}");
    }
}

/* ---- highp fragment shaders (S2, 2026-10-21) ----------------------------- */

/// gl_test's cases 28 (fvgvs), 29 (fvgft) and 30 (fvgin): femtovg's vertex
/// shader with a highp fragment shader, a constant colour, a copy of a
/// varying and femtovg's uniform block `frag[14]` read at a constant index
/// (`fvgtest_*`, drawn with `drm_shim_ref.sh run`, tools/mali/testdata/
/// cases.txt). Their fragment programs go through the fp32 path
/// (nir_fs.rs `select_highp`): the varying is loaded as fp32 lanes and
/// converted once, at the colour, the pair by one `V2F32_TO_V2F16`; the
/// uniform's four words are pushed to the FAU after the blend constants
/// (`bi_opt_push_ubo`) and converted from there. The last field says
/// whether the draw set the uniforms (`--uniform-values`): gl_ref.py cannot
/// set an array declared through a macro, so fvgtest_inner's words are 0.
const HIGHP_POPULATION: [(&str, &str, &str, &str, bool); 3] = [
    ("fvgtest_green", td!("fvgtest_green.vert"), td!("fvgtest_green.frag"), td!("fvgtest_green.expect.json"), true),
    (
        "fvgtest_ftcoord",
        td!("fvgtest_ftcoord.vert"),
        td!("fvgtest_ftcoord.frag"),
        td!("fvgtest_ftcoord.expect.json"),
        true,
    ),
    ("fvgtest_inner", td!("fvgtest_inner.vert"), td!("fvgtest_inner.frag"), td!("fvgtest_inner.expect.json"), false),
];

#[test]
fn the_highp_fragment_programs_are_byte_equal_to_mesa_26_2_1() {
    for (name, vs, fs, json, values) in HIGHP_POPULATION {
        let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
        let rf = region(json, "fs.fragment").expect("fs.fragment");
        assert_words(&format!("{name} fs.fragment"), &trim(&f.code), &rf.words);
        let names = uniform_names(vs, fs);
        let uv = |d: u16| if values { uniform_value(&names, &f.uniforms, d) } else { 0 };
        assert_facts(name, &rf, f.info.work_reg_count, f.info.preload, &f.info.fau, VIEWPORT_64X64, &uv);
        let _k = store::test_keep();
        let fsp = store::with_scratch(|s| {
            let l = codegen::lower_source(SHC_STAGE_FRAGMENT, fs.as_bytes(), &[], s).expect("the front end takes it");
            super::FsPrecision::of(&l.module, s.alloc_str(fs).expect("room"), s).expect("the precision")
        })
        .expect("the scratch arena is free");
        assert_eq!(fsp, super::FsPrecision { highp: true, ..Default::default() }, "{name}");
    }
}

/// The S4 population (#1384): one minimal highp program per ALU row,
/// each drawn once on the shim (`drm_shim_ref.sh run`, Mesa 26.2.1; the
/// shim-refs item of plan #152), its vertex shader `v = c` built for the
/// fragment shader's reads, and femtovg's `fill_color.nodiscard.noscissor`
/// (Slint's solid fill, the S4 acceptance program) with main.vert, drawn
/// with `--uniform-values`; then femtovg's `fill_gradient*` and
/// `fill_gradient_conic*` (the general ffma of `mix()`, the dither's
/// reassociated `fract(..) - 0.5`, atan2's copysign and its `fabs`es of
/// non-negative values; plan #153). Every region is compared whole, with
/// the register count, preload and FAU words. (name, VS, FS, expect.json,
/// the vertex shader's uniform words are gl_ref.py's values.) Then the
/// `fill_image_gradient_conic*` programs of one look-up: the conic ramp's
/// `d` as the computed coordinate `vec2(d, 0.0)` (staging `MOV.i32 r2, r1^`,
/// `MOV.i32 r3, 0x0`; plan #153 texcoord). The two `*glyph*` ones make a
/// second look-up ([`MaliError::TexCount`], [`FEMTOVG`]).
const S4_POPULATION: [(&str, &str, &str, &str, bool); 34] = [
    ("s4_fmin", td!("s4_fmin.vert"), td!("s4_fmin.frag"), td!("s4_fmin.expect.json"), false),
    ("s4_fmax", td!("s4_fmax.vert"), td!("s4_fmax.frag"), td!("s4_fmax.expect.json"), false),
    ("s4_ffma", td!("s4_ffma.vert"), td!("s4_ffma.frag"), td!("s4_ffma.expect.json"), false),
    ("s4_fdot2", td!("s4_fdot2.vert"), td!("s4_fdot2.frag"), td!("s4_fdot2.expect.json"), false),
    ("s4_flt", td!("s4_flt.vert"), td!("s4_flt.frag"), td!("s4_flt.expect.json"), false),
    ("s4_fle", td!("s4_fle.vert"), td!("s4_fle.frag"), td!("s4_fle.expect.json"), false),
    ("s4_feq", td!("s4_feq.vert"), td!("s4_feq.frag"), td!("s4_feq.expect.json"), false),
    ("s4_fsqrt", td!("s4_fsqrt.vert"), td!("s4_fsqrt.frag"), td!("s4_fsqrt.expect.json"), false),
    ("s4_frcp", td!("s4_frcp.vert"), td!("s4_frcp.frag"), td!("s4_frcp.expect.json"), false),
    ("s4_ffract", td!("s4_ffract.vert"), td!("s4_ffract.frag"), td!("s4_ffract.expect.json"), false),
    ("s4_ffloor", td!("s4_ffloor.vert"), td!("s4_ffloor.frag"), td!("s4_ffloor.expect.json"), false),
    ("s4_fceil", td!("s4_fceil.vert"), td!("s4_fceil.frag"), td!("s4_fceil.expect.json"), false),
    ("s4_fsat", td!("s4_fsat.vert"), td!("s4_fsat.frag"), td!("s4_fsat.expect.json"), false),
    ("s4_fsign", td!("s4_fsign.vert"), td!("s4_fsign.frag"), td!("s4_fsign.expect.json"), false),
    ("s4_fneg", td!("s4_fneg.vert"), td!("s4_fneg.frag"), td!("s4_fneg.expect.json"), false),
    ("s4_b2f", td!("s4_b2f.vert"), td!("s4_b2f.frag"), td!("s4_b2f.expect.json"), false),
    ("s4_bcsel", td!("s4_bcsel.vert"), td!("s4_bcsel.frag"), td!("s4_bcsel.expect.json"), false),
    ("s4_icmp", td!("s4_icmp.vert"), td!("s4_icmp.frag"), td!("s4_icmp.expect.json"), false),
    ("s4_iconv", td!("s4_iconv.vert"), td!("s4_iconv.frag"), td!("s4_iconv.expect.json"), false),
    ("s4_fragcoord", td!("s4_fragcoord.vert"), td!("s4_fragcoord.frag"), td!("s4_fragcoord.expect.json"), false),
    (
        "fvg_fill_color_nodiscard_noscissor",
        td!("fvg_fill_color_nodiscard_noscissor.vert"),
        td!("fvg_fill_color_nodiscard_noscissor.frag"),
        td!("fvg_fill_color_nodiscard_noscissor.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient",
        td!("fvg_fill_gradient.vert"),
        td!("fvg_fill_gradient.frag"),
        td!("fvg_fill_gradient.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_glyph",
        td!("fvg_fill_gradient_glyph.vert"),
        td!("fvg_fill_gradient_glyph.frag"),
        td!("fvg_fill_gradient_glyph.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_glyph_nodiscard",
        td!("fvg_fill_gradient_glyph_nodiscard.vert"),
        td!("fvg_fill_gradient_glyph_nodiscard.frag"),
        td!("fvg_fill_gradient_glyph_nodiscard.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_nodiscard",
        td!("fvg_fill_gradient_nodiscard.vert"),
        td!("fvg_fill_gradient_nodiscard.frag"),
        td!("fvg_fill_gradient_nodiscard.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_nodiscard_noscissor",
        td!("fvg_fill_gradient_nodiscard_noscissor.vert"),
        td!("fvg_fill_gradient_nodiscard_noscissor.frag"),
        td!("fvg_fill_gradient_nodiscard_noscissor.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_conic",
        td!("fvg_fill_gradient_conic.vert"),
        td!("fvg_fill_gradient_conic.frag"),
        td!("fvg_fill_gradient_conic.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_conic_glyph",
        td!("fvg_fill_gradient_conic_glyph.vert"),
        td!("fvg_fill_gradient_conic_glyph.frag"),
        td!("fvg_fill_gradient_conic_glyph.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_conic_glyph_nodiscard",
        td!("fvg_fill_gradient_conic_glyph_nodiscard.vert"),
        td!("fvg_fill_gradient_conic_glyph_nodiscard.frag"),
        td!("fvg_fill_gradient_conic_glyph_nodiscard.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_conic_nodiscard",
        td!("fvg_fill_gradient_conic_nodiscard.vert"),
        td!("fvg_fill_gradient_conic_nodiscard.frag"),
        td!("fvg_fill_gradient_conic_nodiscard.expect.json"),
        true,
    ),
    (
        "fvg_fill_gradient_conic_nodiscard_noscissor",
        td!("fvg_fill_gradient_conic_nodiscard_noscissor.vert"),
        td!("fvg_fill_gradient_conic_nodiscard_noscissor.frag"),
        td!("fvg_fill_gradient_conic_nodiscard_noscissor.expect.json"),
        true,
    ),
    (
        "fvg_fill_image_gradient_conic",
        td!("fvg_fill_image_gradient_conic.vert"),
        td!("fvg_fill_image_gradient_conic.frag"),
        td!("fvg_fill_image_gradient_conic.expect.json"),
        true,
    ),
    (
        "fvg_fill_image_gradient_conic_nodiscard",
        td!("fvg_fill_image_gradient_conic_nodiscard.vert"),
        td!("fvg_fill_image_gradient_conic_nodiscard.frag"),
        td!("fvg_fill_image_gradient_conic_nodiscard.expect.json"),
        true,
    ),
    (
        "fvg_fill_image_gradient_conic_nodiscard_noscissor",
        td!("fvg_fill_image_gradient_conic_nodiscard_noscissor.vert"),
        td!("fvg_fill_image_gradient_conic_nodiscard_noscissor.frag"),
        td!("fvg_fill_image_gradient_conic_nodiscard_noscissor.expect.json"),
        true,
    ),
];

/// One program pair against its expect.json, the vertex shader built for
/// the fragment shader's record (its reads and moved code, as the link
/// builds it): both IDVS programs, the fragment program, every fact. The
/// fragment shader's uniform words are the trace's zeros (the draw's
/// values do not reach the FAU dump of the fragment stage, as for
/// fvgtest_inner); the vertex shader's are gl_ref.py's when `vs_values`.
fn check_program_for(name: &str, vs: &str, fs: &str, json: &str, vs_values: bool) {
    let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
    let v = compile_vs_for(vs, fs);
    let vp = VIEWPORT_64X64;
    /* gl_ref.py numbers the program's active uniforms: the vertex shader's
     * first (femtovg's `viewSize` is 0: 16 + c), which `uniform_names`
     * cannot read past the sources' preprocessor lines */
    let uv_v = |d: u16| {
        if !vs_values {
            return 0;
        }
        let (_, first, _) =
            v.uniforms.iter().find(|r| d >= r.1 && d < r.1 + r.2).expect("a uniform row holds the dword");
        (16.0 + (d - first) as f32).to_bits()
    };
    let (pos, var) = vs_regions(&v);
    let rp = region(json, "vs.position").expect("vs.position");
    assert_words(&format!("{name} vs.position"), &pos, &rp.words);
    assert_facts(&format!("{name} vs.position"), &rp, v.info.work_reg_count, v.info.preload, &v.info.fau, vp, &uv_v);
    /* a program whose fragment shader reads no varying has no varying
     * program (fvg_stencil_nodiscard: the trace found none) */
    match region(json, "vs.varying") {
        Some(rv) => {
            assert_words(&format!("{name} vs.varying"), &var, &rv.words);
            let sec = v.info.secondary.expect("a varying program");
            assert_facts(&format!("{name} vs.varying"), &rv, sec.work_reg_count, sec.preload, &v.info.fau, vp, &uv_v);
            assert_eq!(Some(sec.offset), secondary_offset(json), "{name}: secondary_offset");
        }
        None => {
            assert!(var.is_empty() && v.info.secondary.is_none(), "{name}: a varying program the trace has not");
            assert_eq!(secondary_offset(json), None, "{name}: secondary_offset");
        }
    }
    let rf = region(json, "fs.fragment").expect("fs.fragment");
    assert_words(&format!("{name} fs.fragment"), &trim(&f.code), &rf.words);
    assert_facts(&format!("{name} fs.fragment"), &rf, f.info.work_reg_count, f.info.preload, &f.info.fau, vp, &|_| 0);
    assert_eq!(f.code.len(), (rf.words.len() * 8 + 16).div_ceil(128) * 16, "{name}: the padding");
}

#[test]
fn the_s4_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json, values) in S4_POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program_for(name, vs, fs, json, values)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), S4_POPULATION.len(), failed.join("\n"));
}

/* ---- S5: if (2026-10-22)---------------------------------------------- */

/// The S5 population (#1384): femtovg's `fill_color.nodiscard` (Slint's
/// solid fill under a scissor: the `if` of `scissorMask`, two real blocks
/// with the varying loaded before it and the uniforms read inside, which
/// `nir_opt_peephole_select` therefore leaves, and the phi of its result),
/// `stencil.nodiscard` (the same `if`, dead: `result` is the constant
/// white, so `nir_opt_dead_cf` removes it and no varying is read: no
/// varying program), gl_test's fvgtest_cover (case 31: `strokeMask()`
/// and `scissorMask(fpos)` as two colour lanes), and the two programs
/// whose `if`s test an integer: `texture_copy_unclipped` (`texType == 1`
/// and `== 2` on `int(frag[10].z)`, both flattened to `CSEL`s over the
/// fp32 texels of `vec4 color`, the compare `ICMP_OR.u32.eq` against the
/// table's byte `0x1000000.b3`) and `fill_color.glyph.nodiscard` (the
/// scissor `if` as blocks with the `.v4` load sunk after its compare,
/// then `glyphTextureType == 1` flattened, the glyph texel stored to a
/// highp local and so fetched at fp32). Each byte-equal to the shim in
/// both stages, drawn as the S4 population was (`vs_values`: the vertex
/// shader's uniform words are gl_ref.py's).
const S5_POPULATION: [(&str, &str, &str, &str, bool); 5] = [
    (
        "fvg_texture_copy_unclipped",
        td!("fvg_texture_copy_unclipped.vert"),
        td!("fvg_texture_copy_unclipped.frag"),
        td!("fvg_texture_copy_unclipped.expect.json"),
        true,
    ),
    (
        "fvg_fill_color_glyph_nodiscard",
        td!("fvg_fill_color_glyph_nodiscard.vert"),
        td!("fvg_fill_color_glyph_nodiscard.frag"),
        td!("fvg_fill_color_glyph_nodiscard.expect.json"),
        true,
    ),
    (
        "fvg_fill_color_nodiscard",
        td!("fvg_fill_color_nodiscard.vert"),
        td!("fvg_fill_color_nodiscard.frag"),
        td!("fvg_fill_color_nodiscard.expect.json"),
        true,
    ),
    (
        "fvg_stencil_nodiscard",
        td!("fvg_stencil_nodiscard.vert"),
        td!("fvg_stencil_nodiscard.frag"),
        td!("fvg_stencil_nodiscard.expect.json"),
        true,
    ),
    ("fvgtest_cover", td!("fvgtest_cover.vert"), td!("fvgtest_cover.frag"), td!("fvgtest_cover.expect.json"), false),
];

#[test]
fn the_s5_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json, values) in S5_POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program_for(name, vs, fs, json, values)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), S5_POPULATION.len(), failed.join("\n"));
}

/* ---- S6: discard and the bounded loop (2026-10-23) --------------------- */

/// The S6 population (#1384): the discard programs. gl_test's
/// fvgtest_discard2 (case 33: one `if (ftcoord.y < -100.0) discard;`, the
/// smallest, `DISCARD.f32.lt` carrying the program's helper discard) and
/// fvgtest_discard (case 32: the stroke and the scissor `if`s with the
/// discard between them); femtovg's `fill_color`, `fill_color.glyph` and
/// `stencil`, the `.nodiscard` S5 programs plus `if (strokeAlpha <
/// strokeThr) discard;` (the compare fused into one `DISCARD.f32.lt r0,
/// u10.w0`, the scissor branches after it, `stencil`'s `DISCARD` right
/// before ATEST); and gl_test's discloop (case 213, s6_discloop: a discard
/// before and one after a `for` loop of four passes, which unrolls and
/// folds into `u_w * 10.0`, `* 20.0`, `* 30.0`, the sum `v_uv.x + v_uv.y`
/// of the second discard's compare moved into the vertex shader by the
/// link). Each byte-equal to the shim in both stages, the femtovg ones
/// drawn with gl_ref.py's uniform values (`vs_values`).
const S6_POPULATION: [(&str, &str, &str, &str, bool); 6] = [
    (
        "fvgtest_discard2",
        td!("fvgtest_discard2.vert"),
        td!("fvgtest_discard2.frag"),
        td!("fvgtest_discard2.expect.json"),
        false,
    ),
    (
        "fvgtest_discard",
        td!("fvgtest_discard.vert"),
        td!("fvgtest_discard.frag"),
        td!("fvgtest_discard.expect.json"),
        false,
    ),
    ("fvg_fill_color", td!("fvg_fill_color.vert"), td!("fvg_fill_color.frag"), td!("fvg_fill_color.expect.json"), true),
    (
        "fvg_fill_color_glyph",
        td!("fvg_fill_color_glyph.vert"),
        td!("fvg_fill_color_glyph.frag"),
        td!("fvg_fill_color_glyph.expect.json"),
        true,
    ),
    ("fvg_stencil", td!("fvg_stencil.vert"), td!("fvg_stencil.frag"), td!("fvg_stencil.expect.json"), true),
    ("s6_discloop", td!("s6_discloop.vert"), td!("s6_discloop.frag"), td!("s6_discloop.expect.json"), false),
];

#[test]
fn the_s6_population_is_byte_equal_to_mesa_26_2_1() {
    let mut failed = Vec::new();
    for (name, vs, fs, json, values) in S6_POPULATION {
        if let Err(e) = std::panic::catch_unwind(|| check_program_for(name, vs, fs, json, values)) {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
            failed.push(format!("{name}: {m}"));
        }
    }
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), S6_POPULATION.len(), failed.join("\n"));
    /* the record says so: the frame runs a discarding program with no
     * colour written and marks its coverage modified */
    for (name, _, fs, _, _) in S6_POPULATION {
        let f = compile(SHC_STAGE_FRAGMENT, fs).expect("compiles");
        assert!(f.info.can_discard, "{name}: can_discard");
    }
    let plain = compile(SHC_STAGE_FRAGMENT, td!("fvg_fill_color_nodiscard.frag")).expect("compiles");
    assert!(!plain.info.can_discard);
}

/// gl_test's databreak (case 87): a `for` of eight passes with a `break`
/// behind a varying's compare and a `continue` in pass 2 behind another.
/// Mesa's `complex_unroll` nests each pass in the previous pass's guard,
/// and `nir_opt_peephole_select` flattens the nest into one block: a
/// ladder of `FCMP_OR`/`CSEL` over constants, innermost pass first, the
/// `s * 8.0` moved into the vertex shader by the link
/// (docs/notes/mali-loop-break.md). Byte-equal to the shim in both stages.
#[test]
fn case87_databreak_is_byte_equal_to_mesa_26_2_1() {
    check_program_for(
        "case87_databreak",
        td!("case87_databreak.vert"),
        td!("case87_databreak.frag"),
        td!("case87_databreak.expect.json"),
        false,
    );
    let f = compile(SHC_STAGE_FRAGMENT, td!("case87_databreak.frag")).expect("compiles");
    assert!(!f.info.can_discard);
}

/// The 35 femtovg fragment programs (tools/shaders/femtovg, each under its
/// `#define`s as tools/mali/testdata/fvg_<name>.frag holds it, with
/// main.vert and its shim reference), and whether each compiles byte-equal
/// to the shim in both stages today: the count S6 is measured by
/// (fill_color.nodiscard.noscissor from S4; fill_color.nodiscard,
/// stencil.nodiscard, texture_copy_unclipped and
/// fill_color.glyph.nodiscard from S5; fill_color, fill_color.glyph and
/// stencil from S6; the gradient and conic programs from plan #153's ffma;
/// fill_image, fill_image_gradient and fill_image_gradient_conic, each
/// plain, nodiscard and nodiscard_noscissor, from its texcoord, the
/// computed coordinate). A program not counted is refused by a gap
/// (`the_corpus_verdicts_are_pinned` names it); one that becomes
/// byte-equal moves here with the step that made it.
const FEMTOVG: [(&str, bool); 35] = [
    ("fill_color", true),
    ("fill_color_glyph", true),
    ("fill_color_glyph_nodiscard", true),
    ("fill_color_nodiscard", true),
    ("fill_color_nodiscard_noscissor", true),
    ("fill_gradient", true),
    ("fill_gradient_glyph", true),
    ("fill_gradient_glyph_nodiscard", true),
    ("fill_gradient_nodiscard", true),
    ("fill_gradient_nodiscard_noscissor", true),
    ("fill_gradient_conic", true),
    ("fill_gradient_conic_glyph", true),
    ("fill_gradient_conic_glyph_nodiscard", true),
    ("fill_gradient_conic_nodiscard", true),
    ("fill_gradient_conic_nodiscard_noscissor", true),
    ("fill_image", true),
    ("fill_image_glyph", false),
    ("fill_image_glyph_nodiscard", false),
    ("fill_image_nodiscard", true),
    ("fill_image_nodiscard_noscissor", true),
    ("fill_image_gradient", true),
    ("fill_image_gradient_glyph", false),
    ("fill_image_gradient_glyph_nodiscard", false),
    ("fill_image_gradient_nodiscard", true),
    ("fill_image_gradient_nodiscard_noscissor", true),
    ("fill_image_gradient_conic", true),
    ("fill_image_gradient_conic_glyph", false),
    ("fill_image_gradient_conic_glyph_nodiscard", false),
    ("fill_image_gradient_conic_nodiscard", true),
    ("fill_image_gradient_conic_nodiscard_noscissor", true),
    ("filter_image", false),
    ("filter_image_nodiscard", false),
    ("stencil", true),
    ("stencil_nodiscard", true),
    ("texture_copy_unclipped", true),
];

/// The femtovg count: every program [`FEMTOVG`] counts is byte-equal to
/// the shim in both stages, every other is refused, and the count is 27
/// of 35.
#[test]
fn the_femtovg_programs_byte_equal_to_the_shim_are_counted() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut counted = Vec::new();
    let mut failed = Vec::new();
    for (name, equal) in FEMTOVG {
        let read = |ext: &str| std::fs::read_to_string(root.join(format!("fvg_{name}.{ext}"))).expect("the reference");
        let (vs, fs, json) = (read("vert"), read("frag"), read("expect.json"));
        if equal {
            let r = std::panic::catch_unwind(|| check_program_for(name, &vs, &fs, &json, true));
            match r {
                Ok(()) => counted.push(name),
                Err(e) => {
                    let m = e
                        .downcast_ref::<String>()
                        .cloned()
                        .unwrap_or_else(|| format!("{:?}", e.downcast_ref::<&str>()));
                    failed.push(format!("{name}: {m}"));
                }
            }
        } else if compile(SHC_STAGE_FRAGMENT, &fs).is_ok() {
            failed.push(format!("{name}: compiles, but is not counted (byte-equal, or not yet?)"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    assert_eq!(counted.len(), 27, "{counted:?}");
}

/// `nir_opt_peephole_select` and the `if` forms, against pan_shc (whose
/// SPIR-V front end makes the same NIR for these shapes): an `if` of one
/// operation over varyings loaded before it (s5_peep_small) and one of
/// twelve (s5_peep_big) lose their branches and are a `CSEL` of the
/// condition; an `if` with no else whose then side is nine operations over
/// uniforms (s5_if_noelse: past the GL front end's limit, and bifrost's
/// peephole may not speculate a `load_ubo`) keeps its `BRANCHZ`,
/// retargeted to the join when its empty else block goes
/// (`bi_opt_control_flow`), and loses the then side's jump to the block
/// laid out next; an `if` nested in such a then side (s5_if_nested) is
/// flattened by bifrost's call, its uniform lanes being the outer side's
/// loads already (`nir_opt_cse`), so the outer `if` alone keeps its two
/// branches around a then side of bcsels; constant phi sources
/// (s5_phi_const) are `IADD_IMM` moves at the ends of the sides
/// (`bi_out_of_ssa`). Each row says how many relative branches the
/// program keeps.
#[test]
fn the_if_forms_are_byte_equal_to_pan_shc() {
    let cases = [
        ("s5_peep_small", td!("s5_peep_small.frag"), td!("pan_shc-out/s5_peep_small.json"), 0),
        ("s5_peep_big", td!("s5_peep_big.frag"), td!("pan_shc-out/s5_peep_big.json"), 0),
        ("s5_if_noelse", td!("s5_if_noelse.frag"), td!("pan_shc-out/s5_if_noelse.json"), 1),
        ("s5_if_nested", td!("s5_if_nested.frag"), td!("pan_shc-out/s5_if_nested.json"), 2),
        ("s5_phi_const", td!("s5_phi_const.frag"), td!("pan_shc-out/s5_phi_const.json"), 2),
    ];
    for (name, fs, json, branches) in cases {
        check_fs_pan_shc(fs, json);
        let f = compile(SHC_STAGE_FRAGMENT, fs).expect("compiles");
        let n = trim(&f.code).iter().filter(|w| text(**w).starts_with("BRANCHZ.")).count();
        assert_eq!(n, branches, "{name}: relative branches");
    }
}

/// The code motion's record: fill_color's fragment record carries
/// `ftcoord.x * 2.0 - 1.0` (one expression of five nodes, the varying's
/// lane 0), the bytes round-trip through `encode`/`decode`, the vertex
/// shader built for it stores the result (its varying program has the
/// `FMA.f32 r0, r0, 2.0, -1.0`), and the one compiled alone does not, so
/// the link refuses that pair by its own line.
#[test]
fn fill_color_s_stroke_term_is_moved_into_the_vertex_shader() {
    use super::motion::{self, Moved, Node};
    let fs_src = td!("fvg_fill_color_nodiscard_noscissor.frag");
    let vs_src = td!("fvg_fill_color_nodiscard_noscissor.vert");
    let f = compile(SHC_STAGE_FRAGMENT, fs_src).expect("compiles");
    assert_eq!(f.info.moved.len(), 1);
    let m = f.info.moved[0];
    assert_eq!((m.location, m.comp, m.n), (0, 0, 5), "{m:?}");
    assert!(m.nodes().contains(&Node::Lane { location: 0, comp: 0 }));
    assert!(m.nodes().contains(&Node::Const(0x4000_0000)) && m.nodes().contains(&Node::Const(0xbf80_0000)));
    assert!(matches!(m.nodes()[m.n as usize - 1], Node::Op { op: super::nir::Alu::Fadd, .. }), "{m:?}");
    let mut bytes = [0u8; motion::MAX_BYTES];
    let n = motion::encode(&f.info.moved, &mut bytes).expect("encodes");
    let mut back = [Moved::NONE; motion::MAX_MOVED];
    assert_eq!(motion::decode(&bytes[..n], &mut back), Ok(1));
    assert_eq!(back[0], m);
    let fma = |s: &Stage| {
        vs_regions(s)
            .1
            .iter()
            .any(|w| text(*w).starts_with("FMA.f32") && text(*w).contains("0x40000000, 0x3F800000.neg"))
    };
    let built = compile_vs_for(vs_src, fs_src);
    assert!(fma(&built), "the program's vertex shader computes the term");
    assert_eq!(built.info.moved, f.info.moved);
    let alone = compile(SHC_STAGE_VERTEX, vs_src).expect("compiles");
    assert!(!fma(&alone) && alone.info.moved.is_empty(), "the vertex shader alone is the plain copy");
    /* fill_color itself (S6) carries the same move */
    let with_discard = compile(SHC_STAGE_FRAGMENT, include_str!("../../../../tools/shaders/femtovg/fill_color.frag"))
        .expect("compiles");
    assert_eq!(with_discard.info.moved, f.info.moved);
}

/// A dot product is not moved (motion.rs, module text: the linker sees one
/// `fdot2`), and the varying it reads is read whole: `dot(v.xy, vec2(2.0,
/// 3.0)) * v.z` compiles to pan_shc's bytes in both stages (the fragment
/// shader's `FMA t, v.y, 3.0, -0.0` with 3.0 in the FAU, `FMA v.x, 2.0, t`
/// and the product with v.z, all four lanes loaded; the vertex shader the
/// plain copy, every lane stored), with nothing moved and nothing
/// compacted.
#[test]
fn a_dot_product_is_not_moved_and_keeps_its_fma_by_pan_shc() {
    let (vs, fs, json) = (td!("s4_motion_dot.vert"), td!("s4_motion_dot.frag"), td!("pan_shc-out/s4_motion_dot.json"));
    let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
    assert!(f.info.moved.is_empty(), "{:?}", f.info.moved);
    assert!(!f.info.compacted);
    let pr = super::fragment_reads(&f.info.shader()).expect("encodes");
    assert_eq!((pr.reads(), pr.moved()), (&[(0u8, 0xfu8)][..], &[][..]));
    check_fs_pan_shc(fs, json);
    let built = compile_vs_for(vs, fs);
    check_vs_record_pan_shc(&built, json);
    let alone = compile(SHC_STAGE_VERTEX, vs).expect("compiles");
    assert_eq!(built.code, alone.code, "the program's vertex shader is the shader's own");
}

/// `(v.x + v.y) * v.z`: the sum (two lanes, no constant) is moved once,
/// into v.x's lane, and v.y is no longer read (motion.rs, module text:
/// the loads a moved post-dominator post-dominated are dead): the fragment
/// record reads x and z, its vertex shader computes `FADD x, y` and stores
/// it and z, compacted, as pan_shc's does, byte for byte in both stages.
#[test]
fn a_sum_of_two_lanes_is_moved_once_by_pan_shc() {
    use super::motion::Node;
    use super::nir::Alu;
    let (vs, fs, json) = (td!("s4_motion_sum.vert"), td!("s4_motion_sum.frag"), td!("pan_shc-out/s4_motion_sum.json"));
    let f = compile(SHC_STAGE_FRAGMENT, fs).expect("the fragment shader compiles");
    assert_eq!(f.info.moved.len(), 1, "{:?}", f.info.moved);
    let m = f.info.moved[0];
    assert_eq!((m.location, m.comp), (0, 0));
    assert_eq!(
        m.nodes(),
        &[
            Node::Lane { location: 0, comp: 0 },
            Node::Lane { location: 0, comp: 1 },
            Node::Op { op: Alu::Fadd, src: [0, 1, 0] }
        ]
    );
    assert!(f.info.compacted);
    let pr = super::fragment_reads(&f.info.shader()).expect("encodes");
    assert_eq!(pr.reads(), &[(0u8, 0b101u8)][..]);
    check_fs_pan_shc(fs, json);
    let built = compile_vs_for(vs, fs);
    check_vs_record_pan_shc(&built, json);
    assert_eq!(built.info.moved, f.info.moved);
}

/// The record's encoding of a moved expression numbers its operations
/// (motion.rs `OPS`: fadd 0, fmul 1, fneg 2, ffma 3, the numbers in every
/// record key, so fixed) and round-trips each; an operation outside the
/// table is the back end's defect, never another operation's number; an
/// entry of no node does not decode.
#[test]
fn a_moved_expression_s_operations_round_trip_by_number() {
    use super::motion::{self, Moved, Node};
    use super::nir::Alu;
    use super::Defect;
    let op = |op, src| Node::Op { op, src };
    let mut m = Moved { location: 1, comp: 2, ..Moved::NONE };
    let nodes = [
        Node::Lane { location: 1, comp: 2 },
        Node::Const(0x4000_0000),
        Node::Lane { location: 1, comp: 0 },
        op(Alu::Ffma, [0, 1, 2]),
        op(Alu::Fneg, [3, 0, 0]),
        op(Alu::Fmul, [4, 1, 0]),
        op(Alu::Fadd, [5, 2, 0]),
    ];
    m.nodes[..nodes.len()].copy_from_slice(&nodes);
    m.n = nodes.len() as u8;
    let mut bytes = [0u8; motion::MAX_BYTES];
    let n = motion::encode(&[m], &mut bytes).expect("every operation has a number");
    assert_eq!(n, 3 + 6 * nodes.len());
    /* the numbers, as the key bytes have them */
    let number = |k: usize| bytes[3 + 6 * k + 1];
    assert_eq!((bytes[3 + 6 * 3], number(3), number(4), number(5), number(6)), (2, 3, 2, 1, 0));
    let mut back = [Moved::NONE; motion::MAX_MOVED];
    assert_eq!(motion::decode(&bytes[..n], &mut back), Ok(1));
    assert_eq!(back[0], m);
    /* an operation the table lacks */
    let mut bad = m;
    bad.nodes[3] = op(Alu::Fmax, [0, 1, 0]);
    assert_eq!(motion::encode(&[bad], &mut bytes), Err(MaliError::Defect(Defect::MovedOp)));
    /* bytes encode never writes */
    let defect = Err(MaliError::Defect(Defect::IrIndex));
    assert_eq!(motion::decode(&[1, 2, 0], &mut back), defect, "an entry of no node");
    assert_eq!(motion::decode(&[1, 2, 1, 2, 4, 0, 0, 0, 0], &mut back), defect, "a number past the table");
    assert_eq!(motion::decode(&[1, 2, 1, 2, 3, 0, 1, 2, 0], &mut back), Ok(1), "ffma's number");
    assert_eq!(back[0].nodes(), &[op(Alu::Ffma, [0, 1, 2])]);
}

/// A moved `ffma` reaches the vertex graph as an `ffma` (`apply`, after the
/// record's encoding and decoding): `ffma(v.x, 2.0, v.y)` in v.x's lane is
/// one Ffma node over the attribute's lanes and the constant, and the lane
/// is that node.
#[test]
fn a_moved_ffma_is_applied_as_an_ffma() {
    use super::motion::{self, Moved, Node};
    use super::nir::{Alu, Kind, Nir, Val};
    let mut m = Moved { location: 0, comp: 0, ..Moved::NONE };
    let nodes = [
        Node::Lane { location: 0, comp: 0 },
        Node::Const(0x4000_0000),
        Node::Lane { location: 0, comp: 1 },
        Node::Op { op: Alu::Ffma, src: [0, 1, 2] },
    ];
    m.nodes[..nodes.len()].copy_from_slice(&nodes);
    m.n = nodes.len() as u8;
    let mut bytes = [0u8; motion::MAX_BYTES];
    let n = motion::encode(&[m], &mut bytes).expect("encodes");
    let mut back = [Moved::NONE; motion::MAX_MOVED];
    assert_eq!(motion::decode(&bytes[..n], &mut back), Ok(1));
    let r = store::with_scratch(|s| {
        let mut vs = Nir::new(s).expect("a graph");
        let attr = vs.load(Kind::Input { loc: 0, comps: 4 }).expect("the attribute");
        let lanes = [Val::Lane(attr, 0), Val::Lane(attr, 1), Val::Lane(attr, 2), Val::Lane(attr, 3)];
        let mut varyings = [Some(lanes)];
        motion::apply(&mut vs, &mut varyings, &back[..1]).expect("applies");
        let Some(Val::Lane(id, 0)) = varyings[0].map(|l| l[0]) else { panic!("{:?}", varyings[0]) };
        (vs.nodes[id as usize].kind, vs.nodes[id as usize].src, varyings[0].map(|l| l[1]))
    })
    .expect("the scratch arena is free");
    assert_eq!(r.0, Kind::Alu(Alu::Ffma));
    assert_eq!(r.1, [Val::Lane(0, 0), Val::Const(0x4000_0000), Val::Lane(0, 1)]);
    assert_eq!(r.2, Some(Val::Lane(0, 1)), "the other lanes stay the attribute's");
}

/* ---- the arithmetic refusals (plan #153 alu-lower) -------------------- */

/// The study-alu references the online compiler takes, each pair drawn once
/// on the shim without `--uniform-values` (docs/notes/mali-alu-ops.md: name,
/// VS, FS, expect.json).
const ALU_POPULATION: [(&str, &str, &str, &str); 1] =
    [("alu_branchlane", td!("alu_branchlane.vert"), td!("alu_branchlane.frag"), td!("alu_branchlane.expect.json"))];

/// Case 73's branchlane: two `if (u_c.y > 0.5) discard;` of a mediump
/// shader, each a 16-bit `flt` of one lane (`FCMP.v2f16.lt.m1 0.5.h00,
/// u2.w0.h11`) and `demote_if`'s `DISCARD.b32 mask.h00`, which
/// `bi_fuse_discard_fcmp` makes `DISCARD.f32.lt 0x38000000.h1, u2.w0.h1`
/// (the swizzles composed), then the constant colour. Every program the
/// shim's to the byte; the fragment program discards.
#[test]
fn the_alu_population_is_byte_equal_to_mesa_26_2_1() {
    for (name, vs, fs, json) in ALU_POPULATION {
        check_program_for(name, vs, fs, json, false);
    }
    let f = compile(SHC_STAGE_FRAGMENT, td!("alu_branchlane.frag")).expect("alu_branchlane.frag compiles");
    assert!(f.info.can_discard);
}

/// alu_branchquad's discards test lanes y and w of a mediump varying: Mesa
/// keeps the varying whole (`LD_VAR_BUF_IMM ... .v4`, four vertex stores),
/// a link rule no other reference pins, so the compare of a varying lane
/// stays refused.
#[test]
fn a_mediump_discard_of_a_varying_lane_is_refused() {
    assert_eq!(gap(SHC_STAGE_FRAGMENT, td!("alu_branchquad.frag")), MaliError::DiscardForm);
}
