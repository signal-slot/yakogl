// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A recording OpenGL ES 3.0 that renders nothing.
//!
//! wgpu's GLES backend loads its entry points through `get_proc`. This answers
//! the queries the way YakoGL does on the Pi 4 (its version strings, and the
//! limits its GL core reports: src/gl's glGetIntegerv table), accepts every
//! shader and program, keeps the GLSL of every linked program, and counts per
//! frame what the draws ask for. Entry points it does not implement return a
//! stub that does nothing and returns 0: correct for every `void` call, and a
//! value-returning call that matters is implemented below.

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Mutex;

type GLenum = u32;
type GLuint = u32;
type GLint = i32;
type GLsizei = i32;

pub struct Program {
    pub vs: String,
    pub fs: String,
}

#[derive(Default, Clone)]
pub struct Frame {
    pub draws: u32,
    pub instanced: u32,
    pub max_instances: i32,
    pub vertices: u64,
    pub stencil_draws: u32,
    pub depth_draws: u32,
    pub blend_draws: u32,
    pub fbo_binds: u32,
    pub tex_uploads: u32,
    pub tex_bytes: u64,
    pub buf_uploads: u32,
    pub buf_bytes: u64,
    pub ubo_binds: u32,
    pub programs_used: std::collections::BTreeSet<u32>,
    pub tex_targets: std::collections::BTreeMap<u32, u32>,
    pub new_textures: Vec<(u32, u32, u32, u32, u32)>, // target, internal format, w, h, depth
}

#[derive(Default)]
struct State {
    next: GLuint,
    shader_src: HashMap<GLuint, (GLenum, String)>,
    prog_shaders: HashMap<GLuint, Vec<GLuint>>,
    pub programs: Vec<Program>,
    buffers: HashMap<GLuint, Vec<u8>>,
    bound: HashMap<GLenum, GLuint>,
    tex_bound: HashMap<(u32, GLenum), GLuint>,
    active_unit: u32,
    stencil: bool,
    depth: bool,
    blend: bool,
    program: GLuint,
    frame: Frame,
}

static ST: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = ST.lock().unwrap();
    f(g.get_or_insert_with(|| State { next: 1, ..Default::default() }))
}

pub fn take_frame() -> Frame {
    with(|s| std::mem::take(&mut s.frame))
}
pub fn take_programs() -> Vec<Program> {
    with(|s| std::mem::take(&mut s.programs))
}

// ---- GL enums used here (Khronos values) ----
const GL_VENDOR: GLenum = 0x1F00;
const GL_RENDERER: GLenum = 0x1F01;
const GL_VERSION: GLenum = 0x1F02;
const GL_EXTENSIONS: GLenum = 0x1F03;
const GL_SHADING_LANGUAGE_VERSION: GLenum = 0x8B8C;
const GL_STENCIL_TEST: GLenum = 0x0B90;
const GL_DEPTH_TEST: GLenum = 0x0B71;
const GL_BLEND: GLenum = 0x0BE2;
const GL_TEXTURE0: GLenum = 0x84C0;
const GL_VERTEX_SHADER: GLenum = 0x8B31;

/// GL_MAX_UNIFORM_BLOCK_SIZE: YakoGL's 6144 bytes (MAX_UNIFORM_DWORDS 1536 * 4),
/// unless BEVY_GL_REC_UBO_BYTES asks for another value. Bevy's view and
/// skinned-mesh bind groups need 16,320 and 16,384 bytes, so at 6144 wgpu
/// refuses them and no frame is drawn; the override measures the frame YakoGL
/// would get with a larger limit.
fn ubo_bytes() -> &'static [i64] {
    static V: std::sync::OnceLock<[i64; 1]> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        [std::env::var("BEVY_GL_REC_UBO_BYTES").ok().and_then(|v| v.parse().ok()).unwrap_or(1536 * 4)]
    })
}

