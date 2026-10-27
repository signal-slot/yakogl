// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The board layer's Rust side (driver/zephyr/lib/board/,
//! driver/docs/one-method.md): the result record, defined once
//! ([`record`]), the Raspberry Pi 4's framebuffer decisions ([`rpi4`]), and
//! the i.MX8MP samples' hold before their reset ([`imx8mp`]), and when the
//! NanoPC-T6 glue still feeds its watchdog ([`nanopc_t6`]) and what its
//! image prints before Zephyr's console exists ([`nanopc_t6_early`]) and
//! its GPU power-on ([`nanopc_t6_gpu`]).
//! The C side reaches both through `ffi::board`.

pub mod imx8mp;
pub mod nanopc_t6;
#[cfg(any(feature = "rk3588", test))]
pub mod nanopc_t6_early;
#[cfg(any(all(feature = "rk3588", feature = "mali"), test))]
pub mod nanopc_t6_gpu;
pub mod record;
pub mod rpi4;

#[cfg(test)]
mod tests {
    use super::record::{self, *};

    const F: u32 = GM_SEL_F << GM_SEL_T_SHIFT;
    const P0: u32 = GM_SEL_P0 << GM_SEL_T_SHIFT;
    const P1: u32 = GM_SEL_P1 << GM_SEL_T_SHIFT;

    fn gm_record(tag: u32, s: &[GmScene]) -> [u32; 4] {
        let w = gm_page_words(tag, s).unwrap();
        [w0(kind::GM_PAGE, tag as u8), w[0], w[1], w[2]]
    }

