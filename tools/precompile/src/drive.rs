// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The product's shader back end, driven on the host as the GL core drives it.
//!
//! Every call below goes through `SHC_ONLINE`, the `struct shc_backend` an
//! image installs as `g_shaderc`, so a record made here is a record the
//! board makes for the same calls. The order is the GL core's:
//!
//! ```text
//!   glCompileShader      compile(stage, text)               the shader object's copy
//!   glLinkProgram        compile_vs_linked(text, map)       when the compile left the
//!                                                           attribute layout to the link
//!                        link(vs, &mut fs)                  may recompile the fragment shader
//!                                                           (a varying the VS never writes
//!                                                           read as 0; a constant W)
//!   a draw that blends   blend_variant(linked fs, key)      (gl::draw::fs_variant_for)
//!                        link(vs, &mut variant)             as the draw links it
//! ```
//!
//! and the holders are the GL core's too: the shader object keeps its copy,
//! the program its own, so a link that replaces the program's fragment
//! shader leaves the shader object's record alive, as on the board. After
//! each call every record the cache indexes is read back ([`capture`]), so a
//! record that a later call frees is captured all the same.

use core::ffi::c_char;
use yakogl::shaderc::online::{backend, store, SHC_ONLINE};
use yakogl::shaderc::{
    release_shader, retain_shader, ShcAttribBinding, ShcAttribMap, ShcShader, SHC_ATTRIB_NO_LOCATION,
    SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
};

/// A record's identity in the cache: its key, stage and blend key.
pub type Id = ([u8; 32], u32, u32);

/// The bytes of the keep region lent for one run: far more than an image
/// lends (the product sizes its own, CONFIG_YAKOGL_SHADERC_KEEP_BYTES), so
/// that no job here is refused for room. The keep size changes no code the
/// compiler makes: a record either fits or the compile is refused.
const KEEP_BYTES: usize = 64 << 20;

/// Lend the compiler its two regions, once, as an OSAL does at boot. The
/// scratch is the size an image lends by default and the online refusal
/// fixtures were measured with (store::SCRATCH_BYTES, which
/// CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES's default equals): a program too large
/// for it is refused here as it is on the board.
pub fn lend() -> Result<(), String> {
    let scratch: &'static mut [u8] = Box::leak(vec![0u8; store::SCRATCH_BYTES].into_boxed_slice());
    /* u64s: the keep region must be 8-byte aligned */
    let keep: &'static mut [u64] = Box::leak(vec![0u64; KEEP_BYTES / 8].into_boxed_slice());
    // SAFETY: both buffers are leaked, so they outlive every compile, and
    // nothing else ever touches them; this process is single-threaded.
    let (rs, rk) = unsafe {
        (
            store::shc_online_lend_scratch(scratch.as_mut_ptr(), scratch.len()),
            store::shc_online_lend_keep(keep.as_mut_ptr() as *mut u8, KEEP_BYTES),
        )
    };
    if rs != 0 || rk != 0 {
        return Err(format!("the compiler refused its regions (scratch {rs}, keep {rk})"));
    }
    /* The user's switches a compile reads from its source, at an image's
     * boot values: the `if` lowering (PVR_BACKEND_OPT_IF_LOWERING) keys a
     * record when it is not the default, so a table made under another
     * would be rows an image's look-up never finds. Set, not assumed. */
    yakogl::shaderc::set_if_lowering(yakogl::shaderc::IfLowering::Default.as_u32());
    Ok(())
}

/// The log of a refused call, as the GL core would put it in an info log.
fn log_text(log: &[c_char]) -> String {
    let n = log.iter().position(|&c| c == 0).unwrap_or(log.len());
    String::from_utf8_lossy(&log[..n].iter().map(|&c| c as u8).collect::<Vec<u8>>()).into_owned()
}

/// `b` as lower-case hex.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The shaders a job holds, as the GL core's objects hold them; all released
/// when the job ends ([`Holders::release_all`]).
#[derive(Default)]
pub struct Holders(Vec<ShcShader>);

impl Holders {
    fn keep(&mut self, sh: ShcShader) -> ShcShader {
        self.0.push(sh);
        sh
    }

    /// Release every holder; the keep region must then be empty, or a record
    /// leaked (a holder the GL core would give back that this did not).
    pub fn release_all(&mut self) -> Result<(), String> {
        for sh in self.0.drain(..) {
            release_shader(&sh);
        }
        let held = store::keep_in_use();
        if held != 0 || backend::stats().0 != 0 {
            return Err(format!("{held} bytes of keep still held after a job released everything"));
        }
        Ok(())
    }
}

