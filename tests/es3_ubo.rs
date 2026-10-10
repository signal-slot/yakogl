// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 uniform blocks through the GL API on the host, the way dEQP-GLES3
//! `functional.ubo.*` drives them (`1b0e4316`, `b0c0fa6b`): each case's interface
//! is rebuilt from `es3fUniformBlockTests.cpp`, its shaders generated as
//! `glsUniformBlockCase.cpp` writes them, and the case run as that file's
//! `UniformBlockCase::iterate` does -- the std140 reference layout, the
//! queried layout and the checks dEQP makes of it, the data copied into the
//! queried layout and uploaded per block or into one buffer of ranges, and
//! a draw.
//!
//! The host cannot render, so where dEQP compares an image this compares
//! what the image is made from: each value the generated shaders read, in
//! the row of the stage's default block the compiled shader reads it from,
//! as the draw hands it to the back end. That is the GL core's whole part of
//! a ubo case; what the back end's program does with those rows is not
//! measured here.
//!
//! Every group of `functional.ubo` but `random` is here, case for case
//! (2,032 cases); `random` needs dEQP's RandomUniformBlockCase generator,
//! which is not ported.
//!
//! An ES 2.0-reporting build (`report-es2`) has no uniform blocks:
//! `es2_build_has_no_blocks` is the only test it runs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_ubo
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_ubo
//! ```

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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

/// One stage as a draw handed it over: the compiled shader's uniform rows
/// (name, dword offset, dwords, elements) and the block's dwords.
#[derive(Clone, Default)]
struct Stage {
    rows: Vec<(String, u16, u16, u8)>,
    block: Vec<u32>,
}

/// The last draw's two stages.
static DRAWN: Mutex<Option<(Stage, Stage)>> = Mutex::new(None);

unsafe fn stage_of(sh: *const yakogl::shaderc::ShcShader, cpu: *const c_void, dwords: u16) -> Stage {
    let Some(sh) = sh.as_ref() else { return Stage::default() };
    let rows = sh
        .uniform_table()
        .iter()
        .map(|u| {
            let n: Vec<u8> = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            (String::from_utf8_lossy(&n).into_owned(), u.dword_off, u.dwords, u.array_len)
        })
        .collect();
    let block = if cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(cpu as *const u32, usize::from(dwords)).to_vec()
    };
    Stage { rows, block }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let vs = stage_of(d.vs, d.vs_uniform_cpu, d.vs_uniform_dwords);
    let fs = stage_of(d.fs, d.fs_uniform_cpu, d.fs_uniform_dwords);
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some((vs, fs));
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-ubo".as_ptr() as *const c_char,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---------------------------------------------------------------- ES 2.0 -- */

/// The ES 2.0 report has no uniform blocks to bind: the ES 3.0 limits it
/// answers are not asked of it here, only that a program with none reports
/// none.
#[cfg(feature = "report-es2")]
#[test]
fn es2_build_has_no_blocks() {
    let _g = context();
    // SAFETY: plain GL calls; one value each.
    unsafe {
        let p = glCreateProgram();
        let mut n: GLint = -1;
        glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut n);
        assert_eq!(n, 0);
        glDeleteProgram(p);
    }
}

/* ------------------------------------------------------------- ES 3.0 -- */

#[cfg(all(not(feature = "report-es2"), feature = "shaderc-online"))]
#[allow(clippy::needless_range_loop)] // dEQP's loops, kept in its shape
mod es3 {
    use super::*;
    use std::rc::Rc;

    /* ---- dEQP's interface model (glsUniformBlockCase.hpp) ---------------- */

    const PREC_LOW: u32 = 1;
    const PREC_MED: u32 = 1 << 1;
    const PREC_HIGH: u32 = 1 << 2;
    const PREC_MASK: u32 = PREC_LOW | PREC_MED | PREC_HIGH;
    const SHARED: u32 = 1 << 3;
    const PACKED: u32 = 1 << 4;
    const STD140: u32 = 1 << 5;
    const ROW_MAJOR: u32 = 1 << 6;
    const COLUMN_MAJOR: u32 = 1 << 7;
    const LAYOUT_MASK: u32 = SHARED | PACKED | STD140 | ROW_MAJOR | COLUMN_MAJOR;
    const MATRIX_MASK: u32 = ROW_MAJOR | COLUMN_MAJOR;
    const DECL_VERTEX: u32 = 1 << 8;
    const DECL_FRAGMENT: u32 = 1 << 9;
    const UNUSED_VERTEX: u32 = 1 << 10;
    const UNUSED_FRAGMENT: u32 = 1 << 11;
    const UNUSED_BOTH: u32 = UNUSED_VERTEX | UNUSED_FRAGMENT;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Scalar {
        F,
        I,
        U,
        B,
    }

    /// A basic type: GLSL name, GL token, scalar kind, columns, rows (a
    /// vector is one column).
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Basic {
        glsl: &'static str,
        gl: u32,
        sc: Scalar,
        cols: u32,
        rows: u32,
    }

    impl Basic {
        fn is_mat(self) -> bool {
            self.cols > 1
        }
        fn scalars(self) -> u32 {
            self.cols * self.rows
        }
    }

    const fn b(glsl: &'static str, gl: u32, sc: Scalar, cols: u32, rows: u32) -> Basic {
        Basic { glsl, gl, sc, cols, rows }
    }

    /// es3fUniformBlockTests.cpp `basicTypes`, in its order.
    const BASICS: [Basic; 25] = [
        b("float", GL_FLOAT, Scalar::F, 1, 1),
        b("vec2", GL_FLOAT_VEC2, Scalar::F, 1, 2),
        b("vec3", GL_FLOAT_VEC3, Scalar::F, 1, 3),
        b("vec4", GL_FLOAT_VEC4, Scalar::F, 1, 4),
        b("int", GL_INT, Scalar::I, 1, 1),
        b("ivec2", GL_INT_VEC2, Scalar::I, 1, 2),
        b("ivec3", GL_INT_VEC3, Scalar::I, 1, 3),
        b("ivec4", GL_INT_VEC4, Scalar::I, 1, 4),
        b("uint", GL_UNSIGNED_INT, Scalar::U, 1, 1),
        b("uvec2", GL_UNSIGNED_INT_VEC2, Scalar::U, 1, 2),
        b("uvec3", GL_UNSIGNED_INT_VEC3, Scalar::U, 1, 3),
        b("uvec4", GL_UNSIGNED_INT_VEC4, Scalar::U, 1, 4),
        b("bool", GL_BOOL, Scalar::B, 1, 1),
        b("bvec2", GL_BOOL_VEC2, Scalar::B, 1, 2),
        b("bvec3", GL_BOOL_VEC3, Scalar::B, 1, 3),
        b("bvec4", GL_BOOL_VEC4, Scalar::B, 1, 4),
        b("mat2", GL_FLOAT_MAT2, Scalar::F, 2, 2),
        b("mat3", GL_FLOAT_MAT3, Scalar::F, 3, 3),
        b("mat4", GL_FLOAT_MAT4, Scalar::F, 4, 4),
        b("mat2x3", GL_FLOAT_MAT2x3, Scalar::F, 2, 3),
        b("mat2x4", GL_FLOAT_MAT2x4, Scalar::F, 2, 4),
        b("mat3x2", GL_FLOAT_MAT3x2, Scalar::F, 3, 2),
        b("mat3x4", GL_FLOAT_MAT3x4, Scalar::F, 3, 4),
        b("mat4x2", GL_FLOAT_MAT4x2, Scalar::F, 4, 2),
        b("mat4x3", GL_FLOAT_MAT4x3, Scalar::F, 4, 3),
    ];

    fn basic(glsl: &str) -> Basic {
        *BASICS.iter().find(|t| t.glsl == glsl).expect("a basic type")
    }

    fn vec_of(n: u32) -> Basic {
        BASICS[(n - 1) as usize]
    }

