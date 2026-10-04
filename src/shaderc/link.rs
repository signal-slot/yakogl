// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The link rule both shader back ends apply.
//!
//! `ShcShader`'s vertex section stores only `vs_output_size`, not the list of
//! output varyings, so the check is: every FS input varying lands inside the
//! VS output vector (`vtxout_off + components <= vs_output_size`, and past
//! `gl_Position`, which occupies dwords 0..3), and a named input takes the
//! offset of the VS output of the same name. Exact per-location component
//! matching against the VS's individual outputs is not expressible with the
//! current header (noted in `SHADERC_NOTES.md`); when that field is added this
//! check can be tightened.
//!
//! It lives here rather than in `offline.rs` because the remote back end runs
//! the identical rule on the identical structs — it compiles the shaders on a
//! host instead of reading them from a table, which changes nothing about how
//! the two stages are wired together. Only the name the log is prefixed with
//! differs, so the caller passes it. (This module is this port's `shc_link.c`.)

use core::ffi::c_char;
use core::fmt::Write;

use super::{ShcShader, ShcVarying};
#[cfg(feature = "gpu-vivante")]
use super::{SHC_VIV_MAX_IO, SHC_VIV_NO_REG, SHC_VIV_POINT_COORD};
use crate::util::{name_bytes, Bytes, CStrWriter, EINVAL};

/// `gl_Position` occupies the first four dwords of the VS output vector.
pub const VS_POS_DWORDS: u32 = 4;

/// Two entries whose stored names read back equal, if there are any.
///
/// GLSL cannot declare one name twice, so this never means what it says: it
/// means the fixed-size name field truncated two DIFFERENT names into the
/// same bytes, and the reflection has lost the difference. Pairing then takes
/// whichever came first and the shader renders, wrongly and silently.
///
/// It is not a hypothetical margin. The longest varying name we ship is
/// `qt_BaseColorMap_uv_coords1`, 26 of the field's 32 characters, and Qt
/// Quick 3D builds these as `qt_<MapName>_uv_coords<N>` -- so the headroom is
/// a few characters of map name. Task #47 removes the limit; until then, a
/// link that cannot be done correctly is refused rather than guessed.
fn duplicate_name<'a, T>(items: &'a [T], name: impl Fn(&'a T) -> &'a [u8]) -> Option<&'a [u8]> {
    for (i, a) in items.iter().enumerate() {
        let na = name(a);
        if na.is_empty() {
            continue;
        }
        if items[..i].iter().any(|b| name(b) == na) {
            return Some(na);
        }
    }
    None
}

/// A name that uses every byte its field can store, if there is one.
///
/// Both writers of these fields reserve the last byte for the NUL --
/// `util::set_name` copies `src.len().min(field.len() - 1)`, `util::cname`
/// stops at `i + 1 < N` -- so a field is never *full*: a 40-character name
/// and a 31-character one are stored identically, as 31 bytes and a NUL.
/// The longest storable name is therefore the signature of truncation, and
/// the only one there is. Checking for a missing NUL instead would be a check
/// that can never fire.
///
/// A cut name is wrong in three different ways: `glGetActiveAttrib` reports
/// it (and dEQP compares that string exactly), `glGetUniformLocation` matches
/// on it, and the two stages are PAIRED on it. The last renders the wrong
/// picture rather than raising an error, which is why this refuses the shader
/// instead of logging and continuing. Pairing is also why the duplicate-name
/// check below does not subsume this one: a VS output and an FS input that
/// come from *different* GLSL varyings truncated alike are a duplicate in
/// neither stage's own table, and pair silently.
///
/// A name of exactly the longest storable length is refused too, although it
/// is intact: nothing in the table distinguishes the two cases, and guessing
/// is what this is here to stop. The measured margin says how often that
/// costs anything -- the longest varying we ship is `qt_BaseColorMap_uv_coords1`
/// at 26 of 32, and Qt Quick 3D generates the family as
/// `qt_<MapName>_uv_coords<N>`. Task #47's name pool removes the limit, and
/// lets the generator refuse truncation where it can still tell.
fn name_maxes_field(field: &[c_char]) -> bool {
    !field.is_empty() && name_bytes(field).len() == field.len() - 1
}

