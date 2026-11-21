// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the PE, RA and early-depth words of a draw that
//! docs/notes/gc-tiled-wire.md W4/W5 move into a tiled shadow (knob
//! `gc_tiled_rt` on), compared with Mesa's words for the same draw in
//! `tools/vivante/testdata/tiled-ez.golden` (cs_golden mode 30, Mesa 26.2.1
//! etnaviv on the GC7000UL drm-shim row).
//!
//! The draw, on both sides: a 64x64 RGBA8 texture and a 64x64 D24S8
//! renderbuffer (depth and stencil attachment) in one framebuffer object, a
//! depth clear, then one GL_LEQUAL, depth-writing quad at z 0.5 with no
//! stencil test, no discard and no gl_FragDepth, then glFinish.
//!
//! What is compared is each register's value at the draw (the last write
//! before DRAW_INSTANCED): PE_COLOR_FORMAT, PE_COLOR_STRIDE, PE_LOGIC_OP,
//! PE_DEPTH_CONFIG, PE_DEPTH_STRIDE, RA_EARLY_DEPTH and PE_MEM_CONFIG. Every
//! bit that differs is named in [`EXPECTED`] with its reason, and the list is
//! exact both ways: a bit that differs and is not listed fails, and so does a
//! listed bit that no longer differs (the list is then stale).
//!
//! How it runs: the real GL core and Vivante back end over a stand-in OSAL
//! (the allocator and submit of tests/gc_tiled_rt.rs, without its model of
//! the GPU), the knob set through `gc_debug_set_tiled_rt`. The knob-off run
//! of the same draw is checked as well, so the test also says which of the
//! golden's words the knob brings us to.
//!
//! Built without `report-es2` (the library reports ES 3.0); nothing here
//! depends on the report.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_tiled_ez_golden
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::gc_debug_set_tiled_rt;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

const GOLDEN: &str = include_str!("../tools/vivante/testdata/tiled-ez.golden");

/// PE_MEM_CONFIG's tile-status modes (Mesa's state_3d.xml.h; regs.rs has no
/// name for them, nothing of ours writes them).
const PE_MEM_CONFIG_COLOR_TS_MODE: u32 = 0x0100_0000;
const PE_MEM_CONFIG_DEPTH_TS_MODE: u32 = 0x0400_0000;
/// PE_LOGIC_OP's DITHER_MODE field (Mesa's state_3d.xml.h).
const PE_LOGIC_OP_DITHER_MODE: u32 = 0x0000_0060;

/* ---- the stand-in OSAL: host memory and the streams --------------------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end, the knob and
/// `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: Vec<u32> =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) }.to_vec();
        p.kicks.push(words);
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/* ---- our stream ---------------------------------------------------------------------- */

/// Each register's value at the stream's first DRAW_INSTANCED (its last
/// write before it), or `None` when the stream has no draw.
fn ours_at_draw(words: &[u32]) -> Option<BTreeMap<u32, u32>> {
    let mut regs = BTreeMap::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(v) = words.get(i + 1 + k) {
                        regs.insert(base + 4 * k as u32, *v);
                    }
                }
                i += 1 + count;
                i += i % 2; /* the pad of an even-length run */
            }
            GC_OP_DRAW_INSTANCED => return Some(regs),
            GC_OP_STALL => i += 2,
            _ => i += 1, /* NOP */
        }
    }
    None
}

/* ---- Mesa's stream: the golden ------------------------------------------------------ */

