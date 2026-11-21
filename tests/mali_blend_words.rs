// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Blend descriptor and the blend shader of every dEQP-GLES2
//! fragment_ops.blend state, drawn through the GL core into the Mali back
//! end over a mock of its OSAL, against Mesa 26.2.1's drm-shim references
//! (blend-words, plan #373 cause 3).
//!
//! The references are tools/mali/testdata/pan_blend_ref/ (made by
//! tools/mali/pan_blend_ref.py, read here at run time as
//! src/shaderc/online/mali/blend_tests.rs reads them): states.tsv holds each
//! distinct state of the dEQP groups (and one colour-mask state), blend.tsv
//! the Blend descriptor Mesa emitted for it (the mode, word 0, the equation
//! word, the constant, the fixed-function internal) and, for a Shader-mode
//! state, the file of its blend shader's words. This test replays Mesa's
//! draw: a 64x64 `GL_RGBA`/`GL_UNSIGNED_BYTE` texture framebuffer, the
//! reference's program (an ES 2 fragment shader writing a mediump varying),
//! blend colour (0.2, 0.4, 0.6, 0.8); per state glClear, the equations and
//! factors, the colour mask, one draw and glFinish. From each submitted
//! command stream it reads, as the GPU would, the RUN_IDVS's Blend
//! descriptor and, in Shader mode, the words at the shader's address, and
//! compares:
//!
//! - the mode (`shader`, `fixed-function`, `opaque`) with blend.tsv's;
//! - word 0 (load destination, enable, the blend constant) and word 1 (the
//!   equation, 0 in Shader mode) with blend.tsv's, bit for bit;
//! - for a fixed-function or opaque descriptor, the internal word (Num
//!   Comps, RT, the conversion's memory format and raw) with blend.tsv's;
//! - for a Shader-mode descriptor, the internal word's low 32 bits with
//!   Mesa's 0 (the board's descriptor quoted in src/mali/frame.rs; the PC is
//!   the high half, which blend.tsv does not keep), and the blend shader's
//!   words with shaders/<file>.words (trailing zero words left out, as
//!   blend_tests.rs compares them; the words after Mesa's last must be zero
//!   to the end of the 128-byte block it falls in).
//!
//! The states compared: all of states.tsv (2205: equation_src_func_dst_func
//! 630, rgb_func_alpha_func src and dst, rgb_equation_alpha_equation 9, the
//! colour-mask state). tests/mali_blend_states.rs drives 1899 states; the
//! test counts those that states.tsv holds (its equation_src_func_dst_func and
//! rgb_equation_alpha_equation families, and the rgb_func_alpha_func states
//! whose alpha pair is one dEQP draws) and prints the count; the others have
//! no reference.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_blend_words
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::cs::Instr;
use yakogl::mali::genxml::v10::*;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu =
    OsalMaliGpu { gpu_id: 0xa867_0005, tiler_features: 0x809, cs_reg_count: 96, unpreserved_cs_reg_count: 4 };
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back.
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);
/// Every buffer: (GPU VA, CPU address, size).
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());
/// Every submit: the stream's GPU VA and length in bytes.
static KICKS: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let gpu = 0x40_0000_0000 + n * (16 << 20);
    BUFFERS.lock().unwrap().push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: n as *mut c_void };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    assert_eq!(len, core::mem::size_of::<OsalMaliKick>());
    let k = &*kick.cast::<OsalMaliKick>();
    KICKS.lock().unwrap().push((k.stream_gpu, k.stream_bytes as usize));
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// The buffer holding GPU address `gpu` and `len` bytes after it: its base,
/// CPU address and size.
fn buffer_of(gpu: u64, len: usize) -> Option<(u64, usize, usize)> {
    BUFFERS
        .lock()
        .unwrap()
        .iter()
        .copied()
        .find(|&(base, _, size)| gpu >= base && gpu + len as u64 <= base + size as u64)
}

/// `len` bytes at GPU address `gpu`.
fn mem(gpu: u64, len: usize) -> Vec<u8> {
    let (base, cpu, _) = buffer_of(gpu, len).unwrap_or_else(|| panic!("no buffer holds 0x{gpu:x}+{len}"));
    // SAFETY: the buffer is leaked, `size` bytes long, and the range is inside it.
    unsafe { core::slice::from_raw_parts((cpu + (gpu - base) as usize) as *const u8, len).to_vec() }
}

fn words_at(gpu: u64, n: usize) -> Vec<u64> {
    mem(gpu, 8 * n).as_chunks::<8>().0.iter().map(|w| u64::from_le_bytes(*w)).collect()
}