/// Every name of one stage, refused if any may have been truncated.
fn reject_truncated_names(sh: &ShcShader, log: &mut CStrWriter, who: &str, stage: &str) -> i32 {
    let varyings = &sh.varyings[..(sh.num_varyings as usize).min(sh.varyings.len())];
    if let Some(v) = varyings.iter().find(|v| name_maxes_field(&v.name)) {
        return reject_truncated(log, who, stage, "varying", &v.name);
    }
    let attribs = &sh.attribs[..(sh.num_attribs as usize).min(sh.attribs.len())];
    if let Some(a) = attribs.iter().find(|a| name_maxes_field(&a.name)) {
        return reject_truncated(log, who, stage, "attribute", &a.name);
    }
    if let Some(u) = sh.uniform_table().iter().find(|u| name_maxes_field(&u.name)) {
        return reject_truncated(log, who, stage, "uniform", &u.name);
    }
    0
}

fn reject_truncated(log: &mut CStrWriter, who: &str, stage: &str, what: &str, field: &[c_char]) -> i32 {
    let _ = write!(
        log,
        "{who}: the {stage} {what} named '{}' is as long as the reflection's {}-byte name field \
         stores, so it may have been truncated -- and a truncated name is reported, matched and \
         PAIRED on (yakogl task #47)",
        Bytes(name_bytes(field)),
        field.len()
    );
    -EINVAL
}

/// The message both GPUs' link arms give for it.
fn reject_duplicate(log: &mut CStrWriter, who: &str, what: &str, dup: &[u8]) -> i32 {
    let _ = write!(
        log,
        "{who}: two {what} are both named '{}': the reflection's fixed name field truncated two \
         different names together, so they cannot be paired (yakogl task #47)",
        Bytes(dup)
    );
    -EINVAL
}

/// Pair the FS inputs with the VS outputs and bounds-check them. `who` names
/// the back end in the log (`"offline shaderc"`, `"remote shaderc"`).
pub fn by_name(vs: &ShcShader, fs: &mut ShcShader, log: &mut CStrWriter, who: &str) -> i32 {
    let vs_outputs: &[ShcVarying] = &vs.varyings[..(vs.num_varyings as usize).min(vs.varyings.len())];
    let num_inputs = (fs.num_varyings as usize).min(fs.varyings.len());

    /* before pairing by name, make sure the names still distinguish them */
    if let Some(d) = duplicate_name(vs_outputs, |o| name_bytes(&o.name)) {
        return reject_duplicate(log, who, "vertex outputs", d);
    }
    if let Some(d) = duplicate_name(&fs.varyings[..num_inputs], |i| name_bytes(&i.name)) {
        return reject_duplicate(log, who, "fragment inputs", d);
    }

    /* GLSL ES 1.00 pairs: a fragment input reads the vertex output of the
     * same name, wherever each shader placed it. A matrix varying is listed
     * column by column, the later columns unnamed (pco's record and the
     * online compiler's both), and the vertex shader packs its columns
     * consecutively: an unnamed input follows the one before it. Skipping
     * it left a later column at the fragment shader's own packing, which is
     * the vertex shader's only when nothing precedes the matrix there. */
    let mut prev: Option<(u8, u8)> = None;
    for v in fs.varyings[..num_inputs].iter_mut() {
        if v.name[0] == 0 {
            /* A column has one to four components; an empty entry (a
             * count past what the record filled) keeps its own offset for
             * the checks below to refuse. */
            if let Some((off, comps)) = prev.filter(|_| (1..=4).contains(&v.components)) {
                v.vtxout_off = off + comps;
                prev = Some((v.vtxout_off, v.components));
            }
            continue;
        }
        let o = vs_outputs.iter().find(|o| name_bytes(&o.name) == name_bytes(&v.name));
        match o {
            Some(o) if o.components >= v.components => {
                v.vtxout_off = o.vtxout_off;
                prev = Some((v.vtxout_off, v.components));
            }
            _ => {
                let _ = write!(
                    log,
                    "{who}: fragment varying '{}{}",
                    Bytes(name_bytes(&v.name)),
                    if o.is_some() {
                        "' is narrower in the vertex shader"
                    } else {
                        "' is not written by the vertex shader"
                    }
                );
                return -EINVAL;
            }
        }
    }

    if (vs.vs_output_size as u32) < VS_POS_DWORDS {
        let _ = write!(log, "{who}: VS emits no gl_Position (vs_output_size={})", vs.vs_output_size);
        return -EINVAL;
    }

    for v in fs.varyings[..num_inputs].iter() {
        let end = v.vtxout_off as u32 + v.components as u32;

        if (v.vtxout_off as u32) < VS_POS_DWORDS {
            let _ =
                write!(log, "{who}: FS varying loc{} overlaps gl_Position (vtxout_off={})", v.location, v.vtxout_off);
            return -EINVAL;
        }
        if end > vs.vs_output_size as u32 {
            let _ = write!(
                log,
                "{who}: FS varying loc{} reads VS output dwords up to {} but VS emits only {}",
                v.location, end, vs.vs_output_size
            );
            return -EINVAL;
        }
    }

    0
}