/// YakoGL's answers (src/gl, glGetIntegerv; the named constants resolved at
/// origin/main 6eba4302). An enum not listed answers 0.
fn integer(pname: GLenum) -> &'static [i64] {
    match pname {
        0x0D33 => &[4096],                   // MAX_TEXTURE_SIZE
        0x8073 => &[256],                    // MAX_3D_TEXTURE_SIZE
        0x88FF => &[256],                    // MAX_ARRAY_TEXTURE_LAYERS
        0x851C => &[4096],                   // MAX_CUBE_MAP_TEXTURE_SIZE
        0x84E8 => &[4096],                   // MAX_RENDERBUFFER_SIZE
        0x0D3A => &[4096, 4096],             // MAX_VIEWPORT_DIMS
        0x8869 => &[16],                     // MAX_VERTEX_ATTRIBS
        0x8A30 => ubo_bytes(),               // MAX_UNIFORM_BLOCK_SIZE (see ubo_bytes)
        0x8A2B | 0x8A2D => &[12],            // MAX_VERTEX/FRAGMENT_UNIFORM_BLOCKS
        0x8A2E => &[24],                     // MAX_COMBINED_UNIFORM_BLOCKS
        0x8A2F => &[24],                     // MAX_UNIFORM_BUFFER_BINDINGS
        0x8A34 => &[4],                      // UNIFORM_BUFFER_OFFSET_ALIGNMENT
        0x8872 => &[16],                     // MAX_TEXTURE_IMAGE_UNITS
        0x8B4C => &[8],                      // MAX_VERTEX_TEXTURE_IMAGE_UNITS
        0x8B4D => &[16],                     // MAX_COMBINED_TEXTURE_IMAGE_UNITS
        0x8824 => &[1],                      // MAX_DRAW_BUFFERS
        0x8CDF => &[3],                      // MAX_COLOR_ATTACHMENTS
        0x8D57 => &[1],                      // MAX_SAMPLES
        0x8B4B => &[15 * 4],                 // MAX_VARYING_COMPONENTS
        0x8DFC => &[15],                     // MAX_VARYING_VECTORS
        0x9122 | 0x9125 => &[15 * 4],        // MAX_VERTEX_OUTPUT / FRAGMENT_INPUT_COMPONENTS
        0x8DFD | 0x8DFB => &[1536 / 4],      // MAX_FRAGMENT / VERTEX_UNIFORM_VECTORS
        0x8B4A | 0x8B49 => &[1536],          // MAX_VERTEX / FRAGMENT_UNIFORM_COMPONENTS
        0x8D6B => &[0xFF_FFFF],              // MAX_ELEMENT_INDEX
        0x821B => &[3],                      // MAJOR_VERSION
        0x821C => &[0],                      // MINOR_VERSION
        0x821D => &[0],                      // NUM_EXTENSIONS
        0x9111 => &[0],                      // MAX_SERVER_WAIT_TIMEOUT
        _ => &[0],
    }
}

// ---- the entry points ----
unsafe extern "system" fn stub() -> usize {
    0
}

