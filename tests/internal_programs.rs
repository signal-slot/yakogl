// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The back ends' own programs, our compiler's, against Mesa's.
//!
//! A product image binds the programs `yakogl-precompile --internal` made
//! with our compiler (src/pvr/internal.rs, src/vivante/internal.rs); Mesa's
//! programs for the same jobs are reference material (tests/reference/
//! pvr_internal.rs, gc_clear_shader.rs). The bytes may differ. What must not
//! is what they do: here each of ours runs in the byte model
//! (tests/usc_exec) on the inputs the backend gives it -- the target's
//! descriptor at sh0, the uniforms after it, gl_FragCoord's pair where its
//! record says, the pixel in the output registers -- beside Mesa's on the
//! same pixel, and every output register must be the same. The GC7000's
//! two are Mesa's to the word.
//!
//! The metadata the backend binds them with -- temporaries, outputs, the
//! uniform block, the pixel read -- is pinned for ours and printed beside
//! Mesa's (`--nocapture`), so a compiler change that moves a program shows
//! here as well as in the committed file.

#![cfg(all(mesa_reference, feature = "shaderc-online"))]

#[cfg(feature = "gpu-powervr")]
mod usc_exec;

#[cfg(feature = "gpu-powervr")]
mod powervr {
    use super::usc_exec::{run_binary, Inputs};
    use std::collections::BTreeMap;
    use yakogl::pvr::internal::*;
    use yakogl::pvr::mesa_internal::*;
    use yakogl::shaderc::ShcShader;