/// `compile(stage, text)`: glCompileShader. The shader object's copy, held.
pub fn compile(h: &mut Holders, stage: u32, text: &str) -> Result<ShcShader, String> {
    let src = format!("{text}\0");
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let f = SHC_ONLINE.compile.ok_or("the back end has no compile")?;
    // SAFETY: a NUL-terminated source, a real output, and the log's length.
    let rc = unsafe { f(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        /* one line, as tests/online_corpus.rs writes the refusal fixtures */
        return Err(log_text(&log).trim_end().replace('\n', " / "));
    }
    Ok(h.keep(out))
}

/// glLinkProgram on copies of the two shader objects' shaders: the attribute
/// layout first when the vertex shader's compile left it to the link
/// (`gl::program::link_attrib_layout`), then `link`. Returns the program's
/// two shaders, held; the shader objects keep theirs.
pub fn link(h: &mut Holders, vs_text: &str, vs: &ShcShader, fs: &ShcShader) -> Result<(ShcShader, ShcShader), String> {
    /* the program's copies of the shader objects' shaders */
    retain_shader(vs);
    retain_shader(fs);
    let (mut rvs, mut rfs) = (*vs, *fs);
    if rvs.layout_at_link != 0 {
        let laid_out = attrib_layout(&rvs).and_then(|map| {
            let src = format!("{vs_text}\0");
            let mut out = ShcShader::ZERO;
            let mut log = [0 as c_char; 1024];
            let f = SHC_ONLINE.compile_vs_linked.ok_or("the back end has no compile_vs_linked")?;
            // SAFETY: a NUL-terminated source, a map and an output that live
            // across the call, and the log's length.
            let rc = unsafe { f(src.as_ptr() as *const c_char, &map, &mut out, log.as_mut_ptr(), log.len()) };
            match rc {
                0 => Ok(out),
                _ => Err(format!("the vertex shader with the program's attribute locations: {}", log_text(&log))),
            }
        });
        match laid_out {
            /* the program lets go of its plain copy and keeps the laid-out one */
            Ok(out) => {
                release_shader(&rvs);
                rvs = out;
            }
            Err(e) => {
                h.keep(rvs);
                h.keep(rfs);
                return Err(e);
            }
        }
    }
    /* `link` replaces the program's fragment copy in place when it compiles
     * it again (and releases the copy it replaced): the program holds
     * whatever is there afterwards, linked or not */
    let linked = link_into(&rvs, &mut rfs);
    h.keep(rvs);
    h.keep(rfs);
    linked.map(|()| (rvs, rfs))
}

/// `link(vs, fs)`, the program's fragment copy updated in place.
fn link_into(vs: &ShcShader, fs: &mut ShcShader) -> Result<(), String> {
    let mut log = [0 as c_char; 1024];
    let f = SHC_ONLINE.link.ok_or("the back end has no link")?;
    // SAFETY: two valid, distinct shaders this back end made, and the log's length.
    let rc = unsafe { f(vs, fs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log_text(&log));
    }
    Ok(())
}

/// A draw's variant: `blend_variant(fs, key)` of the program's (linked)
/// fragment shader, then linked to the program's vertex shader, as
/// `gl::draw::fs_variant_for` does. The variant is held, as the program's
/// variant slot holds it.
pub fn variant(h: &mut Holders, vs: &ShcShader, fs: &ShcShader, key: u32) -> Result<(), String> {
    let mut v = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let f = SHC_ONLINE.blend_variant.ok_or("the back end has no blend_variant")?;
    // SAFETY: a shader this back end made, a real output, and the log's length.
    let rc = unsafe { f(fs, key, &mut v, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log_text(&log));
    }
    /* the slot holds what the link left in it (a variant compiled again
     * replaces the one it was given, which the link releases) */
    let linked = link_into(vs, &mut v);
    h.keep(v);
    linked
}

/// A variant of a fragment shader no listed program links: what a draw asks
/// of a program whose link substituted nothing, the plain record's variant.
pub fn variant_unlinked(h: &mut Holders, fs: &ShcShader, key: u32) -> Result<(), String> {
    let mut v = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let f = SHC_ONLINE.blend_variant.ok_or("the back end has no blend_variant")?;
    // SAFETY: as in `variant`.
    let rc = unsafe { f(fs, key, &mut v, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log_text(&log));
    }
    h.keep(v);
    Ok(())
}

/// The attribute locations glLinkProgram gives a vertex shader whose compile
/// left them to the link, for a program with no glBindAttribLocation:
/// `gl::program::attrib_layout` with no bindings -- each active attribute
/// the lowest run of free locations, in declaration order; an inactive one
/// none.
fn attrib_layout(vs: &ShcShader) -> Result<ShcAttribMap, String> {
    const LOCS: usize = yakogl::gl::context::MAX_VERTEX_ATTRIBS;
    if vs.declared.is_null() {
        return Err("the vertex shader left its layout to the link but declared nothing".into());
    }
    // SAFETY: the compile's list of num_declared entries, kept with the shader.
    let declared = unsafe { core::slice::from_raw_parts(vs.declared, vs.num_declared as usize) };
    let mut map = ShcAttribMap::ZERO;
    let n = declared.len().min(map.b.len());
    for (d, s) in map.b.iter_mut().zip(declared) {
        *d = ShcAttribBinding { location: SHC_ATTRIB_NO_LOCATION, ..*s };
    }
    map.count = n as u8;
    let mut taken = [false; LOCS];
    for d in map.b[..n].iter_mut().filter(|d| d.gl_type == 0 || d.active != 0) {
        let cols = d.columns.max(1) as usize;
        let Some(l) = (0..=LOCS.saturating_sub(cols)).find(|&l| taken[l..l + cols].iter().all(|t| !t)) else {
            return Err(format!("link error: no {cols} free location(s) left for an attribute"));
        };
        d.location = l as u8;
        taken[l..l + cols].iter_mut().for_each(|t| *t = true);
    }
    Ok(map)
}

/// The stage a path's extension names.
pub fn stage_of(path: &str) -> u32 {
    if path.ends_with(".vert") {
        SHC_STAGE_VERTEX
    } else {
        SHC_STAGE_FRAGMENT
    }
}