/// What both back ends' `struct shc_backend.link` reduces to: the null
/// checks, then [`by_name`].
///
/// # Safety
/// `vs` and `fs` are null or point at distinct valid `ShcShader` objects;
/// `log` is null or valid for `log_sz` bytes.
pub unsafe fn link_shaders(
    vs: *const ShcShader,
    fs: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
    who: &str,
) -> i32 {
    // SAFETY: the caller passes a null log or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };

    if vs.is_null() || fs.is_null() {
        let _ = write!(log, "{who}: NULL VS/FS in link");
        return -EINVAL;
    }
    // SAFETY: both are non-null and point at distinct shc_shader objects (the
    // program's VS and its private FS copy) that nothing else touches during
    // the call.
    let (vs, fs) = unsafe { (&*vs, &mut *fs) };
    let rc = reject_truncated_names(vs, &mut log, who, "vertex");
    if rc != 0 {
        return rc;
    }
    let rc = reject_truncated_names(fs, &mut log, who, "fragment");
    if rc != 0 {
        return rc;
    }
    by_name(vs, fs, &mut log, who)
}

/* ---- the V3D 4.2 ---------------------------------------------------------- *
 *
 * The V3D pairs nothing at this level: the program compile (at link, in the
 * online back end) lays the vertex shader's outputs out in the order the
 * fragment shader reads them, by name, and refuses a fragment input no vertex
 * output of that name feeds. What is left for the table rule is what both
 * GPUs' rules share: the names in the reflection must be the names in the
 * sources. A cut or doubled name would be matched on by
 * glGetUniformLocation/glGetAttribLocation and reported by glGetActive*, so it
 * is refused here, before the program is built.
 */

/// The V3D's `struct shc_backend.link` table rule: the null checks and the
/// name checks of [`link_shaders`], without the PowerVR's vertex output
/// offsets (the program compile places the outputs).
///
/// # Safety
/// As [`link_shaders`].
#[cfg(feature = "gpu-v3d")]
pub unsafe fn link_shaders_v3d(
    vs: *const ShcShader,
    fs: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
    who: &str,
) -> i32 {
    // SAFETY: the caller passes a null log or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };
    if vs.is_null() || fs.is_null() {
        let _ = write!(log, "{who}: NULL VS/FS in link");
        return -EINVAL;
    }
    // SAFETY: both are non-null and point at valid shc_shader objects.
    let (vs, fs) = unsafe { (&*vs, &*fs) };
    for (sh, stage) in [(vs, "vertex"), (fs, "fragment")] {
        let rc = reject_truncated_names(sh, &mut log, who, stage);
        if rc != 0 {
            return rc;
        }
        let varyings = &sh.varyings[..(sh.num_varyings as usize).min(sh.varyings.len())];
        if let Some(d) = duplicate_name(varyings, |v| name_bytes(&v.name)) {
            let what = if stage == "vertex" { "vertex outputs" } else { "fragment inputs" };
            return reject_duplicate(&mut log, who, what, d);
        }
    }
    0
}