unsafe extern "system" fn get_error() -> GLenum {
    0
}
unsafe extern "system" fn get_string(name: GLenum) -> *const u8 {
    match name {
        GL_VERSION => c"OpenGL ES 3.0 YakoGL".as_ptr() as _,
        GL_SHADING_LANGUAGE_VERSION => c"OpenGL ES GLSL ES 3.00".as_ptr() as _,
        GL_VENDOR => c"YakoGL".as_ptr() as _,
        GL_RENDERER => c"Broadcom V3D 4.2 (YakoGL)".as_ptr() as _,
        GL_EXTENSIONS => c"".as_ptr() as _,
        _ => c"".as_ptr() as _,
    }
}
unsafe extern "system" fn get_stringi(_name: GLenum, _i: GLuint) -> *const u8 {
    c"".as_ptr() as _
}
unsafe extern "system" fn get_integerv(pname: GLenum, out: *mut GLint) {
    for (i, v) in integer(pname).iter().enumerate() {
        unsafe { *out.add(i) = *v as GLint };
    }
}
unsafe extern "system" fn get_integer64v(pname: GLenum, out: *mut i64) {
    for (i, v) in integer(pname).iter().enumerate() {
        unsafe { *out.add(i) = *v };
    }
}
unsafe extern "system" fn get_booleanv(_pname: GLenum, out: *mut u8) {
    unsafe { *out = 0 };
}
unsafe extern "system" fn get_floatv(pname: GLenum, out: *mut f32) {
    // ALIASED_POINT_SIZE_RANGE, ALIASED_LINE_WIDTH_RANGE: [1, 1]
    if pname == 0x846D || pname == 0x846E {
        unsafe {
            *out = 1.0;
            *out.add(1) = 1.0;
        }
    } else {
        unsafe { *out = 0.0 };
    }
}
unsafe extern "system" fn get_shader_precision_format(_s: GLenum, _p: GLenum, range: *mut GLint, prec: *mut GLint) {
    unsafe {
        *range = 127;
        *range.add(1) = 127;
        *prec = 23;
    }
}
unsafe extern "system" fn get_internalformativ(_t: GLenum, _f: GLenum, _p: GLenum, n: GLsizei, out: *mut GLint) {
    // NUM_SAMPLE_COUNTS 0, SAMPLES nothing: single-sampled only (YakoGL's MAX_SAMPLES is 1)
    if n > 0 {
        unsafe { *out = 0 };
    }
}

