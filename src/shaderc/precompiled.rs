// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The precompiled shaders: shaders our own compiler compiled at image-build
//! time. In an image with the compiler they are the tier in front of it; in
//! an image without one ([`SHC_PRECOMPILED`]) they are the whole back end.
//!
//! The table is what `yakogl-precompile` (tools/precompile) wrote for this
//! build's GPU and ES level from tools/shaders/precompiled.list: every record
//! the compiler's cache held when the list's sources were compiled, its
//! programs linked and their variants asked for, through the same entry
//! points the GL core calls. A row is found by the key the cache uses -- the
//! record key (the source's SHA-256 chained with the zeroed varyings, the
//! attribute map, the constant W, the sampler-state key and a non-default
//! `if` lowering; `record_key`), the stage and the blend key -- so the
//! compiler's back end asks, in order: the keep cache, this table, the
//! compiler (`online::backend`). A hit runs no compile
//! ([`shc_precompiled_hits`] counts them, `shc_online_compiler_runs` does not
//! move); a miss compiles as it would without the table.
//!
//! Without the compiler a miss is refused by name, `-ENOENT`, with one line
//! per cause in the log the GL core puts in the info log (and on the console
//! once per cause): a source the table does not hold, a variant it does not
//! hold, a program it did not link (see [`SHC_PRECOMPILED`]).
//!
//! A row owns nothing: its `ShcShader` points into the image, its `priv`
//! points at the row, and retaining or releasing it does nothing. A back end
//! reads a row's source and facts through [`view`] (the compiler's as it
//! reads a keep record's), so a variant of a row or the link of a row is
//! asked for the same way. Two answers the table carries besides the rows spare the front
//! end at a link and at a program's first draw: the verdict of every link the
//! generator made ([`linked`]) and every row's sampler-state sensitivity
//! ([`sensitivity`]).
//!
//! The rows were compiled with the compiler's switches at their boot values;
//! with any of them changed ([`applies`]) the table is not consulted, so a
//! measurement that flips one compiles as before. The table names the
//! compiler that made it (its toolchain mark, logged once at the first
//! look-up), and a table for another GPU or ES level does not build.

use crate::shaderc::ShcShader;

#[cfg(feature = "precompiled-shaders")]
#[allow(unused_imports)]
use crate::shaderc::{
    pad_attribs, pad_varyings, ShcAttrib, ShcAttribBinding, ShcAttribMap, ShcBlob, ShcBlock, ShcBlockMember, ShcBlocks,
    ShcPrecompiledLink, ShcSource, ShcUniform, ShcVarying, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
};
#[cfg(all(feature = "precompiled-shaders", feature = "gpu-vivante"))]
#[allow(unused_imports)]
use crate::shaderc::{pad_io, ShcViv, ShcVivIo, SHC_VIV_MAX_IO};

/* the table: `#[path = "<YAKOGL_PRECOMPILED_TABLE>"] mod table;` (build.rs) */
#[cfg(feature = "precompiled-shaders")]
include!(concat!(env!("OUT_DIR"), "/precompiled_table.rs"));

#[cfg(all(feature = "precompiled-shaders", feature = "gpu-v3d"))]
compile_error!("feature \"precompiled-shaders\": yakogl-precompile makes no table for the V3D");

/// The GPU and ES level this build's compiler makes code for, as a table's
/// toolchain mark names them (`gpu=<..> es2=<0|1>`).
pub const BUILD_GPU: &str = if cfg!(feature = "gpu-vivante") { "gc7000" } else { "powervr" };

