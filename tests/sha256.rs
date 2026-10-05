// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! SHA-256 known-answer tests, and the offline shader table's invariant: every
//! source text hashes to a blob row of the same name (what glsl2usc hashed
//! is what gles_offline_shader_source() hands the application).
//!
//!   cargo test --no-default-features --features host-test,gpu-powervr

//! Both tables have the same shape, but this one names the PowerVR table:
//! a `gpu-vivante` build compiles `vivante_blobs.rs` instead.
#![cfg(feature = "gpu-powervr")]

use std::ffi::{c_char, CStr};

use yakogl::shaderc::mesa_reference::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::offline::{gles_offline_shader_source, shc_offline_register, SHC_OFFLINE};
use yakogl::shaderc::sha256::{sha256, Sha256};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn known_answers() {
    assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(
        hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        hex(&sha256(&vec![b'a'; 1_000_000])),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn incremental_matches_one_shot() {
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 + 3) as u8).collect();
    for split in [0, 1, 55, 56, 63, 64, 65, 119, 120, 128, 999, 1000] {
        let mut c = Sha256::new();
        c.update(&data[..split]);
        c.update(&data[split..]);
        assert_eq!(c.finalize(), sha256(&data), "split at {split}");
    }
    /* every padding case: lengths around the 56- and 64-byte boundaries */
    for len in 0..200 {
        let mut c = Sha256::new();
        for b in &data[..len] {
            c.update(core::slice::from_ref(b));
        }
        assert_eq!(c.finalize(), sha256(&data[..len]), "len {len}");
    }
}

#[test]
fn every_source_text_is_in_the_blob_table() {
    assert!(!SHC_SOURCE_TABLE.is_empty());
    for s in SHC_SOURCE_TABLE.iter() {
        let text = s.text.strip_suffix('\0').unwrap_or_else(|| panic!("{}: text has no trailing NUL", s.name));
        assert!(!text.contains('\0'), "{}: NUL inside the text", s.name);
        let digest = sha256(text.as_bytes());
        assert!(
            SHC_BLOB_TABLE.iter().any(|b| b.source_name() == s.name && b.sha256 == digest),
            "{}: no blob row with sha256={}",
            s.name,
            hex(&digest)
        );
    }
}

#[test]
fn offline_backend_finds_every_source() {
    let compile = SHC_OFFLINE.compile.unwrap();
    let mut log = [0 as c_char; 256];

    // SAFETY: plain C-ABI calls with valid pointers.
    unsafe {
        assert!(core::ptr::eq(shc_offline_register(), &SHC_OFFLINE));
        assert!(gles_offline_shader_source(core::ptr::null()).is_null());
        assert!(gles_offline_shader_source(c"no such shader".as_ptr()).is_null());

        for s in SHC_SOURCE_TABLE.iter() {
            let name = std::ffi::CString::new(s.name).unwrap();
            let text = gles_offline_shader_source(name.as_ptr());
            assert!(!text.is_null(), "{}", s.name);
            /* first match wins, like the C strcmp loop */
            let first = SHC_SOURCE_TABLE.iter().find(|o| o.name == s.name).unwrap();
            assert_eq!(text, first.text.as_ptr() as *const c_char);

            let digest = sha256(CStr::from_ptr(text).to_bytes());
            let row = SHC_BLOB_TABLE.iter().find(|b| b.blend_key == 0 && b.sha256 == digest).unwrap();
            let mut out = ShcShader::ZERO;
            assert_eq!(compile(row.stage, text, &mut out, log.as_mut_ptr(), log.len()), 0, "{}", s.name);
            assert_eq!(log[0], 0);
            assert_eq!(out.usc, row.shader.usc);
            assert!(!out.priv_.is_null());
        }
    }
}

