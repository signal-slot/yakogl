// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Derived from Mesa 26.2.1 src/imagination/pco/ (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! A disassembler: one decoded group written as text.
//!
//! It exists so a person can read what the decoder made of a blob and compare
//! it with what the shader was meant to be. It writes through
//! [`core::fmt::Write`], so it works in a `no_std` build against a fixed
//! buffer as well as against a `String` on the host.

use super::decode::Group;
use super::isa::{bit_set, ops_for, Region};
use super::isa_tables::ENUMS;
use core::fmt::{Result, Write};

/// Name a field's value through its enumeration, when it has one.
fn field_text(w: &mut impl Write, r: &Region, use_idx: usize) -> Result {
    let set = bit_set(r.set);
    let u = &r.variant().uses[use_idx];
    let f = &set.fields[u.field];
    let raw = r.vals[use_idx];
    if let Some(e) = f.enum_id {
        if let Some(n) = ENUMS[e].name_of(raw) {
            return write!(w, "{}={}", u.name, n);
        }
    }
    write!(w, "{}={}", u.name, f.xform.decode(raw))
}

fn region_text(w: &mut impl Write, label: &str, r: &Region) -> Result {
    write!(w, "    {label}: {} [{}B]", r.variant().name, r.bytes())?;
    for (i, u) in r.variant().uses.iter().enumerate() {
        if u.fixed.is_some() || u.name.is_empty() {
            continue;
        }
        w.write_str(" ")?;
        field_text(w, r, i)?;
    }
    w.write_str("\n")
}

/// Write one group as a listing, at shader offset `off`.
pub fn group_text(w: &mut impl Write, off: usize, g: &Group) -> Result {
    let alu = match g.alutype {
        0 => "main",
        2 => "bitwise",
        _ => "control",
    };
    writeln!(w, "[{off:04x}] {} bytes, {alu}", g.total)?;
    region_text(w, "hdr", &g.hdr)?;
    for (slot, phase, r) in g.instrs() {
        write!(w, "    phase{slot}")?;
        let mut first = true;
        for op in ops_for(r, phase) {
            write!(w, "{}{}", if first { " " } else { "|" }, op.name)?;
            first = false;
        }
        w.write_str("\n")?;
        region_text(w, "  instr", r)?;
    }
    for (label, r) in [("lower_src", &g.lower), ("upper_src", &g.upper), ("iss", &g.iss), ("dst", &g.dst)] {
        if let Some(r) = r {
            region_text(w, label, r)?;
        }
    }
    if g.word_pad != 0 || g.align_pad != 0 {
        writeln!(w, "    pad: word={} align={}", g.word_pad, g.align_pad)?;
    }
    Ok(())
}

/// gl_test case 150 (shprobe) arm B's vertex shader, compiled and listed:
/// the shared registers it reads are the ones the probe names.
#[cfg(all(test, feature = "gpu-powervr"))]
mod shprobe_tests {
    use super::super::decode;
    use crate::shaderc::online::{store, SHC_ONLINE};
    use crate::shaderc::{ShcShader, SHC_STAGE_VERTEX};
    use core::ffi::c_char;

    /// The text gl_test.c's shp_vs_src builds for `n_vec4` (keep the two alike).
    fn shp_vs_src(n_vec4: usize) -> String {
        const IDX: [usize; 9] = [0, 32, 60, 63, 64, 65, 66, 67, 69];
        const COMP: [char; 4] = ['x', 'y', 'z', 'w'];
        let mut s = format!(
            "#version 100\nattribute vec4 a_pos;\nuniform vec4 u_p[{n_vec4}];\nvarying vec4 v_c;\nvoid main() {{\n    vec4 r = u_p[58].y * vec4(1.0, 0.0, 1.0, 1.0);\n"
        );
        for (j, &i) in IDX.iter().enumerate().filter(|(_, &i)| i < n_vec4) {
            s += &format!("    r += u_p[{}].{} * u_p[{i}];\n", 56 + j / 4, COMP[j % 4]);
        }
        s + "    v_c = r;\n    gl_Position = a_pos;\n}\n"
    }

    /// The listing of one arm's vertex shader, its record, and its bytes.
    fn listing(n_vec4: usize) -> (String, ShcShader, Vec<u8>) {
        compiled(SHC_STAGE_VERTEX, &shp_vs_src(n_vec4))
    }