fn new_names(n: GLsizei, out: *mut GLuint) {
    with(|s| {
        for i in 0..n as usize {
            unsafe { *out.add(i) = s.next };
            s.next += 1;
        }
    });
}
unsafe extern "system" fn gen_names(n: GLsizei, out: *mut GLuint) {
    new_names(n, out)
}
unsafe extern "system" fn create_shader(t: GLenum) -> GLuint {
    with(|s| {
        let id = s.next;
        s.next += 1;
        s.shader_src.insert(id, (t, String::new()));
        id
    })
}
unsafe extern "system" fn shader_source(sh: GLuint, n: GLsizei, strs: *const *const c_char, lens: *const GLint) {
    let mut text = String::new();
    for i in 0..n as usize {
        let p = unsafe { *strs.add(i) };
        let l = if lens.is_null() { -1 } else { unsafe { *lens.add(i) } };
        let part = if l < 0 {
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        } else {
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p as *const u8, l as usize) }).into_owned()
        };
        text.push_str(&part);
    }
    with(|s| {
        if let Some(e) = s.shader_src.get_mut(&sh) {
            e.1 = text;
        }
    });
}
unsafe extern "system" fn get_shaderiv(_sh: GLuint, pname: GLenum, out: *mut GLint) {
    // COMPILE_STATUS 1, INFO_LOG_LENGTH 0
    unsafe { *out = if pname == 0x8B81 { 1 } else { 0 } };
}
unsafe extern "system" fn get_info_log(_o: GLuint, n: GLsizei, len: *mut GLsizei, log: *mut c_char) {
    unsafe {
        if n > 0 && !log.is_null() {
            *log = 0;
        }
        if !len.is_null() {
            *len = 0;
        }
    }
}
unsafe extern "system" fn create_program() -> GLuint {
    with(|s| {
        let id = s.next;
        s.next += 1;
        s.prog_shaders.insert(id, Vec::new());
        id
    })
}
unsafe extern "system" fn attach_shader(p: GLuint, sh: GLuint) {
    with(|s| s.prog_shaders.entry(p).or_default().push(sh));
}
unsafe extern "system" fn link_program(p: GLuint) {
    with(|s| {
        let (mut vs, mut fs) = (String::new(), String::new());
        for sh in s.prog_shaders.get(&p).cloned().unwrap_or_default() {
            if let Some((t, src)) = s.shader_src.get(&sh) {
                if *t == GL_VERTEX_SHADER {
                    vs = src.clone();
                } else {
                    fs = src.clone();
                }
            }
        }
        s.programs.push(Program { vs, fs });
    });
}
unsafe extern "system" fn get_programiv(_p: GLuint, pname: GLenum, out: *mut GLint) {
    // LINK_STATUS 1; everything else (log length, active counts) 0
    unsafe { *out = if pname == 0x8B82 { 1 } else { 0 } };
}
unsafe extern "system" fn get_location(_p: GLuint, _name: *const c_char) -> GLint {
    with(|s| {
        s.next += 1;
        s.next as GLint
    })
}
unsafe extern "system" fn use_program(p: GLuint) {
    with(|s| s.program = p);
}
unsafe extern "system" fn bind_buffer(t: GLenum, b: GLuint) {
    with(|s| {
        s.bound.insert(t, b);
    });
}
unsafe extern "system" fn bind_buffer_range(t: GLenum, _i: GLuint, b: GLuint, _o: isize, _n: isize) {
    with(|s| {
        s.bound.insert(t, b);
        if t == 0x8A11 {
            s.frame.ubo_binds += 1; // UNIFORM_BUFFER
        }
    });
}
unsafe extern "system" fn bind_buffer_base(t: GLenum, i: GLuint, b: GLuint) {
    unsafe { bind_buffer_range(t, i, b, 0, 0) }
}
unsafe extern "system" fn buffer_data(t: GLenum, n: isize, data: *const c_void, _u: GLenum) {
    with(|s| {
        let id = *s.bound.get(&t).unwrap_or(&0);
        let mut v = vec![0u8; n.max(0) as usize];
        if !data.is_null() {
            v.copy_from_slice(unsafe { std::slice::from_raw_parts(data as *const u8, n as usize) });
            s.frame.buf_uploads += 1;
            s.frame.buf_bytes += n as u64;
        }
        s.buffers.insert(id, v);
    });
}
unsafe extern "system" fn buffer_sub_data(t: GLenum, off: isize, n: isize, data: *const c_void) {
    with(|s| {
        let id = *s.bound.get(&t).unwrap_or(&0);
        s.frame.buf_uploads += 1;
        s.frame.buf_bytes += n as u64;
        if let Some(b) = s.buffers.get_mut(&id) {
            let (o, n) = (off as usize, n as usize);
            if o + n <= b.len() && !data.is_null() {
                b[o..o + n].copy_from_slice(unsafe { std::slice::from_raw_parts(data as *const u8, n) });
            }
        }
    });
}
unsafe extern "system" fn map_buffer_range(t: GLenum, off: isize, n: isize, _a: u32) -> *mut c_void {
    with(|s| {
        let id = *s.bound.get(&t).unwrap_or(&0);
        let b = s.buffers.entry(id).or_default();
        if b.len() < (off + n) as usize {
            b.resize((off + n) as usize, 0);
        }
        s.frame.buf_uploads += 1;
        s.frame.buf_bytes += n as u64;
        unsafe { b.as_mut_ptr().add(off as usize) as *mut c_void }
    })
}
unsafe extern "system" fn unmap_buffer(_t: GLenum) -> u8 {
    1
}
unsafe extern "system" fn fence_sync(_c: GLenum, _f: u32) -> *const c_void {
    Box::leak(Box::new(0u8)) as *const u8 as *const c_void
}
unsafe extern "system" fn client_wait_sync(_s: *const c_void, _f: u32, _t: u64) -> GLenum {
    0x911A // ALREADY_SIGNALED
}
unsafe extern "system" fn get_synciv(_s: *const c_void, _p: GLenum, n: GLsizei, len: *mut GLsizei, out: *mut GLint) {
    unsafe {
        if n > 0 {
            *out = 0x9119; // SIGNALED
        }
        if !len.is_null() {
            *len = 1;
        }
    }
}
unsafe extern "system" fn is_true() -> u8 {
    1
}
unsafe extern "system" fn check_framebuffer_status(_t: GLenum) -> GLenum {
    0x8CD5 // FRAMEBUFFER_COMPLETE
}
unsafe extern "system" fn bind_framebuffer(_t: GLenum, fb: GLuint) {
    with(|s| {
        if fb != 0 {
            s.frame.fbo_binds += 1;
        }
    });
}
unsafe extern "system" fn active_texture(u: GLenum) {
    with(|s| s.active_unit = u - GL_TEXTURE0);
}
unsafe extern "system" fn bind_texture(t: GLenum, id: GLuint) {
    with(|s| {
        s.tex_bound.insert((s.active_unit, t), id);
    });
}
unsafe extern "system" fn tex_storage_2d(t: GLenum, _l: GLsizei, f: GLenum, w: GLsizei, h: GLsizei) {
    with(|s| s.frame.new_textures.push((t, f, w as u32, h as u32, 1)));
}
unsafe extern "system" fn tex_storage_3d(t: GLenum, _l: GLsizei, f: GLenum, w: GLsizei, h: GLsizei, d: GLsizei) {
    with(|s| s.frame.new_textures.push((t, f, w as u32, h as u32, d as u32)));
}
unsafe extern "system" fn tex_image_2d(
    t: GLenum, _l: GLint, f: GLint, w: GLsizei, h: GLsizei, _b: GLint, _fmt: GLenum, _ty: GLenum, px: *const c_void,
) {
    with(|s| {
        s.frame.new_textures.push((t, f as u32, w as u32, h as u32, 1));
        if !px.is_null() {
            s.frame.tex_uploads += 1;
            s.frame.tex_bytes += (w as u64) * (h as u64) * 4;
        }
    });
}
unsafe extern "system" fn tex_sub_image_2d(
    _t: GLenum, _l: GLint, _x: GLint, _y: GLint, w: GLsizei, h: GLsizei, _f: GLenum, _ty: GLenum, _px: *const c_void,
) {
    with(|s| {
        s.frame.tex_uploads += 1;
        s.frame.tex_bytes += (w as u64) * (h as u64) * 4; // an estimate: 4 bytes a texel
    });
}
unsafe extern "system" fn tex_sub_image_3d(
    _t: GLenum, _l: GLint, _x: GLint, _y: GLint, _z: GLint, w: GLsizei, h: GLsizei, d: GLsizei, _f: GLenum,
    _ty: GLenum, _px: *const c_void,
) {
    with(|s| {
        s.frame.tex_uploads += 1;
        s.frame.tex_bytes += (w as u64) * (h as u64) * (d as u64) * 4;
    });
}
fn cap(c: GLenum, on: bool) {
    with(|s| match c {
        GL_STENCIL_TEST => s.stencil = on,
        GL_DEPTH_TEST => s.depth = on,
        GL_BLEND => s.blend = on,
        _ => {}
    });
}
unsafe extern "system" fn enable(c: GLenum) {
    cap(c, true)
}
unsafe extern "system" fn disable(c: GLenum) {
    cap(c, false)
}
fn draw(count: GLsizei, instances: GLsizei) {
    with(|s| {
        let f = &mut s.frame;
        f.draws += 1;
        f.vertices += count.max(0) as u64 * instances.max(1) as u64;
        if instances > 1 {
            f.instanced += 1;
        }
        f.max_instances = f.max_instances.max(instances);
        f.stencil_draws += s.stencil as u32;
        f.depth_draws += s.depth as u32;
        f.blend_draws += s.blend as u32;
        f.programs_used.insert(s.program);
        for ((_u, t), id) in s.tex_bound.iter() {
            if *id != 0 {
                *f.tex_targets.entry(*t).or_default() += 1;
            }
        }
    });
}
unsafe extern "system" fn draw_arrays(_m: GLenum, _f: GLint, n: GLsizei) {
    draw(n, 1)
}
unsafe extern "system" fn draw_elements(_m: GLenum, n: GLsizei, _t: GLenum, _i: *const c_void) {
    draw(n, 1)
}
unsafe extern "system" fn draw_arrays_instanced(_m: GLenum, _f: GLint, n: GLsizei, i: GLsizei) {
    draw(n, i)
}
unsafe extern "system" fn draw_elements_instanced(_m: GLenum, n: GLsizei, _t: GLenum, _p: *const c_void, i: GLsizei) {
    draw(n, i)
}
unsafe extern "system" fn draw_range_elements(_m: GLenum, _s: GLuint, _e: GLuint, n: GLsizei, _t: GLenum, _p: *const c_void) {
    draw(n, 1)
}

