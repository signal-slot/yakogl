// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Offline shader back end (Zephyr / bare metal).
//!
//! The PCO+glslang toolchain is too large to run on-target, so app shaders are
//! compiled to USC + wiring metadata at build time (`yakogl-tools glsl2usc`) and
//! embedded in `blobs.rs`. At runtime glShaderSource()/glCompileShader() route
//! here: compile() hashes the GLSL text (self-contained SHA-256) and looks the
//! digest up in `SHC_BLOB_TABLE`; link() validates the FS input varyings
//! against the VS output vector.
//!
//! This is the `g_shaderc` back end selected for the offline targets. An
//! online PCO back end would be a separate module for Linux/PX5/dev; only one
//! is the initial value of `g_shaderc` in a given build.

use core::ffi::{c_char, c_void};
use core::fmt::Write;

use super::sha256::sha256;
use super::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use super::{ShcBackend, ShcBlob, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use crate::util::{cstr_bytes, CStrWriter, EINVAL, ENOENT};

/// Lower-case hex of a digest, for the compile log.
struct Hex<'a>(&'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

fn stage_name(stage: u32) -> &'static str {
    if stage == SHC_STAGE_VERTEX {
        "vertex"
    } else {
        "fragment"
    }
}

/// The table row as the opaque `priv` of the shader handed out for it.
fn blob_priv(b: &'static ShcBlob) -> *const c_void {
    b as *const ShcBlob as *const c_void
}

/* ---- compile: hash source, look up embedded blob ---- */

unsafe extern "C" fn offline_compile(
    stage: u32,
    glsl: *const c_char,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    // SAFETY: the caller passes a null log or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };

    if glsl.is_null() || out.is_null() {
        let _ = log.write_str("offline shaderc: NULL source or output");
        return -EINVAL;
    }

    // SAFETY: glsl is a non-null NUL-terminated string (the GL layer's copy of the source).
    let digest = sha256(unsafe { cstr_bytes(glsl) });

    for b in SHC_BLOB_TABLE.iter() {
        if b.stage != stage || b.blend_key != 0 {
            continue;
        }
        if b.sha256 != digest {
            continue;
        }
        /* Hit: hand back the embedded shader (usc still points at the
         * static table array; it has static lifetime). */
        let mut sh = b.shader;
        sh.priv_ = blob_priv(b);
        // SAFETY: out is non-null and points at the caller's shc_shader.
        unsafe { out.write(sh) };
        return 0;
    }

    let _ = write!(
        log,
        /* the generator is `yakogl-tools glsl2usc` (tools/glsl2usc.sh runs it
         * over the starter set) and its output is src/shaderc/blobs.rs */
        "offline shaderc: {} shader not in offline table (sha256={}); \
         add it to the glsl2usc inputs and regenerate blobs.rs (yakogl-tools glsl2usc)",
        stage_name(stage),
        Hex(&digest)
    );
    -ENOENT
}

/* ---- the table's source texts (include/gles_offline.h) ---- */

/// The GLSL text of a table shader by its source basename, NUL-terminated;
/// null for a name that is not in the table.
#[no_mangle]
pub unsafe extern "C" fn gles_offline_shader_source(name: *const c_char) -> *const c_char {
    if name.is_null() {
        return core::ptr::null();
    }
    // SAFETY: name is a non-null NUL-terminated string.
    let name = unsafe { cstr_bytes(name) };
    for s in SHC_SOURCE_TABLE.iter() {
        if s.name.as_bytes() == name {
            /* text carries its own NUL (see ShcSource) */
            return s.text.as_ptr() as *const c_char;
        }
    }
    core::ptr::null()
}

/* ---- blend variants: the same source, compiled for one blend state ---- */