    #[derive(Clone)]
    enum Ty {
        Basic(Basic, u32),
        Array(Box<Ty>, u32),
        Struct(Rc<Struct>, u32),
    }

    impl Ty {
        fn flags(&self) -> u32 {
            match self {
                Ty::Basic(_, f) | Ty::Struct(_, f) => *f,
                Ty::Array(..) => 0,
            }
        }
    }

    struct Struct {
        name: &'static str,
        members: Vec<(&'static str, Ty, u32)>,
    }

    struct Uniform {
        name: &'static str,
        ty: Ty,
        flags: u32,
    }

    struct Block {
        name: &'static str,
        instance: Option<&'static str>,
        array: u32,
        flags: u32,
        uniforms: Vec<Uniform>,
    }

    #[derive(Default)]
    struct Iface {
        structs: Vec<Rc<Struct>>,
        blocks: Vec<Block>,
    }

    fn bt(glsl: &str, flags: u32) -> Ty {
        Ty::Basic(basic(glsl), flags)
    }
    fn arr(t: Ty, n: u32) -> Ty {
        Ty::Array(Box::new(t), n)
    }
    fn u(name: &'static str, ty: Ty, flags: u32) -> Uniform {
        Uniform { name, ty, flags }
    }

    /* ---- the std140 reference layout (computeStd140Layout) --------------- */

    #[derive(Clone, Debug, PartialEq)]
    struct Entry {
        name: String,
        ty: Basic,
        size: i32,
        block: i32,
        offset: i32,
        array_stride: i32,
        matrix_stride: i32,
        row_major: bool,
    }

    #[derive(Clone, Debug)]
    struct BlockEntry {
        name: String,
        size: i32,
        active: Vec<i32>,
    }

    #[derive(Default, Debug)]
    struct Layout {
        blocks: Vec<BlockEntry>,
        uniforms: Vec<Entry>,
    }

    impl Layout {
        fn uniform(&self, name: &str) -> Option<usize> {
            self.uniforms.iter().position(|e| e.name == name)
        }
        fn block(&self, name: &str) -> Option<usize> {
            self.blocks.iter().position(|e| e.name == name)
        }
    }

    /// deRoundUp32 / deAlign32, for the non-negative values here.
    fn round_up(x: i32, to: i32) -> i32 {
        match x % to {
            0 => x,
            r => x + to - r,
        }
    }

    fn byte_alignment(t: Basic) -> i32 {
        match t.rows {
            1 => 4,
            2 => 8,
            _ => 16,
        }
    }

    fn array_stride_of(t: Basic) -> i32 {
        (t.scalars() as i32 * 4).max(16)
    }

    fn base_alignment(ty: &Ty) -> i32 {
        match ty {
            Ty::Basic(t, f) if t.is_mat() => {
                let n = if f & ROW_MAJOR != 0 { t.cols } else { t.rows };
                array_stride_of(vec_of(n))
            }
            Ty::Basic(t, _) => byte_alignment(*t),
            Ty::Array(e, _) => round_up(base_alignment(e), 16),
            Ty::Struct(s, _) => round_up(s.members.iter().map(|m| base_alignment(&m.1)).max().unwrap_or(0), 16),
        }
    }

    fn merge_flags(prev: u32, new: u32) -> u32 {
        let pick = |mask: u32| (if new & mask != 0 { new } else { prev }) & mask;
        pick(SHARED | PACKED | STD140) | pick(MATRIX_MASK)
    }

    fn std140(layout: &mut Layout, off: &mut i32, blk: i32, prefix: &str, ty: &Ty, mut flags: u32) {
        let al = base_alignment(ty);
        *off = round_up(*off, al);
        let row_major = |own: u32, flags: u32| (if own & MATRIX_MASK != 0 { own } else { flags }) & ROW_MAJOR != 0;
        match ty {
            Ty::Basic(t, own) => {
                let mut e = Entry {
                    name: prefix.into(),
                    ty: *t,
                    size: 1,
                    block: blk,
                    offset: *off,
                    array_stride: 0,
                    matrix_stride: 0,
                    row_major: false,
                };
                if t.is_mat() {
                    let rm = row_major(*own, flags);
                    let (vs, nv) = if rm { (t.cols, t.rows) } else { (t.rows, t.cols) };
                    let stride = array_stride_of(vec_of(vs));
                    e.matrix_stride = stride;
                    e.row_major = rm;
                    *off += nv as i32 * stride;
                } else {
                    *off += t.scalars() as i32 * 4;
                }
                layout.uniforms.push(e);
            }
            Ty::Array(el, n) => match &**el {
                Ty::Basic(t, own) => {
                    let mut e = Entry {
                        name: format!("{prefix}[0]"),
                        ty: *t,
                        size: *n as i32,
                        block: blk,
                        offset: *off,
                        array_stride: 0,
                        matrix_stride: 0,
                        row_major: false,
                    };
                    if t.is_mat() {
                        let rm = row_major(*own, flags);
                        let (vs, nv) = if rm { (t.cols, t.rows) } else { (t.rows, t.cols) };
                        let stride = array_stride_of(vec_of(vs));
                        e.array_stride = stride * nv as i32;
                        e.matrix_stride = stride;
                        e.row_major = rm;
                        *off += nv as i32 * *n as i32 * stride;
                    } else {
                        e.array_stride = array_stride_of(*t);
                        *off += e.array_stride * *n as i32;
                    }
                    layout.uniforms.push(e);
                }
                other => {
                    for k in 0..*n {
                        std140(layout, off, blk, &format!("{prefix}[{k}]"), other, flags);
                    }
                }
            },
            Ty::Struct(s, own) => {
                if own & MATRIX_MASK != 0 {
                    flags = flags & !MATRIX_MASK | own & MATRIX_MASK;
                }
                for (name, mty, _) in &s.members {
                    std140(layout, off, blk, &format!("{prefix}.{name}"), mty, flags);
                }
                *off = round_up(*off, al);
            }
        }
    }

    fn std140_layout(iface: &Iface) -> Layout {
        let mut l = Layout::default();
        for blk in &iface.blocks {
            let prefix = if blk.instance.is_some() { format!("{}.", blk.name) } else { String::new() };
            let mut off = 0;
            let bi = l.blocks.len() as i32;
            let first = l.uniforms.len();
            for un in &blk.uniforms {
                let name = format!("{prefix}{}", un.name);
                std140(&mut l, &mut off, bi, &name, &un.ty, merge_flags(blk.flags, un.flags));
            }
            let end = l.uniforms.len();
            for k in 0..blk.array.max(1) {
                let name = if blk.array > 0 { format!("{}[{k}]", blk.name) } else { blk.name.into() };
                l.blocks.push(BlockEntry { name, size: off, active: (first as i32..end as i32).collect() });
            }
        }
        l
    }

