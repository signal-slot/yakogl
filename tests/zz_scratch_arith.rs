#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
#[test]
fn scratch() {
    for n in 1..=4 {
        for (st, ext) in [(SHC_STAGE_VERTEX, "vert"), (SHC_STAGE_FRAGMENT, "frag")] {
            let src = std::fs::read_to_string(format!("tools/mali/testdata/gm2_arithconst_{n}.{ext}")).unwrap();
            let mut out = ShcShader::ZERO;
            let mut log = [0 as c_char; 1024];
            let z = format!("{src}\0");
            let f = yakogl::shaderc::online::SHC_ONLINE.compile.unwrap();
            let rc = unsafe { f(st, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
            let t: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
            println!("VERDICT {n} {ext} rc {rc} {t}");
        }
    }
}