/// Does a table made for `gpu` and `es2` fit this build?
pub const fn mark_matches(gpu: &str, es2: bool) -> bool {
    let (a, b) = (gpu.as_bytes(), BUILD_GPU.as_bytes());
    if a.len() != b.len() || es2 != crate::REPORT_ES2 {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/* A table for another GPU or ES level stops the build: its rows are another
 * compiler's code, and nothing at run time could tell. */
#[cfg(feature = "precompiled-shaders")]
const _: () = assert!(
    mark_matches(table::SHC_PRECOMPILED_GPU, table::SHC_PRECOMPILED_ES2),
    "the precompiled table was made for another GPU or ES level (its toolchain mark); regenerate it for this build"
);

/// Look-ups the table answered since boot.
static HITS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// The table is consulted (the C ABI's switch, [`shc_precompiled_set_enabled`]).
static ENABLED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// The mark has been logged.
#[cfg(feature = "precompiled-shaders")]
static MARKED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// `uint32_t shc_precompiled_hits(void)`: how many look-ups the precompiled
/// table answered since boot, wrapping. With `shc_online_compiler_runs` a run
/// says what the table spared: a hit is a compile that did not run.
#[no_mangle]
pub extern "C" fn shc_precompiled_hits() -> u32 {
    HITS.load(core::sync::atomic::Ordering::Relaxed)
}

/// `void shc_precompiled_set_enabled(int on)`: consult the table (1, the
/// default) or compile everything (0), for a measurement against the
/// compiler. Read at each look-up; without the compiler there is nothing to
/// measure against and the table is always consulted.
#[no_mangle]
pub extern "C" fn shc_precompiled_set_enabled(on: core::ffi::c_int) {
    ENABLED.store(on != 0, core::sync::atomic::Ordering::Relaxed);
}

/// `size_t shc_precompiled_rows(void)`: the table's rows; 0 in a build
/// without one.
#[no_mangle]
pub extern "C" fn shc_precompiled_rows() -> usize {
    rows()
}

/// `const char *shc_precompiled_toolchain(void)`: the table's toolchain mark
/// (`yakogl=<version> commit=<sha>[+dirty] gpu=<gpu> es2=<0|1>`), NUL-
/// terminated; "" in a build without one.
#[no_mangle]
pub extern "C" fn shc_precompiled_toolchain() -> *const core::ffi::c_char {
    #[cfg(feature = "precompiled-shaders")]
    let mark = table::SHC_PRECOMPILED_TOOLCHAIN_C;
    #[cfg(not(feature = "precompiled-shaders"))]
    let mark = "\0";
    mark.as_ptr() as *const core::ffi::c_char
}

/// The table's rows (0 without one).
pub fn rows() -> usize {
    #[cfg(feature = "precompiled-shaders")]
    return table::SHC_PRECOMPILED_TABLE.len();
    #[cfg(not(feature = "precompiled-shaders"))]
    0
}

/// The compiler's switches stand where the rows were compiled: at their boot
/// values. A row is the code of one setting of them, and the record key does
/// not carry them (they are read when a shader compiles), so a table made at
/// the defaults answers only while they hold: the user's (native cube, the
/// projective divide and level, the sampler-state variants, the `if`
/// lowering), the GC7000's measurement knob (`shc_debug_uniidx`), and the
/// switches the host tests flip to compare a compile with and without a
/// step.
fn switches_at_boot() -> bool {
    use crate::shaderc as s;
    let user = s::native_cube_enabled() == (s::PVR_NATIVE_CUBE_DEFAULT != 0)
        && !s::proj_divide_enabled()
        && s::proj_lod_mode() == s::PVR_PROJ_LOD_DEFAULT
        && s::lod_variants_enabled() == (true, true)
        && s::if_lowering() == s::IfLowering::Default;
    #[cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
    let user = user
        && crate::shaderc::online::select::uniidx_knobs() == 0
        && !crate::shaderc::online::vivante::alloc::lane_moves_disabled();
    #[cfg(all(test, feature = "shaderc-online"))]
    let user = user
        && !crate::shaderc::online::lower::RELOCATE_OFF.load(core::sync::atomic::Ordering::Relaxed)
        && !crate::shaderc::online::lower::FOLD_OFF.load(core::sync::atomic::Ordering::Relaxed)
        && crate::shaderc::online::powervr::unroll::early_exit_on();
    user
}

/// The table is consulted now: there is one, it is enabled (always, without
/// the compiler), and the switches are at their boot values (without the
/// compiler a switch moved off them makes every look-up a refusal: no row is
/// the code of that setting).
pub fn applies() -> bool {
    cfg!(feature = "precompiled-shaders")
        && rows() != 0
        && (!cfg!(feature = "shaderc-online") || ENABLED.load(core::sync::atomic::Ordering::Relaxed))
        && switches_at_boot()
}

/// Log the table's toolchain mark, once.
#[cfg(feature = "precompiled-shaders")]
fn mark_once() {
    if !MARKED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        glog!(
            "yakogl: precompiled shaders: {} rows, {} links of {}; toolchain: {}",
            table::SHC_PRECOMPILED_TABLE.len(),
            table::SHC_PRECOMPILED_LINKS.len(),
            table::SHC_PRECOMPILED_LIST,
            table::SHC_PRECOMPILED_TOOLCHAIN
        );
    }
}

/// The row with this identity, as the shader a look-up hands out (its
/// `priv` the row), counted as a hit; `None` when the table has none or
/// is not consulted ([`applies`]).
pub fn find(key: &[u8; 32], stage: u32, blend_key: u32) -> Option<ShcShader> {
    #[cfg(feature = "precompiled-shaders")]
    if applies() {
        mark_once();
        let rows = &table::SHC_PRECOMPILED_TABLE;
        let i = (0..rows.len()).find(|&i| {
            rows[i].stage == stage && rows[i].blend_key == blend_key && &table::SHC_PRECOMPILED_KEYS[i] == key
        })?;
        HITS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let mut sh = rows[i].shader;
        sh.priv_ = &rows[i] as *const ShcBlob as *const core::ffi::c_void;
        return Some(sh);
    }
    let _ = (key, stage, blend_key);
    None
}

/// The row behind a shader [`find`] handed out.
fn row_of(sh: &ShcShader) -> Option<usize> {
    #[cfg(feature = "precompiled-shaders")]
    {
        let rows = &table::SHC_PRECOMPILED_TABLE;
        let size = core::mem::size_of::<ShcBlob>();
        let off = (sh.priv_ as usize).checked_sub(rows.as_ptr() as usize)?;
        if off % size == 0 && off / size < rows.len() {
            return Some(off / size);
        }
    }
    let _ = sh;
    None
}

/// A row as a back end reads a record: what it was compiled from and for.
#[derive(Clone, Copy)]
pub struct RowView {
    /// Its record key.
    pub sha: [u8; 32],
    pub stage: u32,
    pub blend_key: u32,
    /// The source, without its NUL.
    pub source: &'static [u8],
    /// The source's name in the list (its path under tools/shaders).
    pub name: &'static str,
    pub w_constant: bool,
    pub lod_key: crate::shaderc::LodKey,
    pub if_lowering: crate::shaderc::IfLowering,
    pub attrib_map: Option<&'static crate::shaderc::ShcAttribMap>,
    pub blocks: crate::shaderc::ShcBlocks,
}

/// The row behind `sh`: `None` for a shader that is not a row.
pub fn view(sh: &ShcShader) -> Option<RowView> {
    row_of(sh).and_then(row_view)
}

/// Row `i` as a back end reads a record.
#[cfg(feature = "precompiled-shaders")]
fn row_view(i: usize) -> Option<RowView> {
    let src = &table::SHC_PRECOMPILED_SOURCES[usize::from(table::SHC_PRECOMPILED_SOURCE[i])];
    let source = src.text.as_bytes().strip_suffix(b"\0").unwrap_or(src.text.as_bytes());
    let row = &table::SHC_PRECOMPILED_TABLE[i];
    Some(RowView {
        sha: table::SHC_PRECOMPILED_KEYS[i],
        stage: row.stage,
        blend_key: row.blend_key,
        source,
        name: src.name,
        w_constant: table::SHC_PRECOMPILED_W_CONSTANT[i],
        lod_key: crate::shaderc::LodKey::from_bits(table::SHC_PRECOMPILED_LOD_KEY[i]),
        if_lowering: crate::shaderc::IfLowering::from_u32(table::SHC_PRECOMPILED_IF_LOWERING[i]),
        attrib_map: table::SHC_PRECOMPILED_ATTRIB_MAP[i],
        blocks: table::SHC_PRECOMPILED_BLOCKS[i],
    })
}

/// Row `i` as a back end reads a record: no rows without a table.
#[cfg(not(feature = "precompiled-shaders"))]
fn row_view(_: usize) -> Option<RowView> {
    None
}

/// The fragment shader's record key a recorded link of `vs` and `fs` (with
/// `fs_blend_key`) left in the program: `fs` itself when it substituted
/// nothing. `None` when the table recorded no such link or is not consulted.
pub fn linked(vs: &[u8; 32], fs: &[u8; 32], fs_blend_key: u32) -> Option<[u8; 32]> {
    #[cfg(feature = "precompiled-shaders")]
    if applies() {
        return table::SHC_PRECOMPILED_LINKS
            .iter()
            .find(|l| &l.vs == vs && &l.fs == fs && l.fs_blend_key == fs_blend_key)
            .map(|l| l.linked_fs);
    }
    let _ = (vs, fs, fs_blend_key);
    None
}

/// The sampler-state sensitivity (`LodKey::bits`) the table recorded for the
/// row behind `sh`: what `lod_sensitivity` answers without the front end.
pub fn sensitivity(sh: &ShcShader) -> Option<u64> {
    let i = row_of(sh)?;
    if !applies() {
        return None;
    }
    #[cfg(feature = "precompiled-shaders")]
    let bits = Some(table::SHC_PRECOMPILED_SENSITIVITY[i]);
    #[cfg(not(feature = "precompiled-shaders"))]
    let bits = {
        let _ = i;
        None
    };
    bits
}

/* ---- the back end of an image without the compiler ------------------------- */

/// Is `fs` linked to `vs` the same with any partner? A pair the table did not
/// link is linked by the table-level rules alone ([`crate::shaderc::link`])
/// only when the compiler's own link rules could not have decided anything
/// for it: the vertex shader declares no uniform (so no uniform can differ
/// between the stages), and the fragment shader declares no uniform and
/// reads no varying (so none can be undeclared, unwritten, mistyped or over
/// the budget, and nothing is sampled for the constant W to change). Any
/// other pair the table did not link is refused by name.
#[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
fn link_invariant(vs: &ShcShader, fs: &ShcShader) -> bool {
    #[cfg(feature = "gpu-vivante")]
    let fs_inputs = fs.viv.num_in;
    #[cfg(not(feature = "gpu-vivante"))]
    let fs_inputs = fs.num_varyings;
    vs.num_uniforms == 0 && fs.num_uniforms == 0 && fs_inputs == 0
}

#[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
mod alone {
    //! `struct shc_backend` with the precompiled shaders as the whole back
    //! end: every entry point a look-up or a refusal by name, `-ENOENT`, the
    //! line in the log (the GL core's info log) and, once per cause, on the
    //! console.
    use super::{find, link_invariant, linked, sensitivity, table, view};
    use crate::shaderc::record_key::record_key;
    use crate::shaderc::sha256::sha256;
    use crate::shaderc::{LodKey, ShcAttribMap, ShcBackend, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
    use crate::util::{cstr_bytes, CStrWriter, EINVAL, ENOENT};
    use core::ffi::c_char;
    use core::fmt::Write as _;

    fn stage_name(stage: u32) -> &'static str {
        if stage == SHC_STAGE_VERTEX {
            "vertex"
        } else {
            "fragment"
        }
    }

    fn hex(b: &[u8]) -> impl core::fmt::Display + '_ {
        struct H<'a>(&'a [u8]);
        impl core::fmt::Display for H<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
            }
        }
        H(b)
    }

    /// The refusal of a source the table does not hold (`laid_out`: the
    /// vertex shader with the program's attribute locations).
    fn refuse_source(log: &mut CStrWriter, stage: u32, text: &[u8], laid_out: bool) -> i32 {
        let what = if laid_out { " laid out with the program's attribute locations" } else { "" };
        let _ = write!(
            log,
            "precompiled shaders: {} shader sha256={}{what} is not in this image's precompiled shaders and the \
             image has no shader compiler; add it to {} and rebuild",
            stage_name(stage),
            hex(&sha256(text)),
            table::SHC_PRECOMPILED_LIST
        );
        glog_once!(
            "yakogl: precompiled shaders: a {} shader sha256={} is not in the table (no compiler)",
            stage_name(stage),
            hex(&sha256(text))
        );
        -ENOENT
    }

    unsafe extern "C" fn compile(
        stage: u32,
        glsl: *const c_char,
        out: *mut ShcShader,
        log: *mut c_char,
        log_sz: usize,
    ) -> i32 {
        // SAFETY: the caller passes a null log, or one valid for log_sz bytes.
        let mut log = unsafe { CStrWriter::new(log, log_sz) };
        if glsl.is_null() || out.is_null() {
            let _ = log.write_str("precompiled shaders: NULL source or output");
            return -EINVAL;
        }
        // SAFETY: a NUL-terminated source (the GL layer's copy).
        let text = unsafe { cstr_bytes(glsl) };
        let key = record_key(text, None, None, false, LodKey::NONE, crate::shaderc::if_lowering());
        match find(&key, stage, 0) {
            Some(sh) => {
                // SAFETY: out is non-null and points at the caller's shc_shader.
                unsafe { out.write(sh) };
                0
            }
            None => refuse_source(&mut log, stage, text, false),
        }
    }

    unsafe extern "C" fn compile_vs_linked(
        glsl: *const c_char,
        map: *const ShcAttribMap,
        out: *mut ShcShader,
        log: *mut c_char,
        log_sz: usize,
    ) -> i32 {
        // SAFETY: as in `compile`.
        let mut log = unsafe { CStrWriter::new(log, log_sz) };
        if glsl.is_null() || map.is_null() || out.is_null() {
            let _ = log.write_str("precompiled shaders: NULL source, map or output");
            return -EINVAL;
        }
        // SAFETY: a NUL-terminated source and a map the caller holds across the call.
        let (text, map) = unsafe { (cstr_bytes(glsl), &*map) };
        let key = record_key(text, None, Some(map), false, LodKey::NONE, crate::shaderc::if_lowering());
        match find(&key, SHC_STAGE_VERTEX, 0) {
            Some(sh) => {
                // SAFETY: as in `compile`.
                unsafe { out.write(sh) };
                0
            }
            None => refuse_source(&mut log, SHC_STAGE_VERTEX, text, true),
        }
    }

    /// The program's fragment shader as the table linked it, then the
    /// table-level rules (`shaderc::link`); a pair it did not link, refused
    /// by name unless both shaders are link-invariant ([`link_invariant`]).
    unsafe extern "C" fn link(vs: *const ShcShader, fs: *mut ShcShader, log: *mut c_char, log_sz: usize) -> i32 {
        if vs.is_null() || fs.is_null() {
            // SAFETY: as in `compile`.
            let _ = unsafe { CStrWriter::new(log, log_sz) }.write_str("precompiled shaders: NULL shader to link");
            return -EINVAL;
        }
        // SAFETY: two valid, distinct shaders (the struct's contract).
        let (vsh, fsh) = unsafe { (&*vs, &mut *fs) };
        let (Some(v), Some(f)) = (view(vsh), view(fsh)) else {
            // SAFETY: as in `compile`.
            let _ = unsafe { CStrWriter::new(log, log_sz) }
                .write_str("precompiled shaders: a shader of this program is not a precompiled shader");
            return -EINVAL;
        };
        match linked(&v.sha, &f.sha, f.blend_key) {
            Some(l) if l == f.sha => {}
            /* the table compiled the fragment shader again for this program */
            Some(l) => match find(&l, SHC_STAGE_FRAGMENT, f.blend_key) {
                Some(sh) => *fsh = sh,
                None => return refuse_link(log, log_sz, v.name, f.name),
            },
            None if link_invariant(vsh, fsh) => {}
            None => return refuse_link(log, log_sz, v.name, f.name),
        }
        // SAFETY: the caller's contract, passed straight through.
        #[cfg(feature = "gpu-powervr")]
        let rc = unsafe { crate::shaderc::link::link_shaders(vs, fs, log, log_sz, "precompiled shaders") };
        // SAFETY: as above.
        #[cfg(feature = "gpu-vivante")]
        let rc = unsafe { crate::shaderc::link::link_shaders_viv(vs, fs, log, log_sz, "precompiled shaders") };
        /* the GC7000's two stages share one constant file */
        #[cfg(feature = "gpu-vivante")]
        if rc == 0 {
            // SAFETY: as above.
            if let Err((a, b)) = crate::vivante::shader::constant_files_fit(unsafe { &*vs }, unsafe { &*fs }) {
                // SAFETY: as in `compile`.
                let _ = write!(
                    unsafe { CStrWriter::new(log, log_sz) },
                    "precompiled shaders: link error: the constant registers the two stages need (vertex {a}, \
                     fragment {b}) exceed the {} this GPU has",
                    crate::vivante::regs::GC_NUM_CONSTANTS
                );
                return -EINVAL;
            }
        }
        rc
    }

    fn refuse_link(log: *mut c_char, log_sz: usize, vs: &str, fs: &str) -> i32 {
        // SAFETY: the caller passes a null log, or one valid for log_sz bytes.
        let mut w = unsafe { CStrWriter::new(log, log_sz) };
        let _ = write!(w, "precompiled shaders: program {vs}/{fs} was not linked at build time");
        glog_once!("yakogl: precompiled shaders: program {vs}/{fs} was not linked at build time");
        -ENOENT
    }

    /// A variant the table holds, of the row `sh`: its record key is the
    /// row's source and facts (`lod_key`: the sampler-state key asked for),
    /// as the compiler would key it.
    fn variant(sh: &ShcShader, blend_key: u32, lod: Option<LodKey>, out: *mut ShcShader, log: &mut CStrWriter) -> i32 {
        let Some(r) = view(sh) else {
            let _ = log.write_str("precompiled shaders: a variant of a shader that is not a precompiled shader");
            return -EINVAL;
        };
        let lod_key = lod.unwrap_or(r.lod_key);
        let map = if lod.is_some() { r.attrib_map } else { None };
        let key = record_key(r.source, None, map, r.w_constant, lod_key, r.if_lowering);
        match find(&key, r.stage, blend_key) {
            Some(v) => {
                // SAFETY: out is non-null (the callers check) and points at the caller's shc_shader.
                unsafe { out.write(v) };
                0
            }
            None => {
                let (what, k) = match lod {
                    Some(l) => ("sampler key", l.bits()),
                    None => ("blend key", u64::from(blend_key)),
                };
                let _ = write!(log, "precompiled shaders: {} has no variant for {what} {k:#x}", r.name);
                glog_once!("yakogl: precompiled shaders: {} has no variant for {what} {k:#x}", r.name);
                -ENOENT
            }
        }
    }

    unsafe extern "C" fn blend_variant(
        fs: *const ShcShader,
        blend_key: u32,
        out: *mut ShcShader,
        log: *mut c_char,
        log_sz: usize,
    ) -> i32 {
        // SAFETY: as in `compile`.
        let mut log = unsafe { CStrWriter::new(log, log_sz) };
        if fs.is_null() || out.is_null() {
            let _ = log.write_str("precompiled shaders: NULL shader or output");
            return -EINVAL;
        }
        // SAFETY: a valid shader, read by value because `out` may be the same object.
        let fs = unsafe { fs.read() };
        if blend_key == 0 {
            // SAFETY: as above.
            unsafe { out.write(fs) };
            return 0;
        }
        variant(&fs, blend_key, None, out, &mut log)
    }

    unsafe extern "C" fn lod_sensitivity(sh: *const ShcShader) -> u64 {
        // SAFETY: null, or a valid shader.
        unsafe { sh.as_ref() }.and_then(sensitivity).unwrap_or(0)
    }

    unsafe extern "C" fn sampler_variant(
        sh: *const ShcShader,
        key: u64,
        out: *mut ShcShader,
        log: *mut c_char,
        log_sz: usize,
    ) -> i32 {
        // SAFETY: as in `compile`.
        let mut log = unsafe { CStrWriter::new(log, log_sz) };
        if sh.is_null() || out.is_null() {
            let _ = log.write_str("precompiled shaders: NULL shader or output");
            return -EINVAL;
        }
        // SAFETY: as in `blend_variant`.
        let sh = unsafe { sh.read() };
        if key == 0 {
            // SAFETY: as above.
            unsafe { out.write(sh) };
            return 0;
        }
        variant(&sh, 0, Some(LodKey::from_bits(key)), out, &mut log)
    }

    /// The back end of an image with the precompiled shaders and no
    /// compiler (`g_shaderc` there).
    pub static SHC_PRECOMPILED: ShcBackend = ShcBackend {
        compile: Some(compile),
        link: Some(link),
        blend_variant: Some(blend_variant),
        compile_vs_linked: Some(compile_vs_linked),
        lod_sensitivity: Some(lod_sensitivity),
        sampler_variant: Some(sampler_variant),
    };
}
#[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
pub use alone::SHC_PRECOMPILED;