    /* ---- values (generateValues; dEQP's own generator, not its numbers) -- */

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        fn int(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u32) as i32
        }
    }

    fn generate(layout: &Layout, data: &mut [Vec<u8>], seed: u64) {
        let mut rnd = Rng(seed * 2 + 0x9e37_79b9_7f4a_7c15);
        for (bi, blk) in layout.blocks.iter().enumerate() {
            for &k in &blk.active {
                let e = &layout.uniforms[k as usize];
                let nv = match (e.ty.is_mat(), e.row_major) {
                    (false, _) => 1,
                    (true, true) => e.ty.rows,
                    (true, false) => e.ty.cols,
                };
                let vs = e.ty.scalars() / nv;
                for el in 0..e.size {
                    for v in 0..nv {
                        let vec_at = (e.offset + el * e.array_stride + v as i32 * e.matrix_stride) as usize;
                        for c in 0..vs {
                            let at = vec_at + 4 * c as usize;
                            /* random bit patterns for true (dEQP's note) */
                            let truth = rnd.next() & 1 != 0;
                            let w: u32 = match e.ty.sc {
                                Scalar::F => (rnd.int(-9, 9) as f32).to_bits(),
                                Scalar::I => rnd.int(-9, 9) as u32,
                                Scalar::U => rnd.int(0, 9) as u32,
                                Scalar::B if truth => rnd.next() | 1,
                                Scalar::B => 0,
                            };
                            data[bi][at..at + 4].copy_from_slice(&w.to_le_bytes());
                        }
                    }
                }
            }
        }
    }

    /// Component (element, column, row) of entry `e` in `data`.
    fn comp(e: &Entry, data: &[u8], el: i32, c: u32, r: u32) -> u32 {
        let base = e.offset + el * e.array_stride;
        let at = (if e.ty.is_mat() {
            if e.row_major {
                base + r as i32 * e.matrix_stride + c as i32 * 4
            } else {
                base + c as i32 * e.matrix_stride + r as i32 * 4
            }
        } else {
            base + (c * e.ty.rows + r) as i32 * 4
        }) as usize;
        u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
    }

    /* ---- the shaders (generateVertexShader / generateFragmentShader) ----- */

    fn layout_fmt(f: u32) -> String {
        let names = [
            (SHARED, "shared"),
            (PACKED, "packed"),
            (STD140, "std140"),
            (ROW_MAJOR, "row_major"),
            (COLUMN_MAJOR, "column_major"),
        ];
        names.iter().filter(|(bit, _)| f & bit != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(", ")
    }

    fn prec_fmt(f: u32) -> &'static str {
        match f & PREC_MASK {
            PREC_LOW => "lowp",
            PREC_MED => "mediump",
            _ => "highp",
        }
    }

    fn decl(src: &mut String, ty: &Ty, name: &str) {
        let f = ty.flags();
        if f & LAYOUT_MASK != 0 {
            src.push_str(&format!("layout({}) ", layout_fmt(f & LAYOUT_MASK)));
        }
        if f & PREC_MASK != 0 {
            src.push_str(&format!("{} ", prec_fmt(f)));
        }
        match ty {
            Ty::Basic(t, _) => src.push_str(&format!("{} {name}", t.glsl)),
            Ty::Array(..) => {
                let mut sizes = Vec::new();
                let mut cur = ty;
                while let Ty::Array(e, n) = cur {
                    sizes.push(*n);
                    cur = &**e;
                }
                match cur {
                    Ty::Basic(t, ef) => {
                        if ef & LAYOUT_MASK != 0 {
                            src.push_str(&format!("layout({}) ", layout_fmt(ef & LAYOUT_MASK)));
                        }
                        if ef & PREC_MASK != 0 {
                            src.push_str(&format!("{} ", prec_fmt(*ef)));
                        }
                        src.push_str(t.glsl);
                    }
                    Ty::Struct(s, _) => src.push_str(s.name),
                    Ty::Array(..) => unreachable!(),
                }
                src.push_str(&format!(" {name}"));
                for n in sizes {
                    src.push_str(&format!("[{n}]"));
                }
            }
            Ty::Struct(s, _) => src.push_str(&format!("{} {name}", s.name)),
        }
        src.push_str(";\n");
    }

    fn compare_funcs(iface: &Iface) -> String {
        fn collect(ty: &Ty, out: &mut Vec<Basic>) {
            match ty {
                Ty::Basic(t, _) => out.push(*t),
                Ty::Array(e, _) => collect(e, out),
                Ty::Struct(s, _) => s.members.iter().for_each(|m| collect(&m.1, out)),
            }
        }
        let mut types = Vec::new();
        for blk in &iface.blocks {
            for un in &blk.uniforms {
                collect(&un.ty, &mut types);
            }
        }
        let mut need: Vec<Basic> = Vec::new();
        for t in types {
            if t.sc == Scalar::F && t.scalars() > 1 {
                need.push(basic("float"));
                if t.is_mat() {
                    need.push(vec_of(t.rows));
                }
            }
            need.push(t);
        }
        let mut s = String::new();
        for t in BASICS {
            if !need.contains(&t) {
                continue;
            }
            let body = match (t.sc, t.cols, t.rows) {
                (Scalar::F, 1, 1) => "return abs(a - b) < 0.05 ? 1.0 : 0.0;".to_string(),
                (Scalar::F, 1, n) => (0..n)
                    .map(|k| format!("compare_float(a.{0}, b.{0})", ["x", "y", "z", "w"][k as usize]))
                    .collect::<Vec<_>>()
                    .join("*"),
                (Scalar::F, c, r) => {
                    (0..c).map(|k| format!("compare_{}(a[{k}], b[{k}])", vec_of(r).glsl)).collect::<Vec<_>>().join("*")
                }
                _ => "return a == b ? 1.0 : 0.0;".to_string(),
            };
            let body = if body.starts_with("return") { body } else { format!("return {body};") };
            let p = if t.sc == Scalar::B { "" } else { "highp " };
            s.push_str(&format!("mediump float compare_{0}({p}{0} a, {p}{0} b) {{ {body} }}\n", t.glsl));
        }
        s
    }

    fn value_src(e: &Entry, data: &[u8], el: i32) -> String {
        let lit = |w: u32| -> String {
            match e.ty.sc {
                Scalar::F => format!("{:.1}", f32::from_bits(w)),
                Scalar::I => format!("{}", w as i32),
                Scalar::U => format!("{w}u"),
                Scalar::B => (if w != 0 { "true" } else { "false" }).into(),
            }
        };
        /* constructed column by column */
        let at = (0..e.ty.cols).flat_map(|c| (0..e.ty.rows).map(move |r| (c, r)));
        let vals: Vec<String> = at.map(|(c, r)| lit(comp(e, data, el, c, r))).collect();
        if vals.len() > 1 {
            format!("{}({})", e.ty.glsl, vals.join(", "))
        } else {
            vals.concat()
        }
    }

    fn compare_src(src: &mut String, ty: &Ty, src_name: &str, api: &str, layout: &Layout, data: &[u8], unused: u32) {
        match ty {
            Ty::Basic(t, _) => {
                let e = &layout.uniforms[layout.uniform(api).unwrap_or_else(|| panic!("{api} in the reference"))];
                src.push_str(&format!("\tresult *= compare_{}({src_name}, {});\n", t.glsl, value_src(e, data, 0)));
            }
            Ty::Array(el, n) => {
                if let Ty::Basic(t, _) = &**el {
                    let e = &layout.uniforms[layout.uniform(&format!("{api}[0]")).expect("in the reference")];
                    for k in 0..*n {
                        src.push_str(&format!(
                            "\tresult *= compare_{}({src_name}[{k}], {});\n",
                            t.glsl,
                            value_src(e, data, k as i32)
                        ));
                    }
                } else {
                    for k in 0..*n {
                        let (s, a) = (format!("{src_name}[{k}]"), format!("{api}[{k}]"));
                        compare_src(src, el, &s, &a, layout, data, unused);
                    }
                }
            }
            Ty::Struct(s, _) => {
                for (name, mty, mflags) in &s.members {
                    if mflags & unused != 0 {
                        continue;
                    }
                    let (s, a) = (format!("{src_name}.{name}"), format!("{api}.{name}"));
                    compare_src(src, mty, &s, &a, layout, data, unused);
                }
            }
        }
    }

    fn declarations(iface: &Iface, stage: u32) -> String {
        let mut s = String::new();
        for st in &iface.structs {
            s.push_str(&format!("struct {}\n{{\n", st.name));
            for (name, ty, _) in &st.members {
                s.push('\t');
                decl(&mut s, ty, name);
            }
            s.push_str("};\n");
        }
        for blk in iface.blocks.iter().filter(|b| b.flags & stage != 0) {
            if blk.flags & LAYOUT_MASK != 0 {
                s.push_str(&format!("layout({}) ", layout_fmt(blk.flags & LAYOUT_MASK)));
            }
            s.push_str(&format!("uniform {}\n{{\n", blk.name));
            for un in &blk.uniforms {
                s.push('\t');
                if un.flags & LAYOUT_MASK != 0 {
                    s.push_str(&format!("layout({}) ", layout_fmt(un.flags & LAYOUT_MASK)));
                }
                decl(&mut s, &un.ty, un.name);
            }
            s.push('}');
            if let Some(inst) = blk.instance {
                s.push_str(&format!(" {inst}"));
                if blk.array > 0 {
                    s.push_str(&format!("[{}]", blk.array));
                }
            }
            s.push_str(";\n");
        }
        s.push('\n');
        s.push_str(&compare_funcs(iface));
        s
    }

    fn compares(iface: &Iface, layout: &Layout, data: &[Vec<u8>], vertex: bool) -> String {
        let (decl_bit, unused) = if vertex { (DECL_VERTEX, UNUSED_VERTEX) } else { (DECL_FRAGMENT, UNUSED_FRAGMENT) };
        let mut s = String::new();
        for blk in iface.blocks.iter().filter(|b| b.flags & decl_bit != 0) {
            let api = if blk.instance.is_some() { format!("{}.", blk.name) } else { String::new() };
            for k in 0..blk.array.max(1) {
                let post = if blk.array > 0 { format!("[{k}]") } else { String::new() };
                let bi = layout.block(&format!("{}{post}", blk.name)).expect("the block");
                let srcp = blk.instance.map_or(String::new(), |i| format!("{i}{post}."));
                for un in blk.uniforms.iter().filter(|u| u.flags & unused == 0) {
                    let (name, api_name) = (format!("{srcp}{}", un.name), format!("{api}{}", un.name));
                    compare_src(&mut s, &un.ty, &name, &api_name, layout, &data[bi], unused);
                }
            }
        }
        s
    }

    fn vertex_shader(iface: &Iface, layout: &Layout, data: &[Vec<u8>]) -> String {
        format!(
            "#version 300 es\nin highp vec4 a_position;\nout mediump float v_vtxResult;\n\n{}\n\
             void main (void)\n{{\n    gl_Position = a_position;\n    mediump float result = 1.0;\n\
             {}    v_vtxResult = result;\n}}\n",
            declarations(iface, DECL_VERTEX),
            compares(iface, layout, data, true)
        )
    }

    fn fragment_shader(iface: &Iface, layout: &Layout, data: &[Vec<u8>]) -> String {
        format!(
            "#version 300 es\nin mediump float v_vtxResult;\n\
             layout(location = 0) out mediump vec4 dEQP_FragColor;\n\n{}\n\
             void main (void)\n{{\n    mediump float result = 1.0;\n\
             {}    dEQP_FragColor = vec4(1.0, v_vtxResult, result, 1.0);\n}}\n",
            declarations(iface, DECL_FRAGMENT),
            compares(iface, layout, data, false)
        )
    }

    /* ---- the GL side (getGLUniformLayout and the checks of iterate) ------ */

    fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
        let c = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [c.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            if ok == 1 {
                return Ok(s);
            }
            let mut log = [0 as c_char; 1024];
            let mut n: GLsizei = 0;
            glGetShaderInfoLog(s, 1024, &mut n, log.as_mut_ptr());
            glDeleteShader(s);
            Err(core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned())
        }
    }

    fn gl_layout(p: GLuint) -> Result<Layout, String> {
        let mut l = Layout::default();
        // SAFETY: plain GL calls; every out pointer has room for what is asked.
        unsafe {
            let (mut nu, mut nb): (GLint, GLint) = (0, 0);
            glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut nu);
            glGetProgramiv(p, GL_ACTIVE_UNIFORM_BLOCKS, &mut nb);
            for k in 0..nb as u32 {
                let biv = |pname| {
                    let mut x: GLint = -1;
                    glGetActiveUniformBlockiv(p, k, pname, &mut x);
                    x
                };
                let size = biv(GL_UNIFORM_BLOCK_DATA_SIZE);
                let len = biv(GL_UNIFORM_BLOCK_NAME_LENGTH);
                let n = biv(GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS);
                let mut buf = vec![0 as c_char; len.max(1) as usize];
                glGetActiveUniformBlockName(p, k, buf.len() as GLsizei, core::ptr::null_mut(), buf.as_mut_ptr());
                let mut active = vec![-1; n.max(0) as usize];
                if n > 0 {
                    glGetActiveUniformBlockiv(p, k, GL_UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES, active.as_mut_ptr());
                }
                let name = core::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                l.blocks.push(BlockEntry { name, size, active });
            }
            let idx: Vec<u32> = (0..nu as u32).collect();
            let q = |pname| {
                let mut v = vec![-1; nu as usize];
                if nu > 0 {
                    glGetActiveUniformsiv(p, nu, idx.as_ptr(), pname, v.as_mut_ptr());
                }
                v
            };
            let (ty, size, nl, bi, off, ast, mst, rm) = (
                q(GL_UNIFORM_TYPE),
                q(GL_UNIFORM_SIZE),
                q(GL_UNIFORM_NAME_LENGTH),
                q(GL_UNIFORM_BLOCK_INDEX),
                q(GL_UNIFORM_OFFSET),
                q(GL_UNIFORM_ARRAY_STRIDE),
                q(GL_UNIFORM_MATRIX_STRIDE),
                q(GL_UNIFORM_IS_ROW_MAJOR),
            );
            if err() != GL_NO_ERROR {
                return Err("Active uniform query failed".into());
            }
            for k in 0..nu as usize {
                let mut buf = vec![0 as c_char; nl[k].max(1) as usize];
                let (mut len, mut sz, mut t): (GLsizei, GLint, GLenum) = (0, 0, 0);
                glGetActiveUniform(p, k as u32, buf.len() as GLsizei, &mut len, &mut sz, &mut t, buf.as_mut_ptr());
                if len + 1 != nl[k] || sz != size[k] || t != ty[k] as u32 {
                    return Err("glGetActiveUniform() and glGetActiveUniformsiv() differ".into());
                }
                let name = core::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                let Some(tb) = BASICS.iter().find(|b| b.gl == t) else {
                    return Err(format!("{name}: type {t:#x}"));
                };
                l.uniforms.push(Entry {
                    name,
                    ty: *tb,
                    size: size[k],
                    block: bi[k],
                    offset: off[k],
                    array_stride: ast[k],
                    matrix_stride: mst[k],
                    row_major: rm[k] != 0,
                });
            }
        }
        Ok(l)
    }

    /// The checks `UniformBlockCase::iterate` makes of the queried layout
    /// before it draws; the first message, as dEQP's result would name it.
    fn check_layout(iface: &Iface, r: &Layout, g: &Layout, p: GLuint) -> Result<(), String> {
        /* checkLayoutIndices */
        for e in &g.uniforms {
            if e.block < 0 || e.block as usize >= g.blocks.len() {
                return Err(format!("Invalid layout: block index of {}", e.name));
            }
        }
        for blk in &g.blocks {
            if blk.active.iter().any(|&k| k < 0 || k as usize >= g.uniforms.len()) {
                return Err(format!("Invalid layout: active uniform index in {}", blk.name));
            }
        }
        /* checkLayoutBounds */
        for e in &g.uniforms {
            let blk = &g.blocks[e.block as usize];
            let (nv, nc) = if e.ty.is_mat() {
                if e.row_major {
                    (e.ty.rows, e.ty.cols)
                } else {
                    (e.ty.cols, e.ty.rows)
                }
            } else {
                (1, e.ty.scalars())
            };
            let vec = nc as i32 * 4;
            let (mat, arr) = ((nv as i32 - 1) * e.matrix_stride, (e.size - 1) * e.array_stride);
            let max = vec.max(mat + vec).max(arr + vec).max(arr + mat + vec);
            if e.offset < 0 || e.offset + max > blk.size {
                return Err(format!("Invalid layout: {} out of block bounds", e.name));
            }
        }
        for blk in &iface.blocks {
            for k in 0..blk.array.max(1) {
                let name = if blk.array > 0 { format!("{}[{k}]", blk.name) } else { blk.name.into() };
                let Some(gb) = g.block(&name) else { continue };
                /* compareTypes */
                for &k in &g.blocks[gb].active {
                    let ge = &g.uniforms[k as usize];
                    match r.uniform(&ge.name) {
                        None => return Err(format!("Invalid layout: '{}' not in the reference", ge.name)),
                        Some(re) if r.uniforms[re].ty != ge.ty => {
                            return Err(format!("Invalid layout: type of '{}'", ge.name))
                        }
                        _ => {}
                    }
                }
            }
            /* compareStd140Blocks / compareSharedBlocks */
            let name = if blk.array > 0 { format!("{}[0]", blk.name) } else { blk.name.into() };
            if blk.flags & (STD140 | SHARED) == 0 {
                continue;
            }
            let rb = r.block(&name).expect("the reference block");
            let Some(gb) = g.block(&name) else { return Err(format!("block '{name}' not found")) };
            if r.blocks[rb].active.len() != g.blocks[gb].active.len() {
                return Err(format!(
                    "{name}: {} active uniforms, expected {}",
                    g.blocks[gb].active.len(),
                    r.blocks[rb].active.len()
                ));
            }
            for &k in &r.blocks[rb].active {
                let re = &r.uniforms[k as usize];
                let Some(gi) = g.uniform(&re.name) else { return Err(format!("Uniform '{}' not found", re.name)) };
                let ge = &g.uniforms[gi];
                let same = if blk.flags & STD140 != 0 {
                    (re.ty, re.size, re.offset, re.array_stride, re.matrix_stride, re.row_major)
                        == (ge.ty, ge.size, ge.offset, ge.array_stride, ge.matrix_stride, ge.row_major)
                } else {
                    (re.ty, re.size, re.row_major) == (ge.ty, ge.size, ge.row_major)
                };
                if !same {
                    return Err(format!("Layout mismatch in '{}': expected {re:?}, got {ge:?}", re.name));
                }
            }
        }
        /* checkIndexQueries */
        for (k, blk) in g.blocks.iter().enumerate() {
            let c = CString::new(blk.name.as_str()).unwrap();
            // SAFETY: a NUL-terminated name.
            let q = unsafe { glGetUniformBlockIndex(p, c.as_ptr()) };
            if q != k as u32 {
                return Err(format!("glGetUniformBlockIndex({}) = {q}, expected {k}", blk.name));
            }
        }
        Ok(())
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        PerBlock,
        Single,
    }

    /// copyUniformData: every reference entry into the queried layout.
    fn copy_data(g: &Layout, gdata: &mut [Vec<u8>], r: &Layout, rdata: &[Vec<u8>]) {
        for (rb, blk) in r.blocks.iter().enumerate() {
            let Some(gb) = g.block(&blk.name) else { continue };
            for &k in &blk.active {
                let re = &r.uniforms[k as usize];
                let Some(gi) = g.uniform(&re.name) else { continue };
                let ge = &g.uniforms[gi];
                for el in 0..ge.size {
                    for c in 0..re.ty.cols {
                        for row in 0..re.ty.rows {
                            let w = comp(re, &rdata[rb], el, c, row);
                            let base = ge.offset + el * ge.array_stride;
                            let at = (if ge.ty.is_mat() {
                                if ge.row_major {
                                    base + row as i32 * ge.matrix_stride + c as i32 * 4
                                } else {
                                    base + c as i32 * ge.matrix_stride + row as i32 * 4
                                }
                            } else {
                                base + row as i32 * 4
                            }) as usize;
                            gdata[gb][at..at + 4].copy_from_slice(&w.to_le_bytes());
                        }
                    }
                }
            }
        }
    }

    /// The default-block row the compiled stage reads `row` from: the
    /// words of element `el`, column `c`, row `r` (a column strided as the
    /// online reflection lays rows out: 2 dwords for 2 rows, else 4).
    fn row_word(st: &Stage, row: &str, t: Basic, el: i32, c: u32, r: u32) -> Result<u32, String> {
        let Some(&(_, off, dw, n)) = st.rows.iter().find(|x| x.0 == row) else {
            return Err(format!("no row '{row}' in the compiled stage"));
        };
        let el_dw = u32::from(dw) / u32::from(n.max(1));
        let col_dw = match (t.is_mat(), t.rows) {
            (false, _) => 0,
            (true, 2) => 2,
            (true, _) => 4,
        };
        let at = u32::from(off) + el as u32 * el_dw + c * col_dw + r;
        st.block.get(at as usize).copied().ok_or(format!("'{row}' word {at} past the block of {}", st.block.len()))
    }

    /// What the draw handed the back end for one stage, against the values
    /// the generated shader compares with. `Err` names the first wrong word.
    fn check_rows(
        st: &Stage,
        ty: &Ty,
        row: &str,
        api: &str,
        r: &Layout,
        rdata: &[u8],
        unused: u32,
    ) -> Result<(), String> {
        let leaf = |t: &Basic, api_name: &str, n: u32| -> Result<(), String> {
            let e = &r.uniforms[r.uniform(api_name).expect("in the reference")];
            for el in 0..n as i32 {
                for c in 0..t.cols {
                    for rr in 0..t.rows {
                        let want = comp(e, rdata, el, c, rr);
                        let want = if t.sc == Scalar::B { u32::from(want != 0) } else { want };
                        let got = row_word(st, row, *t, el, c, rr)?;
                        let ok = if t.sc == Scalar::B { (got != 0) == (want != 0) } else { got == want };
                        if !ok {
                            return Err(format!(
                                "'{row}' element {el} column {c} row {rr}: {got:#x}, the shader compares with {want:#x}"
                            ));
                        }
                    }
                }
            }
            Ok(())
        };
        match ty {
            Ty::Basic(t, _) => leaf(t, api, 1),
            Ty::Array(el, n) => match &**el {
                Ty::Basic(t, _) => leaf(t, &format!("{api}[0]"), *n),
                other => (0..*n).try_for_each(|k| {
                    check_rows(st, other, &format!("{row}[{k}]"), &format!("{api}[{k}]"), r, rdata, unused)
                }),
            },
            Ty::Struct(s, _) => s.members.iter().filter(|m| m.2 & unused == 0).try_for_each(|(name, mty, _)| {
                check_rows(st, mty, &format!("{row}.{name}"), &format!("{api}.{name}"), r, rdata, unused)
            }),
        }
    }

    /// One case end to end; `Err` is the first thing that would fail it.
    fn run_case(iface: &Iface, mode: Mode) -> Result<(), String> {
        let r = std140_layout(iface);
        let mut rdata: Vec<Vec<u8>> = r.blocks.iter().map(|b| vec![0u8; b.size as usize]).collect();
        generate(&r, &mut rdata, 1);
        let vs_src = vertex_shader(iface, &r, &rdata);
        let fs_src = fragment_shader(iface, &r, &rdata);
        let vs = shader(GL_VERTEX_SHADER, &vs_src).map_err(|e| format!("Compile failed (vertex): {e}\n{vs_src}"))?;
        let fs =
            shader(GL_FRAGMENT_SHADER, &fs_src).map_err(|e| format!("Compile failed (fragment): {e}\n{fs_src}"))?;
        // SAFETY: plain GL calls on handles this case owns.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, vs);
            glAttachShader(p, fs);
            glLinkProgram(p);
            glDeleteShader(vs);
            glDeleteShader(fs);
            let res = run_program(iface, mode, &r, &rdata, p);
            glUseProgram(0);
            glDeleteProgram(p);
            while glGetError() != GL_NO_ERROR {}
            res
        }
    }

    /// [`run_case`] from the link on: the layout, the upload, the draw and
    /// the rows it handed over.
    ///
    /// # Safety
    /// `p` is a program of the current context; plain GL calls whose every
    /// pointer outlives the call.
    unsafe fn run_program(iface: &Iface, mode: Mode, r: &Layout, rdata: &[Vec<u8>], p: GLuint) -> Result<(), String> {
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            return Err(format!("Link failed: {}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy()));
        }
        let g = gl_layout(p)?;
        check_layout(iface, r, &g, p).map_err(|e| format!("{e}\n{g:#?}"))?;
        glUseProgram(p);
        for k in 0..g.blocks.len() as u32 {
            glUniformBlockBinding(p, k, k);
        }
        if err() != GL_NO_ERROR {
            return Err("Failed to set uniform block bindings".into());
        }
        let mut gdata: Vec<Vec<u8>> = g.blocks.iter().map(|b| vec![0u8; b.size as usize]).collect();
        copy_data(&g, &mut gdata, r, rdata);
        let mut bufs = vec![0u32; g.blocks.len().max(1)];
        glGenBuffers(bufs.len() as GLsizei, bufs.as_mut_ptr());
        if mode == Mode::PerBlock {
            for (k, d) in gdata.iter().enumerate() {
                glBindBuffer(GL_UNIFORM_BUFFER, bufs[k]);
                glBufferData(GL_UNIFORM_BUFFER, d.len() as GLsizeiptr, d.as_ptr() as *const c_void, GL_STATIC_DRAW);
                glBindBufferBase(GL_UNIFORM_BUFFER, k as u32, bufs[k]);
            }
        } else {
            let mut align: GLint = 0;
            glGetIntegerv(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT, &mut align);
            let mut all = Vec::new();
            let mut offs = Vec::new();
            for d in &gdata {
                if align > 0 {
                    all.resize(round_up(all.len() as i32, align) as usize, 0u8);
                }
                offs.push(all.len());
                all.extend_from_slice(d);
            }
            glBindBuffer(GL_UNIFORM_BUFFER, bufs[0]);
            glBufferData(GL_UNIFORM_BUFFER, all.len() as GLsizeiptr, all.as_ptr() as *const c_void, GL_STATIC_DRAW);
            for (k, d) in gdata.iter().enumerate() {
                glBindBufferRange(GL_UNIFORM_BUFFER, k as u32, bufs[0], offs[k] as GLintptr, d.len() as GLsizeiptr);
            }
        }
        if err() != GL_NO_ERROR {
            return Err("uniform buffer upload or binding failed".into());
        }
        static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
        static IDX: [u16; 6] = [0, 1, 2, 2, 1, 3];
        let loc = glGetAttribLocation(p, c"a_position".as_ptr());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(loc as u32);
        *DRAWN.lock().unwrap() = None;
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        glDisableVertexAttribArray(loc as u32);
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        if err() != GL_NO_ERROR {
            return Err("Draw failed".into());
        }
        let Some((vst, fst)) = DRAWN.lock().unwrap().take() else {
            return Err("no draw reached the back end".into());
        };
        for (st, decl_bit, unused) in [(&vst, DECL_VERTEX, UNUSED_VERTEX), (&fst, DECL_FRAGMENT, UNUSED_FRAGMENT)] {
            for blk in iface.blocks.iter().filter(|b| b.flags & decl_bit != 0) {
                let api = if blk.instance.is_some() { format!("{}.", blk.name) } else { String::new() };
                for k in 0..blk.array.max(1) {
                    let post = if blk.array > 0 { format!("[{k}]") } else { String::new() };
                    let rb = r.block(&format!("{}{post}", blk.name)).expect("the block");
                    let rowp = blk.instance.map_or(String::new(), |i| format!("{i}{post}."));
                    for un in blk.uniforms.iter().filter(|u| u.flags & unused == 0) {
                        let (row, api) = (format!("{rowp}{}", un.name), format!("{api}{}", un.name));
                        check_rows(st, &un.ty, &row, &api, r, &rdata[rb], unused)
                            .map_err(|e| format!("Image compare failed (stage {decl_bit:#x}): {e}"))?;
                    }
                }
            }
        }
        Ok(())
    }

    /* ---- the cases of es3fUniformBlockTests.cpp ---------------------------- */

    /// A case-name part and the flags it stands for.
    type Choice = (&'static str, u32);

    const PRECISIONS: [Choice; 3] = [("lowp", PREC_LOW), ("mediump", PREC_MED), ("highp", PREC_HIGH)];
    const LAYOUTS: [Choice; 3] = [("shared", SHARED), ("packed", PACKED), ("std140", STD140)];
    const MATRICES: [Choice; 2] = [("row_major", ROW_MAJOR), ("column_major", COLUMN_MAJOR)];
    const MODES: [(&str, Mode); 2] = [("per_block_buffer", Mode::PerBlock), ("single_buffer", Mode::Single)];

    /// BlockBasicTypeCase.
    fn basic_type_case(ty: Ty, flags: u32, instances: u32) -> Iface {
        let mut blk = Block { name: "Block", instance: None, array: 0, flags, uniforms: Vec::new() };
        blk.uniforms.push(u("var", ty, 0));
        if instances > 0 {
            blk.instance = Some("block");
            blk.array = instances;
        }
        Iface { structs: Vec::new(), blocks: vec![blk] }
    }

    const STAGES: [Choice; 3] =
        [("vertex", DECL_VERTEX), ("fragment", DECL_FRAGMENT), ("both", DECL_VERTEX | DECL_FRAGMENT)];

    /// The `_vertex`, `_fragment` and (but for `packed`) `_both` cases of
    /// one interface, as es3fUniformBlockTests.cpp adds them.
    fn staged(out: &mut Vec<(String, Iface, Mode)>, base: &str, layout: u32, mode: Mode, mk: impl Fn(u32) -> Iface) {
        for (stage, bits) in STAGES {
            if stage == "both" && layout & PACKED != 0 {
                continue;
            }
            out.push((format!("{base}_{stage}"), mk(bits), mode));
        }
    }

    fn inst_suffix(inst: u32) -> &'static str {
        if inst > 0 {
            "_instance_array"
        } else {
            ""
        }
    }

    /// createBlockBasicTypeCases.
    fn basic_type_cases(out: &mut Vec<(String, Iface, Mode)>, name: &str, ty: Ty, flags: u32, instances: u32) {
        staged(out, name, flags, Mode::PerBlock, |bits| basic_type_case(ty.clone(), flags | bits, instances));
    }

    fn struct_s_t(kind: &str) -> (Rc<Struct>, Option<Rc<Struct>>) {
        match kind {
            "single_struct" | "single_struct_array" => (
                Rc::new(Struct {
                    name: "S",
                    members: vec![
                        ("a", bt("ivec3", PREC_HIGH), UNUSED_BOTH),
                        ("b", arr(bt("mat3", PREC_MED), 4), 0),
                        ("c", bt("vec4", PREC_HIGH), 0),
                    ],
                }),
                None,
            ),
            "single_nested_struct" | "mixed" => {
                let s = Rc::new(Struct {
                    name: "S",
                    members: vec![
                        ("a", bt("ivec3", PREC_HIGH), 0),
                        ("b", arr(bt("mat3", PREC_MED), 4), 0),
                        ("c", bt("vec4", PREC_HIGH), UNUSED_BOTH),
                    ],
                });
                let t = Rc::new(Struct {
                    name: "T",
                    members: vec![("a", bt("mat3", PREC_MED), 0), ("b", Ty::Struct(s.clone(), 0), 0)],
                });
                (s, Some(t))
            }
            _ => {
                let s = Rc::new(Struct {
                    name: "S",
                    members: vec![
                        ("a", bt("ivec3", PREC_HIGH), 0),
                        ("b", arr(bt("ivec2", PREC_MED), 4), 0),
                        ("c", bt("vec4", PREC_HIGH), UNUSED_BOTH),
                    ],
                });
                let t = Rc::new(Struct {
                    name: "T",
                    members: vec![("a", bt("mat3", PREC_MED), 0), ("b", arr(Ty::Struct(s.clone(), 0), 3), 0)],
                });
                (s, Some(t))
            }
        }
    }

    /// BlockSingleStructCase, ...StructArray, ...NestedStruct,
    /// ...NestedStructArray and ...NestedStructMixedMatrixPacking (`m`, `ma`:
    /// the matrix flags of `t` and `s`).
    fn struct_case(kind: &str, flags: u32, instances: u32, m: u32, ma: u32) -> Iface {
        let (s, t) = struct_s_t(kind);
        let mut blk = Block { name: "Block", instance: None, array: 0, flags, uniforms: Vec::new() };
        match kind {
            "single_struct" => blk.uniforms.push(u("s", Ty::Struct(s.clone(), 0), 0)),
            "single_struct_array" => {
                blk.uniforms.push(u("u", bt("uint", PREC_LOW), 0));
                blk.uniforms.push(u("s", arr(Ty::Struct(s.clone(), 0), 3), 0));
                blk.uniforms.push(u("v", bt("vec4", PREC_MED), 0));
            }
            "single_nested_struct" | "single_nested_struct_array" | "mixed" => {
                let t = t.clone().unwrap();
                let (sm, tm) = if kind == "mixed" { (ma, m) } else { (0, 0) };
                blk.uniforms.push(u("s", Ty::Struct(s.clone(), sm), 0));
                blk.uniforms.push(u("v", bt("vec2", PREC_LOW), UNUSED_BOTH));
                if kind == "single_nested_struct_array" {
                    blk.uniforms.push(u("t", arr(Ty::Struct(t, 0), 2), 0));
                } else {
                    blk.uniforms.push(u("t", Ty::Struct(t, tm), 0));
                }
                blk.uniforms.push(u("u", bt("uint", PREC_HIGH), 0));
            }
            _ => unreachable!(),
        }
        if instances > 0 {
            blk.instance = Some("block");
            blk.array = instances;
        }
        let mut structs = vec![s];
        structs.extend(t);
        Iface { structs, blocks: vec![blk] }
    }

    #[test]
    fn single_basic_type_and_array() {
        let _g = context();
        let mut cases = Vec::new();
        for (ln, lf) in LAYOUTS {
            for t in BASICS {
                let tn = t.glsl;
                if t.sc == Scalar::B {
                    let name = format!("single_basic_type.{ln}.{tn}");
                    basic_type_cases(&mut cases, &name, Ty::Basic(t, 0), lf, 0);
                } else {
                    for (pn, pf) in PRECISIONS {
                        let name = format!("single_basic_type.{ln}.{pn}_{tn}");
                        basic_type_cases(&mut cases, &name, Ty::Basic(t, pf), lf, 0);
                    }
                }
                if t.is_mat() {
                    for (mn, mf) in MATRICES {
                        for (pn, pf) in PRECISIONS {
                            let name = format!("single_basic_type.{ln}.{mn}_{pn}_{tn}");
                            basic_type_cases(&mut cases, &name, Ty::Basic(t, pf), lf | mf, 0);
                        }
                    }
                }
                let ep = if t.sc == Scalar::B { 0 } else { PREC_HIGH };
                let name = format!("single_basic_array.{ln}.{tn}");
                basic_type_cases(&mut cases, &name, arr(Ty::Basic(t, ep), 3), lf, 0);
                if t.is_mat() {
                    for (mn, mf) in MATRICES {
                        let name = format!("single_basic_array.{ln}.{mn}_{tn}");
                        basic_type_cases(&mut cases, &name, arr(Ty::Basic(t, PREC_HIGH), 3), lf | mf, 0);
                    }
                }
            }
        }
        run_all(cases);
    }

    #[test]
    fn instance_array_basic_type() {
        let _g = context();
        let mut cases = Vec::new();
        for (ln, lf) in LAYOUTS {
            for t in BASICS {
                let ep = if t.sc == Scalar::B { 0 } else { PREC_HIGH };
                let name = format!("instance_array_basic_type.{ln}.{}", t.glsl);
                basic_type_cases(&mut cases, &name, Ty::Basic(t, ep), lf, 3);
                if t.is_mat() {
                    for (mn, mf) in MATRICES {
                        let name = format!("instance_array_basic_type.{ln}.{mn}_{}", t.glsl);
                        basic_type_cases(&mut cases, &name, Ty::Basic(t, PREC_HIGH), lf | mf, 3);
                    }
                }
            }
        }
        run_all(cases);
    }

    /// BlockMultiBasicTypesCase and BlockMultiNestedStructCase.
    fn multi_case(nested: bool, fa: u32, fb: u32, instances: u32) -> Iface {
        let mut structs = Vec::new();
        let (mut a, mut b) = (
            Block { name: "BlockA", instance: Some("blockA"), array: instances, flags: fa, uniforms: Vec::new() },
            Block { name: "BlockB", instance: Some("blockB"), array: instances, flags: fb, uniforms: Vec::new() },
        );
        if nested {
            let s = Rc::new(Struct {
                name: "S",
                members: vec![
                    ("a", bt("mat3", PREC_LOW), 0),
                    ("b", arr(bt("ivec2", PREC_MED), 4), 0),
                    ("c", bt("vec4", PREC_HIGH), 0),
                ],
            });
            let t = Rc::new(Struct {
                name: "T",
                members: vec![
                    ("a", bt("uint", PREC_MED), UNUSED_BOTH),
                    ("b", Ty::Struct(s.clone(), 0), 0),
                    ("c", bt("bvec4", 0), 0),
                ],
            });
            a.uniforms = vec![
                u("a", bt("float", PREC_HIGH), 0),
                u("b", Ty::Struct(s.clone(), 0), 0),
                u("c", bt("uvec3", PREC_LOW), UNUSED_BOTH),
            ];
            b.uniforms = vec![
                u("a", bt("mat2", PREC_MED), 0),
                u("b", Ty::Struct(t.clone(), 0), 0),
                u("c", bt("bvec4", 0), UNUSED_BOTH),
                u("d", bt("bool", 0), 0),
            ];
            structs = vec![s, t];
        } else {
            a.uniforms = vec![
                u("a", bt("float", PREC_HIGH), 0),
                u("b", bt("uvec3", PREC_LOW), UNUSED_BOTH),
                u("c", bt("mat2", PREC_MED), 0),
            ];
            b.uniforms = vec![
                u("a", bt("mat3", PREC_MED), 0),
                u("b", bt("ivec2", PREC_LOW), 0),
                u("c", bt("vec4", PREC_HIGH), UNUSED_BOTH),
                u("d", bt("bool", 0), 0),
            ];
        }
        Iface { structs, blocks: vec![a, b] }
    }

    #[test]
    fn multi_basic_types_and_nested_structs() {
        let _g = context();
        let mut cases = Vec::new();
        for (kind, nested) in [("multi_basic_types", false), ("multi_nested_struct", true)] {
            for (mn, mode) in MODES {
                for (ln, lf) in LAYOUTS {
                    for inst in [0u32, 3] {
                        let base = format!("{kind}.{mn}.{ln}{}", inst_suffix(inst));
                        /* the three staged cases declare both blocks alike */
                        staged(&mut cases, &base, lf, mode, |bits| multi_case(nested, lf | bits, lf | bits, inst));
                        let (a, b) = (lf | DECL_VERTEX, lf | DECL_FRAGMENT);
                        cases.push((format!("{base}_mixed"), multi_case(nested, a, b, inst), mode));
                    }
                }
            }
        }
        run_all(cases);
    }

    /// Cases a back end refuses at link for a limit of its own, with the
    /// log line that says so. The GC7000 copies a block into the constant
    /// registers, of which its two stages share 320 (src/vivante); these
    /// four need 184 + 184. Reading blocks from memory on that GPU is what
    /// lifts it. A case on the list that starts passing fails the test too,
    /// so the list cannot outlive the limit.
    #[cfg(feature = "gpu-vivante")]
    const KNOWN_LIMITS: [(&str, &str); 4] = [
        ("single_nested_struct_array.per_block_buffer.shared_instance_array_both", "exceed the 320 this GPU has"),
        ("single_nested_struct_array.per_block_buffer.std140_instance_array_both", "exceed the 320 this GPU has"),
        ("single_nested_struct_array.single_buffer.shared_instance_array_both", "exceed the 320 this GPU has"),
        ("single_nested_struct_array.single_buffer.std140_instance_array_both", "exceed the 320 this GPU has"),
    ];
    #[cfg(not(feature = "gpu-vivante"))]
    const KNOWN_LIMITS: [(&str, &str); 0] = [];

    fn run_all(cases: Vec<(String, Iface, Mode)>) {
        let n = cases.len();
        let mut failed = Vec::new();
        for (name, iface, mode) in cases {
            let known = KNOWN_LIMITS.iter().find(|(k, _)| *k == name).map(|(_, why)| *why);
            match (run_case(&iface, mode), known) {
                (Ok(()), None) => {}
                (Err(e), Some(why)) if e.contains(why) => {}
                (Ok(()), Some(_)) => failed.push(format!("{name}: passes; take it off KNOWN_LIMITS")),
                (Err(e), _) => failed.push(format!("{name}: {e}")),
            }
        }
        assert!(failed.is_empty(), "{} of {n} cases fail:\n{}", failed.len(), failed.join("\n\n"));
    }

    #[test]
    fn structs_and_nested_structs() {
        let _g = context();
        let mut cases = Vec::new();
        for kind in ["single_struct", "single_struct_array", "single_nested_struct", "single_nested_struct_array"] {
            for (mn, mode) in MODES {
                for (ln, lf) in LAYOUTS {
                    for inst in [0u32, 3] {
                        if mode == Mode::Single && inst == 0 {
                            continue;
                        }
                        let base = format!("{kind}.{mn}.{ln}{}", inst_suffix(inst));
                        staged(&mut cases, &base, lf, mode, |bits| struct_case(kind, lf | bits, inst, 0, 0));
                    }
                }
            }
        }
        run_all(cases);
    }

    /// `layout(row_major) S s;` and `layout(column_major) T t;`: a struct
    /// member's own matrix layout over the block's, the 192 cases of
    /// single_nested_struct_mixed_matrix_packing.
    #[test]
    fn nested_struct_mixed_matrix_packing() {
        let _g = context();
        let mut cases = Vec::new();
        for (mn, mode) in MODES {
            for (ln, lf) in LAYOUTS {
                for ((bn, bf), (sn, sf), (an, af)) in matrix_triples() {
                    for inst in [0u32, 3] {
                        if mode == Mode::Single && inst == 0 {
                            continue;
                        }
                        let what = format!("{ln}_block_{bn}_matrix_{sn}_matrixarray_{an}{}", inst_suffix(inst));
                        let base = format!("single_nested_struct_mixed_matrix_packing.{mn}.{what}");
                        staged(&mut cases, &base, lf, mode, |bits| struct_case("mixed", lf | bf | bits, inst, sf, af));
                    }
                }
            }
        }
        run_all(cases);
    }

    /// glGetUniformfv of every matrix type reads back, column-major, what
    /// glUniformMatrix*fv set, plain and transposed: dEQP-GLES3
    /// uniform_api.value.{initial,assigned}.*.get_uniform.basic.matCxR. The
    /// getter took a matrix's columns for its rows, so mat2x3, mat2x4,
    /// mat3x4 and mat4x3 read the wrong dwords (24 host dEQP cases; mat3x2
    /// and mat4x2 came out right only because their columns pack).
    ///
    /// Revert check: index the block by `i / mat`, `i % mat` in
    /// `program::get_uniform` and the non-square rows below fail.
    #[test]
    fn non_square_matrices_read_back_what_was_set() {
        type Setter = unsafe extern "C" fn(GLint, GLsizei, GLboolean, *const GLfloat);
        let _g = context();
        let mats: [(&str, u32, u32, Setter); 9] = [
            ("mat2", 2, 2, glUniformMatrix2fv),
            ("mat3", 3, 3, glUniformMatrix3fv),
            ("mat4", 4, 4, glUniformMatrix4fv),
            ("mat2x3", 2, 3, glUniformMatrix2x3fv),
            ("mat2x4", 2, 4, glUniformMatrix2x4fv),
            ("mat3x2", 3, 2, glUniformMatrix3x2fv),
            ("mat3x4", 3, 4, glUniformMatrix3x4fv),
            ("mat4x2", 4, 2, glUniformMatrix4x2fv),
            ("mat4x3", 4, 3, glUniformMatrix4x3fv),
        ];
        for stage in [GL_VERTEX_SHADER, GL_FRAGMENT_SHADER] {
            let decls: String = mats.iter().map(|(t, ..)| format!("uniform highp {t} u_{t};\n")).collect();
            let sum: String = mats.iter().map(|(t, ..)| format!(" + u_{t}[1][1]")).collect();
            let vs = if stage == GL_VERTEX_SHADER {
                format!("#version 300 es\n{decls}in vec4 a;\nvoid main() {{ gl_Position = a * (0.0{sum}); }}\n")
            } else {
                "#version 300 es\nin vec4 a;\nvoid main() { gl_Position = a; }\n".to_string()
            };
            let fs = if stage == GL_FRAGMENT_SHADER {
                format!("#version 300 es\nprecision highp float;\n{decls}out vec4 o;\nvoid main() {{ o = vec4(0.0{sum}); }}\n")
            } else {
                "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n".to_string()
            };
            // SAFETY: plain GL calls on handles this test owns; every
            // pointer outlives its call and holds what is asked.
            unsafe {
                let p = glCreateProgram();
                glAttachShader(p, shader(GL_VERTEX_SHADER, &vs).unwrap());
                glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs).unwrap());
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                assert_eq!(ok, 1);
                glUseProgram(p);
                for &(t, cols, rows, set) in &mats {
                    let name = CString::new(format!("u_{t}")).unwrap();
                    let loc = glGetUniformLocation(p, name.as_ptr());
                    assert!(loc >= 0, "{t}");
                    let n = (cols * rows) as usize;
                    let vals: Vec<f32> = (0..n).map(|k| k as f32 + 1.0).collect();
                    for transpose in [0u8, 1] {
                        set(loc, 1, transpose, vals.as_ptr());
                        assert_eq!(err(), GL_NO_ERROR, "{t}");
                        let mut back = vec![-1.0f32; n];
                        glGetUniformfv(p, loc, back.as_mut_ptr());
                        /* transposed input is row-major: value (c, r) is at r * cols + c */
                        let want: Vec<f32> = (0..n)
                            .map(|i| {
                                let (c, r) = (i as u32 / rows, i as u32 % rows);
                                vals[if transpose != 0 { (r * cols + c) as usize } else { i }]
                            })
                            .collect();
                        assert_eq!(back, want, "{t} transpose {transpose} stage {stage:#x}");
                    }
                }
                glUseProgram(0);
                glDeleteProgram(p);
            }
        }
    }

    /// Every (block, matrix, matrix array) choice of row_major and
    /// column_major, in dEQP's order.
    fn matrix_triples() -> Vec<(Choice, Choice, Choice)> {
        let mut v = Vec::new();
        for bm in MATRICES {
            for sm in MATRICES {
                for am in MATRICES {
                    v.push((bm, sm, am));
                }
            }
        }
        v
    }
}