/// Each register's value at the golden's first DRAW_INSTANCED, read from
/// `cs_decode.py`'s print: `LOAD_STATE <addr> <name> count=<n> <values>`, a
/// run's registers 4 bytes apart (a long run's print ends in `...`, and the
/// values it leaves out are left out here too).
fn golden_at_draw() -> BTreeMap<u32, u32> {
    let mut regs = BTreeMap::new();
    for l in GOLDEN.lines() {
        if l.starts_with("DRAW_INSTANCED") {
            return regs;
        }
        if !l.starts_with("LOAD_STATE") {
            continue;
        }
        let mut it = l.split_whitespace().skip(1);
        let reg = u32::from_str_radix(it.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        let vals = it.skip_while(|t| !t.starts_with("count=")).skip(1).take_while(|t| t.starts_with("0x"));
        for (k, t) in vals.enumerate() {
            regs.insert(reg + 4 * k as u32, u32::from_str_radix(&t[2..], 16).unwrap());
        }
    }
    panic!("the golden has no DRAW_INSTANCED");
}

/* ---- the rig ------------------------------------------------------------------------ */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// cs_golden mode 30's shaders, word for word.
const VS: &str = "attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.5, 0.25, 0.125, 1.0); }\n";

/// cs_golden mode 30's quad, a triangle strip at z 0.5.
static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// Draw cs_golden mode 30's frame on the stand-in OSAL with the knob as
/// given, and return the registers at the draw of the one kick that holds a
/// draw. The knob is put back to off.
fn mode30(knob: bool) -> BTreeMap<u32, u32> {
    with_platform(|p| {
        p.bump.clear();
        p.kicks.clear();
    });
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(|| {
        gc_debug_set_tiled_rt(knob as i32);
        // SAFETY: plain GL calls; null texel data; QUAD is a static.
        unsafe {
            let (mut fb, mut t, mut rb) = (0, 0, 0);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, 64, 64);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glViewport(0, 0, 64, 64);

            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            glBindAttribLocation(p, 0, c"a_pos".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed");
            glUseProgram(p);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 16, QUAD.as_ptr() as *const c_void);

            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glClear(GL_DEPTH_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            glFinish();
        }
        no_error("mode 30");
        let kicks = with_platform(|p| p.kicks.clone());
        let drawn: Vec<_> = kicks.iter().filter_map(|k| ours_at_draw(k)).collect();
        assert_eq!(drawn.len(), 1, "one kick holds the draw ({} kicks)", kicks.len());
        drawn.into_iter().next().unwrap()
    });
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    gc_debug_set_tiled_rt(0);
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_tiled_rt.rs: nothing still pointing at
         * the memory of a run reads freed memory */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

/* ---- the comparison ----------------------------------------------------------------- */

/// The registers compared, by name.
const COMPARED: [(u32, &str); 7] = [
    (GC_PE_COLOR_FORMAT, "PE_COLOR_FORMAT"),
    (GC_PE_COLOR_STRIDE, "PE_COLOR_STRIDE"),
    (GC_PE_LOGIC_OP, "PE_LOGIC_OP"),
    (GC_PE_DEPTH_CONFIG, "PE_DEPTH_CONFIG"),
    (GC_PE_DEPTH_STRIDE, "PE_DEPTH_STRIDE"),
    (GC_RA_EARLY_DEPTH, "RA_EARLY_DEPTH"),
    (GC_PE_MEM_CONFIG, "PE_MEM_CONFIG"),
];

/// One expected difference: the bits of `reg` where our knob-on word and
/// Mesa's differ, and why.
struct Expected {
    reg: u32,
    bits: u32,
    why: &'static str,
}

/// Every bit in which the knob-on words differ from the golden's. None of
/// them is a defect: each is a difference by design or a step not yet built.
const EXPECTED: &[Expected] = &[
    Expected {
        reg: GC_PE_COLOR_FORMAT,
        bits: GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW,
        why: "Mesa 26.2.1 makes the RGBA8 texture super-tiled on this core (SUPERTILED_TEXTURE, \
              etnaviv_resource.c:577-581; a TILED colour target comes only from a scanout modifier, \
              which an ES 2.0 harness on the shim cannot ask for). W4's shadow is TILED by design: \
              the RS tiles it in and copies it back, and the RS of this part reads a TILED source \
              (gc-tiled-wire.md W4)",
    },
    Expected {
        reg: GC_PE_LOGIC_OP,
        bits: PE_LOGIC_OP_DITHER_MODE,
        why: "Mesa dithers (GL_DITHER is on by default; DITHER_MODE(3) with the blob's pattern in \
              PE_DITHER, etnaviv_blend.c:196-201); this back end does not dither, on any target \
              (tools/vivante/README.md, 'Which differences in a depth-mode diff are expected'). \
              Not the knob's: the knob-off word has the same field",
    },
    Expected {
        reg: GC_PE_MEM_CONFIG,
        bits: PE_MEM_CONFIG_COLOR_TS_MODE | PE_MEM_CONFIG_DEPTH_TS_MODE,
        why: "Mesa gives both surfaces a tile-status buffer (COLOR_TS_MODE, DEPTH_TS_MODE); the \
              knob's W1-W5 have none: TS is W7-W9, gated on the X2's MMU-off probe \
              (gc-tiled-wire.md W7)",
    },
];

fn expected_bits(reg: u32) -> u32 {
    EXPECTED.iter().filter(|e| e.reg == reg).fold(0, |m, e| m | e.bits)
}

/// The knob-on words at the draw are Mesa's, but for the bits [`EXPECTED`]
/// names; and every bit it names does differ.
#[test]
fn knob_on_pe_ra_and_early_depth_words_match_mesas_but_the_listed_bits() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let g = golden_at_draw();
    let on = mode30(true);
    let mut wrong = Vec::new();
    for (reg, name) in COMPARED {
        let theirs = *g.get(&reg).unwrap_or_else(|| panic!("the golden writes no {name} before its draw"));
        let ours = *on.get(&reg).unwrap_or_else(|| panic!("knob on: no {name} before the draw"));
        let want = expected_bits(reg);
        if (ours ^ theirs) != want {
            wrong.push(format!(
                "{name}: ours {ours:#010x}, Mesa {theirs:#010x}, differ in {:#010x}, listed {want:#010x}",
                ours ^ theirs
            ));
        }
    }
    assert!(wrong.is_empty(), "knob-on words against tiled-ez.golden:\n{}", wrong.join("\n"));
    for e in EXPECTED {
        assert!(COMPARED.iter().any(|(r, _)| *r == e.reg), "listed {:#x} is not compared", e.reg);
        assert!(!e.why.is_empty());
    }
}

/// The fields the item names, one by one, so a failure says which: the
/// shadow is TILED (not linear) under SINGLE_BUFFER(2), the depth surface is
/// super-tiled with early depth in both halves, and the RA's early test
/// writes.
#[test]
fn knob_on_takes_mesas_tiled_layout_and_early_depth_fields() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let g = golden_at_draw();
    let on = mode30(true);
    let (ours, theirs) = (|r: u32| on[&r], |r: u32| g[&r]);
    let sb = |v: u32| (v >> 8) & 3;
    assert_eq!(sb(ours(GC_PE_LOGIC_OP)), 2, "SINGLE_BUFFER(2): a 32-bit target that is not linear");
    assert_eq!(sb(ours(GC_PE_LOGIC_OP)), sb(theirs(GC_PE_LOGIC_OP)));
    let ez = GC_PE_DEPTH_SUPER_TILED | GC_PE_DEPTH_EARLY_Z | GC_PE_DEPTH_DISABLE_ZS;
    assert_eq!(theirs(GC_PE_DEPTH_CONFIG) & ez, ez, "Mesa: super-tiled depth, early Z, DISABLE_ZS");
    assert_eq!(ours(GC_PE_DEPTH_CONFIG) & ez, ez);
    assert_eq!(ours(GC_RA_EARLY_DEPTH), theirs(GC_RA_EARLY_DEPTH), "RA_EARLY_DEPTH");
    assert_ne!(ours(GC_RA_EARLY_DEPTH) & GC_RA_EARLY_DEPTH_TEST_ENABLE, 0);
    assert_eq!(ours(GC_RA_EARLY_DEPTH) & GC_RA_EARLY_DEPTH_WRITE_DISABLE, 0);
    assert_eq!(ours(GC_PE_DEPTH_STRIDE), theirs(GC_PE_DEPTH_STRIDE), "a super-tiled depth's stride");
}