#[cfg(test)]
mod tests {
    use super::*;

    /// A table names the GPU and ES level it was made for; only this
    /// build's fit (the build-time assertion above is this function).
    #[test]
    fn a_table_for_another_gpu_or_es_level_does_not_fit() {
        assert!(mark_matches(BUILD_GPU, crate::REPORT_ES2));
        assert!(!mark_matches(BUILD_GPU, !crate::REPORT_ES2), "another ES level");
        let other = if BUILD_GPU == "powervr" { "gc7000" } else { "powervr" };
        assert!(!mark_matches(other, crate::REPORT_ES2), "another GPU");
        assert!(!mark_matches("powervrx", crate::REPORT_ES2));
    }

    /// Every scalar and every array a row or a compile puts in a shader (the
    /// pointers by what they point at), as text: two shaders are the same
    /// shader exactly when these are equal.
    #[cfg(all(feature = "precompiled-shaders", feature = "shaderc-online"))]
    fn sig(s: &ShcShader) -> String {
        use std::fmt::Write as _;
        let mut o = String::new();
        let _ = write!(
            o,
            "{:?}",
            (
                (s.usc_dw, s.temps, s.num_varyings, s.uses_w, s.uses_discard, s.reads_front_facing, s.meta_off),
                (s.fs_front_face_off1, s.num_outputs, s.num_tile_buffers, s.tile_buffer_sh, s.num_attribs),
                (s.vs_output_size, s.vs_psize_off, s.vs_vertex_id_reg1, s.vs_instance_id_reg1, s.uniform_dwords),
                (s.uniform_sh_dwords, s.num_uniforms, s.blend_key, s.uses_fbfetch, s.blend_consts_off, s.uses_pntc),
                (s.uses_z, s.fragcoord_ytf_off1, s.native_cube, s.layout_at_link, s.num_declared),
            )
        );
        let _ = write!(o, " code {:?}", s.usc_words());
        for v in &s.varyings {
            let _ = write!(o, " v{:?}", (v.location, v.components, v.interp, v.vtxout_off, v.name));
        }
        for x in &s.attribs {
            let _ = write!(o, " a{:?}", (x.location, x.components, x.vi_reg, x.format, x.name, x.gl_type));
        }
        for u in s.uniform_table() {
            let _ = write!(o, " u{:?}", (u.name, u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding, u.inner));
        }
        if !s.declared.is_null() {
            // SAFETY: the shader's list of num_declared entries.
            for d in unsafe { core::slice::from_raw_parts(s.declared, usize::from(s.num_declared)) } {
                let _ = write!(o, " d{:?}", (d.name, d.location, d.columns, d.gl_type, d.active));
            }
        }
        #[cfg(feature = "gpu-vivante")]
        {
            let v = &s.viv;
            let _ = write!(
                o,
                " viv{:?}",
                (v.num_in, v.num_out, v.vs_reg_of_in, v.vs_pos_out_reg, v.vs_pointsize_out_reg, v.vs_id_in_reg),
            );
            let _ = write!(
                o,
                "{:?}",
                (
                    v.ps_color_out_reg,
                    v.ps_depth_out_reg,
                    v.vs_load_balancing,
                    v.input_count_unk8,
                    v.const_contents(),
                    v.const_data()
                )
            );
            for x in v.in_.iter().chain(v.out.iter()) {
                let _ = write!(o, " io{:?}", (x.reg, x.slot, x.components, x.interp, x.name));
            }
        }
        o
    }