    /// `driver/zephyr/lib/board/include/yakogl_record.h` is
    /// `record::write_c_header`'s output, byte for byte. With
    /// YK_RECORD_BLESS=1 the test writes the file instead.
    #[test]
    fn c_header_is_generated_from_the_table() {
        let mut want = String::new();
        record::write_c_header(&mut want).unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../zephyr/lib/board/include/yakogl_record.h");
        if std::env::var_os("YK_RECORD_BLESS").is_some() {
            std::fs::write(path, &want).unwrap();
        }
        let have = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            have == want,
            "{path} is not record.rs's table: run YK_RECORD_BLESS=1 cargo test -p yakogl-drivers board"
        );
    }

    /// The scenes of the synthetic page session in driver/rust/tests/gm-record.
    fn fixture_scene(i: usize) -> GmScene {
        let fps = 20 + 7 * i as u32;
        let s = GmScene { fps, frames: fps * 10, ..GmScene::default() };
        let pic = |probe: [u32; 5], crc16, crc16q, mean| GmScene { probe, crc16, crc16q, mean, w: 800, h: 600, ..s };
        match i {
            0 => pic([0xff80_40ff, 0x1020_30ff, 0x2030_40ff, 0x3040_50ff, 0x4050_60ff], 0x3a1c, 0x7f02, [128, 64, 32]),
            16 => GmScene { fps: GM_FPS_UNSUPPORTED, ..s },
            21 => GmScene { w: 0, h: 0, ..s }, // the readback failed
            22 => GmScene {
                fps: 6,
                frames: 61,
                ..pic([0x6080_a0ff, 0x80a0_c0ff, 0x3050_70ff, 0x1828_38ff, 0xc0c0_c0ff], 0x51e4, 0x0c3b, [92, 118, 140])
            },
            23 => GmScene { fps: GM_FPS_FAILED, ..s },
            25 => pic([0x0000_ffff, 0x0000_00ff, 0x0000_00ff, 0x0000_00ff, 0x0000_00ff], 0x9d20, 0x44c1, [0, 0, 51]),
            32 => GmScene { fps: 70_000, frames: 700_000, ..s },
            _ => s,
        }
    }

    /// The fixture's runs, as board.sh imx8mp gm names them: F pages but 9,
    /// both P pages of scenes 0, 21 and 22, P1 alone of scene 25, the cluster rate.
    fn fixture_runs() -> Vec<(String, [u32; 4])> {
        let mut runs = Vec::new();
        for page in (0..GM_F_PAGES).filter(|&p| p != 9) {
            let s: Vec<GmScene> = (3 * page..3 * page + 3).map(|i| fixture_scene(i as usize)).collect();
            runs.push((format!("gm-f{page:02}"), gm_record(F | page, &s)));
        }
        for (scene, pages) in [(0u32, "ab"), (21, "ab"), (22, "ab"), (25, "b")] {
            for p in pages.chars() {
                let t = if p == 'a' { P0 } else { P1 };
                runs.push((format!("gm-p{scene:02}{p}"), gm_record(t | scene, &[fixture_scene(scene as usize)])));
            }
        }
        let w = rate_words(3600, 60_012, 1280, 800);
        runs.push(("cl-rate".into(), [w0(kind::RATE, STATUS_PASS as u8), w[0], w[1], w[2]]));
        runs
    }

    /// driver/rust/tests/gm-record: the runs' snvs.txt are these encoders'
    /// records (the form board.sh's run writes), and write_gm_lines decodes
    /// them into glmark2-pages.txt, the console lines `board.sh imx8mp
    /// gm-lines` prints. YK_RECORD_BLESS=1 rewrites both.
    #[test]
    fn gm_record_fixture_decodes_to_the_console_lines() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/gm-record");
        let bless = std::env::var_os("YK_RECORD_BLESS").is_some();
        let runs = fixture_runs();
        let mut recs = Vec::new();
        for (name, rec) in &runs {
            let path = format!("{dir}/runs/20261017-1200-{name}-00000000/snvs.txt");
            let want = format!("{:08x} {:08x} {:08x} {:08x}\n", rec[0], rec[1], rec[2], rec[3]);
            if bless {
                std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
                std::fs::write(&path, &want).unwrap();
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), want, "{path}");
            recs.push(*rec);
        }
        let mut lines = String::new();
        write_gm_lines(&mut lines, &recs).unwrap();
        // the order of the records does not matter
        let mut rev = String::new();
        recs.reverse();
        write_gm_lines(&mut rev, &recs).unwrap();
        assert_eq!(lines, rev);
        let path = format!("{dir}/glmark2-pages.txt");
        if bless {
            std::fs::write(&path, &lines).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), lines, "{path}: YK_RECORD_BLESS=1 rewrites it");
        // what the readers key on (tools/cts/common-tally.py)
        assert!(lines.starts_with(
            "glmark2-pages: decoded from kind GM_PAGE records: F pages 0-8, 10 (missing 9); \
             P0 pages of scenes 0, 21-22 (missing 1-20, 23-32); P1 pages of scenes 0, 21-22, 25 (missing 1-20, 23-24, 26-32)\n"
        ));
        for want in [
            "[build] use-vbo=false: FPS: 20 FrameTime: 50.000 ms\n\
             glmark2-frame: [build] use-vbo=false: 200 frames; us per frame: not carried (GM_PAGE)\n\
             glmark2-picture: [build] use-vbo=false: 800x600; probes c f8804000, tl 10203000, tr 20304000, \
             bl 30405000, br 40506000; crc16 3a1c, crc16q 7f02; mean 128 64 32; black no\n",
            "[desktop] effect=shadow:windows=4: Unsupported\n[buffer]",
            "glmark2-picture: [jellyfish] <default>: absent (the readback failed)\n",
            "[terrain] <default>: FPS: 6 FrameTime: 166.667 ms\n",
            "[shadow] <default>: Set up failed\n[refract]",
            "glmark2-picture: [conditionals] fragment-steps=0:vertex-steps=0: absent (page P0 of scene 25 missing)\n",
            "FPS: 65533 FrameTime: 0.015 ms\n",
            "throughput: 1280x800: 3600 frames in 60012 ms, 60.0 fps, 61.4 Mpx/s (kind RATE, status pass)\n",
        ] {
            assert!(lines.contains(want), "{want}\nnot in\n{lines}");
        }
        // scenes 27..29 have no page: no line names them
        assert!(!lines.contains("fragment-complexity=low"), "{lines}");
    }
}