    fn row(i: usize) -> &'static ShcShader {
        &PVR_INTERNAL_TABLE[i].shader
    }

    fn words(sh: &ShcShader) -> &'static [u32] {
        // SAFETY: a row of the generated table points at its `[u32; usc_dw]` static.
        unsafe { std::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }
    }

    fn bytes(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|d| d.to_le_bytes()).collect()
    }

    /// Ours and Mesa's on one input; every output register the same.
    fn same(what: &str, ours: &[u32], mesa: &[u32], inp: &Inputs, ours_inp: &Inputs) -> BTreeMap<u32, u32> {
        let theirs = run_binary(&bytes(mesa), inp, false).unwrap_or_else(|e| panic!("{what}: Mesa's runs: {e}"));
        let got = run_binary(&bytes(ours), ours_inp, true).unwrap_or_else(|e| panic!("{what}: ours runs: {e}"));
        assert!(!theirs.is_empty(), "{what}: Mesa's writes nothing, which proves nothing");
        assert_eq!(got, theirs, "{what}: ours writes other outputs than Mesa's");
        got
    }

    /// The frame load, at every class of target: on a target `h` rows high,
    /// flipped and not, single- and multisampled, at pixels across it, ours
    /// reads the texel Mesa's reads and packs it as Mesa's does. Mesa's
    /// program reads gl_FragCoord's registers as they are; ours gets y
    /// mirrored through the pair the backend writes per draw and undoes it
    /// with `u_flip`, the same pair (fbload.frag), so a flipped target is
    /// where the two could part.
    #[test]
    fn the_frame_load_reads_the_texel_mesas_reads() {
        let progs: [(&str, usize, &[u32]); 4] = [
            ("rgba8", PVR_INTERNAL_FBLOAD, &PVR_USC_FBLOAD),
            ("rgba16f", PVR_INTERNAL_FBLOAD_F16X4, &PVR_USC_FBLOAD_F16X4),
            ("r16f", PVR_INTERNAL_FBLOAD_F16, &PVR_USC_FBLOAD_F16),
            ("r32f", PVR_INTERNAL_FBLOAD_F32, &PVR_USC_FBLOAD_F32),
        ];
        let h = 48u32;
        let mut runs = 0;
        for (class, i, mesa) in progs {
            let sh = row(i);
            /* the record says where the backend puts what it reads */
            assert_eq!(sh.uniform_dwords, 2, "{class}: u_flip, the block");
            let ytf = sh.fragcoord_ytf_off1 as usize;
            assert_eq!(ytf, 20 + 2 + 1, "{class}: gl_FragCoord's pair straight after the block");
            for flip in [false, true] {
                let pair = if flip { [-1.0f32, h as f32] } else { [1.0, 0.0] };
                for samples in [1u32, 4] {
                    for (x, y) in [(0u32, 0u32), (5, 7), (47, 0), (0, 47), (31, 16), (47, 47)] {
                        let mut inp = Inputs::new(x * 64 + y + samples);
                        inp.nearest = true;
                        inp.samples = samples;
                        /* the register's own position: the pixel's centre, or a sample in it */
                        let (fx, fy) = if samples == 1 { (0.5, 0.5) } else { (0.375, 0.625) };
                        let xp = (x as f32 + 0.5).to_bits();
                        let yp = (y as f32 + 0.5).to_bits();
                        let xs = (x as f32 + fx).to_bits();
                        let ys = (y as f32 + fy).to_bits();
                        inp.frag_xy = [xp, xs, yp, ys];
                        let mut ours = inp.clone();
                        ours.shared[20] = pair[0].to_bits();
                        ours.shared[21] = pair[1].to_bits();
                        ours.shared[ytf - 1] = pair[0].to_bits();
                        ours.shared[ytf] = pair[1].to_bits();
                        let what = format!("{class} flip={flip} samples={samples} pixel ({x}, {y})");
                        same(&what, words(sh), mesa, &inp, &ours);
                        runs += 1;
                    }
                }
            }
        }
        assert_eq!(runs, 4 * 2 * 2 * 6);
    }

    /// The masked clears: all fourteen partial masks, RGBA8 and RGBA16F,
    /// each over several pixels and colours, write what Mesa's write --
    /// the written channels the colour (the four uniform dwords), the
    /// others the pixel.
    #[test]
    fn the_masked_clears_write_what_mesas_write() {
        let mesa: [&[u32]; 28] = [
            &PVR_USC_CLEAR_M1,
            &PVR_USC_CLEAR_M2,
            &PVR_USC_CLEAR_M3,
            &PVR_USC_CLEAR_M4,
            &PVR_USC_CLEAR_M5,
            &PVR_USC_CLEAR_M6,
            &PVR_USC_CLEAR_M7,
            &PVR_USC_CLEAR_M8,
            &PVR_USC_CLEAR_M9,
            &PVR_USC_CLEAR_MA,
            &PVR_USC_CLEAR_MB,
            &PVR_USC_CLEAR_MC,
            &PVR_USC_CLEAR_MD,
            &PVR_USC_CLEAR_ME,
            &PVR_USC_CLEAR_M1_F16X4,
            &PVR_USC_CLEAR_M2_F16X4,
            &PVR_USC_CLEAR_M3_F16X4,
            &PVR_USC_CLEAR_M4_F16X4,
            &PVR_USC_CLEAR_M5_F16X4,
            &PVR_USC_CLEAR_M6_F16X4,
            &PVR_USC_CLEAR_M7_F16X4,
            &PVR_USC_CLEAR_M8_F16X4,
            &PVR_USC_CLEAR_M9_F16X4,
            &PVR_USC_CLEAR_MA_F16X4,
            &PVR_USC_CLEAR_MB_F16X4,
            &PVR_USC_CLEAR_MC_F16X4,
            &PVR_USC_CLEAR_MD_F16X4,
            &PVR_USC_CLEAR_ME_F16X4,
        ];
        let colours = [[0.25f32, 0.5, 0.75, 1.0], [0.0, 1.0, 0.003, 0.5], [1.0, 0.0, 0.0, 0.0], [0.6, 0.2, 0.9, 0.1]];
        for (k, theirs) in mesa.iter().enumerate() {
            let (class, m) = if k < 14 { (0, k as u8 + 1) } else { (1, k as u8 - 13) };
            let ours = yakogl::pvr::backend::clear_masked_program(class, m).expect("a partial mask's program");
            for (n, c) in colours.iter().enumerate() {
                let mut inp = Inputs::new(100 + n as u32);
                for (j, v) in c.iter().enumerate() {
                    inp.shared[j] = v.to_bits();
                }
                inp.pixout = 0x8040_2010u32.rotate_left(8 * n as u32);
                inp.pixout1 = 0x3c00_3800 ^ (n as u32) << 4;
                let what = format!("class {class} mask {m:#x} colour {c:?}");
                same(&what, words(ours), theirs, &inp, &inp);
            }
        }
    }

    /// The quad's vertex shader writes the position it reads, as Mesa's.
    #[test]
    fn the_quad_vertex_shader_writes_what_mesas_writes() {
        for seed in 0..8 {
            let inp = Inputs::new(seed);
            same(&format!("seed {seed}"), words(row(PVR_INTERNAL_QUAD_VS)), &PVR_USC_FBLOAD_VS, &inp, &inp);
        }
    }

    /// What the backend binds each program with, ours pinned, beside the
    /// figures the backend bound Mesa's with (its hand-built descriptors).
    #[test]
    fn the_internal_programs_metadata() {
        /* name, row, (temps, dwords, outputs, uniform dwords, fbfetch, ytf
         * pair + 1) of ours, and Mesa's temps and dwords */
        type Meta = (u32, u32, u32, u32, u32, u32);
        /* a program's row, its pinned figures, and Mesa's temps and dwords */
        type Entry = (usize, Meta, (u32, usize));
        let fb = |i: usize, want: Meta, mesa: (u32, usize)| (i, want, mesa);
        let mut rows: Vec<(&str, Entry)> = vec![
            (
                "quad_vs",
                fb(PVR_INTERNAL_QUAD_VS, (0, 6, 0, 0, 0, 0), (PVR_USC_FBLOAD_VS_TEMPS, PVR_USC_FBLOAD_VS_DWORDS)),
            ),
            ("fbload", fb(PVR_INTERNAL_FBLOAD, (26, 116, 1, 2, 0, 23), (PVR_USC_FBLOAD_TEMPS, PVR_USC_FBLOAD_DWORDS))),
            (
                "fbload_f16x4",
                fb(
                    PVR_INTERNAL_FBLOAD_F16X4,
                    (45, 138, 2, 2, 0, 23),
                    (PVR_USC_FBLOAD_F16X4_TEMPS, PVR_USC_FBLOAD_F16X4_DWORDS),
                ),
            ),
            (
                "fbload_f16",
                fb(
                    PVR_INTERNAL_FBLOAD_F16,
                    (44, 126, 1, 2, 0, 23),
                    (PVR_USC_FBLOAD_F16_TEMPS, PVR_USC_FBLOAD_F16_DWORDS),
                ),
            ),
            (
                "fbload_f32",
                fb(
                    PVR_INTERNAL_FBLOAD_F32,
                    (44, 124, 1, 2, 0, 23),
                    (PVR_USC_FBLOAD_F32_TEMPS, PVR_USC_FBLOAD_F32_DWORDS),
                ),
            ),
        ];
        let masked_mesa: [(u32, usize); 28] = [
            (PVR_USC_CLEAR_M1_TEMPS, PVR_USC_CLEAR_M1_DWORDS),
            (PVR_USC_CLEAR_M2_TEMPS, PVR_USC_CLEAR_M2_DWORDS),
            (PVR_USC_CLEAR_M3_TEMPS, PVR_USC_CLEAR_M3_DWORDS),
            (PVR_USC_CLEAR_M4_TEMPS, PVR_USC_CLEAR_M4_DWORDS),
            (PVR_USC_CLEAR_M5_TEMPS, PVR_USC_CLEAR_M5_DWORDS),
            (PVR_USC_CLEAR_M6_TEMPS, PVR_USC_CLEAR_M6_DWORDS),
            (PVR_USC_CLEAR_M7_TEMPS, PVR_USC_CLEAR_M7_DWORDS),
            (PVR_USC_CLEAR_M8_TEMPS, PVR_USC_CLEAR_M8_DWORDS),
            (PVR_USC_CLEAR_M9_TEMPS, PVR_USC_CLEAR_M9_DWORDS),
            (PVR_USC_CLEAR_MA_TEMPS, PVR_USC_CLEAR_MA_DWORDS),
            (PVR_USC_CLEAR_MB_TEMPS, PVR_USC_CLEAR_MB_DWORDS),
            (PVR_USC_CLEAR_MC_TEMPS, PVR_USC_CLEAR_MC_DWORDS),
            (PVR_USC_CLEAR_MD_TEMPS, PVR_USC_CLEAR_MD_DWORDS),
            (PVR_USC_CLEAR_ME_TEMPS, PVR_USC_CLEAR_ME_DWORDS),
            (PVR_USC_CLEAR_M1_F16X4_TEMPS, PVR_USC_CLEAR_M1_F16X4_DWORDS),
            (PVR_USC_CLEAR_M2_F16X4_TEMPS, PVR_USC_CLEAR_M2_F16X4_DWORDS),
            (PVR_USC_CLEAR_M3_F16X4_TEMPS, PVR_USC_CLEAR_M3_F16X4_DWORDS),
            (PVR_USC_CLEAR_M4_F16X4_TEMPS, PVR_USC_CLEAR_M4_F16X4_DWORDS),
            (PVR_USC_CLEAR_M5_F16X4_TEMPS, PVR_USC_CLEAR_M5_F16X4_DWORDS),
            (PVR_USC_CLEAR_M6_F16X4_TEMPS, PVR_USC_CLEAR_M6_F16X4_DWORDS),
            (PVR_USC_CLEAR_M7_F16X4_TEMPS, PVR_USC_CLEAR_M7_F16X4_DWORDS),
            (PVR_USC_CLEAR_M8_F16X4_TEMPS, PVR_USC_CLEAR_M8_F16X4_DWORDS),
            (PVR_USC_CLEAR_M9_F16X4_TEMPS, PVR_USC_CLEAR_M9_F16X4_DWORDS),
            (PVR_USC_CLEAR_MA_F16X4_TEMPS, PVR_USC_CLEAR_MA_F16X4_DWORDS),
            (PVR_USC_CLEAR_MB_F16X4_TEMPS, PVR_USC_CLEAR_MB_F16X4_DWORDS),
            (PVR_USC_CLEAR_MC_F16X4_TEMPS, PVR_USC_CLEAR_MC_F16X4_DWORDS),
            (PVR_USC_CLEAR_MD_F16X4_TEMPS, PVR_USC_CLEAR_MD_F16X4_DWORDS),
            (PVR_USC_CLEAR_ME_F16X4_TEMPS, PVR_USC_CLEAR_ME_F16X4_DWORDS),
        ];
        const NAMES: [&str; 28] = [
            "clear_m1",
            "clear_m2",
            "clear_m3",
            "clear_m4",
            "clear_m5",
            "clear_m6",
            "clear_m7",
            "clear_m8",
            "clear_m9",
            "clear_ma",
            "clear_mb",
            "clear_mc",
            "clear_md",
            "clear_me",
            "clear_m1_f16x4",
            "clear_m2_f16x4",
            "clear_m3_f16x4",
            "clear_m4_f16x4",
            "clear_m5_f16x4",
            "clear_m6_f16x4",
            "clear_m7_f16x4",
            "clear_m8_f16x4",
            "clear_m9_f16x4",
            "clear_ma_f16x4",
            "clear_mb_f16x4",
            "clear_mc_f16x4",
            "clear_md_f16x4",
            "clear_me_f16x4",
        ];
        for (k, name) in NAMES.iter().enumerate() {
            let (class, m) = if k < 14 { (0, k as u8 + 1) } else { (1, k as u8 - 13) };
            let sh = yakogl::pvr::backend::clear_masked_program(class, m).expect("a partial mask's program");
            let i = PVR_INTERNAL_TABLE.iter().position(|b| std::ptr::eq(&b.shader, sh)).expect("a row");
            let want = if class == 0 { (23, 38, 1, 4, 1, 0) } else { (24, 46, 2, 4, 1, 0) };
            rows.push((name, (i, want, masked_mesa[k])));
        }
        println!(
            "{:<16} {:>12} {:>12} {:>7} {:>8} {:>7} {:>6}",
            "program", "temps o/M", "dwords o/M", "outputs", "uniform", "fbfetch", "ytf"
        );
        let mut wrong = Vec::new();
        for (name, (i, want, (mtemps, mdw))) in &rows {
            let sh = row(*i);
            let got = (
                u32::from(sh.temps),
                sh.usc_dw,
                u32::from(sh.num_outputs),
                u32::from(sh.uniform_dwords),
                u32::from(sh.uses_fbfetch),
                u32::from(sh.fragcoord_ytf_off1),
            );
            println!(
                "{name:<16} {:>5} / {:<4} {:>5} / {:<4} {:>7} {:>8} {:>7} {:>6}",
                got.0, mtemps, got.1, mdw, got.2, got.3, got.4, got.5
            );
            if got != *want {
                wrong.push(format!("{name}: {got:?}, pinned {want:?}"));
            }
            /* the frame and clear programs need no ISP feedback word */
            if sh.num_attribs == 0 && sh.meta_off != yakogl::shaderc::SHC_NO_META {
                wrong.push(format!("{name}: meta_off {:#x}, not SHC_NO_META", sh.meta_off));
            }
        }
        assert!(
            wrong.is_empty(),
            "the internal programs moved (regenerate src/pvr/internal.rs and re-pin):\n{}",
            wrong.join("\n")
        );
    }

    /// The clear-attach programs are our encoder's and Mesa's at once.
    #[test]
    fn the_clear_attach_programs_are_mesas_words() {
        assert_eq!(PVR_INTERNAL_CLEAR_ATTACH1, PVR_USC_CLEAR_FS_WORDS);
        assert_eq!(PVR_INTERNAL_CLEAR_ATTACH2, PVR_USC_CLEAR_FS2_WORDS);
        assert_eq!(PVR_INTERNAL_CLEAR_ATTACH3, PVR_USC_CLEAR_FS3_WORDS);
    }
}