    /// `src` compiled for `stage` by the online back end: listing, record, bytes.
    fn compiled(stage: u32, src: &str) -> (String, ShcShader, Vec<u8>) {
        let _s = store::test_lock();
        let text = format!("{src}\0");
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let f = SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, a log of the size given.
        let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        // SAFETY: the back end NUL-terminates the log.
        let why = unsafe { crate::util::cstr_bytes(log.as_ptr()) };
        assert_eq!(rc, 0, "compiles; rc {rc}: {}", core::str::from_utf8(why).unwrap_or("<not utf8>"));
        let bytes: Vec<u8> = out.usc_words().iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut t = String::new();
        let mut off = 0;
        while off < bytes.len() {
            let g = decode::decode_group(&bytes, off).expect("our own output decodes");
            super::group_text(&mut t, off, &g).expect("listing");
            off += g.total;
        }
        (t, out, bytes)
    }

    /// The shared registers a listing reads, and the largest one it names
    /// DIRECTLY. A direct read is `sbN=shared sN=<n>`. An indexed read is
    /// `sbN=idx0 sN=<3 | k << 3>` (shared[idx0 + k]), with idx0 the value of
    /// the last `imm32=<v>` written to `dbN=idx0 dN=5` before it.
    fn shared_reads(t: &str) -> (Vec<u32>, Option<u32>) {
        let mut v: Vec<u32> = Vec::new();
        let mut direct_max = None;
        let mut idx0: Option<u32> = None;
        let mut imm: Option<u32> = None;
        let num = |w: &str| w.split_once('=').and_then(|(_, n)| n.parse::<u32>().ok());
        let words: Vec<&str> = t.split_whitespace().collect();
        for w in words.windows(2) {
            if w[0].starts_with("imm32=") {
                imm = num(w[0]);
            } else if w[0] == "dbN=idx0" && w[1] == "dN=5" {
                idx0 = imm;
            } else if w[0].starts_with("sb") && w[0].ends_with("=shared") {
                if let Some(n) = num(w[1]) {
                    v.push(n);
                    direct_max = direct_max.max(Some(n));
                }
            } else if w[0].starts_with("sb") && w[0].ends_with("=idx0") {
                let s = num(w[1]).expect("an indexed source names its target");
                assert_eq!(s & 7, 3, "an idx0 read of the shared file: {w:?}");
                v.push(idx0.expect("idx0 is written before it is read") + (s >> 3));
            }
        }
        v.sort_unstable();
        v.dedup();
        (v, direct_max)
    }

    /// Arm B's block is u_p[70] from sh0 (280 dwords), and the shader reads
    /// exactly the vec4s the probe names (u_p[0, 32, 60, 63, 64..67, 69]) and
    /// the selectors u_p[56..58].y (sh224..233): no other shared register, so
    /// a cell's colour on the board is the read of the registers it names.
    /// No direct read goes past sh255 (`prepare::index_high_shared`).
    #[test]
    fn shprobe_arm_b_reads_the_shared_registers_it_names() {
        let (t, out, _) = listing(70);
        assert_eq!(out.num_uniforms, 1, "one uniform, u_p");
        // SAFETY: the shader's own count describes its own table.
        let u = unsafe { &*out.uniforms };
        assert_eq!((u.dword_off, u.dwords), (0, 280), "u_p[70] is sh0..sh279");
        let mut want: Vec<u32> = Vec::new();
        for i in [0u32, 32, 60, 63, 64, 65, 66, 67, 69] {
            want.extend(4 * i..4 * i + 4);
        }
        want.extend(224..234);
        want.sort_unstable();
        let (reads, direct_max) = shared_reads(&t);
        assert_eq!(reads, want, "the shared registers read:\n{t}");
        assert_eq!(direct_max, Some(255), "no direct read past sh255:\n{t}");
    }

    /// Arm A (u_p[64]) stays inside sh0..sh255: the control reads nothing past it.
    #[test]
    fn shprobe_arm_a_stays_below_sh256() {
        let (t, out, _) = listing(64);
        // SAFETY: as above.
        let u = unsafe { &*out.uniforms };
        assert_eq!((u.dword_off, u.dwords), (0, 256), "u_p[64] is sh0..sh255");
        let (r, _) = shared_reads(&t);
        assert_eq!(r.last(), Some(&255), "the last read is u_p[63].w:\n{t}");
        assert!(!t.contains("dbN=idx0"), "nothing past sh255, so no index register:\n{t}");
    }