/// The full address of a blend shader whose low 32 bits the descriptor
/// holds: the one buffer (of the 4 GiB windows the buffers are in) that
/// holds it.
fn shader_address(low: u32) -> u64 {
    let b = BUFFERS.lock().unwrap();
    let mut hits: Vec<u64> = b
        .iter()
        .map(|&(base, _, size)| (base & !0xffff_ffff | u64::from(low), base, size))
        .filter(|&(a, base, size)| a >= base && a < base + size as u64)
        .map(|(a, _, _)| a)
        .collect();
    hits.dedup();
    assert_eq!(hits.len(), 1, "blend shader at low 0x{low:x}: buffers {hits:x?}");
    hits[0]
}

/* ---- what the back end built --------------------------------------------- */

/// One draw's Blend descriptor, as its four 32-bit words, and the blend
/// shader's full address when the mode is Shader.
#[derive(Debug)]
struct DrawOut {
    word0: u32,
    word1: u32,
    internal: u64,
    shader: Option<u64>,
}

/// The draws of the kicks since `from`, read as the GPU would: the stream's
/// register moves are followed and each RUN_IDVS reads the Blend pointer
/// (r50, its low four bits the target count).
fn draws(from: usize) -> Vec<DrawOut> {
    let kicks = KICKS.lock().unwrap()[from..].to_vec();
    let mut out = Vec::new();
    for (gpu, bytes) in kicks {
        let raw = mem(gpu, bytes);
        let mut regs = [0u32; 128];
        for w in raw.as_chunks::<8>().0 {
            match Instr::decode(u64::from_le_bytes(*w)) {
                Ok(Instr::Move48(m)) => {
                    regs[m.destination as usize] = m.immediate as u32;
                    regs[m.destination as usize + 1] = (m.immediate >> 32) as u32;
                }
                Ok(Instr::Move32(m)) => regs[m.destination as usize] = m.immediate,
                Ok(Instr::RunIdvs(_)) => {
                    let blend = (u64::from(regs[50]) | u64::from(regs[51]) << 32) & !0xf;
                    let b = mem(blend, Blend::LENGTH);
                    let w = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
                    let internal = u64::from(w(2)) | u64::from(w(3)) << 32;
                    let shader = (internal & 3 == u64::from(BlendMode::SHADER.0))
                        .then(|| shader_address((internal >> 32) as u32 & !0xf));
                    out.push(DrawOut { word0: w(0), word1: w(1), internal, shader });
                }
                _ => {}
            }
        }
    }
    out
}

/* ---- the references ------------------------------------------------------ */

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata/pan_blend_ref")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("a hex number")
}

fn trim(mut v: Vec<u64>) -> Vec<u64> {
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

/// One line of states.tsv: id, eqRGB srcRGB dstRGB eqA srcA dstA, mask.
struct State {
    id: usize,
    name: String,
    p: [GLenum; 6],
    mask: [u8; 4],
}

fn states() -> Vec<State> {
    read("states.tsv")
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let p: Vec<GLenum> = f[2].split(' ').map(hex).collect();
            let m: Vec<u8> = f[3].split(' ').map(|b| b.parse().expect("a mask bit")).collect();
            State {
                id: f[0].parse().expect("an id"),
                name: f[1].to_string(),
                p: p.try_into().expect("six enums"),
                mask: m.try_into().expect("four mask bits"),
            }
        })
        .collect()
}

/// One line of blend.tsv.
struct Expect {
    mode: BlendMode,
    word0: u32,
    word1: u32,
    constant: u32,
    /// Fixed-function or opaque: the internal word as Mesa packed it.
    internal: Option<u64>,
    /// Shader: the reference's words, trailing zeros left out.
    words: Option<Vec<u64>>,
}