unsafe extern "C" fn offline_blend_variant(fs: *const ShcShader, blend_key: u32, out: *mut ShcShader) -> i32 {
    if fs.is_null() || out.is_null() {
        return -EINVAL;
    }
    // SAFETY: fs is non-null and points at a shc_shader; read by value because out may be the same object.
    let fs = unsafe { fs.read() };
    // SAFETY: priv is null or the table row offline_compile()/offline_blend_variant() stored.
    let Some(base) = (unsafe { (fs.priv_ as *const ShcBlob).as_ref() }) else {
        return -EINVAL;
    };

    if blend_key == 0 {
        // SAFETY: out is non-null and points at the caller's shc_shader.
        unsafe { out.write(fs) };
        return 0;
    }
    for b in SHC_BLOB_TABLE.iter() {
        if b.stage != SHC_STAGE_FRAGMENT || b.blend_key != blend_key {
            continue;
        }
        if b.sha256 != base.sha256 {
            continue;
        }
        let mut sh = b.shader;
        sh.priv_ = blob_priv(b);
        // SAFETY: out is non-null and points at the caller's shc_shader.
        unsafe { out.write(sh) };
        return 0;
    }
    -ENOENT /* not precompiled: add a --blend variant to glsl2usc.sh */
}

/* ---- link on the Vivante GC7000 ----
 *
 * That GPU pairs the stages by register: the fragment shader reads varying i
 * from its input register i+1, and the vertex shader's output map has to name
 * the register that feeds it. Matching is by GLSL name (what `varying` means
 * in GLSL ES 1.00), falling back to the location when a shader carries none.
 */
#[cfg(feature = "gpu-vivante")]
unsafe extern "C" fn offline_link(vs: *const ShcShader, fs: *mut ShcShader, log: *mut c_char, log_sz: usize) -> i32 {
    // SAFETY: the caller's contract, passed straight through.
    unsafe { super::link::link_shaders_viv(vs, fs, log, log_sz, "offline shaderc") }
}

/* ---- link: validate FS varyings against the VS output vector ----
 *
 * The rule itself is `super::link::by_name()`, which the remote back end
 * applies to the same ShcShader pair; only the name the log is prefixed with
 * differs. */

#[cfg(not(feature = "gpu-vivante"))]
unsafe extern "C" fn offline_link(vs: *const ShcShader, fs: *mut ShcShader, log: *mut c_char, log_sz: usize) -> i32 {
    // SAFETY: the caller's contract, passed straight through.
    unsafe { super::link::link_shaders(vs, fs, log, log_sz, "offline shaderc") }
}

/* ---- backend registration ---- */

/// The offline back end: the default for Zephyr / bare-metal builds
/// (`g_shaderc` in `shaderc/mod.rs` starts out pointing here).
pub static SHC_OFFLINE: ShcBackend = ShcBackend {
    compile: Some(offline_compile),
    link: Some(offline_link),
    blend_variant: Some(offline_blend_variant),
    compile_vs_linked: None,
    lod_sensitivity: None,
    sampler_variant: None,
};

/// Registration hook: lets a build explicitly (re)select the offline backend,
/// e.g. if another backend was linked in the same image. Returns the backend so
/// callers can also use it directly.
#[no_mangle]
pub unsafe extern "C" fn shc_offline_register() -> *const ShcBackend {
    // SAFETY: single-threaded by contract; a plain store, no reference to the static is formed.
    unsafe { super::g_shaderc = &SHC_OFFLINE };
    &SHC_OFFLINE
}

/// The link tests below are the PowerVR link's: a `gpu-vivante` build pairs
/// the stages by register instead (see the other arm of `offline_link`), and
/// its own tests live with the backend that reads the wiring.
///
/// The log strings and the table invariants are covered by tests/sha256.rs;
/// these pin the link-time varying wiring and the blend-variant lookup.
#[cfg(all(test, not(feature = "gpu-vivante")))]
mod tests {
    use super::*;
    use crate::shaderc::ShcVarying;
    use std::format;
    use std::string::String;