/// For the record of what the knob changes: with the knob off the same draw
/// is linear and late, and differs from the golden in more than the listed
/// bits (SINGLE_BUFFER, SUPER_TILED and EARLY_Z of the depth, the RA word).
#[test]
fn knob_off_is_further_from_mesa() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let g = golden_at_draw();
    let off = mode30(false);
    assert_eq!((off[&GC_PE_LOGIC_OP] >> 8) & 3, 1, "knob off: linear, SINGLE_BUFFER(1)");
    assert_eq!(off[&GC_PE_DEPTH_CONFIG] & (GC_PE_DEPTH_SUPER_TILED | GC_PE_DEPTH_EARLY_Z), 0);
    assert_ne!(off[&GC_RA_EARLY_DEPTH], g[&GC_RA_EARLY_DEPTH], "knob off: late depth");
    assert_eq!(off[&GC_PE_LOGIC_OP] & PE_LOGIC_OP_DITHER_MODE, 0, "no dither, knob off as on");
    let extra: Vec<_> =
        COMPARED.iter().filter(|(r, _)| (off[r] ^ g[r]) & !expected_bits(*r) != 0).map(|(_, n)| *n).collect();
    assert!(extra.contains(&"PE_DEPTH_CONFIG") && extra.contains(&"RA_EARLY_DEPTH"), "{extra:?}");
}