/* ---- the same question on the Vivante GC7000 --------------------------- *
 *
 * That GPU pairs the stages by register: the fragment shader reads varying i
 * from its input register i+1, and the vertex shader's output map has to name
 * the register that feeds it. Matching is by GLSL name (what `varying` means
 * in GLSL ES 1.00), falling back to the slot when a shader carries none.
 *
 * It lives beside `by_name` for the reason that module doc gives: both back
 * ends must apply one rule. The offline back end had this arm and the remote
 * one did not, so a `gpu-vivante` build with `YAKOGL_SHADERC=remote` ran the
 * PowerVR check instead -- and since `vs_output_size` is unused on this GPU
 * and always 0, *every* program failed to link with "VS emits no gl_Position".
 */
#[cfg(feature = "gpu-vivante")]
pub fn by_register(vs: &ShcShader, fs: &mut ShcShader, log: &mut CStrWriter, who: &str) -> i32 {
    let outs = &vs.viv.out[..(vs.viv.num_out as usize).min(SHC_VIV_MAX_IO)];

    /* The registers this GPU has for the varyings the program uses. This
     * is the machine's rule, not the language's: the GC7000 gives every
     * varying a register of its own, so it counts varyings where GLSL ES
     * 1.00 Appendix A 7 packs them (`packing`, applied before this by the
     * back ends that know the varyings' types), and it refuses more than 15
     * floats although they pack into four rows (SHADERC_NOTES.md, "The
     * resource budget"). Counted twice: as rows against MAX_VARYING_VECTORS
     * and as components against the component limits (ES 3.0 2.12.8).
     * Nothing past the link may be the first to see an out-of-range
     * program: the ninth vec4 varying once panicked in the draw (6719382). */
    let used = &fs.viv.in_[..(fs.viv.num_in as usize).min(SHC_VIV_MAX_IO)];
    let components: usize = used.iter().map(|i| i.components as usize).sum();
    let max_vectors = crate::gl::context::MAX_VARYING_VECTORS;
    if used.len() > max_vectors || components > max_vectors * 4 {
        let _ = write!(
            log,
            "{who}: {} varyings, {components} components, exceed GL_MAX_VARYING_VECTORS {max_vectors} / GL_MAX_VARYING_COMPONENTS {}",
            used.len(),
            max_vectors * 4
        );
        return -EINVAL;
    }

    for i in 0..(fs.viv.num_in as usize).min(SHC_VIV_MAX_IO) {
        let in_ = fs.viv.in_[i];
        /* gl_PointCoord's row has no vertex output: the rasteriser makes it
         * (Mesa's etnaviv gives it VS register 0, which the driver does for
         * NO_REG). A `gl_` name is no user varying, so this is only ever that
         * row. It is counted in the limits above, as GLSL ES 1.00 Appendix A
         * 7 counts a referenced gl_PointCoord. */
        if name_bytes(&in_.name) == SHC_VIV_POINT_COORD.as_bytes() {
            fs.viv.vs_reg_of_in[i] = SHC_VIV_NO_REG;
            continue;
        }
        let out = outs.iter().find(|o| {
            if in_.name[0] != 0 && o.name[0] != 0 {
                in_.name() == o.name()
            } else {
                in_.slot == o.slot
            }
        });
        fs.viv.vs_reg_of_in[i] = out.map_or(SHC_VIV_NO_REG, |o| o.reg);
        if !out.is_some_and(|o| o.components >= in_.components) {
            let _ = write!(
                log,
                "{who}: fragment varying {i} '{}{}",
                Bytes(name_bytes(&in_.name)),
                if out.is_some() {
                    "' is narrower in the vertex shader"
                } else {
                    "' is not written by the vertex shader"
                }
            );
            return -EINVAL;
        }
    }
    0
}