#[cfg(feature = "gpu-vivante")]
mod gc7000 {
    use yakogl::shaderc::{ShcShader, ShcViv, ShcVivIo, SHC_VIV_NO_REG};
    use yakogl::vivante::internal::*;
    use yakogl::vivante::mesa_clear_shader::{GC_CLEAR_PS, GC_CLEAR_VS};

    fn words(sh: &ShcShader) -> &'static [u32] {
        // SAFETY: both tables point at their own `[u32; usc_dw]` statics.
        unsafe { std::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }
    }

    fn name(n: &[core::ffi::c_char]) -> String {
        n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
    }

    fn io(l: &[ShcVivIo]) -> String {
        l.iter()
            .map(|e| format!("{}:{}:{}:{}:{}", e.reg, e.slot, e.components, e.interp, name(&e.name)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Everything of a record but its pointers, the register list its stage
    /// does not have, and the fields of the other stage.
    fn view(sh: &ShcShader, vertex: bool) -> String {
        let v: &ShcViv = &sh.viv;
        let attribs: Vec<String> = sh.attribs[..sh.num_attribs as usize]
            .iter()
            .map(|a| {
                format!("{}:{}:{}:{}:{}:{:#x}", a.location, a.components, a.vi_reg, a.format, name(&a.name), a.gl_type)
            })
            .collect();
        let uniforms: Vec<String> = sh
            .uniform_table()
            .iter()
            .map(|u| {
                format!(
                    "{}:{}:{}:{:#x}:{}:{}:{}",
                    name(&u.name),
                    u.dword_off,
                    u.dwords,
                    u.gl_type,
                    u.array_len,
                    u.binding,
                    u.inner
                )
            })
            .collect();
        let stage = if vertex {
            format!(
                "in {} pos {} psize {} id {} lb {:#x} unk8 {}",
                io(&v.in_[..v.num_in as usize]),
                v.vs_pos_out_reg,
                v.vs_pointsize_out_reg,
                v.vs_id_in_reg,
                v.vs_load_balancing,
                v.input_count_unk8
            )
        } else {
            format!(
                "in {} pairs {:?} colour {} depth {} unk8 {} consts {:?} {:?}",
                io(&v.in_[..v.num_in as usize]),
                &v.vs_reg_of_in[..v.num_in as usize],
                v.ps_color_out_reg,
                v.ps_depth_out_reg,
                v.input_count_unk8,
                v.const_contents(),
                v.const_data()
            )
        };
        format!(
            "temps {} attribs {attribs:?} uniforms {uniforms:?} dwords {} out {} {stage}",
            sh.temps,
            sh.uniform_dwords,
            io(&v.out[..v.num_out as usize]),
        )
    }

    /// Our compiler's clear pair is etna's: the same instruction words, and
    /// the same record in every field the backend reads for that stage. Two
    /// fields differ, both the other stage's: a vertex shader's
    /// `vs_reg_of_in` (which input pairs with which vertex output -- a
    /// fragment shader's) and `ps_color_out_reg` (a fragment shader's), which
    /// ours leaves at SHC_VIV_NO_REG and etna's table at 0.
    #[test]
    fn the_clear_shaders_are_etnas() {
        let (vs, ps) =
            (&GC_INTERNAL_TABLE[GC_INTERNAL_CLEAR_VS].shader, &GC_INTERNAL_TABLE[GC_INTERNAL_CLEAR_PS].shader);
        assert_eq!(words(vs), words(&GC_CLEAR_VS.0), "the vertex shader's words");
        assert_eq!(words(ps), words(&GC_CLEAR_PS.0), "the fragment shader's words");
        assert_eq!(view(vs, true), view(&GC_CLEAR_VS.0, true));
        assert_eq!(view(ps, false), view(&GC_CLEAR_PS.0, false));
        assert_eq!((vs.viv.vs_reg_of_in, vs.viv.ps_color_out_reg), ([SHC_VIV_NO_REG; 16], SHC_VIV_NO_REG));
        assert_eq!((GC_CLEAR_VS.0.viv.vs_reg_of_in, GC_CLEAR_VS.0.viv.ps_color_out_reg), ([0; 16], 0));
    }
}
