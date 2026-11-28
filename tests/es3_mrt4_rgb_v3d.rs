// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An alpha-less colour attachment at draw buffers 1 and 3 on V3D (rgb4-pin-v3d,
//! plan #368 RC14): attachments 0-2 are RGBA8 renderbuffers and attachment 3
//! (and, in the second set, attachment 1) is GL_RGB8 or GL_RGB565.
//!
//! The defect (D1 of item mesa-rt4-v3d). yakogl stores such a format as RGBA8
//! and only tracks attachment 0's alpha bits (`GlesDraw::target_alpha_bits`,
//! src/gl/draw.rs `set_target_format`, src/v3d/state.rs `job_state`). Those bits
//! drive (a) the alpha bit of the COLOR_WRITE_MASKS nibble, (b) the
//! DST_ALPHA -> ONE, ONE_MINUS_DST_ALPHA -> ZERO and SRC_ALPHA_SATURATE -> ZERO
//! blend factor mapping of the one BLEND_CFG, and (c) the tile load's
//! `force_alpha_1` (src/v3d/backend.rs `no_alpha`, V3dJobClear::force_alpha_1,
//! driver/src/v3d/v3d_job.c `emit_tile_list`, which sets it on RT 0's load
//! only; the RT 1..3 loads set none). Mesa 26.2.1 does it per render target:
//! `force_alpha_1 = util_format_has_alpha1(format)` per surface
//! (src/gallium/drivers/v3d/v3dx_rcl.c:99) and, for the alpha-less RTs, a second
//! BLEND_CFG with `dst_alpha_one` (v3dx_emit.c:533-563; COLOR_WRITE_MASKS at
//! :571-585). The Mesa C sources are not vendored; the packet fields are read
//! from third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml.
//!
//! Pinned and passing today: the framebuffer is COMPLETE, the RT 3 surface is
//! the RGBA8 one with internal type 8 and 32 bpp in the real job builder's
//! TILE_RENDERING_MODE_CFG_COLOR, glReadPixels of attachment 3 reads alpha 255.
//! Pinned as `#[ignore = "defect: ..."]` (the answer Mesa gives), for RT 3 and
//! for RT 1: (a) the RT's COLOR_WRITE_MASKS nibble disables alpha, (b) a
//! GL_DST_ALPHA blend uses factor ONE for it, (c) its LOAD_TILE_BUFFER_GENERAL
//! sets "Force alpha 1". The fix belongs to the owners of src/v3d/**, src/gl/**
//! (#359, #361); the oracles below (`rt_write_mask`, `rt_blend_factors`,
//! `rt_load_force_alpha`) are to be edited to the fix's interface.
//!
//! The draw-target count is set through the knob, as tests/es3_mrt4_v3d.rs.
//!
//! Run:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,test-knobs --test es3_mrt4_rgb_v3d
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-v3d",
    feature = "no-shader-table",
    feature = "test-knobs",
    not(feature = "report-es2")
))]
#![allow(dead_code)]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::ffi::*;
use yakogl::v3d::state;

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