    /// The read of sh256 is pco's pair, byte for byte: pco_shc (Mesa 26.2.1,
    /// `try_legalize_large_hwreg_offsets`) compiles this same arm-B text to
    /// `86 92 40 13 00 01 00 00 00 00 85 0c` (movi32 idx0 = 256) then
    /// `35 82 00 87 83 0c 00 00 00 45` (mbyp r5, shared[idx0 + 0]), at its
    /// offsets 0x19a and 0x1a6 (records/am62p/sh-index). The direct form this
    /// replaces, `35 82 00 87 80 08 01 00 00 45`, read sh0 on the board; the
    /// test above asserts no direct read past sh255 is left.
    ///
    /// The mbyp's last byte is its destination, the temporary the allocator
    /// chose (pco's r5; ours moves with the program, r1 since the multiply-add
    /// fusion): every byte before it is compared, and the destination is
    /// checked to be a temporary.
    #[test]
    fn shprobe_sh256_read_is_pcos_pair() {
        let (_, _, bytes) = listing(70);
        const PAIR: [u8; 21] = [
            0x86, 0x92, 0x40, 0x13, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x85, 0x0c, //
            0x35, 0x82, 0x00, 0x87, 0x83, 0x0c, 0x00, 0x00, 0x00,
        ];
        let at = bytes.windows(PAIR.len()).position(|w| w == PAIR);
        let at = at.expect("the sh256 read is pco's movi32 + mbyp pair, up to the mbyp's destination");
        let g = decode::decode_group(&bytes, at + 12).expect("the mbyp decodes");
        let mut t = String::new();
        super::group_text(&mut t, at + 12, &g).expect("listing");
        assert!(t.contains("dbN=temp"), "the mbyp writes a temporary:\n{t}");
    }

    /// A fragment shader sampling 14 textures: descriptor 13's image state
    /// is sh260 and its sampler state sh268 (20 dwords a descriptor). pco
    /// (Mesa 26.2.1 pco_shc, the same text) sets idx0 = 260 and reads both
    /// through it, in the 11-bit form (`3lo_3b11i_2b8i_3b11i_3m`, image
    /// `idx0 s0=3`, sampler `idx0 s2=67` = shared[idx0 + 8]); its idx0 write
    /// is `86 92 40 13 04 01 00 00 00 00 85 0c`. The other 13 samples stay
    /// direct.
    #[test]
    fn a_sample_of_descriptor_13_reads_it_through_idx0_as_pco_does() {
        let mut src = String::from("#version 100\nprecision mediump float;\nvarying vec2 v_t;\n");
        for i in 0..14 {
            src += &format!("uniform sampler2D u_s{i};\n");
        }
        src += "void main() {\n    vec4 c = vec4(0.0);\n";
        for i in 0..14 {
            src += &format!("    c += texture2D(u_s{i}, v_t);\n");
        }
        src += "    gl_FragColor = c;\n}\n";
        let (t, _, bytes) = compiled(crate::shaderc::SHC_STAGE_FRAGMENT, &src);
        const IDX0_260: [u8; 12] = [0x86, 0x92, 0x40, 0x13, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x85, 0x0c];
        assert!(bytes.windows(IDX0_260.len()).any(|w| w == IDX0_260), "idx0 = 260, pco's bytes:\n{t}");
        let smp = t.split("\n[").skip_while(|g| !g.contains("imm32=260")).nth(1).expect("a group after the idx0 write");
        assert!(smp.contains("smp"), "the sample follows its idx0 write:\n{smp}");
        assert!(
            smp.contains("lower_src: 3lo_3b11i_2b8i_3b11i_3m [6B] sb0=idx0 s0=3 ") && smp.contains(" sb2=idx0 s2=67 "),
            "pco's form and operands:\n{smp}"
        );
        let (_, direct_max) = shared_reads(&t);
        assert!(direct_max.is_some_and(|m| m < 256), "no direct read past sh255:\n{t}");
    }