    /// `uses_z` is set on exactly the table's rows whose source reads
    /// `gl_FragCoord.z` -- the rule pco applies (`uses: z=1`, a Z coefficient
    /// set at cf0..3 before W), checked here against the sources themselves
    /// rather than against the generator that set it. The 32 Qt particle
    /// shaders, every variant of each (192 rows), and nothing else.
    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn uses_z_marks_exactly_the_rows_that_read_the_fragment_depth() {
        let reads_z = |name: &str| {
            let n = name.trim_end_matches('\0');
            SHC_SOURCE_TABLE
                .iter()
                .find(|s| s.name.trim_end_matches('\0') == n)
                .is_some_and(|s| s.text.contains("gl_FragCoord.z"))
        };
        let mut set = 0;
        for row in SHC_BLOB_TABLE.iter() {
            let want = u8::from(row.stage == crate::shaderc::SHC_STAGE_FRAGMENT && reads_z(row.name));
            assert_eq!(row.shader.uses_z, want, "{}: blend key {:#x}", row.name, row.blend_key);
            set += usize::from(want);
        }
        assert_eq!(set, 192, "rows with uses_z set");
    }

    fn varying(name: &str, location: u8, components: u8, vtxout_off: u8) -> ShcVarying {
        let mut v = ShcVarying { location, components, interp: 0, vtxout_off, name: [0; 32] };
        crate::util::set_name(&mut v.name, name.as_bytes());
        v
    }

    fn stage(varyings: &[ShcVarying], vs_output_size: u8) -> ShcShader {
        let mut s = ShcShader::ZERO;
        s.varyings[..varyings.len()].copy_from_slice(varyings);
        s.num_varyings = varyings.len() as u8;
        s.vs_output_size = vs_output_size;
        s
    }

    /// Run the link; returns the code and the log text.
    fn link(vs: &ShcShader, fs: &mut ShcShader) -> (i32, String) {
        let mut log = [0x55 as c_char; 256];
        // SAFETY: valid distinct shaders and a log of the announced size.
        let rc = unsafe { offline_link(vs, fs, log.as_mut_ptr(), log.len()) };
        // SAFETY: the writer NUL-terminated the log.
        let text = unsafe { cstr_bytes(log.as_ptr()) };
        (rc, String::from_utf8(text.to_vec()).unwrap())
    }