/// Names whose entry points are implemented, the rest get `stub`.
pub fn get_proc(name: &str) -> *const c_void {
    let f: *const c_void = match name {
        "glGetError" => get_error as _,
        "glGetString" => get_string as _,
        "glGetStringi" => get_stringi as _,
        "glGetIntegerv" => get_integerv as _,
        "glGetInteger64v" => get_integer64v as _,
        "glGetBooleanv" => get_booleanv as _,
        "glGetFloatv" => get_floatv as _,
        "glGetShaderPrecisionFormat" => get_shader_precision_format as _,
        "glGetInternalformativ" => get_internalformativ as _,
        "glGenBuffers" | "glGenTextures" | "glGenFramebuffers" | "glGenRenderbuffers" | "glGenVertexArrays"
        | "glGenSamplers" | "glGenQueries" | "glGenTransformFeedbacks" => gen_names as _,
        "glCreateShader" => create_shader as _,
        "glShaderSource" => shader_source as _,
        "glGetShaderiv" => get_shaderiv as _,
        "glGetShaderInfoLog" | "glGetProgramInfoLog" => get_info_log as _,
        "glCreateProgram" => create_program as _,
        "glAttachShader" => attach_shader as _,
        "glLinkProgram" => link_program as _,
        "glGetProgramiv" => get_programiv as _,
        "glGetUniformLocation" | "glGetUniformBlockIndex" | "glGetAttribLocation" => get_location as _,
        "glUseProgram" => use_program as _,
        "glBindBuffer" => bind_buffer as _,
        "glBindBufferRange" => bind_buffer_range as _,
        "glBindBufferBase" => bind_buffer_base as _,
        "glBufferData" => buffer_data as _,
        "glBufferSubData" => buffer_sub_data as _,
        "glMapBufferRange" => map_buffer_range as _,
        "glUnmapBuffer" => unmap_buffer as _,
        "glFenceSync" => fence_sync as _,
        "glClientWaitSync" => client_wait_sync as _,
        "glGetSynciv" => get_synciv as _,
        "glIsSync" => is_true as _,
        "glCheckFramebufferStatus" => check_framebuffer_status as _,
        "glBindFramebuffer" => bind_framebuffer as _,
        "glActiveTexture" => active_texture as _,
        "glBindTexture" => bind_texture as _,
        "glTexStorage2D" => tex_storage_2d as _,
        "glTexStorage3D" => tex_storage_3d as _,
        "glTexImage2D" => tex_image_2d as _,
        "glTexSubImage2D" => tex_sub_image_2d as _,
        "glTexSubImage3D" => tex_sub_image_3d as _,
        "glEnable" => enable as _,
        "glDisable" => disable as _,
        "glDrawArrays" => draw_arrays as _,
        "glDrawElements" => draw_elements as _,
        "glDrawArraysInstanced" => draw_arrays_instanced as _,
        "glDrawElementsInstanced" => draw_elements_instanced as _,
        "glDrawRangeElements" => draw_range_elements as _,
        _ => stub as _,
    };
    f
}