    /// The text gl_test.c's shp_src builds for case 166's arms A and B (the
    /// fragment stage; keep the two alike).
    fn fshp_fs_src(n_vec4: usize) -> String {
        const IDX: [usize; 9] = [0, 32, 60, 63, 64, 65, 66, 67, 69];
        const COMP: [char; 4] = ['x', 'y', 'z', 'w'];
        let mut s = format!(
            "#version 100\nprecision mediump float;\nuniform vec4 u_p[{n_vec4}];\nvoid main() {{\n    vec4 r = u_p[58].y * vec4(1.0, 0.0, 1.0, 1.0);\n"
        );
        for (j, &i) in IDX.iter().enumerate().filter(|(_, &i)| i < n_vec4) {
            s += &format!("    r += u_p[{}].{} * u_p[{i}];\n", 56 + j / 4, COMP[j % 4]);
        }
        s + "    gl_FragColor = r;\n}\n"
    }

    /// The text gl_test.c's fshp_units_src builds for case 166's arm C.
    fn fshp_units_src() -> String {
        const COMP: [char; 4] = ['x', 'y', 'z', 'w'];
        let mut s = String::from("#version 100\nprecision mediump float;\nuniform vec4 u_w[4];\n");
        for k in 0..16 {
            s += &format!("uniform sampler2D u_s{k};\n");
        }
        s += "void main() {\n    vec2 c = vec2(0.5);\n    vec4 r = vec4(0.0);\n";
        for k in 0..16 {
            s += &format!("    r += u_w[{}].{} * texture2D(u_s{k}, c);\n", k / 4, COMP[k % 4]);
        }
        s + "    gl_FragColor = r;\n}\n"
    }

    /// The vec4s u_p[i] of `idx`, their selectors (selector j is sh224 + j)
    /// and the constant's selector sh233, as sh numbers.
    fn probe_regs(idx: &[u32]) -> Vec<u32> {
        let mut want: Vec<u32> = idx.iter().flat_map(|&i| 4 * i..4 * i + 4).collect();
        want.extend(224..224 + idx.len() as u32);
        want.push(233);
        want.dedup();
        want.sort_unstable();
        want
    }

    /// Case 166 arm B: the fragment block is u_p[70] from sh0 (no sampler, so
    /// no descriptor ahead of it), and the shader reads exactly the vec4s the
    /// probe names and the selectors, as in the vertex stage; nothing past
    /// sh255 is read directly.
    #[test]
    fn fshprobe_arm_b_reads_the_shared_registers_it_names() {
        let (t, out, _) = compiled(crate::shaderc::SHC_STAGE_FRAGMENT, &fshp_fs_src(70));
        assert_eq!(out.num_uniforms, 1, "one uniform, u_p");
        // SAFETY: the shader's own count describes its own table.
        let u = unsafe { &*out.uniforms };
        assert_eq!((u.dword_off, u.dwords), (0, 280), "u_p[70] is sh0..sh279");
        let (reads, direct_max) = shared_reads(&t);
        assert_eq!(reads, probe_regs(&[0, 32, 60, 63, 64, 65, 66, 67, 69]), "the shared registers read:\n{t}");
        assert_eq!(direct_max, Some(255), "no direct read past sh255:\n{t}");
    }

    /// Case 166 arm A (u_p[64]) stays inside sh0..sh255: no index register.
    #[test]
    fn fshprobe_arm_a_stays_below_sh256() {
        let (t, _, _) = compiled(crate::shaderc::SHC_STAGE_FRAGMENT, &fshp_fs_src(64));
        let (reads, _) = shared_reads(&t);
        assert_eq!(reads, probe_regs(&[0, 32, 60, 63]), "the shared registers read:\n{t}");
        assert!(!t.contains("dbN=idx0"), "nothing past sh255, so no index register:\n{t}");
    }