/// What the back end was handed by one draw, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::blend` as the GL core set it.
    blend: bool,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`, when the V3D takes the draw.
    job: Option<V3dJobState>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d));
    let job = if d.index_count != 0 { state::job_state(d).ok() } else { None };
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        blend: d.blend,
        refused,
        job,
    });
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
    name: c"es3-mrt4-rgb".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::v3d::debug::v3d_debug_set_draw_targets(4);
    assert_eq!(backend::draw_targets(), 4);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_BLEND);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the framebuffer, the program, the draw ---------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// A bound 8x8 framebuffer with one renderbuffer of each sized format on
/// colour attachments 0.., all of them draw buffers.
fn framebuffer(formats: &[GLenum]) -> Fbo {
    let (mut fb, mut rbs) = (0, Vec::new());
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, f) in formats.iter().enumerate() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, *f, 8, 8);
            assert_eq!(err(), GL_NO_ERROR, "storage {f:#x}");
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
            rbs.push(rb);
        }
        let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
    }
    Fbo { fb, rbs }
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

/// The current program: one `out` per entry of `types` ("vec4", "ivec4",
/// "uvec4").
fn program(types: &[&str]) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, t) in types.iter().enumerate() {
        fs += &format!("layout(location = {i}) out {t} o{i};\n");
    }
    fs += "void main() {\n";
    for (i, t) in types.iter().enumerate() {
        fs += &match *t {
            "vec4" => format!("o{i} = vec4(0.25, 0.5, 0.75, 0.5);\n"),
            "ivec4" => format!("o{i} = ivec4(-1, 2, -3, 4);\n"),
            _ => format!("o{i} = uvec4(1u, 2u, 3u, 4u);\n"),
        };
    }
    fs += "}\n";
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// GL_BLEND on: colour `src * a + dst * (1 - a)`, alpha `dst - src`.
fn blend_on() {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_BLEND);
        glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_REVERSE_SUBTRACT);
        glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE);
    }
}

/// Draw one triangle and hand back what the V3D's job state carries for it.
fn draw() -> Seen {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/* ---- Mesa's packet definitions, read from v3d_packet.xml ---- */

const XML: &str = "third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml";

struct Pkt {
    code: u8,
    name: String,
    /// The `sub-id` field's default (the `Tile Rendering Mode Cfg` family), if any.
    sub_id: Option<u8>,
    fields: Vec<(String, u32, u32)>,
    /// Bytes with the opcode byte.
    size: usize,
}

fn attr(line: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let at = line.find(&key)? + key.len();
    let end = line[at..].find('"')?;
    Some(line[at..at + end].to_string())
}

/// The V3D 4.2 packets: those with no `min_ver` above 42 (the 7.1 twins share
/// codes and names).
fn packets() -> &'static [Pkt] {
    static P: OnceLock<Vec<Pkt>> = OnceLock::new();
    P.get_or_init(|| {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(XML))
            .expect("Mesa's v3d_packet.xml");
        let mut out: Vec<Pkt> = Vec::new();
        let mut cur: Option<Pkt> = None;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with("<packet ") {
                let v71 = attr(l, "min_ver").is_some_and(|v| v.parse::<u32>().unwrap() > 42);
                let p = Pkt {
                    code: attr(l, "code").map(|c| c.parse().unwrap()).unwrap_or(0),
                    name: attr(l, "name").unwrap(),
                    sub_id: None,
                    fields: Vec::new(),
                    size: 1,
                };
                if l.ends_with("/>") {
                    if !v71 {
                        out.push(p);
                    }
                } else if v71 {
                    /* a 7.1 packet: its fields are read into a throwaway */
                    cur = Some(Pkt { code: 255, ..p });
                } else {
                    cur = Some(p);
                }
            } else if l.starts_with("<field ") {
                if let Some(p) = cur.as_mut() {
                    let name = attr(l, "name").unwrap();
                    let size: u32 = attr(l, "size").unwrap().parse().unwrap();
                    let start: u32 = attr(l, "start").unwrap().parse().unwrap();
                    if name == "sub-id" {
                        p.sub_id = attr(l, "default").map(|d| d.parse().unwrap());
                    }
                    p.size = p.size.max(1 + (start + size).div_ceil(8) as usize);
                    p.fields.push((name, start, size));
                }
            } else if l.starts_with("</packet>") {
                if let Some(p) = cur.take() {
                    if p.code != 255 {
                        out.push(p);
                    }
                }
            }
        }
        out
    })
}

fn pkt(name: &str) -> &'static Pkt {
    let mut it = packets().iter().filter(|p| p.name == name);
    let p = it.next().unwrap_or_else(|| panic!("{XML}: no 4.2 packet \"{name}\""));
    assert!(it.next().is_none(), "{XML}: \"{name}\" twice");
    p
}

/// One decoded packet: its Mesa name and the bytes after the opcode.
#[derive(Clone, Debug)]
struct Dec {
    name: &'static str,
    data: Vec<u8>,
}

impl Dec {
    /// Field `field` of this packet, bit `start` counted from the byte after the opcode.
    fn get(&self, field: &str) -> u64 {
        let p = pkt(self.name);
        let &(_, start, size) = p
            .fields
            .iter()
            .find(|f| f.0 == field)
            .unwrap_or_else(|| panic!("{XML}: \"{}\" has no field \"{field}\"", self.name));
        let mut v: u128 = 0;
        for (i, b) in self.data.iter().enumerate().take(16) {
            v |= (*b as u128) << (8 * i);
        }
        ((v >> start) & ((1u128 << size) - 1)) as u64
    }
}

/// Walk a control list from its opcode bytes: the packet sizes are the XML's.
fn decode(bytes: &[u8]) -> Vec<Dec> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let code = bytes[at];
        let cands: Vec<&Pkt> = packets().iter().filter(|p| p.code == code).collect();
        assert!(!cands.is_empty(), "opcode {code} at byte {at}: not a 4.2 packet in {XML}");
        let size = cands[0].size;
        for c in &cands {
            assert_eq!(c.size, size, "{XML}: opcode {code} has packets of different sizes");
        }
        let p = if cands.len() == 1 {
            cands[0]
        } else {
            let sub = bytes[at + 1] & 0xf;
            cands.iter().find(|c| c.sub_id == Some(sub)).unwrap_or_else(|| panic!("opcode {code} sub-id {sub}"))
        };
        assert!(at + size <= bytes.len(), "packet \"{}\" runs past the list", p.name);
        out.push(Dec { name: &p.name, data: bytes[at + 1..at + size].to_vec() });
        at += size;
    }
    out
}

/* ---- the C job builder ---- */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_job.c` and v3d_frame.c, built as a shared object of this
/// test's own and reached through the handle only (RTLD_LOCAL).
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_mrt4_rgb_v3d");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("es3_mrt4_rgb_v3d-{}.so", std::process::id()));
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args([
                "driver/src/v3d/v3d_job.c",
                "driver/src/v3d/v3d_frame.c",
                "driver/src/v3d/v3d_tex.c",
                "driver/src/v3d/v3d_tiling_mesa.c",
                "-lm",
                "-o",
            ])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "driver/src/v3d/v3d_job.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the job builder loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_job.c");
    p
}

type BeginFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobTarget, *mut V3dCl, *mut V3dCl, *mut V3dCl) -> bool;
type EndFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobClear, *mut u32, *mut u32) -> bool;

const BCL_GPU: u32 = 0x1000_0000;
const RCL_GPU: u32 = 0x1100_0000;
const IND_GPU: u32 = 0x1200_0000;
const LIST_BYTES: usize = 16 * 1024;

/// What the job builder wrote for one job with no draws.
struct Lists {
    bcl: Vec<Dec>,
    rcl: Vec<Dec>,
    tile: Vec<Dec>,
}

impl Lists {
    fn rcl_named(&self, name: &str) -> Vec<&Dec> {
        self.rcl.iter().filter(|d| d.name == name).collect()
    }
    fn tile_named(&self, name: &str) -> Vec<&Dec> {
        self.tile.iter().filter(|d| d.name == name).collect()
    }
}

fn build(t: &V3dJobTarget, c: &V3dJobClear) -> Option<Lists> {
    let mut bcl_buf = vec![0u8; LIST_BYTES];
    let mut rcl_buf = vec![0u8; LIST_BYTES];
    let mut ind_buf = vec![0u8; LIST_BYTES];
    let mut bcl = V3dCl {
        buf: bcl_buf.as_mut_ptr(),
        gpu: BCL_GPU,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let mut rcl = V3dCl {
        buf: rcl_buf.as_mut_ptr(),
        gpu: RCL_GPU,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let mut ind = V3dCl {
        buf: ind_buf.as_mut_ptr(),
        gpu: IND_GPU,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let mut job = V3dJob::ZERO;
    let (mut start, mut end) = (0u32, 0u32);
    // SAFETY: the C signatures of v3d_job.h; the lists are live for the calls.
    let ok = unsafe {
        let begin: BeginFn = std::mem::transmute(sym("v3d_job_begin"));
        let finish: EndFn = std::mem::transmute(sym("v3d_job_end"));
        begin(&mut job, t, &mut bcl, &mut rcl, &mut ind) && finish(&mut job, c, &mut start, &mut end)
    };
    if !ok {
        return None;
    }
    let tile_bytes = &ind_buf[(start - IND_GPU) as usize..(end - IND_GPU) as usize];
    Some(Lists {
        bcl: decode(&bcl_buf[..bcl.used as usize]),
        rcl: decode(&rcl_buf[..rcl.used as usize]),
        tile: decode(tile_bytes),
    })
}

/* ---- the oracles ---------------------------------------------------------- */

/// A GL_RGB8 / GL_RGB565 renderbuffer is stored as RGBA8 (`GlesDraw` target
/// format RGBA8); this is the attachment-3 format in the rows below.
const RGB: [GLenum; 2] = [GL_RGB8, GL_RGB565];

const V3D_FACTOR_NAMES: [(u32, &str); 3] =
    [(V3D_BLEND_FACTOR_ZERO, "ZERO"), (V3D_BLEND_FACTOR_ONE, "ONE"), (V3D_BLEND_FACTOR_DST_ALPHA, "DST_ALPHA")];

fn fname(f: u32) -> String {
    V3D_FACTOR_NAMES.iter().find(|n| n.0 == f).map_or(format!("factor {f}"), |n| n.1.to_string())
}

/// RT `rt`'s COLOR_WRITE_MASKS nibble as v3d_job.c `emit_state` writes it
/// (a set bit disables a channel; v3dx_emit.c:571-585): RT 0 from `colormask`,
/// RT 1..3 from `rt_colormask` with `rt_colormask_own`, else RT 0's nibble.
fn rt_write_mask(s: &Seen, rt: usize) -> u32 {
    let j = s.job.as_ref().expect("the V3D takes the draw");
    let off = !j.colormask & 0xf;
    if rt > 0 && j.rt_colormask_own {
        !(j.rt_colormask >> (4 * (rt - 1))) & 0xf
    } else {
        off
    }
}

/// The (source, destination) RGB blend factors RT `rt` is blended with. The
/// job state holds one BLEND_CFG for every RT today, so this is that one;
/// Mesa's second BLEND_CFG for the alpha-less RTs (v3dx_emit.c:533-563) is
/// what the fix adds, and this is where it is read.
fn rt_blend_factors(s: &Seen, _rt: usize) -> (u32, u32) {
    let j = s.job.as_ref().expect("the V3D takes the draw");
    (j.rgb_src, j.rgb_dst)
}

/// The "Force alpha 1" of the real job builder's load of RT `rt` (RT 0's
/// from `V3dJobClear::force_alpha_1`, which the back end sets from RT 0's alpha
/// bits), for a job that loads all four RTs and whose alpha-less RT is `rt`.
/// `force` is what the back end hands the builder for that RT: today a single
/// bool, true here (the format of `rt` has no alpha).
fn rt_load_force_alpha(rt: usize) -> u64 {
    let mut c = V3dJobClear { z: 1.0, force_alpha_1: true, ..Default::default() };
    c.load_rt0 = true;
    c.load_rt = [true, true, true];
    let l = build(&target(), &c).expect("the job builder takes four RGBA8 RTs");
    let ld: Vec<&Dec> =
        l.tile_named("Load Tile Buffer General").into_iter().filter(|d| d.get("Buffer to Load") == rt as u64).collect();
    assert_eq!(ld.len(), 1, "one load of RT {rt} (v3dx_rcl.c:198-212)");
    ld[0].get("Force alpha 1")
}

const W: u32 = 64;
const H: u32 = 64;
const ADDR: [u32; 4] = [0x2000_0000, 0x2100_0000, 0x2200_0000, 0x2300_0000];

/// A typed job of four RGBA8 RTs, as the back end builds for the GL
/// framebuffer below (an alpha-less format is an RGBA8 surface).
fn target() -> V3dJobTarget {
    let mut t = V3dJobTarget {
        width: W,
        height: H,
        address: ADDR[0],
        stride: W * 4,
        tile_alloc: 0x3000_0000,
        rts: 4,
        typed: true,
        rt_format: [V3D_OUTPUT_IMAGE_FORMAT_RGBA8; 4],
        ..Default::default()
    };
    for n in 1..4 {
        t.rt[n - 1] = V3dJobRt { address: ADDR[n], stride: W * 4, ..Default::default() };
    }
    t
}

/// Attachments 0-2 RGBA8 and `at` GL_RGB8 / GL_RGB565 with the program
/// writing a vec4 each, blending off, the draw taken.
fn rgb_draw(fmt: GLenum, at: usize) -> (Fbo, Seen) {
    let mut formats = [GL_RGBA8; 4];
    formats[at] = fmt;
    let fbo = framebuffer(&formats);
    program(&["vec4"; 4]);
    let s = draw();
    (fbo, s)
}

fn rgb_blend_draw(fmt: GLenum, at: usize) -> (Fbo, Seen) {
    let mut formats = [GL_RGBA8; 4];
    formats[at] = fmt;
    let fbo = framebuffer(&formats);
    program(&["vec4"; 4]);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_BLEND);
        glBlendFuncSeparate(GL_DST_ALPHA, GL_ONE_MINUS_DST_ALPHA, GL_ONE, GL_ZERO);
    }
    let s = draw();
    (fbo, s)
}

/* ---- passing today --------------------------------------------------------- */

/// RGBA8 x3 plus an alpha-less attachment 3 is a COMPLETE framebuffer, and the
/// draw reaches the V3D with four targets.
#[test]
fn rgb_attachment_3_is_complete_and_drawn() {
    let _g = context();
    for f in RGB {
        let (_fbo, s) = rgb_draw(f, 3);
        assert_eq!(s.refused, None, "{f:#x}");
        assert_eq!(s.targets.len(), 4);
        assert!(s.targets.iter().all(|t| t.0 != 0));
        assert!(s.job.is_some());
    }
}

/// The alpha-less RT 3 is the RGBA8 surface, as RT 0-2, and the real job
/// builder configures it with internal type 8, bpp 32 (v3dx_format_table.c
/// :225-233; v3d_packet.xml TILE_RENDERING_MODE_CFG_COLOR).
#[test]
fn rt3_is_rgba8_type_8_bpp_32() {
    let _g = context();
    for f in RGB {
        let (_fbo, s) = rgb_draw(f, 3);
        assert_eq!(s.targets[3].1, s.targets[0].1, "{f:#x}: RT 3 stores as RGBA8 like RT 0");
    }
    let l = build(&target(), &V3dJobClear { z: 1.0, ..Default::default() }).expect("four RGBA8 RTs");
    let c = l.rcl_named("Tile Rendering Mode Cfg (Color)");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].get("Render Target 3 Internal Type"), V3D_INTERNAL_TYPE_8 as u64);
    assert_eq!(c[0].get("Render Target 3 Internal BPP"), V3D_INTERNAL_BPP_32 as u64);
}

