// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every ES 1.00 gl_test shader, ours against the reference, field by field.
//!
//! gl_test is the suite that runs on the board, so when a case draws the wrong
//! picture this is the instrument that says why: the same source through both
//! back ends in one process, with nothing excluded. It is what found the
//! fragment stage's register 0.
//!
//! Needs the compile server (`YAKOGL_SHADERC_SERVER`, default 127.0.0.1:7779)
//! and so is not part of `check.sh`; run it by name:
//!
//! ```sh
//! cargo test --no-default-features \
//!     --features host-test,gpu-vivante,shaderc-remote,shaderc-online \
//!     --test gltest_diff -- --nocapture
//! ```

#![cfg(all(feature = "shaderc-remote", feature = "shaderc-online", feature = "gpu-vivante"))]
use core::ffi::c_char;
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

fn run(remote: bool, stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let mut s = String::from(src);
    s.push('\0');
    let f = if remote {
        yakogl::shaderc::remote::SHC_REMOTE.compile.unwrap()
    } else {
        yakogl::shaderc::online::SHC_ONLINE.compile.unwrap()
    };
    let rc = unsafe { f(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        let m: Vec<u8> = log.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        Err(String::from_utf8_lossy(&m).into_owned())
    }
}

fn ops(sh: &ShcShader) -> Vec<String> {
    sh.usc_words()
        .chunks_exact(4)
        .map(|q| match decode_instr(&[q[0], q[1], q[2], q[3]]) {
            Ok(i) => i.op().name.to_string(),
            Err(e) => format!("<{e:?}>"),
        })
        .collect()
}

fn viv_io(sh: &ShcShader) -> String {
    let v = &sh.viv;
    let f = |io: &yakogl::shaderc::ShcVivIo| {
        let n: Vec<u8> = io.name().iter().map(|c| *c as u8).collect();
        format!("{}:r{}c{}", String::from_utf8_lossy(&n), io.reg, io.components)
    };
    format!(
        "in[{}] out[{}]",
        v.in_[..v.num_in as usize].iter().map(f).collect::<Vec<_>>().join(" "),
        v.out[..v.num_out as usize].iter().map(&f).collect::<Vec<_>>().join(" ")
    )
}

#[test]
fn every_gl_test_es100_shader() {
    let dir = "tools/shaders/gl_test_es100";
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".vert") || n.ends_with(".frag"))
        .collect();
    names.sort();
    println!("\n=== ES 1.00 gl_test shaders: online vs remote ===");
    for name in &names {
        let src = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        let stage = if name.ends_with(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        match (run(false, stage, &src), run(true, stage, &src)) {
            (Ok(o), Ok(r)) => {
                let same_words = o.usc_words() == r.usc_words();
                let (x, y) = (ops(&o), ops(&r));
                let mut flags = Vec::new();
                if o.temps != r.temps {
                    flags.push(format!("temps {} vs {}", o.temps, r.temps));
                }
                if o.vs_output_size != r.vs_output_size {
                    flags.push(format!("vs_output_size {} vs {}", o.vs_output_size, r.vs_output_size));
                }
                if o.viv.ps_color_out_reg != r.viv.ps_color_out_reg {
                    flags.push(format!("PS_COLOR {:?} vs {:?}", o.viv.ps_color_out_reg, r.viv.ps_color_out_reg));
                }
                if o.uses_w != r.uses_w {
                    flags.push(format!("uses_w {} vs {}", o.uses_w, r.uses_w));
                }
                if o.viv.num_in != r.viv.num_in || o.viv.num_out != r.viv.num_out {
                    flags.push(format!("io {} vs {}", viv_io(&o), viv_io(&r)));
                }
                if o.num_attribs != r.num_attribs {
                    flags.push(format!("attribs {} vs {}", o.num_attribs, r.num_attribs));
                }
                if o.uniform_dwords != r.uniform_dwords {
                    flags.push(format!("uniform_dwords {} vs {}", o.uniform_dwords, r.uniform_dwords));
                }
                if o.uniform_sh_dwords != r.uniform_sh_dwords {
                    flags.push(format!("uniform_sh {} vs {}", o.uniform_sh_dwords, r.uniform_sh_dwords));
                }
                let verdict = if same_words {
                    "IDENTICAL"
                } else if x == y {
                    "same ops"
                } else {
                    "DIFFERENT OPS"
                };
                println!("\n  {name:<22} {verdict}   {}", flags.join(" | "));
                if !same_words && x != y {
                    println!("      ours  {x:?}");
                    println!("      ref   {y:?}");
                }
            }
            (a, b) => println!("\n  {name:<22} ours_ok={} ref_ok={}  {:?}", a.is_ok(), b.is_ok(), a.err().or(b.err())),
        }
    }
}