    /// Case 166 arm B's read of sh256 is the idx0 form: `movi32 idx0 = 256`
    /// (pco's bytes, the same as the vertex stage's, test
    /// `shprobe_sh256_read_is_pcos_pair`) followed by a move whose source is
    /// `shared[idx0 + 0]` (`sb0=idx0 s0=3`).
    #[test]
    fn fshprobe_sh256_read_is_the_idx0_form() {
        let (t, _, bytes) = compiled(crate::shaderc::SHC_STAGE_FRAGMENT, &fshp_fs_src(70));
        const IDX0_256: [u8; 12] = [0x86, 0x92, 0x40, 0x13, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x85, 0x0c];
        assert!(bytes.windows(IDX0_256.len()).any(|w| w == IDX0_256), "idx0 = 256, pco's bytes:\n{t}");
        let mv = t.split("\n[").skip_while(|g| !g.contains("imm32=256")).nth(1).expect("a group after the idx0 write");
        assert!(mv.contains("mbyp") && mv.contains(" sb0=idx0 s0=3 "), "a move of shared[idx0 + 0]:\n{mv}");
    }

    /// Case 166 arm C: 16 fragment samplers put the descriptors at sh0..sh319
    /// (20 dwords each) and u_w[4] at sh320..sh335. The shader reads every
    /// descriptor's image (sh 20k) and sampler (sh 20k + 8) state, and the
    /// weights; samplers 13, 14, 15 (sh260, 280, 300) and the weights are read
    /// through idx0, every other register directly below sh256.
    #[test]
    fn fshprobe_arm_c_reads_sixteen_descriptors_and_the_weights() {
        let (t, _, _) = compiled(crate::shaderc::SHC_STAGE_FRAGMENT, &fshp_units_src());
        let mut want: Vec<u32> = (0..16u32).flat_map(|k| [20 * k, 20 * k + 8]).collect();
        want.extend(320..336);
        want.sort_unstable();
        let (reads, direct_max) = shared_reads(&t);
        assert_eq!(reads, want, "the shared registers read:\n{t}");
        assert_eq!(direct_max, Some(248), "the last direct read is descriptor 12's sampler state:\n{t}");
        for base in [260, 280, 300] {
            let smp = t
                .split("\n[")
                .skip_while(|g| !g.contains(&format!("imm32={base}\n")))
                .nth(1)
                .unwrap_or_else(|| panic!("idx0 = {base} for descriptor {}:\n{t}", base / 20));
            assert!(
                smp.contains("smp")
                    && smp.contains("lower_src: 3lo_3b11i_2b8i_3b11i_3m [6B] sb0=idx0 s0=3 ")
                    && smp.contains(" sb2=idx0 s2=67 "),
                "descriptor {}: image shared[idx0 + 0], sampler shared[idx0 + 8], pco's form:\n{smp}",
                base / 20
            );
        }
    }

    /// The `fcnorm` of each `smp` in a listing, in order.
    fn smp_fcnorms(t: &str) -> Vec<u32> {
        t.split("\n[")
            .filter(|g| g.contains("phase") && g.contains("smp"))
            .filter_map(|g| g.split_whitespace().find_map(|w| w.strip_prefix("fcnorm=")?.parse().ok()))
            .collect()
    }

    /// An `isampler2D` / `usampler2D` look-up is pco's `smp` without
    /// `fcnorm` (Mesa pco_nir_tex.c:236: `fcnorm` is "the destination type
    /// is float"), in each form the 2D arms select -- the implicit level,
    /// a replaced level, a texel fetch -- and a `sampler2D` one keeps it.
    #[test]
    fn an_integer_sampler_reads_without_fcnorm() {
        /* an ES 2.0 report refuses every `#version 300 es` source */
        if crate::REPORT_ES2 {
            return;
        }
        let fs = |sampler: &str, ty: &str, lookup: &str| {
            std::format!(
                "#version 300 es\nprecision highp float;\nuniform highp {sampler} u_t;\nin vec2 v_uv;\n\
                 layout(location = 0) out vec4 o;\nvoid main() {{\n    {ty} t = {lookup};\n    o = vec4(t) / 255.0;\n}}\n"
            )
        };
        for (sampler, ty, want) in [("usampler2D", "uvec4", 0u32), ("isampler2D", "ivec4", 0), ("sampler2D", "vec4", 1)]
        {
            for lookup in ["texture(u_t, v_uv)", "textureLod(u_t, v_uv, 1.0)", "texelFetch(u_t, ivec2(v_uv), 0)"] {
                let (t, _, _) = compiled(crate::shaderc::SHC_STAGE_FRAGMENT, &fs(sampler, ty, lookup));
                assert_eq!(smp_fcnorms(&t), [want], "{sampler} {lookup}:\n{t}");
            }
        }
    }
}