    fn table_row(name: &str, blend_key: u32) -> &'static ShcBlob {
        SHC_BLOB_TABLE.iter().find(|b| b.source_name() == name && b.blend_key == blend_key).unwrap()
    }

    #[test]
    fn the_source_name_of_a_table_shader_is_its_basename() {
        let base = table_row("uniform_color.frag", 0);
        let mut sh = base.shader;
        sh.priv_ = blob_priv(base);
        assert_eq!(crate::shaderc::shader_source_name(&sh), "uniform_color.frag");
        let plain = ShcShader::ZERO;
        assert_eq!(crate::shaderc::shader_source_name(&plain), "");
    }

    #[test]
    fn hex_is_lower_case_and_zero_padded() {
        assert_eq!(format!("{}", Hex(&[0x00, 0x0f, 0xa0, 0xff])), "000fa0ff");
        assert_eq!(format!("{}", Hex(&[])), "");
    }

    #[test]
    fn stage_names() {
        assert_eq!(stage_name(SHC_STAGE_VERTEX), "vertex");
        assert_eq!(stage_name(SHC_STAGE_FRAGMENT), "fragment");
    }

    #[test]
    fn link_by_name_patches_each_input_to_the_vs_offset() {
        /* the two stages number and place the varyings differently (sprite.vert/.frag do) */
        let vs = stage(&[varying("progress", 0, 1, 4), varying("fTexS", 1, 4, 5)], 9);
        let mut fs = stage(&[varying("fTexS", 0, 4, 99), varying("progress", 1, 1, 77)], 0);
        assert_eq!(link(&vs, &mut fs), (0, String::new()));
        assert_eq!(fs.varyings[0].vtxout_off, 5);
        assert_eq!(fs.varyings[1].vtxout_off, 4);
        /* nothing else of the input changes */
        assert_eq!((fs.varyings[0].location, fs.varyings[0].components), (0, 4));
        assert_eq!((fs.varyings[1].location, fs.varyings[1].components), (1, 1));
    }

    #[test]
    fn link_accepts_a_wider_vertex_output() {
        let vs = stage(&[varying("v", 0, 4, 4)], 8);
        let mut fs = stage(&[varying("v", 0, 2, 0)], 0);
        assert_eq!(link(&vs, &mut fs).0, 0);
        assert_eq!(fs.varyings[0].vtxout_off, 4);
    }

    #[test]
    fn a_varying_name_as_long_as_the_field_stores_refuses_the_link() {
        /* 31 characters: `set_name` and `cname` both reserve the last byte
         * for the NUL, so 31 is the most a 32-byte field holds and a longer
         * name is stored as exactly this. Nothing in the table tells the two
         * apart -- and the stages are PAIRED on the string, so guessing
         * renders the wrong picture with no error. The link is refused
         * instead (task #47). */
        let long = "qt_SomeVeryLongMapName_uv_coord";
        assert_eq!(long.len(), 31, "the sample has to use the whole field to test it");
        let vs = stage(&[varying(long, 0, 2, 4)], 6);
        let mut fs = stage(&[varying("v_uv", 0, 2, 4)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains(long) && log.contains("truncated"), "{log}");
        assert!(log.contains("vertex varying"), "the log says which stage and which kind: {log}");
    }

    #[test]
    fn a_name_one_byte_shorter_still_links() {
        /* The refusal is the longest storable length and nothing below it:
         * 30 of 32 characters is a name the field provably did not cut. */
        let ok = "qt_SomeVeryLongMapName_uv_coor";
        assert_eq!(ok.len(), 30);
        let vs = stage(&[varying(ok, 0, 2, 4)], 6);
        let mut fs = stage(&[varying(ok, 0, 2, 0)], 0);
        assert_eq!(link(&vs, &mut fs).0, 0);
    }

    #[test]
    fn two_varyings_truncated_alike_would_have_paired_the_wrong_one() {
        /* What the refusal above prevents: these are two DIFFERENT names in
         * the GLSL, identical after the field cut them, and `by_name` takes
         * the first match -- so the fragment shader would have read the wrong
         * vertex output and drawn something plausible. */
        let a = "qt_BaseColorMapAAAA_uv_coords000";
        let b = "qt_BaseColorMapAAAA_uv_coords001";
        assert_eq!((a.len(), b.len()), (32, 32), "differing only past the 31st byte");
        let mut va = varying(a, 0, 2, 4);
        let vb = varying(b, 1, 2, 6);
        assert_eq!(
            crate::util::name_bytes(&va.name),
            crate::util::name_bytes(&vb.name),
            "the field stores 31 bytes of each, so they collide"
        );
        va.vtxout_off = 4;
        let vs = stage(&[va, vb], 8);
        /* The fragment shader reads `b`, the SECOND vertex output. Truncated,
         * it matches the first -- `by_name` takes the first match -- so
         * without the refusal the link succeeds and hands the fragment shader
         * `a`'s offset (4) where `b`'s (6) was meant. */
        let mut fs = stage(&[varying(b, 0, 2, 0)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL, "refused rather than paired: {log}");
        assert_eq!(fs.varyings[0].vtxout_off, 0, "a refused link patches nothing");
    }

    #[test]
    fn no_shipped_shader_name_reaches_the_field_limit() {
        /* The refusal above is a wall the shipped table must not already be
         * standing against: if one blob's name used its whole field, every
         * link of that shader would fail. It also keeps the margin a measured
         * number in the suite rather than a claim in a commit message --
         * `qt_BaseColorMap_uv_coords1`, 26 of 32, when this was written. */
        let mut worst = (0usize, 0usize, String::new(), "");
        let mut note = |n: &[u8], cap: usize, kind: &'static str| {
            if n.len() > worst.0 {
                worst = (n.len(), cap, String::from_utf8_lossy(n).into_owned(), kind);
            }
        };
        for b in SHC_BLOB_TABLE.iter() {
            let sh = &b.shader;
            for v in &sh.varyings[..(sh.num_varyings as usize).min(sh.varyings.len())] {
                note(crate::util::name_bytes(&v.name), v.name.len(), "varying");
            }
            for a in &sh.attribs[..(sh.num_attribs as usize).min(sh.attribs.len())] {
                note(crate::util::name_bytes(&a.name), a.name.len(), "attribute");
            }
            for u in sh.uniform_table() {
                note(crate::util::name_bytes(&u.name), u.name.len(), "uniform");
            }
        }
        let (len, cap, name, kind) = worst;
        assert!(
            len < cap.saturating_sub(1).max(1),
            "the {kind} '{name}' is {len} of {cap} bytes: the table is at the truncation limit \
             and every link of that shader now fails -- task #47's name pool is overdue"
        );
        std::println!("longest shipped name: {kind} '{name}', {len} of {cap} bytes");
    }

    #[test]
    fn link_compares_whole_names() {
        let vs = stage(&[varying("v_uv2", 0, 2, 4)], 6);
        let mut fs = stage(&[varying("v_uv", 0, 2, 4)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("'v_uv' is not written by the vertex shader"), "{log}");
    }

    #[test]
    fn link_ignores_vs_outputs_the_fs_does_not_read() {
        let vs = stage(&[varying("a", 0, 4, 4), varying("b", 1, 4, 8)], 12);
        let mut fs = stage(&[varying("b", 0, 4, 0)], 0);
        assert_eq!(link(&vs, &mut fs).0, 0);
        assert_eq!(fs.varyings[0].vtxout_off, 8);
    }

    #[test]
    fn link_without_varyings_needs_only_gl_position() {
        let mut fs = stage(&[], 0);
        assert_eq!(link(&stage(&[], 4), &mut fs).0, 0);
        let (rc, log) = link(&stage(&[], 3), &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("no gl_Position"), "{log}");
    }

    #[test]
    fn link_by_location_keeps_the_offset_of_an_unnamed_input() {
        /* an input without a name is wired by the offset the table carries */
        let vs = stage(&[varying("other", 0, 4, 4)], 12);
        let mut fs = stage(&[varying("", 1, 4, 8)], 0);
        assert_eq!(link(&vs, &mut fs), (0, String::new()));
        assert_eq!(fs.varyings[0].vtxout_off, 8);
    }

    #[test]
    fn link_range_checks_unnamed_inputs() {
        let vs = stage(&[], 8);
        let mut fs = stage(&[varying("", 2, 4, 3)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("loc2 overlaps gl_Position (vtxout_off=3)"), "{log}");

        let mut fs = stage(&[varying("", 2, 4, 5)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("loc2 reads VS output dwords up to 9 but VS emits only 8"), "{log}");

        /* ending exactly at the output size is fine */
        let mut fs = stage(&[varying("", 2, 4, 4)], 0);
        assert_eq!(link(&vs, &mut fs).0, 0);
    }

    #[test]
    fn link_range_checks_the_patched_offset_not_the_fs_one() {
        /* the FS row says 4 (fine), the VS placed it at 6 where 4 components do not fit into 8 */
        let vs = stage(&[varying("v", 0, 4, 6)], 8);
        let mut fs = stage(&[varying("v", 0, 4, 4)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("up to 10 but VS emits only 8"), "{log}");
    }

    #[test]
    fn link_checks_every_input_not_only_the_first() {
        let vs = stage(&[varying("a", 0, 4, 4), varying("b", 1, 4, 8)], 12);
        let mut fs = stage(&[varying("a", 0, 4, 0), varying("b", 1, 4, 0), varying("c", 2, 1, 0)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("'c' is not written"), "{log}");
    }

    #[test]
    fn link_offset_arithmetic_does_not_wrap_at_a_byte() {
        let vs = stage(&[], 255);
        let mut fs = stage(&[varying("", 0, 4, 253)], 0);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("up to 257 but VS emits only 255"), "{log}");
    }

    #[test]
    fn link_clamps_varying_counts_to_the_array() {
        let mut vs = stage(&[varying("v", 0, 4, 4)], 8);
        vs.num_varyings = 200;
        let mut fs = stage(&[varying("v", 0, 4, 0)], 0);
        assert_eq!(link(&vs, &mut fs).0, 0);

        /* an over-long FS count reads the 16 array entries and no further; the zeroed ones sit on gl_Position */
        fs.num_varyings = 200;
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("overlaps gl_Position (vtxout_off=0)"), "{log}");
    }

    #[test]
    fn link_with_a_null_log_still_reports_the_code() {
        let vs = stage(&[], 0);
        let mut fs = stage(&[], 0);
        // SAFETY: a null log is allowed.
        assert_eq!(unsafe { offline_link(&vs, &mut fs, core::ptr::null_mut(), 0) }, -EINVAL);
    }

    #[test]
    fn compile_hands_out_the_base_row_as_priv() {
        let row = table_row("uniform_color.frag", 0);
        let src = SHC_SOURCE_TABLE.iter().find(|s| s.name == "uniform_color.frag").unwrap();
        let mut out = ShcShader::ZERO;
        // SAFETY: the table text is NUL-terminated; a null log is allowed.
        let rc = unsafe {
            offline_compile(SHC_STAGE_FRAGMENT, src.text.as_ptr() as *const c_char, &mut out, core::ptr::null_mut(), 0)
        };
        assert_eq!(rc, 0);
        assert!(core::ptr::eq(out.priv_ as *const ShcBlob, row), "never a blend variant of the same digest");
        assert_eq!(out.blend_key, 0);
        /* the same text is not a vertex shader */
        // SAFETY: as above.
        let rc = unsafe {
            offline_compile(SHC_STAGE_VERTEX, src.text.as_ptr() as *const c_char, &mut out, core::ptr::null_mut(), 0)
        };
        assert_eq!(rc, -ENOENT);
    }

    #[test]
    fn blend_variant_rejects_null_arguments() {
        let mut out = ShcShader::ZERO;
        let mut fs = ShcShader::ZERO;
        fs.priv_ = blob_priv(table_row("uniform_color.frag", 0));
        // SAFETY: deliberately null arguments.
        unsafe {
            assert_eq!(offline_blend_variant(core::ptr::null(), 1, &mut out), -EINVAL);
            assert_eq!(offline_blend_variant(&fs, 1, core::ptr::null_mut()), -EINVAL);
        }
    }

    #[test]
    fn blend_variant_finds_the_row_of_the_same_source_and_key() {
        let base = table_row("uniform_color.frag", 0);
        let var = SHC_BLOB_TABLE.iter().find(|b| b.source_name() == "uniform_color.frag" && b.blend_key != 0).unwrap();
        /* hand-built: only priv matters for the lookup */
        let mut fs = ShcShader::ZERO;
        fs.priv_ = blob_priv(base);
        let mut out = ShcShader::ZERO;
        // SAFETY: valid shaders.
        assert_eq!(unsafe { offline_blend_variant(&fs, var.blend_key, &mut out) }, 0);
        assert_eq!(out.usc, var.shader.usc);
        assert_eq!(out.blend_key, var.blend_key);
        assert!(core::ptr::eq(out.priv_ as *const ShcBlob, var));

        /* another source's variant of the same key is a different row */
        let other = table_row("interp_color.frag", var.blend_key);
        assert_ne!(out.usc, other.shader.usc);
    }

    #[test]
    fn blend_variant_of_a_variant_is_keyed_by_the_shared_digest() {
        let mut keys = SHC_BLOB_TABLE.iter().filter(|b| b.source_name() == "uniform_color.frag" && b.blend_key != 0);
        let (a, b) = (keys.next().unwrap(), keys.next().unwrap());
        let mut fs = a.shader;
        fs.priv_ = blob_priv(a);
        let mut out = ShcShader::ZERO;
        // SAFETY: valid shaders.
        assert_eq!(unsafe { offline_blend_variant(&fs, b.blend_key, &mut out) }, 0);
        assert_eq!(out.usc, b.shader.usc);
    }

    #[test]
    fn blend_variant_key_zero_copies_the_input_even_in_place() {
        let base = table_row("uniform_color.frag", 0);
        let mut fs = base.shader;
        fs.priv_ = blob_priv(base);
        fs.temps = 0x1234; /* marks the caller's copy */
        let p: *mut ShcShader = &mut fs;
        // SAFETY: in == out is allowed (the input is read by value first).
        assert_eq!(unsafe { offline_blend_variant(p, 0, p) }, 0);
        assert_eq!(fs.temps, 0x1234);
        assert_eq!(fs.usc, base.shader.usc);
    }

    #[test]
    fn blend_variant_missing_key_or_foreign_base_is_enoent() {
        let mut out = ShcShader::ZERO;
        out.temps = 0x4321;
        let mut fs = ShcShader::ZERO;
        fs.priv_ = blob_priv(table_row("uniform_color.frag", 0));
        // SAFETY: valid shaders.
        assert_eq!(unsafe { offline_blend_variant(&fs, 0x3fff_ffff, &mut out) }, -ENOENT);
        assert_eq!(out.temps, 0x4321, "out is untouched on failure");

        /* a vertex row has no fragment variants */
        let key = SHC_BLOB_TABLE.iter().find(|b| b.blend_key != 0).unwrap().blend_key;
        fs.priv_ = blob_priv(table_row("passthrough.vert", 0));
        // SAFETY: valid shaders.
        assert_eq!(unsafe { offline_blend_variant(&fs, key, &mut out) }, -ENOENT);
    }
}

/// The GC7000 table's own invariant: a fragment shader that reads
/// `gl_FrontFacing` appears twice, and the lookup finds the twin.
#[cfg(all(test, feature = "gpu-vivante"))]
mod viv_front_facing_tests {
    use super::*;
    use crate::shaderc::SHC_VIV_KEY_FRONT_FLIPPED;

    fn row(name: &str, blend_key: u32) -> &'static ShcBlob {
        SHC_BLOB_TABLE.iter().find(|b| b.source_name() == name && b.blend_key == blend_key).unwrap()
    }

    #[test]
    fn a_front_facing_shader_has_a_second_binary_under_the_same_sha256() {
        /* upstream 6c8044b: the hardware's gl_FrontFacing carries no sense, so
         * the comparison is compiled in and the table holds one binary per
         * winding. The twin is keyed by the ORIGINAL source's hash, because
         * that is what the application hands glShaderSource. */
        let base = row("es300/facing.frag", 0);
        let twin = row("es300/facing.frag @front_cw", SHC_VIV_KEY_FRONT_FLIPPED);

        assert_eq!(base.sha256, twin.sha256, "the lookup finds the twin from the compiled shader");
        assert_eq!(base.shader.reads_front_facing, 1);
        assert_eq!(twin.shader.reads_front_facing, 1);
        assert_eq!(base.shader.usc_dw, twin.shader.usc_dw);
        /* the two differ in the compare and the select, and in nothing else --
         * Mesa's ine against ieq (etnaviv_nir.c) */
        // SAFETY: each shader's code is usc_dw words at usc.
        let (a, b) = unsafe {
            (
                core::slice::from_raw_parts(base.shader.usc, base.shader.usc_dw as usize),
                core::slice::from_raw_parts(twin.shader.usc, twin.shader.usc_dw as usize),
            )
        };
        let differ: std::vec::Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
        assert_eq!(differ, [a.len() - 2, a.len() - 1], "only the last instruction differs");
    }

    #[test]
    fn the_variant_lookup_returns_the_twin_for_the_front_flipped_key() {
        let base = row("es300/facing.frag", 0);
        let mut fs = base.shader;
        fs.priv_ = blob_priv(base);
        let mut out = ShcShader::ZERO;
        // SAFETY: valid distinct shaders.
        assert_eq!(unsafe { offline_blend_variant(&fs, SHC_VIV_KEY_FRONT_FLIPPED, &mut out) }, 0);
        assert_eq!(out.usc, row("es300/facing.frag @front_cw", SHC_VIV_KEY_FRONT_FLIPPED).shader.usc);
    }

    #[test]
    fn a_shader_that_does_not_read_it_has_no_twin() {
        let base = row("color_uv.frag", 0);
        assert_eq!(base.shader.reads_front_facing, 0);
        let mut fs = base.shader;
        fs.priv_ = blob_priv(base);
        let mut out = ShcShader::ZERO;
        // SAFETY: valid distinct shaders.
        assert_eq!(unsafe { offline_blend_variant(&fs, SHC_VIV_KEY_FRONT_FLIPPED, &mut out) }, -ENOENT);
    }
}