/// `by_register` behind the C signature both back ends register.
///
/// # Safety
/// `vs` and `fs` are null or point at distinct valid `ShcShader` objects;
/// `log` is null or valid for `log_sz` bytes.
#[cfg(feature = "gpu-vivante")]
pub unsafe fn link_shaders_viv(
    vs: *const ShcShader,
    fs: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
    who: &str,
) -> i32 {
    // SAFETY: the caller passes a null log or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };

    if vs.is_null() || fs.is_null() {
        let _ = write!(log, "{who}: NULL VS/FS in link");
        return -EINVAL;
    }
    // SAFETY: both are non-null and point at distinct shc_shader objects.
    let (vs, fs) = unsafe { (&*vs, &mut *fs) };
    let rc = reject_truncated_names(vs, &mut log, who, "vertex");
    if rc != 0 {
        return rc;
    }
    let rc = reject_truncated_names(fs, &mut log, who, "fragment");
    if rc != 0 {
        return rc;
    }
    by_register(vs, fs, &mut log, who)
}

/// The truncation and duplicate guards, on both GPUs' link arms.
///
/// `offline.rs` has the PowerVR pairing tests, but its whole test module is
/// compiled out under `gpu-vivante`, so the GC7000 arm had no coverage of
/// these at all -- and the guards are the same code on both paths precisely
/// so that a Qt shader cannot be mis-wired on one GPU and refused on the
/// other.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::{ShcAttrib, ShcUniform, SHC_UNIFORM_NO_BLOCK};
    use crate::util::{cstr_bytes, set_name};
    use std::string::String;

    /// A one-varying stage, named.
    fn with_varying(name: &[u8]) -> ShcShader {
        let mut s = ShcShader::ZERO;
        set_name(&mut s.varyings[0].name, name);
        s.varyings[0].components = 2;
        /* Past gl_Position, which owns the first four dwords of the vector. */
        s.varyings[0].vtxout_off = 4;
        s.num_varyings = 1;
        s.vs_output_size = 6;
        s
    }

    fn with_attrib(name: &[u8]) -> ShcShader {
        let mut s = ShcShader::ZERO;
        s.attribs[0] = ShcAttrib::ZERO;
        set_name(&mut s.attribs[0].name, name);
        s.num_attribs = 1;
        s.vs_output_size = 4;
        s
    }

    fn uniform(name: &[u8]) -> ShcUniform {
        let mut u = ShcUniform::ZERO;
        set_name(&mut u.name, name);
        u.dword_off = SHC_UNIFORM_NO_BLOCK;
        u
    }

    /// Run whichever link arm this build has, and return its code and log.
    ///
    /// # Safety
    /// `vs` and `fs` outlive the call, as do any uniform tables they point at.
    unsafe fn run(vs: &ShcShader, fs: &mut ShcShader) -> (i32, String) {
        let mut log = [0x55 as c_char; 256];
        // SAFETY: two distinct valid shaders and a log of the stated size.
        let rc = unsafe {
            #[cfg(feature = "gpu-vivante")]
            {
                link_shaders_viv(vs, fs, log.as_mut_ptr(), log.len(), "test")
            }
            #[cfg(not(feature = "gpu-vivante"))]
            {
                link_shaders(vs, fs, log.as_mut_ptr(), log.len(), "test")
            }
        };
        // SAFETY: the writer NUL-terminated the log.
        let text = unsafe { cstr_bytes(log.as_ptr()) };
        (rc, String::from_utf8(text.to_vec()).unwrap())
    }

    /// A program whose fragment shader reads `n` varyings of `comps`
    /// components, each written by the vertex shader by name.
    #[cfg(feature = "gpu-vivante")]
    fn viv_pair(n: usize, comps: u8) -> (ShcShader, ShcShader) {
        use crate::shaderc::ShcVivIo;
        let (mut vs, mut fs) = (ShcShader::ZERO, ShcShader::ZERO);
        for i in 0..n {
            let mut io = ShcVivIo { reg: 1 + i as u8, slot: 1 + i as u8, components: comps, interp: 0, name: [0; 32] };
            set_name(&mut io.name, format!("v{i}").as_bytes());
            vs.viv.out[i] = io;
            fs.viv.in_[i] = io;
        }
        vs.viv.num_out = n as u8;
        fs.viv.num_in = n as u8;
        (vs, fs)
    }

    /// A program using one varying more than GL_MAX_VARYING_VECTORS reports
    /// fails to link, with a log naming the limit; at the limit it links. By
    /// rows, since this machine gives each varying a register whatever its
    /// width: sixteen floats are refused though they are only 16 components.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_program_past_the_reported_varying_limits_fails_to_link() {
        let max = crate::gl::context::MAX_VARYING_VECTORS;
        let (vs, mut fs) = viv_pair(max, 4);
        // SAFETY: two valid shaders.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, 0, "{max} vec4 varyings, {} components, are within the limits: {log}", max * 4);
        for comps in [4u8, 1] {
            let (vs, mut fs) = viv_pair(max + 1, comps);
            // SAFETY: two valid shaders.
            let (rc, log) = unsafe { run(&vs, &mut fs) };
            assert_eq!(rc, -EINVAL, "{} varyings of {comps} must not link", max + 1);
            assert!(log.contains("GL_MAX_VARYING_VECTORS"), "the log names the limit: {log}");
        }
    }

    /// gl_PointCoord's fragment input row has no vertex output and links
    /// anyway, unpaired (NO_REG: the driver gives it VS register 0, as Mesa
    /// does), while the varying beside it still pairs; and it counts toward
    /// the limits, as GLSL ES 1.00 Appendix A 7 counts a referenced
    /// gl_PointCoord: the most varyings plus it is one row too many.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn gl_point_coord_links_unpaired_and_counts_toward_the_limits() {
        use crate::shaderc::{ShcVivIo, SHC_VIV_POINT_COORD};
        let with_pntc = |n: usize| {
            let (vs, mut fs) = viv_pair(n, 4);
            let mut p = ShcVivIo { reg: 1 + n as u8, slot: 25, components: 2, interp: 0, name: [0; 32] };
            set_name(&mut p.name, SHC_VIV_POINT_COORD.as_bytes());
            fs.viv.in_[n] = p;
            fs.viv.num_in = n as u8 + 1;
            (vs, fs)
        };
        let (vs, mut fs) = with_pntc(1);
        // SAFETY: two valid shaders.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, 0, "a vertex shader need not write gl_PointCoord: {log}");
        assert_eq!(fs.viv.vs_reg_of_in[0], 1, "the varying still pairs");
        assert_eq!(fs.viv.vs_reg_of_in[1], SHC_VIV_NO_REG, "gl_PointCoord is unpaired");

        let max = crate::gl::context::MAX_VARYING_VECTORS;
        let (vs, mut fs) = with_pntc(max);
        // SAFETY: two valid shaders.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, -EINVAL, "{max} varyings and gl_PointCoord are {} rows", max + 1);
        assert!(log.contains("GL_MAX_VARYING_VECTORS"), "{log}");
    }

    /// 31 characters: the most a 32-byte field stores.
    const MAXED: &[u8] = b"qt_SomeVeryLongMapName_uv_coord";

    /// A matrix varying's later columns are unnamed and follow the first:
    /// with a vec2 packed before the mat2 in the vertex shader, the
    /// fragment shader's two columns read dwords 6 and 8, not their own
    /// packing's 4 and 6.
    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn a_matrix_varyings_later_columns_follow_its_first() {
        let v = |name: &[u8], loc: u8, comps: u8, off: u8| {
            let mut x = ShcVarying::ZERO;
            set_name(&mut x.name, name);
            x.location = loc;
            x.components = comps;
            x.vtxout_off = off;
            x
        };
        let mut vs = ShcShader::ZERO;
        vs.varyings[0] = v(b"a", 0, 2, 4);
        vs.varyings[1] = v(b"m", 1, 2, 6);
        vs.varyings[2] = v(b"", 2, 2, 8);
        vs.num_varyings = 3;
        vs.vs_output_size = 10;
        let mut fs = ShcShader::ZERO;
        fs.varyings[0] = v(b"m", 0, 2, 4);
        fs.varyings[1] = v(b"", 1, 2, 6);
        fs.num_varyings = 2;
        // SAFETY: two valid shaders on this stack frame.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, 0, "{log}");
        assert_eq!((fs.varyings[0].vtxout_off, fs.varyings[1].vtxout_off), (6, 8));
    }

    #[test]
    fn a_varying_at_the_storable_limit_is_refused_on_this_gpus_link_arm() {
        let vs = with_varying(MAXED);
        let mut fs = with_varying(b"v_uv");
        // SAFETY: both shaders outlive the call and hold no uniform table.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, -EINVAL, "{log}");
        assert!(log.contains("vertex varying"), "the log names the stage and the kind: {log}");
        assert!(log.contains("32-byte name field"), "and the field: {log}");
        assert!(log.contains("qt_SomeVeryLongMapName_uv_coord"), "and the name: {log}");
    }

    #[test]
    fn a_32_and_a_33_character_name_are_the_same_31_bytes_and_both_refused() {
        /* Why the rule is "as long as the field stores" and not "fills the
         * field with no NUL": `set_name` copies `len().min(field.len() - 1)`
         * and `cname` stops at `i + 1 < N`, so neither writer can ever leave
         * a field without its NUL. These two names differ in the GLSL, are
         * identical in the table, and the guard has nothing else to go on. */
        let n32 = b"qt_SomeVeryLongMapName_uv_coords";
        let n33 = b"qt_SomeVeryLongMapName_uv_coords1";
        assert_eq!((n32.len(), n33.len()), (32, 33));
        let a = with_varying(n32);
        let b = with_varying(n33);
        assert_eq!(name_bytes(&a.varyings[0].name), name_bytes(&b.varyings[0].name));
        assert_eq!(name_bytes(&a.varyings[0].name), MAXED, "both cut to the same 31 bytes");
        for vs in [&a, &b] {
            let mut fs = with_varying(b"v_uv");
            // SAFETY: both shaders outlive the call.
            assert_eq!(unsafe { run(vs, &mut fs) }.0, -EINVAL);
        }
    }

    #[test]
    fn a_uniform_at_the_storable_limit_is_refused() {
        /* 63 of 64. The margin here is the smallest of the four fields: the
         * longest name we ship is 59 characters. */
        let name = b"ubDirectionalLights.directionalLights[0].dimensionsInvertedXXXX";
        assert_eq!(name.len(), 63);
        let tab = [uniform(name)];
        let mut vs = ShcShader::ZERO;
        vs.vs_output_size = 4;
        vs.uniforms = tab.as_ptr();
        vs.num_uniforms = 1;
        let mut fs = ShcShader::ZERO;
        // SAFETY: `tab` outlives the call, and vs/fs are distinct.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, -EINVAL, "{log}");
        assert!(log.contains("vertex uniform"), "{log}");
        assert!(log.contains("64-byte name field"), "{log}");
    }

    #[test]
    fn an_attribute_at_the_storable_limit_is_refused() {
        let vs = with_attrib(MAXED);
        let mut fs = ShcShader::ZERO;
        // SAFETY: both shaders outlive the call.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, -EINVAL, "{log}");
        assert!(log.contains("vertex attribute"), "{log}");
    }

    #[test]
    fn a_fragment_stage_is_checked_too_and_named_as_the_fragment() {
        let mut vs = ShcShader::ZERO;
        vs.vs_output_size = 4;
        let mut fs = with_varying(MAXED);
        // SAFETY: both shaders outlive the call.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, -EINVAL, "{log}");
        assert!(log.contains("fragment varying"), "{log}");
    }

    #[test]
    fn a_name_one_byte_under_the_limit_is_not_refused() {
        /* The guard is the limit and nothing below it: 30 of 32 is a name the
         * field provably did not cut, and it has to link. */
        let ok = b"qt_SomeVeryLongMapName_uv_coor";
        assert_eq!(ok.len(), 30);
        let vs = with_varying(ok);
        let mut fs = with_varying(ok);
        // SAFETY: both shaders outlive the call.
        let (rc, log) = unsafe { run(&vs, &mut fs) };
        assert_eq!(rc, 0, "{log}");
    }
}