fn dump(tag: &str, sh: &ShcShader) {
    println!(
        "  {tag}: temps {} ps_color_out_reg {} vs_output_size {} uses_w {}",
        sh.temps, sh.viv.ps_color_out_reg, sh.vs_output_size, sh.uses_w
    );
    println!("      {}", viv_io(sh));
    for (i, q) in sh.usc_words().chunks_exact(4).enumerate() {
        match decode_instr(&[q[0], q[1], q[2], q[3]]) {
            Ok(ins) => {
                let d = ins.get("DST").map(|v| ((v >> 3) & 0x7f, (v >> 10) & 0xf));
                let s: Vec<String> = (0..3)
                    .filter(|n| ins.get(&format!("SRC{n}_USE")) == Some(1))
                    .map(|n| {
                        format!(
                            "s{n}=g{}r{}sw{:#x}",
                            ins.get(&format!("SRC{n}_RGROUP")).unwrap_or(0),
                            ins.get(&format!("SRC{n}_REG")).unwrap_or(0),
                            ins.get(&format!("SRC{n}")).unwrap_or(0)
                        )
                    })
                    .collect();
                println!("      [{i}] {:<8} dst={:?} {}", ins.op().name, d, s.join(" "));
            }
            Err(e) => println!("      [{i}] <{e:?}>"),
        }
    }
}

/// The fragment shaders in detail, register by register.
///
/// `color_uv.frag` is the one that matters: it is `gl_test_fs_src`, the
/// default pair, which eleven gl_test cases link. Ours builds its
/// `vec4(v_uv, 1.0, 1.0)` constructor in **register 0** and the reference
/// builds it in register 2 -- and register 0 in the fragment stage is where
/// the rasteriser leaves `gl_FragCoord`.
#[test]
fn the_fragment_shaders_register_by_register() {
    for name in ["color_uv.frag", "texture.frag", "texture2.frag"] {
        let src = std::fs::read_to_string(format!("tools/shaders/gl_test_es100/{name}")).unwrap();
        println!("\n=== {name} ===");
        println!("{}", src.trim());
        match (run(false, SHC_STAGE_FRAGMENT, &src), run(true, SHC_STAGE_FRAGMENT, &src)) {
            (Ok(o), Ok(r)) => {
                dump("OURS", &o);
                dump("REF ", &r);
            }
            (a, b) => println!("  ours_ok={} ref_ok={}", a.is_ok(), b.is_ok()),
        }
    }
}

/// The wedge pair's temps: ours beside the reference's, as numbers.
///
/// `hsl_to_rgb_vertex` is the one dEQP shader that hangs the GC7000 on its
/// own (task #52g). Two mechanisms are on file for why, and both of them are
/// about the temp count: the driver puts it on the wire through a 6-bit field
/// (`x & 0x3f`, so 67 arrives as 3), and the load-balancing band it lands in
/// was never measured above 21. Either way the number itself is ours, and the
/// question the lead asked is how far it is from what the same source needs.
///
/// So this prints ours and the reference's side by side for all four shaders
/// of the pair -- the two that wedge and the two fragment twins that do not.
/// It asserts nothing: it is a measurement, and the day the allocator
/// improves the numbers must be free to move.
///
/// The sources are cts-runner's byte-exact copies of what dEQP hands us; a
/// paraphrase already cost one wrong refutation of a hypothesis (153/t41
/// against the real 220/t67), so nothing here is retyped.
#[test]
fn the_wedge_pair_temps_ours_against_the_reference() {
    println!("\n=== the wedge pair: temps, ours against the reference ===");
    println!("  {:<28} {:>5} {:>7} {:>5} {:>7}", "shader", "ours", "instrs", "ref", "instrs");
    for (file, stage) in [
        ("hsl_to_rgb_vertex.vert", SHC_STAGE_VERTEX),
        ("rgb_to_hsl_vertex.vert", SHC_STAGE_VERTEX),
        ("hsl_to_rgb_fragment.frag", SHC_STAGE_FRAGMENT),
        ("rgb_to_hsl_fragment.frag", SHC_STAGE_FRAGMENT),
    ] {
        let path = format!("tools/cts/testdata/wedge-shaders/{file}");
        let Ok(src) = std::fs::read_to_string(&path) else {
            println!("  {file:<28} (missing: {path})");
            continue;
        };
        let one = |sh: &Result<ShcShader, String>| match sh {
            Ok(s) => (i64::from(s.temps), (s.usc_words().len() / 4) as i64),
            Err(_) => (-1, -1),
        };
        let o = run(false, stage, &src);
        let r = run(true, stage, &src);
        let (ot, oi) = one(&o);
        let (rt, ri) = one(&r);
        println!("  {file:<28} {ot:>5} {oi:>7} {rt:>5} {ri:>7}");
        /* Where the instructions went. A `mov` here is not the source's: it
         * is if-conversion's save / restore / select per slot, and it is also
         * where the temps come from -- each saved and each then-value is a
         * fresh register whose live range spans the arm. */
        for (who, sh) in [("ours", &o), ("ref ", &r)] {
            let Ok(sh) = sh else { continue };
            let all = ops(sh);
            let movs = all.iter().filter(|n| *n == "mov").count();
            let sel = all.iter().filter(|n| n.starts_with("select")).count();
            let br = all.iter().filter(|n| n.starts_with("branch")).count();
            println!(
                "      {who}: {} instrs = {movs} mov + {sel} select + {br} branch + {} other",
                all.len(),
                all.len() - movs - sel - br
            );
        }
        if let Err(e) = &o {
            println!("      ours refused: {}", e.trim());
        }
        if let Err(e) = &r {
            println!("      ref refused:  {}", e.trim());
        }
    }
}