#[test]
fn offline_backend_logs() {
    let compile = SHC_OFFLINE.compile.unwrap();
    let link = SHC_OFFLINE.link.unwrap();
    let blend_variant = SHC_OFFLINE.blend_variant.unwrap();
    let mut log = [0x55 as c_char; 256];
    let mut out = ShcShader::ZERO;
    let text = |log: &[c_char]| unsafe { CStr::from_ptr(log.as_ptr()) }.to_str().unwrap().to_owned();

    // SAFETY: plain C-ABI calls with valid (or deliberately null) pointers.
    unsafe {
        assert_eq!(compile(SHC_STAGE_VERTEX, core::ptr::null(), &mut out, log.as_mut_ptr(), log.len()), -22);
        assert_eq!(text(&log), "offline shaderc: NULL source or output");

        assert_eq!(compile(SHC_STAGE_FRAGMENT, c"abc".as_ptr(), &mut out, log.as_mut_ptr(), log.len()), -2);
        assert_eq!(
            text(&log),
            "offline shaderc: fragment shader not in offline table \
             (sha256=ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad); \
             add it to the glsl2usc inputs and regenerate blobs.rs (yakogl-tools glsl2usc)"
        );
        /* truncation keeps the NUL inside the buffer; a null log is fine */
        assert_eq!(compile(SHC_STAGE_VERTEX, c"abc".as_ptr(), &mut out, log.as_mut_ptr(), 20), -2);
        assert_eq!(text(&log), "offline shaderc: ve");
        assert_eq!(compile(SHC_STAGE_VERTEX, c"abc".as_ptr(), &mut out, core::ptr::null_mut(), 0), -2);

        assert_eq!(link(core::ptr::null(), &mut out, log.as_mut_ptr(), log.len()), -22);
        assert_eq!(text(&log), "offline shaderc: NULL VS/FS in link");

        let mut vs = ShcShader::ZERO;
        let mut fs = ShcShader::ZERO;
        fs.num_varyings = 1;
        fs.varyings[0].name[0] = b'v' as c_char;
        fs.varyings[0].components = 2;
        fs.varyings[0].location = 3;
        assert_eq!(link(&vs, &mut fs, log.as_mut_ptr(), log.len()), -22);
        assert_eq!(text(&log), "offline shaderc: fragment varying 'v' is not written by the vertex shader");
        vs.num_varyings = 1;
        vs.varyings[0].name[0] = b'v' as c_char;
        vs.varyings[0].components = 1;
        assert_eq!(link(&vs, &mut fs, log.as_mut_ptr(), log.len()), -22);
        assert_eq!(text(&log), "offline shaderc: fragment varying 'v' is narrower in the vertex shader");
        vs.varyings[0].components = 2;
        vs.varyings[0].vtxout_off = 2;
        vs.vs_output_size = 3;
        assert_eq!(link(&vs, &mut fs, log.as_mut_ptr(), log.len()), -22);
        assert_eq!(text(&log), "offline shaderc: VS emits no gl_Position (vs_output_size=3)");
        vs.vs_output_size = 5;
        assert_eq!(link(&vs, &mut fs, log.as_mut_ptr(), log.len()), -22);
        assert_eq!(text(&log), "offline shaderc: FS varying loc3 overlaps gl_Position (vtxout_off=2)");
        vs.varyings[0].vtxout_off = 4;
        assert_eq!(link(&vs, &mut fs, log.as_mut_ptr(), log.len()), -22);
        assert_eq!(text(&log), "offline shaderc: FS varying loc3 reads VS output dwords up to 6 but VS emits only 5");
        vs.vs_output_size = 6;
        assert_eq!(link(&vs, &mut fs, log.as_mut_ptr(), log.len()), 0);
        assert_eq!(text(&log), "");
        assert_eq!(fs.varyings[0].vtxout_off, 4);

        /* blend variants: keyed by the base row's digest */
        assert_eq!(blend_variant(&fs, 1, &mut out, core::ptr::null_mut(), 0), -22); /* no priv */
        let var = SHC_BLOB_TABLE.iter().find(|b| b.blend_key != 0).unwrap();
        let base = SHC_BLOB_TABLE.iter().find(|b| b.blend_key == 0 && b.sha256 == var.sha256).unwrap();
        let src = SHC_SOURCE_TABLE.iter().find(|s| s.name == base.source_name()).unwrap();
        let mut f = ShcShader::ZERO;
        assert_eq!(compile(SHC_STAGE_FRAGMENT, src.text.as_ptr() as *const c_char, &mut f, log.as_mut_ptr(), 256), 0);
        assert_eq!(blend_variant(&f, var.blend_key, &mut out, core::ptr::null_mut(), 0), 0);
        assert_eq!(out.usc, var.shader.usc);
        assert_eq!(out.blend_key, var.blend_key);
        assert_eq!(blend_variant(&f, 0, &mut out, core::ptr::null_mut(), 0), 0);
        assert_eq!(out.usc, f.usc);
        assert_eq!(blend_variant(&f, 0x3fff_ffff, &mut out, core::ptr::null_mut(), 0), -2);
    }
}