    /// `compile(stage, text)` through the back end; the shader and the
    /// compiler runs it cost.
    #[cfg(all(feature = "precompiled-shaders", feature = "shaderc-online"))]
    fn compile(stage: u32, text: &str) -> (ShcShader, u32) {
        let src = std::format!("{text}\0");
        let mut out = ShcShader::ZERO;
        let mut log = [0 as core::ffi::c_char; 256];
        let f = crate::shaderc::online::SHC_ONLINE.compile.unwrap();
        let n0 = crate::shaderc::online::store::shc_online_compiler_runs();
        // SAFETY: a NUL-terminated source, a real output and the log's length.
        let rc = unsafe { f(stage, src.as_ptr() as *const core::ffi::c_char, &mut out, log.as_mut_ptr(), log.len()) };
        assert_eq!(rc, 0, "{}", log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect::<String>());
        (out, crate::shaderc::online::store::shc_online_compiler_runs().wrapping_sub(n0))
    }

    /// The table's own text of row `i`, without its NUL.
    #[cfg(feature = "precompiled-shaders")]
    fn text_of(i: usize) -> &'static str {
        let t = table::SHC_PRECOMPILED_SOURCES[usize::from(table::SHC_PRECOMPILED_SOURCE[i])].text;
        t.strip_suffix('\0').unwrap_or(t)
    }

    /// A source the table holds compiles to its row with no compiler run,
    /// and the row is, field for field, what the compiler makes of it with
    /// the table out of the way; a source the table does not hold compiles.
    #[test]
    #[cfg(all(feature = "precompiled-shaders", feature = "shaderc-online"))]
    fn a_hit_runs_no_compile_and_is_what_the_compiler_makes() {
        let _serial = crate::shaderc::online::store::test_lock();
        let plain: Vec<usize> = (0..rows())
            .filter(|&i| {
                table::SHC_PRECOMPILED_TABLE[i].blend_key == 0
                    && table::SHC_PRECOMPILED_KEYS[i] == crate::shaderc::sha256::sha256(text_of(i).as_bytes())
            })
            .collect();
        assert!(!plain.is_empty(), "the table has plain rows");
        for &i in &plain {
            let stage = table::SHC_PRECOMPILED_TABLE[i].stage;
            let hits = shc_precompiled_hits();
            let (hit, runs) = compile(stage, text_of(i));
            assert_eq!(runs, 0, "row {i}: a hit runs no compile");
            assert_eq!(shc_precompiled_hits(), hits.wrapping_add(1), "and is counted");
            assert!(core::ptr::eq(hit.priv_, &table::SHC_PRECOMPILED_TABLE[i] as *const ShcBlob as *const _));
            assert!(crate::shaderc::online::backend::is_ours(&hit), "a row is the compiler's");
            shc_precompiled_set_enabled(0);
            let (made, runs) = compile(stage, text_of(i));
            shc_precompiled_set_enabled(1);
            assert_eq!(runs, 1, "with the table off, a compile");
            assert!(sig(&hit) == sig(&made), "row {i}: the row is not what the compiler makes of its source");
            crate::shaderc::release_shader(&made);
            crate::shaderc::release_shader(&hit);
        }
        let miss = "precision mediump float; void main(){ gl_FragColor = vec4(0.3125); } // no row has me\n";
        let hits = shc_precompiled_hits();
        let (sh, runs) = compile(SHC_STAGE_FRAGMENT, miss);
        assert_eq!((runs, shc_precompiled_hits()), (1, hits), "a miss compiles");
        crate::shaderc::release_shader(&sh);
    }

    /// Every program the table recorded, replayed in the GL core's order --
    /// both shaders compiled, linked, the sampler-state question asked of
    /// both, and each of its fragment shader's recorded variants asked and
    /// linked -- runs no compiler at all with the table, and the compiler
    /// every step without it; and each step hands out the same shader (the
    /// same sensitivity) either way. Prints both counts.
    #[test]
    #[cfg(all(feature = "precompiled-shaders", feature = "shaderc-online"))]
    fn the_tables_programs_run_no_compiler() {
        let _serial = crate::shaderc::online::store::test_lock();
        let b = &crate::shaderc::online::SHC_ONLINE;
        let row = |key: &[u8; 32], blend: u32| {
            (0..rows())
                .find(|&i| &table::SHC_PRECOMPILED_KEYS[i] == key && table::SHC_PRECOMPILED_TABLE[i].blend_key == blend)
        };
        /* the programs: each plain link (blend 0) whose shaders are plain rows */
        let programs: Vec<(usize, usize)> = table::SHC_PRECOMPILED_LINKS
            .iter()
            .filter(|l| l.fs_blend_key == 0)
            .filter_map(|l| Some((row(&l.vs, 0)?, row(&l.fs, 0)?)))
            .filter(|&(v, f)| {
                [v, f]
                    .iter()
                    .all(|&i| table::SHC_PRECOMPILED_KEYS[i] == crate::shaderc::sha256::sha256(text_of(i).as_bytes()))
            })
            .collect();
        assert!(!programs.is_empty(), "the table recorded programs");
        let replay = || -> (u32, Vec<String>) {
            let mut made = Vec::new();
            let n0 = crate::shaderc::online::store::shc_online_compiler_runs();
            for &(v, f) in &programs {
                let (vs, _) = compile(SHC_STAGE_VERTEX, text_of(v));
                let (fs, _) = compile(SHC_STAGE_FRAGMENT, text_of(f));
                crate::shaderc::retain_shader(&fs);
                let mut rfs = fs;
                let mut log = [0 as core::ffi::c_char; 256];
                // SAFETY: two shaders this back end made, and the log's length.
                assert_eq!(unsafe { (b.link.unwrap())(&vs, &mut rfs, log.as_mut_ptr(), log.len()) }, 0);
                for sh in [&vs, &rfs] {
                    // SAFETY: a shader this back end made.
                    let bits = unsafe { (b.lod_sensitivity.unwrap())(sh) };
                    made.push(std::format!("{} sensitivity {bits:#x}", sig(sh)));
                }
                /* the variants the generator asked of this program: its links
                 * of a variant of the same fragment source to the same vertex shader */
                let mut keys: Vec<u32> = table::SHC_PRECOMPILED_LINKS
                    .iter()
                    .filter(|l| l.fs_blend_key != 0 && l.vs == table::SHC_PRECOMPILED_KEYS[v])
                    .filter(|l| row(&l.fs, l.fs_blend_key).is_some_and(|vi| text_of(vi) == text_of(f)))
                    .map(|l| l.fs_blend_key)
                    .collect();
                keys.sort_unstable();
                keys.dedup();
                for k in keys {
                    let mut var = ShcShader::ZERO;
                    // SAFETY: a shader this back end made, a real output, the log's length.
                    let rc = unsafe { (b.blend_variant.unwrap())(&rfs, k, &mut var, log.as_mut_ptr(), log.len()) };
                    assert_eq!(rc, 0);
                    // SAFETY: as the link above.
                    assert_eq!(unsafe { (b.link.unwrap())(&vs, &mut var, log.as_mut_ptr(), log.len()) }, 0);
                    made.push(sig(&var));
                    crate::shaderc::release_shader(&var);
                }
                for sh in [&rfs, &fs, &vs] {
                    crate::shaderc::release_shader(sh);
                }
            }
            (crate::shaderc::online::store::shc_online_compiler_runs().wrapping_sub(n0), made)
        };
        let (with, hits) = replay();
        shc_precompiled_set_enabled(0);
        let (without, compiled) = replay();
        shc_precompiled_set_enabled(1);
        assert_eq!(hits.len(), compiled.len());
        for (i, (h, c)) in hits.iter().zip(&compiled).enumerate() {
            assert!(h == c, "step {i} of the replay: the table's shader is not the compiler's");
        }
        std::println!(
            "precompiled: {} programs, {} shaders: {with} compiler runs with the table, {without} without",
            programs.len(),
            hits.len()
        );
        assert_eq!(with, 0, "the table answers every step of its own programs");
        assert!(without >= 2 * programs.len() as u32, "without it each program compiles");
    }

    /* ---- an image without the compiler ------------------------------------ */

    /// `compile(stage, text)` through the image's back end: the return code,
    /// the shader and the log.
    #[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
    fn alone_compile(stage: u32, text: &str) -> (i32, ShcShader, String) {
        let src = std::format!("{text}\0");
        let mut out = ShcShader::ZERO;
        let mut log = [0 as core::ffi::c_char; 512];
        let f = SHC_PRECOMPILED.compile.unwrap();
        // SAFETY: a NUL-terminated source, a real output and the log's length.
        let rc = unsafe { f(stage, src.as_ptr() as *const core::ffi::c_char, &mut out, log.as_mut_ptr(), log.len()) };
        (rc, out, log_of(&log))
    }

    #[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
    fn log_of(log: &[core::ffi::c_char]) -> String {
        log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
    }

    /// Without the compiler a source is its row or a refusal by name: the
    /// table's own text is found (no compile exists to run), a text it does
    /// not hold is -ENOENT with the agreed line, sha256 and list named.
    #[test]
    #[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
    fn without_the_compiler_a_source_is_its_row_or_refused_by_name() {
        let i = (0..rows())
            .find(|&i| {
                table::SHC_PRECOMPILED_TABLE[i].blend_key == 0
                    && table::SHC_PRECOMPILED_KEYS[i] == crate::shaderc::sha256::sha256(text_of(i).as_bytes())
            })
            .expect("a plain row");
        let (rc, sh, log) = alone_compile(table::SHC_PRECOMPILED_TABLE[i].stage, text_of(i));
        assert_eq!((rc, log.as_str()), (0, ""));
        assert!(core::ptr::eq(sh.priv_, &table::SHC_PRECOMPILED_TABLE[i] as *const ShcBlob as *const _));
        assert!(crate::shaderc::is_online(&sh), "a row is the compiler's work (its uniform rows are)");
        let miss = "precision mediump float; void main(){ gl_FragColor = vec4(0.4375); } // no row has me\n";
        let (rc, _, log) = alone_compile(SHC_STAGE_FRAGMENT, miss);
        assert_eq!(rc, -crate::util::ENOENT);
        let sha: String =
            crate::shaderc::sha256::sha256(miss.as_bytes()).iter().map(|b| std::format!("{b:02x}")).collect();
        assert_eq!(
            log,
            std::format!(
                "precompiled shaders: fragment shader sha256={sha} is not in this image's precompiled shaders and \
                 the image has no shader compiler; add it to {} and rebuild",
                table::SHC_PRECOMPILED_LIST
            )
        );
    }

    /// gl_test's ES 1.00 passthrough vertex shader is in the gl_test_es100
    /// app's table: an image without the compiler finds it. (Take it out of
    /// the list and this test sees the refusal.)
    #[test]
    #[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
    fn a_listed_source_is_found_without_the_compiler() {
        let text = include_str!("../../tools/shaders/gl_test_es100/passthrough.vert");
        let (rc, _, log) = alone_compile(SHC_STAGE_VERTEX, text);
        assert_eq!(rc, 0, "{log}");
    }

    /// A variant the table does not hold, and a program it did not link, are
    /// each refused by name, -ENOENT; a program it linked links.
    #[test]
    #[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
    fn without_the_compiler_an_unlisted_variant_or_program_is_refused_by_name() {
        let b = &SHC_PRECOMPILED;
        let plain = |i: usize| {
            table::SHC_PRECOMPILED_TABLE[i].blend_key == 0
                && table::SHC_PRECOMPILED_KEYS[i] == crate::shaderc::sha256::sha256(text_of(i).as_bytes())
        };
        let row = |key: &[u8; 32]| (0..rows()).find(|&i| plain(i) && &table::SHC_PRECOMPILED_KEYS[i] == key);
        let l = table::SHC_PRECOMPILED_LINKS
            .iter()
            .find(|l| l.fs_blend_key == 0 && row(&l.vs).is_some() && row(&l.fs).is_some())
            .expect("a recorded program of two plain rows");
        let (v, f) = (row(&l.vs).unwrap(), row(&l.fs).unwrap());
        let (_, vs, _) = alone_compile(SHC_STAGE_VERTEX, text_of(v));
        let (_, fs, _) = alone_compile(SHC_STAGE_FRAGMENT, text_of(f));
        let mut log = [0 as core::ffi::c_char; 512];
        let mut linked = fs;
        // SAFETY: two shaders this back end handed out, and the log's length.
        let rc = unsafe { (b.link.unwrap())(&vs, &mut linked, log.as_mut_ptr(), log.len()) };
        assert_eq!(rc, 0, "a recorded program links: {}", log_of(&log));

        /* a blend key no row has */
        let mut out = ShcShader::ZERO;
        // SAFETY: a row, a real output and the log's length.
        let rc = unsafe { (b.blend_variant.unwrap())(&linked, 0x1234_5678, &mut out, log.as_mut_ptr(), log.len()) };
        let name = view(&linked).unwrap().name;
        assert_eq!(rc, -crate::util::ENOENT);
        assert_eq!(log_of(&log), std::format!("precompiled shaders: {name} has no variant for blend key 0x12345678"));

        /* a vertex shader and a fragment shader the table never linked, neither link-invariant */
        let other = (0..rows()).find(|&i| {
            plain(i)
                && table::SHC_PRECOMPILED_TABLE[i].stage == SHC_STAGE_FRAGMENT
                && table::SHC_PRECOMPILED_TABLE[i].shader.num_uniforms != 0
                && !table::SHC_PRECOMPILED_LINKS.iter().any(|x| x.vs == l.vs && x.fs == table::SHC_PRECOMPILED_KEYS[i])
        });
        let other = other.expect("a fragment shader the table never linked to this vertex shader");
        let (_, mut fs2, _) = alone_compile(SHC_STAGE_FRAGMENT, text_of(other));
        // SAFETY: as above.
        let rc = unsafe { (b.link.unwrap())(&vs, &mut fs2, log.as_mut_ptr(), log.len()) };
        assert_eq!(rc, -crate::util::ENOENT);
        assert_eq!(
            log_of(&log),
            std::format!(
                "precompiled shaders: program {}/{} was not linked at build time",
                view(&vs).unwrap().name,
                view(&fs2).unwrap().name
            )
        );
    }

    /// What a GL client sees of a refusal: COMPILE_STATUS false with the line
    /// in the info log, a program of it that does not link, and
    /// GL_INVALID_OPERATION when it is used.
    #[test]
    #[cfg(all(feature = "precompiled-shaders", not(feature = "shaderc-online")))]
    fn a_gl_client_sees_the_refusal() {
        use crate::gl::consts::*;
        use crate::gl::program::{glAttachShader, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram};
        use crate::gl::shader::{glCompileShader, glCreateShader, glGetShaderInfoLog, glGetShaderiv, glShaderSource};
        use crate::gl::types::*;
        let gl = crate::testutil::TestGl::new(8, 8);
        let text = std::ffi::CString::new("precision mediump float; void main(){ gl_FragColor = vec4(0.5); } // gl\n")
            .unwrap();
        let vtext = std::ffi::CString::new(include_str!("../../tools/shaders/gl_test_es100/passthrough.vert")).unwrap();
        // SAFETY: plain GL calls on the test's context; every pointer is valid for the call.
        unsafe {
            let compile = |kind: GLenum, t: &std::ffi::CString| {
                let s = glCreateShader(kind);
                let p = [t.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                s
            };
            let fs = compile(GL_FRAGMENT_SHADER, &text);
            let mut ok: GLint = -1;
            glGetShaderiv(fs, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, GL_FALSE as GLint, "a refused shader does not compile");
            let mut buf = [0u8; 512];
            let mut len: GLsizei = 0;
            glGetShaderInfoLog(fs, buf.len() as GLsizei, &mut len, buf.as_mut_ptr() as *mut GLchar);
            let log = core::str::from_utf8(&buf[..len as usize]).unwrap();
            assert!(
                log.starts_with("precompiled shaders: fragment shader sha256=")
                    && log.ends_with(
                        "the image has no shader compiler; add it to tools/shaders/precompiled.list and rebuild"
                    ),
                "{log}"
            );
            let vs = compile(GL_VERTEX_SHADER, &vtext);
            glGetShaderiv(vs, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, GL_TRUE as GLint, "the listed vertex shader compiles");
            let p = glCreateProgram();
            glAttachShader(p, vs);
            glAttachShader(p, fs);
            glLinkProgram(p);
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, GL_FALSE as GLint, "a program of a refused shader does not link");
            glUseProgram(p);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }
}
