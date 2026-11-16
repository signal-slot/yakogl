// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A program with more than 255 active uniforms links, answers a location
//! for each, and hands each value to the draw where its shader reads it
//! (#7106).
//!
//! GL ES limits uniforms by storage, not by name, and an array of structs is
//! one active uniform per element member. Qt Quick 3D declares its light
//! data that way (a plain struct uniform holding `LightData lights[16]`) and
//! its PrincipledMaterial has 453 active uniforms; the shader record's
//! count was a byte and the GL program's location table 255 entries, so the
//! compile was refused (`TooManyUniforms`). Here a fragment shader of the
//! same form has 16 lights of 18 members, 288 locations past the index byte
//! where the old locations stopped, plus the light count.
//!
//! What is checked, through the GL API on the host: the link succeeds and
//! GL_ACTIVE_UNIFORMS counts every element member; every
//! `u_lights.lights[i].m` has its own location; after a `glUniform*` of a
//! distinct value to each, the draw's fragment block holds that value at the
//! dword the compiled shader's row for that name gives.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_many_uniforms
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// A drawn fragment stage: its rows (name, dword offset) and its block.
type Drawn = (Vec<(String, u16)>, Vec<u32>);

/// The last draw's fragment stage.
static DRAWN: Mutex<Option<Drawn>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let Some(fs) = d.fs.as_ref() else { return 0 };
    let rows = fs
        .uniform_table()
        .iter()
        .map(|u| {
            let n: Vec<u8> = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            (String::from_utf8_lossy(&n).into_owned(), u.dword_off)
        })
        .collect();
    let block = core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, usize::from(d.fs_uniform_dwords)).to_vec();
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some((rows, block));
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"many-uniforms".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

const LIGHTS: usize = 16;
/// The struct's members: six vec4 and twelve floats.
const VEC4S: [&str; 6] = ["position", "direction", "up", "right", "diffuse", "specular"];
const FLOATS: usize = 12;

fn members() -> Vec<(String, usize)> {
    let mut m: Vec<(String, usize)> = VEC4S.iter().map(|n| (n.to_string(), 4)).collect();
    m.extend((0..FLOATS).map(|k| (format!("c{k}"), 1)));
    m
}

fn fragment_source() -> String {
    let decl: String = members()
        .iter()
        .map(|(n, w)| if *w == 4 { format!("    vec4 {n};\n") } else { format!("    float {n};\n") })
        .collect();
    let uses: String = members()
        .iter()
        .map(|(n, w)| {
            if *w == 4 {
                format!("        acc += u_lights.lights[i].{n};\n")
            } else {
                format!("        acc.x += u_lights.lights[i].{n};\n")
            }
        })
        .collect();
    format!(
        "#version 300 es\nprecision highp float;\n\
         struct LightData {{\n{decl}}};\n\
         struct cbLights {{\n    LightData lights[{LIGHTS}];\n}};\n\
         uniform cbLights u_lights;\nuniform int u_count;\n\
         layout(location = 0) out vec4 o;\n\
         void main() {{\n    vec4 acc = vec4(0.0);\n    for (int i = 0; i < u_count; i++) {{\n{uses}    }}\n    o = acc;\n}}\n"
    )
}

const VERTEX: &str = "#version 300 es\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); gl_PointSize = 1.0; }\n";

unsafe fn shader(kind: GLenum, src: &str) -> GLuint {
    let s = glCreateShader(kind);
    let c = CString::new(src).unwrap();
    let p = c.as_ptr();
    glShaderSource(s, 1, &p, core::ptr::null());
    glCompileShader(s);
    let mut ok = 0;
    glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
    if ok == 0 {
        let mut log = vec![0u8; 1024];
        let mut n = 0;
        glGetShaderInfoLog(s, 1024, &mut n, log.as_mut_ptr() as *mut c_char);
        panic!("compile: {}", String::from_utf8_lossy(&log[..n as usize]));
    }
    s
}

/// The value the test stores in lane `k` of light `i`'s member `m`.
fn value(i: usize, m: usize, k: usize) -> f32 {
    1000.0 * i as f32 + 10.0 * m as f32 + k as f32 + 0.5
}

#[test]
fn a_program_of_more_than_255_uniforms_links_and_every_value_reaches_its_row() {
    // SAFETY: both tables are statics; one test in this file.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());

        let prog = glCreateProgram();
        glAttachShader(prog, shader(GL_VERTEX_SHADER, VERTEX));
        glAttachShader(prog, shader(GL_FRAGMENT_SHADER, &fragment_source()));
        glLinkProgram(prog);
        let mut ok = 0;
        glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut log = vec![0u8; 1024];
            let mut n = 0;
            glGetProgramInfoLog(prog, 1024, &mut n, log.as_mut_ptr() as *mut c_char);
            panic!("link: {}", String::from_utf8_lossy(&log[..n as usize]));
        }
        let ms = members();
        let mut active = 0;
        glGetProgramiv(prog, GL_ACTIVE_UNIFORMS, &mut active);
        assert_eq!(active as usize, LIGHTS * ms.len() + 1, "every element member and the count");

        glUseProgram(prog);
        let loc_of = |name: &str| {
            let c = CString::new(name).unwrap();
            glGetUniformLocation(prog, c.as_ptr())
        };
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..LIGHTS {
            for (m, (n, w)) in ms.iter().enumerate() {
                let name = format!("u_lights.lights[{i}].{n}");
                let l = loc_of(&name);
                assert!(l >= 0, "{name}: no location");
                assert!(seen.insert(l), "{name}: location {l} given twice");
                let v: Vec<f32> = (0..*w).map(|k| value(i, m, k)).collect();
                if *w == 4 {
                    glUniform4fv(l, 1, v.as_ptr());
                } else {
                    glUniform1f(l, v[0]);
                }
                assert_eq!(glGetError(), GL_NO_ERROR, "{name}");
            }
        }
        glUniform1i(loc_of("u_count"), LIGHTS as i32);
        glDrawArrays(GL_POINTS, 0, 1);
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");

        let (rows, block) = DRAWN.lock().unwrap().take().expect("a draw reached the back end");
        assert!(rows.len() > 255, "{} rows in the record", rows.len());
        for i in 0..LIGHTS {
            for (m, (n, w)) in ms.iter().enumerate() {
                let name = format!("u_lights.lights[{i}].{n}");
                let off = rows.iter().find(|(r, _)| *r == name).unwrap_or_else(|| panic!("{name}: no row")).1;
                for k in 0..*w {
                    let got = f32::from_bits(block[usize::from(off) + k]);
                    assert_eq!(got, value(i, m, k), "{name} lane {k} at dword {}", usize::from(off) + k);
                }
            }
        }
    }
}