fn expects() -> HashMap<usize, Expect> {
    read("blend.tsv")
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let id: usize = f[0].parse().expect("an id");
            let mode = match f[2] {
                "shader" => BlendMode::SHADER,
                "fixed-function" => BlendMode::FIXED_FUNCTION,
                "opaque" => BlendMode::OPAQUE,
                "off" => BlendMode::OFF,
                m => panic!("state {id}: mode {m}"),
            };
            let internal = (f[6] != "-").then(|| {
                /* Num Comps/RT/Conversion: "4/0/RGBA8 TB RGBA/raw=false" */
                let i: Vec<&str> = f[6].split('/').collect();
                let memory_format = match i[2] {
                    "RGBA8 TB RGBA" => Format::RGBA8_TB.0 << 12 | RgbComponentOrder::RGBA.0,
                    other => panic!("state {id}: format {other}"),
                };
                let raw = match i[3] {
                    "raw=false" => false,
                    "raw=true" => true,
                    other => panic!("state {id}: {other}"),
                };
                let ib = InternalBlend {
                    mode,
                    fixed_function: BlendFixedFunction {
                        num_comps: i[0].parse().expect("Num Comps"),
                        rt: i[1].parse().expect("RT"),
                        conversion: InternalConversion { memory_format, raw },
                    },
                    ..Default::default()
                };
                u64::from_le_bytes(ib.pack().expect("the reference's internal packs"))
            });
            let words = (f[7] != "-").then(|| {
                trim(
                    read(&format!("{}.words", f[7]))
                        .lines()
                        .map(|w| u64::from_str_radix(w.trim(), 16).expect("a word"))
                        .collect(),
                )
            });
            let e = Expect {
                mode,
                word0: hex(f[3]),
                word1: hex(f[4]),
                constant: f[5].parse().expect("a constant"),
                internal,
                words,
            };
            (id, e)
        })
        .collect()
}

/* ---- the draw ------------------------------------------------------------ */

/// pan_blend_ref.py's program: dEQP's blend program, a mediump varying.
const VS: &str = "attribute highp vec4 a_position;\nattribute mediump vec4 a_color;\nvarying mediump vec4 v_color;\n\
                  void main() { gl_Position = a_position; v_color = a_color; }\n";
const FS: &str = "varying mediump vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";

static POS: [f32; 12] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0];
static COL: [f32; 24] = [
    0.25, 0.5, 0.75, 0.5, 0.25, 0.5, 0.75, 0.5, 0.25, 0.5, 0.75, 0.5, 0.25, 0.5, 0.75, 0.5, 0.25, 0.5, 0.75, 0.5, 0.25,
    0.5, 0.75, 0.5,
];

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