/// glReadPixels of attachment 3 reads alpha 255 from the alpha-less format
/// (ES 3.0 4.3.2: a missing alpha reads as 1).
#[test]
fn rgb_attachment_3_reads_alpha_255() {
    let _g = context();
    for f in RGB {
        let (_fbo, _s) = rgb_draw(f, 3);
        let mut px = [0u8; 4];
        // SAFETY: plain GL calls; room for one RGBA8 pixel.
        unsafe {
            glReadBuffer(GL_COLOR_ATTACHMENT0 + 3);
            glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(px[3], 255, "{f:#x}: alpha of attachment 3");
    }
}

/* ---- defect pins: RT 3 and RT 1 ---------------------------------------------- */

fn pin_mask(at: usize) {
    let _g = context();
    for f in RGB {
        let (_fbo, s) = rgb_draw(f, at);
        assert_eq!(
            rt_write_mask(&s, at) & 0x8,
            0x8,
            "{f:#x}: RT {at}'s COLOR_WRITE_MASKS nibble disables alpha, as RT 0's does for an RGB RT 0 \
             (state.rs colormask &= 7); Mesa v3dx_emit.c:571-585"
        );
        assert_eq!(rt_write_mask(&s, 0), 0, "RT 0 (RGBA8) still writes alpha");
    }
}

fn pin_blend(at: usize) {
    let _g = context();
    for f in RGB {
        let (_fbo, s) = rgb_blend_draw(f, at);
        assert_eq!(s.refused, None);
        let (src, dst) = rt_blend_factors(&s, at);
        assert_eq!(
            (fname(src), fname(dst)),
            ("ONE".to_string(), "ZERO".to_string()),
            "{f:#x}: DST_ALPHA / ONE_MINUS_DST_ALPHA on RT {at} use 1 / 0 (a second BLEND_CFG with dst_alpha_one, \
             v3dx_emit.c:533-563)"
        );
    }
}

fn pin_load(at: usize) {
    assert_eq!(
        rt_load_force_alpha(at),
        1,
        "RT {at}'s LOAD_TILE_BUFFER_GENERAL sets Force alpha 1 (util_format_has_alpha1, v3dx_rcl.c:99)"
    );
}

#[test]
#[ignore = "defect: the alpha bit of RT 3's COLOR_WRITE_MASKS nibble is not set for GL_RGB8/GL_RGB565 (RT 0's alpha bits only)"]
fn rt3_write_mask_disables_alpha() {
    pin_mask(3);
}

#[test]
#[ignore = "defect: DST_ALPHA on a blend with an alpha-less RT 3 keeps DST_ALPHA (one BLEND_CFG, RT 0's alpha bits decide)"]
fn rt3_dst_alpha_blend_uses_one() {
    pin_blend(3);
}

#[test]
#[ignore = "defect: RT 3's LOAD_TILE_BUFFER_GENERAL never sets force_alpha_1 (v3d_job.c emit_tile_list, RT 0 only)"]
fn rt3_load_forces_alpha_1() {
    pin_load(3);
}

#[test]
#[ignore = "defect: the alpha bit of RT 1's COLOR_WRITE_MASKS nibble is not set for GL_RGB8/GL_RGB565 (RT 0's alpha bits only)"]
fn rt1_write_mask_disables_alpha() {
    pin_mask(1);
}

#[test]
#[ignore = "defect: DST_ALPHA on a blend with an alpha-less RT 1 keeps DST_ALPHA (one BLEND_CFG, RT 0's alpha bits decide)"]
fn rt1_dst_alpha_blend_uses_one() {
    pin_blend(1);
}

#[test]
#[ignore = "defect: RT 1's LOAD_TILE_BUFFER_GENERAL never sets force_alpha_1 (v3d_job.c emit_tile_list, RT 0 only)"]
fn rt1_load_forces_alpha_1() {
    pin_load(1);
}