/// The context, pan_blend_ref.py's 64x64 RGBA texture framebuffer and
/// program, the blend colour.
fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file. The vertex arrays are static.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        let (mut tex, mut fb) = (0, 0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_color".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        glViewport(0, 0, 64, 64);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(0);
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
        glClearColor(0.5, 0.5, 0.5, 0.5);
        glEnable(GL_BLEND);
        glBlendColor(0.2, 0.4, 0.6, 0.8);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// One state as pan_blend_ref.py draws it; the draw the back end built.
fn draw(s: &State) -> DrawOut {
    let from = KICKS.lock().unwrap().len();
    let refused = backend::draws_refused();
    // SAFETY: plain GL calls; the vertex arrays are static.
    let e = unsafe {
        glColorMask(1, 1, 1, 1);
        glClear(GL_COLOR_BUFFER_BIT);
        glBlendEquationSeparate(s.p[0], s.p[3]);
        glBlendFuncSeparate(s.p[1], s.p[2], s.p[4], s.p[5]);
        glColorMask(s.mask[0], s.mask[1], s.mask[2], s.mask[3]);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
        glGetError()
    };
    assert_eq!((e, backend::draws_refused() - refused), (GL_NO_ERROR, 0), "state {} {}: the draw", s.id, s.name);
    let mut d = draws(from);
    assert_eq!(d.len(), 1, "state {} {}: one draw in the frame", s.id, s.name);
    d.pop().unwrap()
}

/// The differences of one state, empty when it equals the reference.
fn compare(d: &DrawOut, x: &Expect) -> Vec<String> {
    let mut diff = Vec::new();
    let mode = BlendMode((d.internal & 3) as u32);
    if mode != x.mode {
        diff.push(format!("mode {mode:?}, Mesa {:?}", x.mode));
    }
    if d.word0 != x.word0 {
        diff.push(format!("word0 0x{:08x}, Mesa 0x{:08x}", d.word0, x.word0));
    }
    if d.word1 != x.word1 {
        diff.push(format!("equation 0x{:08x}, Mesa 0x{:08x}", d.word1, x.word1));
    }
    match (x.internal, &x.words, &d.shader) {
        (Some(i), _, _) if d.internal != i => diff.push(format!("internal 0x{:016x}, Mesa 0x{i:016x}", d.internal)),
        (None, Some(mesa), Some(pc)) => {
            if d.internal as u32 != 0 {
                diff.push(format!("shader internal low word 0x{:08x}, Mesa 0", d.internal as u32));
            }
            /* Mesa's words, then zeros to the end of the 128-byte block
             * the shader was uploaded in */
            let n = mesa.len();
            let end = (pc + 8 * n as u64).next_multiple_of(128);
            let ours = words_at(*pc, ((end - pc) / 8) as usize);
            if ours[..n] != mesa[..] || ours[n..].iter().any(|&w| w != 0) {
                diff.push(format!("shader words {:x?}, Mesa {mesa:x?}", trim(ours.clone())));
            }
        }
        (None, Some(_), None) => diff.push("no blend shader".into()),
        _ => {}
    }
    diff
}

/// Every state of states.tsv; the ones that differ from Mesa's (none: a
/// difference found later is pinned here with `#[ignore]` and a fix).
#[test]
fn every_blend_state_s_descriptor_and_shader_equal_mesa_s() {
    let states = states();
    let expects = expects();
    assert_eq!(states.len(), 2205);
    assert_eq!(expects.len(), states.len());
    setup();
    let (mut shader, mut fixed, mut opaque) = (0, 0, 0);
    let mut differ = Vec::new();
    for s in &states {
        let x = &expects[&s.id];
        /* the reference's constant is word 0's Blend Constant field */
        assert_eq!(x.word0 >> 16, x.constant, "state {}: blend.tsv's constant", s.id);
        let d = draw(s);
        match x.mode {
            BlendMode::SHADER => shader += 1,
            BlendMode::FIXED_FUNCTION => fixed += 1,
            _ => opaque += 1,
        }
        let diff = compare(&d, x);
        if !diff.is_empty() {
            differ.push(format!("{} {}: {}", s.id, s.name, diff.join("; ")));
        }
    }
    let covered = blend_states_covered(&states);
    eprintln!(
        "blend words: {} states compared ({shader} shader, {fixed} fixed-function, {opaque} opaque), {} differ; \
         {covered} of tests/mali_blend_states.rs's 1899 states have a reference",
        states.len(),
        differ.len()
    );
    assert_eq!((shader, fixed, opaque), (1404, 799, 2));
    assert!(differ.is_empty(), "{} states differ from Mesa's:\n{}", differ.len(), differ.join("\n"));
}

/// How many of the states tests/mali_blend_states.rs draws states.tsv holds
/// (all four channels written).
fn blend_states_covered(states: &[State]) -> usize {
    const EQ: [GLenum; 3] = [GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT];
    const FUNCS: [GLenum; 14] = [
        GL_ZERO,
        GL_ONE,
        GL_SRC_COLOR,
        GL_ONE_MINUS_SRC_COLOR,
        GL_DST_COLOR,
        GL_ONE_MINUS_DST_COLOR,
        GL_SRC_ALPHA,
        GL_ONE_MINUS_SRC_ALPHA,
        GL_DST_ALPHA,
        GL_ONE_MINUS_DST_ALPHA,
        GL_CONSTANT_COLOR,
        GL_ONE_MINUS_CONSTANT_COLOR,
        GL_CONSTANT_ALPHA,
        GL_ONE_MINUS_CONSTANT_ALPHA,
    ];
    const ALPHA_PAIRS: [(GLenum, GLenum); 6] = [
        (GL_ZERO, GL_ONE),
        (GL_ONE, GL_ZERO),
        (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
        (GL_DST_ALPHA, GL_ONE_MINUS_DST_ALPHA),
        (GL_CONSTANT_ALPHA, GL_ONE_MINUS_CONSTANT_ALPHA),
        (GL_SRC_ALPHA_SATURATE, GL_DST_COLOR),
    ];
    let src: Vec<GLenum> = FUNCS.iter().copied().chain([GL_SRC_ALPHA_SATURATE]).collect();
    let mut driven = Vec::new();
    for e in EQ {
        for &s in &src {
            for d in FUNCS {
                driven.push([e, s, d, e, s, d]);
            }
        }
    }
    for &s in &src {
        for d in FUNCS {
            for (sa, da) in ALPHA_PAIRS {
                driven.push([GL_FUNC_ADD, s, d, GL_FUNC_ADD, sa, da]);
            }
        }
    }
    for r in EQ {
        for a in EQ {
            driven.push([r, GL_ONE, GL_ONE, a, GL_ONE, GL_ONE]);
        }
    }
    assert_eq!(driven.len(), 1899);
    let have: HashSet<[GLenum; 6]> = states.iter().filter(|s| s.mask == [1; 4]).map(|s| s.p).collect();
    driven.iter().filter(|p| have.contains(*p)).count()
}
